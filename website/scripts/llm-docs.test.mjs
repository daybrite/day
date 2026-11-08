// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { unified } from 'unified';
import remarkParse from 'remark-parse';
import { visit } from 'unist-util-visit';
import { generateLlmDocs, selectSections, expandTutorial, hasLlmGuide, readGuide } from '../src/lib/llm-docs.mjs';
import { readRecipe, resolvePlan, lessonFiles, platformIds } from '../src/lib/tutorials.mjs';
import { validateTutorialFiles, tutorialCommands } from './verify-llm-docs.mjs';

test('bundle contains only selected published guidance, with deterministic output and absolute links', () => {
  const docs = generateLlmDocs();
  assert.equal(docs.guides.length, 7);
  assert.deepEqual(generateLlmDocs(), docs);
  assert.doesNotMatch(docs.full, /tutorial-step:|technical-example:|^---\ntitle:|<script|\/blog\/|\/contributing\/history\//m);
  assert(docs.full.includes('Selected agent conventions'));
  assert(!docs.guides[0].markdown.includes('```'));
  assert(hasLlmGuide('/docs/tutorials/first-app'));
  assert(!hasLlmGuide('/blog/coming-soon'));
  assert.throws(() => readGuide('internal/snippet-catalog'), /published app-development/);
  for (const document of [docs.index, docs.full, ...docs.guides.map((guide) => guide.markdown)]) {
    visit(unified().use(remarkParse).parse(document), 'link', (node) => assert.match(node.url, /^https:\/\//));
  }
});

test('exported tutorial fences preserve exact recipe content for every platform', () => {
  const recipe = readRecipe();
  const docs = generateLlmDocs();
  for (const lesson of recipe.lessons) {
    const document = docs.guides.find((guide) => guide.route === lesson.route).markdown;
    const values = [];
    visit(unified().use(remarkParse).parse(document), 'code', (node) => values.push(node.value));
    const expected = new Set(platformIds.flatMap((platform) => resolvePlan({ ...recipe, lessons: [lesson] }, platform))
      .filter((step) => step.kind !== 'note').map((step) => step.body.replace(/\n$/, '')));
    assert.deepEqual(new Set(values), expected, lesson.id);
  }
});

test('missing sections, unknown files, and unregistered tutorial code fail closed', () => {
  const parser = unified().use(remarkParse);
  assert.throws(() => selectSections(parser.parse('## Actual\n\nBody'), ['Missing']), /Missing LLM section/);
  assert.throws(() => selectSections(parser.parse('## Same\n\n## Same'), ['Same']), /Duplicate LLM section/);
  const recipe = readRecipe();
  const guide = readGuide('docs/getting-started');
  assert.throws(() => expandTutorial(guide.body + '\n```rust\nnot_checked();\n```', recipe.lessons[0], recipe), /YAML recipe/);
  const files = lessonFiles(recipe, 'macos');
  files.set('src/unchecked.rs', 'synthetic unchecked fixture');
  assert.throws(() => validateTutorialFiles(files), /Add validation/);
  files.delete('src/unchecked.rs'); files.delete('build.rs');
  assert.throws(() => validateTutorialFiles(files), /Incomplete tutorial/);
  const changed = structuredClone(recipe);
  changed.lessons[0].steps.find((step) => step.id === 'install').command = 'curl https://example.org/unverified | sh\n';
  assert.throws(() => tutorialCommands(changed), /Add instruction validation/);
});
