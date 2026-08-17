---
title: "Buttons"
description: "The button piece and its styles on every backend, from plain to prominent to destructive."
---

<!-- Copyright © The Daybrite Project
     SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Buttons

```rust
button("Save").action(save)                 // the platform's ordinary button
button("Save").prominent().action(save)     // its accent / default-action button
button("Delete").tint(RUST).action(delete)  // filled in a color you choose
button("Send").enabled(move || !busy.get()) // the platform's own disabled rendering
```

## The rule: a button is always a native button

`button()` realizes the platform's own button control on every backend, whatever modifiers it
carries. It is never composed into a container with a tap handler.

This is not a style preference. A native button carries a large amount of behavior an app would
have to reimplement, badly, one platform at a time:

- **Focus and keyboard.** Tab order, the focus ring, Space and Enter activation, and on macOS the
  return-key default binding.
- **Accessibility.** The button role, so a screen reader announces it as a button and offers its
  activation action. A `div` or a `UIView` with a tap gesture announces nothing.
- **Pressed and hover rendering.** Every platform's own timing and treatment — Material's ripple,
  UIKit's dimming, AppKit's bezel highlight, the `:active` state on the web.
- **Platform subtleties.** Pointer effects on iPadOS, the Windows focus rectangle, right-to-left
  mirroring, high-contrast and reduced-motion behavior, minimum hit targets.

So when a backend cannot honor a modifier, it **ignores that modifier** and still draws a button.
It never substitutes something that is not one. A plain button on one platform is a much smaller
loss than a colored rectangle that no longer behaves like a button anywhere.

## Styles

| Modifier | What it asks for |
| --- | --- |
| *(none)* | The platform's ordinary button |
| `.bordered()` | A visually contained button where the stock look is borderless (iOS's plain button reads as a link) |
| `.prominent()` | The platform's accent / default-action button |
| `.tint(color)` | A filled button in an app-chosen color |

`.tint()` wins over `.bordered()` and `.prominent()`, being the more specific ask. It takes a
reactive color, so a button can recolour with app state without being rebuilt:

```rust
button("Record").tint(move || if recording.get() { RUST } else { SLATE })
```

The **label color is not yours to set**: `ButtonStyleSpec::on_tint` picks whichever of black or
white contrasts better against the fill, by WCAG's contrast ratio. A pale amber gets dark text and
a saturated navy gets white, with nothing said at the call site.

Comparing the two ratios matters more than it sounds. A mid amber has a relative luminance of
0.44, so a "brighter than half" test calls it dark and puts white on it — 2.2:1, unreadable —
where black would be 9.7:1. The two ratios cross at 0.179, not 0.5.

## Per-toolkit

| toolkit | `.prominent()` | `.tint(c)` |
| --- | --- | --- |
| AppKit | return-key default button | `bezelColor` + an attributed title (see below) |
| UIKit | `borderedProminent` configuration | `filled` configuration + `baseBackgroundColor` |
| GTK | `suggested-action` | a per-color CSS class on the display's provider |
| Qt | `setDefault` (styles vary) | a stylesheet with explicit `:hover`/`:pressed`/`:disabled` |
| Android | the stock M3 filled button | `backgroundTint` on the `MaterialButton` |
| ArkUI | the stock filled capsule | `NODE_BACKGROUND_COLOR` + `NODE_FONT_COLOR` |
| XAML | `AccentButtonStyle` where the resource set has it | `Background` + `Foreground` |
| web-dom | `.day-btn.prominent` | `.day-btn.tinted` with the color in a CSS variable |

Three honest caveats:

**AppKit** colors the label through an ATTRIBUTED title rather than `contentTintColor`. On a
bordered `NSButton`, `contentTintColor` tints template images and AppKit keeps drawing the title
in its own control text color — which rendered white-on-rust as black-on-rust. The consequence
is that `ButtonPatch::Title` has to re-apply the attributed title, so the backend remembers each
button's style to do that.

**Qt** has no native accent button, so `.prominent()` asks the style for the default-button
treatment and otherwise leaves the stock look. Its tint is a stylesheet, which suppresses the
native bevel — the `:hover`, `:pressed` and `:disabled` rules are spelled out to replace what the
bevel would have given.

**XAML** composes a local `Background` over the template's own brushes, and a local value wins
over the theme's PointerOver and Pressed brushes. A tinted button there dims less on press than a
stock one. Fixing that properly needs a full control template, which would replace the control
rather than style it — so it is left as the smaller wrong.
