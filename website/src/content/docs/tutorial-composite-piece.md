---
title: "Tutorial: A composite piece (no native code)"
description: A step-by-step guide to building a reusable widget — a star rating — purely by composing Day's core primitives, with no platform-specific code. Works on every backend for free.
order: 30
---

Most Day widgets you will ever build do not need a single line of platform code. They are
**composite pieces** — new widgets assembled entirely from primitives Day already ships. A composite
piece is pure Rust, has no cargo features, no `build.rs`, and no per-toolkit source files. You add it
to an app as a plain dependency and it runs on **all ten targets** — AppKit, UIKit, Android, GTK, Qt,
WinUI — for free, because every leaf it composes is already a real native control on each one.

In this tutorial you will build one end to end: a **star rating** control — a row of tappable stars
bound to a `Signal<usize>`. By the end you will have a `day-piece-rating` crate you can `.max(5)`,
`.star_size(32.0)`, and drop next to a label, exactly like a built-in.

## 1. What a composite piece is (and why it needs no backend code)

Day has two kinds of pieces:

| | **Native piece** | **Composite piece** |
|---|---|---|
| What it wraps | a *new* native control (`NSComboBox`, `WKWebView`, …) | *existing* Day pieces |
| Per-toolkit code | one renderer per backend (Obj-C, C++, Java…) | none |
| Cargo features | `appkit` / `gtk` / `qt` / `uikit` / `widget` / `winui` | none |
| Extra build assets | `build.rs`, shims, Gradle/SwiftPM entries | none |
| Reference | [the native-piece tutorial](/docs/tutorial-native-piece) · `day-piece-picker` | this tutorial · `day-piece-rating` |

A native piece exists to introduce a native widget Day does not already have. But a star rating is
just a **row of small drawings that react to taps** — and Day already gives you `row`, `canvas` (a
real native 2D surface on every platform), `Shape`, `Signal`, and `.on_tap`. Compose those and there
is nothing left for a backend to do: the AppKit build draws the star with Core Graphics, the Android
build with `android.graphics.Canvas`, the GTK build with Cairo — and *you wrote none of that*.

This is the rule, not the exception. **Most widgets should be composite.** Reach for a native piece
only when you genuinely need a control the toolkits provide and Day does not yet wrap.

The composition toolkit lives in the prelude and is worth knowing before we start:

- `column` / `row` / `zstack` — stack children (vertical, horizontal, layered).
- `.overlay(...)` / `.overlay_aligned(align, ...)` with `Alignment` — draw an annotation on top
  without changing layout size (badges, corner dots).
- `.background(color)` / `.corner_radius(r)` / `.padding(...)` / `.frame(w, h)` — surface + inset.
- `canvas(|d, size| …)` with `Shape` — a native drawing surface for anything custom.
- the `Modifier` trait + `.modifier(m)` — a reusable, by-value view transform (a card, a chip).
- `ButtonStyle` / `FilledButtonStyle` + `Button::style` — a pluggable button appearance.
- `with_environment(...)` / `environment::<T>()` — pass ambient values down a subtree.

Every one of those is pure composition — no per-backend work — and each is what makes the rest of
this tutorial possible.

## 2. Scaffold the crate

**Start with the scaffolder.** `day new piece` generates a ready-to-build crate — `Cargo.toml`,
`.gitignore`, `README.md`, and a sample `src/lib.rs` — so you never assemble the boilerplate by hand:

```bash
day new piece day-piece-rating          # no --toolkits ⇒ a composite piece
```

The generated crate builds immediately (`cargo build`) and depends on a **remote** Day release, so it
works as a standalone repo outside the Day workspace. Pass `--id dev.acme.rating` to set the
reverse-DNS id (defaults to `dev.example.<name>`), or `--local <path-to-day-checkout>` if you are
developing against a local Day clone rather than the published crates. The rest of this tutorial walks
through what the scaffolder emits and how to flesh it out.

A composite piece is an ordinary library crate. It depends on three Day crates and **nothing
platform-specific** — no toolkit crates, no feature table.

```toml
# day-piece-rating/Cargo.toml
[package]
name = "day-piece-rating"
version = "0.1.0"
edition = "2021"

[dependencies]
day-pieces = "0.1"    # the primitives + the prelude (row, canvas, Decorate, …)
day-core = "0.1"      # Piece / BuildCx / RNode / AnyPiece
day-reactive = "0.1"  # Signal (also re-exported through day-pieces' prelude)

# Note what is NOT here: no [features], no dep:day-appkit / day-gtk / day-android,
# no build.rs. That absence IS the composition-first payoff.
```

> In the Day workspace these are `{ workspace = true }` instead of a version. Either way, contrast
> this with a native piece's `Cargo.toml`, which carries a `[features]` block (one feature per
> backend) and often a `build.rs` — see [the native-piece tutorial](/docs/tutorial-native-piece).

Now `src/lib.rs`. Everything comes from the pieces prelude; we additionally name `RNode` from
`day-core` because it is the return type of `Piece::build`:

```rust
use day_core::RNode;
use day_pieces::prelude::*;
```

## 3. Design the builder

Day pieces follow a **config-struct + chainable-setter** pattern (the same shape as `slider(...)`,
`button(...)`, or the `combo_box` piece). A free function creates the piece with sensible defaults;
methods return `Self` so calls chain. The two-way value is a `Signal` passed in by the caller — the
control reads it to draw and writes it back on tap.

```rust
/// A star-rating control bound to `value` (the number of filled stars).
pub struct Rating {
    value: Signal<usize>,
    max: usize,
    star_size: f64,
    editable: bool,
    color: Color,
}

/// Create a rating bound to `value`. Defaults: 5 stars, 28pt, tappable, gold.
pub fn rating(value: Signal<usize>) -> Rating {
    Rating {
        value,
        max: 5,
        star_size: 28.0,
        editable: true,
        color: Color::hex(0xF5A623),
    }
}

impl Rating {
    /// How many stars to show (default 5).
    pub fn max(mut self, n: usize) -> Self {
        self.max = n;
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
}
```

## 4. Compose the body

A piece becomes usable by implementing the `Piece` trait — a single `build` method that returns the
node it created. Once `Rating` implements `Piece`, it *automatically* gains `.id(…)`, `.padding(…)`,
`.frame(…)`, `.any()` and the rest, from the blanket `Decorate` impl. We never touch a backend.

### The star, as a `Shape::Polygon`

A five-pointed star is ten points on two alternating radii — a tip on the outer radius, a valley on
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

### One star = one reactive `canvas`

Each star is a fixed-size `canvas` that draws the polygon: **filled** if its index is within the
current value, **outlined** otherwise. The draw closure reads `value.get()` — a *tracked* read — so
the canvas re-records exactly when the signal changes. When `editable`, an `.on_tap` writes this
star's 1-based position back into the signal.

```rust
fn star(i: usize, value: Signal<usize>, size_pt: f64, editable: bool, color: Color) -> AnyPiece {
    let star = canvas(move |d, size| {
        let radius = size.width.min(size.height) / 2.0 - 1.0; // 1pt margin for the stroke
        let shape = Shape::Polygon(star_points(size, 5, radius, radius * 0.42));
        if i < value.get() {
            d.fill(shape, color); // selected → solid
        } else {
            d.stroke(shape, color, 1.5); // empty → outline
        }
    })
    .frame(size_pt, size_pt);

    if editable {
        star.on_tap(move || value.set(i + 1))
    } else {
        star
    }
}
```

### The body: a `row` of stars

`build` lays the stars in a `row`. Because the count is dynamic, collect them into a `PieceVec`
(the runtime-heterogeneous child sequence) and hand that to `row`:

```rust
impl Piece for Rating {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Rating { value, max, star_size, editable, color } = self;
        let stars: Vec<AnyPiece> =
            (0..max).map(|i| star(i, value, star_size, editable, color)).collect();
        row(PieceVec(stars)).spacing(4.0).build(cx)
    }
}
```

That is the entire piece. Note the reactivity model: the row and its `max` canvases are built **once**.
Tapping the third star calls `value.set(3)`; only the star canvases that read `value` re-record — the
first three fill, the last two outline — with no tree diff and no re-execution of `build`. This is
Day's build-once, bind-forever model, and you got it without writing a binding by hand: `canvas`'s
tracked read wired it for you.

## 5. Use it in an app

The app depends on `day-piece-rating` like any crate. There is **no feature to enable**, no
platform block, no conditional compilation — that is the whole point:

```toml
# the app's Cargo.toml
[dependencies]
day = "0.1"
day-piece-rating = "0.1"   # a plain dependency — nothing else to wire
```

Then use it exactly like a built-in piece, bound to your own `Signal`:

```rust
use day::prelude::*;
use day_piece_rating::rating;

fn review_form() -> AnyPiece {
    let stars = Signal::new(3usize);
    column((
        label("How was it?").font(Font::Title),
        rating(stars).max(5).star_size(32.0),
        // reacts live: reads `stars`, re-renders only this label when a star is tapped
        label(move || format!("{} / 5", stars.get())),
    ))
    .spacing(12.0)
    .padding(20.0)
    .any()
}
```

Run `day launch -p macos-appkit`, then `-p android-widget`, then `-p linux-gtk` — the same rating
renders natively on each, drawn by that platform's own 2D API. You wrote zero platform code.

## 6. Going further

The star rating is one instance of a general move: **build widgets by composing primitives, and let
the native leaves do the platform work.** The same approach covers most of a design system:

- **A card** is a `Modifier` — a reusable transform you apply with `.modifier(m)`:

  ```rust
  pub struct Card { pub tint: Color }
  impl Modifier for Card {
      fn apply(self, content: AnyPiece) -> AnyPiece {
          content.padding(16.0).background(self.tint).corner_radius(12.0)
      }
  }
  // usage: my_content.modifier(Card { tint: Color::hex(0xF2F2F7) })
  ```

  For a one-off you do not even need the type — a plain closure is a `Modifier` via the blanket impl:
  `my_content.modifier(|c: AnyPiece| c.padding(16.0).corner_radius(12.0))`.

- **A badge** is `.overlay_aligned(Alignment::TopTrailing, dot)` — an annotation layered on top of an
  avatar or icon without disturbing its layout size.

- **A chip** is a labelled `.background(...).corner_radius(...)` capsule; a **pill button** is a
  `ButtonStyle` (`FilledButtonStyle` is the shipped example) applied with `Button::style`.

- **Themed subtrees** flow through `with_environment(value, || …)` and are read back with
  `environment::<T>()` — no prop-drilling, no backend code.

Every one of these is composite: pure Rust, no cargo features, works on all ten targets the day you
publish it. Reach for [the native-piece tutorial](/docs/tutorial-native-piece) only when you truly need a
native control Day does not already wrap — the exception that proves how far composition alone will
take you.
