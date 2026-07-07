//! day-piece-rating — flagship COMPOSE pieces (DESIGN §8, the composition-first tier).
//!
//! Everything here is built PURELY from Day's core primitives ([`row`], [`canvas`], the
//! [`Decorate`] modifiers, [`with_environment`], …). There is **no** per-backend/native code and
//! **no** cargo features: these widgets work on every backend for free — the flagship demonstration
//! that native pieces are the exception, not the rule. Drop the crate in as a plain dependency and
//! call [`rating`], [`badge`], or the [`Card`] modifier from `use day::prelude::*` code.

use day_core::RNode;
use day_pieces::prelude::*;
use day_reactive::Signal;

/// The default filled-star tint: a warm gold/amber.
const GOLD: Color = Color::rgb(1.0, 0.72, 0.0);
/// A subtle translucent card surface (reads as light gray on light, a lift on dark).
const CARD_BG: Color = Color::rgba(0.5, 0.5, 0.5, 0.12);
/// The badge pill fill (iOS system blue).
const BADGE_BLUE: Color = Color::hex(0x0A_84_FF);

// ---------------------------------------------------------------------------
// rating — a tappable star rating, drawn with canvas polygons
// ---------------------------------------------------------------------------

/// A star rating bound to a `Signal<usize>`: `.max` stars, of which `1..=value` are drawn FILLED and
/// the rest OUTLINED. When [`editable`](Rating::editable) (the default), tapping the *i*-th star sets
/// the signal to `i + 1`. It is reactive by construction — each star is a [`canvas`] whose draw
/// closure reads the signal, so changing `value` re-records exactly the stars whose fill flips.
///
/// Pure composition: a [`row`] of canvas polygons, no per-backend code.
///
/// ```ignore
/// let stars = Signal::new(3usize);
/// rating(stars).max(5).star_size(28.0).color(Color::hex(0xFFB800))
/// ```
pub struct Rating {
    value: Signal<usize>,
    max: u32,
    star_size: f64,
    editable: bool,
    color: Color,
    id_prefix: Option<String>,
}

/// Build a [`Rating`] bound two-way to `value` (5 gold, editable, 28-pt stars by default).
pub fn rating(value: Signal<usize>) -> Rating {
    Rating {
        value,
        max: 5,
        star_size: 28.0,
        editable: true,
        color: GOLD,
        id_prefix: None,
    }
}

impl Rating {
    /// How many stars to show (clamped to at least 1; default 5).
    pub fn max(mut self, max: u32) -> Self {
        self.max = max.max(1);
        self
    }
    /// The side length of each square star in points (default 28.0).
    pub fn star_size(mut self, points: f64) -> Self {
        self.star_size = points;
        self
    }
    /// Whether tapping a star updates the bound signal (default `true`). Set `false` for a read-only
    /// display (e.g. an average rating).
    pub fn editable(mut self, editable: bool) -> Self {
        self.editable = editable;
        self
    }
    /// The filled-star color; empty stars are drawn as an outline in the same hue (default gold).
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Assign a dayscript/a11y id scheme so the widget is scriptable. Because a rating is a
    /// COMPOSITE of individually tappable stars (not one node), an inherent `id` sets the row's id
    /// to `prefix` AND each star's id to `prefix:N` (1-based) — so a walkthrough can `tap` a
    /// specific star (`prefix:4` sets the value to 4). Shadows [`Decorate::id`], which would only
    /// tag the row (leaving the stars unaddressable).
    pub fn id(mut self, prefix: impl Into<String>) -> Self {
        self.id_prefix = Some(prefix.into());
        self
    }
}

impl Piece for Rating {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Rating {
            value,
            max,
            star_size,
            editable,
            color,
            id_prefix,
        } = self;
        let stars: Vec<AnyPiece> = (0..max as usize)
            .map(|i| {
                let s = star(i, value, star_size, color, editable);
                match &id_prefix {
                    Some(p) => s.id(format!("{p}:{}", i + 1)),
                    None => s,
                }
            })
            .collect();
        let stars = row(PieceVec(stars)).spacing((star_size * 0.15).max(2.0));
        match id_prefix {
            Some(p) => stars.id(p).build(cx),
            None => stars.build(cx),
        }
    }
}

/// One star: a fixed-size [`canvas`] that fills a 5-point polygon when `index < value`, else strokes
/// it (an empty outline). Editable stars carry an `on_tap` that sets the signal to `index + 1`.
fn star(index: usize, value: Signal<usize>, size: f64, color: Color, editable: bool) -> AnyPiece {
    let star = canvas(move |d, sz| {
        let pts = star_points(sz);
        // Stars 1..=value are filled; `index` is 0-based, so filled when `index < value`.
        if index < value.get() {
            d.fill(Shape::Polygon(pts), color);
        } else {
            let width = (sz.width.min(sz.height) * 0.08).max(1.5);
            d.stroke(Shape::Polygon(pts), color, width);
        }
    })
    .frame(size, size);
    if editable {
        star.on_tap(move || value.set(index + 1))
    } else {
        star
    }
}

/// The 10 vertices of a 5-point star inscribed in `size`, centered, pointing up. Even indices sit on
/// the outer radius, odd indices on the inner radius (the classic star notch ratio).
fn star_points(size: Size) -> Vec<Point> {
    use std::f64::consts::PI;
    let cx = size.width / 2.0;
    let cy = size.height / 2.0;
    let outer = (size.width.min(size.height) / 2.0) * 0.92;
    let inner = outer * 0.4;
    (0..10)
        .map(|k| {
            let radius = if k % 2 == 0 { outer } else { inner };
            // Start at the top point (-90°) and step 36° per vertex, clockwise.
            let angle = -PI / 2.0 + (k as f64) * (PI / 5.0);
            Point::new(cx + radius * angle.cos(), cy + radius * angle.sin())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Card — a reusable surface, expressed as a Modifier
// ---------------------------------------------------------------------------

/// A reusable card surface, applied via [`Decorate::modifier`]: pads its content, paints a subtle
/// translucent background, and rounds the corners. Pure composition over
/// [`padding`](Decorate::padding) / [`background`](Decorate::background) /
/// [`corner_radius`](Decorate::corner_radius).
///
/// ```ignore
/// column((label("Title"), label("Body"))).modifier(Card)
/// ```
pub struct Card;

impl Modifier for Card {
    fn apply(self, content: AnyPiece) -> AnyPiece {
        content
            .padding(16.0)
            .background(CARD_BG)
            .corner_radius(12.0)
    }
}

// ---------------------------------------------------------------------------
// badge — a numbered pill overlaid on a piece's top-trailing corner
// ---------------------------------------------------------------------------

/// Overlay a small numbered blue pill on the TOP-TRAILING corner of `over` (a notification badge),
/// via [`Decorate::overlay_aligned`]. Returns `over` unchanged when `count <= 0`, so a zero count
/// shows no badge. Pure composition — the pill is a padded, rounded, colored [`label`].
///
/// ```ignore
/// badge(3, icon.any())  // the icon with a "3" pill in its corner
/// ```
pub fn badge(count: i64, over: AnyPiece) -> AnyPiece {
    if count <= 0 {
        return over;
    }
    let pill = label(format!("{count}"))
        .font(Font::Caption2)
        .color(Color::WHITE)
        .padding(Insets::symmetric(6.0, 2.0))
        .background(BADGE_BLUE)
        .corner_radius(10.0);
    over.overlay_aligned(Alignment::TopTrailing, pill)
}
