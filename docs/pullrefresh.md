---
title: "Pull-to-refresh"
description: "The pull-to-refresh piece: native refresh controls where they exist, a faithful emulation where they don't."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Pull-to-refresh

Let people pull down on scrollable content to reload it. `day-piece-pullrefresh` wraps that
content, starts the app’s refresh action, and displays the refreshing state until the work ends.

## Add to your app

Defined in [`day-piece-pullrefresh`](https://github.com/daybrite/day/tree/main/pieces/day-piece-pullrefresh). Add this to your app’s `Cargo.toml`
`[dependencies]` table, using the same Day Git source as the rest of your app:

```toml
[dependencies]
day-piece-pullrefresh = { git = "https://github.com/daybrite/day.git" }
```

`day build -t <target>` enables this direct dependency’s matching toolkit feature
automatically. No dependency entry is needed in `Day.toml`.

| Platforms / toolkits | Availability |
| --- | --- |
| UIKit, Android, ArkUI | Native refresh control and pull gesture. |
| AppKit, GTK | Emulated indicator with native overscroll detection. |
| Qt, WinUI, Web | Composed indicator; start refresh programmatically rather than relying on a native pull gesture. |

## Authoring

```rust
use day_piece_pullrefresh::pull_to_refresh;

let refreshing = Signal::new(false);

// One reload path for EVERY begin — pull gesture, dayscript toggle, or a button:
watch(move || refreshing.get(), move |now, _| {
    if *now {
        let done = refreshing.setter();
        std::thread::spawn(move || {
            reload();            // off the UI thread
            done.set(false);     // completion hops back and dismisses the indicator
        });
    }
});

pull_to_refresh(refreshing, scroll(rows)).id(id!(feed_refresh))   // or list(items, …)
```

The bound `refreshing: Signal<bool>` is **two-way** (the same contract as [`UIRefreshControl`](https://developer.apple.com/documentation/uikit/uirefreshcontrol),
[`SwipeRefreshLayout.setRefreshing`](https://developer.android.com/reference/androidx/swiperefreshlayout/widget/SwipeRefreshLayout), and ArkUI `Refresh`):

- a **user pull** sets it `true` (and runs the optional `.on_refresh(f)` sugar);
- the app sets it **`false`** when the reload completes (from a thread via `Signal::setter`, or
  inside `day::task`);
- the app may set it **`true`** to begin a refresh programmatically (a toolbar button, ⌘R, …).

Prefer the `watch`-on-the-signal idiom above over `.on_refresh` when a programmatic path exists;
it makes every begin take the same route. `day_piece_pullrefresh::support()` reports the compiled
toolkit's tier (`Native` / `Emulated`).

## Scripting

The piece's node accepts `Event::ToggleChanged` as a synthetic begin/end, so the existing dayscript
`toggle:` step drives it identically on every toolkit:

```yaml
- toggle: { id: feed_refresh, value: true }     # begin (runs the app's reload)
- assert_text: { id: refresh_status, key: refresh_status_refreshing }
```

## Per-toolkit realization

Platform guidance: [Android: swipe-to-refresh](https://developer.android.com/develop/ui/views/touch-and-input/swipe).

| Target | Tier | Mechanism |
|---|---|---|
| ios-uikit | **Native** | A passthrough host view attaches a [`UIRefreshControl`](https://developer.apple.com/documentation/uikit/uirefreshcontrol) to the descendant [`UIScrollView`](https://developer.apple.com/documentation/uikit/uiscrollview) when day mounts it (`didAddSubview:`, which covers `list()`, since a [`UITableView`](https://developer.apple.com/documentation/uikit/uitableview) is a [`UIScrollView`](https://developer.apple.com/documentation/uikit/uiscrollview)). |
| android-mdc | **Native** | This crate's `DayPullRefresh extends SwipeRefreshLayout` (AndroidX, added to Gradle via `[package.metadata.day.android]`); the scrollable mounts directly into it. |
| harmony-arkui | **Native** | [`ARKUI_NODE_REFRESH`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/capi-native-node-h.md) created by this crate's own NDK shim; pull events via `NODE_REFRESH_ON_REFRESH`, indicator via `NODE_REFRESH_REFRESHING`. |
| macos-appkit | Emulated | Spinner-chip overlay + the pull gesture from elastic scrolling: the clip view's bounds go negative during a trackpad rubber-band; crossing ~60 pt begins a refresh. |
| gtk | Emulated | Overlay + [`GtkScrolledWindow::edge-overshot`](https://docs.gtk.org/gtk4/class.ScrolledWindow.html) (Top), GTK's purpose-built overshoot signal. |
| qt / xaml | Emulated | Overlay + programmatic only (desktop Qt has no elastic overscroll; XAML's `RefreshContainer` is touch-only; a native tier is a follow-up for touch devices). |
| mock | Emulated | Composition path; drives the piece's tests via `ToggleChanged`. |

The emulated indicator is the built-in `spinner()` in a floating chip, shown while `refreshing`:
pure composition (`when` + overlay container) with no per-toolkit code.

## The container-piece recipe

On the wrap-based platforms the realized node is the native refresh wrapper and day mounts the
scrollable **as a Day child inside it**; see [docs/extending](extending.md) ("Container pieces") for the
`cx.native` + fill-layout + `cx.under` recipe this piece establishes.

## Limits

- The wrapped child should be the scrollable (`scroll(...)` or `list(...)`). If the child's
  realized view isn't scroll-backed, the pull gesture is inert (the overlay + programmatic path
  still work).
- iOS: a programmatic begin shows the control's spinner without auto-revealing it (UIKit's
  standard behavior); a user pull reveals it naturally.
- Emulated gesture thresholds (AppKit ~60 pt, GTK overshoot) are heuristics tuned in the piece.
