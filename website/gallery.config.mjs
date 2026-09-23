// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// The gallery's extensibility surface: which Day apps daybrite.dev indexes, and nothing else.
//
// Model
// -----
// Every Day app's own website publishes the machine-readable screenshot index `day screenshot
// index` writes, carrying each capture's absolute URL, shot id, localized title and caption,
// source path, platform-toolkit, device, theme, locale and pixel size (docs/screenshots.md,
// DESIGN.md §14.7) — and publishes it twice, once per build: the newest release's at
// `<host>/gallery/gallery.json`, and the newest build of the default branch's at
// `<host>/main/gallery/gallery.json`. This site READS one of those indexes and links the images
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
 * Whether the gallery shows each app's newest RELEASE rather than the newest build of its default
 * branch.
 *
 * Off: the branch build is the one every push refreshes, so its screenshots match the app as it
 * is today, and a release's can be months behind. Turn it on once apps release often enough that
 * a release's screenshots are the ones worth showing. Either way the other build is the fallback,
 * so an app that has not published the preferred one still appears — and a site with no release
 * publishes its single build where both settings find it. `preferRelease` on an app overrides
 * this for that app.
 */
export const preferReleaseScreenshots = false;

/**
 * The apps this gallery indexes, in display order.
 *
 * @typedef {object} App
 * @property {string}  id        Repository name, and the URL segment: `/gallery/Day-Rise/`.
 * @property {string}  label     Display name.
 * @property {string}  blurb     One sentence on what the app is, for its page and its hub card.
 * @property {string}  repo      GitHub repository — where a shot's `source` path resolves.
 * @property {string}  metadata  The site's root index, `<host>/gallery/gallery.json`: its release
 *                               build's, or its only build's before it has shipped a release. The
 *                               branch build's index beside it, under `main/`, is derived from it.
 * @property {boolean} [preferRelease]  Override `preferReleaseScreenshots` for this app alone.
 * @property {string} [site]     The app's own website. Defaults to the index's `site` field.
 * @property {string} [web]      Its hosted web-dom build, when that lives somewhere other than
 *                               `<site>/webapp/` — the address every app whose index carries a
 *                               web-dom column gets by default.
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
      'One app showing every implemented Piece, rendered with native widgets on each target.',
    repo: 'https://github.com/daybrite/Day-Showcase',
    site: 'https://showcase.daybrite.dev',
    metadata: 'https://showcase.daybrite.dev/gallery/gallery.json',
    hero: 'home',
    // The showcase's web build takes its route from the URL fragment (`day_dom_set_hash` writes
    // it, a `hashchange` listener reads it back), so a gallery row can open the very screen it
    // photographs. `webShots` are the shots whose id IS the route — the `Section` enum keys in the
    // app's src/lib.rs — and `webRoutes` covers the rest, because a shot id is NOT always a route:
    // several rows capture a STATE of a page rather than a page. A shot in neither gets no link.
    // Checked by loading each fragment against the built app, not read off the enum.
    webShots: [
      'controls', 'text', 'textareas', 'dates', 'focus', 'layout', 'grid', 'stack', 'tabs', 'menus',
      'toolbars', 'list', 'tree', 'model', 'query', 'canvas', 'animation', 'resources', 'media',
      'webview', 'system', 'network', 'notify', 'speech', 'files', 'localization', 'scripting',
      'tweaks', 'benchmark', 'about',
    ],
    webRoutes: {
      home: '',
      'controls-pickers': 'controls',
      'grid-spanning': 'grid',
      'stack-detail': 'stack',
      'tabs-one': 'tabs',
      'tree-final': 'tree',
      'textareas-code': 'textareas',
      'webview-embedded': 'webview',
      'toolbars-filtered': 'toolbars',
      // `preferences` is a separate window and `cover` a fullscreen presentation, so neither is
      // reachable by a fragment. Lottie, Map and the SwiftUI benchmark have no web build page.
      preferences: null,
      cover: null,
      lottie: null,
      map: null,
      'benchmark-swiftui': null,
    },
    // Every shot the walkthrough keeps carries a `title:` in the app's own dayscript, so the
    // gallery needs no hiding and no relabelling here (2026-09).
    hide: [],
    labels: {},
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
    id: 'Day-Trader',
    label: 'Day Trader',
    blurb:
      'A stock watchlist that opens on the day at a glance: how many symbols moved which way, a sparkline per card, and the detail behind each one.',
    repo: 'https://github.com/daybrite/Day-Trader',
    metadata: 'https://daybrite.github.io/Day-Trader/gallery/gallery.json',
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
  {
    id: 'Day-Tunes',
    label: 'Day Tunes',
    blurb:
      'An internet radio player with station search, favorites, listening history, and native playback controls.',
    repo: 'https://github.com/daybrite/Day-Tunes',
    metadata: 'https://daybrite.github.io/Day-Tunes/gallery/gallery.json',
    hero: 'browse',
  },
  {
    id: 'Games-Fair',
    label: 'Games Fair',
    blurb:
      'Ten classic games in one app, from solitaire and sudoku to minesweeper, charades and reversi, each board drawn on a canvas by the game itself and playable offline.',
    repo: 'https://github.com/Games-Fair/Games-Fair',
    metadata: 'https://games-fair.github.io/Games-Fair/gallery/gallery.json',
    hero: 'home',
    // Every game is a route (`day::routes!` in the app's src/lib.rs) and web-dom mirrors the
    // current route into the URL hash, so a gallery row opens the screen it photographs.
    // `webShots` are the shots whose id IS the route; `webRoutes` covers the rest. Each fragment
    // was loaded against the published build rather than read off the enum.
    webShots: ['blockblast', 'charades', 'mines', 'pipes', 'reversi', 'solitaire', 'sudoku'],
    webRoutes: {
      home: '',
      'breakout-a': 'breakout',
      'sirtet-b': 'sirtet',
      g2048: 'twentyfortyeight',
      // A difficulty sheet, a rules panel and a solved board are states inside a game, which a
      // fragment cannot open on its own.
      'g2048-difficulty': null,
      'reversi-rules': null,
      'pipes-complete': null,
    },
    // The gallery shows one row per game, plus the two states worth a second look. Left out:
    // `smoke`, which is the launch check, the untitled twin each game's walkthrough takes a
    // moment after its titled shot, and the steps Reversi's and Pipes' own scripts walk through.
    order: [
      'home', 'solitaire', 'blockblast', 'breakout-a', 'sirtet-b', 'sudoku', 'g2048',
      'g2048-difficulty', 'mines', 'charades', 'reversi', 'reversi-rules', 'pipes',
      'pipes-complete',
    ],
    hide: [
      'smoke', 'blockblast-lift', 'solitaire-flight', 'charades-card', 'breakout-b', 'sirtet-a',
      'g2048-easy', 'reversi-opening', 'reversi-two-players', 'reversi-settings',
      'reversi-computer', 'pipes-opening', 'pipes-locked', 'pipes-settings', 'pipes-rules',
      'pipes-medium', 'pipes-large',
    ],
    labels: { 'reversi-rules': 'Reversi · how to play', 'pipes-complete': 'Pipes · solved' },
  },
];

export default { platforms, apps, preferReleaseScreenshots };
