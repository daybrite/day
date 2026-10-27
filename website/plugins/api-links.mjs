// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

/** Distinguish links to Rust signatures from links to explanatory guides. */
export default function apiLinks() {
  return tree => {
    function visit(node) {
      if (node.type === 'element' && ['pre', 'code'].includes(node.tagName)) return;
      if (node.type === 'element' && node.tagName === 'a') {
        const href = node.properties?.href;
        // Every API link goes to docs.rs (scripts/docs/check-api-links.py checks them).
        if (typeof href === 'string' && /^https:\/\/docs\.rs\/[^/]+\/[^/]+\/[^?#]+\.html(?:[?#]|$)/.test(href)) {
          node.properties.className = [...[node.properties.className ?? []].flat(), 'api-link'];
          node.properties.title ??= 'Rust API reference';
        }
      }
      for (const child of node.children ?? []) visit(child);
    }
    visit(tree);
  };
}
