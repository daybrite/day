// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// THE curation of the internal reference docs (`docs/*.md`): one ordered placement per doc,
// consumed by the /docs/internal index page, the reference-index generator
// (integrations/gallery.mjs), and the internal prev/next pager. `scripts/ci/doc-links.py`
// fails the build when a doc under `docs/` is missing from this file (or listed twice, or
// listed without existing), so a new doc must be placed here to land — which is exactly how
// twelve docs went missing from every hand-maintained index before this file existed.
//
// `note` is the one-line right-hand column on the reference index; the doc's own frontmatter
// `description` is the longer card/meta text.

export const groups = [
  {
    heading: 'Core & UI',
    blurb:
      'Framework-level UI: navigation, lists, layout containers, drawing, text, and the cross-cutting concerns every app touches.',
    docs: [
      ['text', 'labels, fonts, semantic styles, wrapping'],
      ['text-runs', 'styled runs inside one label'],
      ['markdown', 'inline Markdown in labels'],
      ['buttons', 'button styles per backend'],
      ['navigation', 'selector/stack mapping per platform, routes'],
      ['deep-links', 'custom URL schemes, delivery, launcher shortcuts'],
      ['dialogs', 'alert/confirm/prompt, native presentation, results'],
      ['menus', 'app menu bar, context menus, roles and shortcuts'],
      ['toolbars', 'window toolbars: the item vocabulary, symbol icons, per-desktop realization'],
      ['windows', 'secondary windows, the Preferences window, the cover fallback'],
      ['window-image', "capturing the app's own window as a PNG"],
      ['tabs', 'tabbed containers'],
      ['grid', 'the eager grid: rows, spans, flexible columns'],
      ['forms', 'form/section/labeled groupings'],
      ['baseline', 'baseline alignment across toolkits'],
      ['size-classes', 'size classes and navigation re-presentation'],
      ['scroll', 'scrolling and programmatic scroll targets'],
      ['search', 'searchable() surfaces per platform'],
      ['cover', 'fullscreen covers and dismissal control'],
      ['focus', 'keyboard focus as a signal: bindings, rules, per-backend map'],
      ['list', 'the native recycling list: row protocol, heights, selection'],
      ['canvas', 'the canvas display list and gestures'],
      ['shapes', 'canvas drawing, shape pieces, gestures'],
      ['progress', 'determinate bars and spinners'],
      ['picker', 'the built-in one-of-N picker: menu, segmented, and inline styles'],
      ['textarea', 'multi-line text: editing, selection, spell-check'],
      ['localization', 'Fluent mechanics, arguments, fallback'],
      ['accessibility', 'roles, per-backend attribute mapping, the audit'],
      ['lifecycle', 'app phases and their per-platform availability'],
      ['async', 'tasks, resources, and the runtime rules'],
      ['resources', 'asset packaging and the zero-copy runtime path'],
      ['vectors', 'resolution-independent SVG glyphs and the `vector` piece'],
      ['color', 'the `Color`/`Paint` currency, what a native picker returns, and a proposal to widen it'],
      ['icons', "`day icon`: every platform's app-icon set from one master"],
      ['files', 'file I/O and platform paths'],
    ],
  },
  {
    heading: 'Pieces',
    blurb:
      'Standalone UI Pieces: native widgets that live in their own crates and plug in without any core changes.',
    docs: [
      ['swiftui', '`day-piece-swiftui` — embed your own SwiftUI views (macOS, iOS)'],
      ['webview', '`day-piece-webview` — embedded web view, remote and bundled sites'],
      ['webview-eval', 'web view JavaScript evaluation: API and per-platform support'],
      ['map', '`day-piece-map` — native maps'],
      ['media', '`day-piece-media` — audio/video playback'],
      ['lottie', '`day-piece-lottie` — Lottie animations'],
      ['combobox', '`day-piece-combobox` — free-form text plus a native dropdown'],
      ['searchfield', '`day-piece-searchfield` — the search input'],
      ['activity', '`day-piece-activity` — activity spinners'],
      ['pullrefresh', '`day-piece-pullrefresh` — pull-to-refresh for scrollables'],
      ['datepicker', '`day-piece-datetime` — native date & time pickers'],
      ['colorpicker', '`day-piece-colorpicker` — a color well: the platform chooser, or one Day composes'],
      ['badge', 'app-icon numeric badge (proposed)'],
      ['tweaks', 'per-toolkit native configuration: accessors, packaged tweaks, recipes'],
    ],
  },
  {
    heading: 'Parts',
    blurb:
      'Headless capability crates, the non-UI counterpart of Pieces. They provide device and system access without any widgets.',
    docs: [
      ['battery', '`day-part-battery`'],
      ['clipboard', '`day-part-clipboard`'],
      ['prefs', '`day-part-prefs`'],
      ['fs', '`day-part-fs`'],
      ['notify', '`day-part-local-notify` (proposed successor design)'],
      ['network', '`day-part-network`'],
      ['sensors', '`day-part-sensors`'],
      ['haptics', '`day-part-haptics`'],
      ['deviceinfo', '`day-part-deviceinfo`'],
      ['http', '`day-part-http`'],
      ['permissions', '`day-part-permissions`'],
      ['location', '`day-part-location`'],
      ['timezone', '`day-part-timezone`'],
      ['speech', '`day-part-speech`, also daybridge’s reference implementation'],
    ],
  },
  {
    heading: 'Platform & tooling',
    blurb:
      'Platform backends, the extension model for writing your own Pieces, per-backend support matrices, API conventions, and tooling.',
    docs: [
      ['harmonyos', 'OpenHarmony toolchain setup and quirks'],
      ['web', 'the `web-dom` backend — wasm build, dayscript bridge, static hosting'],
      ['extending', 'piece registration internals'],
      ['bridge', 'daybridge: foreign-language arms of a Rust API'],
      ['coverage-matrix', 'which piece kinds each backend renders (generated, CI-gated)'],
      ['duty-matrix', 'which backend implements which Toolkit duty (generated, CI-gated)'],
      ['recorder-matrix', 'event → recorded dayscript step coverage (generated, CI-gated)'],
      ['break', '`day-break` consent-first crash reporting'],
      ['lite', '`day-lite` JS/TS miniapps and superapp embedding'],
      ['store', 'store listings and `day store`'],
      ['agent', 'dayscript sessions, `day drive`, and the agent-facing tooling'],
      ['api-style', 'the API design conventions Day itself follows'],
      ['vscode', 'editor setup'],
      ['environment', 'toolchain/SDK discovery env vars (DAY_CPPWINRT, DAY_WINDOWS_KITS_ROOT, …)'],
    ],
  },
];

/** Every curated id, in display order — the internal docs' canonical sequence. */
export const orderedIds = groups.flatMap((g) => g.docs.map(([id]) => id));
