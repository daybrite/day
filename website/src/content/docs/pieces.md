---
title: Pieces
description: "Day's unit of UI: what a Piece is, how trees are composed, and what happens when one is built."
order: 11
section: Explanation
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

A **Piece** describes a control, layout, or group of controls in a Day interface. Pieces compose
into a tree, and the selected backend creates the native widgets. For example, a `label` becomes
an `NSTextField` on macOS or a `TextView` on Android. The [API tour](/docs/reference/api-tour) has examples;
this page explains composition and construction.

The Rust blocks on this page are API excerpts or integration fragments, not complete apps.
Use the [beginner journey](/docs/tutorials/first-app) for compile-tested complete files.
Source contracts: [Piece and PieceSeq](https://github.com/daybrite/day/blob/main/crates/day-core/src/build.rs),
[containers](https://github.com/daybrite/day/blob/main/crates/day-pieces/src/containers.rs), and
[structural pieces](https://github.com/daybrite/day/blob/main/crates/day-pieces/src/structure.rs).

The Rust blocks on this page are API excerpts or integration fragments, not complete apps.
Use the [beginner journey](/docs/tutorials/first-app) for compile-tested complete files.
Source contracts: [Piece and PieceSeq](https://github.com/daybrite/day/blob/main/crates/day-core/src/build.rs),
[containers](https://github.com/daybrite/day/blob/main/crates/day-pieces/src/containers.rs), and
[structural pieces](https://github.com/daybrite/day/blob/main/crates/day-pieces/src/structure.rs).

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/examples/catalog) and [catalog setup](/docs/how-to/localization).

## Start with a control

In code, a Piece is a small plain value, usually a builder struct returned by a free function:

```rust
use day::prelude::*;

label(res::str::hello())                       // → Label (a builder)
button(res::str::save()).action(|| save())     // → Button
column((label(res::str::first_label()), label(res::str::second_label())))     // → Column
```

## Composing trees

Containers take a tuple of children. Here, `temp` is a `Signal<f64>` shared by a slider
and a label:

```rust
column((
    label(res::str::temperature()).font(Font::Headline),
    row((
        slider(temp).range(0.0..=40.0),
        label(res::str::temperature(temp)),
    ))
    .spacing(Space::S),
))
.spacing(Space::M)
.padding(Space::L)
```

Tuples work up to sixteen children; past that (or when the shape is computed at runtime), pass a
`Vec` or an array, and an `Option` for a child that may be absent (`None` builds nothing):

```rust
let stars: Vec<_> = (0..5).map(|i| star(i)).collect();
row(stars).spacing(Space::XS)
row((icon, (d.precip > 0).then(|| label(res::str::precipitation(d.precip as i64)))))
```

`AnyPiece` is the type-erased form: a boxed build closure. Reach for it at a boundary that needs
one single type, such as a `Vec<AnyPiece>` whose elements differ in type, a stored builder, or a
function that branches between two different pieces:

```rust
fn status_badge(online: bool) -> AnyPiece {
    if online {
        label(res::str::online()).font(Font::Caption).any()
    } else {
        spinner().any() // a different piece type, so both arms erase
    }
}
```

An ordinary [page](/docs/reference/glossary#page) or component function does **not** erase. It returns a concrete piece type, or
`impl Piece` to avoid naming that type:

```rust
fn settings_page(dark_mode: Signal<bool>) -> impl Piece {
    column((
        label(res::str::settings_title()).font(Font::Title),
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
label(res::str::saved()).font(Font::Caption).padding(Space::S)   // typed first
label(res::str::saved()).padding(Space::S).font(Font::Caption)   // generic first — same result
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
| Controls | `button`, `toggle`, `slider`, `text_field`, `secure_field`, `picker` (menu/segmented/inline), `progress`, `spinner` |
| Layout | `column`, `row`, `zstack`, `grid`, `scroll`, `spacer`, `divider`, `form`/`section` |
| Structure | `when`, `each`, `with_environment` |
| Collections | `list` (native recycling) |
| Drawing | `canvas`, `shape` (`rectangle`, `circle`, `capsule`, `arc`, …), `image`, `vector` |
| Navigation | `nav`, `nav_stack`, `nav_link`, `toolbar` |
| Presentation | `alert`, `confirm`, `prompt`, `cover`, menus |

Anything beyond this vocabulary (a combo box, a map, a web view, a Lottie animation, an [embedded
SwiftUI view](/docs/reference/swiftui)) lives in a separate *piece crate* (`day-piece-*`) that you
add as an ordinary Cargo dependency. Optional widgets are separate dependencies, so apps include only the piece crates they use. The [extension model](/contributing/extending)
explains how those crates plug in.

Each built-in has a reference page with per-platform notes under
[internal reference](/docs/reference), for example [text](/docs/reference/text),
[lists](/docs/reference/list), and [dialogs](/docs/reference/dialogs).

## What happens at build

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
  owns the native widget, its layout state, and the [reactive](/docs/reference/glossary#reactive) scope its [bindings](/docs/reference/glossary#binding) live in.

Each Piece value is consumed once when built. Page and structural builders can be called again
when their host mounts a new subtree. Everything dynamic afterward flows through
[signals](/docs/explanation/reactivity), which are bound to individual native attributes during that single
build. The [reactivity page](/docs/explanation/reactivity) covers what that means for your code, including the
costs.


With `title = res::str::hi()`, the label's `build` creates three things that live together:

```text
   Piece (builder)          realized tree node             native widget
  ┌───────────────┐   build   ┌──────────────────┐  realize  ┌─────────────┐
  │ label(title)  │ ────────► │ kind: "label"    │ ────────► │ NSTextField │
  │ .id(id!(hi)) │           │ handle ──────────┼───────────│  (AppKit)   │
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
- The **native widget** is created immediately through the toolkit [backend](/docs/reference/glossary#backend) (an `NSButton`, a
  `GtkEntry`, …) and inserted into its native parent at the right index. Containers like
  `column` and `row` get a plain native container view; decorators (`padding`, `frame`) get no
  widget at all and exist purely in Day's tree.
- The **scope** owns every binding and event handler the build created. When the node is later
  removed (a `when` arm switches, an `each` row disappears), disposing the scope tears down its
  bindings and handlers in one step, and the native widget is released.

The details of that machinery (the tree structure, measurement, and how events travel back)
are on [How rendering works](/contributing/rendering).

## Conditional and repeated structure

Because build runs once, structural change is explicit. Two Pieces express
it:

```rust
// A subtree that exists only while the condition holds. The condition is tracked;
// changing its boolean result switches arms and disposes the previous arm's scope.
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
data; a [model](/docs/explanation/model) collection supplies one directly, and `list` accepts the
same sources.

`each` compares the old and new key sequences to decide which rows to keep, build, or remove. A
`when` flip or a row removal is a real structural edit (native widgets are added and removed), so it
costs more than a bound-attribute update. For long scrolling data, prefer
[`list`](/docs/reference/list), which hands rows to the platform's recycling list widget instead of
materializing every row.

## Identity, for testing and accessibility

Any Piece can carry a declared, typed `ElementId`:

```rust
button(res::str::save()).action(save).id(id!(save_button))
```

[dayscript](/docs/how-to/dayscript) targets elements by id, [accessibility](/docs/how-to/accessibility) uses them
as stable automation identifiers, and debug output prints them. They're optional everywhere, but pages you intend to test should id their
interactive elements; `day lint` catches an id used twice and a `navigate` to a [route](/docs/reference/glossary#route) that
doesn't exist.

`id!(save_button)` declares the ID; YAML refers to `save_button`. The catalog supplies
the human-readable title independently. For recycled rows, use `.id_of` so the ID follows
the current item. See [element IDs](/contributing/backend-contracts/element-ids).

## Where Pieces come from

You can use pieces from the framework, combine them into your own components, or add native
controls through separate crates:

1. **Built-ins**: the vocabulary above, implemented in `day-pieces` with a renderer in every
   toolkit backend.
2. **Composite pieces**: plain Rust functions or builder structs that compose existing Pieces.
   They need no new native code; their target support is limited by the controls and capabilities they compose. Most of your app is this; so
   are the
   in-tree [`day-piece-rating`](/contributing/tutorials/composite-piece) and [`day-piece-settings`](/docs/reference/settings), and the
   [star-rating tutorial](/contributing/tutorials/composite-piece).
3. **Native pieces**: a new leaf widget with a per-toolkit implementation, registered at link
   time. This is how `day-piece-webview` wraps `WKWebView`/`WebView`/`WebKitGTK`, how
   [`day-piece-swiftui`](/docs/reference/swiftui) hosts [your own SwiftUI views](/docs/reference/swiftui) on macOS and iOS,
   and how you'd wrap a platform control Day doesn't cover. See the
   [native piece tutorial](/contributing/tutorials/native-piece).

Composite pieces reuse existing widget implementations. Native pieces require an implementation
for each [toolkit](/docs/reference/glossary#toolkit) you support (a piece that only implements AppKit and UIKit renders a labeled
[placeholder](/docs/reference/glossary#placeholder) elsewhere, so the gap is visible and the app keeps running).

---

Next: [Reactivity](/docs/explanation/reactivity), the [signals](/docs/reference/glossary#signal) that update the widget tree.
