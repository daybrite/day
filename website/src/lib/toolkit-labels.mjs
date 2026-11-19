// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { platforms } from './platforms.mjs';

const targets = new Map(platforms.filter(p => !p.deviceClass).map(p => [p.id, p]));

/** Validate explicit target IDs; two IDs describe an OS family without picking one toolkit. */
export function toolkitLabel(id) {
  const ids = typeof id === 'string' ? id.split(',').map(value => value.trim()) : [];
  if (!ids.length || ids.length > 2 || new Set(ids).size !== ids.length || ids.some(id => !targets.has(id))) {
    throw new Error(`Invalid toolkit label ID: ${JSON.stringify(id)}`);
  }
  const marks = ids.map(id => ({ id, icon: id.endsWith('-gtk') ? 'linux-gtk' : id.endsWith('-qt') ? 'linux-qt' : id }));
  return {
    ids,
    marks,
    name: ids.map(id => targets.get(id).chip ?? targets.get(id).toolkit).join(' / '),
    attributes: ids.length === 1 ? { 'data-pf': ids[0] } : {
      'data-targets': ids.join(' '),
      style: `--pf: color-mix(in srgb, var(--pf-${ids[0]}) 50%, var(--pf-${ids[1]}))`,
    },
  };
}
