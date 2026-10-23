---
navTitle: "Composite piece"
title: "Tutorial: A composite piece (no native code)"
description: A step-by-step guide to building a reusable widget (a star rating) by composing Day's core primitives, with no platform-specific code. It works on every backend.
order: 41
section: Contributing
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

A composite piece combines existing Day pieces into a reusable control. It is a Rust library
crate and needs no additional native toolkit implementation. The `example-star-rating`
package in this tutorial is a user-created example, not a shipped Day crate. This tutorial builds a star-rating control bound to
a `Signal<usize>`, with configurable star count and size.

## 1. What a composite piece is (and why it needs no backend code)

Day has two kinds of pieces:

| | **Native piece** | **Composite piece** |
|---|---|---|
| What it wraps | a *new* native control (`NSComboBox`, `WKWebView`, …) | *existing* Day pieces |
| Per-toolkit code | one renderer per backend (Obj-C, C++, Java…) | none |
| Cargo features | `appkit` / `gtk` / `qt` / `uikit` / `mdc` / `xaml` / `winui` | none |
| Extra build assets | `build.rs`, shims, Gradle/SwiftPM entries | none |
| Reference | [the native-piece tutorial](/contributing/tutorials/native-piece) · [`day-piece-texteditor`](/docs/reference/texteditor) | this tutorial · `example-star-rating` |

A native piece exists to introduce a native widget Day does not already have. A star rating is a
row of small drawings that react to taps, and Day already gives you [`row`](https://daybrite.dev/api/day_pieces/fn.row.html), [`canvas`](https://daybrite.dev/api/day_pieces/fn.canvas.html) (a native 2D
surface on every platform), `Shape`, `Signal`, and [`.on_tap`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_tap). Compose those and the existing leaves
do the platform work: the AppKit build draws the star with Core Graphics, the Android build with
`android.graphics.Canvas`, and the GTK build with Cairo, all from the one [`canvas`](https://daybrite.dev/api/day_pieces/fn.canvas.html) closure.

Most widgets should be composite. Reach for a native piece only when you need a control the
toolkits provide and Day does not yet wrap.

The composition toolkit lives in the prelude:

- [`column`](https://daybrite.dev/api/day_pieces/fn.column.html) / [`row`](https://daybrite.dev/api/day_pieces/fn.row.html) / [`zstack`](https://daybrite.dev/api/day_pieces/fn.zstack.html): stack children (vertical, horizontal, layered).
- [`.overlay(...)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.overlay) / [`.overlay_aligned(align, ...)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.overlay_aligned) with `Alignment`: draw an annotation on top
  without changing layout size (badges, corner dots).
- [`.background(color)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.background) / [`.corner_radius(r)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.corner_radius) / [`.padding(...)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.padding) / [`.frame(w, h)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame): surface + inset.
- [`canvas(|d, size| …)`](https://daybrite.dev/api/day_pieces/fn.canvas.html) with `Shape`: a native drawing surface for anything custom.
- the `Modifier` trait + [`.modifier(m)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.modifier): a reusable, by-value view transform (a card, a chip).
- [`.plain()`](https://daybrite.dev/api/day_pieces/trait.ButtonBuilder.html#tymethod.plain),
  [`.bordered()`](https://daybrite.dev/api/day_pieces/trait.ButtonBuilder.html#tymethod.bordered), and
  [`.prominent()`](https://daybrite.dev/api/day_pieces/trait.ButtonBuilder.html#tymethod.prominent): native button appearances.
- [`with_environment(...)`](https://daybrite.dev/api/day_pieces/fn.with_environment.html) / [`environment::<T>()`](https://daybrite.dev/api/day_pieces/fn.environment.html): pass ambient values down a subtree.

## 2. Scaffold the crate

Start with the scaffolder. `day new piece` generates a ready-to-build crate (`Cargo.toml`,
`.gitignore`, `README.md`, and a sample `src/lib.rs`) and a `demo/` app that shows it:

```bash
day new piece example-star-rating          # no --toolkits ⇒ a composite piece
```

The generated crate builds immediately (`cargo build`) and depends on a remote Day release, so it
works as a standalone repo outside the Day workspace. Pass `--id dev.acme.rating` to set the
reverse-DNS id (defaults to `dev.example.<name>`), or `--local <path-to-day-checkout>` if you are
developing against a local Day clone rather than the published crates. `--composite` forces a
composite piece even when `--toolkits` is present, and `--no-input` skips the interactive prompts
(scripts and CI). From `demo/`, `day launch -t <target> --script dayscript/demo.yaml` runs a
one-page app that shows the piece on any target, and `--no-demo` skips it. The rest of this
tutorial walks through what the scaffolder emits and how to flesh it out.

A composite piece is an ordinary library crate. It depends on three Day crates and nothing
platform-specific.

```toml
# example-star-rating/Cargo.toml
[package]
name = "example-star-rating"
version = "0.1.0"
edition = "2024"

[dependencies]
# The framework crates come from git until they're on crates.io (`day new piece` writes this).
day-pieces = { git = "https://github.com/daybrite/day.git" }   # primitives + prelude (row, canvas, Decorate, …)
day-core = { git = "https://github.com/daybrite/day.git" }     # Piece / BuildCx / RNode / AnyPiece
day-reactive = { git = "https://github.com/daybrite/day.git" } # Signal (also re-exported through day-pieces' prelude)
day-fluent = { git = "https://github.com/daybrite/day.git" }

[build-dependencies]
day-build = { git = "https://github.com/daybrite/day.git" }

# Note what is not here: no [features], no dep:day-appkit / day-gtk / day-android,
# no build.rs. A composed piece needs no per-toolkit code at all.

# The scaffolder appends this so the crate is its own cargo workspace and builds standalone.
[workspace]
```

> [!NOTE] Inside the Day workspace
> These are `{ workspace = true }` instead of a version. A native piece's `Cargo.toml` also
> carries a `[features]` block (one feature per backend) and often a `build.rs`; see
> [the native-piece tutorial](/contributing/tutorials/native-piece).

Now `src/lib.rs`. Everything comes from the pieces prelude; `RNode` also comes from `day-core`,
because it is the return type of [`Piece::build`](https://daybrite.dev/api/day_core/trait.Piece.html#tymethod.build):

```rust
use day_core::RNode;
use day_pieces::prelude::*;
```

## 3. Design the builder

Day pieces follow a **config-struct + chainable-setter** pattern (the same shape as [`slider(...)`](https://daybrite.dev/api/day_pieces/fn.slider.html),
[`button(...)`](https://daybrite.dev/api/day_pieces/fn.button.html), or the `combo_box` piece). A free function creates the piece with defaults; methods
return `Self` so calls chain. The two-way value is a `Signal` passed in by the caller: the control
reads it to draw and writes it back on tap. This compact implementation uses `Signal<usize>`;
the shipped piece accepts any `Binding<usize>` and reactive editability.

Give the piece a [private catalog](/docs/how-to/localization#give-a-crate-a-private-catalog).
In `build.rs`, call [`day_build::generate_locales().expect("localization codegen");`](https://daybrite.dev/api/day_build/fn.generate_locales.html) from
`main`, and add `day_fluent::locales!();` at the root of `src/lib.rs`. In
`resource/locales/en-US/app.ftl`:

```ftl
rating_star = Rate { $stars } out of { $maximum } stars
```

```rust
/// The default filled-star tint: a warm gold/amber.
const GOLD: Color = Color::rgb(1.0, 0.72, 0.0);

/// A star-rating control bound to `value` (the number of filled stars).
pub struct Rating {
    value: Signal<usize>,
    max: u32,
    star_size: f64,
    editable: bool,
    color: Color,
    star_id: Option<std::rc::Rc<dyn Fn(usize) -> day_core::ElementId>>,
}

/// Create a rating bound to `value`. Defaults: 5 stars, 28pt, tappable, gold.
pub fn rating(value: Signal<usize>) -> Rating {
    Rating {
        value,
        max: 5,
        star_size: 28.0,
        editable: true,
        color: GOLD,
        star_id: None,
    }
}

impl Rating {
    /// How many stars to show (clamped to at least 1; default 5).
    pub fn max(mut self, n: u32) -> Self {
        self.max = n.max(1);
        self
    }
    /// Edge length of each star, in points (default 28).
    pub fn star_size(mut self, pt: f64) -> Self {
        self.star_size = pt;
        self
    }
    /// Whether taps change the value (default `true`; pass `false` for a read-only display).
    pub fn editable(mut self, yes: bool) -> Self {
        self.editable = yes;
        self
    }
    /// The star tint (default gold).
    pub fn color(mut self, c: Color) -> Self {
        self.color = c;
        self
    }
    pub fn star_id(mut self, id: impl Fn(usize) -> day_core::ElementId + 'static) -> Self {
        self.star_id = Some(std::rc::Rc::new(id));
        self
    }
}
```

The example's `.star_id(|n| id!(rating_star, n))` method names each star. It assigns each
tappable star a typed ID using its 1-based position; [`.id(id!(rating_row))`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id) separately names
the whole piece. A dayscript can then `tap: { id: 'rating_star:3' }`.
The old [`.id(prefix)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id) string-prefix setter is no longer supported. See
[element IDs](/contributing/backend-contracts/element-ids) for keyed families and canonical spellings.

## 4. Compose the body

A piece becomes usable by implementing the `Piece` trait: a single [`build`](https://daybrite.dev/api/day_core/trait.Piece.html#tymethod.build) method that returns the
node it created. Once `Rating` implements `Piece`, it automatically gains [`.id(…)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id), [`.padding(…)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.padding),
[`.frame(…)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame), [`.any()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.any) and the rest, from the blanket `Decorate` impl.

### The star, as a `Shape::Polygon`

A five-pointed star is ten points on two alternating radii: a tip on the outer radius, a valley on
the inner one. This helper computes them, centered in whatever size layout hands the canvas:

```rust
/// Vertices of an `points`-pointed star centered in `size`, first tip pointing up.
fn star_points(size: Size, points: usize, outer: f64, inner: f64) -> Vec<Point> {
    let cx = size.width / 2.0;
    let cy = size.height / 2.0;
    let step = std::f64::consts::PI / points as f64; // half-sector: tip → valley
    let mut angle = -std::f64::consts::FRAC_PI_2;     // start at the top
    let mut out = Vec::with_capacity(points * 2);
    for i in 0..points * 2 {
        let r = if i % 2 == 0 { outer } else { inner };
        out.push(Point::new(cx + r * angle.cos(), cy + r * angle.sin()));
        angle += step;
    }
    out
}
```

### One star = one reactive [`canvas`](https://daybrite.dev/api/day_pieces/fn.canvas.html)

Each star is a fixed-size [`canvas`](https://daybrite.dev/api/day_pieces/fn.canvas.html) that draws the polygon: filled if its index is within the
current value, outlined otherwise. The draw closure reads [`value.get()`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.get), a tracked read, so
the canvas re-records exactly when the signal changes. When `editable`, an [`.on_tap`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_tap) writes this
star's 1-based position back into the signal.

```rust
fn star(
    i: usize,
    value: Signal<usize>,
    maximum: u32,
    size_pt: f64,
    editable: bool,
    color: Color,
    id: Option<day_core::ElementId>,
) -> AnyPiece {
    let star = canvas(move |d, size| {
        let radius = size.width.min(size.height) / 2.0 - 1.0; // 1pt margin for the stroke
        let shape = Shape::Polygon(star_points(size, 5, radius, radius * 0.42));
        if i < value.get() {
            d.fill(shape, color); // selected → solid
        } else {
            d.stroke(shape, color, 1.5); // empty → outline
        }
    });
    let star = Decorated::new(star).a11y(move |a| {
        a.role(if editable { Role::Button } else { Role::Image })
            .label(res::str::rating_star(res::str::RatingStar {
                stars: (i + 1) as i64,
                maximum: i64::from(maximum),
            }))
            .selected(move || i < value.get())
    });
    let star = match id {
        Some(id) => star.id(id),
        None => star,
    };

    if editable {
        star.on_tap(move || value.set(i + 1))
            .focusable()
            .on_key(move |key| match key.key.as_str() {
                "ArrowLeft" => value.update(|n| *n = n.saturating_sub(1)),
                "ArrowRight" => value.update(|n| *n = (*n + 1).min(maximum as usize)),
                _ => {}
            })
            .frame(size_pt, size_pt)
            .any()
    } else {
        star.frame(size_pt, size_pt).any()
    }
}
```

### The body: a [`row`](https://daybrite.dev/api/day_pieces/fn.row.html) of stars

[`build`](https://daybrite.dev/api/day_core/trait.Piece.html#tymethod.build) lays the stars in a [`row`](https://daybrite.dev/api/day_pieces/fn.row.html). Because the count is dynamic, collect them into a `Vec` (a
`Vec` of pieces is a child sequence like a tuple) and hand that to [`row`](https://daybrite.dev/api/day_pieces/fn.row.html):

```rust
impl Piece for Rating {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Rating {
            value,
            max,
            star_size,
            editable,
            color,
            star_id,
        } = self;
        let stars: Vec<_> = (0..max as usize)
            .map(|i| {
                let id = star_id.as_ref().map(|make_id| make_id(i + 1));
                star(i, value, max, star_size, editable, color, id)
            })
            .collect();
        row(stars).spacing(Space::XS).build(cx)
    }
}
```

That completes the piece. The row and its `max` canvases are built once. Tapping the third star
calls [`value.set(3)`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.set); only the star canvases that read `value` re-record (the first three fill, the
last two outline), and [`build`](https://daybrite.dev/api/day_core/trait.Piece.html#tymethod.build) never runs again. This is Day's build-once reactive model; the
tracked read in the [`canvas`](https://daybrite.dev/api/day_pieces/fn.canvas.html) closure is the only binding.

## 5. Use it in an app

The app depends on `example-star-rating` like any crate, with one line in `Cargo.toml`:

```toml
# the app's Cargo.toml (the framework crates come from git until they're on crates.io —
# `day new app` writes this form for you)
[dependencies]
day = { git = "https://github.com/daybrite/day.git" }
example-star-rating = { path = "../example-star-rating" }   # a plain dependency, nothing else to wire
```

Then use it like a built-in piece, bound to your own `Signal`:

```rust
use day::prelude::*;
use example_star_rating::rating;

fn review_form() -> impl Piece {
    let stars = Signal::new(3usize);
    column((
        label(res::str::how_was_it()).font(Font::Title),
        rating(stars)
            .max(5)
            .star_size(32.0)
            .star_id(|n| id!(rating_star, n))
            .id(id!(rating_row)),
        // reacts live: reads `stars`, re-renders only this label when a star is tapped
        label(move || res::str::rating_value(stars.get() as i64).format()),
    ))
    .spacing(Space::M)
    .padding(20.0)
}
```

Run `day launch -t macos-appkit`, then `-t android-mdc`, then `-t linux-gtk`. The same rating
renders natively on each, drawn by that platform's 2D API from the one [`canvas`](https://daybrite.dev/api/day_pieces/fn.canvas.html) closure.

## 6. Going further

The same approach covers most of a design system:

- **A card** is a `Modifier`, a reusable transform you apply with [`.modifier(m)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.modifier):

  ```rust
  pub struct Card { pub tint: Color }
  impl Modifier for Card {
      fn apply(self, content: AnyPiece) -> AnyPiece {
          content.padding(16.0).background(self.tint).corner_radius(12.0)
      }
  }
  // usage: my_content.modifier(Card { tint: Color::hex(0xF2F2F7) })
  ```

  For a one-off you do not even need the type; a plain closure is a `Modifier` via the blanket impl:
  `my_content`[`.modifier`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.modifier)`(|c: AnyPiece| c`[`.padding`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.padding)`(16.0)`[`.corner_radius`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.corner_radius)`(12.0))`.

- **A badge** is [`.overlay_aligned(Alignment::TopTrailing, dot)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.overlay_aligned), an annotation layered on top of an
  avatar or icon without disturbing its layout size. Keep this helper in the app until other
  consumers need it.

- **A chip** is a labeled [`.background`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.background)`(...)`[`.corner_radius`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.corner_radius)`(...)` capsule. For a **native button**, choose
  [`.plain()`](https://daybrite.dev/api/day_pieces/trait.ButtonBuilder.html#tymethod.plain),
  [`.bordered()`](https://daybrite.dev/api/day_pieces/trait.ButtonBuilder.html#tymethod.bordered), or
  [`.prominent()`](https://daybrite.dev/api/day_pieces/trait.ButtonBuilder.html#tymethod.prominent).

- **Themed subtrees** flow through [`with_environment(value, || …)`](https://daybrite.dev/api/day_pieces/fn.with_environment.html) and are read back with
  [`environment::<T>()`](https://daybrite.dev/api/day_pieces/fn.environment.html), so a value set once reaches every descendant without being passed through
  each builder.

