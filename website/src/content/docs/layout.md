---
title: Layout
description: "The parent-proposes, child-chooses layout protocol, native measurement, and incremental relayout."
order: 13
section: Concepts
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day computes layout itself. Native toolkits each have their own layout system (Auto Layout, GTK's
size groups, Android's measure/layout passes), and they don't agree with each other, so Day
bypasses them, computes every widget's frame itself, and positions widgets absolutely inside their
native container. Day still asks the platform to measure: the toolkit is always the authority on
how big a piece of text or a control wants to be.

This page explains the protocol, why it works this way, and where the costs are.

## Parent proposes, child chooses

Day uses the SwiftUI-style negotiation protocol. A parent offers a child a **proposal** (an
optional width and optional height), and the child answers with the size it wants:

```rust
pub struct Proposal { pub width: Option<f64>, pub height: Option<f64> }

pub trait Layout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size;
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect);
}
```

The two-phase pass looks like this for a simple form row:

```text
measure                                     place
───────                                     ─────
row gets Proposal { w: 400, h: None }       row gets Rect { 0,0 400×32 }
 ├─ measures label   → 88×20                 ├─ places label  at (0,6)   88×20
 ├─ measures spacer  → flexible              ├─ (spacer takes the slack)
 └─ measures toggle  → 52×32                 └─ places toggle at (348,0) 52×32
row answers 400×32
```

Containers like `row` and `column` measure rigid children first, then divide the remaining space
among flexible ones (`spacer`, anything marked `.grow()`). A child is never forced: if you propose
100 points to a label that needs 120, it answers 120, and the parent decides what to do about the
overflow (usually: let `scroll` handle it).

The `Layout` trait is public and has no private privileges; `column` is implemented with the
same trait a custom masonry or flow container would use.

One consequence to learn early is that **containers don't stretch children by default.** A
`column` is as wide as its widest child; a pane you want to fill available space needs `.grow()`.
Forgetting this shows up as a view collapsing to its content size, or to nothing when it has no
content.

## Native measurement, especially text

Leaf Pieces answer `measure` by asking the real widget. This matters most for text, which is
**height-for-width**: propose a width, and the toolkit's own text engine (Core Text, Pango,
minikin, QFontMetrics) reports the wrapped height. Day never guesses at glyph metrics, so a
label wraps exactly where the platform would wrap it, in every script and locale.

The cost is that measurement is a real call into the toolkit, and on Android it's a JNI
round-trip. Negotiation multiplies these probes, so Day carries a **measure cache** per node,
keyed by the quantized proposal, and bounds the distinct proposals a parent may probe per child
per pass. The design assumes the cache: the mock toolkit's tests assert measure-call counts, so
a caching regression fails tests.

## Incremental relayout

When a binding changes something size-affecting (a label's text grows, a font changes), the node
is marked dirty and the dirt bubbles up to the nearest **layout boundary**: a node whose size is
externally fixed, like the window root, a `scroll`, or a node with an explicit two-axis
`.frame(w, h)`. At the turn boundary, layout re-enters *there*, not at the root:

```text
window root (boundary)
 └─ column
     ├─ header                    unchanged: answers from measure cache
     └─ scroll (boundary)  ◄──── relayout re-enters here
         └─ column
             ├─ row              re-measured: contains the dirty label
             │   └─ label*       ← text changed
             └─ row              unchanged: pruned (same proposal, size, origin)
```

Frames are diffed with a half-pixel epsilon before touching the toolkit, so a text change that
doesn't move anything costs one native `set_text` and zero frame updates.

Relayout needs boundaries because, inside a negotiated stack, one child's new size changes its
siblings' proposals, so pruning is only safe from a node whose own proposal is stable.
Mock-toolkit golden tests pin this behavior down.

## The modifier vocabulary

Day's layout modifiers are few and compose left to right:

```rust
label("Total")
    .padding(Insets::symmetric(12.0, 6.0))  // or .padding(8.0) for all edges
    .frame(200.0, 44.0)                     // fixed size (or .width / .height for one axis)
    .grow()                                 // take flexible space in the parent's axis

column((a, b, c)).spacing(8.0).align(HAlign::Leading)
row((x, spacer(), y))                       // spacer pushes x and y apart
zstack((photo, badge)).align(Alignment::TopTrailing)
scroll(long_column)
```

`padding`, `frame`, and friends are layout-only wrapper nodes: they exist in Day's tree but
create no native widget, so nesting them is cheap.

## Windows, safe areas, and direction

- **Window sizing:** the minimum window size is the one the app declares
  (`WindowOptions::min_size`, an `Option<Size>` that defaults to none), applied verbatim; Day
  doesn't derive a minimum from content measurement. The window relayouts on native resize and
  never shrinks on its own.
- **Safe areas and keyboards** (mobile): the root applies safe-area insets as padding by default;
  a root-level `scroll` converts them to content insets and slides the focused field above the
  keyboard. Backends with an edge-to-edge mode (Android's immersive opt-in) stop clamping the
  top and report the insets through `day::safe_area()` instead. Paint a background unpadded
  and pad the content by those insets to run it under the system bars.
- **Right-to-left**: since Day owns placement, RTL is a single x-mirror applied at place time.
  `Layout` implementations are written direction-naive with leading/trailing coordinates, and the
  backends set the native per-view direction so text, cursors, and assistive technology agree
  with the mirrored layout.

## Tradeoffs

Owning layout gives Day the same negotiation on every platform, testable on the
[mock toolkit](/docs/rendering#the-mock-toolkit) without a display, and it is why per-locale
reflow and RTL are features of the framework that every backend shares. It also has costs:

- **You give up native layout idioms.** Auto Layout constraints, Compose modifiers, GTK size
  groups: none of that applies inside a Day window. If your team's muscle memory is one
  platform's layout system, Day's is a new (if small) one to learn.
- **Measurement crosses the FFI:** the cache keeps this off the hot path, but a pathological
  layout (thousands of unique text leaves invalidating at once) pays real per-leaf costs,
  especially over JNI. The native [`list`](/docs/internal/list) exists so that long
  scrolling content doesn't become that case.
- **Deep negotiation is O(children) per level.** SwiftUI has the same cost; it is rarely a
  problem, but it becomes visible when you build a custom `Layout`.

---

Next: [Styling](/docs/styling), what you can restyle and what stays native.
