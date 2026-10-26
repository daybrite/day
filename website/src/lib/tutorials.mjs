// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import yaml from 'js-yaml';

export const recipeURL = new URL('../../tutorials/session-counter.yaml', import.meta.url);
export const platformIds = ['macos', 'windows', 'linux'];
const safePath = (path) => typeof path === 'string' && /^(?:\.|[\w.-]+(?:\/[\w.-]+)*)$/.test(path) && !path.split('/').includes('..');
const check = (condition, message) => { if (!condition) throw new Error(`Tutorial recipe: ${message}`); };
const only = (object, keys, label) => {
  check(object && typeof object === 'object' && !Array.isArray(object), `${label} must be a mapping`);
  for (const key of Object.keys(object)) check(keys.includes(key), `${label}: unknown key ${key}`);
};

export function validateRecipe(recipe) {
  only(recipe, ['schema', 'id', 'title', 'project', 'platforms', 'lessons'], 'recipe');
  check(recipe.schema === 1, 'unsupported schema');
  check(/^[a-z][a-z0-9-]*$/.test(recipe.id), 'invalid id');
  check(typeof recipe.title === 'string' && safePath(recipe.project) && recipe.project !== '.', 'invalid project');
  check(JSON.stringify(Object.keys(recipe.platforms).sort()) === JSON.stringify([...platformIds].sort()), 'expected macos, windows, linux');
  for (const [id, platform] of Object.entries(recipe.platforms)) {
    only(platform, ['label', 'target', 'toolkit', 'shell', 'prerequisites', 'quit'], id);
    for (const key of ['label', 'target', 'toolkit', 'prerequisites', 'quit']) check(typeof platform[key] === 'string' && platform[key], `${id}: missing ${key}`);
    check(platform.shell === (id === 'windows' ? 'powershell' : 'bash'), `${id}: wrong shell`);
  }
  const ids = new Set(), sources = new Set(), routes = new Set();
  check(Array.isArray(recipe.lessons) && recipe.lessons.length > 0, 'lessons are required');
  for (const lesson of recipe.lessons) {
    only(lesson, ['id', 'source', 'route', 'steps'], 'lesson');
    check(/^[a-z][a-z0-9-]*$/.test(lesson.id) && !ids.has(lesson.id), `duplicate/invalid lesson ${lesson.id}`);
    check(safePath(lesson.source) && lesson.source.endsWith('.md') && !sources.has(lesson.source), 'invalid/duplicate source');
    check(/^\/docs\/tutorials\/[a-z0-9-]+$/.test(lesson.route) && !routes.has(lesson.route), 'invalid/duplicate route');
    ids.add(lesson.id); sources.add(lesson.source); routes.add(lesson.route);
    check(Array.isArray(lesson.steps) && lesson.steps.length > 0, `${lesson.id}: no steps`);
    const steps = new Set();
    for (const step of lesson.steps) {
      only(step, ['id', 'kind', 'cwd', 'path', 'mode', 'language', 'content', 'command', 'platforms', 'checks', 'compatibility'], lesson.id);
      check(/^[a-z][a-z0-9-]*$/.test(step.id) && !steps.has(step.id), `${lesson.id}: duplicate/invalid step ${step.id}`);
      steps.add(step.id);
      check(['note', 'file', 'run', 'launch'].includes(step.kind), `unknown step kind ${step.kind}`);
      const fields = { note:['content'], file:['cwd','path','mode','language','content'], run:['cwd','command','compatibility'], launch:['cwd','command','checks'] };
      only(step, ['id','kind','platforms', ...fields[step.kind]], `${lesson.id}/${step.id}`);
      if (step.platforms) check(Array.isArray(step.platforms) && step.platforms.length && step.platforms.every(p => platformIds.includes(p)), 'invalid platforms');
      if (step.kind !== 'note') check(safePath(step.cwd), `${step.id}: invalid working directory`);
      if (step.kind === 'file') {
        check(safePath(step.path) && step.path !== '.', `${step.id}: invalid file path`);
        check(['write', 'append'].includes(step.mode) && /^[a-z]+$/.test(step.language), `${step.id}: invalid file mode/language`);
      }
      const body = ['run', 'launch'].includes(step.kind) ? step.command : step.content;
      check(typeof body === 'string' || body && typeof body === 'object' && !Array.isArray(body), `${step.id}: missing body`);
      if (typeof body === 'object') for (const key of Object.keys(body)) check(platformIds.includes(key), `${step.id}: unknown platform variant ${key}`);
      if (step.kind === 'launch') check(Array.isArray(step.checks) && step.checks.length > 0, `${step.id}: launch needs UI checks`);
      if (step.checks) check(step.kind === 'launch' && step.checks.every(s => s && typeof s === 'object' && Object.keys(s).length === 1), 'invalid UI checks');
      if (step.compatibility) check(step.compatibility === 'install' && step.kind === 'run', 'invalid compatibility action');
      for (const platform of platformIds) if (applies(step, platform)) resolveStep(recipe, lesson, step, platform);
    }
  }
  return recipe;
}
export function parseRecipe(source) { return validateRecipe(yaml.load(source, { schema: yaml.JSON_SCHEMA })); }
export function readRecipe(path = recipeURL) { return parseRecipe(readFileSync(path, 'utf8')); }
export function recipeHash(recipe) { return createHash('sha256').update(JSON.stringify(recipe)).digest('hex'); }
export function applies(step, platform) { return !step.platforms || step.platforms.includes(platform); }
export function resolveStep(recipe, lesson, step, platform) {
  check(platformIds.includes(platform), `unknown platform ${platform}`);
  if (!applies(step, platform)) return null;
  const values = recipe.platforms[platform];
  const raw = ['run', 'launch'].includes(step.kind) ? step.command : step.content;
  let body = typeof raw === 'string' ? raw : raw[platform];
  check(typeof body === 'string' && body.trim(), `${lesson.id}/${step.id}: missing ${platform} variant`);
  body = body.replace(/\{\{(\w+)\}\}/g, (_, key) => {
    check(Object.hasOwn(values, key), `unknown substitution ${key}`);
    return values[key];
  });
  check(!body.includes('{{'), `${step.id}: malformed substitution`);
  return { ...step, key: `${lesson.id}/${step.id}`, body, platform, shell: values.shell, target: values.target };
}
export function resolvePlan(recipe, platform) {
  return recipe.lessons.flatMap(lesson => lesson.steps.map(step => resolveStep(recipe, lesson, step, platform)).filter(Boolean));
}
export function lessonFiles(recipe, platform, count = recipe.lessons.length) {
  const files = new Map();
  for (const step of resolvePlan({ ...recipe, lessons: recipe.lessons.slice(0, count) }, platform)) {
    if (step.kind === 'file') files.set(step.path, (step.mode === 'append' ? files.get(step.path) ?? '' : '') + step.body);
  }
  return files;
}

// Every step occurs exactly once, in recipe order. Executable examples belong in the recipe.
export function validateLessonSource(lesson, source) {
  const markers = [...source.matchAll(/<!-- tutorial-step: ([a-z0-9-]+\/[a-z0-9-]+) -->/g)].map(m => m[1]);
  check(JSON.stringify(markers) === JSON.stringify(lesson.steps.map(s => `${lesson.id}/${s.id}`)), `${lesson.source}: missing, duplicate, or reordered steps`);
  check((source.match(/<!-- tutorial-/g) ?? []).length === markers.length, `${lesson.source}: malformed/obsolete marker`);
  check(!/^\s*```/m.test(source), `${lesson.source}: put executable examples in the YAML recipe`);
}
