// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
// Check artifact completeness; Rustdoc diagnostics and item links need separate review.
import { readFileSync, statSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const website = fileURLToPath(new URL('..', import.meta.url));
const dist = resolve(process.argv[2] ?? resolve(website, 'dist'));
const manifest = JSON.parse(readFileSync(resolve(website, 'src/data/api-crates.json'), 'utf8'));
const crates = manifest.groups.flatMap(group => group.crates);
const entries = ['index.html', 'docs/reference/api/index.html', `${manifest.outDir}/index.html`,
  ...crates.map(crate => `${manifest.outDir}/${crate.pkg.replaceAll('-', '_')}/index.html`)];
const missing = entries.filter(entry => {
  try { const file = statSync(resolve(dist, entry)); return !file.isFile() || file.size === 0; }
  catch { return true; }
});
if (missing.length) {
  console.error(`Incomplete site/API artifact in ${dist}:\n${missing.join('\n')}`);
  process.exitCode = 1;
} else {
  console.log(`Site/API artifact complete: site, API bridge, and ${crates.length} crate entries.`);
}
