use day::prelude::*;
use day_piece_rating::{Card, badge, rating};

use crate::palette::{AMBER, AZURE, CORAL, INK, RUST, SKY, SLATE, TEAL, VIOLET};
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
/// centres orbit it).
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
    // A 3×2 grid of width-flexible swatches (like the Kinds grid) — every swatch responds to
    // the shared angle: linear lines spin about the unit-box centre, radial centres orbit it.
    const H: f64 = 72.0;
    section((
        grid((
            grid_row((
                // Dawn: the icon's amber down into its rust sun-base.
                rectangle()
                    .fill_linear(linear(
                        UnitPoint::TOP,
                        UnitPoint::BOTTOM,
                        vec![(0.0, AMBER), (1.0, RUST)],
                    ))
                    .height(H)
                    .id("gradient-vertical")
                    .grow_w(),
                rounded_rectangle(12.0)
                    .fill_linear(linear(
                        UnitPoint::LEADING,
                        UnitPoint::TRAILING,
                        vec![(0.0, VIOLET), (1.0, SKY)],
                    ))
                    .height(H)
                    .id("gradient-horizontal")
                    .grow_w(),
                // The website's sunrise gradient (--grad-day): amber through coral into blue.
                circle()
                    .fill_linear(linear(
                        UnitPoint::TOP_LEADING,
                        UnitPoint::BOTTOM_TRAILING,
                        vec![(0.0, AMBER), (0.5, CORAL), (1.0, AZURE)],
                    ))
                    .height(H)
                    .id("gradient-stops")
                    .grow_w(),
            )),
            grid_row((
                rounded_rectangle(12.0)
                    .fill_linear(linear(
                        UnitPoint::LEADING,
                        UnitPoint::TRAILING,
                        vec![(0.0, TEAL), (1.0, INK)],
                    ))
                    .height(H)
                    .id("gradient-angle")
                    .grow_w(),
                // Radial: off-center highlight, and a multi-stop "sunset" in a non-square
                // frame (the unit-space radius stretches elliptically to the bounds).
                circle()
                    .fill_radial(radial(
                        UnitPoint::new(0.35, 0.35),
                        0.75,
                        vec![(0.0, Color::hex(0xD9E6FF)), (1.0, SKY)],
                    ))
                    .height(H)
                    .id("gradient-radial-offset")
                    .grow_w(),
                rounded_rectangle(12.0)
                    .fill_radial(radial(
                        UnitPoint::BOTTOM,
                        1.0,
                        vec![(0.0, AMBER), (0.5, RUST), (1.0, INK)],
                    ))
                    .height(H)
                    .id("gradient-radial-stops")
                    .grow_w(),
            )),
        ))
        .spacing(12.0),
        labeled(
            crate::res::str::gradient_angle(),
            slider(angle).range(0.0..=360.0).id("gradient-angle-slider"),
        ),
    ))
    .title(crate::res::str::gradients_title())
}

/// The nine shape kinds in a 3×3 grid whose cells split the section width evenly (`grow_w`
/// marks every column flexible — docs/grid.md) and whose drawing scales with the cell. ONE
/// angle slider rotates every shape live. Each cell draws through [`shape_group_fn`], sizing
/// its shape to the largest box whose ROTATED bounding box still fits the laid-out cell —
/// rotation never clips, at any angle, on backends that clip a canvas to its bounds (Qt,
/// Android, the web).
fn shapes_section() -> impl Piece {
    let angle = Signal::new(0.0f64);
    const H: f64 = 96.0;
    // A Kinds cell: `make()`'s shape at `aspect` (height:width), centred, shrink-to-fit under
    // the shared rotation. Reads `angle` inside the recorder, so the slider re-records live.
    let cell = move |aspect: f64, make: fn() -> ShapePiece| {
        shape_group_fn(move |size| {
            let a = angle.get();
            let (s, c) = a.to_radians().sin_cos();
            let (s, c) = (s.abs(), c.abs());
            let avail_w = (size.width - 8.0).max(1.0);
            let avail_h = (size.height - 8.0).max(1.0);
            // Largest w × (w·aspect) whose rotated bounding box fits the cell.
            let w = (avail_w / (c + aspect * s)).min(avail_h / (s + aspect * c));
            let (uw, uh) = (w / size.width, w * aspect / size.height);
            vec![
                make()
                    .rotate(a)
                    .at((1.0 - uw) / 2.0, (1.0 - uh) / 2.0, uw, uh),
            ]
        })
        .height(H)
    };
    section((
        grid((
            grid_row((
                cell(0.5, || rectangle().fill(SKY))
                    .id("shape-rect")
                    .grow_w(),
                cell(0.5, || rounded_rectangle(12.0).fill(VIOLET))
                    .id("shape-rrect")
                    .grow_w(),
                cell(1.0, || circle().fill(TEAL))
                    .id("shape-circle")
                    .grow_w(),
            )),
            grid_row((
                cell(0.45, || capsule().fill(CORAL))
                    .id("shape-capsule")
                    .grow_w(),
                cell(0.55, || ellipse().stroke(AZURE, 4.0))
                    .id("shape-ellipse")
                    .grow_w(),
                cell(1.0, || arc(135.0, 270.0).stroke(TEAL, 6.0))
                    .id("shape-arc")
                    .grow_w(),
            )),
            grid_row((
                // Line + polygon resolve unit points against their box (docs/shapes.md §3.1).
                cell(1.0, || line((0.1, 0.85), (0.9, 0.15)).stroke(SLATE, 4.0))
                    .id("shape-line")
                    .grow_w(),
                cell(1.0, || {
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
                    .fill(AMBER)
                })
                .id("shape-polygon")
                .grow_w(),
                // A multi-shape group in ONE canvas leaf (docs/shapes.md §3.6): a target —
                // ring, disc, four tick lines — spun by rotating just the LINES (each line's
                // spec spans the group's box, so `.rotate` orbits its endpoints about the
                // centre); the centred ring and disc are rotation-invariant, and the figure
                // stays inside its circumcircle, so it needs no shrink-to-fit.
                shape_group_fn(move |size| {
                    let a = angle.get();
                    let side = (size.width.min(size.height) - 8.0).max(1.0);
                    let (uw, uh) = (side / size.width, side / size.height);
                    let (ux, uy) = ((1.0 - uw) / 2.0, (1.0 - uh) / 2.0);
                    // The disc's own unit rect, composed into the centred square box.
                    let (dx, dy) = (ux + 0.38 * uw, uy + 0.38 * uh);
                    vec![
                        circle().stroke(RUST, 4.0).inset(4.0).at(ux, uy, uw, uh),
                        circle().fill(RUST).at(dx, dy, 0.24 * uw, 0.24 * uh),
                        line((0.5, 0.0), (0.5, 0.14))
                            .stroke(RUST, 3.0)
                            .rotate(a)
                            .at(ux, uy, uw, uh),
                        line((0.5, 0.86), (0.5, 1.0))
                            .stroke(RUST, 3.0)
                            .rotate(a)
                            .at(ux, uy, uw, uh),
                        line((0.0, 0.5), (0.14, 0.5))
                            .stroke(RUST, 3.0)
                            .rotate(a)
                            .at(ux, uy, uw, uh),
                        line((0.86, 0.5), (1.0, 0.5))
                            .stroke(RUST, 3.0)
                            .rotate(a)
                            .at(ux, uy, uw, uh),
                    ]
                })
                .height(H)
                .id("shape-group")
                .grow_w(),
            )),
        ))
        .spacing(12.0),
        labeled(
            crate::res::str::shapes_angle(),
            slider(angle).range(0.0..=360.0).id("shapes-angle-slider"),
        ),
    ))
    .title(crate::res::str::shapes_kinds())
}

/// Three custom-drawn readings of ONE volume signal — the arc dial, a VU-style segment
/// meter, and a sunrise (the sun climbs from the left horizon to the zenith and sets to the
/// right as the value runs 0→100). Laid out like the grids above: three width-flexible cells
/// splitting the row evenly, each canvas re-recording at its laid-out size.
fn gauge_section() -> impl Piece {
    let level = Signal::new(40.0f64);
    const H: f64 = 120.0;
    section((
        labeled(
            crate::res::str::volume_label(),
            slider(level).range(0.0..=100.0).id("gauge-slider"),
        ),
        grid((grid_row((
            gauge(level).height(H).grow_w(),
            led_meter(level).height(H).grow_w(),
            sunrise_meter(level).height(H).grow_w(),
        )),))
        .spacing(12.0),
    ))
    .title(crate::res::str::canvas_gauge())
}

/// A VU-style segment meter: twelve bottom-anchored bars in a rising ramp, lit up to the
/// level — teal through amber into coral at the top of the scale, the unlit tail dimmed.
fn led_meter(level: Signal<f64>) -> AnyPiece {
    canvas(move |d, size| {
        const N: usize = 12;
        let gap = (size.width * 0.012).clamp(3.0, 8.0);
        let w = (size.width - 16.0 - gap * (N as f64 - 1.0)) / N as f64;
        let max_h = size.height - 16.0;
        if w <= 1.0 || max_h <= 8.0 {
            return;
        }
        let frac = (level.get() / 100.0).clamp(0.0, 1.0);
        let lit = (frac * N as f64).round() as usize;
        let track = Color::rgba(0.5, 0.5, 0.55, 0.25);
        for i in 0..N {
            let t = (i as f64 + 1.0) / N as f64;
            let h = max_h * (0.35 + 0.65 * t);
            let color = if i < lit {
                if t > 0.85 {
                    CORAL
                } else if t > 0.6 {
                    AMBER
                } else {
                    TEAL
                }
            } else {
                track
            };
            d.fill(
                Shape::RoundedRect(
                    Rect::new(8.0 + i as f64 * (w + gap), 8.0 + (max_h - h), w, h),
                    (w / 2.0).min(4.0),
                ),
                color,
            );
        }
    })
    .a11y(move |a| {
        a.role(Role::Meter)
            .label(crate::res::str::volume_label().format())
            .value(format!("{:.0}", level.get_untracked()))
    })
    .id("gauge-led")
}

/// A sunrise meter: the sun travels a half-circle above the horizon — rising from the left
/// at 0, zenith at 50, setting to the right at 100 — with rays, a faint path track, and a
/// ground line. All geometry derives from the laid-out size.
fn sunrise_meter(level: Signal<f64>) -> AnyPiece {
    canvas(move |d, size| {
        let frac = (level.get() / 100.0).clamp(0.0, 1.0);
        let horizon_y = size.height - 24.0;
        let cx = size.width / 2.0;
        let r = (size.width / 2.0 - 26.0).min(horizon_y - 26.0);
        if r <= 10.0 {
            return;
        }
        let track = Color::rgba(0.5, 0.5, 0.55, 0.3);
        // The sun's path, then the ground.
        d.stroke(
            Shape::Arc {
                rect: Rect::new(cx - r, horizon_y - r, r * 2.0, r * 2.0),
                start_deg: 180.0,
                sweep_deg: 180.0,
            },
            track,
            2.0,
        );
        d.fill(
            Shape::Rect(Rect::new(
                8.0,
                horizon_y,
                size.width - 16.0,
                size.height - horizon_y - 8.0,
            )),
            Color::rgba(0.5, 0.5, 0.55, 0.15),
        );
        d.stroke(
            Shape::Line(
                Point::new(8.0, horizon_y),
                Point::new(size.width - 8.0, horizon_y),
            ),
            SLATE,
            2.5,
        );
        // Sun position along the half-circle (y-down coords: subtract the sine).
        let ang = std::f64::consts::PI * (1.0 - frac);
        let (sx, sy) = (cx + r * ang.cos(), horizon_y - r * ang.sin());
        let sun_r = (r * 0.16).clamp(6.0, 15.0);
        for i in 0..8 {
            let ra = f64::from(i) * std::f64::consts::FRAC_PI_4;
            let (rc, rs) = (ra.cos(), ra.sin());
            d.stroke(
                Shape::Line(
                    Point::new(sx + rc * (sun_r + 4.0), sy + rs * (sun_r + 4.0)),
                    Point::new(sx + rc * (sun_r + 9.0), sy + rs * (sun_r + 9.0)),
                ),
                RUST,
                2.5,
            );
        }
        d.fill(
            Shape::Ellipse(Rect::new(sx - sun_r, sy - sun_r, sun_r * 2.0, sun_r * 2.0)),
            AMBER,
        );
    })
    .a11y(move |a| {
        a.role(Role::Meter)
            .label(crate::res::str::volume_label().format())
            .value(format!("{:.0}", level.get_untracked()))
    })
    .id("gauge-sunrise")
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
    let accent = TEAL;

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
            badge(3, rounded_rectangle(10.0).fill(SLATE).frame(48.0, 48.0)),
        ))
        .spacing(20.0),
        // 3) ButtonStyle — a FilledButtonStyle button next to a plain one for contrast.
        row((
            button(crate::res::str::compose_plain_btn()).id("compose-plain-btn"),
            button(crate::res::str::compose_styled_btn())
                .style(FilledButtonStyle { color: SKY })
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
