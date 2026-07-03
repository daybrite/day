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
// Each CI job uploads an artifact `screenshots-<platform>` containing `<locale>/<shot>.png`
// (crates/day-cli/src/script.rs). `artifactPattern` maps a (suite, platform) pair to that
// artifact name, so a future suite that uploads `screenshots-widgets-<platform>` only needs its
// own `artifactPattern`. The assembly prefers `preferLocales` in order, then any locale present.

/** @typedef {{ id: string, label: string, os: string, toolkit: string }} Platform */
/** @typedef {{ id: string, label: string }} Shot */

/** The ten CI targets, grouped by OS for display. Order here is display order. */
export const platforms = /** @type {Platform[]} */ ([
  { id: 'macos-appkit', label: 'AppKit', os: 'macOS', toolkit: 'AppKit' },
  { id: 'macos-gtk', label: 'GTK 4', os: 'macOS', toolkit: 'GTK 4 · libadwaita' },
  { id: 'macos-qt', label: 'Qt 6', os: 'macOS', toolkit: 'Qt 6 Widgets' },
  { id: 'ios-uikit', label: 'UIKit', os: 'iOS', toolkit: 'UIKit' },
  { id: 'android-widget', label: 'android.widget', os: 'Android', toolkit: 'android.widget' },
  { id: 'linux-gtk', label: 'GTK 4', os: 'Linux', toolkit: 'GTK 4 · libadwaita' },
  { id: 'linux-qt', label: 'Qt 6', os: 'Linux', toolkit: 'Qt 6 Widgets' },
  { id: 'windows-winui', label: 'WinUI 3', os: 'Windows', toolkit: 'WinUI 3' },
  { id: 'windows-gtk', label: 'GTK 4', os: 'Windows', toolkit: 'GTK 4 · libadwaita' },
  { id: 'windows-qt', label: 'Qt 6', os: 'Windows', toolkit: 'Qt 6 Widgets' },
]);

/**
 * Screenshot suites. Today just the Showcase app; the shape scales to more sample apps and to
 * per-component snapshot sets (add another entry with its own `artifactPattern` + `shots`).
 * @type {{ id: string, label: string, blurb: string, artifactPattern: string,
 *          preferLocales: string[], platforms: string[], hero: string, shots: Shot[] }[]}
 */
export const suites = [
  {
    id: 'showcase',
    label: 'Day Showcase',
    blurb:
      'One Rust program — every implemented Piece — realized with real native widgets on each target.',
    // `{platform}` is substituted with the platform id.
    artifactPattern: 'screenshots-{platform}',
    preferLocales: ['default', 'en', 'fr'],
    platforms: platforms.map((p) => p.id),
    hero: 'home',
    shots: [
      { id: 'home', label: 'Home' },
      { id: 'controls', label: 'Controls' },
      { id: 'gauge', label: 'Canvas gauge' },
      { id: 'modals', label: 'Dialogs' },
      { id: 'tabs-one', label: 'Tabs' },
      { id: 'stack-detail', label: 'Navigation stack' },
      { id: 'about', label: 'About' },
    ],
  },
];

export default { platforms, suites };
