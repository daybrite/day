// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { createHash } from 'node:crypto';
import { appendFile, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { join, resolve } from 'node:path';
import { apps } from '../gallery.config.mjs';
import { latestRelease, releaseIdentity } from './release-gallery.mjs';
import { selectCaptures } from './gallery-selection.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));

export async function pruneGalleryCache(directory = root, configuredApps = apps) {
  const manifest = JSON.parse(await readFile(join(directory, 'src/data/gallery-manifest.json'), 'utf8'));
  const active = new Set(manifest.apps.flatMap(app => app.shots.flatMap(shot => shot.byColumn.flatMap(column =>
    Object.values(column.captures).map(image => image.src.split('/').slice(2, 5).join('/'))))));
  const cache = join(directory, '.cache/release-gallery');
  for (const app of await readdir(cache)) {
    for (const tag of await readdir(join(cache, app))) {
      for (const identity of await readdir(join(cache, app, tag))) {
        const path = join(cache, app, tag, identity);
        if (!active.has(`${app}/${tag}/${identity.slice(0, 16)}-dedup`)) {
          await rm(path, { recursive: true, force: true });
          continue;
        }
        const config = configuredApps.find(item => item.id === app);
        if (!config) throw new Error(`Unknown cached app: ${app}`);
        const renditions = selectCaptures(config, JSON.parse(await readFile(join(path, 'renditions.json'), 'utf8')));
        const retained = new Set(renditions.map(image => image.relative));
        for (const file of await readdir(path, { recursive: true })) {
          if (file.endsWith('.webp') && !retained.has(file)) await rm(join(path, file));
        }
        await writeFile(join(path, 'renditions.json'), JSON.stringify(renditions));
      }
    }
  }
}

export function galleryCacheKey(releases, policy) {
  const identities = releases.map(({ app, assets }) => [app.id, app.repo, assets.release.tag_name, releaseIdentity(assets)]);
  return createHash('sha256').update(JSON.stringify([identities, policy])).digest('hex');
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.includes('--prune')) {
    await pruneGalleryCache();
    console.log('Screenshot cache retains only the releases and renditions used by this build.');
  } else {
    // Resolve live release metadata even when CI has a cache. A cache must never pin a release.
    const releases = await Promise.all(apps.map(async app => ({ app, assets: await latestRelease(app) })));
    const inputs = ['gallery.config.mjs', 'src/lib/platforms.mjs', 'scripts/gallery-selection.mjs',
      'scripts/release-gallery.mjs', 'scripts/unpack-release-gallery.py', 'package-lock.json'];
    const policy = await Promise.all(inputs.map(path => readFile(new URL(`../${path}`, import.meta.url), 'utf8')));
    const key = galleryCacheKey(releases, policy);
    console.log(`Screenshot cache: ${key}`);
    if (process.env.GITHUB_OUTPUT) await appendFile(process.env.GITHUB_OUTPUT, `key=${key}\n`);
  }
}
