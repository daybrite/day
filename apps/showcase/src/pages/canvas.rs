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

/// Linear gradients (docs/shapes.md §7): `.fill_linear` on shape pieces — static, multi-stop,
/// and one whose geometry is driven live by the angle slider.
fn gradients_section() -> impl Piece {
    let angle = Signal::new(0.0f64);
    section((
        row((
            rectangle()
                .fill_linear(LinearGradient::vertical(
                    Color::hex(0x2E6FB8),
                    Color::hex(0x7FB2E5),
                ))
                .frame(56.0, 56.0)
                .id("gradient-vertical"),
            rounded_rectangle(12.0)
                .fill_linear(LinearGradient::horizontal(
                    Color::hex(0x8E44AD),
                    Color::hex(0xE67E22),
                ))
                .frame(76.0, 56.0)
                .id("gradient-horizontal"),
            circle()
                .fill_linear(LinearGradient::new(
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
            // The gradient line is reactive like any other shape property: the slider swings
            // its end point around the shape's unit box.
            rounded_rectangle(12.0)
                .fill_linear(move || {
                    let t = angle.get().to_radians();
                    let (dx, dy) = (t.cos() * 0.5, t.sin() * 0.5);
                    LinearGradient::new(
                        UnitPoint::new(0.5 - dx, 0.5 - dy),
                        UnitPoint::new(0.5 + dx, 0.5 + dy),
                        vec![(0.0, Color::hex(0x16A085)), (1.0, Color::hex(0x2C3E50))],
                    )
                })
                .frame(76.0, 56.0)
                .id("gradient-angle"),
        ))
        .spacing(12.0),
        // Radial: centered glow, off-center highlight, and a multi-stop "sunset" in a
        // non-square frame (the unit-space radius stretches elliptically to the bounds).
        row((
            circle()
                .fill_radial(RadialGradient::centered(
                    Color::hex(0xFFF2B0),
                    Color::hex(0xE67E22),
                ))
                .frame(56.0, 56.0)
                .id("gradient-radial"),
            circle()
                .fill_radial(RadialGradient::new(
                    UnitPoint::new(0.35, 0.35),
                    0.75,
                    vec![(0.0, Color::hex(0xBBDEFB)), (1.0, Color::hex(0x1D5FA8))],
                ))
                .frame(56.0, 56.0)
                .id("gradient-radial-offset"),
            rounded_rectangle(12.0)
                .fill_radial(RadialGradient::new(
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
