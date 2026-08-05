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
  },
  {
    id: 'macos-gtk',
    os: 'macOS',
    toolkit: 'GTK 4',
    toolkitLong: 'GTK 4 · libadwaita',
    shellKind: 'chrome',
    shell: 'macos',
  },
  {
    id: 'macos-qt',
    os: 'macOS',
    toolkit: 'Qt 6',
    toolkitLong: 'Qt 6 Widgets',
    shellKind: 'chrome',
    shell: 'macos',
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
  },
  {
    id: 'linux-gtk',
    os: 'Linux',
    toolkit: 'GTK 4',
    toolkitLong: 'GTK 4 · libadwaita',
    shellKind: 'chrome',
    shell: 'gnome',
    primary: true,
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
  },
  {
    id: 'windows-xaml',
    os: 'Windows',
    toolkit: 'XAML',
    toolkitLong: 'XAML Islands',
    shellKind: 'chrome',
    shell: 'windows',
    primary: true,
  },
  {
    id: 'windows-gtk',
    os: 'Windows',
    toolkit: 'GTK 4',
    toolkitLong: 'GTK 4 · libadwaita',
    shellKind: 'chrome',
    shell: 'windows',
  },
  {
    id: 'windows-qt',
    os: 'Windows',
    toolkit: 'Qt 6',
    toolkitLong: 'Qt 6 Widgets',
    shellKind: 'chrome',
    shell: 'windows',
  },
  {
    id: 'harmony-arkui',
    os: 'HarmonyOS',
    toolkit: 'ArkUI',
    toolkitLong: 'ArkUI · NodeAPI',
    shellKind: 'bezel',
    shell: 'harmony',
    primary: true,
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
  },
]);

/** By id, for the lookups every consumer does. */
export const platformsById = /** @type {Record<string, Platform>} */ (
  Object.fromEntries(platforms.map((p) => [p.id, p]))
);

/** The eight the landing grid shows. */
export const primaryPlatforms = platforms.filter((p) => p.primary);

/** The window decoration for a desktop/web target, if it has one. */
export const chromeOf = (/** @type {string} */ id) =>
  platformsById[id]?.shellKind === 'chrome' ? platformsById[id].shell : undefined;

/** The hardware bezel for a phone-class target, if it has one. */
export const bezelOf = (/** @type {string} */ id) =>
  platformsById[id]?.shellKind === 'bezel' ? platformsById[id].shell : undefined;
