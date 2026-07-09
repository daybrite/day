---
title: Layout
description: "The parent-proposes, child-chooses layout protocol, native measurement, and incremental relayout."
order: 13
section: Concepts
---

Day owns layout. Native toolkits each have their own layout system (Auto Layout, GTK's size
groups, Android's measure/layout passes), and they don't agree with each other — so Day bypasses
them, computes every widget's frame itself, and positions widgets absolutely inside their native
container. What Day does *not* bypass is native measurement: the platform is always the authority
on how big a piece of text or a control wants to be.

This page explains the protocol, why it works this way, and where the costs are.

## Parent proposes, child chooses

Day uses the SwiftUI-style negotiation protocol. A parent offers a child a **proposal** — an
optional width and optional height — and the child answers with the size it wants:

```rust
pub struct Proposal { pub width: Option<f64>, pub height: Option<f64> }

pub trait Layout {
    fn measure(&mut ops, children, proposal: Proposal) -> Size;
    fn place(&mut ops, children, bounds: Rect);
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
overflow (usually: let `scroll` handle it, or let the window's minimum size grow).

The `Layout` trait is public and has no private privileges — `column` is implemented with the
same trait a custom masonry or flow container would use.

One consequence worth internalizing early: **containers don't stretch children by default.** A
`column` is as wide as its widest child; a pane you want to fill available space needs `.grow()`.
Forgetting this is the most common layout surprise for newcomers (it shows up as a view
collapsing to its content size, or to nothing when it has no content).

## Native measurement, especially text

Leaf Pieces answer `measure` by asking the real widget. This matters most for text, which is
**height-for-width**: propose a width, and the toolkit's own text engine — Core Text, Pango,
minikin, QFontMetrics — reports the wrapped height. Day never guesses at glyph metrics, so a
label wraps exactly where the platform would wrap it, in every script and locale.

The cost is that measurement is a real call into the toolkit, and on Android it's a JNI
round-trip. Negotiation multiplies these probes, so Day carries a **measure cache** per node,
keyed by the quantized proposal, and bounds the distinct proposals a parent may probe per child
per pass. The cache isn't an optimization bolted on later — the design assumes it, and the mock
toolkit's tests assert measure-call counts so a regression in "fine-grained" is a failing test,
not a slow app.

## Incremental relayout

When a binding changes something size-affecting — a label's text grows, a font changes — the node
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

Why boundaries and not "stop wherever the size didn't change"? Because inside a negotiated stack,
one child's new size changes its *siblings'* proposals — you can only prune safely from a node
whose own proposal is stable. This is the subtlest part of Day's layout engine, and it's pinned
down by mock-toolkit golden tests rather than by hope.

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

`padding`, `frame`, and friends are layout-only wrapper nodes — they exist in Day's tree but
create no native widget, so nesting them is cheap.

## Windows, safe areas, and direction

- **Window sizing.** The minimum window size comes from measuring the root under a zero proposal
  — the smallest the content can actually be — not from its ideal size. (An earlier system in
  Day's lineage used the unconstrained ideal and produced windows that couldn't shrink; this is
  the lesson learned.) The window relayouts on native resize and never auto-shrinks on you.
- **Safe areas and keyboards** (mobile): the root applies safe-area insets as padding by default;
  a root-level `scroll` converts them to content insets and slides the focused field above the
  keyboard.
- **Right-to-left**: since Day owns placement, RTL is a single x-mirror applied at place time.
  `Layout` implementations are written direction-naive with leading/trailing coordinates, and the
  backends set the native per-view direction so text, cursors, and assistive technology agree
  with the mirrored layout.

## Tradeoffs, stated plainly

Owning layout buys cross-platform predictability — the same negotiation everywhere, testable on
the [mock toolkit](/docs/rendering#the-mock-toolkit) without a display — and it's what makes
per-locale reflow and RTL a framework feature instead of five platform projects. What it costs:

- **You give up native layout idioms.** Auto Layout constraints, Compose modifiers, GTK size
  groups — none of that applies inside a Day window. If your team's muscle memory is one
  platform's layout system, Day's is a new (if small) one to learn.
- **Measurement crosses the FFI.** The cache keeps this off the hot path, but a pathological
  layout (thousands of unique text leaves invalidating at once) pays real per-leaf costs,
  especially over JNI. The native [`list`](/docs/internal/list) exists precisely so long
  scrolling content doesn't become that case.
- **Deep negotiation is O(children) per level.** Same as SwiftUI; fine in practice, worth knowing
  when you build a custom `Layout`.

---

Next: [Styling](/docs/styling) — what you can restyle, and what stays native on purpose.
