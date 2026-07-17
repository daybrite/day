# day-script

The engine that lets a script drive a running Day app: tap buttons, type text, navigate,
check what's on screen, and take screenshots.

Scripts are plain YAML. `day launch --script walkthrough.yaml` starts the app with the
engine listening, runs the steps in order, and fails loudly when an assertion doesn't hold
— which is how Day's own demo app is tested on every platform in CI. The same engine
answers `day drive`, which runs steps one at a time so a person or a coding agent can work
against a live app.

The engine listens only when invited: it starts its local server only if the launcher put
a port and a one-time token in the environment. An ordinary run of your app carries no
server at all.

It is embedded automatically by [`day`](https://crates.io/crates/day) and driven by
[`day-cli`](https://crates.io/crates/day-cli).

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
