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
 * @property {string[]} [webShots]  Shots whose id IS the fragment that opens them in `web`.
 * @property {Record<string,string|null>} [webRoutes]  Shot id → the fragment that opens that
 *                               screen, for the shots whose id is not one, and `null` where the
 *                               screen is unreachable by a fragment. A shot in neither list gets
 *                               no launch link.
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
    // photographs. `webShots` are the shots whose id IS the route — the `Section` enum keys in the
    // app's src/lib.rs — and `webRoutes` covers the rest, because a shot id is NOT always a route:
    // several rows capture a STATE of a page rather than a page. A shot in neither gets no link.
    // Checked by loading each fragment against the built app, not read off the enum.
    webShots: [
      'controls', 'dates', 'focus', 'text', 'textareas', 'localization', 'canvas', 'animation',
      'grid', 'list', 'layout', 'tree', 'model', 'query', 'refresh', 'tabs', 'stack', 'media',
      'webview', 'menus', 'system', 'services', 'resources', 'tweaks', 'crash', 'about',
    ],
    webRoutes: {
      home: '',
      'list-item-100': 'list',
      'stack-detail': 'stack',
      'tabs-one': 'tabs',
      'tabs-two': 'tabs',
      'tabs-three': 'tabs',
      'stack-root': 'stack',
      'list-bottom': 'list',
      'list-item-100-shuffled': 'list',
      'list-deleted': 'list',
      'layout-even-columns': 'layout',
      'tree-final': 'tree',
      'textareas-code': 'textareas',
      'webview-embedded': 'webview',
      // `preferences` is a separate window and `cover` a fullscreen presentation, so neither is
      // reachable by a fragment. The web build has no Toolbars page; both toolbar rows would
      // land on About.
      preferences: null,
      cover: null,
      toolbars: null,
      'toolbars-filtered': null,
    },
    // `back-home` photographs the navigation returning to a screen `home` already shows, and the
    // two benchmark variants are the dense grid and the SwiftUI comparison — Apple-only refinements
    // of the `benchmark` row rather than screens of their own.
    hide: ['back-home', 'benchmark-dense', 'benchmark-swiftui'],
    // Rows the walkthrough captures without a `title:`, which would otherwise read as a shot id
    // in title case. Each is a STATE of the page named before the separator. The durable fix is a
    // `title:` on those `screenshot:` steps in the app's own dayscript; until then, here.
    labels: {
      'layout-even-columns': 'Layout · even columns',
      'list-bottom': 'List · scrolled to the end',
      'list-item-100-shuffled': 'List · shuffled',
      'list-deleted': 'List · after a delete',
      'stack-root': 'Stack · root',
      'tabs-two': 'Tabs · second tab',
      'tabs-three': 'Tabs · third tab',
      'tree-final': 'Tree · after the moves',
      'webview-embedded': 'Web view · embedded',
      speech: 'Speech',
    },
  },
  {
    id: 'Day-Rise',
    label: 'Day Rise',
    blurb:
      'The starting point every Day app shares: the project the day CLI scaffolds, captured exactly as it generates it.',
    repo: 'https://github.com/daybrite/Day-Rise',
    metadata: 'https://daybrite.github.io/Day-Rise/gallery/gallery.json',
    hero: 'welcome',
    labels: { 'after-new-window': 'After a second window' },
  },
  {
    id: 'Day-Skies',
    label: 'Day Skies',
    blurb:
      'A weather app whose sky follows the conditions, with an hourly strip, a ten-day forecast and detail cards for what you check next.',
    repo: 'https://github.com/daybrite/Day-Skies',
    metadata: 'https://daybrite.github.io/Day-Skies/gallery/gallery.json',
    labels: { 'san-francisco-fahrenheit': 'San Francisco · in Fahrenheit' },
  },
  {
    id: 'Day-Tradr',
    label: 'Day Tradr',
    blurb:
      'A stock watchlist that opens on the day at a glance: how many symbols moved which way, a sparkline per card, and the detail behind each one.',
    repo: 'https://github.com/daybrite/Day-Tradr',
    metadata: 'https://daybrite.github.io/Day-Tradr/gallery/gallery.json',
    labels: {
      'watchlist-chip-absolute': 'Watchlist · absolute change',
      'watchlist-sorted': 'Watchlist · sorted',
      detail: 'Symbol detail',
      'detail-1m': 'Symbol detail · one month',
      'detail-no-overlay': 'Symbol detail · without the overlay',
      manage: 'Manage the watchlist',
    },
  },
  {
    id: 'Day-News',
    label: 'Day News',
    blurb:
      'A feed reader in three panes on a desktop and three taps on a phone, handling RSS, Atom, RDF and JSON Feed.',
    repo: 'https://github.com/daybrite/Day-News',
    metadata: 'https://daybrite.github.io/Day-News/gallery/gallery.json',
    labels: {
      'seeded-fixtures': 'A seeded library',
      'search-results': 'Search results',
      'keyboard-next': 'Timeline · walked by keyboard',
      'all-read': 'Timeline · all read',
      'tag-scope': 'Timeline · scoped to a tag',
      'sidebar-hidden': 'Sidebar hidden',
    },
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
