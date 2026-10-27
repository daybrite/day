// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { checkCargo, checkDocument, checkRepository } from './doc-dependency-versions.mjs';
const versions = new Map([
  ['day-app', { version: '0.6.1', core: true }],
  ['day-build', { version: '0.6.1', core: true }],
  ['day-cli', { version: '0.6.1', core: true }],
  ['day-piece-webview', { version: '0.4.4', core: false }],
]);

test('checks aliased, multiline, target, workspace, and bare Cargo dependencies', () => {
  for (const input of [
    'day = { package = "day-app", version = "0.2" }',
    '[dependencies.renamed]\npackage = "day-app"\nversion = "0.2"',
    '[target.\'cfg(windows)\'.dependencies]\nday-build = "0.5"',
    '[workspace.dependencies]\nday-build = "0.5"',
    '[build-dependencies]\nday-build = { version = "0.5", optional = true }',
  ]) assert.match(checkCargo(input, versions).join('\n'), /expected 0.6.1 or 0.6/);
  assert.deepEqual(checkCargo('[dependencies]\nday = { package = "day-app", version = "0.6", features = ["model"] }', versions), []);
  assert.deepEqual(checkCargo('day-build = { path = "../day-build" }', versions), []);
  assert.deepEqual(checkCargo('day-build = { workspace = true }', versions), []);
});

test('external releases use their own versions and unsupported requirements fail', () => {
  assert.deepEqual(checkCargo('day-piece-webview = "0.4.4"', versions), []);
  for (const version of ['0.4', '0.4.3', '0.6', '*', '>=0.1', '=0.6']) {
    assert.ok(checkCargo(`day-piece-webview = "${version}"`, versions).length);
  }
  assert.match(checkCargo('day-part-unknown = "0.1"', versions)[0], /no documented release/);
  assert.ok(checkCargo('day = { package = "day-app", version =', versions).length);
  assert.deepEqual(checkCargo('[package]\nname = "sample-app"\nversion = "0.1.0"\n[dependencies]\nserde = "1"', versions), []);
});

test('copyable fences, inline dependencies, CLI pins, and API URLs are checked', () => {
  const input = '```toml\n[dependencies.renamed]\npackage = "day-app"\nversion = "0.2"\n```\n' +
    '`day-build = "0.3"`\n' +
    'day doctor verify --day-version 0.2.0\n' +
    '[API](https://docs.rs/day-piece-webview/0.4.3/day_piece_webview/)';
  assert.equal(checkDocument(input, versions).length, 4);
  assert.equal(checkDocument('  ```toml\n  [dependencies]\n  "day-build" = "0.2"\n  ```', versions).length, 1);
  assert.equal(checkDocument('```toml\n[dependencies."day-build"]\nversion = "0.2"\n```', versions).length, 1);
  assert.deepEqual(checkDocument('day doctor verify --day-version latest\nApp example version 0.1.0.', versions), []);
});

test('repository checks derive versions and include tutorials and READMEs, excluding unpublished history', () => {
  const root = mkdtempSync(join(tmpdir(), 'day-doc-versions-'));
  const put = (path, value) => { const target = join(root, path); mkdirSync(join(target, '..'), { recursive: true }); writeFileSync(target, value); };
  try {
    put('Cargo.toml', '[workspace]\nmembers = ["crates/day"]\n[workspace.package]\nversion = "0.6.1"');
    put('crates/day/Cargo.toml', '[package]\nname = "day-app"\nversion.workspace = true');
    put('website/src/data/external-crates.json', '[]');
    put('website/src/data/doc-dependencies.json', '{}');
    put('website/src/data/content-routes.json', JSON.stringify({
      model: { source: 'docs/model.md' }, history: { source: 'docs/history.md', published: false },
    }));
    put('docs/model.md', '```toml\nday = { package = "day-app", version = "0.6" }\n```');
    put('docs/history.md', '```toml\nday = "0.2"\n```');
    assert.equal(checkRepository(root).documents, 1);
    put('website/tutorials/app.yaml', 'steps:\n  - file: Cargo.toml\n    content: |\n      [dependencies]\n      renamed = { package = "day-app", version = "0.2" }\n');
    assert.throws(() => checkRepository(root), /app.yaml.steps.0.content.*uses "0.2"/);
    put('website/tutorials/app.yaml', 'steps: []');
    put('crates/day/README.md', '```toml\nday = "0.2"\n```');
    assert.throws(() => checkRepository(root), /README.md.*uses "0.2"/);
    put('crates/day/README.md', 'Current setup is in the guide.');
    put('Cargo.toml', '[workspace]\nmembers = ["crates/day"]\n[workspace.package]\nversion = "0.7.0"');
    assert.throws(() => checkRepository(root), /expected 0.7.0 or 0.7/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
