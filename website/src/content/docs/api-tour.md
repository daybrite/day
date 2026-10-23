---
title: API tour
description: A guided tour of Pieces, signals, layout, inputs, navigation, localization, and extensibility.
order: 10
section: Reference
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Browse examples of common controls and UI patterns. These are API fragments, not complete apps.
Use the
[beginner journey](/docs/tutorials/first-app) and
[technical recipes](/docs/reference/examples/catalog#technical-recipes) for designated compiled code. They use `day::prelude::*` for the common APIs. For the underlying
concepts, see [Pieces](/docs/explanation/pieces), [Reactivity](/docs/explanation/reactivity), and [Layout](/docs/explanation/layout).

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/examples/catalog) and [catalog setup](/docs/how-to/localization).

## A first app

[`launch`](https://daybrite.dev/api/day/fn.launch.html) takes window options and a `root` closure that returns a Piece. Entry points and loop
ownership vary by toolkit; mobile/web hosts can own the loop. The desktop fragment below needs
a manifest, build script, and resource catalog as shown in the first-app tutorial.

```rust
use day::prelude::*;

day::resources!();

fn main() {
    day::launch(res::window_options().size(480.0, 640.0), root);
}

fn root() -> impl Piece {
    label(res::str::hello_native_world()).padding(Space::XL)
}
```

## Signals: state that binds

A `Signal<T>` is a `Copy` [reactive](/docs/reference/glossary#reactive) cell; copying copies a handle to one shared slot, not the
value, so the same [signal](/docs/reference/glossary#signal) can live in as many closures as you like. Reading inside
a bound closure subscribes it; writing re-runs exactly the closures that read it
([Reactivity](/docs/explanation/reactivity) explains the model).

```rust
let count = Signal::new(0i64);

count.get();            // read (tracks the caller as a dependency)
count.set(5);           // replace
count.update(|c| *c += 1);   // mutate in place
count.with(|c| c.abs());     // borrow without cloning
count.get_untracked();  // read without creating a dependency
```

A closure passed to a reactive property, such as a label’s text, tracks its signal reads.
Changing the signal reruns that [binding](/docs/reference/glossary#binding). An ordinary closure or event
handler does not become reactive just because it reads a signal.

```rust
// The message reads the signal and follows both count and locale changes.
label(res::str::click_count(count))
```

## Text, buttons, and layout

([Layout](/docs/explanation/layout) is the full model behind the containers here.)

Pieces compose with plain function calls; containers take a tuple of children and expose builder
methods for spacing, padding, and alignment.

```rust
column((
    label(res::str::counter()).font(Font::Title),
    row((
        button(res::str::decrement())
            .action(move || count.update(|c| *c -= 1))
            .id(id!(decrement_button)),
        label(res::str::counter_value(count)).id(id!(counter_label)),
        button(res::str::increment())
            .action(move || count.update(|c| *c += 1))
            .id(id!(increment_button)),
    ))
    .spacing(Space::S),
    divider(),
    spacer(),
))
.spacing(Space::M)
.align(HAlign::Leading)
.padding(Space::L)
```

Wrap any subtree in [`scroll(...)`](https://daybrite.dev/api/day_pieces/fn.scroll.html) to make it scroll natively.

## Inputs

(Each input is a [piece](/docs/reference/glossary#piece); [Pieces](/docs/explanation/pieces) covers the vocabulary.)

Editable controls take a signal directly. User input changes the signal, and changes from
your code update the control.

```rust
let name = Signal::new(String::new());
let volume = Signal::new(40.0);
let subscribed = Signal::new(false);
#[derive(Clone, Copy, PartialEq)]
enum Size {
    S,
    M,
    L,
}
let size = Signal::new(Size::M);

column((
    labeled(
        res::str::name(),
        text_field(name)
            .placeholder(res::str::your_name())
            .id(id!(name_field)),
    ),
    labeled(
        res::str::volume(),
        slider(volume).range(0.0..=100.0).id(id!(volume_slider)),
    ),
    labeled(
        res::str::subscribe(),
        toggle(subscribed).id(id!(subscribe_toggle)),
    ),
    picker(
        size,
        [
            (Size::S, res::str::small()),
            (Size::M, res::str::medium()),
            (Size::L, res::str::large()),
        ],
    )
    .segmented()
    .a11y(|a| a.label(res::str::size()))
    .id(id!(size_picker)),
))
```

[`picker`](https://daybrite.dev/api/day_pieces/fn.picker.html) is one-of-N over a typed value (`size` is a `Signal<Size>` here, any `Clone + PartialEq`
type works) with three native stylings ([`.menu()`](https://daybrite.dev/api/day_pieces/trait.PickerBuilder.html#tymethod.menu), [`.segmented()`](https://daybrite.dev/api/day_pieces/trait.PickerBuilder.html#tymethod.segmented), [`.inline()`](https://daybrite.dev/api/day_pieces/trait.PickerBuilder.html#tymethod.inline));
[`text_area`](https://daybrite.dev/api/day_pieces/fn.text_area.html) is the multi-line counterpart of [`text_field`](https://daybrite.dev/api/day_pieces/fn.text_field.html).

Keyboard focus is a binding too: [`.focused(editing)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.focused) ties a control to a `Signal<bool>`, or
[`.focused((field, Field::Name))`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.focused) binds one control of a form sharing a `Signal<Option<Field>>`:
native focus changes write the signal, writing the signal moves focus (and `None` dismisses the
soft keyboard on mobile). [`text_field`](https://daybrite.dev/api/day_pieces/fn.text_field.html)`(...)`[`.on_submit`](https://daybrite.dev/api/day_pieces/trait.TextFieldBuilder.html#tymethod.on_submit)`(...)` handles the Return key, so chaining
fields is one signal write. The [focus reference](/docs/reference/focus) has the rules and the
per-platform map.

## Conditionals and collections

[`when`](https://daybrite.dev/api/day_pieces/fn.when.html) shows a subtree while a condition holds; it is itself reactive. Chain [`.otherwise`](https://daybrite.dev/api/day_pieces/trait.WhenBuilder.html#tymethod.otherwise) for the
else arm.

```rust
when(
    move || !name.with(|s| s.is_empty()),
    move || label(res::str::greeting(name)),
)
.otherwise(|| label(res::str::tell_me_your_name()))
```

Keyed collections ([`each`](https://daybrite.dev/api/day_pieces/fn.each.html)) build one child per item and reconcile by key when the list changes,
so each row keeps its own state across updates.

## Progress and canvas

[`progress`](https://daybrite.dev/api/day_pieces/fn.progress.html) takes a fraction (a value or a reactive closure); [`spinner`](https://daybrite.dev/api/day_pieces/fn.spinner.html) is indeterminate. [`canvas`](https://daybrite.dev/api/day_pieces/fn.canvas.html)
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

(The whole model, with per-platform mappings: [Navigation](/docs/how-to/navigation).)

Navigation state lives in signals too. Changing a selection or path updates the native
container; using its tabs or back button updates the signal. Choose between two containers:

**[`nav`](https://daybrite.dev/api/day_pieces/fn.nav.html)** is a one-of-N choice bound to a `Signal<String>`. Its [`.style`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.style) picks the native
chrome: `Sidebar` becomes a `NavigationSplitView` (an [`AdwNavigationSplitView`](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.NavigationSplitView.html) on GTK, an
[`NSSplitView`](https://developer.apple.com/documentation/appkit/nssplitview) source list on macOS, a pushing list on mobile); `Tabs` becomes a native tab widget.

```rust
let section = Signal::new(String::new());
nav(section)
    .style(NavStyle::Sidebar)
    .title(res::str::my_app())
    .header(sidebar_header)
    .item("home", res::str::home(), home_page)
    .item("settings", res::str::settings(), settings_page)
```

**[`nav_stack`](https://daybrite.dev/api/day_pieces/fn.nav_stack.html)** is a push/pop stack bound to a `Signal<Vec<String>>` path. Day reconciles the
native stack ([`UINavigationController`](https://developer.apple.com/documentation/uikit/uinavigationcontroller), [`AdwNavigationView`](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.NavigationView.html), the Android back stack) to the path.

```rust
let path = Signal::new(Vec::<String>::new());
nav_stack(path, home_view).destination(|key| detail_view(key))
// push:  path.update(|p| p.push("item-42".into()));
// the native back button writes the pop back into `path`.
```

Each navigation surface uses a separate signal. A `Tabs` [nav host](/docs/reference/glossary#nav host) or a [`nav_stack`](https://daybrite.dev/api/day_pieces/fn.nav_stack.html)
inside a `Sidebar` nav host can be nested without additional state synchronization. Keys don't have to be strings: declare a
`day::routes! { enum Section { Home => "home", … } }` enum (or implement `Route` by hand for
keys that carry data, like `Item { id: u32 }` ↔ `"item-42"`) and bind the nav host to
`Signal<Option<Section>>` and the stack to `Signal<Vec<Item>>`. It's the same API, compile-checked
([navigation guide](/docs/how-to/navigation#typed-routes)).

## Deep links and dayscript

A string-route adapter maps those signals to routes, so keys also serve as [routes](/docs/reference/glossary#route):

```rust
navigate("settings");   // select the settings section / tab
nav_back();             // pop the innermost surface
current_route();        // the full path, outermost surface first
```

The same keys drive deep links (`DAY_DEEPLINK=settings`;
[deep links reference](/docs/reference/deep-links)) and [dayscript](/docs/reference/glossary#dayscript) automation
(`navigate: { route: settings }`; [testing with dayscript](/docs/how-to/dayscript)).

## Localization and accessibility

Text localizes through [Fluent](/docs/reference/glossary#fluent) with [`tr`](https://daybrite.dev/api/day/fn.tr.html), including interpolated signal arguments. Every Piece can
carry accessibility metadata.

```rust
label(res::str::greeting(name));

progress(move || volume.get() / 100.0)
    .a11y(|a| a.role(Role::Meter).label(res::str::volume_level()));
```

## Ids and testing

Give any Piece a stable [`.id(id!(name))`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id) and dayscript can find, drive, and assert it, using the same
script on every platform.

```rust
button(res::str::increment())
    .action(move || count.update(|c| *c += 1))
    .id(id!(increment_button))
```

## Extending with Day Pieces

A native component you write (or install) plugs in like a built-in. The showcase's flavor
picker is an external `combo_box` Piece from a separate crate, with free-form text entry plus a
native dropdown, both bound to signals:

```rust
use day_pieces::combobox::combo_box;

let flavors = Signal::new(vec!["vanilla".into(), "chocolate".into()]);
let flavor  = Signal::new(String::new());   // the typed-or-picked text
combo_box(flavors, flavor).id(id!(flavor_combo))
```

Day Pieces ship as ordinary Rust crates. [The extension model](/contributing/extending) explains the
tiers, from pure composition to per-toolkit native code. On macOS and iOS the same mechanism
hosts your own SwiftUI: declare a local Swift package and call the generated typed constructor
(`crate::swiftui::MyView(title, count)`) like any other piece
([SwiftUI embedding](/docs/reference/swiftui)).

Next: [Pieces](/docs/explanation/pieces) for the model behind all of this, or the
[CLI & projects](/docs/reference/cli) that build, launch, and script it.
