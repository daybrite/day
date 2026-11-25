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
import { availableParallelism } from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';
import sharp from 'sharp';
import { selectCaptures } from './gallery-selection.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const rendition = 'webp-1600-q90-v1';
const transientCodes = new Set(['ECONNRESET', 'ETIMEDOUT', 'EAI_AGAIN', 'UND_ERR_SOCKET',
  'UND_ERR_CONNECT_TIMEOUT', 'UND_ERR_HEADERS_TIMEOUT', 'UND_ERR_BODY_TIMEOUT']);

// A release GET can lose its connection before the headers or partway through the body.
// Retry the whole transfer so a partial file never satisfies the digest check.
async function readRelease(url, { request, timeout, headers }, consume) {
  for (let attempt = 1; ; attempt++) {
    const signal = AbortSignal.timeout(timeout);
    try {
      const response = await request(url, { headers, signal });
      if (!response.ok) {
        await response.body?.cancel();
        throw Object.assign(new Error(`HTTP ${response.status}`), { status: response.status });
      }
      return await consume(response);
    } catch (error) {
      const transient = signal.aborted || transientCodes.has(error.code) || transientCodes.has(error.cause?.code)
        || [408, 429, 500, 502, 503, 504].includes(error.status);
      const reason = error.cause?.message ?? error.message;
      if (!transient || attempt === 3) throw new Error(`${url}: ${reason}`, { cause: error });
      console.warn(`[gallery] ${url}: ${reason}; retrying (attempt ${attempt + 1}/3)`);
      await delay(500 * attempt);
    }
  }
}

export function releaseIdentity({ index, archive }) {
  return hash(`${index.digest}:${archive.digest}:${rendition}`);
}

export async function latestRelease(app, request = fetch) {
  const repo = /^https:\/\/github\.com\/([\w.-]+\/[\w.-]+)\/?$/.exec(app.repo)?.[1];
  if (!repo || !/^[\w.-]+$/.test(app.id)) throw new Error('Invalid gallery repository');
  const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN;
  const headers = { Accept: 'application/vnd.github+json', ...(token ? { Authorization: `Bearer ${token}` } : {}) };
  const release = await readRelease(`https://api.github.com/repos/${repo}/releases/latest`,
    { request, timeout: 30_000, headers }, response => response.json());
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

async function download(asset, path, request = fetch) {
  const digest = asset.digest.slice(7);
  if (await fileHash(path).then(value => value === digest, () => false)) return;
  const temporary = `${path}.${process.pid}.part`;
  try {
    await readRelease(asset.browser_download_url, { request, timeout: 600_000 },
      response => pipeline(Readable.fromWeb(response.body), createWriteStream(temporary)));
    if (await fileHash(temporary) !== digest) throw new Error(`Release asset ${asset.name}: checksum mismatch`);
    await rename(temporary, path);
  } finally {
    await rm(temporary, { force: true });
  }
}

// Python streams verified PNGs into bounded encoding batches. Backpressure bounds memory; raw PNGs
// never occupy the disk (a release may contain thousands of very large captures).
export async function encodeArchive(archive, indexPath, cache, log = () => {}, concurrency = Math.min(4, availableParallelism())) {
  if (!Number.isInteger(concurrency) || concurrency < 1 || concurrency > 4) throw new Error('Invalid screenshot concurrency');
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
  let batch = [];
  async function flush() {
    images.push(...await Promise.all(batch));
    batch = [];
  }
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
      const digest = shot.archived?.sha256 ?? shot.sha256;
      let image = encoded.get(digest);
      if (!image) {
        image = (async () => {
          const { data, info } = await sharp(png).resize({ width: 1600, height: 1600, fit: 'inside', withoutEnlargement: true })
            .webp({ quality: 90, effort: 3 }).toBuffer({ resolveWithObject: true });
          await mkdir(join(path, '..'), { recursive: true });
          await writeFile(path, data);
          return { path, width: info.width, height: info.height };
        })();
        encoded.set(digest, image);
      }
      const job = image.then(async result => {
        if (result.path !== path) {
          await mkdir(join(path, '..'), { recursive: true });
          await copyFile(result.path, path);
        }
        return { ...shot, width: result.width, height: result.height, relative: webp };
      });
      job.catch(() => {});
      batch.push(job);
      if (batch.length === concurrency) {
        await flush();
        if (images.length % 500 < concurrency) log(`optimized ${images.length} screenshots`);
      }
    }
    await flush();
    await completed;
    return images;
  } finally {
    if (child.exitCode === null) child.kill();
    await Promise.allSettled(batch);
  }
}

export async function loadReleaseGallery(app, log = () => {}, { directory = root, request = fetch } = {}) {
  const started = performance.now();
  const { release, index, archive } = await latestRelease(app, request);
  const tag = release.tag_name;
  // Asset identities change even if a maintainer replaces an asset on the same tag.
  const identity = releaseIdentity({ index, archive });
  const cache = join(directory, '.cache', 'release-gallery', app.id, tag, identity);
  await mkdir(cache, { recursive: true });
  const indexPath = join(cache, 'gallery.json');
  await download(index, indexPath, request);
  const data = JSON.parse(await readFile(indexPath, 'utf8'));
  if (!Array.isArray(data.screenshots) || !data.screenshots.length) throw new Error(`${app.id} ${tag}: empty screenshot index`);
  const selected = selectCaptures(app, data.screenshots);
  const key = shot => JSON.stringify([shot.platform, shot.device, shot.variant, shot.file]);
  const manifest = join(cache, 'renditions.json');
  const saved = await readFile(manifest, 'utf8').then(JSON.parse).catch(() => null);
  const cached = Array.isArray(saved) ? saved : [];
  const byPath = new Map(cached.map(shot => [key(shot), shot]));
  const usable = await Promise.all(selected.map(async shot => {
    const image = byPath.get(key(shot));
    return image && await stat(join(cache, image.relative)).then(f => f.size > 0, () => false) ? image : null;
  }));
  const missing = selected.filter((_, i) => !usable[i]);
  if (missing.length) {
    log(`${app.id}: ${tag}: optimizing ${missing.length} missing captures (${selected.length} selected from ${data.screenshots.length})`);
    const archivePath = join(cache, archive.name);
    await download(archive, archivePath, request);
    const selectionPath = join(cache, 'selected.json');
    await writeFile(selectionPath, JSON.stringify({ screenshots: missing }));
    const generated = await encodeArchive(archivePath, selectionPath, cache, message => log(`${app.id}: ${message}`));
    if (generated.length !== missing.length) throw new Error(`${app.id} ${tag}: incomplete release archive`);
    for (const shot of generated) byPath.set(key(shot), shot);
    const temporary = `${manifest}.${process.pid}.tmp`;
    await writeFile(temporary, JSON.stringify([...byPath.values()]));
    await rename(temporary, manifest);
  }
  // Keep archive order, including when replacing a damaged rendition. The gallery's
  // first-wins device selection must not change just because a build restores a cache.
  const selectedKeys = new Set(selected.map(key));
  const images = [...byPath.values()].filter(shot => selectedKeys.has(key(shot)));
  log(`${app.id}: ${images.length} selected, ${images.length - missing.length} cached, ${missing.length} encoded in ${((performance.now() - started) / 1000).toFixed(1)}s`);
  const prefix = `gallery-media/${app.id}/${tag}/${identity.slice(0, 16)}-dedup`;
  const copied = new Set();
  const published = [];
  for (const { relative, ...shot } of images) {
    // Some release variants are byte-identical. One content-addressed public image
    // avoids shipping (and downloading) the same capture for every locale or screen.
    const bytes = await readFile(join(cache, relative));
    const filename = `${hash(bytes)}.webp`;
    const dest = join(directory, 'public', prefix, filename);
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
