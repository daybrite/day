// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Only links explicitly marked with title="toolkit:<target-id>" receive platform styling.
// Ordinary words, links, headings, and code are never inferred to be toolkit labels.
import { readFileSync } from 'node:fs';
import { toolkitLabel } from '../src/lib/toolkit-labels.mjs';

const icons = new Map();
function iconStyle(id) {
  if (!icons.has(id)) {
    const svg = readFileSync(new URL(`../src/icons/${id}.svg`, import.meta.url), 'utf8');
    icons.set(id, `--target-icon: url("data:image/svg+xml,${encodeURIComponent(svg)}")`);
  }
  return icons.get(id);
}

export default function explicitToolkitLabels() {
  return (tree, file) => {
    function visit(node) {
      if (['pre', 'script', 'style', 'svg'].includes(node.tagName)) return;
      const title = node.properties?.title;
      if (node.tagName === 'a' && typeof title === 'string' && title.startsWith('toolkit:')) {
        let label;
        try { label = toolkitLabel(title.slice('toolkit:'.length)); }
        catch (error) {
          if (file?.fail) file.fail(error.message, node);
          throw error;
        }
        delete node.properties.title;
        node.children = [{
          type: 'element', tagName: 'span',
          properties: { className: ['doc-target-label'], ...label.attributes },
          children: [
            ...label.marks.map(({ id, icon }) => ({
              type: 'element', tagName: 'span',
              properties: { className: ['doc-target-icon'], ariaHidden: 'true', 'data-pf': id, style: iconStyle(icon) },
              children: [],
            })),
            ...node.children,
          ],
        }];
        return;
      }
      node.children?.forEach(visit);
    }
    visit(tree);
  };
}
