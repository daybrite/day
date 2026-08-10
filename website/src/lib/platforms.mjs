// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// The platform-toolkit table: one record per `(OS, toolkit)` target, and the only place any of
// it is written down.
//
// Before this file the same twelve targets were spelled out four times — the gallery config, the
// landing page's grid, the showcase downloads, and the shell map — which is how `windows-xaml`
// ended up labelled "XAML" in one place, "XAML (system XAML)" in another, and "System XAML ·
// XAML Islands" in a third. Pages still own their own PROSE (a blurb, an install note); what
// lives here is the identity every page has to agree on.
//
// Plain `.mjs`, not `.ts`, on purpose: the Node build scripts (`scripts/hero-shots.mjs`,
// `scripts/assemble-gallery.mjs`, and `gallery.config.mjs` itself) import it directly, and they
// run outside Astro's TypeScript pipeline. JSDoc gives the site the same types either way.

/**
 * @typedef {object} Platform
 * @property {string}  id        Target key, e.g. `macos-appkit`. Matches `day build -p <id>`, the
 *                               CI artifact suffix, `src/icons/<id>.svg`, and `--pf-<id>`.
 *                               Exception: a `deviceClass` entry is a capture-only refinement of
 *                               another target and matches only its CI screenshot artifact.
 * @property {string} [deviceClass]  Set on capture-only entries — the same build as another
 *                               target, walked through on a different device class (`ipad`,
 *                               `tablet`). Gets its own gallery column and nothing else: no
 *                               package, no download row, no landing-grid card.
 * @property {string}  os        The OS, as a reader knows it: `macOS`, `iOS`, `Windows`.
 * @property {string}  toolkit   The toolkit as a card heading names it: `AppKit`, `Material
 *                               Components`, `XAML`.
 * @property {string} [chip]     The gallery's compact caption, where a row header has room for
 *                               two words at most. Defaults to `toolkit`; set only where the two
 *                               genuinely differ (`Material Components` → `Android`).
 * @property {string} [osShort]  The OS where space is tight. Defaults to `os`.
 * @property {string}  toolkitLong  The fuller form for captions, where the extra words earn their
 *                               space: `GTK 4 · libadwaita`. Equal to `toolkit` when there is no
 *                               longer form worth showing.
 * @property {'chrome'|'bezel'} [shellKind]  Which presentational shell the captures wear.
 * @property {string} [shell]    The shell variant: a window decoration (`macos`, `windows`,
 *                               `gnome`, `kde`, `browser`) or a hardware bezel (`iphone`,
 *                               `android`, `harmony`). Absent = the capture renders bare.
 * @property {boolean} [primary] One of the eight the landing page's "Runs natively on" grid
 *                               shows. The other four are the same toolkits on a second OS, which
 *                               the gallery covers but the grid would only repeat.
 * @property {1|2|3|4} tier      The support tier, defined in `tiers` below and documented at
 *                               /docs/platforms#support-tiers.
 */

/** Every CI target, in display order. */
export const platforms = /** @type {Platform[]} */ ([
  {
    id: 'macos-appkit',
    os: 'macOS',
    toolkit: 'AppKit',
    toolkitLong: 'AppKit',
    shellKind: 'chrome',
    shell: 'macos',
    primary: true,
    tier: 1,
  },
  {
    id: 'macos-gtk',
    os: 'macOS',
    toolkit: 'GTK 4',
    toolkitLong: 'GTK 4 · libadwaita',
    shellKind: 'chrome',
    shell: 'macos',
    tier: 4,
  },
  {
    id: 'macos-qt',
    os: 'macOS',
    toolkit: 'Qt 6',
    toolkitLong: 'Qt 6 Widgets',
    shellKind: 'chrome',
    shell: 'macos',
    tier: 4,
  },
  {
    id: 'ios-uikit',
    os: 'iOS & iPadOS',
    osShort: 'iOS',
    toolkit: 'UIKit',
    toolkitLong: 'UIKit',
    shellKind: 'bezel',
    shell: 'iphone',
    primary: true,
    tier: 1,
  },
  {
    id: 'ios-uikit-ipad',
    deviceClass: 'ipad',
    os: 'iOS',
    toolkit: 'UIKit',
    chip: 'iPad',
    toolkitLong: 'UIKit · iPad',
    tier: 1,
  },
  {
    id: 'android-mdc',
    os: 'Android',
    toolkit: 'Material Components',
    chip: 'Android',
    toolkitLong: 'Material Components',
    shellKind: 'bezel',
    shell: 'android',
    primary: true,
    tier: 1,
  },
  {
    id: 'android-mdc-tablet',
    deviceClass: 'tablet',
    os: 'Android',
    toolkit: 'Material Components',
    chip: 'Tablet',
    toolkitLong: 'Material Components · tablet',
    tier: 1,
  },
  {
    id: 'linux-gtk',
    os: 'Linux',
    toolkit: 'GTK 4',
    toolkitLong: 'GTK 4 · libadwaita',
    shellKind: 'chrome',
    shell: 'gnome',
    primary: true,
    tier: 2,
  },
  {
    id: 'linux-qt',
    os: 'Linux',
    toolkit: 'Qt 6 Widgets',
    chip: 'Qt 6',
    toolkitLong: 'Qt 6 Widgets',
    shellKind: 'chrome',
    shell: 'kde',
    primary: true,
    tier: 2,
  },
  {
    id: 'windows-xaml',
    os: 'Windows',
    toolkit: 'XAML',
    toolkitLong: 'XAML Islands',
    shellKind: 'chrome',
    shell: 'windows',
    primary: true,
    tier: 2,
  },
  {
    id: 'windows-gtk',
    os: 'Windows',
    toolkit: 'GTK 4',
    toolkitLong: 'GTK 4 · libadwaita',
    shellKind: 'chrome',
    shell: 'windows',
    tier: 4,
  },
  {
    id: 'windows-qt',
    os: 'Windows',
    toolkit: 'Qt 6',
    toolkitLong: 'Qt 6 Widgets',
    shellKind: 'chrome',
    shell: 'windows',
    tier: 4,
  },
  {
    id: 'harmony-arkui',
    os: 'HarmonyOS',
    toolkit: 'ArkUI',
    toolkitLong: 'ArkUI · NodeAPI',
    shellKind: 'bezel',
    shell: 'harmony',
    primary: true,
    tier: 3,
  },
  {
    id: 'web-dom',
    os: 'Web',
    toolkit: 'DOM',
    chip: 'Web DOM',
    toolkitLong: 'DOM · captured in WebKit',
    shellKind: 'chrome',
    shell: 'browser',
    primary: true,
    tier: 3,
  },
]);

/** By id, for the lookups every consumer does. */
export const platformsById = /** @type {Record<string, Platform>} */ (
  Object.fromEntries(platforms.map((p) => [p.id, p]))
);

/**
 * The support tiers, defined once here and explained at /docs/platforms#support-tiers. A tier
 * says how much testing and maintenance a target gets, not how complete its backend is: a Tier 4
 * target can render every piece and still be a development combination nobody ships.
 *
 * @typedef {object} Tier
 * @property {1|2|3|4} n     The tier number, as the badge and the docs write it.
 * @property {string} name   The tier's name: `Supported`, `Development`.
 * @property {string} blurb  One sentence on what the tier promises, for badge tooltips.
 */
export const tiers = /** @type {Tier[]} */ ([
  {
    n: 1,
    name: 'Supported',
    blurb: 'Fully supported and thoroughly tested; the highest attention to quality and correctness.',
  },
  {
    n: 2,
    name: 'Demi-supported',
    blurb: 'Very high priority, with less direct quality assurance and thorough testing than Tier 1.',
  },
  {
    n: 3,
    name: 'Experimental',
    blurb: 'Tested, but not comprehensively, and not yet exercised by real-world applications.',
  },
  {
    n: 4,
    name: 'Development',
    blurb: 'For compatibility testing and running one toolkit on a second OS; not meant for shipping apps.',
  },
]);

/** A tier by number, for the badge markup. */
export const tierOf = (/** @type {number} */ n) => tiers.find((t) => t.n === n);

/** The targets in a tier, in display order. */
export const platformsInTier = (/** @type {number} */ n) => platforms.filter((p) => p.tier === n);

/** The eight the landing grid shows. */
export const primaryPlatforms = platforms.filter((p) => p.primary);

/** The window decoration for a desktop/web target, if it has one. */
export const chromeOf = (/** @type {string} */ id) =>
  platformsById[id]?.shellKind === 'chrome' ? platformsById[id].shell : undefined;

/** The hardware bezel for a phone-class target, if it has one. */
export const bezelOf = (/** @type {string} */ id) =>
  platformsById[id]?.shellKind === 'bezel' ? platformsById[id].shell : undefined;
