// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! The animation contract on Core Animation (docs/tween.md "The animation contract", §8.4).
//!
//! Every animated change sets the model value at once (Day's tree already holds the target)
//! and adds an explicit animation to the view's layer that plays from the value shown to it:
//! opacity on `opacity`; frame and transform, which AppKit expresses as the view's own geometry
//! (`apply_geometry`), on `position`, `bounds` and `transform`; a container's fill on
//! `backgroundColor`. Each run is one of
//!
//! - a `CABasicAnimation` for a linear cycle,
//! - a `CASpringAnimation` (mass 1, `Curve::spring_constants`, initial velocity 0) for a spring
//!   whose property moves in a straight line,
//! - a `CAKeyframeAnimation` through `AnimSpec::keyframes()` for the ease curves (Day's exact
//!   shapes, which Core Animation's named curves are not), for a spring that turns or scales a
//!   view, and for a color (its keyframes clamped, so every color lies between the two ends),
//!
//! with Core Animation's own timing for the rest of the contract: the delay as a `beginTime`
//! with a backwards fill, `repeatCount` (half the cycles when `autoreverses` pairs them),
//! `HUGE_VALF` for ever. Removed on completion, the animation leaves the model value, the
//! target, showing (rule 5). A retarget reads the presentation layer for its start; a change
//! with no animation removes the run.
//!
//! The fill is `drawRect` content at rest (docs/surfaces.md: corners and gradients are drawn
//! with it, its ink resolves for the appearance it is drawn in, and the offscreen capture
//! renders only what `drawRect` draws). For the length
//! of an animated change it moves to the layer's `backgroundColor`, which Core Animation can
//! interpolate, with `drawRect` leaving it out, and the transaction's completion block hands it
//! back to `drawRect` when the run ends.
use super::*;
use block2::RcBlock;
use objc2::rc::Weak;
use objc2::runtime::AnyObject;
use objc2_quartz_core::{
    CAAnimation, CABasicAnimation, CACurrentMediaTime, CAKeyframeAnimation, CALayer, CAMediaTiming,
    CASpringAnimation, CATransform3D, NSValueCATransform3DAdditions, kCAFillModeBackwards,
};

// ---------------------------------------------------------------------------
// Pure logic (unit-tested below)
// ---------------------------------------------------------------------------

/// Core Animation's `repeatCount` for a spec. Without `autoreverse` it is the number of
/// cycles; with it a forward and a backward pass count as one repeat, so it is half of them
/// (a fraction plays the odd forward cycle). A spec that repeats forever is `HUGE_VALF`.
pub(super) fn repeat_count(spec: &AnimSpec) -> f32 {
    let Some(cycles) = spec.cycles() else {
        return f32::INFINITY;
    };
    let count = if spec.autoreverse {
        f64::from(cycles) / 2.0
    } else {
        f64::from(cycles)
    };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "Core Animation takes a float count; a cycle count beyond f32's range repeats as long as anyone watches"
    )]
    let count = count as f32;
    count
}

/// How one property's run plays on Core Animation.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Plan {
    /// A zero-length cycle after a delay: the start value is held, then the target shows.
    Hold,
    /// A two-value `CABasicAnimation` at constant speed.
    Linear,
    /// A `CASpringAnimation` with these constants for a mass of 1.
    Spring { stiffness: f64, damping: f64 },
    /// A `CAKeyframeAnimation` through these `(offset, progress)` points, linear between them.
    Keyframes(Vec<(f64, f64)>),
}

/// The plan for a run of `spec`, or `None` when it applies at once (no length and no delay).
/// `straight`: the property's value is linear in the progress, so Core Animation's own
/// interpolation between two ends is exact. `clamp`: the in-between values must stay between
/// the ends (a color). `overdamping`: the running system's spring animator takes a damping
/// ratio above 1 (`allowsOverdamping`, macOS 14).
pub(super) fn plan(
    spec: &AnimSpec,
    straight: bool,
    clamp: bool,
    overdamping: bool,
) -> Option<Plan> {
    if spec.duration_ms == 0 {
        return (spec.delay_ms > 0).then_some(Plan::Hold);
    }
    match spec.curve {
        Curve::Linear if straight => return Some(Plan::Linear),
        Curve::Spring { damping, .. } if straight && !clamp && (damping <= 1.0 || overdamping) => {
            if let Some((stiffness, damping)) = spec.curve.spring_constants() {
                return Some(Plan::Spring { stiffness, damping });
            }
        }
        _ => {}
    }
    Some(Plan::Keyframes(
        spec.keyframes()
            .into_iter()
            .map(|k| {
                let progress = if clamp {
                    k.progress.clamp(0.0, 1.0)
                } else {
                    k.progress
                };
                (k.offset, progress)
            })
            .collect(),
    ))
}

/// A layer's geometry as the parts a transition moves: the center of its bounds in the
/// superlayer, the bounds' size, and the transform's scale and rotation (radians, Core
/// Animation's sense). AppKit puts a view's translation in `position`, its scale and rotation
/// in `transform` (see `apply_geometry`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Geo {
    pub(super) center: (f64, f64),
    pub(super) size: (f64, f64),
    pub(super) scale: (f64, f64),
    pub(super) angle: f64,
}

impl Geo {
    /// The geometry of a layer at `position` with bounds `size`, `anchor` (unit point) and the
    /// 2D part `[m11, m12, m21, m22]` of its transform (row vectors, as Core Animation applies
    /// it: `p' = p · M`).
    pub(super) fn read(
        position: (f64, f64),
        size: (f64, f64),
        anchor: (f64, f64),
        m: [f64; 4],
    ) -> Geo {
        let [a, b, c, d] = m;
        let sx = a.hypot(b);
        let (angle, sy) = if sx > 1e-12 {
            (b.atan2(a), (a * d - b * c) / sx)
        } else {
            (0.0, c.hypot(d))
        };
        let (ox, oy) = ((0.5 - anchor.0) * size.0, (0.5 - anchor.1) * size.1);
        Geo {
            center: (position.0 + ox * a + oy * c, position.1 + ox * b + oy * d),
            size,
            scale: (sx, sy),
            angle,
        }
    }

    /// The transform's 2D part: scale, then rotate.
    pub(super) fn matrix(&self) -> [f64; 4] {
        let (s, c) = self.angle.sin_cos();
        let (sx, sy) = self.scale;
        [sx * c, sx * s, -sy * s, sy * c]
    }

    /// The layer `position` that puts this geometry's center where it is, for `anchor`.
    pub(super) fn position(&self, anchor: (f64, f64)) -> (f64, f64) {
        let [a, b, c, d] = self.matrix();
        let (ox, oy) = (
            (0.5 - anchor.0) * self.size.0,
            (0.5 - anchor.1) * self.size.1,
        );
        (
            self.center.0 - (ox * a + oy * c),
            self.center.1 - (ox * b + oy * d),
        )
    }

    /// Part way to `to`: `0` is `self`, `1` is `to`, past `1` overshoots.
    pub(super) fn lerp(&self, to: &Geo, p: f64) -> Geo {
        let l = |a: f64, b: f64| a + (b - a) * p;
        Geo {
            center: (l(self.center.0, to.center.0), l(self.center.1, to.center.1)),
            size: (l(self.size.0, to.size.0), l(self.size.1, to.size.1)),
            scale: (l(self.scale.0, to.scale.0), l(self.scale.1, to.scale.1)),
            angle: l(self.angle, to.angle),
        }
    }

    /// Whether the scale and rotation are the same as `other`'s: then the layer's `position`
    /// and `bounds` move in a straight line between the two.
    pub(super) fn same_shape(&self, other: &Geo) -> bool {
        const EPS: f64 = 1e-9;
        (self.scale.0 - other.scale.0).abs() < EPS
            && (self.scale.1 - other.scale.1).abs() < EPS
            && (self.angle - other.angle).abs() < EPS
    }

    /// Whether the two show the same thing to within a hundredth of a point.
    pub(super) fn near(&self, other: &Geo) -> bool {
        progress(self, other, self).is_none()
    }
}

/// How far `now` is along the way from `from` to `to`, judged by the part of the geometry that
/// moves furthest (scale and rotation counted in points at the layer's edge), so a reading
/// past the target (a spring's overshoot) answers more than 1. `None` when nothing moves more
/// than a hundredth of a point.
pub(super) fn progress(from: &Geo, to: &Geo, now: &Geo) -> Option<f64> {
    let reach = from
        .size
        .0
        .max(from.size.1)
        .max(to.size.0)
        .max(to.size.1)
        .max(1.0);
    let parts = [
        (from.center.0, to.center.0, now.center.0, 1.0),
        (from.center.1, to.center.1, now.center.1, 1.0),
        (from.size.0, to.size.0, now.size.0, 1.0),
        (from.size.1, to.size.1, now.size.1, 1.0),
        (from.scale.0, to.scale.0, now.scale.0, reach),
        (from.scale.1, to.scale.1, now.scale.1, reach),
        (from.angle, to.angle, now.angle, reach / 2.0),
    ];
    let (a, b, n, weight) = parts
        .into_iter()
        .max_by(|x, y| ((x.1 - x.0).abs() * x.3).total_cmp(&((y.1 - y.0).abs() * y.3)))?;
    ((b - a).abs() * weight > 0.01).then(|| (n - a) / (b - a))
}

/// `to`'s rotation unwound so that the run turns by Day's own difference in degrees (a turn
/// from 0° to 360° spins once instead of standing still): the layer's angle is read back
/// modulo a full turn. Keeps `to` when the two disagree by more than a rounding error.
pub(super) fn unwind(from_angle: f64, to_angle: f64, day_turn_deg: f64) -> f64 {
    let want = from_angle + day_turn_deg.to_radians();
    let turns = (want - to_angle) / std::f64::consts::TAU;
    if (turns - turns.round()).abs() < 1e-6 {
        want
    } else {
        to_angle
    }
}

/// Day's geometry for a view: its laid-out frame (`[x, y, w, h]` in its parent) and transform.
pub(super) type DayGeo = ([f64; 4], Transform);

/// Part way from `a` to `b` in Day's terms.
pub(super) fn lerp_day(a: &DayGeo, b: &DayGeo, p: f64) -> DayGeo {
    let l = |x: f64, y: f64| x + (y - x) * p;
    let (ba, ta) = a;
    let (bb, tb) = b;
    (
        [
            l(ba[0], bb[0]),
            l(ba[1], bb[1]),
            l(ba[2], bb[2]),
            l(ba[3], bb[3]),
        ],
        Transform {
            tx: l(ta.tx, tb.tx),
            ty: l(ta.ty, tb.ty),
            sx: l(ta.sx, tb.sx),
            sy: l(ta.sy, tb.sy),
            rotation: day_spec::Angle::deg(l(ta.rotation.degrees(), tb.rotation.degrees())),
            ..*tb
        },
    )
}

/// An sRGB blend of two colors, channel by channel.
pub(super) fn lerp_color(a: day_spec::Color, b: day_spec::Color, p: f64) -> day_spec::Color {
    let l = |x: f64, y: f64| x + (y - x) * p;
    day_spec::Color {
        r: l(a.r, b.r),
        g: l(a.g, b.g),
        b: l(a.b, b.b),
        a: l(a.a, b.a),
    }
}

// ---------------------------------------------------------------------------
// Core Animation
// ---------------------------------------------------------------------------

const OPACITY: &str = "day.opacity";
const POSITION: &str = "day.position";
const BOUNDS: &str = "day.bounds";
const TRANSFORM: &str = "day.transform";
const BACKGROUND: &str = "day.background";

/// A geometry run in flight: both ends in Day's terms and as layer geometry, so what the
/// presentation layer shows can be read back as Day's frame and transform.
#[derive(Clone, Copy)]
struct Flight {
    day: (DayGeo, DayGeo),
    geo: (Geo, Geo),
}

day_core::tls_group! {
    /// View ptr → its geometry run (`move_geometry`). Read while the layer still has the
    /// run's animations; swept on release.
    static FLIGHTS: SideTable<Flight> = SideTable::new();
}

fn key(name: &str) -> Retained<NSString> {
    NSString::from_str(name)
}

/// Whether `layer` still plays the animation Day added under `name`.
fn running(layer: &CALayer, name: &str) -> bool {
    // SAFETY: a lookup by key; the animation returned is an immutable copy.
    unsafe { layer.animationForKey(&key(name)) }.is_some()
}

/// What the layer shows now: its presentation layer, or the model before the first commit.
fn shown(layer: &CALayer) -> Retained<CALayer> {
    // SAFETY: `presentationLayer` returns a fresh copy (or nil before the layer is committed).
    unsafe { layer.presentationLayer() }.unwrap_or_else(|| layer.retain())
}

fn geo_of(layer: &CALayer) -> Geo {
    let p = layer.position();
    let b = layer.bounds();
    let a = layer.anchorPoint();
    let t = layer.transform();
    Geo::read(
        (p.x, p.y),
        (b.size.width, b.size.height),
        (a.x, a.y),
        [t.m11, t.m12, t.m21, t.m22],
    )
}

/// The layer transform for `geo`: its 2D part, the rest of `base` (the model's) kept.
fn transform_of(geo: &Geo, base: CATransform3D) -> CATransform3D {
    let [m11, m12, m21, m22] = geo.matrix();
    CATransform3D {
        m11,
        m12,
        m21,
        m22,
        ..base
    }
}

/// Whether the running system's spring animator takes a damping ratio above 1.
fn overdamping() -> bool {
    CASpringAnimation::new().respondsToSelector(objc2::sel!(setAllowsOverdamping:))
}

fn number(v: f64) -> Retained<AnyObject> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "CALayer.opacity is a float"
    )]
    let v = v as f32;
    Retained::into_super(Retained::into_super(Retained::into_super(
        NSNumber::new_f32(v),
    )))
}

fn point_value(p: (f64, f64)) -> Retained<AnyObject> {
    // SAFETY: boxes a plain struct.
    let v = unsafe { objc2_foundation::NSValue::valueWithPoint(NSPoint::new(p.0, p.1)) };
    Retained::into_super(Retained::into_super(v))
}

fn rect_value(r: NSRect) -> Retained<AnyObject> {
    // SAFETY: boxes a plain struct.
    let v = unsafe { objc2_foundation::NSValue::valueWithRect(r) };
    Retained::into_super(Retained::into_super(v))
}

fn transform_value(t: CATransform3D) -> Retained<AnyObject> {
    // SAFETY: boxes a plain struct.
    let v = unsafe { objc2_foundation::NSValue::valueWithCATransform3D(t) };
    Retained::into_super(Retained::into_super(v))
}

fn cg_color(c: day_spec::Color) -> Retained<objc2_core_graphics::CGColor> {
    nscolor(c).CGColor()
}

fn color_value(c: day_spec::Color) -> Retained<AnyObject> {
    let cg = cg_color(c);
    // SAFETY: a CGColorRef is a Core Foundation object, which Objective-C retains, releases
    // and stores in an NSArray like any other object (toll-free bridging); Core Animation
    // takes `backgroundColor` values as CGColorRefs.
    unsafe { Retained::cast_unchecked::<AnyObject>(cg) }
}

/// The color a `CGColor` shows, in sRGB.
fn day_color(cg: &objc2_core_graphics::CGColor) -> Option<day_spec::Color> {
    let ns = NSColor::colorWithCGColor(cg)?;
    let srgb = ns.colorUsingColorSpace(&objc2_app_kit::NSColorSpace::sRGBColorSpace())?;
    Some(day_spec::Color {
        r: srgb.redComponent(),
        g: srgb.greenComponent(),
        b: srgb.blueComponent(),
        a: srgb.alphaComponent(),
    })
}

/// Adds the animation of `path` under `name` that plays `plan` with `spec`'s timing;
/// `value(progress)` is the property's value at a progress (0 the start, 1 the target).
fn add(
    layer: &CALayer,
    name: &str,
    path: &str,
    spec: &AnimSpec,
    plan: &Plan,
    value: &dyn Fn(f64) -> Retained<AnyObject>,
) {
    let path = NSString::from_str(path);
    let anim: Retained<CAAnimation> = match plan {
        Plan::Hold | Plan::Linear => {
            let a = CABasicAnimation::animationWithKeyPath(Some(&path));
            let end = if *plan == Plan::Hold { 0.0 } else { 1.0 };
            // SAFETY: values of the property's own type (`value` builds them for `path`).
            unsafe {
                a.setFromValue(Some(&value(0.0)));
                a.setToValue(Some(&value(end)));
            }
            Retained::into_super(Retained::into_super(a))
        }
        Plan::Spring { stiffness, damping } => {
            let a = CASpringAnimation::animationWithKeyPath(Some(&path));
            a.setMass(1.0);
            a.setStiffness(*stiffness);
            a.setDamping(*damping);
            a.setInitialVelocity(0.0);
            if a.respondsToSelector(objc2::sel!(setAllowsOverdamping:)) {
                a.setAllowsOverdamping(true);
            }
            // SAFETY: as above.
            unsafe {
                a.setFromValue(Some(&value(0.0)));
                a.setToValue(Some(&value(1.0)));
            }
            Retained::into_super(Retained::into_super(Retained::into_super(a)))
        }
        Plan::Keyframes(points) => {
            let a = CAKeyframeAnimation::animationWithKeyPath(Some(&path));
            let values: Vec<Retained<AnyObject>> = points.iter().map(|&(_, p)| value(p)).collect();
            let times: Vec<Retained<NSNumber>> =
                points.iter().map(|&(t, _)| NSNumber::new_f64(t)).collect();
            // SAFETY: values of the property's own type, one per key time.
            unsafe { a.setValues(Some(&NSArray::from_retained_slice(&values))) };
            a.setKeyTimes(Some(&NSArray::from_retained_slice(&times)));
            Retained::into_super(Retained::into_super(a))
        }
    };
    if *plan == Plan::Hold {
        anim.setDuration(spec.delay_secs());
    } else {
        anim.setDuration(spec.duration_secs());
        if spec.delay_ms > 0 {
            // The start shows through the delay: a backwards fill before a later begin time.
            let now = layer.convertTime_fromLayer(CACurrentMediaTime(), None);
            anim.setBeginTime(now + spec.delay_secs());
            // SAFETY: a constant Core Animation exports.
            anim.setFillMode(unsafe { kCAFillModeBackwards });
        }
        anim.setRepeatCount(repeat_count(spec));
        anim.setAutoreverses(spec.autoreverse);
    }
    layer.addAnimation_forKey(&anim, Some(&key(name)));
}

// ---------------------------------------------------------------------------
// The duties
// ---------------------------------------------------------------------------

/// `set_opacity`: the alpha at once, and the run from the opacity shown to it.
pub(super) fn set_opacity(v: &NSView, opacity: f64, anim: Option<&AnimSpec>) {
    v.setWantsLayer(true);
    let Some(layer) = v.layer() else {
        v.setAlphaValue(opacity);
        return;
    };
    let from = if running(&layer, OPACITY) {
        f64::from(shown(&layer).opacity())
    } else {
        f64::from(layer.opacity())
    };
    v.setAlphaValue(opacity);
    let plan = anim.and_then(|spec| Some((spec, plan(spec, true, false, overdamping())?)));
    match plan {
        Some((spec, plan)) if (from - opacity).abs() > 1e-6 => {
            add(&layer, OPACITY, "opacity", spec, &plan, &|p| {
                number(from + (opacity - from) * p)
            });
        }
        _ => layer.removeAnimationForKey(&key(OPACITY)),
    }
}

/// What a view's geometry shows now in Day's terms: the laid-out frame and the transform, read
/// back from the presentation layer while a run is in flight. `None` for a view Day never
/// placed (a navigation pane, a split view).
pub(super) fn shown_geometry(v: &NSView) -> Option<DayGeo> {
    let ptr = ptr_of(v);
    let model: DayGeo = (
        BASE_FRAMES.with(|m| m.get(ptr))?,
        TRANSFORMS
            .with(|m| m.get(ptr))
            .unwrap_or(Transform::IDENTITY),
    );
    let Some(layer) = v.layer() else {
        return Some(model);
    };
    if !running(&layer, POSITION) {
        return Some(model);
    }
    let Some(flight) = FLIGHTS.with(|m| m.get(ptr)) else {
        return Some(model);
    };
    let now = geo_of(&shown(&layer));
    Some(match progress(&flight.geo.0, &flight.geo.1, &now) {
        Some(p) => lerp_day(&flight.day.0, &flight.day.1, p),
        None => flight.day.1,
    })
}

/// A geometry change (`set_frame`, `set_transform`): `apply` puts the model at `to` at once;
/// with `anim`, the layer then plays from what was shown (`from`, read with
/// [`shown_geometry`] before the change) to it. Without, any run is removed.
pub(super) fn move_geometry(
    v: &NSView,
    from: Option<DayGeo>,
    to: DayGeo,
    anim: Option<&AnimSpec>,
    apply: impl FnOnce(),
) {
    let ptr = ptr_of(v);
    if anim.is_some() && v.layer().is_none() {
        // A view outside a layer-backed subtree has no layer to animate until it asks for one.
        v.setWantsLayer(true);
    }
    let layer = anim.and_then(|_| v.layer());
    let (Some(spec), Some(layer), Some(from)) = (anim, layer, from) else {
        if let Some(layer) = v.layer() {
            stop_geometry(&layer);
        }
        let _ = FLIGHTS.with(|m| m.remove(ptr));
        apply();
        return;
    };
    let start = if running(&layer, POSITION) {
        geo_of(&shown(&layer))
    } else {
        geo_of(&layer)
    };
    apply();
    let mut end = geo_of(&layer);
    end.angle = unwind(
        start.angle,
        end.angle,
        (to.1.rotation - from.1.rotation).degrees(),
    );
    let plan = plan(spec, start.same_shape(&end), false, overdamping());
    stop_geometry(&layer);
    let Some(plan) = plan.filter(|_| !start.near(&end)) else {
        let _ = FLIGHTS.with(|m| m.remove(ptr));
        return;
    };
    let anchor = layer.anchorPoint();
    let anchor = (anchor.x, anchor.y);
    let bounds = layer.bounds();
    let model = layer.transform();
    add(&layer, POSITION, "position", spec, &plan, &|p| {
        point_value(start.lerp(&end, p).position(anchor))
    });
    add(&layer, BOUNDS, "bounds", spec, &plan, &|p| {
        let g = start.lerp(&end, p);
        rect_value(NSRect::new(bounds.origin, NSSize::new(g.size.0, g.size.1)))
    });
    if !start.same_shape(&end) {
        add(&layer, TRANSFORM, "transform", spec, &plan, &|p| {
            transform_value(transform_of(&start.lerp(&end, p), model))
        });
    }
    FLIGHTS.with(|m| {
        m.insert(
            ptr,
            Flight {
                day: (from, to),
                geo: (start, end),
            },
        )
    });
}

fn stop_geometry(layer: &CALayer) {
    for name in [POSITION, BOUNDS, TRANSFORM] {
        layer.removeAnimationForKey(&key(name));
    }
}

/// The fill a container shows now: the layer's while a run plays it, else the one `drawRect`
/// paints, its ink resolved for the view's appearance. `None` for a container with no solid
/// fill.
fn shown_background(v: &DayFlipped) -> Option<day_spec::Color> {
    let model = v.ivars().surface.borrow().as_ref()?.solid()?;
    let model = color::color_for(v, model);
    if v.ivars().layer_fill.get()
        && let Some(layer) = v.layer()
        && running(&layer, BACKGROUND)
        && let Some(cg) = shown(&layer).backgroundColor()
    {
        return day_color(&cg).or(Some(model));
    }
    Some(model)
}

/// `ContainerPatch::Fill`: the fill at once, and with `anim` the run from the solid fill shown
/// to the new solid one, resolved for the view's appearance (`drawRect` takes the ink back at
/// the end, so it follows a later switch). A fill appearing or going away, or a gradient at
/// either end, applies at once.
pub(super) fn set_background(
    v: &DayFlipped,
    fill: Option<day_spec::Paint>,
    anim: Option<&AnimSpec>,
) {
    let from = shown_background(v);
    let to = match fill {
        Some(day_spec::Paint::Solid(ink)) => Some(color::color_for(v, ink)),
        _ => None,
    };
    v.set_background(fill);
    let run = match (anim, from, to) {
        (Some(spec), Some(from), Some(to)) if from != to => {
            plan(spec, true, true, false).map(|plan| (spec, plan, from, to))
        }
        _ => None,
    };
    match run {
        Some((spec, plan, from, to)) => begin_fill(v, spec, &plan, from, to),
        None => end_fill(v),
    }
}

/// Moves the fill to the layer and plays it from `from` to `to`; the completion hands it back
/// to `drawRect` unless a newer run or a plain set took over first.
fn begin_fill(
    v: &DayFlipped,
    spec: &AnimSpec,
    plan: &Plan,
    from: day_spec::Color,
    to: day_spec::Color,
) {
    v.setWantsLayer(true);
    let Some(layer) = v.layer() else {
        end_fill(v);
        return;
    };
    let run = v.ivars().fill_run.get().wrapping_add(1);
    v.ivars().fill_run.set(run);
    v.ivars().layer_fill.set(true);
    let corners = v
        .ivars()
        .surface
        .borrow()
        .as_ref()
        .map(|s| (s.radius, s.corners));
    if let Some((radius, corners)) = corners.filter(|(r, _)| *r > 0.0) {
        // The layer's own background follows the corner radius (its contents do not), so the
        // fill keeps the outline `drawRect` gives it.
        layer.setCornerRadius(radius);
        layer.setMaskedCorners(corner_mask(corners));
    }
    layer.setBackgroundColor(Some(&cg_color(to)));
    // `drawRect` leaves the fill out while the layer carries it.
    NSView::setNeedsDisplay(v, true);
    let weak = Weak::from_retained(&v.retain());
    let done = RcBlock::new(move || {
        if let Some(v) = weak.load()
            && v.ivars().fill_run.get() == run
        {
            ffi_guard::contain((), || end_fill(&v));
        }
    });
    CATransaction::begin();
    // SAFETY: the block lives as long as the transaction needs it (Core Animation copies it).
    unsafe { CATransaction::setCompletionBlock(Some(&done)) };
    add(&layer, BACKGROUND, "backgroundColor", spec, plan, &|p| {
        color_value(lerp_color(from, to, p))
    });
    CATransaction::commit();
}

/// Hands the fill back to `drawRect` and stops a run that plays it.
fn end_fill(v: &DayFlipped) {
    let iv = v.ivars();
    iv.fill_run.set(iv.fill_run.get().wrapping_add(1));
    if !iv.layer_fill.replace(false) {
        return;
    }
    if let Some(layer) = v.layer() {
        layer.removeAnimationForKey(&key(BACKGROUND));
        layer.setBackgroundColor(None);
    }
    NSView::setNeedsDisplay(v, true);
}

/// `read_displayed`: the presentation layer's opacity, the geometry read back as Day's frame
/// (in window content coordinates, top left, like `read_native`) and translation, and the
/// fill a container shows.
pub(super) fn displayed(v: &NSView) -> day_spec::Displayed {
    let opacity = match v.layer() {
        Some(layer) if running(&layer, OPACITY) => f64::from(shown(&layer).opacity()),
        _ => v.alphaValue(),
    };
    let (frame, translation) = match shown_geometry(v) {
        Some(([x, y, w, h], t)) => (
            // SAFETY: reads the view's current superview, which the hierarchy retains.
            unsafe { v.superview() }.and_then(|sup| {
                content_frame(&sup, NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)))
            }),
            Point::new(t.tx, t.ty),
        ),
        None => (native_frame(v), Point::ZERO),
    };
    day_spec::Displayed {
        opacity: Some(opacity),
        frame,
        translation: Some(translation),
        background: v.downcast_ref::<DayFlipped>().and_then(shown_background),
    }
}

/// A rect in `sup`'s coordinates in the window content's, origin top left.
fn content_frame(sup: &NSView, r: NSRect) -> Option<Rect> {
    let content = sup.window()?.contentView()?;
    let r = sup.convertRect_toView(r, Some(&content));
    let y = if content.isFlipped() {
        r.origin.y
    } else {
        let b = content.bounds();
        b.origin.y + b.size.height - (r.origin.y + r.size.height)
    };
    Some(Rect::new(r.origin.x, y, r.size.width, r.size.height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn repeat_counts_follow_core_animation() {
        assert!((repeat_count(&AnimSpec::linear(800)) - 1.0).abs() < 1e-6);
        assert!((repeat_count(&AnimSpec::linear(800).repeat(2, false)) - 3.0).abs() < 1e-6);
        // A forward and a backward pass are one repeat: three cycles are one and a half.
        assert!((repeat_count(&AnimSpec::linear(800).repeat(2, true)) - 1.5).abs() < 1e-6);
        assert!((repeat_count(&AnimSpec::linear(800).repeat(1, true)) - 1.0).abs() < 1e-6);
        assert!(repeat_count(&AnimSpec::linear(600).repeat_forever(true)).is_infinite());
        assert!(repeat_count(&AnimSpec::linear(600).repeat_forever(false)).is_infinite());
    }

    #[test]
    fn plans_pick_the_native_animation_where_it_is_exact() {
        assert_eq!(plan(&AnimSpec::linear(0), true, false, true), None);
        assert_eq!(
            plan(&AnimSpec::linear(0).delay(200), true, false, true),
            Some(Plan::Hold)
        );
        assert_eq!(
            plan(&AnimSpec::linear(300), true, false, true),
            Some(Plan::Linear)
        );
        let spring = AnimSpec::spring(0.9, 0.25);
        let (k, c) = spring.curve.spring_constants().unwrap();
        assert_eq!(
            plan(&spring, true, false, false),
            Some(Plan::Spring {
                stiffness: k,
                damping: c
            })
        );
        // A turning view, a color (clamped) and an over-damped spring on an old system play
        // the keyframes.
        for p in [
            plan(&spring, false, false, true),
            plan(&spring, true, true, true),
            plan(&AnimSpec::spring(0.5, 1.5), true, false, false),
            plan(&AnimSpec::ease_in(300), true, false, true),
            plan(&AnimSpec::linear(300), false, false, true),
        ] {
            assert!(matches!(p, Some(Plan::Keyframes(_))), "{p:?}");
        }
        assert!(matches!(
            plan(&AnimSpec::spring(0.5, 1.5), true, false, true),
            Some(Plan::Spring { .. })
        ));
        let Some(Plan::Keyframes(points)) = plan(&spring, true, true, true) else {
            panic!("a color plays keyframes");
        };
        assert!(points.iter().all(|&(_, p)| (0.0..=1.0).contains(&p)));
        assert!(near(points.last().unwrap().1, 1.0));
        let Some(Plan::Keyframes(points)) = plan(&spring, false, false, true) else {
            panic!("a turning spring plays keyframes");
        };
        assert!(
            points.iter().any(|&(_, p)| p > 1.3),
            "the spring overshoots"
        );
    }

    /// The layer geometry AppKit gave a view placed by `apply_geometry` (measured on macOS 26
    /// in `tests/native_motion.rs`): a 120×30 frame at (50, 60), moved 7 right, scaled 1.5 wide
    /// and turned 20°.
    #[test]
    fn geometry_round_trips_through_the_layer() {
        let m = [
            1.409_538_931_178_858,
            0.513_030_214_988_504,
            -0.342_020_143_325_669_3,
            0.939_692_620_785_905_3,
        ];
        let position = (65.748_744_902_730_52, 40.383_402_088_671_254);
        let geo = Geo::read(position, (120.0, 30.0), (0.0, 0.0), m);
        assert!(near(geo.scale.0, 1.5) && near(geo.scale.1, 1.0));
        assert!(near(geo.angle.to_degrees(), 20.0));
        let back = geo.position((0.0, 0.0));
        assert!(near(back.0, position.0) && near(back.1, position.1));
        for (a, b) in geo.matrix().into_iter().zip(m) {
            assert!((a - b).abs() < 1e-9);
        }
        // The same view with a centered anchor has the same center.
        let centered = Geo::read(geo.position((0.5, 0.5)), (120.0, 30.0), (0.5, 0.5), m);
        assert!(near(centered.center.0, geo.center.0) && near(centered.center.1, geo.center.1));
    }

    #[test]
    fn progress_reads_the_part_that_moves_most() {
        let plain = |x: f64, w: f64| Geo {
            center: (x + w / 2.0, 20.0),
            size: (w, 40.0),
            scale: (1.0, 1.0),
            angle: 0.0,
        };
        let (from, to) = (plain(0.0, 80.0), plain(0.0, 240.0));
        let p = progress(&from, &to, &from.lerp(&to, 0.25)).unwrap();
        assert!(near(p, 0.25));
        // A spring past its target reads more than 1.
        let p = progress(&from, &to, &from.lerp(&to, 1.44)).unwrap();
        assert!(near(p, 1.44));
        assert_eq!(progress(&from, &from, &from), None);
        assert!(from.near(&from.lerp(&from, 0.5)));
        let turned = Geo { angle: 1.0, ..from };
        assert!(near(
            progress(&from, &turned, &from.lerp(&turned, 0.5)).unwrap(),
            0.5
        ));
    }

    #[test]
    fn a_full_turn_spins() {
        assert!(near(unwind(0.0, 0.0, 360.0), std::f64::consts::TAU));
        assert!(
            near(unwind(0.0, 0.5, 30.0), 0.5),
            "disagreement keeps the read angle"
        );
        let to = (-170.0_f64).to_radians();
        assert!(near(
            unwind(170.0_f64.to_radians(), to, 20.0),
            190.0_f64.to_radians()
        ));
    }

    #[test]
    fn day_geometry_and_colors_blend() {
        let a = ([0.0, 0.0, 80.0, 40.0], Transform::IDENTITY);
        let b = ([10.0, 0.0, 240.0, 40.0], Transform::translate(100.0, 0.0));
        let ([x, _, w, _], t) = lerp_day(&a, &b, 0.5);
        assert!(near(x, 5.0) && near(w, 160.0) && near(t.tx, 50.0));
        let gray = lerp_color(day_spec::Color::BLACK, day_spec::Color::WHITE, 0.25);
        assert!(near(gray.r, 0.25) && near(gray.g, 0.25) && near(gray.b, 0.25));
    }
}
