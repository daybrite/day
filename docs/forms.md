---
title: "Forms"
description: "form, section, and labeled: the grouped-settings look on every platform, from Mac form sheets to Android preference lists."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Forms

A form brings related inputs together so people can provide information or change settings. Clear
labels and grouping make it easier to see what each field asks for and how the answers relate.

[`form`](https://docs.rs/day-pieces/latest/day_pieces/fn.form.html),
[`section`](https://docs.rs/day-pieces/latest/day_pieces/fn.section.html), and
[`labeled`](https://docs.rs/day-pieces/latest/day_pieces/fn.labeled.html) provide a shared layout
for preferences, account details, and other forms, while the controls keep their native behavior.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.

## Authoring

```rust
form((
    section((
        labeled(res::str::volume(), slider(volume).range(0.0..=100.0)),
        labeled(res::str::enabled(), toggle(enabled)),
    ))
    .title(res::str::sound()),
    section((
        labeled(res::str::name(), text_field(name)),
        label(res::str::hint()).font(Font::Footnote), // sections take arbitrary pieces, not just rows
    )),
))
```

- [`form(sections)`](https://docs.rs/day-pieces/latest/day_pieces/fn.form.html) — a vertical run (spacing 16) that provides the shared label column to every
  [`labeled`](https://docs.rs/day-pieces/latest/day_pieces/fn.labeled.html) row underneath it, via the scoped environment ([docs/environment](environment.md)).
- [`section`](https://docs.rs/day-pieces/latest/day_pieces/fn.section.html)`(children)`[`.title`](https://docs.rs/day-pieces/latest/day_pieces/trait.FormSectionBuilder.html#tymethod.title)`(t)` — one grouped card: an optional footnote-style header above a
  rounded card (radius 10, content padded 14, spacing 10). Children are arbitrary pieces;
  [`labeled`](https://docs.rs/day-pieces/latest/day_pieces/fn.labeled.html) rows are just the ones that participate in alignment. A [`section`](https://docs.rs/day-pieces/latest/day_pieces/fn.section.html) also works
  standalone, outside any [`form`](https://docs.rs/day-pieces/latest/day_pieces/fn.form.html).
- [`labeled(text, control)`](https://docs.rs/day-pieces/latest/day_pieces/fn.labeled.html) — one form row: the label sits right-aligned in the form-wide column,
  on the same text baseline as the control ([docs/baseline](baseline.md); centered instead when either side
  has no baseline, e.g. a toggle, or on a toolkit that reports none); the control starts at the
  column edge + 12. A control marked [`.grow(Dimension::Both)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.grow)
  (text fields, sliders, or a [`row(( … ))`](https://docs.rs/day-pieces/latest/day_pieces/fn.row.html) wrapper) stretches to the row's remaining width;
  others hug their natural size. Outside a [`form`](https://docs.rs/day-pieces/latest/day_pieces/fn.form.html), the column is just that row's own label width.
  A control whose natural width does not fit beside the column — a two-button stepper in a
  280-point inspector — goes UNDER the label instead, leading-aligned with the control across
  the whole row (the settings-list idiom), rather than clipping at the row's edge. A
  stretching control shrinks rather than overflows, so give one that needs room a
  [`.min_width(pt)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.min_width): a slider beside a color well then stacks instead of vanishing.
  The label also becomes the control's accessible name (a live text follows its source), unless
  the control names itself with [`.a11y`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.a11y)`(|a| a`[`.label`](https://docs.rs/day-pieces/latest/day_pieces/struct.A11yBuilder.html#method.label)`(..))`, so a form needs no repeated label.

## Disabling a group

[`.enabled(reactive)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.enabled)
on any piece enables or disables everything in it. It takes a constant, a signal or a closure:

```rust
form((
    section((labeled(res::str::name(), text_field(name)), labeled(res::str::email(), text_field(email)))),
    button(res::str::send()).action(send),
))
.enabled(move || !sending.get())
```

While the group is disabled:

- every control in it is disabled the platform's way: dimmed, out of the focus order, and
  reported disabled to assistive technology;
- no tap, double tap, long press, press, drag, pan, pinch, rotate, hover or key reaches a
  handler in it, a link in a label or a Markdown view does not open, and a press under way
  ends;
- an [`InteractionState`](gestures.md#state-styling) in it reads `false` for all three states.

A control is enabled when it and every group around it are, so a control's own
[`.enabled(..)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.enabled) still applies inside an enabled group, and its wish is kept while the group is disabled.
Composed pieces do not dim on their own: read the same signal in an [`.opacity(..)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.opacity) to show the
state on a custom row or card.

Day disables buttons, toggles, sliders, pickers and text fields in a group on every toolkit.
The other native controls, the focus order and the accessibility state of the group are the
toolkit's part.

### Toolkit support

Every toolkit disables the whole group. GTK and Qt use their own cascading enabled state,
ArkUI sets `NODE_ENABLED` on the group, the web makes the group `inert`, WinUI disables each
control and stops hit testing at the group, and AppKit, UIKit and Android walk the group and
disable each control in it, remembering each one's own state for when the group is enabled
again.

## The section-card surface

The card's background is **not** a hard-coded color: the container is realized with
`SurfaceRole::SectionCard` (`day-spec` `ContainerProps.role`), and each toolkit maps it to its
own theme-adaptive grouped-content material, so cards follow light/dark mode and platform
theming with no app code:

| Toolkit | [Material](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") |
|---|---|
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | [`NSColor.quaternarySystemFillColor`](https://developer.apple.com/documentation/appkit/nscolor), resolved per-draw in `drawRect:` (stays live across appearance changes) |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | `tertiarySystemFillColor` + layer corner radius |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | libadwaita's [`.card`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.card) style class |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | translucent neutral fill via a scoped stylesheet + `WA_StyledBackground` (no grouped-card palette role exists; a 12% gray adapts to any palette) |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | `?attr/colorSurfaceContainer` (Material 3; falls back to `colorSurfaceVariant`) in a rounded `GradientDrawable` |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | `CardBackgroundFillColorDefaultBrush` theme resource |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | translucent neutral fill + corner radius |

## Layout: how alignment works

[`form`](https://docs.rs/day-pieces/latest/day_pieces/fn.form.html) plants a shared `Rc<Cell<f64>>` column width in the environment; each `LabeledLayout`
registers its label's unconstrained width during **measure** and reads back the running max in
**place**. A row that stacks registers nothing: its label sits above its control, not in the
column, so a long label on a narrow screen (a French settings row on a phone) leaves the other
rows' controls where they were instead of pushing them off the row. Within one layout pass the enclosing stacks measure every child before placing any,
so by place time the max is final; alignment is consistent with no invalidation pass needed. Rows
report the *proposed* width as their size (labels align form-wide, controls may stretch), so a
growing column width never changes a row's measured size and can't oscillate the pass.

Vertically the row asks both children for their first text baseline ([docs/baseline](baseline.md)) and shifts
each down to the deeper of the two, so the label meets the control's inset text instead of the
middle of its box. The row is then as tall as that shared baseline plus the deepest descent below
it. When either side reports no baseline the row centers both, as before.

## Platform guidance

- [Windows: forms](https://learn.microsoft.com/en-us/windows/apps/design/controls/forms)
