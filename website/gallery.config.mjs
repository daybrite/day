// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// The gallery's extensibility surface. Adding a sample app, a platform, or a curated shot is a
// data change here — the assembly (scripts/assemble-gallery.mjs) and the gallery page consume
// this config; neither needs editing to add a new app or component-snapshot set.
//
// Model
// -----
//   suites   — a screenshot-producing thing: a sample app OR a set of component snapshots.
//   platforms — the (OS, toolkit) targets a suite is captured on.
//   shots    — the curated, ordered captures shown per (suite, platform).
//
// Where the images come from
// --------------------------
// Each CI job uploads an artifact `screenshots-<platform>` containing `<variant>/<shot>.png`
// (crates/day-cli/src/script.rs `--variant`): the walkthrough runs once per variant — `light`
// and `dark` under a forced DAY_THEME, and `fr` under `--locale fr`. `artifactPattern` maps a
// (suite, platform) pair to that artifact name, so a future suite that uploads
// `screenshots-widgets-<platform>` only needs its own `artifactPattern`. Each variant may fall
// back to the extra directories listed in `variants` (older artifacts used locale subdirs).

import { platforms as platformTable } from './src/lib/platforms.mjs';

/** @typedef {{ id: string, label: string, os: string, toolkit: string }} Platform */
/** `source` is the path of the code that renders the shot, relative to its SUITE's repository
 *  (`sourceRepo`), not to this one — e.g. `src/pages/controls.rs` in daybrite/Day-Showcase.
 *  Linked from the row header.
 *  @typedef {{ id: string, label: string, source?: string }} Shot */

/** The twelve CI targets, in display order. Names and shells come from the platform table
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
 * Screenshot suites. Today just the Showcase app; the shape scales to more sample apps and to
 * per-component snapshot sets (add another entry with its own `artifactPattern` + `shots`).
 * @type {{ id: string, label: string, blurb: string, artifactPattern: string, sourceRepo?: string,
 *          preferLocales: string[], platforms: string[], hero: string, shots: Shot[] }[]}
 */
export const suites = [
  {
    id: 'showcase',
    label: 'Day Showcase',
    blurb:
      'One Rust program showing every implemented Piece, rendered with native widgets on each target.',
    // The showcase is its own repository (it used to live in this one under apps/showcase/), so
    // its `source` paths resolve there rather than against daybrite/day. A suite whose code DOES
    // live in this repo omits this and gets `site.repo`. Same URL as site.ts's `showcaseRepo`,
    // spelled here because this config is also imported by plain node scripts that cannot read a
    // .ts module.
    sourceRepo: 'https://github.com/daybrite/Day-Showcase',
    // `{platform}` is substituted with the platform id.
    artifactPattern: 'screenshots-{platform}',
    // The capture variants, in display order: theme × locale (CI runs the walkthrough once per
    // combination; `<theme>` alone is English). `dirs` are the artifact subdirectories that may
    // hold the variant (fallbacks cover older artifacts); non-English/dark variants deliberately
    // have NO cross-variant fallback here — assembly must never pass one variant off as another
    // (the gallery page falls back VISIBLY instead). Variant ids stay lowercase (they ride
    // data-* attributes); `dirs` match the CI `--variant` names exactly.
    variants: [
      { id: 'light', label: 'Light · English', dirs: ['light', 'default', 'en'] },
      { id: 'dark', label: 'Dark · English', dirs: ['dark'] },
      { id: 'light-fr', label: 'Light · Français', dirs: ['light-fr', 'fr'] },
      { id: 'dark-fr', label: 'Dark · Français', dirs: ['dark-fr'] },
      { id: 'light-ar', label: 'Light · العربية', dirs: ['light-ar'] },
      { id: 'dark-ar', label: 'Dark · العربية', dirs: ['dark-ar'] },
      { id: 'light-zh-cn', label: 'Light · 中文', dirs: ['light-zh-CN'] },
      { id: 'dark-zh-cn', label: 'Dark · 中文', dirs: ['dark-zh-CN'] },
    ],
    // The PRIMARY target per OS, in display order — one strip column per platform users actually
    // ship to. The secondary desktop combos (macos-gtk/qt, windows-gtk/qt) still run in CI and
    // upload artifacts; they're just not shown here.
    platforms: [
      'ios-uikit',
      'android-mdc',
      'harmony-arkui',
      'macos-appkit',
      'windows-xaml',
      'linux-qt',
      'linux-gtk',
      'web-dom',
    ],
    hero: 'home',
    // ORDER: the Showcase's own top-level navigation list, which is alphabetical by English
    // title (the showcase's src/lib.rs `destinations()`) — so the gallery reads in the same order
    // as the app's sidebar. `home` leads as the hero, and the surfaces that are not their own
    // destination (a window, a modal, a filtered variant) follow the row they are reached from.
    shots: [
      { id: 'home', label: 'Home', source: 'src/lib.rs' },
      { id: 'about', label: 'About', source: 'src/pages/about.rs' },
      { id: 'animation', label: 'Animation', source: 'src/pages/animation.rs' },
      // The benchmark patchwork: the same generated scene on every target, so the row doubles as
      // a cross-platform rendering diff. The walkthrough also captures `benchmark-dense` and
      // `benchmark-swiftui` (Apple targets only); those shots are deliberately not gallery rows —
      // the assembler only consults ids listed here, so they stay in the CI artifacts.
      { id: 'benchmark', label: 'Benchmark', source: 'src/pages/benchmark.rs' },
      { id: 'canvas', label: 'Canvas & shapes', source: 'src/pages/canvas.rs' },
      { id: 'controls', label: 'Controls', source: 'src/pages/controls.rs' },
      { id: 'crash', label: 'Crash reporting', source: 'src/pages/crash.rs' },
      { id: 'dates', label: 'Date & time', source: 'src/pages/dates.rs' },
      { id: 'system', label: 'Device & sensors', source: 'src/pages/system.rs' },
      { id: 'focus', label: 'Focus', source: 'src/pages/focus.rs' },
      { id: 'grid', label: 'Grid', source: 'src/pages/grid.rs' },
      { id: 'list', label: 'List', source: 'src/pages/list.rs' },
      { id: 'list-item-100', label: 'List · programmatic scrolling', source: 'src/pages/list.rs' },
      { id: 'localization', label: 'Localization', source: 'src/pages/localization.rs' },
      { id: 'media', label: 'Media playback', source: 'src/pages/media.rs' },
      { id: 'menus', label: 'Menus & dialogs', source: 'src/pages/menus.rs' },
      // The preferences singleton: a real OS window on desktop, a fullscreen cover where the
      // backend has no secondary windows — the same ids either way (docs/windows.md).
      { id: 'preferences', label: 'Menus & dialogs · preferences window', source: 'src/pages/preferences.rs' },
      { id: 'services', label: 'Platform services', source: 'src/pages/services.rs' },
      { id: 'refresh', label: 'Refresh', source: 'src/pages/refresh.rs' },
      { id: 'resources', label: 'Resources', source: 'src/pages/resources.rs' },
      { id: 'stack-detail', label: 'Stack', source: 'src/pages/stack.rs' },
      // Every backend presents the fullscreen cover — native modal on mobile, topmost child
      // elsewhere (docs/cover.md). Driven from the Stack page.
      { id: 'cover', label: 'Stack · fullscreen cover', source: 'src/pages/stack.rs' },
      { id: 'tabs-one', label: 'Tabs', source: 'src/pages/tabs.rs' },
      { id: 'text', label: 'Text', source: 'src/pages/text.rs' },
      { id: 'textareas', label: 'Text areas', source: 'src/pages/text_areas.rs' },
      { id: 'toolbars', label: 'Toolbars', source: 'src/pages/toolbars.rs' },
      // The toolbar's search field filtering the sidebar by localized word-prefix — the one shot
      // that shows the nav responding to a query (docs/localization.md "Searching").
      { id: 'toolbars-filtered', label: 'Toolbars · sidebar search', source: 'src/pages/toolbars.rs' },
      { id: 'tweaks', label: 'Tweaks', source: 'src/pages/tweaks.rs' },
      { id: 'webview', label: 'Web view', source: 'src/pages/webview.rs' },
    ],
  },
];

export default { platforms, suites };
