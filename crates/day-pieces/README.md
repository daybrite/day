# day-pieces

Day's built-in widget library — the functions Day apps are written in.

Labels, buttons, text fields, toggles, sliders, images, layout containers, scrolling,
lists, tabs, navigation, dialogs, menus, and a drawing canvas: each is a plain Rust
function that returns a value, configured with builder methods. Pass a string and you get
fixed text; pass a signal or a closure and the widget updates live.

```rust
column((
    label(move || format!("Hello, {}!", name.get())),
    text_field(name).placeholder("Your name"),
))
```

Every piece becomes a real native control on every backend: the button you write once is
an `NSButton` on macOS and a Material button on Android.

Depend on [`day`](https://crates.io/crates/day) rather than on this crate — the umbrella
re-exports everything here through its prelude.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
