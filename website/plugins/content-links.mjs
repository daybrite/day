// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { posix } from 'node:path';
import { routes, canonicalPath, isUnpublishedPath } from '../src/lib/content-routes.mjs';

const bySource = new Map(Object.values(routes).map((page) => [page.source, page.route]));

/** Keep repository Markdown links usable both in GitHub and at their public route. */
export function resolveContentLink(href, source) {
  // Local previews use the published Rustdoc, independently of the local API bundle.
  if (/^(?:https:\/\/daybrite\.dev)?\/api(?:[/?#]|$)/.test(href)) {
    return new URL(href, 'https://daybrite.dev').href;
  }
  if (isUnpublishedPath(href)) return null;
  if (/^(https?:|\/)/.test(href)) return canonicalPath(href);
  if (/^(?:[a-z][a-z0-9+.-]*:|#)/i.test(href) || !source) return href;
  const [, bare, suffix] = /^([^?#]*)(.*)$/.exec(href);
  const target = posix.normalize(posix.join(posix.dirname(source), bare));
  if (bySource.has(target)) {
    const route = canonicalPath(bySource.get(target) + suffix);
    return isUnpublishedPath(route) ? null : route;
  }
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
        const destination = resolveContentLink(node.properties.href, meta?.source);
        if (destination === null) {
          node.tagName = 'span';
          node.properties = {};
        } else node.properties.href = destination;
      }
      if (Array.isArray(node.children)) node.children.forEach(visit);
    };
    visit(tree);
  };
}
