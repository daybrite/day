# Activity spinner (external piece)

> **Status: implemented** as `day-piece-activity`, an external Day Piece (like `day-piece-media`)
> registered link-time into each backend's renderer slice without touching day. It wraps each
> toolkit's native indeterminate activity/loading spinner. Unlike the media player it has an
> **intrinsic size** (a spinner is a small square control), so it is a natural-size leaf: no
> `.frame(w, h)` is required to make it visible.

## Authoring

```rust
use day_piece_activity::activity;

let spinning = Signal::new(true);

activity()
    .animating(spinning) // a bool, a Signal<bool>, or a closure (default true)
    .large(false)        // default false: the platform's regular control size
    .id("spinner")

toggle(spinning).id("spinner-toggle"); // flips the same signal → starts/stops the spinner
```

`activity()` builds a native indeterminate spinner, animating by default. `.animating(_)` takes the
same disjoint conversions as `label`/`progress` (via `IntoReactive<bool, _>`): a plain `bool`, a
`Signal<bool>`, or a `Fn() -> bool` closure. A reactive source is watched: whenever it changes,
the piece writes a sparse `ActivityPatch::Animating(bool)` that starts or stops the native
animation. `.large(true)` selects the platform's large control size.

`Activity` implements `Piece`, so `.id()` / `.a11y()` / `.frame()` chain via `Decorate`. It is a
natural-size leaf (`Flex::default()`, and each backend's default `measure` returns the native
indicator's fitting size), so it takes exactly the space the control wants; wrap it in `.frame(w, h)`
only if you want to reserve a fixed region (e.g. to keep surrounding layout stable while it toggles).

There is deliberately no determinate mode here; that is day's built-in `progress(fraction)`
(docs/progress.md). This piece covers the indeterminate "work of unknown extent" spinner.

## Per-backend native realization

| | AppKit | UIKit | GTK | Qt | Android | WinUI |
|---|---|---|---|---|---|---|
| control | `NSProgressIndicator` (Spinning) | `UIActivityIndicatorView` | `gtk4::Spinner` | busy `QProgressBar` (range 0..0) | `android.widget.ProgressBar` | `ProgressRing` |
| native code | objc2-app-kit | objc2-ui-kit | gtk4 crate (core widget) | `src/lib-qt-shim.cpp` | `android/java/…/DayActivity.java` | `src/lib-winui-shim.cpp` |
| run/stop | `startAnimation:` / `stopAnimation:` | `startAnimating` / `stopAnimating` | `start()` / `stop()` | range 0..0 (busy) ↔ 0..1 (frozen) | `View.VISIBLE` ↔ `INVISIBLE` | `IsActive` |
| `.large` | `controlSize` Large/Regular | style Large/Medium | `set_size_request` 48/24 | bigger minimum size | `setScaleX/Y(1.5)` | Width/Height 48 |
| stopped state | stays visible (`displayedWhenStopped`) | stays visible (`hidesWhenStopped = false`) | stays visible (drawn static) | frozen empty bar | INVISIBLE (box kept) | `IsActive(false)` |

**Backend notes:**

- **AppKit**: `NSProgressIndicator` with `style = Spinning` and `indeterminate = true`. `.large`
  maps to `controlSize` (`NSControlSize::Large` vs `Regular`; needs objc2-app-kit's `NSCell`
  feature, which carries `NSControlSize`). `startAnimation:` / `stopAnimation:` are the run/stop
  calls; `setDisplayedWhenStopped(true)` keeps a stopped indicator on screen (a frozen indicator)
  instead of vanishing, matching UIKit.
- **UIKit**: `UIActivityIndicatorView` with the Large/Medium style. objc2-ui-kit binds the whole
  control, so unlike the media piece's hand-rolled `AVPlayerViewController`, no `extern_class!`
  shim is needed. `hidesWhenStopped = false` keeps a stopped indicator visible.
- **GTK**: `gtk4::Spinner` (a core widget, so the feature compiles everywhere). Its natural size is
  tiny, so the piece gives it a `set_size_request` square (48 for `.large`, else 24). A stopped
  spinner is drawn static.
- **Qt**: Qt ships no native spinner widget, so this crate's own C++ shim wraps a `QProgressBar`
  in busy mode (`setRange(0, 0)`), the usual Qt way to show indeterminate progress and the same
  technique day-qt uses for `spinner()`. build.rs compiles the shim against `Qt6Widgets` (already
  linked by day-qt-sys, so it emits no extra link flags). Animating toggles between busy (range
  0..0) and a frozen static bar (range 0..1, value 0). `.large` gives it a bigger minimum size. (A
  busy `QProgressBar` is a horizontal moving-chunk bar rather than a ring; that's what Qt provides
  natively.)
- **Android**: a framework `android.widget.ProgressBar`, whose default style is a circular
  indeterminate spinner, so the piece adds no Gradle dependencies and no permissions. A default
  indeterminate `ProgressBar` always animates while `VISIBLE`; the closest to a stopped-but-present
  spinner is `INVISIBLE` (which keeps the layout box so surrounding layout does not jump). `.large`
  scales the drawable via `setScaleX/Y`. The Java factory
  (`dev.daybrite.day.piece.activity.DayActivity`) is bundled with the crate under `android/java` and
  folded into the app's Gradle build via `[package.metadata.day.android]`, using only day-android's
  public `DayBridge.ctx`.
- **WinUI**: this crate's own C++/WinRT shim wraps a `Windows.UI.Xaml.Controls.ProgressRing` (UWP
  system XAML, no WinAppSDK), boxed via day-winui-sys's `day_winui_box` seam like the media / picker
  / webview WinUI pieces. `IsActive` runs/stops it; `.large` sets Width/Height. Written blind
  (Windows-only, built in CI); creation degrades to a `TextBlock` on any unexpected throw.
- **mock**: the feature exists (so an app can enable `day-piece-activity/mock` uniformly per
  backend) but registers no renderer; the activity kind falls back to day's placeholder leaf.

## Testing

The crate's smoke test boots the piece on the mock toolkit (which realizes unknown kinds as plain
widgets and ignores unknown patches, the same as a backend built without the feature), flips the
bound signal both ways, and must never panic: `cargo test -p day-piece-activity`.

For a live check, wire the showcase activity page to a spinner bound to a `Signal<bool>` and a toggle
over the same signal; the walkthrough navigates to the route, toggles the control, and screenshots
(one always-animating `.large` spinner keeps a visible spinning indicator in the shot).
