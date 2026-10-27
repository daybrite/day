// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { checkCargo, checkDocument, checkRepository } from './doc-dependency-versions.mjs';

test('aliased, multiline, target, workspace, and bare Day dependencies may not name a release', () => {
  for (const input of [
    'day = { package = "day-app", version = "0.6" }',
    '[dependencies.renamed]\npackage = "day-app"\nversion = "0.6.2"',
    '[target.\'cfg(windows)\'.dependencies]\nday-build = "0.6"',
    '[workspace.dependencies]\nday-build = "0.6"',
    '[build-dependencies]\nday-build = { version = "0.6", optional = true }',
    'day-piece-webview = "0.4.4"',
  ]) assert.match(checkCargo(input).join('\n'), /names the release .*cargo add/);
  assert.match(checkCargo('day = { package = "day-app", version = "0.6" }')[0], /cargo add day-app --rename day/);
  assert.deepEqual(checkCargo('day-build = { path = "../day-build" }'), []);
  assert.deepEqual(checkCargo('day-build = { workspace = true }'), []);
  assert.deepEqual(checkCargo('day = { package = "day-app", git = "https://github.com/daybrite/day" }'), []);
  assert.deepEqual(checkCargo('[package]\nname = "sample-app"\nversion = "0.1.0"\n[dependencies]\nserde = "1"'), []);
  assert.ok(checkCargo('day = { package = "day-app", version =').length);
});

test('copyable fences, inline dependencies, CLI pins, and API URLs are checked', () => {
  const input = '```toml\n[dependencies.renamed]\npackage = "day-app"\nversion = "0.2"\n```\n' +
    '`day-build = "0.3"`\n' +
    'day doctor verify --day-version 0.2.0\n' +
    '[API](https://docs.rs/day-piece-webview/0.4.3/day_piece_webview/)';
  assert.equal(checkDocument(input).length, 4);
  assert.equal(checkDocument('  ```toml\n  [dependencies]\n  "day-build" = "0.2"\n  ```').length, 1);
  assert.equal(checkDocument('```toml\n[dependencies."day-build"]\nversion = "0.2"\n```').length, 1);
  assert.deepEqual(checkDocument(
    'day doctor verify --day-version latest\nday new app a --day-version X.Y.Z\n' +
    '```sh\ncargo add day-build\n```\n[API](https://docs.rs/day-piece-webview/latest/day_piece_webview/)\n' +
    'App example version 0.1.0.'), []);
});

test('repository checks include tutorials, READMEs, and the external catalog, excluding unpublished history', () => {
  const root = mkdtempSync(join(tmpdir(), 'day-doc-versions-'));
  const put = (path, value) => { const target = join(root, path); mkdirSync(join(target, '..'), { recursive: true }); writeFileSync(target, value); };
  try {
    put('website/src/data/content-routes.json', JSON.stringify({
      model: { source: 'docs/model.md' }, history: { source: 'docs/history.md', published: false },
    }));
    put('docs/model.md', '```sh\ncargo add day-app --rename day --features model\n```');
    put('docs/history.md', '```toml\nday = "0.2"\n```');
    assert.equal(checkRepository(root).documents, 1);
    put('website/tutorials/app.yaml', 'steps:\n  - file: Cargo.toml\n    content: |\n      [dependencies]\n      renamed = { package = "day-app", version = "0.2" }\n');
    assert.throws(() => checkRepository(root), /app.yaml.steps.0.content.*names the release "0.2"/);
    put('website/tutorials/app.yaml', 'steps: []');
    put('crates/day/README.md', '```toml\nday = "0.2"\n```');
    assert.throws(() => checkRepository(root), /README.md.*names the release "0.2"/);
    put('crates/day/README.md', 'Current setup is in the guide.');
    put('website/src/data/external-crates.json', JSON.stringify([{ pkg: 'day-part-auth', api: 'https://docs.rs/day-part-auth/0.1.0/day_part_auth/' }]));
    assert.throws(() => checkRepository(root), /day-part-auth API URL names 0.1.0/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
