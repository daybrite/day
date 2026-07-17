# day

Write your app once, in Rust, and run it as a real native app on macOS, iOS, Android,
Linux, Windows, and HarmonyOS.

`day` is the crate a Day app depends on. It gathers the whole framework — the widget
library, the reactive state system, layout, and localization — and, through one cargo
feature per build, exactly one native backend. Your code describes the interface; the
backend builds it from the platform's own widgets.

Here is a complete counter:

```rust
use day::prelude::*;

fn counter() -> AnyPiece {
    let count = Signal::new(0i64);
    column((
        label(move || format!("{} clicks", count.get())),
        button("+1").action(move || count.update(|c| *c += 1)),
    ))
    .spacing(12.0)
    .padding(16.0)
    .any()
}
```

On macOS, that function produces real AppKit views. On Android it produces Material
widgets, on Linux GTK, and so on across seven toolkits. The interface is built once; when
a signal changes, Day updates just the native widget that depends on it. There is no
virtual tree and no diffing pass.

Most people start with the command-line tool rather than this crate:
[`day-cli`](https://crates.io/crates/day-cli) creates a project, builds and runs it on any
target, tests it with scripted walkthroughs, and packages signed installers. Run
`cargo install --locked day-cli`, then `day new`.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
