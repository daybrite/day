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
into a tree, and the selected backend creates the native widgets. For example, a [`label`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html) becomes
an `NSTextField` on macOS or a `TextView` on Android. The [API tour](/docs/reference/api-tour) has examples;
this page explains composition and construction.

The Rust blocks on this page are API excerpts or integration fragments, not complete apps.
Use the [beginner journey](/docs/tutorials/first-app) for compile-tested complete files.
Source contracts: [Piece and PieceSeq](https://github.com/daybrite/day/blob/main/crates/day-core/src/build.rs),
[containers](https://github.com/daybrite/day/blob/main/crates/day-pieces/src/containers.rs), and
[structural pieces](https://github.com/daybrite/day/blob/main/crates/day-pieces/src/structure.rs).

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

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

Day's constructors preserve concrete types: [`column()`](https://docs.rs/day-pieces/latest/day_pieces/fn.column.html) returns a `Column`, [`labeled()`](https://docs.rs/day-pieces/latest/day_pieces/fn.labeled.html) a
`Labeled`, and modifiers ([`.id()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id), [`.padding()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding), [`.on_tap()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_tap) …) return `Decorated<P>`, which keeps
the decorated piece's type. Call [`.any()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.any) where a single `AnyPiece` type is
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
| Text | [`label`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html), [`text_area`](https://docs.rs/day-pieces/latest/day_pieces/fn.text_area.html) |
| Controls | [`button`](https://docs.rs/day-pieces/latest/day_pieces/fn.button.html), [`toggle`](https://docs.rs/day-pieces/latest/day_pieces/fn.toggle.html), [`slider`](https://docs.rs/day-pieces/latest/day_pieces/fn.slider.html), [`text_field`](https://docs.rs/day-pieces/latest/day_pieces/fn.text_field.html), [`secure_field`](https://docs.rs/day-pieces/latest/day_pieces/fn.secure_field.html), [`picker`](https://docs.rs/day-pieces/latest/day_pieces/fn.picker.html) (menu/segmented/inline), [`progress`](https://docs.rs/day-pieces/latest/day_pieces/fn.progress.html), [`spinner`](https://docs.rs/day-pieces/latest/day_pieces/fn.spinner.html) |
| Layout | [`column`](https://docs.rs/day-pieces/latest/day_pieces/fn.column.html), [`row`](https://docs.rs/day-pieces/latest/day_pieces/fn.row.html), [`zstack`](https://docs.rs/day-pieces/latest/day_pieces/fn.zstack.html), [`grid`](https://docs.rs/day-pieces/latest/day_pieces/fn.grid.html), [`scroll`](https://docs.rs/day-pieces/latest/day_pieces/fn.scroll.html), [`spacer`](https://docs.rs/day-pieces/latest/day_pieces/fn.spacer.html), [`divider`](https://docs.rs/day-pieces/latest/day_pieces/fn.divider.html), [`form`](https://docs.rs/day-pieces/latest/day_pieces/fn.form.html)/[`section`](https://docs.rs/day-pieces/latest/day_pieces/fn.section.html) |
| Structure | [`when`](https://docs.rs/day-pieces/latest/day_pieces/fn.when.html), [`each`](https://docs.rs/day-pieces/latest/day_pieces/fn.each.html), [`with_environment`](https://docs.rs/day-pieces/latest/day_pieces/fn.with_environment.html) |
| Collections | [`list`](https://docs.rs/day-pieces/latest/day_pieces/fn.list.html) (native recycling) |
| Drawing | [`canvas`](https://docs.rs/day-pieces/latest/day_pieces/fn.canvas.html), [`shape`](https://docs.rs/day-pieces/latest/day_pieces/fn.shape.html) ([`rectangle`](https://docs.rs/day-pieces/latest/day_pieces/fn.rectangle.html), [`circle`](https://docs.rs/day-pieces/latest/day_pieces/fn.circle.html), [`capsule`](https://docs.rs/day-pieces/latest/day_pieces/fn.capsule.html), [`arc`](https://docs.rs/day-pieces/latest/day_pieces/fn.arc.html), …), [`image`](https://docs.rs/day-pieces/latest/day_pieces/fn.image.html), [`vector`](https://docs.rs/day-pieces/latest/day_pieces/fn.vector.html) |
| Navigation | [`nav`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav.html), [`nav_stack`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav_stack.html), [`nav_link`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav_link.html), [`toolbar`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.toolbar) |
| Presentation | [`alert`](https://docs.rs/day-pieces/latest/day_pieces/fn.alert.html), [`confirm`](https://docs.rs/day-pieces/latest/day_pieces/fn.confirm.html), [`prompt`](https://docs.rs/day-pieces/latest/day_pieces/fn.prompt.html), [`cover`](https://docs.rs/day-pieces/latest/day_pieces/fn.cover.html), menus |

Anything beyond this vocabulary (a map, a web view, a Lottie animation, an [embedded
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

- **[`build`](https://docs.rs/day-core/latest/day_core/trait.Piece.html#tymethod.build) takes `self`, not `&self`.** A Piece is consumed exactly once. There is no retained
  view description that Day re-runs and diffs against the last frame.
- **It returns an `RNode`,** a handle to a node in the *realized tree*: the live structure that
  owns the native widget, its layout state, and the [reactive](/docs/reference/glossary#reactive) scope its [bindings](/docs/reference/glossary#binding) live in.

Each Piece value is consumed once when built. Page and structural builders can be called again
when their host mounts a new subtree. Everything dynamic afterward flows through
[signals](/docs/explanation/reactivity), which are bound to individual native attributes during that single
build. The [reactivity page](/docs/explanation/reactivity) covers what that means for your code, including the
costs.


With `title = res::str::hi()`, the label's [`build`](https://docs.rs/day-core/latest/day_core/trait.Piece.html#tymethod.build) creates three things that live together:

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
  [`column`](https://docs.rs/day-pieces/latest/day_pieces/fn.column.html) and [`row`](https://docs.rs/day-pieces/latest/day_pieces/fn.row.html) get a plain native container view; decorators ([`padding`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding), [`frame`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.frame)) get no
  widget at all and exist purely in Day's tree.
- The **scope** owns every binding and event handler the build created. When the node is later
  removed (a [`when`](https://docs.rs/day-pieces/latest/day_pieces/fn.when.html) arm switches, an [`each`](https://docs.rs/day-pieces/latest/day_pieces/fn.each.html) row disappears), disposing the scope tears down its
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

[`each`](https://docs.rs/day-pieces/latest/day_pieces/fn.each.html) takes a *row source* and a row builder. [`items(data, key_of)`](https://docs.rs/day-pieces/latest/day_pieces/fn.items.html) is the row source for plain
data; a [model](/docs/explanation/model) collection supplies one directly, and [`list`](https://docs.rs/day-pieces/latest/day_pieces/fn.list.html) accepts the
same sources.

[`each`](https://docs.rs/day-pieces/latest/day_pieces/fn.each.html) compares the old and new key sequences to decide which rows to keep, build, or remove. A
[`when`](https://docs.rs/day-pieces/latest/day_pieces/fn.when.html) flip or a row removal is a real structural edit (native widgets are added and removed), so it
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
interactive elements; `day lint` catches an id used twice and a [`navigate`](https://docs.rs/day-pieces/latest/day_pieces/fn.navigate.html) to a [route](/docs/reference/glossary#route) that
doesn't exist.

`id!(save_button)` declares the ID; YAML refers to `save_button`. The catalog supplies
the human-readable title independently. For recycled rows, use [`.id_of`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id_of) so the ID follows
the current item. See [element IDs](/docs/reference/element-ids).

## Where Pieces come from

You can use pieces from the framework, combine them into your own components, or add native
controls through separate crates:

1. **Built-ins**: the vocabulary above, implemented in `day-pieces` with a renderer in every
   toolkit backend.
2. **Composite pieces**: plain Rust functions or builder structs that compose existing Pieces.
   They need no new native code; their target support is limited by the controls and capabilities they compose. Most of your app is this; so
   are the
   [composite-piece tutorial](/contributing/tutorials/composite-piece), [`day-piece-settings`](/docs/reference/settings), and the
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

## Add optional components

Optional crates compose with the same piece tree. Choose [charts](/docs/reference/charts),
[Lottie animations](/docs/reference/lottie), [media playback](/docs/reference/media),
[camera capture](/docs/reference/camera), or [web views](/docs/reference/webview) for richer
content. Each guide includes its Cargo dependency, platform requirements, and API links.
The [API index](/docs/reference/api) lists both built-in and external crates.
