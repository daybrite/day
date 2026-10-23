---
navTitle: "Spinners"
title: "Activity spinner"
description: "The activity spinner API, defaults, and native renderer availability."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Activity spinner

An activity indicator shows that work is in progress when the app cannot report a percentage.
`day-piece-activity` provides a native loading spinner that sizes itself without an explicit frame.
The CLI selects the toolkit feature; web and mock have no native activity renderer.

## Add to your app

Defined in [`day-piece-activity`](https://github.com/daybrite/day/tree/main/pieces/day-piece-activity). Add this to your app’s `Cargo.toml`
`[dependencies]` table, using the same Day Git source as the rest of your app:

```toml
[dependencies]
day-piece-activity = { git = "https://github.com/daybrite/day.git" }
```

`day build -t <target>` enables this direct dependency’s matching toolkit feature
automatically. No dependency entry is needed in `Day.toml`.

| Platforms / toolkits | Availability |
| --- | --- |
| AppKit, UIKit, GTK, Qt, Android, WinUI, ArkUI | Native activity indicator; Qt uses a busy progress bar. |
| Web | No renderer; use the built-in [spinner](/docs/reference/progress). |

## Authoring

```rust
use day_piece_activity::activity;

let spinning = Signal::new(true);

activity()
    .animating(spinning) // a bool, a Signal<bool>, or a closure (default true)
    .id(id!(spinner))       // .large() for the platform's large control size

toggle(spinning).id(id!(spinner_toggle)); // flips the same signal → starts/stops the spinner
```

[`activity()`](https://daybrite.dev/api/day_piece_activity/fn.activity.html) builds a native indeterminate spinner, animating by default. [`.animating(_)`](https://daybrite.dev/api/day_piece_activity/trait.ActivityBuilder.html#tymethod.animating) takes the
same disjoint conversions as [`label`](https://daybrite.dev/api/day_pieces/fn.label.html)/[`progress`](https://daybrite.dev/api/day_pieces/fn.progress.html) (via `IntoReactive<bool, _>`): a plain `bool`, a
`Signal<bool>`, or a `Fn() -> bool` closure. A reactive source is watched: whenever it changes,
the piece writes a sparse `ActivityPatch::Animating(bool)` that starts or stops the native
animation. [`.large()`](https://daybrite.dev/api/day_piece_activity/trait.ActivityBuilder.html#tymethod.large) selects the platform's large control size; the default is the regular one.

`Activity` implements `Piece`, so [`.id()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id) / [`.a11y()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.a11y) / [`.frame()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame) chain via `Decorate`. It is a
natural-size leaf ([`Flex::default()`](https://daybrite.dev/api/day_core/struct.Flex.html#method.default), and each toolkit's default `measure` returns the native
indicator's fitting size), so it takes exactly the space the control wants; wrap it in [`.frame(w, h)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame)
only if you want to reserve a fixed region (e.g. to keep surrounding layout stable while it toggles).

There is no determinate mode here, because that is day's built-in [`progress(fraction)`](https://daybrite.dev/api/day_pieces/fn.progress.html)
([docs/progress](progress.md)). This piece covers the indeterminate "work of unknown extent" spinner.

## Per-toolkit native realization

Platform guidance: [Apple: progress indicators](https://developer.apple.com/design/human-interface-guidelines/progress-indicators); [Windows: progress controls](https://learn.microsoft.com/en-us/windows/apps/design/controls/progress-controls).

| | AppKit | UIKit | GTK | Qt | Android | XAML | ArkUI |
|---|---|---|---|---|---|---|---|
| control | [`NSProgressIndicator`](https://developer.apple.com/documentation/appkit/nsprogressindicator) (Spinning) | [`UIActivityIndicatorView`](https://developer.apple.com/documentation/uikit/uiactivityindicatorview) | [`gtk4::Spinner`](https://docs.gtk.org/gtk4/class.Spinner.html) | busy [`QProgressBar`](https://doc.qt.io/qt-6/qprogressbar.html) (range 0..0) | [`android.widget.ProgressBar`](https://developer.android.com/reference/android/widget/ProgressBar) | [`ProgressRing`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.progressring) | [`ARKUI_NODE_LOADING_PROGRESS`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/capi-native-node-h.md) |
| native code | objc2-app-kit | objc2-ui-kit | gtk4 crate (core mdc) | `src/lib-qt-shim.cpp` | `src/DayActivity.java` | `src/lib-xaml-shim.cpp` | C node API (`src/lib-arkui.rs`) |
| run/stop | `startAnimation:` / `stopAnimation:` | `startAnimating` / `stopAnimating` | `start()` / `stop()` | range 0..0 (busy) ↔ 0..1 (frozen) | [`View.VISIBLE`](https://developer.android.com/reference/android/view/View) ↔ `INVISIBLE` | `IsActive` | `ENABLE_LOADING` + `VISIBLE` ↔ `HIDDEN` |
| [`.large`](https://daybrite.dev/api/day_piece_activity/trait.ActivityBuilder.html#tymethod.large) | `controlSize` Large/Regular | style Large/Medium | `set_size_request` 48/24 | bigger minimum size | `setScaleX/Y(1.5)` | Width/Height 48 | 64 vp instead of 32 |
| stopped state | stays visible (`displayedWhenStopped`) | stays visible (`hidesWhenStopped = false`) | stays visible (drawn static) | frozen empty bar | INVISIBLE (box kept) | `IsActive(false)` | HIDDEN (box kept) |

**Toolkit notes:**

- **AppKit**: [`NSProgressIndicator`](https://developer.apple.com/documentation/appkit/nsprogressindicator) with `style = Spinning` and `indeterminate = true`. [`.large`](https://daybrite.dev/api/day_piece_activity/trait.ActivityBuilder.html#tymethod.large)
  maps to `controlSize` ([`NSControlSize::Large`](https://developer.apple.com/documentation/appkit/nscontrolsize) vs `Regular`; needs objc2-app-kit's [`NSCell`](https://developer.apple.com/documentation/appkit/nscell)
  feature, which carries [`NSControlSize`](https://developer.apple.com/documentation/appkit/nscontrolsize)). `startAnimation:` / `stopAnimation:` are the run/stop
  calls; `setDisplayedWhenStopped(true)` keeps a stopped indicator on screen (a frozen indicator)
  instead of vanishing, matching UIKit.
- **UIKit**: [`UIActivityIndicatorView`](https://developer.apple.com/documentation/uikit/uiactivityindicatorview) with the Large/Medium style. objc2-ui-kit binds the whole
  control, so unlike the media piece's hand-rolled [`AVPlayerViewController`](https://developer.apple.com/documentation/avkit/avplayerviewcontroller), no `extern_class!`
  shim is needed. `hidesWhenStopped = false` keeps a stopped indicator visible.
- **GTK**: [`gtk4::Spinner`](https://docs.gtk.org/gtk4/class.Spinner.html) (a core widget, so the feature compiles everywhere). Its natural size is
  tiny, so the piece gives it a `set_size_request` square (48 for [`.large`](https://daybrite.dev/api/day_piece_activity/trait.ActivityBuilder.html#tymethod.large), else 24). A stopped
  spinner is drawn static.
- **Qt**: Qt ships no native spinner widget, so this crate's C++ shim wraps a [`QProgressBar`](https://doc.qt.io/qt-6/qprogressbar.html)
  in busy mode (`setRange(0, 0)`), the usual Qt way to show indeterminate progress and the same
  technique day-qt uses for [`spinner()`](https://daybrite.dev/api/day_pieces/fn.spinner.html). build.rs compiles the shim against `Qt6Widgets` (already
  linked by day-qt-sys, so it emits no extra link flags). Animating toggles between busy (range
  0..0) and a frozen static bar (range 0..1, value 0). [`.large`](https://daybrite.dev/api/day_piece_activity/trait.ActivityBuilder.html#tymethod.large) gives it a bigger minimum size. (A
  busy [`QProgressBar`](https://doc.qt.io/qt-6/qprogressbar.html) is a horizontal moving-chunk bar rather than a ring; that's what Qt provides
  natively.)
- **Android**: a framework [`android.widget.ProgressBar`](https://developer.android.com/reference/android/widget/ProgressBar), whose default style is a circular
  indeterminate spinner, so the piece needs no Gradle dependency or permission. A default
  indeterminate [`ProgressBar`](https://developer.android.com/reference/android/widget/ProgressBar) always animates while `VISIBLE`; the closest to a stopped-but-present
  spinner is `INVISIBLE` (which keeps the layout box so surrounding layout does not jump). [`.large`](https://daybrite.dev/api/day_piece_activity/trait.ActivityBuilder.html#tymethod.large)
  scales the drawable via `setScaleX/Y`. The Java factory
  (`dev.daybrite.day.piece.activity.DayActivity`) is bundled with the crate in `src/DayActivity.java` and
  folded into the app's Gradle build via `[package.metadata.day.android]`, using only day-android's
  public `DayBridge.ctx`.
- **WinUI** (`windows-winui`): this crate's C++/WinRT shim
  wraps a [`ProgressRing`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.progressring)
  (`Microsoft.UI.Xaml.Controls`), boxed via day-xaml-sys's `day_xaml_box` functions like the media /
  picker / webview XAML pieces. `IsActive` runs/stops it; [`.large`](https://daybrite.dev/api/day_piece_activity/trait.ActivityBuilder.html#tymethod.large) sets Width/Height. WinUI 3's
  ring measures to nothing before it loads, so the WinUI build also sets `MinWidth`/`MinHeight`
  (32, or 48 when large). Its renderer measures the native fitting size independently of the
  parent's proposed width, keeping the ring circular in wide columns. Stopping the animation
  preserves that size. Creation degrades to a [`TextBlock`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.textblock) on any unexpected throw.
- **mock**: the feature exists (so an app can enable `day-piece-activity/mock` uniformly per
  toolkit) but registers no renderer; the activity kind falls back to day's placeholder leaf.
