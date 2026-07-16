---
title: Reference index
description: The per-widget and per-subsystem reference pages, straight from the framework's own docs.
order: 61
section: Reference
---

The pages linked here are the framework's internal reference documentation, published as-is from
the repository's `docs/` directory. They're terser than the guides — per-API detail, per-platform
support tables, edge cases — and they're the same files Day's own developers keep current, so
they tend to lead the guides when something changes.

## Subsystems

| Page | Covers |
|---|---|
| [text](/docs/internal/text) | labels, fonts, semantic styles, wrapping |
| [navigation](/docs/internal/navigation) | selector/stack mapping per platform, deep links |
| [dialogs](/docs/internal/dialogs) | alert/confirm/prompt, native presentation, results |
| [menus](/docs/internal/menus) | app menu bar, context menus, roles and shortcuts |
| [tabs](/docs/internal/tabs) | tabbed containers |
| [focus](/docs/internal/focus) | keyboard focus as a signal: bindings, rules, per-backend map |
| [list](/docs/internal/list) | the native recycling list: row protocol, heights, selection |
| [shapes](/docs/internal/shapes) | canvas drawing, shape pieces, gestures |
| [progress](/docs/internal/progress) | determinate bars and spinners |
| [picker](/docs/internal/picker) | date/color/file pickers |
| [searchfield](/docs/internal/searchfield) | the search input |
| [localization](/docs/internal/localization) | Fluent mechanics, arguments, fallback |
| [accessibility](/docs/internal/accessibility) | roles, per-backend attribute mapping, the audit |
| [lifecycle](/docs/internal/lifecycle) | app phases and their per-platform availability |
| [resources](/docs/internal/resources) | asset packaging and the zero-copy runtime path |
| [files](/docs/internal/files) | file I/O and platform paths |
| [extending](/docs/internal/extending) | piece registration internals |
| [tweaks](/docs/internal/tweaks) | per-toolkit native configuration: accessors, packaged tweaks, recipes |
| [api-style](/docs/internal/api-style) | the API design conventions Day itself follows |
| [vscode](/docs/internal/vscode) | editor setup |
| [environment](/docs/internal/environment) | toolchain/SDK discovery env vars (DAY_CPPWINRT, DAY_WINDOWS_KITS_ROOT, …) |
| [harmonyos](/docs/internal/harmonyos) | OpenHarmony toolchain setup and quirks |

## Optional pieces

| Page | Piece crate |
|---|---|
| [webview](/docs/internal/webview) | `day-piece-webview` — embedded web view |
| [map](/docs/internal/map) | `day-piece-map` — native maps |
| [media](/docs/internal/media) | `day-piece-media` — audio/video playback |
| [lottie](/docs/internal/lottie) | `day-piece-lottie` — Lottie animations |
| [activity](/docs/internal/activity) | `day-piece-activity` — activity rings |

## Parts

| Page | Part crate |
|---|---|
| [battery](/docs/internal/battery) | `day-part-battery` |
| [clipboard](/docs/internal/clipboard) | `day-part-clipboard` |
| [prefs](/docs/internal/prefs) | `day-part-prefs` |
| [network](/docs/internal/network) | `day-part-network` |
| [sensors](/docs/internal/sensors) | `day-part-sensors` |
| [haptics](/docs/internal/haptics) | `day-part-haptics` |
| [deviceinfo](/docs/internal/deviceinfo) | `day-part-deviceinfo` |

If a guide and a reference page disagree, trust the reference page and tell us about the guide.
