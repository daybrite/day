// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, realpathSync, existsSync } from 'node:fs';
import { resolve, relative, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { routes, hubs, fixedRoutes, canonicalPath, navigation } from '../src/lib/content-routes.mjs';
import { checkDocSiteLinks } from './doc-site-links.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const seen = new Set([...hubs.map((p) => p.route), ...fixedRoutes]);
const files = (dir) => readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
  e.isDirectory() ? files(resolve(dir, e.name)) : [resolve(dir, e.name)]);
let count = 0;
for (const collection of ['docs', 'internal', 'repository']) {
  const base = resolve(root, 'website/src/content', collection);
  for (const file of files(base).filter((f) => /\.mdx?$/.test(f))) {
    const id = relative(base, file).replace(/\.mdx?$/, '');
    const meta = routes[`${collection}/${id}`];
    assert.ok(meta, `Missing placement: ${file}`);
    assert.equal(realpathSync(file), realpathSync(resolve(root, meta.source)), `Incorrect canonical source: ${file}`);
    assert.ok(/^\/(?:docs|contributing|features)(?:\/|$)/.test(meta.route), `Invalid content route: ${meta.route}`);
    assert.ok(!seen.has(meta.route), `Duplicate route: ${meta.route}`);
    seen.add(meta.route);
    assert.ok(['tutorial', 'how-to', 'reference', 'explanation', 'product'].includes(meta.type), `Invalid type: ${file}`);
    assert.equal(meta.audience, meta.route.startsWith('/contributing') ? 'contributor' : meta.route.startsWith('/docs') ? 'app developer' : 'evaluator');
    if (meta.route.startsWith('/contributing/design/')) assert.ok(meta.status, `Unlabeled design: ${file}`);
    count++;
  }
}
assert.equal(count, Object.keys(routes).length, 'Route catalog contains an unpublished source');
const migrations = JSON.parse(readFileSync(resolve(root, 'website/src/data/route-migrations.json')));
for (const [old, destination] of Object.entries(migrations)) {
  assert.ok(seen.has(destination), `Missing migration destination: ${old} -> ${destination}`);
  assert.equal(canonicalPath(old), destination);
}
for (const meta of Object.values(routes)) {
  if (meta.audience === 'evaluator' || meta.route === '/docs/reference') continue;
  const page = { ...meta, title: meta.route, order: 0 };
  const placements = navigation([page], meta.audience).flatMap((g) => g.pages);
  assert.equal(placements.length, 1, `Expected one sidebar placement: ${meta.route}`);
}
// Source URLs are checked against this checkout, not fetched from GitHub.
let sourceLinks = 0;
for (const file of files(resolve(root, 'website/src')).filter((f) => /\.(?:mdx?|astro|mjs|ts)$/.test(f))) {
  const text = readFileSync(file, 'utf8');
  for (const match of text.matchAll(/https:\/\/github\.com\/daybrite\/day\/(?:blob|tree)\/main\/([^\s)#<>"'`]+)/g)) {
    if (match[1].includes('${') || match[1].includes('…')) continue;
    assert.ok(existsSync(resolve(root, match[1])), `Missing source target: ${file}: ${match[1]}`);
    sourceLinks++;
  }
}
for (const file of [...files(resolve(root, 'crates')), resolve(root, 'DESIGN.md')]) {
  const failures = checkDocSiteLinks(readFileSync(file, 'utf8'), seen);
  assert.deepEqual(failures, [], `Obsolete or unknown documentation URL in ${relative(root, file)}: ${JSON.stringify(failures)}`);
}
console.log(`Content architecture OK: ${count} canonical content pages, ${seen.size} content/fixed/hub routes, ${sourceLinks} local source links; CLI and scaffold documentation URLs resolve.`);
