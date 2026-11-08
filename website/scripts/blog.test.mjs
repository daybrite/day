// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import assert from 'node:assert/strict';
import test from 'node:test';
import { parse } from 'parse5';
import { isDraftPost, newestPostsFirst } from '../src/lib/blog-policy.mjs';
import { withFeedDiscovery } from '../integrations/blog.mjs';

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
