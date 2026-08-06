# day-piece-swiftui

Host your own SwiftUI views inside a Day app, on macOS and iOS.

Write ordinary SwiftUI in an ordinary SwiftPM package — with its own transitive
dependencies and its own `swift test` — and point your `Cargo.toml`'s
`[package.metadata.day.ios/macos]` at it. `day build` compiles the package into the app
and wraps each public view in a hosting view, and the build script generates a typed
Rust constructor per view, so the call-site is just:

```rust
crate::swiftui::MyView("hello", 42)
```

Arguments accept constants, signals, or closures; reactive values re-invoke the view's
initializer live, and SwiftUI keeps its `@State` across updates. For views that need
wiring the generated path can't express, subclass `DaySwiftUIProvider` and call
`swiftui("name")` directly. [docs/swiftui.md](../../docs/swiftui.md) has the full story.

Pieces are Day's reusable UI components, shipped as ordinary crates: one Rust API in
front, a real native control per platform behind it. Enable the backends you build for
with cargo features, and `day build` wires up the native side automatically.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, XAML, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
