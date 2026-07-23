//! Declarative animation (§8.4, docs/animation.md): one box driven by every animatable channel at
//! once. The sliders/picker/stepper only **queue** target values; the buttons at the top commit
//! them inside a single `with_animation`, so scale, rotation, opacity, offset, and colour animate
//! together with the chosen curve and duration.
//!
//! - **Animate!** — animate to the queued slider values.
//! - **Randomize!** — set the sliders to random values, then animate.
//! - **Reset** — set the sliders to their defaults, then animate.
//!
//! The Hue slider builds the box's colour with `Color::hsl` (HSL is accepted at every colour
//! parameter). Animation is backend-executed: on the backends that map the seams the toolkit
//! interpolates; elsewhere the value applies at commit.

use std::cell::Cell;

use day::prelude::*;

use crate::widgets::page;

/// Every signal the page threads together: the queued (`p_`) targets the controls write, the
/// applied (`a_`) state the box renders, and the animation settings. All fields are `Signal`
/// (`Copy`), so the whole struct is `Copy` and drops straight into each button's handler.
#[derive(Clone, Copy)]
struct Anim {
    p_scale: Signal<f64>,
    p_rot: Signal<f64>,
    p_op: Signal<f64>,
    p_offx: Signal<f64>,
    p_offy: Signal<f64>,
    p_hue: Signal<f64>,
    a_scale: Signal<f64>,
    a_rot: Signal<f64>,
    a_op: Signal<f64>,
    a_offx: Signal<f64>,
    a_offy: Signal<f64>,
    a_hue: Signal<f64>,
    curve: Signal<usize>,
    dur: Signal<i64>,
}

impl Anim {
    fn new() -> Self {
        Anim {
            p_scale: Signal::new(1.0),
            p_rot: Signal::new(0.0),
            p_op: Signal::new(1.0),
            p_offx: Signal::new(0.0),
            p_offy: Signal::new(0.0),
            p_hue: Signal::new(210.0),
            a_scale: Signal::new(1.0),
            a_rot: Signal::new(0.0),
            a_op: Signal::new(1.0),
            a_offx: Signal::new(0.0),
            a_offy: Signal::new(0.0),
            a_hue: Signal::new(210.0),
            curve: Signal::new(0),
            dur: Signal::new(600),
        }
    }

    /// Commit every queued value at once, under one animation — so all channels move together.
    fn commit(self) {
        let spec = spec_for(self.curve.get_untracked(), self.dur.get_untracked() as u32);
        with_animation(spec, move || {
            self.a_scale.set(self.p_scale.get_untracked());
            self.a_rot.set(self.p_rot.get_untracked());
            self.a_op.set(self.p_op.get_untracked());
            self.a_offx.set(self.p_offx.get_untracked());
            self.a_offy.set(self.p_offy.get_untracked());
            self.a_hue.set(self.p_hue.get_untracked());
        });
    }

    /// Queue random targets (updating the sliders), then animate to them.
    fn randomize(self) {
        self.p_scale.set(rand_range(0.5, 1.5));
        self.p_rot.set(rand_range(0.0, 360.0));
        self.p_op.set(rand_range(0.2, 1.0));
        self.p_offx.set(rand_range(-72.0, 72.0));
        self.p_offy.set(rand_range(-60.0, 60.0));
        self.p_hue.set(rand_range(0.0, 360.0));
        self.commit();
    }

    /// Queue the defaults (resetting the sliders), then animate back to them.
    fn reset(self) {
        self.p_scale.set(1.0);
        self.p_rot.set(0.0);
        self.p_op.set(1.0);
        self.p_offx.set(0.0);
        self.p_offy.set(0.0);
        self.p_hue.set(210.0);
        self.commit();
    }
}

pub(crate) fn animation_page() -> AnyPiece {
    let s = Anim::new();

    // Three equal-width, distinctly-coloured action buttons across the top.
    let actions = row((
        action_button(
            "anim-randomize",
            "Randomize!",
            Color::hsl(280.0, 0.55, 0.52),
            move || s.randomize(),
        ),
        action_button(
            "anim-go",
            "Animate!",
            Color::hsl(145.0, 0.55, 0.42),
            move || s.commit(),
        ),
        action_button(
            "anim-reset",
            "Reset",
            Color::hsl(0.0, 0.0, 0.42),
            move || s.reset(),
        ),
    ))
    .spacing(10.0);

    let body = column((
        actions,
        stage(s),
        form((section((
            labeled("Scale", slider(s.p_scale).range(0.5..=1.5).id("anim-scale")),
            labeled(
                "Rotation",
                slider(s.p_rot).range(0.0..=360.0).id("anim-rotation"),
            ),
            labeled(
                "Opacity",
                slider(s.p_op).range(0.15..=1.0).id("anim-opacity"),
            ),
            labeled(
                "Offset X",
                slider(s.p_offx).range(-72.0..=72.0).id("anim-offx"),
            ),
            labeled(
                "Offset Y",
                slider(s.p_offy).range(-60.0..=60.0).id("anim-offy"),
            ),
            labeled("Hue", slider(s.p_hue).range(0.0..=360.0).id("anim-hue")),
            labeled(
                "Curve",
                picker(["Spring", "Ease-in-out", "Ease-out", "Linear"], s.curve)
                    .segmented()
                    .id("anim-curve"),
            ),
            labeled("Duration", duration_stepper(s.dur)),
        )),)),
    ))
    .spacing(16.0);

    page(
        crate::res::str::nav_animation(),
        "animation-title",
        Some(crate::res::str::anim_caption()),
        body.any(),
    )
}

/// The box centred in a large stage that fills the page width. The box is a DIRECT child of the
/// stage (no box-sized wrapper between them) and its `.transform` is the OUTERMOST modifier, so the
/// transform moves it within the *stage's* bounds — the only clipping container above it. That's
/// what lets it travel outside its own frame on the toolkits that clip children (Android/GTK/Qt),
/// which AppKit/UIKit allow natively. The slider ranges keep it inside the stage on any screen.
fn stage(s: Anim) -> AnyPiece {
    let (a_hue, a_offx, a_offy, a_scale, a_rot, a_op) =
        (s.a_hue, s.a_offx, s.a_offy, s.a_scale, s.a_rot, s.a_op);
    let box_view = label("")
        .frame(112.0, 112.0)
        .background(move || Color::hsl(a_hue.get(), 0.65, 0.55))
        .corner_radius(24.0)
        .opacity(a_op)
        .transform(move || Transform {
            tx: a_offx.get(),
            ty: a_offy.get(),
            sx: a_scale.get(),
            sy: a_scale.get(),
            rotate_deg: a_rot.get(),
            anchor_x: 0.5,
            anchor_y: 0.5,
        })
        .id("anim-box");
    // A zstack over a transparent sizer: it centres the box (both axes) in a large area and makes
    // that area the box's only clipping parent, so the box travels freely within it.
    zstack((label("").frame(320.0, 300.0), box_view)).any()
}

/// A filled, tappable, equal-width action button (`.grow_w()` makes each take an equal share of the
/// row). Coloured directly so each reads distinctly.
fn action_button(
    id: &'static str,
    text: &'static str,
    color: Color,
    on_tap: impl Fn() + 'static,
) -> AnyPiece {
    row((spacer(), label(text).color(Color::WHITE).bold(), spacer()))
        .padding(Insets::symmetric(6.0, 12.0))
        .background(color)
        .corner_radius(10.0)
        .on_tap(on_tap)
        .id(id)
        .grow_w()
}

/// Curve index (from the segmented picker) + duration → an [`Animation`]. The stepper's duration
/// drives the spring's response too, so it affects every curve.
fn spec_for(curve: usize, dur_ms: u32) -> Animation {
    match curve {
        0 => Animation::spring((dur_ms as f64 / 1000.0).max(0.1), 0.6),
        1 => Animation::ease_in_out(dur_ms),
        2 => Animation::ease_out(dur_ms),
        _ => Animation::linear(dur_ms),
    }
}

/// A −/value/+ stepper for the animation duration (100–3000 ms, 100 ms steps). Day has no stepper
/// piece, so it is composed from two buttons around a reactive readout.
fn duration_stepper(dur: Signal<i64>) -> impl Piece {
    row((
        button("−")
            .bordered()
            .action(move || dur.update(|d| *d = (*d - 100).max(100)))
            .id("anim-dur-dec"),
        label(move || format!("{} ms", dur.get())).id("anim-dur-value"),
        button("+")
            .bordered()
            .action(move || dur.update(|d| *d = (*d + 100).min(3000)))
            .id("anim-dur-inc"),
    ))
    .spacing(12.0)
}

/// A uniform random `f64` in `lo..=hi` — a tiny xorshift seeded once from the clock (no rand dep).
fn rand_range(lo: f64, hi: f64) -> f64 {
    thread_local! { static SEED: Cell<u64> = const { Cell::new(0) }; }
    let x = SEED.with(|s| {
        let mut x = s.get();
        if x == 0 {
            x = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E37_79B9_7F4A_7C15)
                | 1;
        }
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        x
    });
    let unit = (x >> 11) as f64 / (1u64 << 53) as f64;
    lo + (hi - lo) * unit
}
