// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Rehype plugin — turn links into the glossary into defined terms.
//
// Wherever a docs page links a word to its glossary entry (`[piece](/docs/reference/glossary#piece)`), stamp
// the anchor with `class="gloss"` and render its definition beside it at build time.
// global.css draws the light dotted underline, and DocsLayout's script opens a small popover on
// hover or focus with the definition and a link to the page that introduces the concept. The
// markdown stays a plain link: on GitHub, in an editor, or with scripts off, it is still a link
// to the glossary, and a term the glossary stops carrying simply renders as an ordinary link.
//
// The href only has to END with `/docs/reference/glossary#<id>`, so the internal reference docs can use the
// absolute https://daybrite.dev/… form and still get the treatment on the site.

import { res } from '../src/generated/strings.mjs';
import { glossary } from '../src/lib/glossary.mjs';

const byId = new Map(glossary.map((t) => [t.id, t]));
const PATTERN = /\/docs\/reference\/glossary#([a-z0-9-]+)$/;

const text = value => ({ type: 'text', value });
const element = (tagName, properties, children) => ({ type: 'element', tagName, properties, children });

export default function glossaryLinks() {
  return tree => {
    let serial = 0;
    const visit = node => {
      if (!Array.isArray(node.children)) return;
      node.children = node.children.flatMap(child => {
        const match = child.tagName === 'a' && PATTERN.exec(child.properties?.href ?? '');
        const term = match && byId.get(match[1]);
        if (!term) { visit(child); return [child]; }
        const id = `day-gloss-${++serial}`;
        child.properties.className = [...[child.properties.className ?? []].flat(), 'gloss'];
        child.properties['data-gloss-pop'] = id;
        child.properties.style = `${child.properties.style ?? ''};anchor-name:--${id}`;
        child.properties['aria-describedby'] = `${id}-definition`;
        const definition = term.definition.split('`').map((part, i) => i % 2 ? element('code', {}, [text(part)]) : text(part));
        const pop = element('span', { id, className: ['gloss-pop'], popover: 'auto', hidden: true, 'data-pagefind-ignore': true, style: `position-anchor:--${id}` }, [
          element('b', {}, [text(term.term)]),
          element('span', { id: `${id}-definition` }, definition),
          element('span', { className: ['gloss-see'] }, [text(res.str.defined_in() + ' '), element('a', { href: term.see.href }, [text(term.see.title)])]),
        ]);
        return [child, pop];
      });
    };
    visit(tree);
  };
}
