// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Rehype plugin — turn call-out blockquotes into titled admonition boxes.
//
// Markdown keeps writing blockquotes; this gives them a tinted box, a header with an icon and a
// title, and a stable anchor so any one of them can be linked to from outside:
//
//   > [!WARNING] Experimental
//   > body…                          → aside.admonition[data-kind=warning]#admonition-experimental
//
//   > **Which XAML.** body…          → title "Which XAML", kind inferred (see KIND_OF)
//
// Three ways to title one, in precedence order:
//
//   1. text after the `[!KIND]` marker on the same line — the only form that can carry a link,
//      since the title keeps its inline markup;
//   2. a leading `**bold lead.**`, which is how ~40 reference docs already open ("**Status:
//      implemented.** …"), so those pages need no edit at all;
//   3. the kind's own name, for a bare `> [!NOTE]`.
//
// A blockquote with NEITHER a marker NOR a bold lead is left alone: quoted prose (the agent
// prompts in ai.md and getting-started) is a quote, not a call-out, and should still read as one.
//
// `[!NOTE|TIP|IMPORTANT|WARNING|CAUTION]` are GitHub's own alert markers, which matters because
// `docs/*.md` renders here AND on GitHub. Two things are ours alone and print literally over
// there, so keep them to the website-only pages under src/content/docs/: the `[!QUESTION]` and
// `[!STATUS]` kinds, and a title written on the marker line. In `docs/*.md`, write the bare
// marker on its own line with a `**bold lead.**` under it — GitHub renders that as an alert, and
// the lead becomes the title here.

// Only horizontal space after the marker: the line BREAK has to survive, because it is what
// separates `> [!NOTE] A title` from a bare `> [!NOTE]` whose title comes from the bold lead below.
const MARKER = /^[ \t]*\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION|QUESTION|STATUS)\][ \t]*/i;

/** Default header text for a marker with no title of its own. */
const LABEL = {
  note: 'Note',
  tip: 'Tip',
  important: 'Important',
  warning: 'Warning',
  question: 'Question',
  status: 'Status',
};

/** GitHub's CAUTION is our warning; everything else keeps its name. */
const ALIAS = { caution: 'warning' };

/**
 * Kind for a blockquote whose author did not pick one. The reference docs open with a status line
 * ("**Status: implemented** …", "**Implementation status (2026-08-09).** …"), which is metadata
 * about the page rather than a call-out — it gets the quiet neutral box. Everything else is a
 * note. `[!NOTE]` counts as "did not pick one" (it is GitHub's generic marker, and GitHub has no
 * status alert), so a status banner reads the same on the ~40 pages that open with one; any other
 * marker is a deliberate choice and wins.
 */
const KIND_OF = (title) => (/^[\w\s()-]{0,26}\bstatus\b/i.test(title.trim()) ? 'status' : 'note');

// 24×24 stroke icons, drawn in currentColor so each kind's tint carries them. Kept here rather
// than in src/icons/ because that folder is the platform marks, which are a different vocabulary.
const ICONS = {
  note: ['M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Z', 'M12 11v5.4', 'M12 7.7h.01'],
  tip: ['M9 16.4a6 6 0 1 1 6 0 2.6 2.6 0 0 0-1 2v.4h-4v-.4a2.6 2.6 0 0 0-1-2Z', 'M10.2 21.2h3.6'],
  important: ['M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Z', 'M12 7.6v5.6', 'M12 16.6h.01'],
  warning: [
    'M10.6 4.3 2.9 17.6a1.6 1.6 0 0 0 1.4 2.4h15.4a1.6 1.6 0 0 0 1.4-2.4L13.4 4.3a1.6 1.6 0 0 0-2.8 0Z',
    'M12 9.4v4.2',
    'M12 17h.01',
  ],
  question: [
    'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Z',
    'M9.4 9.6a2.7 2.7 0 1 1 3.4 2.6c-.9.3-1.4 1-1.4 1.9v.5',
    'M11.9 17.3h.01',
  ],
  status: ['M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Z', 'M8.2 12.2l2.6 2.6 5-5.4'],
};

const icon = (kind) => ({
  type: 'element',
  tagName: 'svg',
  properties: {
    className: ['admonition-icon'],
    viewBox: '0 0 24 24',
    width: 20,
    height: 20,
    fill: 'none',
    stroke: 'currentColor',
    strokeWidth: 1.7,
    strokeLinecap: 'round',
    strokeLinejoin: 'round',
    ariaHidden: 'true',
    focusable: 'false',
  },
  children: (ICONS[kind] ?? ICONS.note).map((d) => ({
    type: 'element',
    tagName: 'path',
    properties: { d },
    children: [],
  })),
});

/** All the text under a node, for slugs and kind inference. */
const textOf = (node) =>
  node.type === 'text' ? node.value : (node.children ?? []).map(textOf).join('');

/**
 * GitHub-style slug, so an admonition anchor reads like a heading anchor. Long titles are cut at
 * a word boundary — an id is a URL someone pastes into an issue, and `…-shipped-on-appl` helps
 * nobody.
 */
const slugify = (s) => {
  const full = s
    .toLowerCase()
    .replace(/[’']/g, '')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  if (full.length <= 48) return full || 'note';
  const cut = full.slice(0, 48);
  const atWord = cut.slice(0, cut.lastIndexOf('-'));
  return (atWord.length >= 24 ? atWord : cut).replace(/-+$/, '') || 'note';
};

const isBlank = (n) => n.type === 'text' && !n.value.trim();

/** Drop a leading separator left behind after lifting a `**bold lead.**` out of its paragraph. */
function trimLead(children) {
  while (children.length && isBlank(children[0])) children.shift();
  if (children.length && children[0].type === 'text') {
    children[0] = { ...children[0], value: children[0].value.replace(/^[\s.:—–-]+/, '') };
    if (!children[0].value) children.shift();
  }
  return children;
}

/**
 * Split a paragraph's children at the first line break: markdown renders the newline after
 * `> [!NOTE] Title` as a `\n` inside a text node, so that break is where the title ends.
 */
function splitAtBreak(children) {
  for (let i = 0; i < children.length; i++) {
    const node = children[i];
    if (node.type !== 'text' || !node.value.includes('\n')) continue;
    const at = node.value.indexOf('\n');
    const head = children.slice(0, i);
    const before = node.value.slice(0, at);
    const after = node.value.slice(at + 1);
    if (before.trim()) head.push({ ...node, value: before });
    const tail = after ? [{ ...node, value: after }, ...children.slice(i + 1)] : children.slice(i + 1);
    return [head, tail];
  }
  return [children, []];
}

export default function admonitions() {
  return (tree, file) => {
    const used = new Map(); // per-document, so ids stay unique on the page

    const visit = (node) => {
      if (Array.isArray(node.children)) {
        node.children = node.children.map((child) => visit(child) ?? child);
      }
      if (node.type !== 'element' || node.tagName !== 'blockquote') return node;

      const blocks = node.children.filter((c) => c.type === 'element');
      const first = blocks[0];
      if (!first || first.tagName !== 'p') return node;

      // 1. the [!KIND] marker, if any.
      let kind = null;
      let children = [...first.children];
      if (children[0]?.type === 'text') {
        const m = MARKER.exec(children[0].value);
        if (m) {
          kind = m[1].toLowerCase();
          kind = ALIAS[kind] ?? kind;
          const rest = children[0].value.slice(m[0].length);
          if (rest) children[0] = { ...children[0], value: rest };
          else children.shift();
        }
      }

      // 2. the title: marker line remainder, else a leading bold lead, else the kind's name.
      let title = null;
      if (kind) {
        const [head, tail] = splitAtBreak(children);
        if (head.length && textOf({ children: head }).trim()) {
          title = head;
          children = trimLead(tail);
        }
      }
      if (!title) {
        const lead = children.find((c) => !isBlank(c));
        if (lead?.type === 'element' && lead.tagName === 'strong') {
          title = lead.children;
          children = trimLead(children.filter((c) => c !== lead));
        }
      }
      if (!title && !kind) return node; // a plain quote stays a plain quote
      if (!title) title = [{ type: 'text', value: LABEL[kind] ?? 'Note' }];

      // A title written as a sentence ("Tiers move up.") loses its full stop in the header.
      const last = title[title.length - 1];
      if (last?.type === 'text') {
        title = [...title.slice(0, -1), { ...last, value: last.value.replace(/[.:]\s*$/, '') }];
      }

      const label = textOf({ children: title }).trim();
      const inferred = KIND_OF(label);
      // `[!NOTE]` is the generic marker, so a status banner still reads as one; anything else the
      // author wrote is deliberate and stands.
      kind = !kind || (kind === 'note' && inferred === 'status') ? inferred : kind;

      // 3. a stable anchor: the title's slug, numbered only if a page repeats one.
      const base = `admonition-${slugify(label)}`;
      const n = (used.get(base) ?? 0) + 1;
      used.set(base, n);
      const id = n === 1 ? base : `${base}-${n}`;

      // The first paragraph may now be empty (the title WAS the paragraph).
      const body = node.children.filter((c) => c !== first);
      if (children.some((c) => !isBlank(c))) body.unshift({ ...first, children });

      return {
        type: 'element',
        tagName: 'aside',
        // aria-labelledby makes the title the box's accessible name, so "Warning: Experimental"
        // is what a screen reader announces on entry rather than an unnamed group.
        properties: {
          className: ['admonition'],
          'data-kind': kind,
          id,
          ariaLabelledBy: `${id}-title`,
        },
        children: [
          {
            type: 'element',
            tagName: 'div',
            properties: { className: ['admonition-head'] },
            children: [
              icon(kind),
              {
                type: 'element',
                tagName: 'span',
                properties: { className: ['admonition-title'], id: `${id}-title` },
                children: title,
              },
              {
                type: 'element',
                tagName: 'a',
                properties: {
                  className: ['admonition-anchor'],
                  href: `#${id}`,
                  ariaLabel: `Link to “${label}”`,
                },
                children: [{ type: 'text', value: '#' }],
              },
            ],
          },
          {
            type: 'element',
            tagName: 'div',
            properties: { className: ['admonition-body'] },
            children: body,
          },
        ],
      };
    };

    visit(tree);
  };
}
