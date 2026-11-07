// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
// Materialize a pinned public release without depending on its project's Pages deployment.
// The archive is cached; generated lossless WebP files are shared by every site surface.
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile, copyFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import sharp from 'sharp';

const root = fileURLToPath(new URL('../', import.meta.url));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');

export async function loadReleaseGallery(app) {
  const { tag, sha256 } = app.screenshotRelease;
  if (![app.id, tag].every(s => /^[\w.-]+$/.test(s)) || !/^[a-f0-9]{64}$/.test(sha256)) {
    throw new Error('Invalid screenshot release pin');
  }
  const cache = join(root, '.cache', 'release-gallery', app.id, tag, sha256);
  await mkdir(cache, { recursive: true });
  const archive = join(cache, 'screenshots.tar.xz');
  const release = `${app.repo}/releases/download/${tag}`;
  let bytes = await readFile(archive).catch(() => null);
  if (!bytes || hash(bytes) !== sha256) {
    const response = await fetch(`${release}/screenshots.tar.xz`, { signal: AbortSignal.timeout(120_000) });
    if (!response.ok) throw new Error(`${app.id} ${tag}: screenshot download HTTP ${response.status}`);
    bytes = Buffer.from(await response.arrayBuffer());
    if (hash(bytes) !== sha256) throw new Error(`${app.id} ${tag}: screenshot archive checksum mismatch`);
    await writeFile(archive, bytes);
  }
  const manifest = join(cache, 'index.json');
  // Extract only regular PNG members named by the index, never arbitrary tar paths or links.
  // Verify the archived (pre-publication optimization) hash and actual PNG dimensions.
  const extracted = await readFile(manifest).then(() => true, () => false);
  if (!extracted) execFileSync('python3', [fileURLToPath(new URL('./unpack-release-gallery.py', import.meta.url)), archive, cache], { stdio: 'pipe' });
  const data = JSON.parse(await readFile(manifest, 'utf8'));
  const prefix = `gallery-media/${app.id}/${tag}`;
  const published = [];
  for (const shot of data.screenshots) {
    if (app.platforms && !app.platforms.includes(shot.platform)) continue;
    const relative = shot.extractedPath;
    const webp = relative.replace(/\.png$/, '.webp');
    const encoded = join(cache, webp);
    const valid = await readFile(encoded).then(() => true, () => false);
    if (!valid) await sharp(join(cache, relative)).webp({ lossless: true, effort: 4 }).toFile(encoded);
    const dest = join(root, 'public', prefix, webp);
    await mkdir(join(dest, '..'), { recursive: true });
    await copyFile(encoded, dest);
    const { extractedPath, ...metadata } = shot;
    published.push({ ...metadata, url: `/${prefix}/${webp}` });
  }
  return {
    data: { ...data, site: null, screenshots: published },
    source: { channel: 'release', url: `${release}/gallery.json`, tag },
    stale: false,
  };
}
