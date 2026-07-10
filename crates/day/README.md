# day

Cross-platform apps in Rust, rendered with each platform's real native toolkit.

This is the umbrella crate a Day app depends on. It re-exports the piece library, the
reactive core, layout, localization, and — via one cargo feature per binary — exactly one
native backend:

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

That function is a real AppKit view hierarchy on macOS, real Material widgets on Android,
GTK 4 on Linux, and so on across seven toolkits. Pieces are built once; updates flow through
signals into targeted native mutations — there is no diffing pass and no retained virtual tree.

Most people meet Day through its command-line tool rather than this crate directly:
[`day-cli`](https://crates.io/crates/day-cli) scaffolds a project, builds and launches it per
target, drives scripted walkthroughs, and packages signed installers.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
