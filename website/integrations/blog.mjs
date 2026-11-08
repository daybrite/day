// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { parse } from 'parse5';
import { generateSiteStrings } from '../scripts/generate-site-strings.mjs';

const escapeAttribute = (text) => text.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;');

/** Copied HTML, including the embedded demo, does not pass through BaseLayout. */
export function withFeedDiscovery(html, href, title) {
  const document = parse(html, { sourceCodeLocationInfo: true });
  const root = document.childNodes.find((node) => node.nodeName === 'html');
  const head = root?.childNodes.find((node) => node.nodeName === 'head');
  if (head?.childNodes.some((node) => node.nodeName === 'link' &&
    node.attrs.some((attr) => attr.name === 'type' && attr.value === 'application/rss+xml') &&
    node.attrs.some((attr) => attr.name === 'href' && attr.value === href))) return html;
  const link = `<link rel="alternate" type="application/rss+xml" title="${escapeAttribute(title)}" href="${escapeAttribute(href)}">`;
  const offset = head?.sourceCodeLocation?.startTag.endOffset;
  if (offset !== undefined) return html.slice(0, offset) + link + html.slice(offset);
  // Static fragments with an implicit head still need a discoverable feed.
  const start = root?.sourceCodeLocation?.startTag.endOffset ?? html.match(/^\s*<!doctype[^>]*>/i)?.[0].length ?? 0;
  return html.slice(0, start) + `<head>${link}</head>` + html.slice(start);
}

async function addFeedLinks(directory, href, title) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = new URL(entry.name + (entry.isDirectory() ? '/' : ''), directory);
    if (entry.isDirectory()) await addFeedLinks(path, href, title);
    else if (entry.name.endsWith('.html')) {
      const html = await readFile(path, 'utf8');
      const next = withFeedDiscovery(html, href, title);
      if (next !== html) await writeFile(path, next);
    }
  }
}

/** @returns {import('astro').AstroIntegration} */
export default function blog() {
  let feedHref;
  return {
    name: 'day-blog',
    hooks: {
      'astro:config:setup': async () => { await generateSiteStrings(); },
      'astro:config:done': ({ config }) => { feedHref = `${config.base.replace(/\/$/, '')}/rss.xml`; },
      'astro:build:done': async ({ dir }) => {
        const { res } = await import('../src/generated/strings.mjs');
        await addFeedLinks(dir, feedHref, res.str.blog_feed_title());
      },
    },
  };
}
