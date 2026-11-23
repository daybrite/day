// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { latestRelease } from './release-gallery.mjs';
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
