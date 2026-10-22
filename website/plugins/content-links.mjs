// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { posix } from 'node:path';
import { routes, canonicalPath } from '../src/lib/content-routes.mjs';

const bySource = new Map(Object.values(routes).map((page) => [page.source, page.route]));

/** Keep repository Markdown links usable both in GitHub and at their public route. */
export function resolveContentLink(href, source) {
  if (/^(https?:|\/)/.test(href)) return canonicalPath(href);
  if (/^(?:[a-z][a-z0-9+.-]*:|#)/i.test(href) || !source) return href;
  const [, bare, suffix] = /^([^?#]*)(.*)$/.exec(href);
  const target = posix.normalize(posix.join(posix.dirname(source), bare));
  if (bySource.has(target)) return canonicalPath(bySource.get(target) + suffix);
  // Source documents are preserved beside code. Files link to blob; directories to tree.
  const kind = posix.extname(target) || /(?:^|\/)(LICENSE|AGENTS\.md)$/.test(target) ? 'blob' : 'tree';
  return `https://github.com/daybrite/day/${kind}/main/${target}${suffix}`;
}

export default function contentLinks() {
  return (tree, file) => {
    const path = (file?.path || file?.history?.[0] || '').replace(/\\/g, '/');
    const match = /\/content\/(docs|internal|repository)\/(.+)\.(?:md|mdx)$/.exec(path);
    const meta = match ? routes[`${match[1]}/${match[2]}`] : Object.values(routes).find((page) => path.endsWith('/' + page.source));
    const visit = (node) => {
      if (node.type === 'element' && node.tagName === 'a' && typeof node.properties?.href === 'string') {
        node.properties.href = resolveContentLink(node.properties.href, meta?.source);
      }
      if (Array.isArray(node.children)) node.children.forEach(visit);
    };
    visit(tree);
  };
}
