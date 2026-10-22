---
title: "Programmatic scrolling"
description: "scroll_to and scroll position: driving native scroll views from Rust, and what each toolkit can promise."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Programmatic scrolling

Scroll to an item, an edge, or an offset in response to an app action. The `scroll` piece
keeps native gestures and scrolling behavior while letting code reveal content programmatically.

The `scroll` piece stays gesture-first: the native widget owns the viewport, physics, and
indicators. This document covers driving it from code and from dayscript.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

## Authoring

```rust
let jump: Signal<Option<ScrollTarget>> = Signal::new(None);

scroll(column(rows)).scroll_target(jump);

button(res::str::bottom()).action(move || jump.set(Some(ScrollTarget::Bottom)));
button(res::str::item_100()).action(move || {
    jump.set(Some(ScrollTarget::Id(
        id!(row, 100_u64).as_str().to_owned(),
    )));
});
```

`ScrollTarget::Id` still takes a string: convert the typed ID at this boundary. The target row
must declare the matching `.id(id!(row, 100_u64))` (or `.id_of` for a recycled row).

`.scroll_target(sig)` takes a `Signal<Option<ScrollTarget>>`: each `Some(target)` written to it
scrolls there (animated), then the signal resets to `None`, so the same target can be sent
twice in a row. `ScrollTarget` is:

| target | meaning |
|---|---|
| `Top` / `Bottom` | the vertical extremes |
| `Leading` / `Trailing` | the horizontal extremes (start/end in layout direction) |
| `Offset(Point)` | pin the viewport origin to a content-space point (clamped to range) |
| `Id(String)` | reveal the element with that dayscript id inside its nearest enclosing scroll |

Lower-level, `day_core::scroll_to(node, target)` drives any scroll node directly, and
`TreeOps::scroll_reveal(node, animated)` scrolls an element's nearest scroll ancestor so the
element is visible, the same call keyboard avoidance uses ([docs/focus](focus.md)). Reveals are minimal:
content already in view doesn't move.

The showcase's List page is the live reference (`Day-Showcase/src/pages/list.rs`): its scroll
buttons drive the recycling list's row rail (`.scroll_to_row`/`.scroll_to_end`, [docs/list](list.md));
`scroll_target` on a plain `scroll` piece works the same way with a `ScrollTarget`.

## dayscript

```yaml
- scroll_to: { id: page_scroll, edge: bottom }     # top | bottom | leading | trailing
- scroll_to: { id: page_scroll, x: 0, y: 300 }     # pin the viewport origin
- scroll_to: { id: 'row:100' }                     # reveal an element in its nearest scroll
```

The step is unanimated so the next step sees the settled position. `assert_visible` remains a
presence check (realized + nonzero frame; DESIGN Appendix C); it does not test whether an
element is inside the viewport, so pair `scroll_to` with screenshots when the test is about
what's on screen.

## How a target becomes a scroll

`ScrollLayout` reports the content size per scroll node (cached in the tree), so day-core can
compose each target into a content-space rect: edges become 1×1 rects at the extremes, `Offset`
becomes a viewport-sized rect (minimal-reveal on a viewport-sized rect pins the origin exactly),
and reveal-element accumulates native-ancestor origins from the element up to its scroll. Every
toolkit then applies the same "minimal scroll to make the rect visible" rule:

| toolkit | native call |
|---|---|
| AppKit | [`NSView.scrollRectToVisible`](https://developer.apple.com/documentation/appkit/nsview) on the document view |
| UIKit | [`UIScrollView.scrollRectToVisible(_:animated:)`](https://developer.apple.com/documentation/uikit/uiscrollview) |
| Android | offset math + [`ScrollView.smoothScrollTo`](https://developer.android.com/reference/android/widget/ScrollView) / `scrollTo` (per axis class) |
| GTK | adjustment clamp + `set_value` (no animation; GTK adjusts immediately) |
| Qt | scroll-bar clamp + `setValue` (no animation) |
| XAML | [`ScrollViewer.ChangeView`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.scrollviewer) (shim `day_xaml_scroll_to`) |
| ArkUI | `NODE_SCROLL_OFFSET` get/compute/set (300 ms animation when animated) |
| mock | records the computed offset (`MockWidget::scroll_offset`), the unit-test probe |

Nested scrolls reveal in the nearest enclosing scroll only; driving an outer scroll takes a
second target aimed at it.

## Scroll bars and the content width

Most toolkits draw overlay scroll bars that float over the content, so `ScrollLayout` lays
vertical content out at the scroll view's full width. Qt's Fusion style instead draws a classic bar
that takes its width out of the viewport while the content overflows. Content laid out at the full
width would then sit partly under the bar and scroll sideways by the bar's width.

The `Toolkit::scroll_bar_inset` duty reports how much room that bar takes: its width for a
vertical scroll, its height for a horizontal one. When the content overflows, `ScrollLayout`
lays it out again that much narrower, so it fits beside the bar. Content that fits keeps the
full width, because no bar appears. The default is 0, which is right for every overlay-bar toolkit.
Qt answers with its style's `PM_ScrollBarExtent`, or 0 under a style with transient bars.
