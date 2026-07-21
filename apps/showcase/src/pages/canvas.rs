use day::prelude::*;
use day_piece_rating::{Card, badge, rating};

use crate::widgets::{gauge, page};

/// Drawing & composition (docs/shapes.md, docs/canvas.md, DESIGN §8/§11): the unified `shape`
/// piece in every kind, live canvas transforms and gestures, the slider-driven gauge, and the
/// composition-tier widgets (rating, card, badge, button styles, ambient environment) — each
/// group in its own themed section.
pub(crate) fn canvas_page() -> AnyPiece {
    page(
        crate::res::str::nav_canvas(),
        "canvas-title",
        Some(crate::res::str::canvas_caption()),
        form((
            shapes_section(),
            gradients_section(),
            transform_section(),
            gauge_section(),
            compose_section(),
        ))
        .any(),
    )
}

/// Rotate a gradient unit point about the box centre (0.5, 0.5) — the shared angle applied to
/// every swatch's base geometry.
fn spin(p: UnitPoint, deg: f64) -> UnitPoint {
    let (s, c) = deg.to_radians().sin_cos();
    let (dx, dy) = (p.x - 0.5, p.y - 0.5);
    UnitPoint::new(0.5 + dx * c - dy * s, 0.5 + dx * s + dy * c)
}

/// Linear + radial gradients (docs/shapes.md §7): `.fill_linear`/`.fill_radial` on shape pieces.
/// ONE angle slider drives the whole group — each swatch's closure re-records with its base
/// geometry rotated by the shared signal (linear lines spin about the unit-box centre; radial
/// centres orbit it, so the centred glow is — correctly — the only invariant).
fn gradients_section() -> impl Piece {
    let angle = Signal::new(0.0f64);
    // Base geometry + stops per swatch, spun by the shared angle at record time.
    let linear = move |start: UnitPoint, end: UnitPoint, stops: Vec<(f64, Color)>| {
        move || {
            LinearGradient::new(
                spin(start, angle.get()),
                spin(end, angle.get()),
                stops.clone(),
            )
        }
    };
    let radial = move |center: UnitPoint, radius: f64, stops: Vec<(f64, Color)>| {
        move || RadialGradient::new(spin(center, angle.get()), radius, stops.clone())
    };
    section((
        row((
            rectangle()
                .fill_linear(linear(
                    UnitPoint::TOP,
                    UnitPoint::BOTTOM,
                    vec![(0.0, Color::hex(0x2E6FB8)), (1.0, Color::hex(0x7FB2E5))],
                ))
                .frame(56.0, 56.0)
                .id("gradient-vertical"),
            rounded_rectangle(12.0)
                .fill_linear(linear(
                    UnitPoint::LEADING,
                    UnitPoint::TRAILING,
                    vec![(0.0, Color::hex(0x8E44AD)), (1.0, Color::hex(0xE67E22))],
                ))
                .frame(76.0, 56.0)
                .id("gradient-horizontal"),
            circle()
                .fill_linear(linear(
                    UnitPoint::TOP_LEADING,
                    UnitPoint::BOTTOM_TRAILING,
                    vec![
                        (0.0, Color::hex(0xE74C3C)),
                        (0.5, Color::hex(0xF1C40F)),
                        (1.0, Color::hex(0x27AE60)),
                    ],
                ))
                .frame(56.0, 56.0)
                .id("gradient-stops"),
            rounded_rectangle(12.0)
                .fill_linear(linear(
                    UnitPoint::LEADING,
                    UnitPoint::TRAILING,
                    vec![(0.0, Color::hex(0x16A085)), (1.0, Color::hex(0x2C3E50))],
                ))
                .frame(76.0, 56.0)
                .id("gradient-angle"),
        ))
        .spacing(12.0),
        // Radial: centered glow, off-center highlight, and a multi-stop "sunset" in a
        // non-square frame (the unit-space radius stretches elliptically to the bounds).
        row((
            circle()
                .fill_radial(radial(
                    UnitPoint::CENTER,
                    0.5,
                    vec![(0.0, Color::hex(0xFFF2B0)), (1.0, Color::hex(0xE67E22))],
                ))
                .frame(56.0, 56.0)
                .id("gradient-radial"),
            circle()
                .fill_radial(radial(
                    UnitPoint::new(0.35, 0.35),
                    0.75,
                    vec![(0.0, Color::hex(0xBBDEFB)), (1.0, Color::hex(0x1D5FA8))],
                ))
                .frame(56.0, 56.0)
                .id("gradient-radial-offset"),
            rounded_rectangle(12.0)
                .fill_radial(radial(
                    UnitPoint::BOTTOM,
                    1.0,
                    vec![
                        (0.0, Color::hex(0xFFD24A)),
                        (0.5, Color::hex(0xE74C3C)),
                        (1.0, Color::hex(0x2C3E50)),
                    ],
                ))
                .frame(76.0, 56.0)
                .id("gradient-radial-stops"),
        ))
        .spacing(12.0),
        labeled(
            crate::res::str::gradient_angle(),
            slider(angle).range(0.0..=360.0).id("gradient-angle-slider"),
        ),
    ))
    .title(crate::res::str::gradients_title())
}

fn shapes_section() -> impl Piece {
    section((
        row((
            rectangle()
                .fill(Color::hex(0x2F6FDE))
                .frame(56.0, 56.0)
                .id("shape-rect"),
            rounded_rectangle(12.0)
                .fill(Color::hex(0x8E44AD))
                .frame(56.0, 56.0)
                .id("shape-rrect"),
            circle()
                .fill(Color::hex(0x27AE60))
                .frame(56.0, 56.0)
                .id("shape-circle"),
            capsule()
                .fill(Color::hex(0xE67E22))
                .frame(76.0, 40.0)
                .id("shape-capsule"),
        ))
        .spacing(12.0),
        row((
            ellipse()
                .stroke(Color::hex(0xC0392B), 4.0)
                .frame(76.0, 48.0)
                .id("shape-ellipse"),
            arc(135.0, 270.0)
                .stroke(Color::hex(0x16A085), 6.0)
                .frame(56.0, 56.0)
                .id("shape-arc"),
            // Line + polygon resolve unit points against the frame (docs/shapes.md §3.1).
            line((0.1, 0.85), (0.9, 0.15))
                .stroke(Color::hex(0x2C3E50), 4.0)
                .frame(56.0, 56.0)
                .id("shape-line"),
            polygon([
                (0.5, 0.03),
                (0.61, 0.38),
                (0.98, 0.38),
                (0.68, 0.6),
                (0.79, 0.95),
                (0.5, 0.73),
                (0.21, 0.95),
                (0.32, 0.6),
                (0.02, 0.38),
                (0.39, 0.38),
            ])
            .fill(Color::hex(0xF1C40F))
            .frame(56.0, 56.0)
            .id("shape-polygon"),
            // A shape_group flattens its shapes into ONE canvas leaf (docs/shapes.md §3.6):
            // a target — ring, disc, and four `.at`-placed tick lines — as a single native view.
            shape_group([
                circle().stroke(Color::hex(0xC0392B), 4.0).inset(4.0),
                circle()
                    .fill(Color::hex(0xC0392B))
                    .at(0.38, 0.38, 0.24, 0.24),
                line((0.5, 0.0), (0.5, 0.14)).stroke(Color::hex(0xC0392B), 3.0),
                line((0.5, 0.86), (0.5, 1.0)).stroke(Color::hex(0xC0392B), 3.0),
                line((0.0, 0.5), (0.14, 0.5)).stroke(Color::hex(0xC0392B), 3.0),
                line((0.86, 0.5), (1.0, 0.5)).stroke(Color::hex(0xC0392B), 3.0),
            ])
            .frame(56.0, 56.0)
            .id("shape-group"),
        ))
        .spacing(12.0),
    ))
    .title(crate::res::str::shapes_kinds())
}

fn transform_section() -> impl Piece {
    let angle = Signal::new(0.0f64);
    let tapped = Signal::new(false);
    let pos = Signal::new((0.0f64, 0.0f64));
    let base = Signal::new((0.0f64, 0.0f64));
    section((
        labeled(
            crate::res::str::shapes_angle(),
            slider(angle).range(0.0..=360.0).id("shapes-angle-slider"),
        ),
        row((
            // A rounded rectangle rotated live by a slider (canvas CTM transform), inset so the
            // rotated square's corners stay within the canvas frame (backends that clip children
            // to bounds — e.g. Qt — would otherwise shave the corners at an angle).
            rounded_rectangle(10.0)
                .fill(Color::hex(0x2F6FDE))
                .rotate(move || angle.get())
                .inset(20.0)
                .frame(120.0, 120.0)
                .id("shapes-rotator"),
            // Tap to recolor (path-precise hit-testing). `.id` before `.frame` so the identifier
            // lands on the shape leaf (the gesture target), not the frame wrapper.
            circle()
                .fill(move || {
                    if tapped.get() {
                        Color::hex(0xE74C3C)
                    } else {
                        Color::hex(0x3498DB)
                    }
                })
                .on_tap(move || tapped.update(|t| *t = !*t))
                .id("shapes-tap-circle")
                .frame(90.0, 90.0),
            // Drag to move (offset bound to the drag translation).
            rectangle()
                .fill(Color::hex(0x9B59B6))
                .offset(move || pos.get().0, move || pos.get().1)
                .on_drag(move |dr| match dr.phase {
                    DragPhase::Began => base.set(pos.get_untracked()),
                    _ => {
                        let b = base.get_untracked();
                        pos.set((b.0 + dr.translation.x, b.1 + dr.translation.y));
                    }
                })
                .id("shapes-drag-rect")
                .frame(90.0, 90.0),
        ))
        .spacing(20.0),
        label(crate::res::str::shapes_interact_hint()).font(Font::Footnote),
    ))
    .title(crate::res::str::shapes_transform())
}

fn gauge_section() -> impl Piece {
    let level = Signal::new(40.0f64);
    section((
        labeled(
            crate::res::str::volume_label(),
            slider(level).range(0.0..=100.0).id("gauge-slider"),
        ),
        gauge(level),
    ))
    .title(crate::res::str::canvas_gauge())
}

fn compose_section() -> impl Piece {
    // A shared rating signal, driven by tapping stars. Its count is mirrored into a text field:
    // `bind` pushes each newly-tapped value into `rating_text`, so tapping a star updates the field.
    let stars = Signal::new(3usize);
    let rating_text = Signal::new(stars.get().to_string());
    bind(
        move || stars.get(),
        move |n: &usize| rating_text.set(n.to_string()),
    );
    // A custom ambient value flowed via `with_environment` and read back by a descendant.
    #[derive(Clone, Copy)]
    struct Accent(Color);
    let accent = Color::hex(0x30_B0_60);

    section((
        label(crate::res::str::compose_caption()).font(Font::Footnote),
        // 1) Interactive star rating (canvas-polygon compose piece): tap a star, and the text
        //    field beside it updates with the count (the `bind` above drives it).
        labeled(
            crate::res::str::compose_rating_label(),
            rating(stars).id("compose-rating"),
        ),
        labeled(
            crate::res::str::compose_rating_count(),
            text_field(rating_text)
                .placeholder(crate::res::str::compose_rating_placeholder())
                .id("compose-rating-value"),
        ),
        // 2) Card modifier — a reusable surface wrapping arbitrary content — plus the badge
        //    overlay (a numbered pill on an icon's top-trailing corner).
        row((
            column((
                label(crate::res::str::compose_card_title()).font(Font::Headline),
                label(crate::res::str::compose_card_body()),
            ))
            .spacing(4.0)
            .align(HAlign::Leading)
            .modifier(Card),
            badge(
                3,
                rounded_rectangle(10.0)
                    .fill(Color::hex(0x8E_8E_93))
                    .frame(48.0, 48.0),
            ),
        ))
        .spacing(20.0),
        // 3) ButtonStyle — a FilledButtonStyle button next to a plain one for contrast.
        row((
            button(crate::res::str::compose_plain_btn()).id("compose-plain-btn"),
            button(crate::res::str::compose_styled_btn())
                .style(FilledButtonStyle {
                    color: Color::hex(0x0A_84_FF),
                })
                .id("compose-styled-btn"),
        ))
        .spacing(12.0),
        // 4) Ambient environment flow — a descendant tints itself from the provided Accent.
        with_environment(Accent(accent), || {
            let tint = environment::<Accent>().map(|a| a.0).unwrap_or(Color::BLACK);
            label(crate::res::str::compose_env_value())
                .font(Font::Headline)
                .color(tint)
                .id("compose-env-value")
        }),
    ))
    .title(crate::res::str::nav_compose())
}
