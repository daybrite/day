# Programmatic scrolling

> **Status: implemented** on every backend (AppKit, UIKit, Android, GTK, Qt, WinUI, ArkUI, mock).
> One primitive carries it all: `Toolkit::scroll_to(handle, rect, animated)` with
> scrollRectToVisible semantics — day-core composes edges, offsets, and reveal-element targets
> into content-space rects, so each backend only implements "minimal scroll to make this rect
> visible". Verified by mock-toolkit unit tests (`crates/day-pieces/tests/mock_e2e.rs`) and the
> showcase Scrolling page + walkthrough.

The `scroll` piece stays gesture-first: the native widget owns the viewport, physics, and
indicators (DESIGN §7.6). This document covers driving it from code and from dayscript.

## Authoring

```rust
let jump: Signal<Option<ScrollTarget>> = Signal::new(None);

scroll(column(rows)).scroll_target(jump);

button("Bottom").action(move || jump.set(Some(ScrollTarget::Bottom)));
button("Item 100").action(move || jump.set(Some(ScrollTarget::Id("row-100".into()))));
```

`.scroll_target(sig)` takes a `Signal<Option<ScrollTarget>>`: each `Some(target)` written to it
scrolls there (animated), then the signal resets to `None` — write-and-forget, so the same
target can be sent twice in a row. `ScrollTarget` is:

| target | meaning |
|---|---|
| `Top` / `Bottom` | the vertical extremes |
| `Leading` / `Trailing` | the horizontal extremes (start/end in layout direction) |
| `Offset(Point)` | pin the viewport origin to a content-space point (clamped to range) |
| `Id(String)` | reveal the element with that dayscript id inside its nearest enclosing scroll |

Lower-level, `day_core::scroll_to(node, target)` drives any scroll node directly, and
`TreeOps::scroll_reveal(node, animated)` scrolls an element's nearest scroll ancestor so the
element is visible — the same call keyboard avoidance uses (docs/focus.md). Reveals are minimal:
content already in view doesn't move.

The showcase's Scrolling page is the live reference (`apps/showcase/src/pages/scrolling.rs`).

## dayscript

```yaml
- scroll_to: { id: page-scroll, edge: bottom }     # top | bottom | leading | trailing
- scroll_to: { id: page-scroll, x: 0, y: 300 }     # pin the viewport origin
- scroll_to: { id: row-100 }                       # reveal an element in its nearest scroll
```

The step is unanimated so the next step sees the settled position. `assert_visible` remains a
presence check (realized + nonzero frame — DESIGN Appendix C); it does not test whether an
element is inside the viewport, so pair `scroll_to` with screenshots when the point of the test
is what's on screen.

## How a target becomes a scroll

`ScrollLayout` reports the content size per scroll node (cached in the tree), so day-core can
compose each target into a content-space rect: edges become 1×1 rects at the extremes, `Offset`
becomes a viewport-sized rect (minimal-reveal on a viewport-sized rect pins the origin exactly),
and reveal-element accumulates native-ancestor origins from the element up to its scroll. Every
backend then applies the same "minimal scroll to make the rect visible" rule:

| backend | native call |
|---|---|
| AppKit | `NSView.scrollRectToVisible` on the document view |
| UIKit | `UIScrollView.scrollRectToVisible(_:animated:)` |
| Android | offset math + `ScrollView.smoothScrollTo` / `scrollTo` (per axis class) |
| GTK | adjustment clamp + `set_value` (no animation — GTK adjusts immediately) |
| Qt | scroll-bar clamp + `setValue` (no animation) |
| WinUI | `ScrollViewer.ChangeView` (shim `day_winui_scroll_to`) |
| ArkUI | `NODE_SCROLL_OFFSET` get/compute/set (300 ms animation when animated) |
| mock | records the computed offset (`MockWidget::scroll_offset`) — the unit-test probe |

Nested scrolls reveal in the NEAREST enclosing scroll only; driving an outer scroll takes a
second target aimed at it.
