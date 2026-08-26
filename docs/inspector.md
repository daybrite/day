---
title: "Inspector"
description: "The inspector piece: window content beside a trailing properties panel, one visibility signal, a side pane on wide windows and a sheet on compact ones."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Inspector (`inspector`)

> **Status: implemented** (2026-08). Native trailing pane on macos-appkit (a real inspector
> `NSSplitViewItem`), gtk (`AdwOverlaySplitView`, sidebar at the end), qt (the nav `QSplitter`
> mirrored), and xaml (`SplitView`, pane placed right); composed pane + compact sheet on
> ios-uikit, android-mdc, harmony-arkui, web-dom, and mock (`Cap::Inspector = Unsupported`
> there — the piece itself supplies the fallback). Exercised end-to-end by Day-Sketch's
> walkthrough.

An `inspector` places a properties panel on the window's trailing edge, beside the content —
the Keynote/Pages shape: selection on the left, its editable attributes on the right. Like
every Day surface it is a projection of an app-owned signal, not an imperative controller:

```rust
let show = Signal::global(false);
inspector(show, editor(), || form((section((/* property rows */,)),)))
// show:  show.set(true);      — from a toolbar toggle, a menu item, a button
// hide:  show.set(false);
```

- `visible` is any `Binding<bool>`. Every affordance that shows or hides the panel writes the
  same signal, so a toolbar toggle, a menu item, and a button in the content can coexist and
  stay in step — bind a [`toolbar_toggle`](toolbars.md) to it and relabel a menu item from it.
- `content` is the window content the panel sits beside. `panel` is a **builder**, because the
  panel can be re-homed: a compact window presents it inside a fullscreen sheet instead of a
  side pane, and each home builds it fresh in its own scope. The piece wraps the panel in a
  `scroll` on every path.
- `.width(pt)` sets the pane's preferred width (default 280). Where the native pane has a
  user-draggable divider this is the initial width, not a limit.
- `.sheet_done(label)` names the compact sheet's dismiss button (default `✕` — day carries no
  "Done" of its own, so pass a localized one).
- `.edge(PaneEdge::Leading)` puts the pane on the LEADING side of the content instead — a
  utility pane like a layer panel ([docs/tree.md](tree.md)) rather than a properties
  inspector. Default `PaneEdge::Trailing`. On AppKit the leading pane is a plain pinned
  split item (not the system inspector item, whose treatment is trailing-specific); the
  composed form mounts the pane before the content. Two inspectors nest — Day Sketch wraps
  its trailing-inspector editor in a leading layers pane.

## Wide and compact

On a window wider than [`WidthClass::Compact`](size-classes.md) the panel is a side pane. On a
compact window there is no room for one, so while `visible` is true the panel presents as a
fullscreen sheet instead — an unrouted [`cover`](cover.md), so it claims no route segments and
a restored session never reopens a modal. The sheet carries its own dismiss button; the
system back gesture dismisses it too, writing `false` straight back into the app's signal.
Resizing across the breakpoint re-homes the panel automatically in both directions, because
the sheet's open state is derived from `visible` **and** the window's size class rather than
stored anywhere.

The native desktop panes do not re-home on a narrow window — a desktop window dragged narrow
squeezes its split, the same rule the nav sidebar follows.

## Per-backend presentation

| backend | `Cap::Inspector` | pane |
|---|---|---|
| macos-appkit | Native | `NSSplitViewItem` **inspector** item (`inspectorWithViewController:`), full-height under the titlebar. The panel paints an opaque window-background backdrop over the item's vibrancy material — the vibrant variants of the system fills only composite correctly in `allowsVibrancy` views, so grouped cards on the raw material came out near-black in dark mode (and materials capture black offscreen). Width pinned; visibility is Day's alone (the item cannot be user-collapsed). |
| linux-gtk / macos-gtk | Native | `AdwOverlaySplitView` with `sidebar-position=end`, width pinned (GNOME has no draggable sidebars), `show-sidebar` animated. |
| linux-qt / macos-qt | Native | The nav `QSplitter` mirrored: content pane stretches, panel pane trailing at its preferred width, divider draggable. Not a `QDockWidget` — `DayWindow` is a plain `QWidget` with hand-managed chrome, and dock areas need `QMainWindow` (the same trade the toolbar records in the shim). |
| windows-xaml | Native | `SplitView` with `PanePlacement=Right`, `DisplayMode=Inline`: the pane sits beside the content, never over it. |
| ios-uikit, android-mdc, harmony-arkui, web-dom, mock | Unsupported | The piece composes the pane from plain containers — same panel, same signal, a divider but no drag — and presents the compact sheet described above. |

On the native tier the `INSPECTOR` node's two `INSPECTOR_PANE` children have **native-owned
frames**: the split sizes each pane and reports it via `Event::FrameChanged`, and Day lays the
pane's content out inside the reported size — the nav-page contract
([docs/navigation.md](navigation.md)). A backend has to report on the split's own layout
passes, not only when content is inserted or the divider is dragged: at insert time the split
usually has whatever geometry its constructor left it, and content laid out against those
numbers stays that size until something else happens to resize the host.
`InspectorPatch::Visible` shows and hides the pane without a rebuild; a native affordance
hiding it (none of the current four has one) reports back as `Event::InspectorChanged`, which
must never re-fire for a Day-driven patch (the from-native echo rule).

## Extending the panel

The panel is ordinary Day content — `form`/`section`/`labeled` rows bound to the app's model.
Keep each property row's binding small and selection-aware: read the common value of the
selection, write through the model's own commit path so an edit is one undo unit. Day-Sketch's
`src/inspector.rs` is the reference: a property table (label + get + set), each rendered as a
`labeled(text_field(...))` row, with a shared `Binding<String>` implementation that shows a
`multi` placeholder when the selection disagrees and fans a typed value out to every selected
node in one undo turn.
