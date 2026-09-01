---
title: Reference index
description: The per-widget and per-subsystem reference pages, straight from the framework's own docs.
order: 61
section: Reference
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

The pages linked here are the framework's internal reference documentation, published as-is from
the repository's `docs/` directory. They're terser than the guides (per-API detail, per-platform
support tables, edge cases), and they're the same files Day's own developers keep current, so
they tend to lead the guides when something changes.

<!-- BEGIN GENERATED: internal-docs-index (integrations/gallery.mjs, from src/lib/internal-groups.mjs) -->
## Core & UI

Framework-level UI: navigation, lists, layout containers, drawing, text, and the cross-cutting concerns every app touches.

| Page | Covers |
|---|---|
| [text](/docs/internal/text) | labels, fonts, semantic styles, wrapping |
| [text-runs](/docs/internal/text-runs) | styled runs inside one label |
| [markdown](/docs/internal/markdown) | inline Markdown in labels |
| [buttons](/docs/internal/buttons) | button styles per backend |
| [navigation](/docs/internal/navigation) | selector/stack mapping per platform, routes |
| [deep-links](/docs/internal/deep-links) | custom URL schemes, delivery, launcher shortcuts |
| [dialogs](/docs/internal/dialogs) | alert/confirm/prompt, native presentation, results |
| [menus](/docs/internal/menus) | app menu bar, context menus, roles and shortcuts |
| [toolbars](/docs/internal/toolbars) | window toolbars: the item vocabulary, symbol icons, per-desktop realization |
| [windows](/docs/internal/windows) | secondary windows, the Preferences window, the cover fallback |
| [window-image](/docs/internal/window-image) | capturing the app's own window as a PNG |
| [grid](/docs/internal/grid) | the eager grid: rows, spans, flexible columns |
| [forms](/docs/internal/forms) | form/section/labeled groupings |
| [baseline](/docs/internal/baseline) | baseline alignment across toolkits |
| [size-classes](/docs/internal/size-classes) | size classes and navigation re-presentation |
| [scroll](/docs/internal/scroll) | scrolling and programmatic scroll targets |
| [search](/docs/internal/search) | searchable() surfaces per platform |
| [cover](/docs/internal/cover) | fullscreen covers and dismissal control |
| [inspector](/docs/internal/inspector) | the trailing properties pane: native splits, the compact sheet |
| [focus](/docs/internal/focus) | keyboard focus as a signal: bindings, rules, per-backend map |
| [list](/docs/internal/list) | the native recycling list: row protocol, heights, selection |
| [tree](/docs/internal/tree) | the hierarchical tree: nesting, expansion, drag-to-reparent (plan) |
| [canvas](/docs/internal/canvas) | the canvas display list and gestures |
| [shapes](/docs/internal/shapes) | canvas drawing, shape pieces, gestures |
| [progress](/docs/internal/progress) | determinate bars and spinners |
| [picker](/docs/internal/picker) | the built-in one-of-N picker: menu, segmented, and inline styles |
| [textarea](/docs/internal/textarea) | multi-line text: editing, selection, spell-check |
| [texteditor](/docs/internal/texteditor) | `day-piece-texteditor` — editing a StyledText in each platform's rich-text view |
| [localization](/docs/internal/localization) | Fluent mechanics, arguments, fallback |
| [accessibility](/docs/internal/accessibility) | roles, per-backend attribute mapping, the audit |
| [lifecycle](/docs/internal/lifecycle) | app phases and their per-platform availability |
| [async](/docs/internal/async) | tasks, resources, and the runtime rules |
| [state](/docs/internal/state) | where state lives: per-window and app-wide Ambient values, the focused-window rule |
| [model](/docs/internal/model) | the per-property observable store and the Observable derive |
| [persistence](/docs/internal/persistence) | SQLite storage for the model: ModelContainer, the Model derive, migrations |
| [resources](/docs/internal/resources) | asset packaging and the zero-copy runtime path |
| [vectors](/docs/internal/vectors) | resolution-independent SVG glyphs and the `vector` piece |
| [color](/docs/internal/color) | the `Color`/`Paint` currency, what a native picker returns, and a proposal to widen it |
| [icons](/docs/internal/icons) | `day icon`: every platform's app-icon set from one master |
| [files](/docs/internal/files) | file I/O and platform paths |

## Pieces

Standalone UI Pieces: native widgets that live in their own crates and plug in without any core changes.

| Page | Covers |
|---|---|
| [swiftui](/docs/internal/swiftui) | `day-piece-swiftui` — embed your own SwiftUI views (macOS, iOS) |
| [webview](/docs/internal/webview) | `day-piece-webview` — embedded web view, remote and bundled sites |
| [webview-eval](/docs/internal/webview-eval) | web view JavaScript evaluation: API and per-platform support |
| [map](/docs/internal/map) | `day-piece-map` — native maps |
| [media](/docs/internal/media) | `day-piece-media` — audio/video playback |
| [lottie](/docs/internal/lottie) | `day-piece-lottie` — Lottie animations |
| [combobox](/docs/internal/combobox) | `day-piece-combobox` — free-form text plus a native dropdown |
| [searchfield](/docs/internal/searchfield) | `day-piece-searchfield` — the search input |
| [activity](/docs/internal/activity) | `day-piece-activity` — activity spinners |
| [pullrefresh](/docs/internal/pullrefresh) | `day-piece-pullrefresh` — pull-to-refresh for scrollables |
| [datepicker](/docs/internal/datepicker) | `day-piece-datetime` — native date & time pickers |
| [colorpicker](/docs/internal/colorpicker) | `day-piece-colorpicker` — a color well: the platform chooser, or one Day composes |
| [stepper](/docs/internal/stepper) | `day-piece-stepper` — a numeric field with increment/decrement arrows |
| [badge](/docs/internal/badge) | app-icon numeric badge (proposed) |
| [tweaks](/docs/internal/tweaks) | per-toolkit native configuration: accessors, packaged tweaks, recipes |

## Parts

Headless capability crates, the non-UI counterpart of Pieces. They provide device and system access without any widgets.

| Page | Covers |
|---|---|
| [battery](/docs/internal/battery) | `day-part-battery` |
| [clipboard](/docs/internal/clipboard) | `day-part-clipboard` |
| [prefs](/docs/internal/prefs) | `day-part-prefs` |
| [fs](/docs/internal/fs) | `day-part-fs` |
| [notify](/docs/internal/notify) | `day-part-local-notify` (proposed successor design) |
| [network](/docs/internal/network) | `day-part-network` |
| [sensors](/docs/internal/sensors) | `day-part-sensors` |
| [haptics](/docs/internal/haptics) | `day-part-haptics` |
| [deviceinfo](/docs/internal/deviceinfo) | `day-part-deviceinfo` |
| [http](/docs/internal/http) | `day-part-http` |
| [permissions](/docs/internal/permissions) | `day-part-permissions` |
| [location](/docs/internal/location) | `day-part-location` |
| [timezone](/docs/internal/timezone) | `day-part-timezone` |
| [speech](/docs/internal/speech) | `day-part-speech`, also daybridge’s reference implementation |

## Platform & tooling

Platform backends, the extension model for writing your own Pieces, per-backend support matrices, API conventions, and tooling.

| Page | Covers |
|---|---|
| [harmonyos](/docs/internal/harmonyos) | OpenHarmony toolchain setup and quirks |
| [web](/docs/internal/web) | the `web-dom` backend — wasm build, dayscript bridge, static hosting |
| [extending](/docs/internal/extending) | piece registration internals |
| [bridge](/docs/internal/bridge) | daybridge: foreign-language arms of a Rust API |
| [coverage-matrix](/docs/internal/coverage-matrix) | which piece kinds each backend renders (generated, CI-gated) |
| [duty-matrix](/docs/internal/duty-matrix) | which backend implements which Toolkit duty (generated, CI-gated) |
| [recorder-matrix](/docs/internal/recorder-matrix) | event → recorded dayscript step coverage (generated, CI-gated) |
| [logging](/docs/internal/logging) | the `log` facade, levels, per-platform sinks, DAY_LOG, custom loggers |
| [break](/docs/internal/break) | `day-break` consent-first crash reporting |
| [lite](/docs/internal/lite) | `day-lite` JS/TS miniapps and superapp embedding |
| [store](/docs/internal/store) | store listings and `day store` |
| [agent](/docs/internal/agent) | dayscript sessions, `day drive`, and the agent-facing tooling |
| [api-style](/docs/internal/api-style) | the API design conventions Day itself follows |
| [vscode](/docs/internal/vscode) | editor setup |
| [environment](/docs/internal/environment) | toolchain/SDK discovery env vars (DAY_CPPWINRT, DAY_WINDOWS_KITS_ROOT, …) |
<!-- END GENERATED: internal-docs-index -->

If a guide and a reference page disagree, trust the reference page and tell us about the guide.
