---
title: "Cursor"
description: "The .cursor() decorator: one pointer-shape vocabulary, each toolkit's realization, the reactive form, and the shapes only one toolkit names."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Cursor (the pointer's shape over a piece)

> **Status: shipped** (2026-09) as `Decorate::cursor`, `Toolkit::set_cursor`, and `Cap::Cursor`.
> Seven backends draw it; harmony-arkui answers `Unsupported` for now, because HarmonyOS names
> its pointer styles only in ArkTS and the toolkit has no call path into ArkTS yet. Custom
> cursors from `resource/cursors/` are a planned second phase and are not in the tree.

## Authoring

```rust
use day::prelude::*;

link_row.cursor(Cursor::Pointer)
canvas.cursor(Cursor::Crosshair)
divider.cursor(Cursor::ColResize)

// Reactive: a tool palette, or a busy state. A constant, a Signal<Cursor>, or a closure.
let tool = Signal::new(Tool::Pen);
canvas.cursor(move || match tool.get() {
    Tool::Pen => Cursor::Crosshair,
    Tool::Pan => Cursor::Grab,
})

// A shape only one toolkit names, compiled only under that backend's feature.
#[cfg(feature = "appkit")]
trash.cursor(day::cursor::appkit::DISAPPEARING_ITEM)

// Anything a backend names that Day does not; ignored everywhere else.
row.cursor(Cursor::native("screenshot-cursor"))
```

Three rules:

- The cursor applies to the piece's realized widget and its descendants, until a descendant
  sets its own. The nearest ancestor wins.
- `Cursor::Default` releases the piece to the platform rather than pinning an arrow, so a
  reactive cursor can hand back what it took.
- A shape is a pointer affordance and nothing else. A touch screen never shows one, and that
  is correct; an affordance that only a cursor reveals (a resize handle with no visible grip)
  should check `capability(Cap::Cursor)` before it counts on being discovered.

## The vocabulary

`Cursor` is the CSS cursor vocabulary as an enum, plus one open variant. CSS names are the
largest set any toolkit accepts as they are (GTK and the DOM take them verbatim), and every other
toolkit's set is a subset of them or maps onto them, so the enum is the contract and the table
below is the only mapping written down.

| Variant | CSS name | Meaning |
|---|---|---|
| `Default` | `default` | the platform's arrow; also releases the piece |
| `Pointer` | `pointer` | a pointing hand, for links and tap-to-activate content |
| `Text`, `VerticalText` | `text`, `vertical-text` | an I-beam |
| `Crosshair` | `crosshair` | |
| `Move` | `move` | four-way move |
| `Grab`, `Grabbing` | `grab`, `grabbing` | an open and a closed hand |
| `NotAllowed` | `not-allowed` | |
| `Wait`, `Progress` | `wait`, `progress` | busy; `Progress` still takes input |
| `Help` | `help` | |
| `ContextMenu` | `context-menu` | |
| `Copy`, `Alias` | `copy`, `alias` | a drop will copy or link |
| `Cell` | `cell` | a table cell |
| `ZoomIn`, `ZoomOut` | `zoom-in`, `zoom-out` | |
| `None` | `none` | no pointer at all |
| `NsResize`, `EwResize` | `ns-resize`, `ew-resize` | vertical and horizontal resize |
| `NeswResize`, `NwseResize` | `nesw-resize`, `nwse-resize` | diagonal resize |
| `ColResize`, `RowResize` | `col-resize`, `row-resize` | a divider between columns or rows |
| `Native(name)` | the name | one toolkit's own shape, see below |

`Cursor::css_name()` answers the keyword, and `Cursor::NAMED` walks the named set in order,
which is what the Showcase's Cursors page draws.

## Per toolkit

| Toolkit | Mechanism | `Cap::Cursor` | What differs |
|---|---|---|---|
| macos-appkit | an `NSTrackingArea` with `cursorUpdate` per view, owned by a small object that sets the `NSCursor` | Native | no wait, progress, help, or move shape (the arrow, or the open hand for move); zoom, diagonal frame resize, and column/row resize need macOS 15 and fall back below it; `None` hides the pointer while inside |
| gtk | `widget.set_cursor(gdk::Cursor::from_name(css))` | Native | the theme's own drawings; a name the theme lacks falls back to its arrow |
| qt | `QWidget::setCursor(Qt::CursorShape)` | Emulated | no zoom, cell, context-menu, or vertical-text shape; each takes its nearest (`Help` is What's This, `Progress` is Busy) |
| xaml | `PointerEntered`/`PointerExited` on the element record the shape; the host window answers `WM_SETCURSOR` with a Win32 `IDC_*` cursor | Emulated | no grab, zoom, column/row resize, copy, alias, cell, or vertical-text shape; nearest of the classic set. UWP XAML in a Win32 host has no per-element cursor property, which is why the host owns it |
| web-dom | an inline `cursor:` style on the element | Native | the browser's own set; `Default` removes the inline style so the stylesheet's affordance rules apply again |
| android-mdc | `View.setPointerIcon(PointerIcon.getSystemIcon(…))`, API 24 | Native | the full vocabulary; visible only with a mouse or trackpad, on ChromeOS, or in desktop windowing |
| ios-uikit | a `UIPointerInteraction` whose delegate answers a `UIPointerStyle` | Emulated | iPadOS draws pointer effects, not arrows: `Text` and `VerticalText` become a beam, `Pointer`, `Grab`, `Grabbing`, `Copy`, `Alias`, and `ContextMenu` the highlight effect, `Move` the lift effect, `None` hides the pointer, everything else keeps the system pointer |
| harmony-arkui | none yet | Unsupported | `pointer.setPointerStyle` exists only in ArkTS; wiring a hover hook through an ArkTS helper is the follow-up |
| mock | records the last cursor per widget | Native | probe-visible in tests |

The duty and its implementors are in [duty-matrix.md](duty-matrix.md); the capability answers
in [coverage-matrix.md](coverage-matrix.md). Both are generated and diffed in CI.

## Shapes only one toolkit names

The `day::cursor` module carries them, one submodule per toolkit, each gated on the facade's
backend feature:

| Module | Constants |
|---|---|
| `day::cursor::appkit` | `DISAPPEARING_ITEM`, `DRAG_COPY`, `DRAG_LINK`, `CONTEXTUAL_MENU`, `I_BEAM_VERTICAL` |
| `day::cursor::qt` | `WHATS_THIS`, `BUSY`, `UP_ARROW`, `SPLIT_H`, `SPLIT_V` |
| `day::cursor::xaml` | `PERSON`, `PIN`, `UP_ARROW`, `APP_STARTING` |
| `day::cursor::android` | `ALL_SCROLL`, `NO_DROP`, `TOP_RIGHT_DIAGONAL`, `TOP_LEFT_DIAGONAL` |

Each is a `Cursor::Native` value, so the named backend resolves it by its own table and any
other ignores it. The gate is the point: a name only AppKit has is a compile error in a GTK
build rather than a silent arrow, and the app wraps the use in the same `#[cfg(feature = …)]`.
`Cursor::native("…")` is the ungated form for a name Day has no constant for.

## How it reaches the widget

`Decorate::cursor` pushes an op that, after the inner piece is built, calls
`TreeOps::set_node_cursor`, which forwards to `Toolkit::set_cursor` on the node's handle. A
reactive source also binds a closure that calls the same setter on change. The setter is
idempotent and cheap on every toolkit, so there is no patch enum and no extra native node; a
layout-only inner piece has no widget of its own and the op resolves the nearest realized node
the way `.selectable()` does.

## Trying it

The Showcase's **Cursors** page (Controls group): every named shape as a tile to hover, one box
whose shape follows a picker, a nested pair that shows the nearest ancestor winning, and the
toolkit's own extras.

```
day launch -p macos-appkit --script dayscript/cursors.yaml
```

The walkthrough asserts the support row and drives the picker. The shape itself is the
acceptance test, and it needs a person with a mouse: on macOS, in a browser, on the Android
emulator with a mouse attached, and on the iPad simulator with pointer support enabled.
