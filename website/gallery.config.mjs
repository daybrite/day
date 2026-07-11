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

/** @typedef {{ id: string, label: string, os: string, toolkit: string }} Platform */
/** `source` is the repo-relative path of the code that renders the shot (linked from the row
 *  header, e.g. `apps/showcase/src/pages/controls.rs`).
 *  @typedef {{ id: string, label: string, source?: string }} Shot */

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
  { id: 'ohos-arkui', label: 'ArkUI', os: 'HarmonyOS', toolkit: 'ArkUI · NodeAPI' },
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
      'One Rust program showing every implemented Piece, rendered with native widgets on each target.',
    // `{platform}` is substituted with the platform id.
    artifactPattern: 'screenshots-{platform}',
    // The capture variants, in display order. `dirs` are the artifact subdirectories that may
    // hold the variant (fallbacks cover pre-variant artifacts, which used locale names); dark
    // deliberately has NO light fallback — assembly must never pass off a light shot as dark
    // (the gallery page falls back VISIBLY instead).
    variants: [
      { id: 'light', label: 'Light', dirs: ['light', 'default', 'en'] },
      { id: 'dark', label: 'Dark', dirs: ['dark'] },
      { id: 'fr', label: 'Français', dirs: ['fr'] },
    ],
    // Every target the showcase is captured on, HarmonyOS/ArkUI included (the same app + walkthrough
    // runs on the Oniro emulator in CI; pieces without an ArkUI renderer show as placeholders there).
    platforms: platforms.map((p) => p.id),
    hero: 'home',
    shots: [
      { id: 'home', label: 'Home', source: 'apps/showcase/src/lib.rs' },
      { id: 'controls', label: 'Controls form', source: 'apps/showcase/src/pages/controls.rs' },
      { id: 'canvas', label: 'Canvas & shapes', source: 'apps/showcase/src/pages/canvas.rs' },
      { id: 'system', label: 'Device & sensors', source: 'apps/showcase/src/pages/system.rs' },
      { id: 'services', label: 'Platform services', source: 'apps/showcase/src/pages/services.rs' },
      { id: 'modals', label: 'Dialogs', source: 'apps/showcase/src/pages/modals.rs' },
      { id: 'tabs-one', label: 'Tabs', source: 'apps/showcase/src/pages/tabs.rs' },
      { id: 'stack-detail', label: 'Navigation stack', source: 'apps/showcase/src/pages/stack.rs' },
      { id: 'resources', label: 'Bundled resources', source: 'apps/showcase/src/pages/resources.rs' },
      { id: 'webview', label: 'Web view', source: 'apps/showcase/src/pages/webview.rs' },
      { id: 'tweaks', label: 'Tweaks (native config)', source: 'apps/showcase/src/pages/tweaks.rs' },
      { id: 'text', label: 'Typography & custom fonts', source: 'apps/showcase/src/pages/text.rs' },
      { id: 'about', label: 'About', source: 'apps/showcase/src/pages/about.rs' },
    ],
  },
];

export default { platforms, suites };
