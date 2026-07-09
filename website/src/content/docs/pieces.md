---
title: Pieces
description: "Day's unit of UI: what a Piece is, how trees are composed, and what happens when one is built."
order: 11
section: Concepts
---

A **Piece** is Day's unit of UI composition — the thing SwiftUI calls a View and Flutter calls a
Widget. You compose your interface as a tree of Pieces, and Day realizes each one as a real
native widget: a `label` becomes an `NSTextField` on macOS, a `TextView` on Android, a `GtkLabel`
on Linux.

This page covers what a Piece is, how you compose them, and what actually happens when one is
built. If you'd rather see the whole API in one sitting first, the [API tour](/docs/api-tour) is
the faster read; come back here for the model behind it.

## A Piece is a description, built once

In code, a Piece is a small plain value — usually a builder struct returned by a free function:

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

Two things in that signature shape everything else about Day:

- **`build` takes `self`, not `&self`.** A Piece is consumed exactly once. There is no retained
  view description that Day re-runs and diffs against the last frame — the builder is spent the
  moment the native widget exists.
- **It returns an `RNode`,** a handle to a node in the *realized tree*: the live structure that
  owns the native widget, its layout state, and the reactive scope its bindings live in.

Your Piece functions run once, at mount time. Everything dynamic afterward flows through
[signals](/docs/reactivity), which are bound to individual native attributes during that single
build. This is the core trade Day makes: you give up "re-run the view function and let the
framework figure it out", and in exchange there is no virtual tree, no diffing, and no
re-execution of your UI code at runtime. The [reactivity page](/docs/reactivity) covers what that
means in practice, including the costs.

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

`AnyPiece` is the type-erased form — a boxed build closure. You'll use it whenever a function
returns "some Piece" without naming the concrete builder type, which in practice is every page
and component function you write:

```rust
fn settings_page() -> AnyPiece {
    column((
        label(tr("settings-title")).font(Font::Title),
        toggle(dark_mode),
    ))
    .any()   // Decorate::any() erases the concrete type
}
```

Most modifiers (`.id()`, `.padding()`, `.on_tap()` …) already return `AnyPiece`, so the trailing
`.any()` is only needed when the last call in the chain is a piece-specific method.

## The built-in vocabulary

The `day` prelude ships a deliberately small set of Pieces. Roughly grouped:

| Group | Pieces |
|---|---|
| Text | `label` |
| Controls | `button`, `toggle`, `slider`, `text_field`, `progress`, `spinner` |
| Layout | `column`, `row`, `zstack`, `scroll`, `spacer`, `divider` |
| Structure | `when`, `each`, `with_environment` |
| Collections | `list` (native recycling) |
| Drawing | `canvas`, `shape` (`rectangle`, `circle`, `capsule`, `arc`, …), `image` |
| Navigation | `selector`, `stack`, `nav_link` |
| Presentation | `alert`, `confirm`, `prompt`, menus |

Anything beyond this vocabulary — a combo box, a map, a web view, a Lottie animation — lives in
a separate *piece crate* (`day-piece-*`) that you add as an ordinary Cargo dependency. That
split is intentional: the core stays small enough to audit and port, and optional widgets don't
cost you anything unless you use them. The [extension model](/docs/extending) explains how those
crates plug in without touching Day itself.

Each built-in has a reference page with per-platform notes under
[internal reference](/docs/reference) — for example [text](/docs/internal/text),
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
- The **native widget** is created immediately through the toolkit backend (an `NSButton`, a
  `GtkEntry`, …) and inserted into its native parent at the right index. Layout-only Pieces —
  `column`, `row`, `padding` wrappers — get no widget at all; they exist purely in Day's tree.
- The **scope** owns every binding and event handler the build created. When the node is later
  removed (a `when` arm switches, an `each` row disappears), disposing the scope tears down its
  bindings and handlers in one step, and the native widget is released. No manual unsubscription.

The details of that machinery — the tree structure, measurement, and how events travel back —
are on [How rendering works](/docs/rendering).

## Conditional and repeated structure

Because build runs once, structural change is explicit rather than implicit. Two Pieces express
it:

```rust
// A subtree that exists only while the condition holds. The closure re-runs
// when `cond`'s signals change; the old arm's scope is disposed.
when(move || logged_in.get(), move || profile_panel())

// A keyed collection. Rows are created, moved, and disposed by key diffing —
// surviving rows keep their nodes and native widgets.
each(
    move || todos.get(),          // data
    |t| t.id,                     // stable key
    |slot| todo_row(slot),        // per-row builder; slot tracks the item
)
```

This is the one place Day does anything diff-like, and it diffs *keys*, not widget trees: `each`
compares the old and new key sequences to decide which rows to keep, which to build, and which to
dispose. A `when` flip or a row removal is a real structural edit — native widgets are added and
removed — so it costs more than a bound-attribute update. For long scrolling data, prefer
[`list`](/docs/internal/list), which hands rows to the platform's recycling list widget instead
of materializing every row.

## Identity, for testing and accessibility

Any Piece can carry a stable string id:

```rust
button(tr("save")).action(save).id("save-button")
```

Ids serve three audiences at once: [dayscript](/docs/dayscript) targets elements by id,
[accessibility](/docs/accessibility) uses them as stable automation identifiers, and you'll see
them in debug output. They're optional everywhere, but pages you intend to test should id their
interactive elements — `day lint` will point out interactive Pieces without one.

## Where Pieces come from

There are exactly three kinds of Piece, and you can write all three:

1. **Built-ins** — the vocabulary above, implemented in `day-pieces` with a renderer in every
   toolkit backend.
2. **Composite pieces** — plain Rust functions or builder structs that compose existing Pieces.
   No native code, works on every target automatically. Most of your app is this; so is
   something like a [star-rating widget](/docs/tutorial-composite-piece).
3. **Native pieces** — a new leaf widget with a per-toolkit implementation, registered at link
   time. This is how `day-piece-webview` wraps `WKWebView`/`WebView`/`WebKitGTK`, and how you'd
   wrap a platform control Day doesn't cover. See the
   [native piece tutorial](/docs/tutorial-native-piece).

Composite pieces are frictionless; native pieces cost one implementation per toolkit you care
about (a piece that only implements AppKit and UIKit renders a labeled placeholder elsewhere —
visible, not a crash).

---

Next: [Reactivity](/docs/reactivity) — the signals that make a built-once tree move.
