---
title: API tour
description: A guided tour of Pieces, signals, layout, inputs, navigation, localization, and extensibility.
order: 10
section: Concepts
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

This page shows the whole authoring surface in one pass, for people who learn from code.
Everything below is real Day API. The snippets are lifted from the showcase app that produces the
[gallery](/gallery), and `use day::prelude::*;` brings all of it in. The pages after this one
([Pieces](/docs/pieces), [Reactivity](/docs/reactivity), [Layout](/docs/layout)) explain the
model behind what you'll see here.

## A first app

`launch` takes window options and a `root` closure that returns the top Piece. It owns the native
main loop.

```rust
use day::prelude::*;

fn main() {
    day::launch(
        WindowOptions {
            title: "Hello".into(),
            size: Size::new(480.0, 640.0),
            ..Default::default()
        },
        root,
    );
}

fn root() -> impl Piece {
    label("Hello, native world").padding(24.0)
}
```

## Signals: state that binds

A `Signal<T>` is a `Copy` reactive cell; copying copies a handle to one shared slot, not the
value, so the same signal can live in as many closures as you like. Reading inside
a bound closure subscribes it; writing re-runs exactly the closures that read it
([Reactivity](/docs/reactivity) explains the model).

```rust
let count = Signal::new(0i64);

count.get();            // read (tracks the caller as a dependency)
count.set(5);           // replace
count.update(|c| *c += 1);   // mutate in place
count.with(|c| c.abs());     // borrow without cloning
count.get_untracked();  // read without creating a dependency
```

Any closure that reads a signal *becomes reactive*: when the signal changes, only that binding
re-runs, and nothing else in the tree is touched.

```rust
// This label re-reads `count` whenever it changes; nothing else is touched.
label(move || format!("{count} clicks", count = count.get()))
```

## Text, buttons, and layout

([Layout](/docs/layout) is the full model behind the containers here.)

Pieces compose with plain function calls; containers take a tuple of children and expose builder
methods for spacing, padding, and alignment.

```rust
column((
    label("Counter").font(Font::Title),
    row((
        button("–").action(move || count.update(|c| *c -= 1)),
        label(move || count.get().to_string()),
        button("+").action(move || count.update(|c| *c += 1)),
    ))
    .spacing(8.0),
    divider(),
    spacer(),
))
.spacing(12.0)
.align(HAlign::Leading)
.padding(16.0)
```

Wrap any subtree in `scroll(...)` to make it scroll natively.

## Inputs

(Each input is a piece; [Pieces](/docs/pieces) covers the vocabulary.)

Two-way controls take a signal directly; the user's edits flow back into it (origin-tagged, so
there is no feedback echo).

```rust
let name = Signal::new(String::new());
let volume = Signal::new(40.0);
let subscribed = Signal::new(false);
let size = Signal::new(0usize);

column((
    text_field(name).placeholder("Your name"),
    slider(volume).range(0.0..=100.0),
    toggle(subscribed),
    picker(["Small", "Medium", "Large"], size).segmented(),
))
```

`picker` is one-of-N with three native stylings (`.menu()`, `.segmented()`, `.inline()`);
`text_area` is the multi-line counterpart of `text_field`.

Keyboard focus is a binding too: `.focused(editing)` ties a control to a `Signal<bool>`, or
`.focused((field, Field::Name))` binds one control of a form sharing a `Signal<Option<Field>>`:
native focus changes write the signal, writing the signal moves focus (and `None` dismisses the
soft keyboard on mobile). `text_field(...).on_submit(...)` handles the Return key, so chaining
fields is one signal write. The [focus reference](/docs/internal/focus) has the rules and the
per-platform map.

## Conditionals and collections

`when` shows a subtree while a condition holds; it is itself reactive. Chain `.otherwise` for the
else arm.

```rust
when(
    move || !name.with(|s| s.is_empty()),
    move || label(move || format!("Hi, {}", name.get())),
)
.otherwise(|| label("Tell me your name"))
```

Keyed collections (`each`) build one child per item and reconcile by key when the list changes,
so each row keeps its own state across updates.

## Progress and canvas

`progress` takes a fraction (a value or a reactive closure); `spinner` is indeterminate. `canvas`
hands you a native 2D drawing surface; Day never rasterizes it itself.

```rust
progress(move || volume.get() / 100.0);   // determinate, tracks the slider live
spinner();                                 // indeterminate

canvas(move |d, size| {
    let r = Rect::from_size(size).inset(8.0);
    d.stroke(Shape::Arc { rect: r, start_deg: 135.0, sweep_deg: 270.0 },
             Color::rgba(0.5, 0.5, 0.55, 0.35), 6.0);
    let frac = (volume.get() / 100.0).clamp(0.0, 1.0);
    d.stroke(Shape::Arc { rect: r, start_deg: 135.0, sweep_deg: 270.0 * frac },
             Color::hex(0x2F6FDE), 6.0);
})
```

## Navigation

(The whole model, with per-platform mappings: [Navigation](/docs/navigation).)

Day models navigation as a projection of an app-owned signal: you own the state, and the native
container is reconciled to it. There are two primitives:

**`selector`** is a one-of-N choice bound to a `Signal<String>`. Its `.style` picks the native
chrome: `Sidebar` becomes a `NavigationSplitView` (an `AdwNavigationSplitView` on GTK, an
`NSSplitView` source list on macOS, a pushing list on mobile); `Tabs` becomes a native tab widget.

```rust
let section = Signal::new(String::new());
selector(section)
    .style(SelectorStyle::Sidebar)
    .title("My App")
    .header(sidebar_header)
    .item("home",     "Home",     home_page)
    .item("settings", "Settings", settings_page)
```

**`stack`** is a push/pop stack bound to a `Signal<Vec<String>>` path. Day reconciles the
native stack (`UINavigationController`, `AdwNavigationView`, the Android back stack) to the path.

```rust
let path = Signal::new(Vec::<String>::new());
stack(path, home_view).destination(|key| detail_view(key))
// push:  path.update(|p| p.push("item-42".into()));
// the native back button writes the pop back into `path`.
```

Because each surface owns its own signal, nesting costs nothing: a `Tabs` selector or a `stack`
inside a `Sidebar` selector needs no extra wiring. Keys don't have to be strings: declare a
`day::routes! { enum Section { Home => "home", … } }` enum (or implement `Route` by hand for
keys that carry data, like `Item { id: u32 }` ↔ `"item-42"`) and bind the selector to
`Signal<Option<Section>>` and the stack to `Signal<Vec<Item>>`. It's the same API, compile-checked
([navigation guide](/docs/navigation#typed-routes)).

## Deep links and dayscript

A thin string-route adapter sits over those signals, so keys double as routes:

```rust
navigate("settings");   // select the settings section / tab
nav_back();             // pop the innermost surface
current_route();        // the full path, outermost surface first
```

The same keys drive deep links (`DAY_DEEPLINK=settings`;
[deep links reference](/docs/internal/deep-links)) and dayscript automation
(`navigate: { route: settings }`; [testing with dayscript](/docs/dayscript)).

## Localization and accessibility

Text localizes through Fluent with `tr`, including interpolated signal arguments. Every Piece can
carry accessibility metadata.

```rust
label(tr("greeting").arg("name", name));

progress(move || volume.get() / 100.0)
    .a11y(|a| a.role(Role::Meter).label("Volume level"));
```

## Ids and testing

Give any Piece a stable `.id("…")` and dayscript can find, drive, and assert it, using the same
script on every platform.

```rust
button("Increment").action(move || count.update(|c| *c += 1)).id("increment-button")
```

## Extending with Day Pieces

A native component you write (or install) plugs in like a built-in. The showcase's flavor
picker is an external `combo_box` Piece from a separate crate, with free-form text entry plus a
native dropdown, both bound to signals:

```rust
use day_piece_combobox::combo_box;

let flavors = Signal::new(vec!["vanilla".into(), "chocolate".into()]);
let flavor  = Signal::new(String::new());   // the typed-or-picked text
combo_box(flavors, flavor).id("flavor-combo")
```

Day Pieces ship as ordinary Rust crates. [The extension model](/docs/extending) explains the
tiers, from pure composition to per-toolkit native code. On macOS and iOS the same mechanism
hosts your own SwiftUI: declare a local Swift package and call the generated typed constructor
(`crate::swiftui::MyView(title, count)`) like any other piece
([SwiftUI embedding](/docs/internal/swiftui)).

Next: [Pieces](/docs/pieces) for the model behind all of this, or the
[CLI & projects](/docs/cli) that build, launch, and script it.
