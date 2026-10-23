// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
// Check artifact completeness; Rustdoc diagnostics and item links need separate review.
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const website = fileURLToPath(new URL('..', import.meta.url));
const dist = resolve(process.argv[2] ?? resolve(website, 'dist'));
const manifest = JSON.parse(readFileSync(resolve(website, 'src/data/api-crates.json'), 'utf8'));
const crates = manifest.groups.flatMap(group => group.crates);
const entries = ['index.html', 'gallery/index.html', 'showcase/index.html', 'CNAME', 'sitemap.xml',
  'pagefind/pagefind.js', 'pagefind/pagefind-entry.json',
  'docs/reference/api/index.html', `${manifest.outDir}/index.html`,
  'styles/rustdoc.css', 'scripts/theme.js', 'scripts/rustdoc.js',
  ...crates.map(crate => `${manifest.outDir}/${crate.pkg.replaceAll('-', '_')}/index.html`)];
const missing = entries.filter(entry => {
  try { const file = statSync(resolve(dist, entry)); return !file.isFile() || file.size === 0; }
  catch { return true; }
});
if (missing.length) {
  console.error(`Incomplete site/API artifact in ${dist}:\n${missing.join('\n')}`);
  process.exitCode = 1;
} else {
  const bridge = readFileSync(resolve(dist, 'docs/reference/api/index.html'), 'utf8');
  const sharedStyles = [...bridge.matchAll(/<link\b[^>]*rel="stylesheet"[^>]*href="([^"]+)"/g)].map(m => m[1]);
  const errors = [];
  let themed = 0;
  function check(directory) {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = resolve(directory, entry.name);
      if (entry.isDirectory()) check(path);
      else if (entry.name.endsWith('.html')) {
        const html = readFileSync(path, 'utf8');
        if (!/<body\b[^>]*class="[^"]*\brustdoc\b/.test(html)) continue;
        themed++;
        for (const asset of [...sharedStyles, '/styles/rustdoc.css', '/scripts/theme.js', '/scripts/rustdoc.js']) {
          if (!html.includes(`"${asset}"`) || !statSync(resolve(dist, '.' + asset), { throwIfNoEntry: false })?.isFile()) {
            errors.push(`${path}: missing shared asset ${asset}`);
          }
        }
        if (!/<header\b[^>]*class="[^"]*\bnav\b/.test(html) || !/<footer\b[^>]*class="[^"]*\bfooter\b/.test(html)) {
          errors.push(`${path}: missing the shared site header/footer`);
        }
      }
    }
  }
  check(resolve(dist, manifest.outDir));
  if (!sharedStyles.length || !themed) errors.push('No shared site styles or themed Rustdoc pages found.');
  if (errors.length) {
    console.error(errors.join('\n'));
    process.exitCode = 1;
  } else {
    console.log(`Shared Rustdoc presentation: ${themed} pages use the site header, footer, styles and theme assets.`);
    console.log(`Site/API artifact complete: site, API bridge, and ${crates.length} crate entries.`);
  }
}
