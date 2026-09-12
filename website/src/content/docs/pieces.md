---
title: Pieces
description: "Day's unit of UI: what a Piece is, how trees are composed, and what happens when one is built."
order: 11
section: Concepts
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

A **Piece** describes a control, layout, or group of controls in a Day interface. Pieces compose
into a tree, and the selected backend creates the native widgets. For example, a `label` becomes
an `NSTextField` on macOS or a `TextView` on Android. The [API tour](/docs/api-tour) has examples;
this page explains composition and construction.

## A Piece is a description, built once

In code, a Piece is a small plain value, usually a builder struct returned by a free function:

```rust
use day::prelude::*;

label("Hello")                       // → Label (a builder)
button("Save").action(|| save())     // → Button
column((label("a"), label("b")))     // → Column
```

Behind the builders sits one trait with one method:

```rust
pub trait Piece: 'static {
    fn build(self, cx: &mut BuildCx) -> RNode;
}
```

Two parts of that signature matter.

- **`build` takes `self`, not `&self`.** A Piece is consumed exactly once. There is no retained
  view description that Day re-runs and diffs against the last frame.
- **It returns an `RNode`,** a handle to a node in the *realized tree*: the live structure that
  owns the native widget, its layout state, and the [reactive](/docs/glossary#reactive) scope its [bindings](/docs/glossary#binding) live in.

Your Piece functions run once, at mount time. Everything dynamic afterward flows through
[signals](/docs/reactivity), which are bound to individual native attributes during that single
build. The [reactivity page](/docs/reactivity) covers what that means for your code, including the
costs.

## Composing trees

Containers take their children as tuples, so a static tree is written directly:

```rust
column((
    label("Temperature").font(Font::Headline),
    row((
        slider(temp).range(0.0..=40.0),
        label(move || format!("{:.1}°", temp.get())),
    ))
    .spacing(8.0),
))
.spacing(12.0)
.padding(16.0)
```

Tuples work up to sixteen children; past that (or when the shape is computed at runtime), collect
into a `PieceVec`:

```rust
let stars: Vec<AnyPiece> = (0..5).map(|i| star(i)).collect();
row(PieceVec(stars)).spacing(4.0)
```

`AnyPiece` is the type-erased form: a boxed build closure. Reach for it at a boundary that needs
one single type, such as a `PieceVec` like the one above, a stored builder, or a function that
branches between two different pieces:

```rust
fn status_badge(online: bool) -> AnyPiece {
    if online {
        label(tr("online")).font(Font::Caption).any()
    } else {
        spinner().any()   // a different piece type, so both arms erase
    }
}
```

An ordinary [page](/docs/glossary#page) or component function does **not** erase. It returns a concrete piece type, or
`impl Piece` to avoid naming that type:

```rust
fn settings_page() -> impl Piece {
    column((
        label(tr("settings_title")).font(Font::Title),
        toggle(dark_mode),
    ))
}
```

Day's constructors preserve concrete types: `column()` returns a `Column`, `labeled()` a
`Labeled`, and modifiers (`.id()`, `.padding()`, `.on_tap()` …) return `Decorated<P>`, which keeps
the decorated piece's type. Call `.any()` where a single `AnyPiece` type is
required. Calling it on an `AnyPiece` returns the existing value without another allocation.

Because the type is kept, the piece's builders can be chained after a generic modifier, in
either order:

```rust
label("Saved").font(Font::Caption).padding(8.0)   // typed first
label("Saved").padding(8.0).font(Font::Caption)   // generic first — same result
```

A build-time branch between two different piece types takes `Either` rather than erasing both
sides:

```rust
if compact { Either::Left(row(children)) } else { Either::Right(column(children)) }
```

## The built-in vocabulary

The `day` prelude ships a small set of Pieces, grouped roughly as follows:

| Group | Pieces |
|---|---|
| Text | `label`, `text_area` |
| Controls | `button`, `toggle`, `slider`, `text_field`, `picker` (menu/segmented/inline), `progress`, `spinner` |
| Layout | `column`, `row`, `zstack`, `grid`, `scroll`, `spacer`, `divider`, `form`/`section` |
| Structure | `when`, `each`, `with_environment` |
| Collections | `list` (native recycling) |
| Drawing | `canvas`, `shape` (`rectangle`, `circle`, `capsule`, `arc`, …), `image`, `vector` |
| Navigation | `nav`, `nav_stack`, `nav_link`, `toolbar` |
| Presentation | `alert`, `confirm`, `prompt`, `cover`, menus |

Anything beyond this vocabulary (a combo box, a map, a web view, a Lottie animation, an [embedded
SwiftUI view](/docs/internal/swiftui)) lives in a separate *piece crate* (`day-piece-*`) that you
add as an ordinary Cargo dependency. Optional widgets are separate dependencies, so apps include only the piece crates they use. The [extension model](/docs/extending)
explains how those crates plug in.

Each built-in has a reference page with per-platform notes under
[internal reference](/docs/reference), for example [text](/docs/internal/text),
[lists](/docs/internal/list), and [dialogs](/docs/internal/dialogs).

## What happens at build

When a Piece's `build` runs, three things are created together and live together:

```text
   Piece (builder)          realized tree node             native widget
  ┌───────────────┐   build   ┌──────────────────┐  realize  ┌─────────────┐
  │ label("Hi")   │ ────────► │ kind: "label"    │ ────────► │ NSTextField │
  │  .id("hi")    │           │ handle ──────────┼───────────│  (AppKit)   │
  └───────────────┘           │ layout, flex     │           └─────────────┘
                              │ scope ──┐        │
                              │ id, a11y│        │
                              └─────────┼────────┘
                                        ▼
                              reactive Scope: owns this
                              node's bindings + handlers
```

- The **node** records the Piece's kind, its place in the tree, its layout behavior, and its
  accessibility annotations.
- The **native widget** is created immediately through the toolkit [backend](/docs/glossary#backend) (an `NSButton`, a
  `GtkEntry`, …) and inserted into its native parent at the right index. Containers like
  `column` and `row` get a plain native container view; decorators (`padding`, `frame`) get no
  widget at all and exist purely in Day's tree.
- The **scope** owns every binding and event handler the build created. When the node is later
  removed (a `when` arm switches, an `each` row disappears), disposing the scope tears down its
  bindings and handlers in one step, and the native widget is released.

The details of that machinery (the tree structure, measurement, and how events travel back)
are on [How rendering works](/docs/rendering).

## Conditional and repeated structure

Because build runs once, structural change is explicit. Two Pieces express
it:

```rust
// A subtree that exists only while the condition holds. The closure re-runs
// when `cond`'s signals change; the old arm's scope is disposed.
when(move || logged_in.get(), move || profile_panel())

// With an else arm. Exactly one arm is mounted at a time, and the two need
// not return the same Piece type.
when(move || logged_in.get(), move || profile_panel())
    .otherwise(move || sign_in_form())

// A keyed collection. Rows are created, moved, and disposed by key diffing —
// surviving rows keep their nodes and native widgets.
each(
    items(
        move || todos.get(),      // data
        |t: &Todo| t.id,          // stable key
    ),
    |slot| todo_row(slot),        // per-row builder; slot tracks the item
)
```

`each` takes a *row source* and a row builder. `items(data, key_of)` is the row source for plain
data; a [model](/docs/internal/model) collection supplies one directly, and `list` accepts the
same sources.

This is the only place Day diffs anything, and it diffs *keys*, not widget trees: `each` compares
the old and new key sequences to decide which rows to keep, which to build, and which to dispose. A
`when` flip or a row removal is a real structural edit (native widgets are added and removed), so it
costs more than a bound-attribute update. For long scrolling data, prefer
[`list`](/docs/internal/list), which hands rows to the platform's recycling list widget instead of
materializing every row.

## Identity, for testing and accessibility

Any Piece can carry a stable string id:

```rust
button(tr("save")).action(save).id("save-button")
```

[dayscript](/docs/dayscript) targets elements by id, [accessibility](/docs/accessibility) uses them
as stable automation identifiers, and debug output prints them. They're optional everywhere, but pages you intend to test should id their
interactive elements; `day lint` catches an id used twice and a `navigate` to a [route](/docs/glossary#route) that
doesn't exist.

## Where Pieces come from

There are exactly three kinds of Piece, and you can write all three:

1. **Built-ins**: the vocabulary above, implemented in `day-pieces` with a renderer in every
   toolkit backend.
2. **Composite pieces**: plain Rust functions or builder structs that compose existing Pieces.
   They need no native code and work on every [target](/docs/glossary#target) automatically. Most of your app is this; so
   are the
   in-tree `day-piece-rating` and `day-piece-settings`, and the
   [star-rating tutorial](/docs/tutorial-composite-piece).
3. **Native pieces**: a new leaf widget with a per-toolkit implementation, registered at link
   time. This is how `day-piece-webview` wraps `WKWebView`/`WebView`/`WebKitGTK`, how
   `day-piece-swiftui` hosts [your own SwiftUI views](/docs/internal/swiftui) on macOS and iOS,
   and how you'd wrap a platform control Day doesn't cover. See the
   [native piece tutorial](/docs/tutorial-native-piece).

Composite pieces reuse existing widget implementations. Native pieces require an implementation
for each [toolkit](/docs/glossary#toolkit) you support (a piece that only implements AppKit and UIKit renders a labeled
[placeholder](/docs/glossary#placeholder) elsewhere, so the gap is visible and the app keeps running).

---

Next: [Reactivity](/docs/reactivity), the [signals](/docs/glossary#signal) that update the widget tree.
