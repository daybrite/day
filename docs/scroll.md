---
navTitle: "Scrolling"
title: "Scroll views"
description: "The scroll piece: indicators, bounce, paging and snapping, keyboard dismissal, content padding, zoom, the tracked offset and phase, and programmatic scrolling, with what each toolkit can promise."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Scroll views

Scrolling reveals parts of a page or collection that do not fit in the available space. It lets
people explore more content without leaving the current page.

The [`scroll`](https://docs.rs/day-pieces/latest/day_pieces/fn.scroll.html) piece is the
platform's own scroll view, with its gestures, scrolling physics, and indicators. This
reference covers how an app configures it, follows it, and drives it from Rust and
[DayScript](/docs/how-to/dayscript).

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.

## Behavior

```rust
scroll(page)
    .hide_indicators()
    .no_bounce()
    .keyboard_dismiss(KeyboardDismiss::OnDrag)
    .content_padding(Insets { top: 0.0, leading: 0.0, bottom: 88.0, trailing: 0.0 })
```

Each of these is a preference. A toolkit applies what its scroll view offers and ignores the
rest, and where an app needs to know, a `Cap` says which.

| builder | effect | probe |
|---|---|---|
| [`.horizontal()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.horizontal) / [`.axis(axis)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.axis) | the scroll axis | every toolkit |
| [`.hide_indicators()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.hide_indicators) | no scroll bars; the content still scrolls | every toolkit |
| [`.no_bounce()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.no_bounce) | stop at the edges: no rubber band, no edge glow or stretch | ignored where the platform has no overscroll effect |
| [`.always_bounce()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.always_bounce) | the overscroll effect even when the content fits | as above |
| [`.paging()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.paging) | come to rest on whole pages, one viewport per swipe | `Cap::ScrollSnap` |
| [`.snap_interval(points)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.snap_interval) | come to rest on multiples of an interval | `Cap::ScrollSnap` |
| [`.keyboard_dismiss(mode)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.keyboard_dismiss) | when scrolling dismisses the software keyboard | `Cap::ScrollKeyboardDismiss` |
| [`.keyboard_taps(taps)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.keyboard_taps) | what a tap does while the keyboard is up | `Cap::ScrollKeyboardDismiss` (`Native` only) |
| [`.content_padding(insets)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.content_padding) | space around the content, inside the scrolling area | every toolkit (laid out by Day) |
| [`.zoom(range)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.zoom) | pinch to zoom between two scales | `Cap::ScrollZoom` |

### Paging and snapping

[`.paging()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.paging) makes each swipe move one viewport along the scroll axis, which turns a horizontal
scroll of full-width children into a carousel. [`.snap_interval(points)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.snap_interval) stops on multiples of
a fixed distance instead: a strip of 280-point cards with a 12-point gap snaps every 292
points, so a card always rests at the leading edge. Where `Cap::ScrollSnap` is unsupported the
view scrolls freely. For pages that keep their identity and report which one is selected, use
[`pager`](pager.md).

### Keyboard

`KeyboardDismiss::OnDrag` puts the software keyboard away when a drag begins;
`KeyboardDismiss::Interactive` lets it follow the finger down where the platform has that
(UIKit), and is the same as `OnDrag` elsewhere. `KeyboardTaps::Dismiss` makes a tap inside the
scroll view dismiss the keyboard and go no further, so the first tap on a form puts the
keyboard away and the second presses a control. The default, `KeyboardTaps::Deliver`, sends
every tap to the control under it and leaves the keyboard up.

`Cap::ScrollKeyboardDismiss` answers `Native` where the platform's scroll view does this,
`Emulated` where Day resigns focus itself when a drag begins (taps are then always
delivered), and `Unsupported` where there is no software keyboard.

### Content padding

[`.content_padding(insets)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.content_padding) puts space around the content inside the scrolling area. It scrolls
with the content, so the scroll view can run edge to edge under a floating bar while the last
row still clears it. Day lays this out itself: the padding is part of the content size on every
toolkit.

## Offset and phase

```rust
let offset = Signal::new(Point::ZERO);
let phase = Signal::new(ScrollPhase::Idle);

zstack((
    scroll(page).scroll_offset(offset).scroll_phase(phase),
    header
        .opacity(move || 1.0 - (offset.get().y / 120.0).clamp(0.0, 1.0))
        .aligned(Alignment::Top),
))
```

[`.scroll_offset(binding)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.scroll_offset) follows the content offset, in points from the
content's top-left corner. That is the content space `ScrollTarget::Offset` and dayscript's
`scroll_to` `x`/`y` take, so a value read back can be written again. Under a right-to-left
layout the content is mirrored inside that space, so a horizontal scroll opens on its leading
(right) edge and reads its full range there; its offset still counts from the left. The view
writes it as it scrolls, at most once per display frame, and once more with the offset it comes to rest at. Whatever moves the content counts:
a finger, a wheel, momentum, or `scroll_target`. A collapsing header, a parallax layer and a
"jump to top" button that appears after the first screen are all functions of this value.
Without the binding the view installs no scroll observer. Probe `Cap::ScrollOffset`; where it
is unsupported the binding keeps its value.

[`.scroll_phase(binding)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.scroll_phase) follows the user's scrolling:

| `ScrollPhase` | meaning |
|---|---|
| `Idle` | nothing the user did is moving the content |
| `Dragging` | the user is moving it: a finger is down, or a wheel or trackpad gesture is in progress |
| `Coasting` | the finger lifted and the content is still moving: momentum, a bounce settling, a snap |

A programmatic scroll moves the offset and leaves the phase alone. `watch` the binding for the
begin and end moments: `Idle` to `Dragging` is the begin, `Dragging` to anything else is the
finger lifting, and the return to `Idle` is the end of all motion.

Where `Cap::ScrollPhases` is `Native` the platform reports all three. Elsewhere Day derives
the phase from the offset reports: `Dragging` at the first movement, `Idle` once the offset
has held still for 150 ms, and never `Coasting`. That needs `Cap::ScrollOffset`.

## Zoom

```rust
let scale = Signal::new(1.0);
scroll(photo).zoom(1.0..=4.0).zoom_scale(scale);
button(res::str::fit()).action(move || scale.set(1.0));
```

[`.zoom(range)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.zoom) lets the user pinch the content between two scales; `1.0` is its
natural size. [`.zoom_scale(binding)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.zoom_scale) is the scale both ways: it follows the
pinch, and a value the app writes zooms there, animated and clamped to the range, keeping the
center of the viewport in place. A value written while an earlier zoom is still animating
replaces it: the view ends at the last value written.

`Cap::ScrollZoom` is `Native` where the platform's scroll view zooms. Where it is
unsupported the content stays at its natural size, a pinch does nothing, and the binding
keeps the value the app wrote. An app that must zoom everywhere checks the cap and offers
its own controls, or draws on a [canvas](canvas.md) with `on_pinch`.

## Programmatic scrolling

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
must declare the matching [`.id(id!(row, 100_u64))`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id) (or [`.id_of`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id_of) for a recycled row).

[`.scroll_target(sig)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.scroll_target) takes a `Signal<Option<ScrollTarget>>`: each `Some(target)` written to it
scrolls there, then the signal resets to `None`, so the same target can be sent twice in a
row. A write is a command, and the scroll view consumes it; a list's
[`.scroll_to_row`](list.md#programmatic-scrolling) follows the same contract.

The scroll is animated with the platform's own scroll animation on every toolkit, and jumps
instead while the system's reduce-motion setting is on. A tracked offset reports the positions
on the way and the one it rests at. `ScrollTarget` is:

| target | meaning |
|---|---|
| `Top` / `Bottom` | the vertical extremes |
| `Leading` / `Trailing` | the horizontal extremes (start/end in layout direction) |
| `Offset(Point)` | pin the viewport origin to a content-space point (clamped to range) |
| `Id(String)` | reveal the element with that dayscript id inside its nearest enclosing scroll, or the list row it sits in |

On a [list](list.md#programmatic-scrolling), `Top` and `Bottom` scroll by rows through the
list's row duty; the other targets do not apply.

Lower-level, [`day_core::scroll_to(node, target)`](https://docs.rs/day-core/latest/day_core/fn.scroll_to.html) drives any scroll node directly, and
[`TreeOps::scroll_reveal(node, animated)`](https://docs.rs/day-core/latest/day_core/trait.TreeOps.html#tymethod.scroll_reveal) scrolls an element's nearest scroll ancestor so the
element is visible, the same call keyboard avoidance uses ([docs/focus](focus.md)). Reveals are minimal:
content already in view doesn't move.

The showcase's List page is the live reference (`Day-Showcase/src/pages/list.rs`): its scroll
buttons drive the recycling list's row rail ([`.scroll_to_row`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.scroll_to_row)/[`.scroll_to_end`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.scroll_to_end), [docs/list](list.md));
[`scroll_target`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.scroll_target) on a plain [`scroll`](https://docs.rs/day-pieces/latest/day_pieces/fn.scroll.html) piece works the same way with a `ScrollTarget`.

## How a target becomes a scroll

`ScrollLayout` reports the content size per scroll node (cached in the tree), so day-core can
compose each target into a content-space rect: edges become 1×1 rects at the extremes, `Offset`
becomes a viewport-sized rect (minimal-reveal on a viewport-sized rect pins the origin exactly),
and reveal-element accumulates native-ancestor origins from the element up to its scroll. Every
toolkit then applies the same "minimal scroll to make the rect visible" rule:

| toolkit | native call |
|---|---|
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | [`NSView.scrollRectToVisible`](https://developer.apple.com/documentation/appkit/nsview) on the document view |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | [`UIScrollView.scrollRectToVisible(_:animated:)`](https://developer.apple.com/documentation/uikit/uiscrollview) |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | offset math + [`ScrollView.smoothScrollTo`](https://developer.android.com/reference/android/widget/ScrollView) / `scrollTo` (per axis class) |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | adjustment clamp + `set_value`, stepped over the frame clock when animated |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | scroll-bar clamp + `setValue`, through a `QPropertyAnimation` when animated |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | [`ScrollViewer.ChangeView`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.scrollviewer) (shim `day_xaml_scroll_to`) |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | `NODE_SCROLL_OFFSET` get/compute/set (300 ms animation when animated) |
| mock | records the computed offset (`MockWidget::scroll_offset`), the unit-test probe |

Nested scrolls reveal in the nearest enclosing scroll only; driving an outer scroll takes a
second target aimed at it.

## Scroll bars and the content width

Most toolkits draw overlay scroll bars that float over the content, so `ScrollLayout` lays
vertical content out at the scroll view's full width. Qt's Fusion style instead draws a classic bar
that takes its width out of the viewport while the content overflows. Content laid out at the full
width would then sit partly under the bar and scroll sideways by the bar's width.

The [`Toolkit::scroll_bar_inset`](https://docs.rs/day-spec/latest/day_spec/trait.Toolkit.html#method.scroll_bar_inset) duty reports how much room that bar takes: its width for a
vertical scroll, its height for a horizontal one. When the content overflows, `ScrollLayout`
lays it out again that much narrower, so it fits beside the bar. Content that fits keeps the
full width, because no bar appears. The default is 0, which is right for every overlay-bar toolkit.
Qt answers with its style's `PM_ScrollBarExtent`, or 0 under a style with transient bars.

## Toolkit support

What each toolkit does with the preferences above. `Cap` answers are in the generated
[coverage matrix](coverage-matrix.md).

| Toolkit | Indicators | Bounce | Snap | Keyboard dismiss | Offset | Phases | Zoom | Animated `scroll_target` |
|---|---|---|---|---|---|---|---|---|
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | hidden scrollers | the scroll view's elasticity | `Emulated`: the clip view glides to the nearest stop once a live scroll, momentum included, or a wheel burst ends | `Unsupported`: no software keyboard | `Native` | `Native`: live-scroll notifications and each wheel event's phases; a mouse wheel without phases ends 150 ms after its last tick | `Native`: `allowsMagnification` | the clip view's animator |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | `showsVerticalScrollIndicator` and its horizontal twin | `bounces` and `alwaysBounceVertical` or `alwaysBounceHorizontal` | `Native`: `pagingEnabled`, or the target offset UIKit asks for at the end of a drag | `Native`: `keyboardDismissMode`, and a tap recognizer of Day's own for `KeyboardTaps::Dismiss` | `Native` | `Native` | `Native`: `minimumZoomScale` and `maximumZoomScale` | `scrollRectToVisible(_:animated:)` |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | the scroll bars switched off | the over-scroll mode | `Emulated`: the view takes over the fling and the release and scrolls smoothly to the stop the fling would reach | `Native`; `Interactive` acts as `OnDrag` | `Native` | `Native`, read from the view's own touch stream and fling | `Unsupported` | `smoothScrollTo` |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | `GTK_POLICY_EXTERNAL` | ignored: GTK's overshoot belongs to kinetic scrolling | `Unsupported` | `Unsupported` | `Native` | `Native`: a touchpad's scroll begin, end and deceleration; a burst of wheel steps is one drag that ends 150 ms after the last step; a touch or scroll-bar drag from its press and release; `Coasting` ends 100 ms after the content stops | `Unsupported` | a 250 ms ease-out on the frame clock, since GTK has no scroll animation; a wheel turn, press or touch on the view stops it |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | `ScrollBarAlwaysOff` | none: a Qt scroll area stops at its edges | `Emulated`: a glide to the nearest stop once scrolling has been quiet for 150 ms | `Unsupported` | `Native` | derived from the offset (no `Coasting`) | `Unsupported` | a 250 ms `QPropertyAnimation`; a wheel turn stops it |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | `ScrollBarVisibility.Hidden` | the platform's: a `ScrollViewer` has no switch for its overscroll | `Emulated`: the view settles on the nearest stop when a scroll comes to rest | `Unsupported` | `Native` | `Native`: direct manipulation's start, inertia and end, and a wheel burst | `Native`: `ZoomMode` with `MinZoomFactor` and `MaxZoomFactor` | `ChangeView` |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | the scroll bar off | the edge effect | `Native`: the scroll's paging and snap attributes | `Emulated` | `Native` | `Native` | `Unsupported`: the C API's scroll has no zoom | a 300 ms animation of `NODE_SCROLL_OFFSET` |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | hidden scroll bars | [`.no_bounce()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.no_bounce) sets `overscroll-behavior: none`; [`.always_bounce()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.always_bounce) is ignored | `Native`: CSS scroll snap | `Emulated` on a touch screen, `Unsupported` without one | `Native` | `Native`: touch, wheel, keyboard and scroll-bar input with `scrollend` | `Unsupported` | the browser's smooth scrolling |
