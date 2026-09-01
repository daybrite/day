// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// The gallery's extensibility surface: which Day apps daybrite.dev indexes, and nothing else.
//
// Model
// -----
// Every Day app's own website publishes `<host>/gallery/gallery.json` — the machine-readable
// screenshot index `day screenshot index` writes, carrying each capture's absolute URL, shot id,
// localized title and caption, source path, platform-toolkit, device, theme, locale and pixel
// size (docs/screenshots.md, DESIGN.md §14.7). This site READS those indexes and links the images
// where they are hosted. Nothing is copied here, and daybrite.dev's build depends on no other
// repository's CI: an app republishes its gallery on its own schedule, and the next website build
// picks it up.
//
// Adding an app is one entry below. Its rows, columns, languages and themes all come from its own
// index, so an app that captures a new screen or gains a platform shows it without a change here.
// The optional `order` / `labels` / `hide` / `platforms` keys exist for apps whose dayscripts
// carry thin metadata — a shot with no `title:` falls back to a label derived from its id.
//
// Each app gets its own page at /gallery/<id>/, and /gallery/ indexes them.

import { platforms as platformTable } from './src/lib/platforms.mjs';

/** @typedef {{ id: string, label: string, os: string, toolkit: string }} Platform */

/** The capture targets, in display order. Names and shells come from the platform table
 *  (src/lib/platforms.mjs), so a rename lands on the gallery, the landing page and the
 *  showcase at once; `label` is the gallery's own short chip and stays derived from it. */
export const platforms = /** @type {Platform[]} */ (
  platformTable.map((p) => ({
    id: p.id,
    label: p.chip ?? p.toolkit,
    os: p.osShort ?? p.os,
    toolkit: p.toolkitLong,
  }))
);

/**
 * The apps this gallery indexes, in display order.
 *
 * @typedef {object} App
 * @property {string}  id        Repository name, and the URL segment: `/gallery/Day-Rise/`.
 * @property {string}  label     Display name.
 * @property {string}  blurb     One sentence on what the app is, for its page and its hub card.
 * @property {string}  repo      GitHub repository — where a shot's `source` path resolves.
 * @property {string}  metadata  The published `gallery.json`.
 * @property {string} [site]     The app's own website. Defaults to the index's `site` field.
 * @property {string} [web]      Its hosted web-dom build, when it has one.
 * @property {Record<string,string|null>} [webRoutes]  Shot id → the URL fragment that opens that
 *                               screen in `web`, `null` where the screen is unreachable by one.
 *                               A shot absent from the map opens the app's root.
 * @property {string} [hero]     The shot that leads the hub card's carousel.
 * @property {string[]} [order]  Shot ids first in row order; the index's own order fills in behind.
 * @property {Record<string,string>} [labels]  Row headings for shots whose index metadata has none.
 * @property {string[]} [hide]   Shot ids to leave out.
 * @property {string[]} [platforms]  Column allow list. Absent = every platform the index carries.
 * @type {App[]}
 */
export const apps = [
  {
    id: 'Day-Showcase',
    label: 'Day Showcase',
    blurb:
      'One Rust program showing every implemented Piece, rendered with native widgets on each target.',
    repo: 'https://github.com/daybrite/Day-Showcase',
    site: 'https://showcase.daybrite.dev',
    metadata: 'https://showcase.daybrite.dev/gallery/gallery.json',
    web: 'https://showcase.daybrite.dev/webapp/',
    hero: 'home',
    // The showcase's web build takes its route from the URL fragment (`day_dom_set_hash` writes
    // it, a `hashchange` listener reads it back), so a gallery row can open the very screen it
    // photographs. The names are the `Section` enum keys in the app's src/lib.rs — spelled here
    // because a shot id is NOT always a route: several rows capture a STATE of a page rather than
    // a page, and `preferences` (a window) and `cover` (a fullscreen presentation) are reachable
    // by neither. Checked by loading each fragment against the built app, not read off the enum.
    webRoutes: {
      home: '',
      'list-item-100': 'list',
      'stack-detail': 'stack',
      'tabs-one': 'tabs',
      preferences: null,
      cover: null,
      // The web build has no Toolbars page; both toolbar rows would land on About.
      toolbars: null,
      'toolbars-filtered': null,
    },
  },
  {
    id: 'Day-Rise',
    label: 'Day Rise',
    blurb:
      'The project `day new` scaffolds, captured exactly as the CLI generates it — the starting point every Day app shares.',
    repo: 'https://github.com/daybrite/Day-Rise',
    metadata: 'https://daybrite.github.io/Day-Rise/gallery/gallery.json',
    hero: 'welcome',
  },
  {
    id: 'Day-Skies',
    label: 'Day Skies',
    blurb:
      'A weather app whose sky follows the conditions, with an hourly strip, a ten-day forecast and detail cards for what you check next.',
    repo: 'https://github.com/daybrite/Day-Skies',
    metadata: 'https://daybrite.github.io/Day-Skies/gallery/gallery.json',
  },
  {
    id: 'Day-Tradr',
    label: 'Day Tradr',
    blurb:
      'A stock watchlist that opens on the day at a glance: how many symbols moved which way, a sparkline per card, and the detail behind each one.',
    repo: 'https://github.com/daybrite/Day-Tradr',
    metadata: 'https://daybrite.github.io/Day-Tradr/gallery/gallery.json',
  },
  {
    id: 'Day-News',
    label: 'Day News',
    blurb:
      'A feed reader in three panes on a desktop and three taps on a phone, handling RSS, Atom, RDF and JSON Feed.',
    repo: 'https://github.com/daybrite/Day-News',
    metadata: 'https://daybrite.github.io/Day-News/gallery/gallery.json',
  },
  {
    id: 'Day-Sketch',
    label: 'Day Sketch',
    blurb:
      'A vector drawing editor with drag handles, layer arrangement and unlimited undo, keeping each drawing in a plain SQLite file.',
    repo: 'https://github.com/daybrite/Day-Sketch',
    metadata: 'https://daybrite.github.io/Day-Sketch/gallery/gallery.json',
  },
];

export default { platforms, apps };
