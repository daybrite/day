// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtemp, writeFile, rm, symlink } from 'node:fs/promises';
import { resolve, relative } from 'node:path';
import { inspectMarkdown, checkFiles, checkGlossary, root } from './docs-check.mjs';

// Synthetic Markdown fixtures: never shipped UI or API examples.
test('prose rules ignore code and comments, including nested fences', async () => {
  const result = await inspectMarkdown('# Fixture\n\n`simply`\n\n<!-- easy -->\n\n````md\n```text\neasy\n```\n````\n');
  assert.deepEqual(result.issues, []);
});

test('structural and editorial violations fail independently', async () => {
  const result = await inspectMarkdown('# Fixture\n\n### Gap\n\nSimply proceed. [click here](missing.md) ![](image.png)\n\n```\nunclosed\n');
  for (const rule of ['heading-order', 'prose', 'link-label', 'image-alt', 'code-language', 'code-closure']) {
    assert.ok(result.issues.some((item) => item.rule === rule), rule);
  }
});

test('front matter supplies the title and preserves diagnostic lines', async () => {
  const result = await inspectMarkdown('---\ntitle: Fixture\ndescription: Synthetic\n---\n\n## Section\n\nObviously proceed.\n');
  assert.deepEqual(result.issues.map((i) => [i.rule, i.line]), [['prose', 8]]);
});

test('reference links and duplicate heading anchors use the Markdown parser', async () => {
  const result = await inspectMarkdown('# Fixture\n\n## Same\n\n## Same\n\n[Destination][ref]\n\n[ref]: target.md#same-1\n');
  assert.equal(result.links[0].url, 'target.md#same-1');
  assert.ok(result.anchors.has('same-1'));
});

test('local links reject missing files and fragments without accessing the network', async () => {
  const directory = await mkdtemp(resolve(root, 'website/scripts/.docs-check-test-'));
  try {
    const target = resolve(directory, 'target.md');
    const source = resolve(directory, 'source.md');
    await writeFile(target, '# Target\n\n## Exists\n');
    await writeFile(source, '# Source\n\n[Valid](target.md#exists) [Missing](absent.md) [Fragment](target.md#absent) [External](https://example.invalid)\n');
    const result = await checkFiles([source]);
    assert.equal(result.errors.length, 2);
    assert.equal(result.checkedLinks, 1);
    assert.equal(result.skippedLinks, 1);
    assert.ok(result.errors.some((e) => e.includes('missing Markdown heading fragment')));
  } finally {
    await rm(directory, { recursive: true });
  }
});

test('glossary contract requires core terms and unique anchors', () => {
  const issues = checkGlossary([{ id: 'piece' }, { id: 'piece' }]);
  assert.ok(issues.some((i) => i.includes('duplicate')));
  assert.ok(issues.some((i) => i.includes('Missing core term: window')));
  assert.ok(issues.some((i) => i.includes('Incomplete')));
});

test('existing ignored files cannot satisfy public links, including through source symlinks', async () => {
  const directory = await mkdtemp(resolve(root, 'website/scripts/.docs-check-test-'));
  try {
    const source = resolve(directory, 'source.md');
    const local = resolve(directory, 'local.md');
    const alias = resolve(directory, 'alias.md');
    await writeFile(local, '# Local fixture\n');
    await symlink('local.md', alias);
    await writeFile(source, '# Source fixture\n\n[Local](local.md) [Alias](alias.md)\n');
    const sourcePaths = new Set([source, alias].map((p) => relative(root, p).split('\\').join('/')));
    const result = await checkFiles([source], { sourcePaths });
    assert.equal(result.errors.length, 2);
    assert.ok(result.errors.every((e) => e.includes('ignored local file')));
    sourcePaths.add(relative(root, local).split('\\').join('/'));
    assert.deepEqual((await checkFiles([source], { sourcePaths })).errors, []);
  } finally {
    await rm(directory, { recursive: true });
  }
});

test('canonical repository metadata does not duplicate its visible H1', async () => {
  const source = '---\ntitle: Fixture\n---\n\n# Fixture\n\n## Contract\n';
  assert.deepEqual((await inspectMarkdown(source, { bodyTitle: true })).issues, []);
  assert.ok((await inspectMarkdown(source)).issues.some((i) => i.rule === 'title'));
  assert.ok((await inspectMarkdown(source + '\n# Duplicate\n', { bodyTitle: true })).issues.some((i) => i.rule === 'title'));
});


test('generic importance headings are rejected, while quoted rule examples are allowed', async () => {
  const bad = await inspectMarkdown('# Guide\n\n## Why it matters\n\nUseful content.\n');
  assert.ok(bad.issues.some((issue) => issue.rule === 'prose'));
  const good = await inspectMarkdown('# Style\n\nAvoid `Why it matters` headings.\n');
  assert.deepEqual(good.issues, []);
});
