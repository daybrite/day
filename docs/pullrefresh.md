# Pull-to-refresh (external piece)

> **Status: implemented** as `day-piece-pullrefresh`, an external Day Piece registered link-time
> into each backend's renderer slice without touching day. It is the reference **container piece**
> — the first external piece whose native view hosts a Day child (the wrapped scrollable) — and a
> reference for the native-where-possible / emulated-elsewhere pattern. Modeled on SwiftUI's
> `refreshable(action:)`.

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

pull_to_refresh(refreshing, scroll(rows)).id("feed-refresh")   // or list(items, …)
```

The bound `refreshing: Signal<bool>` is **two-way** (the same contract as `UIRefreshControl`,
`SwipeRefreshLayout.setRefreshing`, and ArkUI `Refresh`):

- a **user pull** sets it `true` (and runs the optional `.on_refresh(f)` sugar);
- the app sets it **`false`** when the reload completes — from a thread via `Signal::setter`, or
  inside `day::task`;
- the app may set it **`true`** to begin a refresh programmatically (a toolbar button, ⌘R, …).

Prefer the `watch`-on-the-signal idiom above over `.on_refresh` when a programmatic path exists —
it makes every begin take the same route. `day_piece_pullrefresh::support()` reports the compiled
backend's tier (`Native` / `Emulated`).

## Scripting

The piece's node accepts `Event::ToggleChanged` as a synthetic begin/end, so the existing dayscript
`toggle:` step drives it identically on every backend:

```yaml
- toggle: { id: feed-refresh, value: true }     # begin (runs the app's reload)
- assert_text: { id: refresh-status, key: refresh_status_refreshing }
```

## Per-toolkit realization

| Target | Tier | Mechanism |
|---|---|---|
| ios-uikit | **Native** | A passthrough host view attaches a `UIRefreshControl` to the descendant `UIScrollView` when day mounts it (`didAddSubview:` — covers `list()`: a `UITableView` IS a `UIScrollView`). |
| android-widget | **Native** | This crate's `DayPullRefresh extends SwipeRefreshLayout` (AndroidX, added to Gradle via `[package.metadata.day.android]`); the scrollable mounts directly into it. |
| ohos-arkui | **Native** | `ARKUI_NODE_REFRESH` created by this crate's own NDK shim; pull events via `NODE_REFRESH_ON_REFRESH`, indicator via `NODE_REFRESH_REFRESHING`. |
| macos-appkit | Emulated | Spinner-chip overlay + the pull gesture from ELASTIC scrolling: the clip view's bounds go negative during a trackpad rubber-band; crossing ~60 pt begins a refresh. |
| gtk | Emulated | Overlay + `GtkScrolledWindow::edge-overshot` (Top) — GTK's purpose-built overshoot signal. |
| qt / winui | Emulated | Overlay + programmatic only (desktop Qt has no elastic overscroll; WinUI's `RefreshContainer` is touch-only — native tier is a follow-up for touch devices). |
| mock | Emulated | Composition path; drives the piece's tests via `ToggleChanged`. |

The emulated indicator is the built-in `spinner()` in a floating chip, shown while `refreshing` —
pure composition (`when` + overlay container), no per-backend code.

## The container-piece recipe

On the wrap-based platforms the realized node IS the native refresh wrapper and day mounts the
scrollable **as a Day child inside it** — see docs/extending.md ("Container pieces") for the
`cx.native` + fill-layout + `cx.under` recipe this piece establishes.

## Limits

- The wrapped child should BE the scrollable (`scroll(...)` or `list(...)`). If the child's
  realized view isn't scroll-backed, the pull gesture is inert (the overlay + programmatic path
  still work).
- iOS: a programmatic begin shows the control's spinner without auto-revealing it (UIKit's
  standard behavior); a user pull reveals it naturally.
- Emulated gesture thresholds (AppKit ~60 pt, GTK overshoot) are heuristics tuned in the piece.
