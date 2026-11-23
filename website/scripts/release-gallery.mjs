// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
// Resolve GitHub's latest full release on EVERY build. Only immutable release assets
// supply screenshots; a failed lookup must never silently select a branch or older release.
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile, copyFile, rename, rm, stat } from 'node:fs/promises';
import { createReadStream, createWriteStream } from 'node:fs';
import { pipeline } from 'node:stream/promises';
import { Readable } from 'node:stream';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import sharp from 'sharp';

const root = fileURLToPath(new URL('../', import.meta.url));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const rendition = 'webp-1600-q90-v1';

export async function latestRelease(app, request = fetch) {
  const repo = /^https:\/\/github\.com\/([\w.-]+\/[\w.-]+)\/?$/.exec(app.repo)?.[1];
  if (!repo || !/^[\w.-]+$/.test(app.id)) throw new Error('Invalid gallery repository');
  const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN;
  const response = await request(`https://api.github.com/repos/${repo}/releases/latest`, {
    headers: { Accept: 'application/vnd.github+json', ...(token ? { Authorization: `Bearer ${token}` } : {}) },
    signal: AbortSignal.timeout(30_000),
  });
  if (!response.ok) throw new Error(`${app.id}: latest full release lookup HTTP ${response.status}`);
  const release = await response.json();
  if (release.draft !== false || release.prerelease !== false || !release.published_at || !/^[\w.-]+$/.test(release.tag_name)) {
    throw new Error(`${app.id}: latest release is not a published full release`);
  }
  const index = release.assets?.find(a => a.name === 'gallery.json');
  const archive = ['screenshots.tar.xz', 'screenshots.zip'].map(name => release.assets?.find(a => a.name === name)).find(Boolean);
  if (!index || !archive) throw new Error(`${app.id} ${release.tag_name}: latest full release lacks gallery.json or screenshot bundle`);
  for (const asset of [index, archive]) {
    if (!/^sha256:[a-f0-9]{64}$/.test(asset.digest) || !asset.browser_download_url?.startsWith(`https://github.com/${repo}/releases/download/`)) {
      throw new Error(`${app.id} ${release.tag_name}: invalid release asset or missing SHA-256 digest`);
    }
  }
  return { release, index, archive };
}

async function fileHash(path) {
  const digest = createHash('sha256');
  for await (const chunk of createReadStream(path)) digest.update(chunk);
  return digest.digest('hex');
}

async function download(asset, path) {
  const digest = asset.digest.slice(7);
  if (await fileHash(path).then(value => value === digest, () => false)) return;
  const temporary = `${path}.${process.pid}.part`;
  try {
    const response = await fetch(asset.browser_download_url, { signal: AbortSignal.timeout(600_000) });
    if (!response.ok) throw new Error(`Release asset ${asset.name}: HTTP ${response.status}`);
    await pipeline(Readable.fromWeb(response.body), createWriteStream(temporary));
    if (await fileHash(temporary) !== digest) throw new Error(`Release asset ${asset.name}: checksum mismatch`);
    await rename(temporary, path);
  } finally {
    await rm(temporary, { force: true });
  }
}

// Python streams one verified PNG at a time. Backpressure bounds memory, and raw PNGs
// never occupy the disk (a release may contain thousands of very large captures).
async function encodeArchive(archive, indexPath, cache, log) {
  const child = spawn('python3', [fileURLToPath(new URL('./unpack-release-gallery.py', import.meta.url)), '--stream', archive, indexPath]);
  let stderr = '';
  child.stderr.on('data', chunk => { stderr = (stderr + chunk).slice(-8000); });
  const completed = new Promise((resolve, reject) => {
    child.on('error', reject);
    child.on('close', code => code === 0 ? resolve() : reject(new Error(`Release extraction failed: ${stderr}`)));
  });
  completed.catch(() => {});
  const iterator = child.stdout[Symbol.asyncIterator]();
  let pending = Buffer.alloc(0);
  async function read(size, eof = false) {
    const chunks = [];
    let length = 0;
    while (length < size) {
      if (!pending.length) {
        const next = await iterator.next();
        if (next.done) {
          if (eof && length === 0) return null;
          throw new Error(`Truncated screenshot stream: ${stderr}`);
        }
        pending = next.value;
      }
      const take = Math.min(size - length, pending.length);
      chunks.push(pending.subarray(0, take));
      pending = pending.subarray(take);
      length += take;
    }
    return Buffer.concat(chunks, size);
  }
  const images = [];
  const encoded = new Map();
  try {
    for (;;) {
      const header = await read(4, true);
      if (!header) break;
      const length = header.readUInt32BE();
      if (length > 100_000) throw new Error('Invalid screenshot stream header');
      const { shot, relative, bytes } = JSON.parse((await read(length)).toString());
      if (bytes > 100_000_000) throw new Error('Invalid screenshot stream size');
      const png = await read(bytes);
      const webp = relative.replace(/\.png$/, '.webp');
      const path = join(cache, webp);
      await mkdir(join(path, '..'), { recursive: true });
      const digest = shot.archived?.sha256 ?? shot.sha256;
      let image = encoded.get(digest);
      if (image) {
        await copyFile(image.path, path);
      } else {
        const { data, info } = await sharp(png).resize({ width: 1600, height: 1600, fit: 'inside', withoutEnlargement: true })
          .webp({ quality: 90, effort: 3 }).toBuffer({ resolveWithObject: true });
        await writeFile(path, data);
        image = { path, width: info.width, height: info.height };
        encoded.set(digest, image);
      }
      images.push({ ...shot, width: image.width, height: image.height, relative: webp });
      if (images.length % 500 === 0) log(`optimized ${images.length} screenshots`);
    }
    await completed;
    return images;
  } finally {
    if (child.exitCode === null) child.kill();
  }
}

export async function loadReleaseGallery(app, log = () => {}) {
  const { release, index, archive } = await latestRelease(app);
  const tag = release.tag_name;
  // Asset identities change even if a maintainer replaces an asset on the same tag.
  const identity = hash(`${index.digest}:${archive.digest}:${rendition}`);
  const cache = join(root, '.cache', 'release-gallery', app.id, tag, identity);
  await mkdir(cache, { recursive: true });
  const indexPath = join(cache, 'gallery.json');
  await download(index, indexPath);
  const data = JSON.parse(await readFile(indexPath, 'utf8'));
  if (!Array.isArray(data.screenshots) || !data.screenshots.length) throw new Error(`${app.id} ${tag}: empty screenshot index`);
  const manifest = join(cache, 'renditions.json');
  let images = await readFile(manifest, 'utf8').then(JSON.parse).catch(() => null);
  if (images && (images.length !== data.screenshots.length || !(await Promise.all(images.map(s => stat(join(cache, s.relative)).then(f => f.size > 0, () => false)))).every(Boolean))) images = null;
  if (!images) {
    log(`${app.id}: downloading and optimizing ${tag} (${data.screenshots.length} captures)`);
    const archivePath = join(cache, archive.name);
    await download(archive, archivePath);
    images = await encodeArchive(archivePath, indexPath, cache, message => log(`${app.id}: ${message}`));
    if (images.length !== data.screenshots.length) throw new Error(`${app.id} ${tag}: incomplete release archive`);
    await writeFile(manifest, JSON.stringify(images));
  }
  const prefix = `gallery-media/${app.id}/${tag}/${identity.slice(0, 16)}-dedup`;
  const copied = new Set();
  const published = [];
  for (const { relative, ...shot } of images) {
    // Some release variants are byte-identical. One content-addressed public image
    // avoids shipping (and downloading) the same capture for every locale or screen.
    const bytes = await readFile(join(cache, relative));
    const filename = `${hash(bytes)}.webp`;
    const dest = join(root, 'public', prefix, filename);
    if (!copied.has(filename)) {
      await mkdir(join(dest, '..'), { recursive: true });
      if (!(await stat(dest).then(file => file.size === bytes.length, () => false))) await writeFile(dest, bytes);
      copied.add(filename);
    }
    published.push({ ...shot, url: `/${prefix}/${filename}` });
  }
  return {
    data: { ...data, screenshots: published },
    source: { channel: 'release', url: index.browser_download_url, tag, releaseURL: release.html_url, publishedAt: release.published_at, digest: archive.digest },
    stale: false,
  };
}
