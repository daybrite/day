// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Apps and capture targets shown by the screenshot gallery. Each app publishes its own index;
// scripts/assemble-gallery.mjs combines them and scripts/hero-shots.mjs builds the landing carousel.
// See README.md, “Gallery and screenshot evidence”, for caching and publication checks.

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
 * @property {string} [site]     The app's own website. Defaults to the index's `site` field.
 * @property {{apple?: string, google?: string}} [stores]  Published App Store / Google Play listing URLs.
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
    stores: {
      apple: 'https://apps.apple.com/app/id6802801331',
      google: 'https://play.google.com/store/apps/details?id=dev.daybrite.showcase',
    },
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
    id: 'Day-Travel',
    label: 'Day Travel',
    blurb: 'A travel screen with translated text, local date and number formats, and right-to-left layouts.',
    repo: 'https://github.com/daybrite/Day-Travel',
    site: 'https://daybrite.github.io/Day-Travel',
    platforms: ['ios-uikit', 'android-material', 'harmony-arkui', 'macos-appkit', 'windows-winui', 'linux-gtk', 'linux-qt', 'web-dom'],
    hero: 'paris',
    order: ['paris', 'tokyo', 'new_york', 'rome'],
  },
  {
    id: 'Day-Rise',
    label: 'Day Rise',
    blurb:
      'The starting point every Day app shares: the project the day CLI scaffolds, captured exactly as it generates it.',
    repo: 'https://github.com/daybrite/Day-Rise',
    hero: 'welcome',
    labels: { 'after-new-window': 'After a second window' },
  },
  {
    id: 'Day-Skies',
    label: 'Day Skies',
    blurb:
      'A weather app whose sky follows the conditions, with an hourly strip, a ten-day forecast and detail cards for what you check next.',
    repo: 'https://github.com/daybrite/Day-Skies',
    labels: { 'san-francisco-fahrenheit': 'San Francisco · in Fahrenheit' },
  },
  {
    id: 'Day-Trader',
    label: 'Day Trader',
    blurb:
      'A stock watchlist that opens on the day at a glance: how many symbols moved which way, a sparkline per card, and the detail behind each one.',
    repo: 'https://github.com/daybrite/Day-Trader',
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
    id: 'Fleet-Reader',
    label: 'Fleet Reader',
    blurb:
      'A feed reader in three panes on a desktop and three taps on a phone, handling RSS, Atom, RDF and JSON Feed.',
    repo: 'https://github.com/Fleet-Reader/Fleet-Reader',
    stores: {
      apple: 'https://apps.apple.com/app/id6832401867',
      google: 'https://play.google.com/store/apps/details?id=org.appfair.app.FleetReader',
    },
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
  },
  {
    id: 'Day-Tunes',
    label: 'Day Tunes',
    blurb:
      'An internet radio player with station search, favorites, listening history, and native playback controls.',
    repo: 'https://github.com/daybrite/Day-Tunes',
    hero: 'browse',
  },
  {
    id: 'Stanza-Redux',
    label: 'Stanza Redux',
    blurb:
      'A free EPUB reader with a personal library, online book catalogs, bookmarks, and customizable typography for reading offline.',
    repo: 'https://github.com/Stanza-Redux/Stanza-Redux',
    site: 'https://stanza-redux.app',
    stores: {
      apple: 'https://apps.apple.com/app/id1639831676',
      google: 'https://play.google.com/store/apps/details?id=org.appfair.app.Stanza_Redux',
    },
    hero: 'reading-alice',
  },
  {
    id: 'Day-Plan',
    label: 'Day Plan',
    blurb:
      'An offline planner for tasks and notes, with a My Day view, lists and groups, checklist steps and tags, kept in step between devices through WebDAV servers you choose.',
    repo: 'https://github.com/Day-Plan/Day-Plan',
    stores: {
      apple: 'https://apps.apple.com/app/id6740916318',
      google: 'https://play.google.com/store/apps/details?id=org.appfair.app.SkipNotes',
    },
    hero: 'planner',
    order: ['planner', 'details', 'lists'],
    labels: { planner: 'My Day', details: 'Task details', lists: 'Lists and groups' },
    // The sidebar's views are routes (`day::routes!` in the app's src/navigation.rs) that web-dom
    // mirrors into the URL hash. Each fragment was loaded against the published build. An open
    // task's editor is a state inside a view, which a fragment cannot open on its own.
    webRoutes: { planner: 'today', lists: 'organize', details: null },
  },
  {
    id: 'Games-Fair',
    label: 'Games Fair',
    blurb:
      'Ten classic games in one app, from solitaire and sudoku to minesweeper, charades and reversi, each board drawn on a canvas by the game itself and playable offline.',
    repo: 'https://github.com/Games-Fair/Games-Fair',
    stores: {
      apple: 'https://apps.apple.com/app/id6761661340',
      google: 'https://play.google.com/store/apps/details?id=org.appfair.app.Faire_Games',
    },
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

export default { platforms, apps };
