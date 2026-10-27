---
navTitle: "Progress"
title: "Progress indicators"
description: "progress and spinner: determinate and indeterminate, native on every toolkit."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Progress indicators

A progress indicator tells people that an operation is underway. When the amount of work is known,
it can also show how much has finished and how much remains.

Day provides separate constructors for determinate progress and a busy indicator.

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.

## Authoring

```rust
// Determinate: a bar that fills to a known fraction (0.0..=1.0).
progress(move || downloaded.get() / total.get())

// Indeterminate: an animated spinner / busy bar for work with no known extent.
spinner()
```

Both are plain leaf pieces that work without [`day::task`](https://docs.rs/day-app/latest/day/fn.task.html) or a state machine. A determinate
bar is reactive by construction: pass a `Signal<f64>`, a closure, or a constant and it
tracks the value with the same one-patch-per-change guarantee as [`slider`](https://docs.rs/day-pieces/latest/day_pieces/fn.slider.html).

```rust
let volume = Signal::new(40.0);
column((
    slider(volume).range(0.0..=100.0),
    progress(move || volume.get() / 100.0),   // moves in lockstep with the slider
    spinner(),
))
```

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## API

`crates/day-pieces`:

- [`spinner() -> Progress`](https://docs.rs/day-pieces/latest/day_pieces/fn.spinner.html): indeterminate.
- [`progress(fraction: impl IntoFraction<M>) -> Progress`](https://docs.rs/day-pieces/latest/day_pieces/fn.progress.html): determinate. `IntoFraction`
  mirrors `IntoText`: it is implemented for `f64` (constant), `Signal<f64>`, and any
  `Fn() -> f64`, each under a disjoint marker so the three call forms stay coherent.
- Out-of-range fractions are clamped to `0.0..=1.0` before they ever reach a toolkit,
  so a toolkit never has to defend against `1.7` or `-0.2`.
- A determinate bar takes [`grow_w`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.grow_w) (it fills the available width like a slider); a spinner
  keeps its fixed intrinsic size.

A constant [`progress(0.5)`](https://docs.rs/day-pieces/latest/day_pieces/fn.progress.html) installs no binding. There is nothing to update after build,
so it emits zero runtime patches (asserted in the e2e tests).

## Spinners

Use [`spinner()`](https://docs.rs/day-pieces/latest/day_pieces/fn.spinner.html) when work has no known
completion percentage. It is available through `day::prelude::*` without an additional dependency.

```rust
let running = Signal::new(true);
spinner().animating(running).large()
```

[`animating`](https://docs.rs/day-pieces/latest/day_pieces/trait.ProgressBuilder.html#tymethod.animating)
accepts a Boolean, signal, or closure. It defaults to `true` and changes without rebuilding the
view. Stopping preserves its layout space. Apple and GTK show a stopped indicator; Android and
ArkUI can hide the indicator while retaining its space. Qt uses a busy progress bar and stops
it by selecting a fixed range. The browser pauses its CSS animation.

[`large`](https://docs.rs/day-pieces/latest/day_pieces/trait.ProgressBuilder.html#tymethod.large) selects
the platform's larger indicator. A determinate [`progress(fraction)`](https://docs.rs/day-pieces/latest/day_pieces/fn.progress.html)
uses the same `Progress` type; animation and size options apply to indeterminate indicators.

## Toolkit implementations

Platform guidance: [Apple: progress indicators](https://developer.apple.com/design/human-interface-guidelines/progress-indicators); [Windows: progress controls](https://learn.microsoft.com/en-us/windows/apps/design/controls/progress-controls).

Each toolkit resolves the two variants to its usual native widget:

| Toolkit | Determinate | Indeterminate |
|---------|-------------|---------------|
| AppKit  | [`NSProgressIndicator`](https://developer.apple.com/documentation/appkit/nsprogressindicator) (`.bar`, min 0 / max 1) | [`NSProgressIndicator`](https://developer.apple.com/documentation/appkit/nsprogressindicator) (`.spinning`, animating) |
| UIKit   | [`UIProgressView`](https://developer.apple.com/documentation/uikit/uiprogressview) | [`UIActivityIndicatorView`](https://developer.apple.com/documentation/uikit/uiactivityindicatorview) (animating) |
| GTK 4   | [`GtkProgressBar`](https://docs.gtk.org/gtk4/class.ProgressBar.html) (`set_fraction`) | [`GtkSpinner`](https://docs.gtk.org/gtk4/class.Spinner.html) (`start`) |
| Qt      | [`QProgressBar`](https://doc.qt.io/qt-6/qprogressbar.html) (0..1000) | [`QProgressBar`](https://doc.qt.io/qt-6/qprogressbar.html) in busy mode (`range(0,0)`) |
| Android | `LinearProgressIndicator` (M3, 0..1000) | `LoadingIndicator` (M3 Expressive morphing spinner) |
| WinUI | [`ProgressBar`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.progressbar) (0..1000) | [`ProgressRing`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.progressring) (`IsActive`) |

Notes:

- **Qt has no native spinner widget.** The conventional Qt indeterminate indicator is a busy
  [`QProgressBar`](https://doc.qt.io/qt-6/qprogressbar.html) (`min == max == 0`), so Day uses that rather than emulating a ring. It is a
  horizontal busy bar rather than a circular spinner, the one cross-platform divergence.
- The determinate fraction crosses the C ABI (Qt/Android/WinUI) as an integer tick in
  `0..1000`, the same encoding [`slider`](https://docs.rs/day-pieces/latest/day_pieces/fn.slider.html) uses, so there is no float-ABI concern.

## Sizing

- Determinate bar: fills the proposed width, fixed native height (≈4–20 pt depending on
  toolkit).
- Spinner: a fixed square at its natural size (the engine uses the measured size because
  `grow_w` is false).

## Accessibility

Give a determinate bar a meter role and a label:

```rust
progress(move || volume.get() / 100.0)
    .a11y(|a| a.role(Role::Meter).label(res::str::volume_level()))
```

`Role::Meter` maps to each platform's progress accessibility role.
