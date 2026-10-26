// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { readRecipe, lessonFiles } from '../src/lib/tutorials.mjs';

const root = new URL('../', import.meta.url);
const sha256 = (data) => createHash('sha256').update(data).digest('hex');

/** Fail a build if a published capture or its displayed app source loses its evidence. */
export async function verifyMarketingEvidence() {
  const record = JSON.parse(await readFile(new URL('public/images/tutorial/provenance.json', root), 'utf8'));
  if (Object.keys(record.captures).sort().join(',') !== 'counter-start.png,counter-two.png' ||
      record.target !== 'macos-appkit' || record.locale !== 'en' || record.theme !== 'light') {
    throw new Error('Capture evidence does not match the tutorial presentation.');
  }
  for (const [name, hash] of Object.entries(record.captures)) {
    if (!/^[a-z-]+\.png$/.test(name)) throw new Error(`Invalid capture name: ${name}`);
    const bytes = await readFile(new URL(`public/images/tutorial/${name}`, root));
    if (sha256(bytes) !== hash) throw new Error(`Capture changed without new provenance: ${name}`);
    if (bytes.readUInt32BE(16) !== 800 || bytes.readUInt32BE(20) !== 640) {
      throw new Error(`Unexpected capture dimensions: ${name}`);
    }
  }
  const files = lessonFiles(readRecipe(), 'macos', 3);
  for (const path of ['src/lib.rs', 'src/main.rs', 'resource/locales/en/app.ftl']) {
    if (sha256(files.get(path) ?? '') !== record.app_files[path]) {
      throw new Error(`Tutorial changed; recapture before publishing: ${path}`);
    }
  }
  return Object.keys(record.captures).length;
}
