---
title: Styling
description: "Fonts, colors, and appearance in a framework whose widgets are drawn by the platform — including what you can't restyle."
order: 14
section: Concepts
---

Styling in Day starts from an unusual premise: the platform draws the widgets. A Day button on
macOS is an `NSButton` with AppKit's chrome; on Android it's a Material button. That's the point
of the framework — and it means styling has a different shape than in a renderer, where every
pixel is yours to command.

The honest summary: **you style content and space; the platform styles controls.** Fonts, text
color, padding, backgrounds, corner radii, and everything you draw in a `canvas` are yours.
Button chrome, focus rings, slider tracks, scrollbar appearance, selection highlights — those
belong to the platform, and Day deliberately doesn't paper over them.

## Text

Fonts are semantic-first. Instead of hardcoding point sizes, pick a role and let each platform
map it to its own text-style system:

```rust
label(tr("title")).font(Font::Title)
label(tr("caption")).font(Font::Caption).color(Color::rgb(0.5, 0.5, 0.5))
label("Total").font(Font::Body).weight(FontWeight::Semibold)
label("legalese").italic()
```

The semantic roles (`Title`, `Title2`, `Title3`, `Headline`, `Subheadline`, `Body`, `Callout`,
`Footnote`, `Caption`, `Caption2`) resolve to the platform's typography scale, which is what
keeps text looking correct next to native controls. `Font::System(18.0)` is the escape hatch when
you need an exact size, and `Font::Custom("Family", 18.0)` renders a font you bundle in the
project's `fonts/` directory ([resources guide](/docs/resources)).

## Color, backgrounds, shape

```rust
column((avatar, name, bio))
    .padding(16.0)
    .background(Color::hex(0x1E293B))
    .corner_radius(12.0)
```

`Color` is a plain sRGB value (`Color::rgb`, `Color::rgba`, `Color::hex(0xRRGGBB)`, plus `BLACK`,
`WHITE`, `CLEAR`). `.background()` accepts a static color or a reactive one — a closure or signal
— so appearance can follow state:

```rust
label(move || status.get().to_string())
    .background(move || if error.get() { RED_TINT } else { Color::CLEAR })
```

**A limitation to plan around:** Day does not yet ship semantic *color* tokens or automatic
light/dark adaptation for the colors you specify. Native widget chrome follows the system
appearance on its own (an `NSButton` is correct in dark mode without your help), but a hardcoded
`Color::hex(0xFFFFFF)` background is white in both modes. The design reserves a token system
(`theme::TEXT`, `theme::CARD`, … resolving to `UIColor.label`, Adwaita named colors, and so on)
that hasn't landed yet; until it does, apps that want dark-mode-aware custom colors carry their
own palette and switch it themselves. If you can avoid custom colors on large surfaces, do — the
platform's defaults are already right.

## Reusable style: the Modifier trait

There's no stylesheet language. Reuse is ordinary Rust — a function or a `Modifier`, which is
anything that maps a Piece to a decorated Piece:

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
    // Anywhere below: let palette: Palette = cx.use_context().unwrap();
    home_page()
})
```

## Per-platform divergence

Sometimes the right style differs per platform — denser padding on desktop, larger touch targets
on mobile. Today you branch on the compiled toolkit, which is a process constant and costs
nothing at runtime:

```rust
let pad = if cfg!(feature = "uikit") || cfg!(feature = "widget") { 16.0 } else { 10.0 };
content.padding(pad)
```

The design describes a tidier `per_toolkit(12.0).uikit(16.0).qt(8.0)` value type for this; it's
specified but not yet implemented, so `cfg!` branches are the current idiom. Either way the
philosophy is the same: where platforms genuinely diverge, Day gives you a targeted override
rather than pretending the divergence away.

Piece-specific style hooks exist where a control has real variants — `button(...).style(...)`
takes a `ButtonStyle`, `selector(...).style(SelectorStyle::Sidebar)` picks sidebar vs. tab
presentation — and these map to native variants, not custom drawing.

## What you can't restyle (on purpose)

There is no portable API to recolor a slider track, restyle a scrollbar, or reshape a checkbox.
If a property can't be honored by a toolkit, Day logs it once in debug rather than silently
approximating it with custom drawing. That's a real constraint, and it's the flip side of every
Day control behaving — and evolving with the OS — exactly like a native one.

When a *specific platform* offers the knob you want — an AppKit bezel style, WinUI tick marks —
[tweaks](/docs/tweaks) reach the real native widget and set it, per toolkit, without leaving the
native-widget premise. And when you truly need fully custom visuals, that's what
[`canvas`](/docs/internal/shapes) and [composite pieces](/docs/tutorial-composite-piece) are for:
draw your own leaf, keep native behavior around it.

If your product requires a heavily branded design system on every pixel — custom controls
everywhere, identical on all platforms — a renderer-based framework will fight you less. Day is
for apps that want to look like they belong on each platform. That choice is the subject of
[Why Day (and why not)](/docs/benefits).

---

Next: the [Guides](/docs/navigation) cover the everyday tasks — navigation, localization,
accessibility, testing — or jump to the [API tour](/docs/api-tour) for the whole surface at a
glance.
