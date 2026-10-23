---
navTitle: "Inspectors"
title: "Inspector"
description: "The inspector piece: window content beside a trailing properties panel, one visibility signal, a side pane on wide windows and a sheet on compact ones."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Inspector

Show editable properties beside the selected content. An [`inspector`](https://daybrite.dev/api/day_pieces/fn.inspector.html) puts the panel at the
window’s trailing edge and presents it as a sheet in compact layouts. A shared visibility
signal keeps toolbar, menu, and in-content controls in sync.

```rust
let show = Signal::global(false);
inspector(show, editor(), || form((section((/* property rows */,)),)))
// show:  show.set(true);      — from a toolbar toggle, a menu item, a button
// hide:  show.set(false);
```

- `visible` is any `Binding<bool>`. Every affordance that shows or hides the panel writes the
  same signal, so a toolbar toggle, a menu item, and a button in the content can coexist and
  stay in step; bind a [`toolbar_toggle`](toolbars.md) to it and relabel a menu item from it.
- `content` is the window content the panel sits beside. `panel` is a **builder**, because the
  panel can be re-homed: a compact window presents it inside a fullscreen sheet instead of a
  side pane, and each home builds it fresh in its own scope. The piece wraps the panel in a
  [`scroll`](https://daybrite.dev/api/day_pieces/fn.scroll.html) on every path.
- [`.width(pt)`](https://daybrite.dev/api/day_pieces/trait.InspectorBuilder.html#tymethod.width) sets the pane's preferred width (default 280). Where the native pane has a
  user-draggable divider this is the initial width, not a limit. The composed pane follows the
  width it splits: at 1000 points or more (a tablet in landscape, even beside a layers pane) it
  is at least 320, and it never takes more than two fifths of the split, so a narrow window
  keeps its content.
- [`.half_sheet()`](https://daybrite.dev/api/day_pieces/trait.InspectorBuilder.html#tymethod.half_sheet) makes the compact sheet a half-height one rising from the bottom edge, the
  content it describes staying on show (and, on iOS, interactive) while it is open
  ([docs/cover](cover.md) "Half sheet"). Default: the fullscreen sheet.
- [`.sheet_done(label)`](https://daybrite.dev/api/day_pieces/trait.InspectorBuilder.html#tymethod.sheet_done) names the compact sheet's dismiss button (default `✕`; day carries no
  "Done" of its own, so pass a localized one).
- [`.edge(PaneEdge::Leading)`](https://daybrite.dev/api/day_pieces/trait.InspectorBuilder.html#tymethod.edge) puts the pane on the leading side of the content instead, for a
  utility pane like a layer panel ([docs/tree](tree.md)) rather than a properties
  inspector. Default `PaneEdge::Trailing`. On AppKit the leading pane is a plain pinned
  split item (not the system inspector item, whose treatment is trailing-specific); GTK
  makes the panel the [`GtkPaned`](https://docs.gtk.org/gtk4/class.Paned.html)'s start child; Qt puts the splitter's panel pane
  first (the panes are positional; day-qt maps roles onto positions at realize). The composed form mounts the pane
  before the content, and, unlike the trailing pane, it stays a side pane at every width
  rather than re-homing into the compact sheet, because a layer panel beside a narrow canvas
  works better than a fullscreen modal, which on iOS also detaches the presenting view from
  the window. Two inspectors nest; Day Sketch wraps its trailing-inspector editor in a leading
  layers pane.

## Wide and compact

On a window wider than [`WidthClass::Compact`](size-classes.md) the panel is a side pane. On a
compact window there is no room for one, so while `visible` is true the panel presents as a
fullscreen sheet instead (or, with [`.half_sheet()`](https://daybrite.dev/api/day_pieces/trait.InspectorBuilder.html#tymethod.half_sheet), a half-height one over the content), an
unrouted [`cover`](cover.md), so it claims no route segments and
a restored session never reopens a modal. The sheet carries its own dismiss button; the
system back gesture dismisses it too, writing `false` straight back into the app's signal.
Resizing across the breakpoint re-homes the panel automatically in both directions, because
the sheet's open state is derived from `visible` **and** the window's size class rather than
stored anywhere.

A hidden composed pane reserves no width. While its panel is shown as a compact sheet,
the underlying content also keeps the full width. This follows the visibility binding and
the size class of the owning window, including when the window is resized.

The native desktop panes do not re-home on a narrow window; a desktop window dragged narrow
squeezes its split, the same rule the nav sidebar follows.

Panels containing a native [`list`](https://daybrite.dev/api/day_pieces/fn.list.html) or their own scrolling content should use
[`.not_scrollable()`](https://daybrite.dev/api/day_pieces/trait.InspectorBuilder.html#tymethod.not_scrollable). The default wraps the panel in a scroll view for property forms;
disabling that wrapper gives a recycling list a bounded viewport in both desktop panes
and compact sheets, avoiding nested scrolling and unbounded measurement.

## Per-toolkit presentation

Platform guidance: [Apple: layout](https://developer.apple.com/design/human-interface-guidelines/layout).

| toolkit | `Cap::Inspector` | pane |
|---|---|---|
| macos-appkit | Native | [`NSSplitViewItem`](https://developer.apple.com/documentation/appkit/nssplitviewitem) **inspector** item (`inspectorWithViewController:`), full-height under the titlebar. The panel paints an opaque window-background backdrop over the item's vibrancy material; the vibrant variants of the system fills only composite correctly in `allowsVibrancy` views, so grouped cards on the raw material came out near-black in dark mode (and materials capture black offscreen). Width pinned; visibility is Day's alone (the item cannot be user-collapsed). |
| linux-gtk / macos-gtk | Native | A [`GtkPaned`](https://docs.gtk.org/gtk4/class.Paned.html) with the panel as its end child (start for `PaneEdge::Leading`), each pane a filling `DayCell` so Day's frames never become pane minimums; the divider is draggable down to 160pt and the panel keeps its dragged width across a hide and a reveal; visibility flips at once, no transition (the screenshot rule). Since 2026-09 — the [`AdwOverlaySplitView`](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.OverlaySplitView.html) it replaced pinned the width by the GNOME idiom ([docs/navigation](navigation.md) records the decision). |
| linux-qt / macos-qt | Native | The nav [`QSplitter`](https://doc.qt.io/qt-6/qsplitter.html) mirrored: content pane stretches, panel pane trailing at its preferred width, divider draggable. It is not a [`QDockWidget`](https://doc.qt.io/qt-6/qdockwidget.html); `DayWindow` is a plain [`QWidget`](https://doc.qt.io/qt-6/qwidget.html) with hand-managed chrome, and dock areas need [`QMainWindow`](https://doc.qt.io/qt-6/qmainwindow.html) (the same trade the toolbar records in the shim). |
| windows-winui | Native | [`SplitView`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.splitview) with `PanePlacement=Right`, `DisplayMode=Inline`: the pane sits beside the content, never over it. |
| ios-uikit, android-mdc, harmony-arkui, web-dom, mock | Unsupported | The piece composes the pane from plain containers (same panel, same signal, a divider but no drag) and presents the compact sheet described above. |

On the native tier the `INSPECTOR` node's two `INSPECTOR_PANE` children have **native-owned
frames**: the split sizes each pane and reports it via `Event::FrameChanged`, and Day lays the
pane's content out inside the reported size, the nav-page contract
([docs/navigation](navigation.md)). A toolkit has to report on the split's own layout
passes, not only when content is inserted or the divider is dragged: at insert time the split
usually has whatever geometry its constructor left it, and content laid out against those
numbers stays that size until something else happens to resize the host.
`InspectorPatch::Visible` shows and hides the pane without a rebuild; a native affordance
hiding it (none of the current four has one) reports back as `Event::InspectorChanged`, which
must never re-fire for a Day-driven patch (the from-native echo rule).

## Extending the panel

The panel is ordinary Day content: [`form`](https://daybrite.dev/api/day_pieces/fn.form.html)/[`section`](https://daybrite.dev/api/day_pieces/fn.section.html)/[`labeled`](https://daybrite.dev/api/day_pieces/fn.labeled.html) rows bound to the app's model.
Keep each property row's binding small and selection-aware: read the common value of the
selection, write through the model's own commit path so an edit is one undo unit. Day-Sketch's
`src/inspector.rs` is the reference: a property table (label + get + set), each rendered as a
[`labeled`](https://daybrite.dev/api/day_pieces/fn.labeled.html)`(`[`text_field`](https://daybrite.dev/api/day_pieces/fn.text_field.html)`(...))` row, with a shared `Binding<String>` implementation that shows a
`multi` placeholder when the selection disagrees and fans a typed value out to every selected
node in one undo turn.
