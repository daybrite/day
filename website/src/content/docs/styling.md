---
title: Styling
description: "Fonts, colors, and appearance in a framework whose widgets are drawn by the platform, including what you can't restyle."
order: 14
section: Explanation
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day supports fonts, colors, spacing, backgrounds, and canvas drawing. Native controls retain
the platform’s appearance, including focus rings, slider tracks, and selection highlights.
The sections below describe which properties an app can set and how each toolkit interprets them.

The Rust blocks are integration fragments; `CARD_BG`, `Palette`, and app functions stand for
application definitions. In [`Modifier::apply`](https://docs.rs/day-pieces/latest/day_pieces/trait.Modifier.html#tymethod.apply), erase the decorated result with [`.any()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.any) to meet
its return type. See [decorators](https://github.com/daybrite/day/blob/main/crates/day-pieces/src/decorators.rs)
for the checked-out contract. Native appearance still needs target-specific visual verification.

The Rust blocks are integration fragments; `CARD_BG`, `Palette`, and app functions stand for
application definitions. In [`Modifier::apply`](https://docs.rs/day-pieces/latest/day_pieces/trait.Modifier.html#tymethod.apply), erase the decorated result with [`.any()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.any) to meet
its return type. See [decorators](https://github.com/daybrite/day/blob/main/crates/day-pieces/src/decorators.rs)
for the checked-out contract. Native appearance still needs target-specific visual verification.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Text

Fonts are chosen by role. Pick a role instead of a point size, and each platform maps it to its
text-style system:

```rust
label(res::str::title()).font(Font::Title)
label(res::str::caption()).font(Font::Caption).color(Color::rgb(0.5, 0.5, 0.5))
label(res::str::total()).font(Font::Body).weight(FontWeight::Semibold)
label(res::str::legalese()).italic()
```

The semantic roles (`LargeTitle`, `Title`, `Title2`, `Title3`, `Headline`, `Subheadline`,
`Body`, `Callout`, `Footnote`, `Caption`, `Caption2`) resolve to the platform's typography
scale, so text matches the platform's native controls. Use `Font::System(18.0)` when
you need an exact size. Use [`Font::custom(res::fonts::family, 18.0)`](https://docs.rs/day-spec/latest/day_spec/enum.Font.html#method.custom) for a font bundled
in the project's `resource/fonts/` directory ([resources guide](/docs/how-to/resources)).

## Color, backgrounds, shape

```rust
column((avatar, name, bio))
    .padding(Space::L)
    .background(Color::hex(0x1E293B))
    .corner_radius(12.0)
```

`Color` is a plain sRGB value ([`Color::rgb`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.rgb), [`Color::rgba`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.rgba), [`Color::hex(0xRRGGBB)`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.hex), plus `BLACK`,
`WHITE`, `CLEAR`). [`.background()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.background) accepts a static color or a [reactive](/docs/reference/glossary#reactive) one (a closure or
[signal](/docs/reference/glossary#signal)), so appearance can follow state:

```rust
label(move || status.get().to_string())
    .background(move || if error.get() { RED_TINT } else { Color::CLEAR })
```

There is no `theme::` token module, because the default appearance is already native: text,
controls, separators, and window grounds take the platform's dynamic colors inside each
[backend](/docs/reference/glossary#backend) (`NSColor.labelColor`, Material surface attributes, QPalette
roles), so dark/light tracking needs no app-side tokens. The semantic roles that must cross the spec
do so as typed values: `SurfaceRole` for grouped-card surfaces, `Font` for typography. Colors *you*
specify are applied as given: a hardcoded [`Color::hex(0xFFFFFF)`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.hex) background is white in both modes,
so an app that wants dark-mode-aware custom colors carries its own palette and switches it itself.
Avoid custom colors on large surfaces where you can, since the platform defaults follow appearance
changes. For screenshots and CI, `DAY_THEME=light|dark` forces the appearance on every backend.

## Reusable style: the Modifier trait

There's no stylesheet language. Reuse is ordinary Rust (a function or a `Modifier`, which is
anything that maps a Piece to a decorated Piece):

```rust
pub struct Card;

impl Modifier for Card {
    fn apply(self, content: AnyPiece) -> AnyPiece {
        content
            .padding(Space::L)
            .background(CARD_BG)
            .corner_radius(12.0)
            .any()
    }
}

column((label(res::str::plan()), label(res::str::pro()))).modifier(Card)
```

Any `FnOnce(AnyPiece) -> AnyPiece` is a `Modifier` too, so one-off wrappers don't need a named
type. For app-wide theming, combine this with environment context:

```rust
with_environment(Palette::dark(), || {
    // Anywhere below: let palette = environment::<Palette>().unwrap();
    home_page()
})
```

## Per-platform divergence

Sometimes the right style differs per platform: denser padding on desktop, larger touch targets
on mobile. Today you branch on the compiled [toolkit](/docs/reference/glossary#toolkit), which is a process constant and costs
nothing at runtime:

```rust
let pad = if cfg!(feature = "uikit") || cfg!(feature = "material") { 16.0 } else { 10.0 };
content.padding(pad)
```

Piece-specific style hooks exist where a control has real variants ([`button`](https://docs.rs/day-pieces/latest/day_pieces/fn.button.html)`(...)`[`.style`](https://docs.rs/day-pieces/latest/day_pieces/trait.NavBuilder.html#tymethod.style)`(...)`
takes a `ButtonStyle`, [`nav`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav.html)`(...)`[`.style`](https://docs.rs/day-pieces/latest/day_pieces/trait.NavBuilder.html#tymethod.style)`(NavStyle::Sidebar)` picks [sidebar](/docs/reference/glossary#sidebar) vs. tab
presentation), and these map to native variants, not custom drawing.

## What you can't restyle

There is no portable API to recolor a slider track, restyle a scrollbar, or reshape a checkbox. If a
property can't be honored by a toolkit, Day logs it once in debug rather than silently approximating
it with custom drawing. Because Day doesn't redraw controls, they update with the OS like any native
control.

When a *specific platform* offers the knob you want (an AppKit bezel style, WinUI tick marks),
[tweaks](/docs/how-to/tweaks) reach the native widget and set it, per toolkit. When you need fully custom
visuals, draw your own leaf with [`canvas`](/docs/reference/shapes) or a [composite
piece](/contributing/tutorials/composite-piece) and keep native behavior around it.

If your product requires a heavily branded design system on every pixel (custom controls
everywhere, identical on all platforms), a renderer-based framework is the better fit. Day is
for apps that want to look like they belong on each platform. That choice is the subject of
[Why Day](/features).

---

Next: the [Guides](/docs/how-to/navigation) cover the everyday tasks (navigation, localization,
accessibility, testing), or see the [API tour](/docs/reference/api-tour) for examples of common UI components
and patterns.

## Platform colors for custom artwork

Use [`system_color(role)`](https://docs.rs/day-app/latest/day/fn.system_color.html) when a shape or
canvas should follow the platform's appearance. `SystemColor` provides `Accent`, `Text`,
`SecondaryText`, `Background`, and `Surface`:

```rust
rounded_rectangle(8.0)
    .height(32.0)
    .fill(|| system_color(SystemColor::Accent))
```

A closure tracks palette changes. Calling [`system_color(...)`](https://docs.rs/day-app/latest/day/fn.system_color.html) once and storing its returned
`Color` captures a snapshot. Native controls keep their existing styling; semantic text and
surface roles remain preferable when drawing a custom shape is unnecessary.

| Toolkit | Color source |
| --- | --- |
| AppKit | Native accent, label, secondary-label, window-background, and control-background colors, resolved in the app's effective appearance. System color notifications refresh the palette. |
| UIKit | System blue, label, secondary-label, system-background, and secondary-system-background colors, resolved for the app's light/dark appearance. |
| Web DOM | The current Day stylesheet's accent, foreground, dim foreground, background, and card-background variables. |
| GTK, Qt, Android, WinUI, ArkUI, mock | Day's fixed light/dark fallback palette; these colors do not report a user-selected system accent. |

Colors describe the app-wide appearance. Per-window overrides, individual control tints, and
UIKit contrast traits are not represented. Web palette reads refresh when Day receives an
appearance change; changing arbitrary CSS alone does not notify Rust. Some surface colors have
transparency and need a background beneath them.
