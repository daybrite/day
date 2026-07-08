# Progress indicators (`progress`, `spinner`)

A progress indicator reports the state of ongoing work. Day exposes the two SwiftUI
shapes as separate constructors so the call site says which one it means, with no bare
boolean or sentinel value to decode (docs/api-style.md):

```rust
// Determinate: a bar that fills to a known fraction (0.0..=1.0).
progress(move || downloaded.get() / total.get())

// Indeterminate: an animated spinner / busy bar for work with no known extent.
spinner()
```

Both are plain leaf pieces; they need no `day::task` and no state machine. A determinate
bar is reactive by construction: pass a `Signal<f64>`, a closure, or a constant and it
tracks the value with the same one-patch-per-change guarantee as `slider`.

```rust
let volume = Signal::new(40.0);
column((
    slider(volume).range(0.0..=100.0),
    progress(move || volume.get() / 100.0),   // moves in lockstep with the slider
    spinner(),
))
```

## API

`crates/day-pieces`:

- `spinner() -> Progress`: indeterminate.
- `progress(fraction: impl IntoFraction<M>) -> Progress`: determinate. `IntoFraction`
  mirrors `IntoText`: it is implemented for `f64` (constant), `Signal<f64>`, and any
  `Fn() -> f64`, each under a disjoint marker so the three call forms stay coherent.
- Out-of-range fractions are clamped to `0.0..=1.0` before they ever reach a backend,
  so a backend never has to defend against `1.7` or `-0.2`.
- A determinate bar takes `grow_w` (it fills the available width like a slider); a spinner
  keeps its fixed intrinsic size.

A constant `progress(0.5)` installs no binding. There is nothing to update after build,
so it emits zero runtime patches (asserted in the e2e tests).

## The wire (spec)

`day_spec`:

- `kinds::PROGRESS`.
- `props::ProgressProps { value: Option<f64> }`: `None` means indeterminate; `Some(f)` a
  determinate fraction. This single `Option` carries the determinate/indeterminate
  distinction, so there is no separate "style" flag.
- `props::ProgressPatch::Value(Option<f64>)`: the one sparse patch. A determinate bar only
  ever sends `Some`; a spinner sends nothing after realize.

`Toolkit::realize`/`update`/`measure` dispatch on `kinds::PROGRESS` like any other leaf;
no new trait method was needed.

## Native mapping

Each backend resolves the two variants to its usual native widget:

| Backend | Determinate | Indeterminate |
|---------|-------------|---------------|
| AppKit  | `NSProgressIndicator` (`.bar`, min 0 / max 1) | `NSProgressIndicator` (`.spinning`, animating) |
| UIKit   | `UIProgressView` | `UIActivityIndicatorView` (animating) |
| GTK 4   | `GtkProgressBar` (`set_fraction`) | `GtkSpinner` (`start`) |
| Qt      | `QProgressBar` (0..1000) | `QProgressBar` in busy mode (`range(0,0)`) |
| Android | `LinearProgressIndicator` (M3, 0..1000) | `LoadingIndicator` (M3 Expressive morphing spinner) |
| WinUI 3 | `ProgressBar` (0..1000) | `ProgressRing` (`IsActive`) |

Notes:

- **Qt has no native spinner widget.** The conventional Qt indeterminate indicator is a busy
  `QProgressBar` (`min == max == 0`), so Day uses that rather than emulating a ring. It is a
  horizontal busy bar rather than a circular spinner, the one intentional cross-platform
  divergence.
- The determinate fraction crosses the C ABI (Qt/Android/WinUI) as an integer tick in
  `0..1000`, the same encoding `slider` uses, so there is no float-ABI concern.

## Sizing

- Determinate bar: fills the proposed width, fixed native height (≈4–20 pt depending on
  toolkit).
- Spinner: a fixed square at its natural size (the engine uses the measured size because
  `grow_w` is false).

## a11y

Give a determinate bar a meter role and a label:

```rust
progress(move || volume.get() / 100.0)
    .a11y(|a| a.role(Role::Meter).label("Volume level"))
```

`Role::Meter` maps to each platform's progress accessibility role.

## Testing

- **Unit / e2e (`day-mock`).** The op log records `realize day.progress … value=Some(0.25)`
  and `update day.progress … value=Some(0.75)`. The e2e suite
  (`crates/day-pieces/tests/mock_e2e.rs`) asserts: exactly one value patch per reactive
  change, clamping of out-of-range fractions, that a spinner is indeterminate and emits no
  value updates, and that a constant fraction installs no binding.
- **dayscript.** The determinate fraction is captured in the node probe (like `slider`), so
  a walkthrough can assert it:

  ```yaml
  - set_value: { id: volume-slider, value: 80 }
  - assert_value: { id: volume-progress, value: 0.8 }   # 80/100, reactive
  ```

  The showcase `controls` page carries a `volume-progress` determinate bar bound to the
  volume slider plus a `busy-spinner`, verified on all five local targets.
