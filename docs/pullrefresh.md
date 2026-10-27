---
title: "Pull-to-refresh"
description: "The pull-to-refresh piece: native refresh controls where they exist, a faithful emulation where they don't."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Pull-to-refresh

Pull to refresh lets people request an update by pulling down on scrollable content. A refresh
indicator shows that the request is being handled until the new content is ready.

`day-pieces` wraps that content, starts the app’s refresh action, and displays the refreshing state
until the work ends.

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.
The selected toolkit supplies its native implementation; [`day::pullrefresh::support()`](https://docs.rs/day-pieces/latest/day_pieces/pullrefresh/fn.support.html) reports
its availability after the Day runtime starts.

| Platforms / toolkits | Availability |
| --- | --- |
| UIKit, Android, ArkUI | Native refresh control and pull gesture. |
| AppKit, GTK | Emulated indicator with native overscroll detection. |
| Qt, WinUI, Web | Composed indicator; start refresh programmatically rather than relying on a native pull gesture. |

## Authoring

```rust
use day::pullrefresh::pull_to_refresh;

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

- a **user pull** sets it `true` (and runs the optional [`.on_refresh(f)`](https://docs.rs/day-pieces/latest/day_pieces/pullrefresh/trait.PullRefreshBuilder.html#tymethod.on_refresh) sugar);
- the app sets it **`false`** when the reload completes (from a thread via [`Signal::setter`](https://docs.rs/day-reactive/latest/day_reactive/struct.Signal.html#method.setter), or
  inside [`day::task`](https://docs.rs/day-app/latest/day/fn.task.html));
- the app may set it **`true`** to begin a refresh programmatically (a toolbar button, ⌘R, …).

Prefer the [`watch`](https://docs.rs/day-reactive/latest/day_reactive/fn.watch.html)-on-the-signal idiom above over [`.on_refresh`](https://docs.rs/day-pieces/latest/day_pieces/pullrefresh/trait.PullRefreshBuilder.html#tymethod.on_refresh) when a programmatic path exists;
it makes every begin take the same route. [`day_pieces::pullrefresh::support()`](https://docs.rs/day-pieces/latest/day_pieces/pullrefresh/fn.support.html) reports the active
toolkit's indicator tier (`Native` / `Emulated`).
[`day::pullrefresh::gesture_support()`](https://docs.rs/day-pieces/latest/day_pieces/pullrefresh/fn.gesture_support.html) separately reports whether a user pull can start a refresh.
A composed indicator alone does not imply gesture support.

## Toolkit implementations

Platform guidance: [Android: swipe-to-refresh](https://developer.android.com/develop/ui/views/touch-and-input/swipe).

| Target | Tier | Mechanism |
|---|---|---|
| ios-uikit | **Native** | A passthrough host view attaches a [`UIRefreshControl`](https://developer.apple.com/documentation/uikit/uirefreshcontrol) to the descendant [`UIScrollView`](https://developer.apple.com/documentation/uikit/uiscrollview) when day mounts it (`didAddSubview:`, which covers [`list()`](https://docs.rs/day-pieces/latest/day_pieces/fn.list.html), since a [`UITableView`](https://developer.apple.com/documentation/uikit/uitableview) is a [`UIScrollView`](https://developer.apple.com/documentation/uikit/uiscrollview)). |
| android-material | **Native** | The Android toolkit's `DayPullRefresh extends SwipeRefreshLayout` (AndroidX, added to Gradle via `[package.metadata.day.android]`); the scrollable mounts directly into it. |
| harmony-arkui | **Native** | [`ARKUI_NODE_REFRESH`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/capi-native-node-h.md) created by the toolkit's own NDK shim; pull events via `NODE_REFRESH_ON_REFRESH`, indicator via `NODE_REFRESH_REFRESHING`. |
| macos-appkit | Emulated | Spinner-chip overlay + the pull gesture from elastic scrolling: the clip view's bounds go negative during a trackpad rubber-band; crossing ~60 pt begins a refresh. |
| gtk | Emulated | Overlay + [`GtkScrolledWindow::edge-overshot`](https://docs.gtk.org/gtk4/class.ScrolledWindow.html) (Top), GTK's purpose-built overshoot signal. |
| Qt / WinUI | Emulated | Overlay + programmatic only (desktop Qt has no elastic overscroll; WinUI's `RefreshContainer` is touch-only; a native tier is a follow-up for touch devices). |
| mock | Emulated | Composition path; drives the piece's tests via `ToggleChanged`. |

The emulated indicator is the built-in [`spinner()`](https://docs.rs/day-pieces/latest/day_pieces/fn.spinner.html) in a floating chip, shown while `refreshing`:
pure composition ([`when`](https://docs.rs/day-pieces/latest/day_pieces/fn.when.html) + overlay container) with no per-toolkit code.

## The container-piece recipe

On the wrap-based platforms the realized node is the native refresh wrapper and day mounts the
scrollable **as a Day child inside it**; see [docs/extending](extending.md) ("Container pieces") for the
`cx.native` + fill-layout + `cx.under` recipe this piece establishes.

## Limits

- The wrapped child should be the scrollable ([`scroll(...)`](https://docs.rs/day-pieces/latest/day_pieces/fn.scroll.html) or [`list(...)`](https://docs.rs/day-pieces/latest/day_pieces/fn.list.html)). If the child's
  realized view isn't scroll-backed, the pull gesture is inert (the overlay + programmatic path
  still work).
- iOS: a programmatic begin shows the control's spinner without auto-revealing it (UIKit's
  standard behavior); a user pull reveals it naturally.
- Emulated gesture thresholds (AppKit ~60 pt, GTK overshoot) are heuristics tuned in the piece.
