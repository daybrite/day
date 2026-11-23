// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { latestRelease, releaseIdentity, loadReleaseGallery, encodeArchive } from './release-gallery.mjs';
import { selectCaptures } from './gallery-selection.mjs';
import { galleryCacheKey, pruneGalleryCache } from './release-gallery-cache.mjs';
import { mkdtemp, readFile, writeFile, rm, mkdir, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import sharp from 'sharp';
import { pagePath } from '../src/lib/page-path.mjs';

// Synthetic GitHub API fixtures; no network calls or real image bytes.
const app = { id: 'Fixture', repo: 'https://github.com/example/Fixture' };
const asset = name => ({ name, digest: `sha256:${'a'.repeat(64)}`, browser_download_url: `${app.repo}/releases/download/v1.0.0/${name}` });
const release = { tag_name: 'v1.0.0', draft: false, prerelease: false, published_at: '2026-01-01T00:00:00Z', assets: [asset('gallery.json'), asset('screenshots.tar.xz')] };
const response = data => async url => {
  assert.equal(url, 'https://api.github.com/repos/example/Fixture/releases/latest');
  return { ok: true, json: async () => data };
};
test('GitHub latest full release supplies both assets, including legacy ZIPs', async () => {
  assert.equal((await latestRelease(app, response(release))).release.tag_name, 'v1.0.0');
  assert.equal((await latestRelease(app, response({ ...release, assets: [asset('gallery.json'), asset('screenshots.zip')] }))).archive.name, 'screenshots.zip');
});
test('drafts, prereleases, unpublished releases and incomplete assets never fall back', async () => {
  for (const change of [{ draft: true }, { prerelease: true }, { published_at: null }, { assets: [] }, { assets: [asset('gallery.json'), { ...asset('screenshots.zip'), digest: null }] }]) {
    await assert.rejects(latestRelease(app, response({ ...release, ...change })));
  }
  await assert.rejects(latestRelease(app, async () => ({ ok: false, status: 403 })), /HTTP 403/);
});
test('public page URLs avoid directory redirects without changing files or fragments', () => {
  for (const [before, after] of [
    ['/docs', '/docs/'], ['/gallery/Day-Travel/', '/gallery/Day-Travel/'],
    ['/docs/platforms?q=1#support', '/docs/reference/platforms/?q=1#support'],
    ['/api/day/', '/api/day/'], ['/api/day/index.html#item', '/api/day/index.html#item'],
    ['/rss.xml', '/rss.xml'], ['/gallery-media/v0.2.0/paris.webp', '/gallery-media/v0.2.0/paris.webp'],
    ['/', '/'], ['#section', '#section'], ['https://example.org/path', 'https://example.org/path'],
  ]) assert.equal(pagePath(before), after);
});
test('legacy Android spelling retains its platform shell; Windows XAML remains distinct', async () => {
  const { assembleApp } = await import('./assemble-gallery.mjs');
  const data = {
    platforms: ['android-mdc', 'windows-xaml'], shots: [{ id: 'home' }],
    screenshots: ['android-mdc', 'windows-xaml'].map(platform => ({ platform, shot: 'home', url: '/fixture.webp', width: 360, height: 720 })),
  };
  const result = assembleApp(app, data, false, { tag: 'v1.0.0', channel: 'release' });
  assert.deepEqual(result.columns.map(c => c.id), ['android-material', 'windows-xaml']);
  assert.equal(result.release.tag, 'v1.0.0');
  assert.equal(result.fromMain, false);
});

test('selection preserves visible theme/locale/device choices and archive order', () => {
  const shot = { shot: 'home', platform: 'ios-uikit', theme: 'light', locale: 'en' };
  const screenshots = [shot, { ...shot, device: 'unknown-phone' }, { ...shot, device: 'ipad' },
    { ...shot, theme: 'dark' }, { ...shot, locale: 'fr' }, { ...shot, shot: 'hidden' },
    { ...shot, platform: 'android-mdc' }, { ...shot, platform: 'windows-xaml' }];
  assert.deepEqual(selectCaptures({ hide: ['hidden'], platforms: ['ios-uikit', 'ios-uikit-ipad', 'android-material'] }, screenshots),
    [screenshots[0], screenshots[1], screenshots[2], screenshots[3], screenshots[4], screenshots[6]]);
  const tablet = { ...shot, platform: 'harmony-arkui', device: 'tablet' };
  const phone = { ...tablet, device: 'phone' };
  assert.deepEqual(selectCaptures({}, [tablet, phone]), [tablet, phone]);
  assert.deepEqual(selectCaptures({}, [phone, tablet]), [phone, tablet]);
});

test('CI cache keys follow asset replacement, release tags and policy changes', () => {
  const assets = { release, index: release.assets[0], archive: release.assets[1] };
  const key = (value = assets, policy = 'policy') => galleryCacheKey([{ app, assets: value }], policy);
  assert.equal(key(), key(structuredClone(assets)));
  for (const field of ['index', 'archive']) {
    const replaced = { ...assets, [field]: { ...assets[field], digest: `sha256:${'b'.repeat(64)}` } };
    assert.notEqual(key(), key(replaced));
    assert.notEqual(releaseIdentity(assets), releaseIdentity(replaced));
  }
  assert.notEqual(key(), key({ ...assets, release: { ...release, tag_name: 'v2.0.0' } }));
  assert.notEqual(key(), key(assets, 'changed-policy'));
});

async function imageFixture(t, reverseArchive = false) {
  const dir = await mkdtemp(join(tmpdir(), 'day-gallery-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const png = await sharp({ create: { width: 12, height: 16, channels: 3, background: '#1469a8' } }).png().toBuffer();
  const digest = bytes => `sha256:${createHash('sha256').update(bytes).digest('hex')}`;
  const screenshots = ['home', 'detail', 'hidden'].map(shot => ({ shot, platform: 'ios-uikit', variant: 'light-en', theme: 'light', locale: 'en', file: `${shot}.png`, width: 12, height: 16, sha256: digest(png).slice(7) }));
  const index = Buffer.from(JSON.stringify({ platforms: ['ios-uikit'], screenshots }));
  await writeFile(join(dir, 'source.png'), png);
  await writeFile(join(dir, 'gallery.json'), index);
  const archivePath = join(dir, 'screenshots.zip');
  execFileSync('python3', ['-c', 'import json,sys,zipfile\nfrom pathlib import Path\np=Path(sys.argv[1])\nshots=json.loads((p/"gallery.json").read_text())["screenshots"]\nif sys.argv[2]=="true": shots.reverse()\nwith zipfile.ZipFile(p/"screenshots.zip","w") as z:\n for s in shots:\n  z.writestr(s["platform"]+"/"+s["variant"]+"/"+s["file"],(p/"source.png").read_bytes())', dir, String(reverseArchive)]);
  const zip = await readFile(archivePath);
  const assets = { release: { ...release, assets: [
    { ...asset('gallery.json'), digest: digest(index) },
    { ...asset('screenshots.zip'), digest: digest(zip) },
  ] } };
  assets.index = assets.release.assets[0]; assets.archive = assets.release.assets[1];
  const requests = [];
  const request = async url => {
    requests.push(url);
    return new Response(url.endsWith('/releases/latest') ? JSON.stringify(assets.release) : url.endsWith('gallery.json') ? index : zip);
  };
  return { dir, archivePath, assets, requests, request };
}

test('warm cache needs no archive; selection expansion and missing files regenerate only missing images', async t => {
  const f = await imageFixture(t);
  const logs = [];
  const options = { directory: join(f.dir, 'website'), request: f.request };
  const selectedApp = { ...app, hide: ['detail', 'hidden'] };
  const cold = await loadReleaseGallery(selectedApp, s => logs.push(s), options);
  assert.equal(cold.data.screenshots.length, 1);
  const cache = join(options.directory, '.cache/release-gallery/Fixture/v1.0.0', releaseIdentity(f.assets));
  await rm(join(cache, 'screenshots.zip'));
  f.requests.length = 0;
  assert.deepEqual((await loadReleaseGallery(selectedApp, s => logs.push(s), options)).data, cold.data);
  assert.deepEqual(f.requests, ['https://api.github.com/repos/example/Fixture/releases/latest']);
  assert.match(logs.at(-1), /1 cached, 0 encoded/);
  const expanded = await loadReleaseGallery({ ...app, hide: ['hidden'] }, s => logs.push(s), options);
  assert.equal(expanded.data.screenshots.length, 2);
  assert.match(logs.at(-1), /1 cached, 1 encoded/);
  await rm(join(cache, 'ios-uikit/light-en/home.webp'));
  await loadReleaseGallery({ ...app, hide: ['hidden'] }, s => logs.push(s), options);
  assert.match(logs.at(-1), /1 cached, 1 encoded/);
  f.assets.release.tag_name = 'v2.0.0';
  for (const asset of f.assets.release.assets) asset.browser_download_url = asset.browser_download_url.replace('v1.0.0', 'v2.0.0');
  const updated = await loadReleaseGallery(selectedApp, s => logs.push(s), options);
  assert.match(logs.at(-1), /0 cached, 1 encoded/);
  assert.match(updated.data.screenshots[0].url, /\/v2\.0\.0\//);
  await mkdir(join(options.directory, 'src/data'), { recursive: true });
  await writeFile(join(options.directory, 'src/data/gallery-manifest.json'), JSON.stringify({ apps: [{ shots: [{ byColumn: [{ captures: { light: { src: updated.data.screenshots[0].url } } }] }] }] }));
  await pruneGalleryCache(options.directory, [selectedApp]);
  await assert.rejects(stat(cache), { code: 'ENOENT' });
  await loadReleaseGallery(selectedApp, s => logs.push(s), options);
  assert.match(logs.at(-1), /1 cached, 0 encoded/);
  f.assets.release.prerelease = true;
  await assert.rejects(loadReleaseGallery(selectedApp, () => {}, options), /not a published full release/);
});

test('bounded parallel encoding matches serial bytes, order and duplicate content', async t => {
  const f = await imageFixture(t);
  const results = [];
  for (const concurrency of [1, 4]) {
    const dir = join(f.dir, `encode-${concurrency}`);
    await mkdir(dir);
    const images = await encodeArchive(f.archivePath, join(f.dir, 'gallery.json'), dir, () => {}, concurrency);
    results.push(await Promise.all(images.map(async image => ({ ...image, bytes: (await readFile(join(dir, image.relative))).toString('base64') }))));
  }
  assert.deepEqual(results[0], results[1]);
  assert.equal(new Set(results[0].map(image => image.bytes)).size, 1);
});

test('cold, warm and repaired caches preserve archive order when the index differs', async t => {
  const f = await imageFixture(t, true);
  const options = { directory: join(f.dir, 'website'), request: f.request };
  const expected = ['hidden', 'detail', 'home'];
  const load = async () => (await loadReleaseGallery(app, () => {}, options)).data.screenshots.map(s => s.shot);
  assert.deepEqual(await load(), expected);
  assert.deepEqual(await load(), expected);
  await rm(join(options.directory, '.cache/release-gallery/Fixture/v1.0.0', releaseIdentity(f.assets), 'ios-uikit/light-en/detail.webp'));
  assert.deepEqual(await load(), expected);
});
test('final Markdown URLs preserve glossary popovers and tier badges', async () => {
  const { createMarkdownProcessor } = await import('@astrojs/markdown-remark');
  const { default: contentLinks, pageLinks } = await import('../plugins/content-links.mjs');
  const { default: glossary } = await import('../plugins/glossary-links.mjs');
  const { default: tiers } = await import('../plugins/tier-badge.mjs');
  const processor = await createMarkdownProcessor({ rehypePlugins: [contentLinks, tiers, glossary, pageLinks] });
  const { code } = await processor.render('[piece](/docs/reference/glossary#piece) [Tier 1](/docs/platforms#support-tiers)');
  assert.match(code, /class="gloss"/);
  assert.match(code, /class="tier-badge"/);
  assert.match(code, /href="\/docs\/reference\/glossary\/#piece"/);
  assert.match(code, /href="\/docs\/reference\/platforms\/#support-tiers"/);
  assert.doesNotMatch(code, /href="\/[^"#?]*[^/]#/);
});
