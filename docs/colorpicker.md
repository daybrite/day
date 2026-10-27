---
navTitle: "Color pickers"
title: "Color picker"
description: "A color well in two idioms — the platform's chooser, and one Day draws itself out of pieces and a canvas — bound two-way to a Signal<Color>."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Color picker

Let people choose a color for a drawing, highlight, or theme. `day-pieces` binds the
selection to app state and presents a native color control where available, with a picker built
from Day pieces on the other targets.

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.
The selected toolkit supplies its native implementation; [`day::colorpicker::support()`](https://daybrite.dev/api/day_pieces/colorpicker/fn.support.html) reports
its availability after the Day runtime starts.

| Platforms / toolkits | Availability |
| --- | --- |
| AppKit, UIKit, GTK, Qt, WinUI, Web | Native chooser; Windows alpha selection remains unverified. |
| Android, ArkUI | Composed picker; HarmonyOS device behavior remains unverified. |

[`color_picker(Signal<Color>)`](https://daybrite.dev/api/day_pieces/colorpicker/fn.color_picker.html) is a **color well**: a swatch showing the current color that opens a
chooser when pressed. Both idioms give the app that same control and differ only in who draws the
chooser behind it.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Authoring

```rust
use day::colorpicker::{PickerIdiom, color_picker};

let tint = Signal::new(Color::hex(0xE86A3C));

row((
    vector(gv::star).tint(move || tint.get()).frame(28.0, 28.0),
    color_picker(tint),            // Automatic (default)
    color_picker(tint).alpha(),    // + an opacity channel
    color_picker(tint).composed(), // Day's own panel, on every target
    color_picker(tint).native(),   // the platform's, literally
    color_picker(tint)
        .title(res::str::pick_a_tint()) // the chooser's heading
        .presets(vec![BRAND, ACCENT, INK]) // the composed panel's swatch row
        .id(id!(brand_color)), // the well's dayscript id, both idioms
))
```

The bound signal is **two-way**: a user's pick writes it, and the app writing it repaints the well.
The value is Day's ordinary [`Color`](color.md), so the same signal drives [`.tint(…)`](https://daybrite.dev/api/day_pieces/trait.ButtonBuilder.html#tymethod.tint),
[`.background(…)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.background), a canvas fill or a gradient stop with no conversion.

## Picker styles

[`PickerIdiom`] has three values; the third is the default.

- **`Native`** — uses the toolkit's chooser. On a toolkit without a native chooser, it draws Day's visible `⟨day.piece.colorpicker⟩`
  placeholder, exactly like any other unrendered kind. Pin it only behind a [`support()`](https://daybrite.dev/api/day_pieces/colorpicker/fn.support.html) check.
- **`Composed`** — builds the picker out of ordinary Day pieces: a drawn swatch opening a
  [`cover`](cover.md) that holds a canvas saturation/brightness field, a hue strip, an opacity
  strip and a preset palette. Every part of it is Rust that already runs everywhere, so it is the
  same picker on all targets, including those without a native chooser.
- **`Automatic`** (default) — `Native` where the toolkit has a chooser, `Composed` where it does
  not.

[`day_pieces::colorpicker::support()`](https://daybrite.dev/api/day_pieces/colorpicker/fn.support.html) reports which one `Automatic` will pick. It does not report
whether the picker works (it works on every target), so an app showing a "not supported here"
banner from this answer would be wrong. Use it to say *which* picker the user gets.

## Toolkit implementations

Platform guidance: [Apple: color](https://developer.apple.com/design/human-interface-guidelines/color); [Windows: color picker](https://learn.microsoft.com/en-us/windows/apps/design/controls/color-picker).

| Target | Tier | Native chooser | Notes |
|---|---|---|---|
| macos-appkit | **Native** | [`NSColorWell`](https://developer.apple.com/documentation/appkit/nscolorwell) → the shared [`NSColorPanel`](https://developer.apple.com/documentation/appkit/nscolorpanel) | wheel, sliders, palettes, image spectrum, crayons, screen eyedropper |
| ios-uikit | **Native** | [`UIColorWell`](https://developer.apple.com/documentation/uikit/uicolorwell) → [`UIColorPickerViewController`](https://developer.apple.com/documentation/uikit/uicolorpickerviewcontroller) | grid / spectrum / sliders, eyedropper, iPad popover anchoring — all from the well |
| gtk | **Native** | [`GtkColorDialogButton`](https://docs.gtk.org/gtk4/class.ColorDialogButton.html) → [`GtkColorDialog`](https://docs.gtk.org/gtk4/class.ColorDialog.html) | GTK 4.10+, which day-toolkit-gtk already requires |
| qt | **Native** | swatch [`QPushButton`](https://doc.qt.io/qt-6/qpushbutton.html) → [`QColorDialog`](https://doc.qt.io/qt-6/qcolordialog.html) | Qt has no color-well widget; the shim paints the swatch |
| windows-winui | **Native** | swatch [`Button`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.button) → [`ColorPicker`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.colorpicker) in a `Flyout` | `Microsoft.UI.Xaml.Controls.ColorPicker` |
| web-dom | **Native** | `<input type="color">` | the browser's own picker, which on desktop IS the system chooser |
| android-material | Composed | — | Android ships no color picker in the framework, in Material, or in AndroidX |
| harmony-arkui | Composed | — | the ArkUI C node API has no picker node, and ArkTS has no `ColorPicker` component |
| mock | Native leaf | — | records the leaf's realize/patch traffic; `tests/mock.rs` drives both idioms |

Neither android-material nor harmony-arkui grew a hand-written chooser in Java or ArkTS. Writing one
twice, in two languages, would have produced two dialogs that matched neither the platform nor each
other, for more work than one panel written once in Rust that every other target can also opt
into.

## The composed panel

```text
┌ Pick a tint ─────────────────┐
│ ┌──────────────────────────┐ │   saturation × brightness for the current hue:
│ │            ◯             │ │   the pure hue, washed to white across and black
│ │                          │ │   down — three fills, two of them gradients, so
│ └──────────────────────────┘ │   it stays crisp at any size rather than being a bitmap
│ ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓ │   hue strip
│ ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒ │   opacity strip (only with `.alpha()`), over a checkerboard
│ ▉ #e86a3c                    │   preview + hex
│ ■ ■ ■ ■ ■ ■ ■ ■              │   presets, in a wrapping row
│ ■ ■ ■ ■ ■ ■ ■                │
│ Cancel               Done    │
└──────────────────────────────┘
```

Three details of the panel call for explanation:

- **The panel keeps HSV as its own state**, separate from the bound color. Deriving hue from RGB
  on every change would lose it the moment brightness reached zero (black has no hue), so the
  sliders would jump back to red as the user dragged into the corner. The panel seeds H/S/V once
  per presentation and writes the color out.
- **The bound signal moves live**, as it does under every native chooser: the app sees the tint
  change while the user drags. `Cancel` puts back the color the panel opened on; `Done` keeps the
  pick.
- **The well is drawn, not a tinted button.** A [`button`](https://daybrite.dev/api/day_pieces/fn.button.html)`(hex)`[`.tint`](https://daybrite.dev/api/day_pieces/trait.ButtonBuilder.html#tymethod.tint)`(color)` would be native and
  carry press feedback and keyboard focus for free, but each toolkit applies a tint its own way:
  AppKit composites it through the bezel, Material draws a filled container with its own elevation,
  GTK and Qt route it through their themes, and the web takes CSS. The same color would read
  differently on all nine. A canvas draws what the app asked for. That costs keyboard activation
  (a drawn well takes a press, not a Return key), and the well carries a `Button` role and a
  label so a screen reader still announces it correctly.

The panel presents in an [`unrouted()`](https://daybrite.dev/api/day_pieces/trait.CoverBuilder.html#tymethod.unrouted) [`cover`](cover.md), which keeps a mounted picker out of the
app's route space: [`navigate("settings")`](https://daybrite.dev/api/day_pieces/fn.navigate.html) still goes to settings rather than opening a color
panel keyed `"settings"` (see that method for why the untyped route makes this a real hazard).
Android's system back still dismisses the panel; that path is the cover's own `NavBack` handler
rather than the route adapter.

## Alpha

[`.alpha()`](https://daybrite.dev/api/day_pieces/colorpicker/trait.ColorPickerBuilder.html#tymethod.alpha) offers an opacity channel and lets the bound signal carry a non-opaque value. The
composed panel honors it everywhere. Among the native choosers there is one gap:
`<input type="color">` gained an `alpha` attribute only recently and browsers are still shipping
it, so the web arm sets the attribute and stays opaque where the browser has not implemented it.
There is no reliable feature test that predicts the UI (a browser can accept the property and
still draw an opaque-only picker), so the arm sets it and reads whatever value comes back; an
opaque pick arrives with `a = 1`.

Without [`.alpha()`](https://daybrite.dev/api/day_pieces/colorpicker/trait.ColorPickerBuilder.html#tymethod.alpha) (the default) the piece drops the alpha off any pick, so a chooser with a
stray alpha channel cannot make an app's brand color half-transparent behind its back.

## Precision

A pick crosses back to Rust as the component form `Color`'s `Display` writes and
[`Color::parse`](https://daybrite.dev/api/day_geometry/struct.Color.html#method.parse) reads, four floats rather than a packed 32-bit ARGB, so the toolkits
whose pickers are float-precision ([`NSColor`](https://developer.apple.com/documentation/appkit/nscolor), [`GdkRGBA`](https://docs.gtk.org/gdk4/struct.RGBA.html), [`QColor::getRgbF`](https://doc.qt.io/qt-6/qcolor.html)) lose nothing on the
way. These native representations use 8-bit channels: WinUI's [`Windows.UI.Color`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.color),
`<input type="color">`'s `#rrggbb` value, and anything that round-trips through
[`android.graphics.Color`](https://developer.android.com/reference/android/graphics/Color).

Everything the *chooser* knew that `Color` cannot hold (a wide-gamut pick, which authoring model
the user was in, a dynamic system color) is described in [Colors](color.md).

## Limits

- **Identical chrome across the native idiom.** AppKit's panel is a floating inspector, iOS's a
  sheet, GTK's a dialog, WinUI's a flyout. That difference is the platform's chrome; [`.composed()`](https://daybrite.dev/api/day_pieces/colorpicker/trait.ColorPickerBuilder.html#tymethod.composed) is the escape hatch when an app wants one look.
- **Inline pickers.** The portable API presents a chooser; it does not embed an inline editor.
  Use the composed panel for consistent presentation across platforms.
- **Palettes, recent colors, or an eyedropper of Day's own.** The native choosers each bring their
  own; the composed panel offers [`.presets(…)`](https://daybrite.dev/api/day_pieces/colorpicker/trait.ColorPickerBuilder.html#tymethod.presets) and nothing more.
- **Anything `Color` cannot carry**, covered in [color](color.md).

> [!WARNING]
> **Windows / WinUI: alpha selection.**
> Native alpha selection has not been validated. Test transparent color selection before
> relying on [`.alpha()`](https://daybrite.dev/api/day_pieces/colorpicker/trait.ColorPickerBuilder.html#tymethod.alpha) in a Windows app.

> [!WARNING]
> **HarmonyOS / ArkUI: composed picker.**
> The composed picker has not been exercised on a device. Validate selection, cancellation,
> and the final color in your app before shipping.
