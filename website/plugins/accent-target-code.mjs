// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Reuse the site's platform marks and light/dark accents in documentation labels.
// Decorate labels wherever they occur in headings, tables, and emphasis, plus the first
// mention of each target in a paragraph of platform-specific guidance. Preserve code
// examples, API identifiers, links, and the text used to generate heading anchors.
import { readFileSync } from 'node:fs';
import { platforms } from '../src/lib/platforms.mjs';

const targets = new Map(platforms.filter(p => !p.deviceClass).map(p => [p.id, p]));
const aliases = new Map([
  ['macos', ['macos-appkit']], ['appkit', ['macos-appkit']],
  ['ios', ['ios-uikit']], ['ipados', ['ios-uikit']], ['uikit', ['ios-uikit']],
  ['android material', ['android-material']], ['android', ['android-material']], ['material', ['android-material']],
  ['gtk 4', ['linux-gtk']], ['gtk4', ['linux-gtk']], ['gtk', ['linux-gtk']],
  ['qt 6', ['linux-qt']], ['qt6', ['linux-qt']], ['qt', ['linux-qt']],
  ['windows', ['windows-winui']], ['winui 3', ['windows-winui']], ['winui', ['windows-winui']],
  // XAML also names markup and shared feature identifiers, not the current toolkit.
  ['system xaml', ['windows-xaml']],
  ['harmonyos', ['harmony-arkui']], ['openharmony', ['harmony-arkui']], ['arkui', ['harmony-arkui']],
  ['web dom', ['web-dom']], ['web', ['web-dom']], ['dom', ['web-dom']], ['browser', ['web-dom']],
  // An OS/family label must not silently select just one of its toolkits.
  ['linux', ['linux-gtk', 'linux-qt']], ['apple', ['macos-appkit', 'ios-uikit']],
  ...[...targets.keys()].map(id => [id, [id]]),
]);
const names = [...aliases.keys()].sort((a, b) => b.length - a.length);
// Match names, not components of source paths, API identifiers, or compound words.
const targetPattern = new RegExp(`(?<![\\w.-])(${names.join('|')})(?![\\w-]|::|\\.[\\w])`, 'gi');
const icons = new Map([...targets.keys()].map(id => {
  const mark = id.endsWith('-gtk') ? 'linux-gtk' : id.endsWith('-qt') ? 'linux-qt' : id;
  const svg = readFileSync(new URL(`../src/icons/${mark}.svg`, import.meta.url), 'utf8');
  return [id, `url("data:image/svg+xml,${encodeURIComponent(svg)}")`];
}));
const textOf = n => n.type === 'text' ? n.value : (n.children ?? []).map(textOf).join('');
const icon = id => ({ type: 'element', tagName: 'span', properties: {
  className: ['doc-target-icon'], ariaHidden: 'true', style: `--target-icon: ${icons.get(id)}`,
}, children: [] });
const label = (name, ids) => ({
  type: 'element', tagName: 'span', properties: {
    className: ['doc-target-label'],
    ...(ids.length === 1 ? {'data-pf': ids[0]} : {
      'data-targets': ids.join(' '),
      style: `--pf: color-mix(in srgb, var(--pf-${ids[0]}) 50%, var(--pf-${ids[1]}))`,
    }),
  },
  children: [
    ...ids.map(id => {
      const mark = icon(id);
      mark.properties['data-pf'] = id;
      return mark;
    }),
    {type: 'text', value: name},
  ],
});

function decorateText(value, seen) {
  const children = [];
  let at = 0;
  for (const match of value.matchAll(targetPattern)) {
    const ids = aliases.get(match[0].toLowerCase());
    const key = match[0].toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    if (match.index > at) children.push({type: 'text', value: value.slice(at, match.index)});
    children.push(label(match[0], ids));
    at = match.index + match[0].length;
  }
  if (at < value.length) children.push({type: 'text', value: value.slice(at)});
  return children;
}

const isPlatformSection = title =>
  /\b(toolkits?|platforms?|native|availability|transports?|targets?)\b/i.test(title) ||
  [...title.matchAll(targetPattern)].length > 0;

export default function accentTargetCode() {
  return tree => {
    function visit(node, section = false, decorate = false, seen = new Set()) {
      if (node.type !== 'element' && node.type !== 'root') return;
      const tag = node.tagName;
      if (['pre', 'script', 'style', 'svg'].includes(tag)) return;
      if (node.properties?.className?.includes('doc-target-label')) return;
      if (tag === 'code') {
        const name = textOf(node);
        // Exact target IDs are labels everywhere. Short feature names are labels only
        // in target-specific prose or a heading/table/emphasis context.
        const ids = aliases.get(name.toLowerCase());
        if (ids && (decorate || targets.has(name))) {
          const key = name.toLowerCase();
          if (!seen.has(key)) {
            seen.add(key);
            node.children = [label(name, ids)];
          }
        }
        return;
      }
      if (/^(p|li|dt|dd|th|td|h[1-6])$/.test(tag)) seen = new Set();
      decorate ||= /^(strong|th|td|h[2-6])$/.test(tag) || section;
      const headings = [];
      const children = [];
      for (const child of node.children ?? []) {
        let childSection = headings.at(-1)?.section ?? section;
        const heading = child.type === 'element' && /^h([1-6])$/.exec(child.tagName);
        if (heading) {
          const level = Number(heading[1]);
          while (headings.length && headings.at(-1).level >= level) headings.pop();
          childSection = (headings.at(-1)?.section ?? section) || isPlatformSection(textOf(child));
          headings.push({level, section: childSection});
        }
        if (child.type === 'text' && decorate) {
          children.push(...decorateText(child.value, seen));
        } else {
          visit(child, childSection, decorate, seen);
          children.push(child);
        }
      }
      node.children = children;
    }
    visit(tree);
  };
}
