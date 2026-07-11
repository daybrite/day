# Forms (form / section / labeled)

> **Status: implemented** in `day-pieces` (portable — no per-backend renderer code beyond the
> section-card surface below). A settings-style grouped form: `form` holds `section` cards, and
> `labeled` rows inside them share one right-aligned label column across the whole form, with
> their controls starting on a common left edge — the aligned-labels look every settings UI
> converges on.

## Authoring

```rust
form((
    section((
        labeled(tr("volume"), slider(volume).range(0.0..=100.0)),
        labeled(tr("enabled"), toggle(enabled)),
    ))
    .title(tr("sound")),
    section((
        labeled(tr("name"), text_field(name)),
        label(tr("hint")).font(Font::Footnote),   // sections take arbitrary pieces, not just rows
    )),
))
```

- `form(sections)` — a vertical run (spacing 16) that provides the shared label column to every
  `labeled` row underneath it, via the scoped environment (docs/environment.md).
- `section(children).title(t)` — one grouped card: an optional footnote-style header above a
  rounded card (radius 10, content padded 14, spacing 10). Children are arbitrary pieces;
  `labeled` rows are just the ones that participate in alignment. A `section` also works
  standalone, outside any `form`.
- `labeled(text, control)` — one form row: the label sits right-aligned in the form-wide column,
  vertically centered; the control starts at the column edge + 12. A control marked `.grow()`
  (text fields, sliders, or a `row(( … ))` wrapper) stretches to the row's remaining width;
  others hug their natural size. Outside a `form`, the column is just that row's own label width.

## The section-card surface

The card's background is **not** a hard-coded color: the container is realized with
`SurfaceRole::SectionCard` (`day-spec` `ContainerProps.role`), and each toolkit maps it to its
own theme-adaptive grouped-content material, so cards follow light/dark mode and platform
theming with no app code:

| Toolkit | Material |
|---|---|
| AppKit | `NSColor.quaternarySystemFillColor`, resolved per-draw in `drawRect:` (stays live across appearance changes) |
| UIKit | `tertiarySystemFillColor` + layer corner radius |
| GTK | libadwaita's `.card` style class |
| Qt | translucent neutral fill via a scoped stylesheet + `WA_StyledBackground` (no grouped-card palette role exists; a 12% gray adapts to any palette) |
| Android | `?attr/colorSurfaceContainer` (Material 3; falls back to `colorSurfaceVariant`) in a rounded `GradientDrawable` |
| WinUI | `CardBackgroundFillColorDefaultBrush` theme resource |
| ArkUI | translucent neutral fill + corner radius |

## Layout: how alignment works

`form` plants a shared `Rc<Cell<f64>>` column width in the environment; each `LabeledLayout`
registers its label's unconstrained width during **measure** and reads back the running max in
**place**. Within one layout pass the enclosing stacks measure every child before placing any,
so by place time the max is final — alignment is consistent with no invalidation dances. Rows
report the *proposed* width as their size (labels align form-wide, controls may stretch), so a
growing column width never changes a row's measured size and can't oscillate the pass.

## Verification

- `day-pieces` mock e2e (`form_aligns_labels_and_sections_carry_the_card_surface`): two sections
  carry the card role + radius; label right edges align across sections; control left edges
  align across sections.
- The showcase uses forms on the **Controls**, **Canvas & shapes**, **Device & sensors**, and
  **Platform services** pages; the walkthrough drives every control inside them on all targets,
  and the three screenshot variants (light / dark / fr) verify the card material adapts per
  theme on every toolkit in CI.

## Follow-ups

- `labeled` caption/footer affordances (secondary line under the control, iOS-settings style).
- Optional per-`section` label column (opt out of form-wide alignment for asymmetric sections).
