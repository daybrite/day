# day-piece-rating

Compose-tier example pieces for Day — a star rating control among them — built purely
from Day's public primitives (`row`, `canvas`, gestures, signals) with **zero** per-toolkit
code.

This crate exists to prove the composition tier: if a component can be expressed from core
pieces, it runs on every backend for free.

Pieces are Day's extension unit: a crate with one Rust API and per-toolkit native
renderers, enabled per backend by cargo features (`day build` wires them automatically).

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
