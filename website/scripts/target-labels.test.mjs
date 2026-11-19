// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createMarkdownProcessor } from '@astrojs/markdown-remark';
import explicitToolkitLabels from '../plugins/accent-target-code.mjs';
import { toolkitLabel } from '../src/lib/toolkit-labels.mjs';
import contentLinks from '../plugins/content-links.mjs';
import admonitions from '../plugins/admonitions.mjs';
const text = value => ({ type: 'text', value });
const element = (tagName, ...children) => ({ type: 'element', tagName, properties: {}, children });
const nodes = node => [node, ...(node.children ?? []).flatMap(nodes)];
const labels = node => nodes(node).filter(n => n.properties?.className?.includes('doc-target-label'));
const marked = (id, name) => ({ ...element('a', text(name)), properties: { href: '/docs/reference/platforms', title: `toolkit:${id}` } });

test('ordinary words and target IDs remain untouched in every prose context', () => {
  for (const tag of ['h2', 'h3', 'p', 'strong', 'th', 'td', 'li', 'code']) {
    const tree = element('div', element('h2', text('Toolkit implementations')),
      element(tag, text('web page, web shells, browser permissions, material, DOM, AppKit, GTK, web-dom')));
    const original = structuredClone(tree);
    explicitToolkitLabels()(tree);
    assert.deepEqual(tree, original);
  }
  const link = { ...element('a', text('Web')), properties: { href: '/docs/reference/platforms/web-dom', title: 'Web guide' } };
  const original = structuredClone(link);
  explicitToolkitLabels()(link);
  assert.deepEqual(link, original);
});

test('explicit labels preserve links, formatting and accessible text and are idempotent', () => {
  const link = marked('macos-gtk', 'GTK');
  link.children = [element('code', text('macos-gtk'))];
  link.properties.className = ['existing'];
  explicitToolkitLabels()(link);
  assert.equal(link.properties.href, '/docs/reference/platforms');
  assert.equal(link.properties.title, undefined);
  assert.deepEqual(link.properties.className, ['existing']);
  assert.equal(labels(link)[0].properties['data-pf'], 'macos-gtk');
  const icon = labels(link)[0].children[0];
  assert.equal(icon.properties.ariaHidden, 'true');
  assert.match(icon.properties.style, /data:image\/svg\+xml/);
  assert.equal(labels(link)[0].children[1].tagName, 'code');
  const once = structuredClone(link);
  explicitToolkitLabels()(link);
  assert.deepEqual(link, once);
});

test('OS-family labels retain both marks; component defaults come from the target catalog', () => {
  const link = marked('linux-gtk,linux-qt', 'Linux');
  explicitToolkitLabels()(link);
  assert.equal(labels(link)[0].properties['data-targets'], 'linux-gtk linux-qt');
  assert.equal(labels(link)[0].children.filter(n => n.properties?.ariaHidden === 'true').length, 2);
  assert.equal(toolkitLabel('web-dom').name, 'Web DOM');
  assert.equal(toolkitLabel('macos-qt').marks[0].icon, 'linux-qt');
});

test('missing, duplicate, capture-only and unknown IDs fail instead of silently omitting styling', () => {
  for (const id of [undefined, '', 'web', 'web-dmo', 'ios-uikit-ipad', 'linux-gtk,linux-gtk', 'linux-gtk,linux-qt,web-dom']) {
    assert.throws(() => toolkitLabel(id), /Invalid toolkit label ID/);
    assert.throws(() => explicitToolkitLabels()(marked(id ?? '', 'Synthetic invalid label')), /Invalid toolkit label ID/);
  }
});

test('Markdown labels keep heading anchors, code samples, link destinations and warning severity', async () => {
  const processor = await createMarkdownProcessor({ rehypePlugins: [contentLinks, explicitToolkitLabels, admonitions] });
  const source = '## Web shells\n\nA web page uses browser permissions.\n\n' +
    '## [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom")\n\n' +
    '| Target | Notes |\n| --- | --- |\n| [`macos-gtk`](/docs/reference/platforms "toolkit:macos-gtk") | material and DOM |\n\n' +
    '> [!WARNING]\n> **[WinUI](/docs/reference/platforms/windows-winui "toolkit:windows-winui"): alpha selection.**\n> Synthetic warning.\n\n' +
    '```md\n[Web](/docs/reference/platforms/web-dom "toolkit:web-dom")\n```';
  const result = await processor.render(source);
  assert.deepEqual(result.metadata.headings.map(h => h.slug), ['web-shells', 'web']);
  assert.match(result.code, /<h2 id="web-shells">Web shells<\/h2>/);
  assert.match(result.code, /<p>A web page uses browser permissions\.<\/p>/);
  assert.match(result.code, /href="\/docs\/reference\/platforms\/web-dom"/);
  assert.equal((result.code.match(/class="doc-target-label"/g) ?? []).length, 3);
  assert.match(result.code, /data-kind="warning"/);
  assert.match(result.code, /id="admonition-winui-alpha-selection"/);
  await assert.rejects(processor.render('[Web](/docs/reference/platforms "toolkit:web-typo")'), /Invalid toolkit label ID/);
});
