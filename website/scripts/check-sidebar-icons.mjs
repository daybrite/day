// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile, readdir } from 'node:fs/promises';
import icons from '../src/data/sidebar-icons.json' with { type: 'json' };
import { routes, navigation, hubs } from '../src/lib/content-routes.mjs';

// Derive coverage from the published navigation, not a second hand-maintained page list.
const expected = new Set(['/', '/features', '/examples', '/docs', '/contributing', '/docs/reference/api', 'control:browse']);
for (const audience of ['app developer', 'contributor']) {
  for (const section of navigation(Object.values(routes), audience)) {
    expected.add(section.route);
    for (const page of section.pages) expected.add(page.route);
    for (const group of section.sections) expected.add(`group:${group.id}`);
  }
}
for (const hub of hubs.filter((h) => h.audience === 'evaluator')) expected.add(hub.route);
assert.deepEqual(new Set(Object.keys(icons)), expected, 'Sidebar icon coverage must match every published destination and group');
assert.equal(new Set(Object.values(icons)).size, expected.size, 'Choose a distinct symbol for each sidebar destination/group');
const dir = new URL('../public/icons/docs/', import.meta.url);
const sources = JSON.parse(await readFile(new URL('sources.json', dir), 'utf8'));
assert.match(sources.revision, /^[a-f0-9]{40}$/);
assert.deepEqual(new Set(Object.keys(sources.icons)), new Set(Object.values(icons)), 'Provenance must cover the selected symbols');
assert.deepEqual(new Set((await readdir(dir)).filter((f) => f.endsWith('.svg'))), new Set(Object.values(icons).map((name) => `${name}.svg`)), 'Remove unreferenced SVG assets');
const shapes = new Set();
for (const [item, name] of Object.entries(icons)) {
  assert.match(name, /^[a-z0-9_]+$/, item);
  const svg = await readFile(new URL(`${name}.svg`, dir), 'utf8');
  assert.match(svg, /<svg\b[^>]*height="24"/, `${name}: expected a 24px Material Symbol`);
  assert.match(svg, /<svg\b[^>]*width="24"/, `${name}: expected a 24px Material Symbol`);
  assert.match(svg, /<path\b/, `${name}: empty icon`);
  assert.doesNotMatch(svg, /<(?:script|foreignObject|image|style)\b|\bon\w+=|\b(?:href|src)=/i, `${name}: SVG must be self-contained`);
  const hash = createHash('sha256').update(svg).digest('hex');
  assert.equal(sources.icons[name].sha256, hash, `${name}: SVG differs from its recorded upstream source`);
  assert.equal(sources.icons[name].url, `https://raw.githubusercontent.com/google/material-design-icons/${sources.revision}/symbols/web/${name}/materialsymbolsoutlined/${name}_24px.svg`);
  const paths = [...svg.matchAll(/<path\b[^>]*\bd="([^"]+)"/g)].map((m) => m[1]).join(' ');
  assert(!shapes.has(paths), `${name}: another item uses the same icon geometry`);
  shapes.add(paths);
}
assert.match(await readFile(new URL('LICENSE', dir), 'utf8'), /Apache License/);
console.log(`Sidebar icons OK: ${expected.size} distinct local SVGs cover every destination, group, and browse control`);
