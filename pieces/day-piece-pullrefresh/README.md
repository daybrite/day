# day-piece-pullrefresh

Pull-to-refresh for any Day scrollable — `scroll(...)` or `list(...)` — modeled on SwiftUI's
`refreshable(action:)`:

```rust
let refreshing = Signal::new(false);
pull_to_refresh(refreshing, scroll(rows))
    .on_refresh(move || {
        let done = refreshing.setter();
        std::thread::spawn(move || { reload(); done.set(false); });
    })
```

**Native** where the toolkit has a real implementation — `UIRefreshControl` (iOS), AndroidX
`SwipeRefreshLayout` (Android, added to the app's Gradle build automatically), `ARKUI_NODE_REFRESH`
(HarmonyOS) — and **emulated** elsewhere: a spinner overlay driven by the two-way `refreshing`
signal, with the pull gesture detected from AppKit's elastic scrolling and GTK's `edge-overshot`
signal (Qt/WinUI: overlay + programmatic). `support()` reports the compiled tier. Scriptable on
every backend through dayscript's existing `toggle:` step.

Also the reference **container piece**: the first external Day Piece whose native view hosts a Day
child (docs/extending.md §5 — the `cx.native` + fill-layout + `cx.under` recipe).

Docs: `docs/pullrefresh.md`. Demos: the showcase's Refresh page (plain scroll) and List page
(recycling list).

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust codebase.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
