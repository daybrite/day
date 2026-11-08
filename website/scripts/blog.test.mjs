// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import assert from 'node:assert/strict';
import test from 'node:test';
import { parse } from 'parse5';
import { isDraftPost, newestPostsFirst } from '../src/lib/blog-policy.mjs';
import { withFeedDiscovery } from '../integrations/blog.mjs';
import { feedContent } from '../src/lib/blog-feed.mjs';

// Synthetic entries exercise folder-based privacy even when a draft overrides its slug.
const post = (id, data = {}, filePath) => ({ id, filePath, data: { pubDate: new Date('2026-01-01'), ...data } });

test('draft folder and frontmatter both prevent publication', () => {
  assert.equal(isDraftPost(post('drafts/private')), true);
  assert.equal(isDraftPost(post('announcements/drafts/private')), true);
  assert.equal(isDraftPost(post('public-looking-slug', { draft: false }, 'src/content/blog/drafts/private.md')), true);
  assert.equal(isDraftPost(post('private', { draft: true })), true);
  assert.equal(isDraftPost(post('draftsmanship', { draft: false })), false);
});

test('posts sort newest first with stable ordering for a shared publication date', () => {
  const entries = [post('b'), post('new', { pubDate: new Date('2026-02-01') }), post('a')];
  assert.deepEqual(entries.sort(newestPostsFirst).map((entry) => entry.id), ['new', 'a', 'b']);
});

test('RSS discovery covers copied HTML and implicit heads without duplicate links', () => {
  for (const html of ['<!doctype html><html><head><title>Fixture</title></head><body>Fixture</body></html>',
    '<!doctype html><html><body>Fixture</body></html>', '<!doctype html><p>Fixture</p>']) {
    const output = withFeedDiscovery(html, '/preview/rss.xml', 'Fixture & "feed"');
    assert.equal(withFeedDiscovery(output, '/preview/rss.xml', 'Fixture & "feed"'), output);
    const document = parse(output);
    const root = document.childNodes.find((node) => node.nodeName === 'html');
    const head = root.childNodes.find((node) => node.nodeName === 'head');
    const links = head.childNodes.filter((node) => node.nodeName === 'link');
    assert.equal(links.length, 1);
    const attributes = Object.fromEntries(links[0].attrs.map(({ name, value }) => [name, value]));
    assert.equal(attributes.href, '/preview/rss.xml');
    assert.equal(attributes.title, 'Fixture & "feed"');
    assert.equal(attributes.rel, 'alternate');
    assert.equal(attributes.type, 'application/rss+xml');
  }
});

test('another feed does not suppress discovery of the Day feed', () => {
  const html = '<html><head><link rel="alternate" type="application/rss+xml" href="/other.xml"></head></html>';
  const output = withFeedDiscovery(html, '/rss.xml', 'Synthetic feed fixture');
  assert.match(output, /href="\/rss.xml"/);
  assert.match(output, /href="\/other.xml"/);
});

test('full articles retain formatting and resolve links and images for feed readers', () => {
  // Synthetic article HTML, including the static output of an MDX component.
  const output = feedContent(`<h2 id="example">Example</h2>
    <p>A <strong>complete</strong> article &amp; its links.</p>
    <ul><li>First</li><li>Second</li></ul><pre><code>&lt;Widget /&gt;</code></pre>
    <table><tr><th>Platform</th><td>Native</td></tr></table>
    <astro-island component-url="/component.js"><p>Rendered component content</p></astro-island>
    <a href="/docs/getting-started">Docs</a><a href="../earlier/">Earlier</a>
    <a href="#example">Section</a><a href="https://example.org/">External</a>
    <img src="./diagram.png" alt="Synthetic diagram" width="640">
    <img src="/_astro/optimized.webp" alt="Optimized image">
    <details><summary>More</summary><p>Details</p></details>`,
  'https://daybrite.dev/blog/example/');
  assert.match(output, /<h2 id="example">Example<\/h2>/);
  assert.match(output, /<strong>complete<\/strong>/);
  assert.match(output, /<ul><li>First<\/li><li>Second<\/li><\/ul>/);
  assert.match(output, /<pre><code>&lt;Widget \/&gt;<\/code><\/pre>/);
  assert.match(output, /<th>Platform<\/th><td>Native<\/td>/);
  assert.match(output, /<p>Rendered component content<\/p>/);
  assert.match(output, /href="https:\/\/daybrite.dev\/docs\/getting-started"/);
  assert.match(output, /href="https:\/\/daybrite.dev\/blog\/earlier\/"/);
  assert.match(output, /href="https:\/\/daybrite.dev\/blog\/example\/#example"/);
  assert.match(output, /href="https:\/\/example.org\/"/);
  assert.match(output, /src="https:\/\/daybrite.dev\/blog\/example\/diagram.png" alt="Synthetic diagram" width="640"/);
  assert.match(output, /src="https:\/\/daybrite.dev\/_astro\/optimized.webp"/);
  assert.match(output, /<details><summary>More<\/summary><p>Details<\/p><\/details>/);
  assert.doesNotMatch(output, /astro-island|component-url/);
});

test('feed HTML removes active content and site styling', () => {
  const output = feedContent(`<script>alert('fixture')</script><style>p { display: none }</style>
    <p class="fixture" style="display:none" onclick="alert('fixture')">Readable</p>
    <a href="javascript:alert('fixture')">Unsafe link</a>
    <img src="javascript:alert('fixture')" onerror="alert('fixture')" alt="Fixture">`,
  'https://daybrite.dev/blog/example/');
  assert.match(output, /<p>Readable<\/p>/);
  assert.doesNotMatch(output, /script|style|class=|onclick|onerror|javascript:|alert\(/);
});
