# day-reactive

The state system Day apps are built on: signals, memos, and effects.

A `Signal` holds a value. Read it inside a binding and the dependency is recorded; write
it and exactly the code that depended on it runs again. A `Memo` caches a derived value; an
`Effect` runs side effects when its inputs change. You never register listeners or
unsubscribe by hand — tracking is automatic, and cleanup follows the ownership scopes your
UI creates, so state disappears with the screen that owned it.

Everything runs on the main thread, which keeps the design small: no locks, no `Send`
bounds, no async runtime. Background threads hand results back through a `Setter`, a
write-only handle that is safe to send between threads.

The crate stands on its own, but it was written for
[`day`](https://crates.io/crates/day), which pairs it with native widget trees.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
