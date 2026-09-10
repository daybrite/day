---
title: Styling
description: "Fonts, colors, and appearance in a framework whose widgets are drawn by the platform, including what you can't restyle."
order: 14
section: Concepts
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

In Day the platform draws the widgets: a Day button on macOS is an `NSButton` with AppKit's
chrome; on Android it's a Material button. So styling works differently than in a renderer,
where you control every pixel.

You style content and spacing; the platform styles its controls. Fonts, text color, padding,
backgrounds, corner radii, and everything you draw in a `canvas` are yours. Button chrome, focus
rings, slider tracks, scrollbar appearance, selection highlights: those belong to the platform, and
Day leaves them to it.

## Text

Fonts are chosen by role. Pick a role instead of a point size, and each platform maps it to its
own text-style system:

```rust
label(tr("title")).font(Font::Title)
label(tr("caption")).font(Font::Caption).color(Color::rgb(0.5, 0.5, 0.5))
label("Total").font(Font::Body).weight(FontWeight::Semibold)
label("legalese").italic()
```

The semantic roles (`LargeTitle`, `Title`, `Title2`, `Title3`, `Headline`, `Subheadline`,
`Body`, `Callout`, `Footnote`, `Caption`, `Caption2`) resolve to the platform's typography
scale, so text matches the platform's native controls. `Font::System(18.0)` is the escape hatch when
you need an exact size, and `Font::custom(res::fonts::family, 18.0)` renders a font you bundle
in the project's `resource/fonts/` directory ([resources guide](/docs/resources)).

## Color, backgrounds, shape

```rust
column((avatar, name, bio))
    .padding(16.0)
    .background(Color::hex(0x1E293B))
    .corner_radius(12.0)
```

`Color` is a plain sRGB value (`Color::rgb`, `Color::rgba`, `Color::hex(0xRRGGBB)`, plus `BLACK`,
`WHITE`, `CLEAR`). `.background()` accepts a static color or a [reactive](/docs/glossary#reactive) one (a closure or
[signal](/docs/glossary#signal)), so appearance can follow state:

```rust
label(move || status.get().to_string())
    .background(move || if error.get() { RED_TINT } else { Color::CLEAR })
```

There is no `theme::` token module, because the default appearance is already native: text,
controls, separators, and window grounds take the platform's own dynamic colors inside each
[backend](/docs/glossary#backend) (`NSColor.labelColor`, Material surface attributes, QPalette
roles), so dark/light tracking needs no app-side tokens. The semantic roles that must cross the spec
do so as typed values: `SurfaceRole` for grouped-card surfaces, `Font` for typography. Colors *you*
specify are applied as given: a hardcoded `Color::hex(0xFFFFFF)` background is white in both modes,
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
        content.padding(16.0).background(CARD_BG).corner_radius(12.0)
    }
}

column((label("Plan"), label("Pro"))).modifier(Card)
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
on mobile. Today you branch on the compiled [toolkit](/docs/glossary#toolkit), which is a process constant and costs
nothing at runtime:

```rust
let pad = if cfg!(feature = "uikit") || cfg!(feature = "mdc") { 16.0 } else { 10.0 };
content.padding(pad)
```

Piece-specific style hooks exist where a control has real variants (`button(...).style(...)`
takes a `ButtonStyle`, `nav(...).style(NavStyle::Sidebar)` picks [sidebar](/docs/glossary#sidebar) vs. tab
presentation), and these map to native variants, not custom drawing.

## What you can't restyle

There is no portable API to recolor a slider track, restyle a scrollbar, or reshape a checkbox. If a
property can't be honored by a toolkit, Day logs it once in debug rather than silently approximating
it with custom drawing. Because Day doesn't redraw controls, they update with the OS like any native
control.

When a *specific platform* offers the knob you want (an AppKit bezel style, XAML tick marks),
[tweaks](/docs/tweaks) reach the native widget and set it, per toolkit. When you need fully custom
visuals, draw your own leaf with [`canvas`](/docs/internal/shapes) or a [composite
piece](/docs/tutorial-composite-piece) and keep native behavior around it.

If your product requires a heavily branded design system on every pixel (custom controls
everywhere, identical on all platforms), a renderer-based framework is the better fit. Day is
for apps that want to look like they belong on each platform. That choice is the subject of
[Why Day](/docs/benefits).

---

Next: the [Guides](/docs/navigation) cover the everyday tasks (navigation, localization,
accessibility, testing), or jump to the [API tour](/docs/api-tour) for the whole surface at a
glance.
