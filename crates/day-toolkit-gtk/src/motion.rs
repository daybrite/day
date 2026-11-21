// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Animated property changes on GTK 4's frame clock (docs/tween.md "The animation contract").
//!
//! GTK has no animator for a widget's opacity, its place in a `GtkFixed`, a child transform or
//! a CSS color. libadwaita's `AdwTimedAnimation` has no delay, its `AdwSpringAnimation` no
//! repeat, and both jump to their end while `gtk-enable-animations` is off. So every animated
//! set is a [`Run`], stepped from one tick callback per widget on the window's frame clock (the
//! clock Day's own frames come from). Each frame samples [`AnimSpec::sample`] at the frame time
//! since the run's first frame, which keeps the delay, the repeats (for ever included),
//! autoreverse and the physical spring in one place, and writes `from + (to - from) × progress`,
//! then the target once the run is over.
//!
//! - **Retarget**: a new animated set starts from the value shown, which this module keeps (the
//!   transform, the frame, the fill) or reads from the widget (its opacity).
//! - **Plain set**: drops the property's run, a run that repeats for ever included, and applies
//!   the value at once.
//! - **Not mapped**: a widget that is not on screen takes the value at once, as libadwaita's
//!   own animations do. A widget unmapped in flight resumes on its next frame at the time
//!   really elapsed, so a finite run that ended meanwhile lands on its target there.
//! - **Read back**: [`displayed`] answers the values last written, which are what GTK draws.

use day_spec::sidetable::SideTable;
use day_spec::{AnimSpec, Animatable, Color, Displayed, Ink, Paint, Point, Rect, Size, Transform};
use gtk4::glib;
use gtk4::prelude::*;

use crate::{Handle, widget_key};

thread_local! {
    /// Each animated widget's runs and the values it shows. A [`SideTable`], so `release`'s
    /// sweep drops it; the tick callback then finds nothing and ends.
    static MOTION: SideTable<Motion> = SideTable::new();
}

/// One property moving from `from` to `to` under `spec`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Run<T> {
    from: T,
    to: T,
    spec: AnimSpec,
    /// Frame time (µs) of the run's first frame: elapsed time counts from the frame the change
    /// is drawn with, as Day's own frame-driven animations count.
    start: Option<i64>,
}

impl<T: Animatable> Run<T> {
    pub(crate) fn new(from: T, to: T, spec: AnimSpec) -> Self {
        Run {
            from,
            to,
            spec,
            start: None,
        }
    }

    /// The value at frame time `now` (µs), and whether the run is over. Over, the value is the
    /// target, also after a cycle that ran backwards (the contract's rest rule).
    pub(crate) fn at(&mut self, now: i64) -> (T, bool) {
        let start = *self.start.get_or_insert(now);
        let elapsed = now.saturating_sub(start).max(0) as f64 / 1_000_000.0;
        let sample = self.spec.sample(elapsed);
        if sample.done {
            (self.to, true)
        } else {
            (self.from.lerp(self.to, sample.progress), false)
        }
    }
}

/// Whether `spec` plays at all: a zero-length cycle with no delay is the target at once.
fn plays(spec: &AnimSpec) -> bool {
    spec.duration_ms > 0 || spec.delay_ms > 0
}

/// One widget's runs, and the values it shows that the widget cannot answer itself.
#[derive(Default)]
struct Motion {
    opacity: Option<Run<f64>>,
    /// The transform run and the laid-out size it pivots about.
    transform: Option<(Run<Transform>, Size)>,
    frame: Option<Run<Rect>>,
    /// The fill's color run and the ink it lands on: the last frame writes the ink itself,
    /// so a fill that follows the appearance keeps following it once the run is over.
    background: Option<(Run<Color>, Ink)>,
    /// The transform shown, at rest or in flight.
    transform_shown: Transform,
    /// The frame last placed, in the parent's coordinates.
    frame_shown: Option<Rect>,
    /// Whether a tick callback is stepping the runs.
    ticking: bool,
}

/// What one frame writes.
#[derive(Default)]
struct Step {
    opacity: Option<f64>,
    transform: Option<(Transform, Size)>,
    frame: Option<Rect>,
    background: Option<Ink>,
    running: bool,
}

impl Motion {
    /// Advance every run to frame time `now`, dropping the ones that are over.
    fn advance(&mut self, now: i64) -> Step {
        fn take<T: Animatable>(run: &mut Option<Run<T>>, now: i64) -> Option<T> {
            let (value, done) = run.as_mut()?.at(now);
            if done {
                *run = None;
            }
            Some(value)
        }
        let mut step = Step {
            opacity: take(&mut self.opacity, now),
            frame: take(&mut self.frame, now),
            ..Step::default()
        };
        if let Some((run, ink)) = &mut self.background {
            let (c, done) = run.at(now);
            step.background = Some(if done { *ink } else { Ink::Fixed(c) });
            if done {
                self.background = None;
            }
        }
        if let Some((run, size)) = &mut self.transform {
            let (t, done) = run.at(now);
            step.transform = Some((t, *size));
            if done {
                self.transform = None;
            }
        }
        if let Some((t, _)) = step.transform {
            self.transform_shown = t;
        }
        if step.frame.is_some() {
            self.frame_shown = step.frame;
        }
        step.running = self.opacity.is_some()
            || self.transform.is_some()
            || self.frame.is_some()
            || self.background.is_some();
        self.ticking = step.running;
        step
    }
}

/// Run `f` on `key`'s motion state, made on first use.
fn with_motion<R>(key: usize, f: impl FnOnce(&mut Motion) -> R) -> R {
    MOTION.with(|t| {
        let mut m = t.take(key).unwrap_or_default();
        let r = f(&mut m);
        t.insert(key, m);
        r
    })
}

/// Make sure a tick callback steps `h`'s runs.
fn ensure_ticking(h: &Handle) {
    let key = widget_key(h);
    let start = with_motion(key, |m| !std::mem::replace(&mut m.ticking, true));
    if start {
        h.add_tick_callback(|w, clock| tick(w, clock.frame_time()));
    }
}

/// One frame of `w`'s runs: advance them, then write what they show.
fn tick(w: &gtk4::Widget, now: i64) -> glib::ControlFlow {
    let key = widget_key(w);
    // Advance under the table's borrow, write outside it: the writes reach other tables.
    let Some(step) = MOTION.with(|t| t.with(key, |m| m.advance(now))) else {
        return glib::ControlFlow::Break;
    };
    if let Some(o) = step.opacity {
        w.set_opacity(o.clamp(0.0, 1.0));
    }
    let fixed = w.parent().and_then(|p| p.downcast::<gtk4::Fixed>().ok());
    if let Some(rect) = step.frame
        && let Some(fixed) = &fixed
    {
        crate::place_in_fixed(fixed, w, rect);
    }
    if let Some((t, size)) = step.transform
        && let Some(fixed) = &fixed
    {
        crate::apply_gtk_transform(fixed, w, t, size);
    }
    if let Some(ink) = step.background {
        crate::paint_fill(w, Some(Paint::Solid(ink)));
    }
    if step.running {
        glib::ControlFlow::Continue
    } else {
        glib::ControlFlow::Break
    }
}

/// The spec to play on `h`, or `None` to apply at once: no animation asked, a zero-length one,
/// or a widget that is not on screen.
fn playing(h: &Handle, anim: Option<&AnimSpec>) -> Option<AnimSpec> {
    anim.filter(|a| plays(a) && h.is_mapped()).copied()
}

/// `Toolkit::set_opacity`: the widget's own opacity, the value shown included.
pub(crate) fn set_opacity(h: &Handle, opacity: f64, anim: Option<&AnimSpec>) {
    let key = widget_key(h);
    match playing(h, anim) {
        None => {
            MOTION.with(|t| t.with(key, |m| m.opacity = None));
            h.set_opacity(opacity);
        }
        Some(spec) => {
            let from = h.opacity();
            with_motion(key, |m| m.opacity = Some(Run::new(from, opacity, spec)));
            ensure_ticking(h);
        }
    }
}

/// `Toolkit::set_transform` for a child of a `GtkFixed`, whose transform slot also carries
/// the child's laid-out origin ([`crate::apply_gtk_transform`]).
pub(crate) fn set_transform(
    fixed: &gtk4::Fixed,
    h: &Handle,
    t: Transform,
    size: Size,
    anim: Option<&AnimSpec>,
) {
    let key = widget_key(h);
    match playing(h, anim) {
        None => {
            with_motion(key, |m| {
                m.transform = None;
                m.transform_shown = t;
            });
            crate::apply_gtk_transform(fixed, h, t, size);
        }
        Some(spec) => {
            with_motion(key, |m| {
                m.transform = Some((Run::new(m.transform_shown, t, spec), size));
            });
            ensure_ticking(h);
        }
    }
}

/// The transform `h` shows, if it was ever given one.
pub(crate) fn shown_transform(h: &Handle) -> Option<Transform> {
    MOTION.with(|t| t.with(widget_key(h), |m| m.transform_shown))
}

/// `Toolkit::set_frame` for a child of a `GtkFixed`: placed at once, or moved and resized
/// frame by frame through [`crate::place_in_fixed`], from the frame shown to `frame`. A widget
/// never placed before takes its first frame at once.
pub(crate) fn set_frame(fixed: &gtk4::Fixed, h: &Handle, frame: Rect, anim: Option<&AnimSpec>) {
    let key = widget_key(h);
    let spec = playing(h, anim);
    let animates = with_motion(key, |m| {
        let from = m.frame_shown.filter(|f| !f.approx_eq(&frame, 0.25));
        if let (Some(spec), Some(from)) = (spec, from) {
            m.frame = Some(Run::new(from, frame, spec));
            true
        } else {
            m.frame = None;
            m.frame_shown = Some(frame);
            false
        }
    });
    if animates {
        ensure_ticking(h);
    } else {
        crate::place_in_fixed(fixed, h, frame);
    }
}

/// A container's fill (`ContainerPatch::Fill`): a solid fill is blended per sRGB channel
/// from the color shown to the new ink's color in the current appearance, then the ink
/// itself once over. A fill appearing or going away, or a gradient on either side, applies at
/// once.
pub(crate) fn set_background(h: &Handle, fill: Option<Paint>, anim: Option<&AnimSpec>) {
    let key = widget_key(h);
    let from = match crate::surface_fill(h) {
        Some(Paint::Solid(_)) => crate::surface_background(h),
        _ => None,
    };
    match (playing(h, anim), from, &fill) {
        (Some(spec), Some(from), Some(Paint::Solid(ink))) if from != ink.resolved() => {
            let run = Run::new(from, ink.resolved(), spec);
            with_motion(key, |m| m.background = Some((run, *ink)));
            ensure_ticking(h);
        }
        _ => {
            MOTION.with(|t| t.with(key, |m| m.background = None));
            crate::paint_fill(h, fill);
        }
    }
}

/// `Toolkit::read_displayed`: the values last written, which are what GTK draws.
pub(crate) fn displayed(h: &Handle) -> Displayed {
    let key = widget_key(h);
    let (transform, local) = MOTION
        .with(|t| {
            t.with(key, |m| {
                (
                    m.transform_shown,
                    m.frame.as_ref().and(m.frame_shown).map(|f| f.size),
                )
            })
        })
        .unwrap_or_default();
    Displayed {
        opacity: Some(h.opacity()),
        frame: displayed_frame(h, local),
        translation: Some(Point::new(transform.tx, transform.ty)),
        background: crate::surface_background(h),
    }
}

/// `h`'s frame in window-content space before its transform: its parent's place plus Day's
/// origin for it and the size shown (`size` while a frame run moves it, else its allocation).
fn displayed_frame(h: &Handle, size: Option<Size>) -> Option<Rect> {
    if !h.is_mapped() {
        return None;
    }
    let Some(parent) = h.parent().filter(ObjectExt::is::<gtk4::Fixed>) else {
        return crate::native_frame(h);
    };
    let content = crate::content_root(h)?;
    let at = parent.compute_bounds(&content)?;
    let (ox, oy) = crate::node_origin(h);
    let size = size.unwrap_or_else(|| Size::new(f64::from(h.width()), f64::from(h.height())));
    Some(Rect::new(
        f64::from(at.x()) + f64::from(ox),
        f64::from(at.y()) + f64::from(oy),
        size.width,
        size.height,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use day_spec::Curve;

    const MS: i64 = 1_000;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn a_run_counts_from_its_first_frame_and_holds_the_delay() {
        let mut run = Run::new(1.0, 0.0, AnimSpec::linear(600).delay(900));
        // The first frame is elapsed zero, whatever its timestamp.
        assert_eq!(run.at(5_000 * MS), (1.0, false));
        assert_eq!(run.at(5_450 * MS), (1.0, false));
        let (v, done) = run.at(6_200 * MS);
        assert!(near(v, 0.5) && !done, "{v}");
        assert_eq!(run.at(6_500 * MS), (0.0, true));
    }

    #[test]
    fn autoreverse_plays_back_and_rests_on_the_target() {
        let mut run = Run::new(1.0, 0.0, AnimSpec::linear(800).repeat(1, true));
        run.at(0);
        // 300 ms into the backward second cycle: progress 0.625 → 0.375 shown.
        let (v, _) = run.at(1_100 * MS);
        assert!(near(v, 0.375), "{v}");
        // Over on a backward cycle: the target, not the start.
        assert_eq!(run.at(1_700 * MS), (0.0, true));
    }

    #[test]
    fn a_forever_run_never_ends() {
        let mut run = Run::new(1.0, 0.0, AnimSpec::linear(600).repeat_forever(true));
        run.at(0);
        let (_, done) = run.at(3_600_000 * MS);
        assert!(!done);
    }

    #[test]
    fn a_spring_overshoots_then_lands_exactly() {
        let spec = AnimSpec::spring(0.9, 0.25);
        let mut run = Run::new(0.0, 100.0, spec);
        run.at(0);
        let (peak, _) = run.at(465 * MS);
        assert!(peak > 140.0, "{peak}");
        let end = (spec.duration_secs() * 1_000_000.0).ceil() as i64 + MS;
        assert_eq!(run.at(end), (100.0, true));
        assert!(matches!(spec.curve, Curve::Spring { .. }));
    }

    #[test]
    fn colors_blend_per_channel_between_the_ends() {
        let mut run = Run::new(Color::BLACK, Color::WHITE, AnimSpec::linear(2500));
        run.at(0);
        let (c, _) = run.at(1_000 * MS);
        assert!(near(c.r, 0.4) && near(c.g, 0.4) && near(c.b, 0.4), "{c:?}");
    }

    #[test]
    fn frames_step_origin_and_size() {
        let mut run = Run::new(
            Rect::new(0.0, 10.0, 80.0, 40.0),
            Rect::new(20.0, 10.0, 240.0, 40.0),
            AnimSpec::linear(1200),
        );
        run.at(0);
        let (r, _) = run.at(600 * MS);
        assert!(near(r.size.width, 160.0) && near(r.origin.x, 10.0), "{r:?}");
    }

    #[test]
    fn advance_stops_ticking_once_every_run_is_over() {
        let mut m = Motion {
            opacity: Some(Run::new(1.0, 0.0, AnimSpec::linear(100))),
            transform: Some((
                Run::new(
                    Transform::IDENTITY,
                    Transform::translate(0.0, 12.0),
                    AnimSpec::linear(200),
                ),
                Size::new(40.0, 40.0),
            )),
            ticking: true,
            ..Motion::default()
        };
        assert!(m.advance(0).running);
        let step = m.advance(150 * MS);
        assert_eq!(step.opacity, Some(0.0));
        assert!(step.running && m.opacity.is_none());
        let step = m.advance(250 * MS);
        assert!(!step.running && !m.ticking);
        assert!(near(m.transform_shown.ty, 12.0));
    }

    #[test]
    fn zero_length_specs_apply_at_once() {
        assert!(!plays(&AnimSpec::linear(0)));
        assert!(plays(&AnimSpec::linear(0).delay(100)));
        assert!(plays(&AnimSpec::linear(1)));
    }
}
