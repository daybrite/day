// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createMarkdownProcessor } from '@astrojs/markdown-remark';
import apiLinks from '../plugins/api-links.mjs';
import contentLinks from '../plugins/content-links.mjs';

const link = (href, title) => ({ type: 'element', tagName: 'a',
  properties: { href, ...(title ? { title } : {}) },
  children: [{ type: 'element', tagName: 'code', properties: {},
    children: [{ type: 'text', value: '.on_submit(f)' }] }] });

test('API links retain the exact method destination and readable label', () => {
  for (const origin of ['', 'https://daybrite.dev']) {
    const href = `${origin}/api/day_pieces/trait.TextAreaBuilder.html#tymethod.on_submit`;
    const node = link(href);
    apiLinks()(node);
    assert.equal(node.properties.href, `https://daybrite.dev/api/day_pieces/trait.TextAreaBuilder.html#tymethod.on_submit`);
    assert.equal(node.children[0].children[0].value, '.on_submit(f)');
    assert.deepEqual(node.properties.className, ['api-link']);
    assert.equal(node.properties.title, 'Rust API reference');
    assert.equal(node.children.length, 1);
  }
});

test('guides, other origins, and code samples are not restyled as Day API links', () => {
  for (const href of ['/docs/reference/textarea', '/api/',
    'https://other.example/api/day_pieces/fn.text_area.html']) {
    const node = link(href), before = structuredClone(node);
    apiLinks()(node);
    assert.deepEqual(node, before);
  }
  const sample = { type: 'element', tagName: 'pre', properties: {},
    children: [link('/api/day_pieces/fn.text_area.html')] };
  const before = structuredClone(sample);
  apiLinks()(sample);
  assert.deepEqual(sample, before);
});

test('author-supplied titles and classes survive', () => {
  const node = link('/api/day_pieces/fn.text_area.html', 'Text area constructor');
  node.properties.className = ['existing'];
  apiLinks()(node);
  assert.equal(node.properties.title, 'Text area constructor');
  assert.deepEqual(node.properties.className, ['existing', 'api-link']);
});


test('API links keep heading anchors and text without adding indicators', async () => {
  const processor = await createMarkdownProcessor({ rehypePlugins: [contentLinks, apiLinks] });
  const result = await processor.render('## Sections: [`nav`](/api/day_pieces/fn.nav.html)');
  assert.equal(result.metadata.headings[0].slug, 'sections-nav');
  assert.match(result.code, /href="https:\/\/daybrite.dev\/api\/day_pieces\/fn.nav.html"/);
  assert.doesNotMatch(result.code, /api-link-mark/);
  assert.doesNotMatch(result.code, />API</);
});
