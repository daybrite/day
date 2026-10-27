// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, mkdtemp, rm } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createMarkdownProcessor } from '@astrojs/markdown-remark';
import { readRecipe, validateRecipe, resolvePlan, validateLessonSource, lessonFiles, recipeHash } from '../src/lib/tutorials.mjs';
import { initialPlatform, guessPlatform, tutorialLink } from '../src/lib/tutorial-platform.mjs';
import tutorialSteps from '../plugins/tutorial-steps.mjs';
import { mainCompatibility, shellProgram, runWalkthrough } from './tutorial-walkthrough.mjs';
import { verifyMarketingEvidence } from './verify-marketing-evidence.mjs';
const recipe = readRecipe();
const clone = () => structuredClone(recipe);
const htmlText = html => html.replace(/<[^>]+>/g, '').replaceAll('&lt;', '<').replaceAll('&gt;', '>').replaceAll('&quot;', '"').replace(/&#x([0-9a-f]+);/gi, (_, n) => String.fromCodePoint(parseInt(n,16))).replace(/&#(\d+);/g, (_, n) => String.fromCodePoint(Number(n))).replaceAll('&amp;', '&');

test('all six lessons render exactly the recipe steps for each platform, without a browser', async () => {
  assert.equal(recipe.lessons.length, 6);
  const processor = await createMarkdownProcessor({ syntaxHighlight: false, smartypants: false, remarkPlugins: [tutorialSteps] });
  for (const lesson of recipe.lessons) {
    const fileURL = new URL('../src/content/docs/' + lesson.source, import.meta.url);
    const source = await readFile(fileURL, 'utf8');
    validateLessonSource(lesson, source);
    const result = await processor.render(source, { fileURL });
    const rendered = [...result.code.matchAll(/<div class="tutorial-step" data-tutorial-platform="([^"]+)" data-tutorial-step="([^"]+)">([\s\S]*?)<\/div>/g)];
    assert(rendered.length, lesson.id);
    for (const platform of Object.keys(recipe.platforms)) {
      const plan = resolvePlan({ ...recipe, lessons: [lesson] }, platform);
      const visible = rendered.filter(m => ['all', platform].includes(m[1]));
      assert.deepEqual(visible.map(m => m[2]), plan.map(s => s.key));
      for (const [index, match] of visible.entries()) {
        const step = plan[index];
        if (step.kind === 'note') assert(htmlText(match[3]).includes(step.body));
        else {
          const code = /<pre[^>]*><code[^>]*>([\s\S]*?)<\/code>/.exec(match[3]);
          assert(code, step.key);
          const copy = /data-copy-text="([^"]+)"/.exec(match[3]);
          assert(copy, step.key);
          assert.equal(decodeURIComponent(copy[1]), step.body);
          assert.equal(htmlText(code[1]), step.body, `${platform} ${step.key}`);
        }
      }
    }
  }
});

test('invalid recipes, missing platform content, and source drift fail closed', async () => {
  let r = clone(); r.lessons[0].steps[0].surprise = true;
  assert.throws(() => validateRecipe(r), /unknown key/);
  r = clone(); r.lessons[0].steps.find(s => s.kind === 'file').path = '../outside';
  assert.throws(() => validateRecipe(r), /invalid file path/);
  r = clone(); delete r.lessons[0].steps.find(s => typeof s.command === 'object').command.windows;
  assert.throws(() => validateRecipe(r), /missing windows variant/);
  r = clone(); r.lessons[0].steps[0].content = '{{unknown}}';
  assert.throws(() => validateRecipe(r), /unknown substitution/);
  const lesson = recipe.lessons[0];
  const source = await readFile(new URL('../src/content/docs/' + lesson.source, import.meta.url), 'utf8');
  assert.throws(() => validateLessonSource(lesson, source.replace('first-app/install', 'first-app/create')), /reordered steps/);
  assert.throws(() => validateLessonSource(lesson, source + '\n```sh\nuntracked-command\n```'), /YAML recipe/);
  assert.notEqual(recipeHash(recipe), recipeHash({ ...recipe, title: 'Synthetic changed title' }));
});

test('platform preferences outrank browser guesses and mobile devices are not desktop hosts', () => {
  const nav = { userAgentData: {platform:'Windows'}, userAgent:'Windows' };
  assert.equal(initialPlatform('https://daybrite.dev/docs/tutorials/first-app?platform=linux', 'macos', nav), 'linux');
  assert.equal(initialPlatform('https://daybrite.dev/docs/tutorials/first-app', 'macos', nav), 'macos');
  assert.equal(initialPlatform('https://daybrite.dev/docs/tutorials/first-app?platform=invalid', 'invalid', nav), 'windows');
  assert.equal(guessPlatform({platform:'MacIntel'}), 'macos');
  assert.equal(guessPlatform({userAgent:'X11; Linux x86_64'}), 'linux');
  assert.equal(guessPlatform({platform:'Linux',userAgent:'Android'}), null);
  assert.equal(guessPlatform({platform:'MacIntel',maxTouchPoints:5}), null);
  assert.equal(guessPlatform({userAgent:'Unknown'}), null);
  assert.equal(tutorialLink('/docs/tutorials/state#run', 'https://daybrite.dev/docs/tutorials/first-app', 'windows'), '/docs/tutorials/state?platform=windows#run');
  assert.equal(tutorialLink('https://example.org/docs/tutorials/state', 'https://daybrite.dev', 'windows'), 'https://example.org/docs/tutorials/state');
});

test('main compatibility is explicit and never mutates the reader plan', () => {
  const plan = resolvePlan(recipe, 'windows');
  const before = JSON.stringify(plan);
  const main = mainCompatibility(plan, '/synthetic/checkout');
  assert.equal(JSON.stringify(plan), before);
  // The reader's manifest names no Day release: `cargo add` picks the newest one.
  assert.doesNotMatch(plan.filter(s => s.path === 'Cargo.toml').map(s => s.body).join(''), /day-app|day-build/);
  assert.match(plan.find(s => s.key === 'first-app/dependencies').body, /^cargo add day-app --rename day\n/);
  assert.match(main.find(s => s.key === 'first-app/dependencies').body, /--path '\/synthetic\/checkout\/crates\/day'/);
  assert.deepEqual(main.filter((s, i) => s.body !== plan[i].body).map(s => s.key), ['first-app/install', 'first-app/dependencies']);
  const ps = shellProgram(plan, "C:/Synthetic Fixture's path", 'powershell');
  assert.match(ps, /Fixture''s path/);
  assert.match(ps, /\$PSNativeCommandUseErrorActionPreference = \$true/);
  assert(ps.includes(plan.find(s => s.key === 'first-app/create').body));
});

test('moving samples into YAML preserves the captured tutorial app', async () => {
  assert.equal(await verifyMarketingEvidence(), 2);
  for (const platform of ['macos', 'windows', 'linux']) {
    const files = lessonFiles(recipe, platform);
    assert(files.get('Day.toml').includes(recipe.platforms[platform].target));
    assert.equal(files.get('src/lib.rs'), lessonFiles(recipe, 'macos').get('src/lib.rs'));
  }
});

// Synthetic recipe: no real Day installation or UI. Exercises the actual shell and file runner.
function fixture(fail = false) {
  const r = clone(); r.project = 'fixture';
  const fakeInstall = `node -e 'const f=require("node:fs"),p=require("node:path");const b=p.join(process.env.CARGO_HOME,"bin");f.mkdirSync(b);f.writeFileSync(p.join(b,"day"),"#!/bin/sh\\nprintf synthetic-cli\\n",{mode:493});'\n`;
  r.lessons = [{id:'fixture',source:'fixture.md',route:'/docs/tutorials/fixture',steps:[
    {id:'install',kind:'run',cwd:'.',command:fakeInstall},
    {id:'create',kind:'run',cwd:'.',command:'mkdir fixture\ncd fixture\nexport TUTORIAL_FIXTURE_VALUE=retained\n'},
    {id:'write',kind:'file',cwd:'fixture',path:'data.txt',mode:'write',language:'text',content:'first\n'},
    {id:'append',kind:'file',cwd:'fixture',path:'data.txt',mode:'append',language:'text',content:'second\n'},
    {id:'verify',kind:'run',cwd:'fixture',command: fail ? 'false\nprintf should-not-run > unexpected.txt\n' : 'test "$TUTORIAL_FIXTURE_VALUE" = retained\ntest "$(cat data.txt)" = "$(printf \'first\\nsecond\')"\n'},
  ]}];
  return r;
}
test('walkthrough preserves shell state, writes exact files, and stops on the first failure', {skip:process.platform==='win32'}, async () => {
  const temp = await mkdtemp(join(tmpdir(), 'day-tutorial-fixture-'));
  const platform = process.platform === 'darwin' ? 'macos' : 'linux';
  try {
    const directory = join(temp, "reader's workspace");
    await runWalkthrough({recipe:fixture(),platform,directory});
    assert.equal(await readFile(join(directory,'work/fixture/data.txt'),'utf8'),'first\nsecond\n');
    assert.equal(JSON.parse(await readFile(join(directory,'evidence/result.json'),'utf8')).status,'pass');
    const failed = join(temp,'failure');
    await assert.rejects(runWalkthrough({recipe:fixture(true),platform,directory:failed}),/Walkthrough failed/);
    assert.equal(existsSync(join(failed,'work/fixture/unexpected.txt')),false);
    assert.equal(JSON.parse(await readFile(join(failed,'evidence/result.json'),'utf8')).status,'fail');
    const missing = fixture();missing.lessons[0].steps[2].path='missing/parent.txt';
    await assert.rejects(runWalkthrough({recipe:missing,platform,directory:join(temp,'missing-parent')}),/Walkthrough failed/);
  } finally { await rm(temp,{recursive:true,force:true}); }
});

test('authoring documentation can describe markers without expanding them', async () => {
  const processor = await createMarkdownProcessor({syntaxHighlight:false, remarkPlugins:[tutorialSteps]});
  const result = await processor.render('Use `<!-- tutorial-step: lesson-id/step-id -->` in a lesson.', {fileURL:new URL('../authoring/authoring.md', import.meta.url)});
  assert(result.code.includes('lesson-id/step-id'));
});
