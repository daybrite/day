// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
// Stage the API bundle and the exact compiled site assets its shared footer uses.
import { cp, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
const website = fileURLToPath(new URL('..', import.meta.url));
for (const directory of ['api', '_astro', 'pagefind']) {
  const target = resolve(website, 'public', directory);
  if (process.argv.includes('--clean')) await rm(target, { recursive: true, force: true });
  else await cp(resolve(website, 'dist', directory), target, { recursive: true });
}
console.log(process.argv.includes('--clean') ? 'Cleared generated API preview assets.' :
  'Staged Rustdoc, shared site styles, and search assets for the development server.');
