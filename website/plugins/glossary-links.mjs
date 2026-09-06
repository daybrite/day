// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Rehype plugin — turn links into the glossary into defined terms.
//
// Wherever a docs page links a word to its glossary entry (`[piece](/docs/glossary#piece)`), stamp
// the anchor with `class="gloss"` and the entry's definition and source page as data attributes.
// global.css draws the light dotted underline, and DocsLayout's script opens a small popover on
// hover or focus with the definition and a link to the page that introduces the concept. The
// markdown stays a plain link: on GitHub, in an editor, or with scripts off, it is still a link
// to the glossary, and a term the glossary stops carrying simply renders as an ordinary link.
//
// The href only has to END with `/docs/glossary#<id>`, so the internal reference docs can use the
// absolute https://daybrite.dev/… form and still get the treatment on the site.

import { glossary } from '../src/lib/glossary.mjs';

const byId = new Map(glossary.map((t) => [t.id, t]));
const PATTERN = /\/docs\/glossary#([a-z0-9-]+)$/;

export default function glossaryLinks() {
  return (tree) => {
    const visit = (node) => {
      if (
        node.type === 'element' &&
        node.tagName === 'a' &&
        typeof node.properties?.href === 'string'
      ) {
        const match = PATTERN.exec(node.properties.href);
        const term = match && byId.get(match[1]);
        if (term) {
          node.properties.className = [...[node.properties.className ?? []].flat(), 'gloss'];
          node.properties['data-term'] = term.term;
          node.properties['data-def'] = term.definition;
          node.properties['data-see'] = term.see.href;
          node.properties['data-see-title'] = term.see.title;
        }
      }
      if (Array.isArray(node.children)) node.children.forEach(visit);
    };
    visit(tree);
  };
}
