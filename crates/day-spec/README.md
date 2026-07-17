# day-spec

The contract between Day's engine and its native backends.

Day runs on seven native toolkits, and every backend implements the same Rust trait:
`Toolkit`, defined here. This crate holds that trait and everything both sides must agree
on — the vocabulary of built-in pieces, the typed properties that describe them, the
events that flow back from native widgets, and the registry that lets other crates plug in
renderers of their own.

The split is deliberate. Backends depend on `day-spec` and nothing else of Day, which
keeps each one small and makes the headless test backend a true stand-in for the real
ones.

App code never uses this crate directly; its types arrive re-exported through
[`day`](https://crates.io/crates/day).

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
