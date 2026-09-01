---
title: "Color picker"
description: "A color well in two idioms — the platform's own chooser, and one Day draws itself out of pieces and a canvas — bound two-way to a Signal<Color>."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Color picker (external piece)

> **Status: implemented** as `day-piece-colorpicker`. Six toolkits get a native renderer registered
> link-time into their slice; the other two get the same picker as everyone else, because the
> **composed** idiom is built from ordinary Day pieces and a canvas and therefore needs no arm at
> all. Verified on macos-appkit, macos-gtk, macos-qt, ios-uikit, android-mdc and web-dom; CI builds
> every arm. The XAML arm is written against the Windows SDK and has not been run on Windows yet;
> see [Verification status](#verification-status).

`color_picker(Signal<Color>)` is a **color well**: a swatch showing the current color that opens a
chooser when pressed. Both idioms give the app that same control and differ only in who draws the
chooser behind it.

## Authoring

```rust
use day_piece_colorpicker::{PickerIdiom, color_picker};

let tint = Signal::new(Color::hex(0xE86A3C));

row((
    vector(gv::star).tint(move || tint.get()).frame(28.0, 28.0),

    color_picker(tint),                          // Automatic (default)
    color_picker(tint).alpha(true),              // + an opacity channel
    color_picker(tint).composed(),               // Day's own panel, on every target
    color_picker(tint).native(),                 // the platform's, literally
    color_picker(tint)
        .title(tr("pick_a_tint"))                // the chooser's heading
        .presets(vec![BRAND, ACCENT, INK])       // the composed panel's swatch row
        .key("brand-color"),                     // the well's dayscript id, both idioms
))
```

The bound signal is **two-way**: a user's pick writes it, and the app writing it repaints the well.
The value is Day's ordinary [`Color`](color.md), so the same signal drives `.tint(…)`,
`.background(…)`, a canvas fill or a gradient stop with no conversion.

## The two idioms

[`PickerIdiom`] has three values; the third is the default.

- **`Native`** — realizes the piece's leaf, which six toolkits render with their own chooser. This
  is *literal*: on a toolkit with no renderer it draws Day's visible `⟨day.piece.colorpicker⟩`
  placeholder, exactly like any other unrendered kind. Pin it only behind a `support()` check.
- **`Composed`** — builds the picker out of ordinary Day pieces: a drawn swatch opening a
  [`cover`](cover.md) that holds a canvas saturation/brightness field, a hue strip, an opacity
  strip and a preset palette. Every part of it is Rust that already runs everywhere, so it is the
  same picker on all nine targets, including the two that have no other option.
- **`Automatic`** (default) — `Native` where the toolkit has a chooser, `Composed` where it does
  not.

`day_piece_colorpicker::support()` reports which one `Automatic` will pick. It does not report
whether the picker works (it works on every target), so an app showing a "not supported here"
banner from this answer would be wrong. Use it to say *which* picker the user gets.

## Per-toolkit realization

| Target | Tier | Native chooser | Notes |
|---|---|---|---|
| macos-appkit | **Native** | `NSColorWell` → the shared `NSColorPanel` | wheel, sliders, palettes, image spectrum, crayons, screen eyedropper |
| ios-uikit | **Native** | `UIColorWell` → `UIColorPickerViewController` | grid / spectrum / sliders, eyedropper, iPad popover anchoring — all from the well |
| gtk | **Native** | `GtkColorDialogButton` → `GtkColorDialog` | GTK 4.10+, which day-gtk already requires |
| qt | **Native** | swatch `QPushButton` → `QColorDialog` | Qt has no color-well widget; the shim paints the swatch |
| windows-xaml | **Native** | swatch `Button` → `ColorPicker` in a `Flyout` | `Windows.UI.Xaml.Controls.ColorPicker`, Windows 10 1703+ |
| web-dom | **Native** | `<input type="color">` | the browser's own picker, which on desktop IS the system chooser |
| android-mdc | Composed | — | Android ships no color picker in the framework, in Material, or in AndroidX |
| harmony-arkui | Composed | — | the ArkUI C node API has no picker node, and ArkTS has no `ColorPicker` component |
| mock | Native leaf | — | records the leaf's realize/patch traffic; `tests/mock.rs` drives both idioms |

Neither android-mdc nor harmony-arkui grew a hand-written chooser in Java or ArkTS. Writing one
twice, in two languages, would have produced two dialogs that matched neither the platform nor each
other, for more work than one panel written once in Rust that every other target can also opt
into.

## The composed panel

```
┌ Pick a tint ─────────────────┐
│ ┌──────────────────────────┐ │   saturation × brightness for the current hue:
│ │            ◯             │ │   the pure hue, washed to white across and black
│ │                          │ │   down — three fills, two of them gradients, so
│ └──────────────────────────┘ │   it stays crisp at any size rather than being a bitmap
│ ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓ │   hue strip
│ ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒ │   opacity strip (only with `.alpha(true)`), over a checkerboard
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
- **The well is drawn, not a tinted button.** A `button(hex).tint(color)` would be native and
  carry press feedback and keyboard focus for free, but each toolkit applies a tint its own way:
  AppKit composites it through the bezel, Material draws a filled container with its own elevation,
  GTK and Qt route it through their themes, and the web takes CSS. The same color would read
  differently on all nine. A canvas draws what the app asked for. That costs keyboard activation
  (a drawn well takes a press, not a Return key), and the well carries a `Button` role and a
  label so a screen reader still announces it correctly.

The panel presents in an `unrouted()` [`cover`](cover.md), which keeps a mounted picker out of the
app's route space: `navigate("settings")` still goes to settings rather than opening a color
panel keyed `"settings"` (see that method for why the untyped route makes this a real hazard).
Android's system back still dismisses the panel; that path is the cover's own `NavBack` handler
rather than the route adapter.

## Alpha

`.alpha(true)` offers an opacity channel and lets the bound signal carry a non-opaque value. The
composed panel honors it everywhere. Among the native choosers there is one gap:
`<input type="color">` gained an `alpha` attribute only recently and browsers are still shipping
it, so the web arm sets the attribute and stays opaque where the browser has not implemented it.
There is no reliable feature test that predicts the UI (a browser can accept the property and
still draw an opaque-only picker), so the arm sets it and reads whatever value comes back; an
opaque pick arrives with `a = 1`.

With `.alpha(false)` (the default) the piece drops the alpha off any pick, so a chooser with a
stray alpha channel cannot make an app's brand color half-transparent behind its back.

## Precision

A pick crosses back to Rust as the component form `Color`'s `Display` writes and
[`Color::parse`](color.md) reads, four floats rather than a packed 32-bit ARGB, so the toolkits
whose pickers are float-precision (`NSColor`, `GdkRGBA`, `QColor::getRgbF`) lose nothing on the
way. Three arms are 8-bit at the source and quantize regardless: XAML's `Windows.UI.Color`,
`<input type="color">`'s `#rrggbb` value, and anything that round-trips through
`android.graphics.Color`.

Everything the *chooser* knew that `Color` cannot hold (a wide-gamut pick, which authoring model
the user was in, a dynamic system color) is covered in [color.md](color.md), which also proposes
what to do about it.

## Scripting

The native leaf accepts `Event::TextChanged` carrying any form `Color::parse` reads, so dayscript's
`input:` step drives it on every backend:

```yaml
- input: { id: brand-well, text: "#2f6fde" }
- assert_text: { id: brand-value, text: "#2f6fde" }
```

The composed panel is ordinary pieces, so it is driven by ordinary steps. The well's id is its
route key; the panel's own parts carry fixed ids:

```yaml
- tap: { id: brand-color }              # `.key("brand-color")` — the well
- assert_visible: { id: color-picker-panel }
- assert_text: { id: color-picker-value, text: "#e86a3c" }
- tap: { id: color-picker-cancel }      # or color-picker-done
```

`color-picker-shade`, `color-picker-hue`, `color-picker-opacity` and `color-picker-presets` name
the drawn controls. They take a `tap` at a point, which is how the walkthrough proves
`Decorate::on_tap_at` reports *where* a press landed (see [canvas.md](canvas.md)).

## What this piece does not promise

- **Identical chrome across the native idiom.** AppKit's panel is a floating inspector, iOS's a
  sheet, GTK's a dialog, XAML's a flyout. That difference is the platform's own chrome
  (DESIGN.md [§2](../DESIGN.md)); `.composed()` is the escape hatch when an app wants one look.
- **An embedded/inline style.** Three of the eight toolkits have nothing to embed, and a style that
  silently degrades on most backends is worse than one an app can reason about. The composed panel
  covers the case an inline picker was wanted for.
- **Palettes, recent colors, or an eyedropper of Day's own.** The native choosers each bring their
  own; the composed panel offers `.presets(…)` and nothing more.
- **Anything `Color` cannot carry**, covered in [color.md](color.md).

## Verification status

The showcase's 572-step walkthrough passes on **macos-appkit, macos-gtk, macos-qt, ios-uikit,
android-mdc and web-dom** with the Resources page driving both idioms. The composed panel was
screenshot-reviewed on the first five (it draws the same field, strip, preview, palette and
buttons on each), and on web-dom its steps pass but no screenshot was taken (that needs a
`DAY_WEB_DRIVER` browser, [docs/web.md](web.md)). `assert_no_placeholders` checks the native
leaf's realize traffic on every one of those targets, so the six native arms are known to
register and realize.

The mock e2e (`pieces/day-piece-colorpicker/tests/mock.rs`) covers the leaf's event decode, the
app-write patch, the alpha guard, and the composed panel's mount / pick / cancel / done cycle on
every host.

**harmony-arkui** has not been exercised (it gets the composed panel, and its Rust is the same
code the other eight run, but the OHOS emulator has not run it), and neither has the *interaction*
with each native chooser, which is an OS panel a script cannot drive.

**windows-xaml** was written blind and has since been driven by hand on Windows 11, in Day-Sketch's
inspector, with synthetic mouse input (nothing below is in CI, because opening the flyout is the
part no script reaches). Three of the four assumptions hold: `Button.Flyout` does carry a
`ColorPicker`, `ColorChanged` does fire while the flyout is open and the pick reaches the app, and
`day_xaml_measure` sizes the swatch button sanely. `IsAlphaEnabled` is still unconfirmed — the app
the check ran in keeps its wells opaque and carries opacity on a slider of its own.

The same check found a fifth thing the shim had assumed and should not have: `ColorPicker::Color`
raises `ColorChanged`, so day writing a value in (an inspector following a new selection) came
straight back as if the user had picked it. The app then rewrote the value it had just sent, and
that no-op write still sealed an undo unit, which swallowed the user's next Undo (Day-Sketch's
walkthrough failed there). The shim now brackets the write with an echo guard, the way the GTK arm
always has; Qt never reports anything but an explicit dialog pick, so it never had the hazard.
