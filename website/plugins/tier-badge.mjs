// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Rehype plugin — render support-tier links as tier badges.
//
// Wherever a docs page links to the tier definitions with link text that starts "Tier N"
// (`[Tier 3](/docs/platforms#support-tiers)`, `[Tier 1 · Supported](…)`), stamp the anchor with
// `class="tier-badge" data-tier="N"`; global.css turns that into the pill. The markdown stays a
// plain link, so the same source reads correctly on GitHub and in an editor, and a link that
// stops matching simply renders as an ordinary link again.
//
// The href only has to END with the tiers anchor, so the internal reference docs (`docs/*.md`,
// read on GitHub as often as on the site) can use the absolute https://daybrite.dev/… form.

import { tiers } from '../src/lib/platforms.mjs';

const ANCHOR = '/docs/platforms#support-tiers';

/** Collect an element's text, so `[**Tier 1**](…)` matches like `[Tier 1](…)` does. */
function textOf(node) {
  if (node.type === 'text') return node.value;
  if (Array.isArray(node.children)) return node.children.map(textOf).join('');
  return '';
}

export default function tierBadge() {
  return (tree) => {
    const visit = (node) => {
      if (
        node.type === 'element' &&
        node.tagName === 'a' &&
        typeof node.properties?.href === 'string' &&
        node.properties.href.endsWith(ANCHOR)
      ) {
        const match = /^\s*Tier\s+([1-4])\b/.exec(textOf(node));
        if (match) {
          const tier = tiers.find((t) => t.n === Number(match[1]));
          node.properties.className = [
            ...[node.properties.className ?? []].flat(),
            'tier-badge',
          ];
          node.properties['data-tier'] = match[1];
          // The name and promise, for readers who hover rather than follow the link.
          if (tier) node.properties.title = `Tier ${tier.n} — ${tier.name}: ${tier.blurb}`;
        }
      }
      if (Array.isArray(node.children)) node.children.forEach(visit);
    };
    visit(tree);
  };
}
