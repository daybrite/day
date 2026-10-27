// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { readFile, readdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { unlinkedReferences } from './api-reference-coverage.mjs';

const root = fileURLToPath(new URL('../../', import.meta.url));
const args = process.argv.slice(2);
const audit = args.includes('--audit');
// The docs.rs-equivalent rustdoc scripts/build-api-docs.sh builds; the site publishes none of its own.
const docs = resolve(root, args.find(a => !a.startsWith('--')) ?? 'target/api-check/x86_64-unknown-linux-gnu/doc');
const names = new Set();
async function index(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) await index(path);
    else if (entry.name.endsWith('.html')) {
      const fn = /^fn\.(\w+)\.html$/.exec(entry.name);
      if (fn) names.add(fn[1]);
      // IDs include provided trait methods, required methods and inherent methods.
      for (const match of (await readFile(path, 'utf8')).matchAll(/id="(?:tymethod|method)\.([a-z_]\w*)"/g)) names.add(match[1]);
    }
  }
}
await index(docs);
if (!names.size) throw new Error(`No Rust API found in ${docs}; run scripts/build-api-docs.sh first.`);
const routes = JSON.parse(await readFile(resolve(root, 'website/src/data/content-routes.json'), 'utf8'));
const sources = [...new Set(Object.values(routes).filter(p => p.published !== false).map(p => p.source))].sort();
const exceptions = JSON.parse(await readFile(resolve(root, 'website/authoring/api-reference-exceptions.json'), 'utf8'));
const failures = [], found = [], used = new Set();
for (const source of sources) {
  for (const reference of await unlinkedReferences(await readFile(resolve(root, source), 'utf8'), names)) {
    found.push({ source, ...reference });
    const exception = exceptions[source]?.find(e => e.code === reference.code && e.reason?.trim());
    if (exception) used.add(`${source}\0${exception.code}`);
    else failures.push(`${source}:${reference.line}: Link ${JSON.stringify(reference.code)} to its verified Rust API owner, or document why no published Day item applies.`);
  }
}
for (const [source, entries] of Object.entries(exceptions)) {
  const seen = new Set();
  for (const entry of entries) {
    if (seen.has(entry.code)) failures.push(`${source}: duplicate API reference exception ${JSON.stringify(entry.code)}`);
    seen.add(entry.code);
    if (!used.has(`${source}\0${entry.code}`)) failures.push(`${source}: remove stale API reference exception ${JSON.stringify(entry.code)}`);
  }
}
if (audit) console.log(JSON.stringify(found, null, 2));
else if (failures.length) { console.error(failures.join('\n')); process.exitCode = 1; }
else console.log(`API reference coverage: ${sources.length} documents, ${names.size} published callable names, ${used.size} reviewed exceptions; no unlinked callable references.`);
