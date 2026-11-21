// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Frame-driven motion (docs/tween.md): values that animate, the animations that move them, and
//! the math underneath.
//!
//! [`with_animation`](crate::with_animation) hands a change to the platform's animator. This
//! module is the other path: Day steps the numbers itself on the window's
//! [`FrameClock`] and views or a canvas read them. It has three layers,
//! each usable without the one above:
//!
//! * [`Animated`]: a reactive value with a velocity. A `.opacity(..)`, a `.translation(..)` or
//!   a canvas reading it follows its motion, and it is the one animated-value type in Day.
//! * [`Animation`]: what moves values. [`timing`](Animation::timing) along an [`Easing`],
//!   [`tween`](Animation::tween) under an [`AnimSpec`], [`spring`](Animation::spring) and
//!   [`decay`](Animation::decay) with the value's own velocity, composed with
//!   [`sequence`](Animation::sequence), [`parallel`](Animation::parallel),
//!   [`stagger`](Animation::stagger), [`repeat`](Animation::repeat) and
//!   [`looped`](Animation::looped). [`start`](Animation::start) answers a [`Playing`] handle
//!   that stops it, resets it, and can be awaited.
//! * The math: [`Easing`], [`Spring`] with [`SpringState`], [`Decay`] and [`Interpolation`],
//!   plain allocation-free values an app can also step itself from a frame callback.
//!
//! Reduced motion and scope cleanup are handled here once: a finite animation lands on its end
//! state when the user asked for less motion, and an animation stops when a value it moves is
//! disposed with its scope.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use day_reactive::Signal;
use day_spec::{AnimSample, AnimSpec, Curve};

use crate::frame::{FrameClock, FrameHandle};
use crate::tween::Lerp;

use std::f64::consts::PI;

/// Which end of a curve is shaped: the start, the end, or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ease {
    /// Slow at the start.
    In,
    /// Slow at the end.
    Out,
    /// Slow at both ends.
    InOut,
}

/// An easing curve: a function from progress (0 to 1) to eased progress. Back and elastic
/// curves leave that range on purpose, which is the overshoot they exist to show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Easing {
    /// Progress unchanged.
    Linear,
    /// A CSS-style cubic Bézier through (0, 0) and (1, 1) with these two control points.
    Bezier(f64, f64, f64, f64),
    /// A power curve: 2 is quadratic, 3 cubic.
    Power(Ease, f64),
    /// A quarter of a sine wave.
    Sine(Ease),
    /// A quarter of a circle.
    Circle(Ease),
    /// An exponential curve.
    Exponential(Ease),
    /// A curve that backs up past its start or runs past its end by `overshoot`
    /// (1.70158 overshoots by a tenth).
    Back(Ease, f64),
    /// A spring-like wobble around the end, `bounciness` 1 for the classic curve.
    ElasticOut(f64),
    /// A ball dropped on the end value.
    BounceOut,
}

impl Easing {
    /// The CSS `ease` curve.
    pub const EASE: Easing = Easing::Bezier(0.25, 0.1, 0.25, 1.0);
    /// The CSS `ease-in-out` curve.
    pub const EASE_IN_OUT: Easing = Easing::Bezier(0.42, 0.0, 0.58, 1.0);

    /// The eased value at progress `t`, which is clamped to 0..=1 first.
    #[must_use]
    pub fn value(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::Bezier(x1, y1, x2, y2) => bezier(t, x1, y1, x2, y2),
            Easing::Power(ease, power) => shaped(ease, t, |t| t.powf(power)),
            Easing::Sine(ease) => shaped(ease, t, |t| 1.0 - (t * PI / 2.0).cos()),
            Easing::Circle(ease) => shaped(ease, t, |t| 1.0 - (1.0 - t * t).max(0.0).sqrt()),
            Easing::Exponential(ease) => shaped(ease, t, |t| {
                if t <= 0.0 {
                    0.0
                } else {
                    2f64.powf(10.0 * (t - 1.0))
                }
            }),
            Easing::Back(ease, overshoot) => {
                shaped(ease, t, |t| t * t * ((overshoot + 1.0) * t - overshoot))
            }
            Easing::ElasticOut(bounciness) => {
                if t <= 0.0 || t >= 1.0 {
                    t
                } else {
                    let p = bounciness.max(0.01) * PI;
                    1.0 - (t * PI / 2.0).cos().powi(3) * (t * p).cos()
                }
            }
            Easing::BounceOut => bounce(t),
        }
    }

    /// The curve sampled at `count` evenly spaced points from 0 to 1, for plotting it.
    #[must_use]
    pub fn samples(self, count: usize) -> Vec<f64> {
        let last = count.saturating_sub(1).max(1) as f64;
        (0..count).map(|i| self.value(i as f64 / last)).collect()
    }
}

/// An ease-in function turned into the asked-for mode.
fn shaped(ease: Ease, t: f64, ease_in: impl Fn(f64) -> f64) -> f64 {
    match ease {
        Ease::In => ease_in(t),
        Ease::Out => 1.0 - ease_in(1.0 - t),
        Ease::InOut => {
            if t < 0.5 {
                ease_in(t * 2.0) / 2.0
            } else {
                1.0 - ease_in((1.0 - t) * 2.0) / 2.0
            }
        }
    }
}

fn bounce(t: f64) -> f64 {
    const N: f64 = 7.5625;
    const D: f64 = 2.75;
    if t < 1.0 / D {
        N * t * t
    } else if t < 2.0 / D {
        let t = t - 1.5 / D;
        N * t * t + 0.75
    } else if t < 2.5 / D {
        let t = t - 2.25 / D;
        N * t * t + 0.9375
    } else {
        let t = t - 2.625 / D;
        N * t * t + 0.984_375
    }
}

/// The y of a unit cubic Bézier at the x `x`, solved by Newton's method with a bisection
/// fallback, as browsers evaluate `cubic-bezier()`.
fn bezier(x: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let axis = |t: f64, a: f64, b: f64| {
        let u = 1.0 - t;
        3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
    };
    let slope = |t: f64, a: f64, b: f64| {
        let u = 1.0 - t;
        3.0 * u * u * a + 6.0 * u * t * (b - a) + 3.0 * t * t * (1.0 - b)
    };
    let mut t = x;
    for _ in 0..8 {
        let error = axis(t, x1, x2) - x;
        if error.abs() < 1e-6 {
            return axis(t, y1, y2);
        }
        let d = slope(t, x1, x2);
        if d.abs() < 1e-6 {
            break;
        }
        t = (t - error / d).clamp(0.0, 1.0);
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    t = x;
    for _ in 0..24 {
        let at = axis(t, x1, x2);
        if (at - x).abs() < 1e-6 {
            break;
        }
        if at < x {
            lo = t;
        } else {
            hi = t;
        }
        t = f64::midpoint(lo, hi);
    }
    axis(t, y1, y2)
}

/// A damped spring: what pulls a value toward its target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    /// How hard it pulls. Higher is faster.
    pub stiffness: f64,
    /// How much it resists motion. Lower overshoots more.
    pub damping: f64,
    /// The weight it moves. Higher is slower and overshoots more.
    pub mass: f64,
}

impl Default for Spring {
    /// A spring that settles without visible overshoot: stiffness 170, damping 26, mass 1.
    fn default() -> Self {
        Spring::new(170.0, 26.0)
    }
}

impl Spring {
    /// A spring of mass 1 with this stiffness and damping.
    #[must_use]
    pub const fn new(stiffness: f64, damping: f64) -> Self {
        Spring {
            stiffness,
            damping,
            mass: 1.0,
        }
    }

    /// The spring with another mass.
    #[must_use]
    pub const fn mass(mut self, mass: f64) -> Self {
        self.mass = mass;
        self
    }

    /// The spring an [`AnimSpec::spring`] of this `response` (the period of the undamped
    /// oscillation, in seconds) and damping `ratio` describes, with a mass of 1: the same
    /// motion on the frame-driven path as a native spring animation of that spec.
    #[must_use]
    pub fn from_response(response: f64, ratio: f64) -> Self {
        let (stiffness, damping) = Curve::Spring {
            response,
            damping: ratio,
        }
        .spring_constants()
        .unwrap_or((170.0, 26.0));
        Spring::new(stiffness, damping)
    }

    /// The spring as an animation curve: its response and damping ratio, for
    /// [`AnimSpec`]. `AnimSpec::spring(response, ratio)` with these two is the native
    /// animation that matches this spring released at rest.
    #[must_use]
    pub fn curve(self) -> Curve {
        let mass = self.mass.max(1e-6);
        let omega0 = (self.stiffness.max(0.0) / mass).sqrt().max(1e-6);
        Curve::Spring {
            response: std::f64::consts::TAU / omega0,
            damping: self.damping / (2.0 * mass * omega0),
        }
    }

    /// Advances `state` toward `target` by `dt` seconds. A long frame is split into steps of at
    /// most a millisecond, so a stiff spring stays stable when the display stalls.
    pub fn step(self, state: &mut SpringState, target: f64, dt: f64) {
        let mut left = dt.clamp(0.0, 0.25);
        let mass = self.mass.max(1e-6);
        while left > 0.0 {
            let h = left.min(0.001);
            let force = -self.stiffness * (state.value - target) - self.damping * state.velocity;
            state.velocity += force / mass * h;
            state.value += state.velocity * h;
            left -= h;
        }
    }

    /// Whether `state` has come to rest on `target`: both its distance and its speed are below
    /// what a display can show.
    #[must_use]
    pub fn settled(self, state: SpringState, target: f64) -> bool {
        (state.value - target).abs() < 0.01 && state.velocity.abs() < 0.01
    }

    /// The spring's answer to a step from 0 to 1, sampled at `count` evenly spaced times over
    /// `seconds`, for plotting it.
    #[must_use]
    pub fn response(self, seconds: f64, count: usize) -> Vec<f64> {
        let mut state = SpringState::default();
        let dt = seconds / count.saturating_sub(1).max(1) as f64;
        (0..count)
            .map(|i| {
                if i > 0 {
                    self.step(&mut state, 1.0, dt);
                }
                state.value
            })
            .collect()
    }
}

/// Where a sprung value is and how fast it is moving, kept between steps.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpringState {
    /// The value now.
    pub value: f64,
    /// Its speed, in units per second.
    pub velocity: f64,
}

impl SpringState {
    /// A value at rest.
    #[must_use]
    pub const fn at(value: f64) -> Self {
        SpringState {
            value,
            velocity: 0.0,
        }
    }
}

/// A fling that coasts: a velocity that keeps `rate` of itself every millisecond, as a scroll
/// view decelerates. 0.998 is the platforms' usual scroll rate; lower stops sooner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decay {
    /// The share of the velocity kept per millisecond, below 1.
    pub rate: f64,
}

impl Decay {
    /// Advances `state` by `dt` seconds: the value moves by the distance covered while the
    /// velocity fell.
    pub fn step(self, state: &mut SpringState, dt: f64) {
        let ms = (dt * 1000.0).clamp(0.0, 250.0);
        let rate = self.rate.clamp(0.0, 0.999_999);
        let k = rate.ln();
        let factor = rate.powf(ms);
        // The integral of v·rate^t over the step, in seconds.
        state.value += state.velocity * (factor - 1.0) / k / 1000.0;
        state.velocity *= factor;
    }

    /// Whether the fling has stopped: its speed is below a point per second.
    #[must_use]
    pub fn stopped(self, state: SpringState) -> bool {
        state.velocity.abs() < 1.0
    }

    /// Where a fling from `state` comes to rest: the distance left is the velocity over the
    /// share lost per second.
    #[must_use]
    pub fn rest(self, state: SpringState) -> f64 {
        let k = self.rate.clamp(0.0, 0.999_999).ln();
        state.value - state.velocity / k / 1000.0
    }
}

// ---------------------------------------------------------------------------
// Interpolation: one range mapped onto another
// ---------------------------------------------------------------------------

/// A piecewise-linear map from an input range onto an output range: `[0, 120]` of scroll onto
/// `[1, 0]` of opacity, or onto two colors.
///
/// The output is anything with a [`Lerp`]: numbers, points, rects, and colors, which blend in
/// OKLab. Outside the input range the map clamps to the nearest end unless
/// [`extend`](Interpolation::extend) lets the outer segments run on.
///
/// ```rust,ignore
/// let fade = Interpolation::new([0.0, 120.0], [1.0, 0.0]);
/// assert_eq!(fade.at(60.0), 0.5);
/// let heat = Interpolation::new([0.0, 0.5, 1.0], [BLUE, WHITE, RED]);
/// ```
///
/// [`Animated::interpolate`] is the reactive form, for a modifier that follows a value.
#[derive(Clone, Debug, PartialEq)]
pub struct Interpolation<T: Lerp> {
    /// The first stop, which every map has.
    first: (f64, T),
    /// The stops after it, by ascending input.
    rest: Vec<(f64, T)>,
    /// Whether the first segment runs on below the range.
    extend_start: bool,
    /// Whether the last segment runs on above the range.
    extend_end: bool,
}

impl<T: Lerp> Interpolation<T> {
    /// The map through `input[i]` → `output[i]`. The inputs ascend; a map of one stop is
    /// constant. An empty map does not compile.
    #[must_use]
    pub fn new<const N: usize>(input: [f64; N], output: [T; N]) -> Self {
        const { assert!(N >= 1, "an interpolation needs at least one stop") };
        let mut stops = input.into_iter().zip(output);
        let Some(first) = stops.next() else {
            unreachable!("N >= 1 is asserted at compile time")
        };
        Interpolation {
            first,
            rest: stops.collect(),
            extend_start: false,
            extend_end: false,
        }
    }

    /// Lets both outer segments run on past the range instead of clamping. A color's channels
    /// still stay in range.
    #[must_use]
    pub fn extend(mut self) -> Self {
        self.extend_start = true;
        self.extend_end = true;
        self
    }

    /// Lets the first segment run on below the range.
    #[must_use]
    pub fn extend_start(mut self) -> Self {
        self.extend_start = true;
        self
    }

    /// Lets the last segment run on above the range.
    #[must_use]
    pub fn extend_end(mut self) -> Self {
        self.extend_end = true;
        self
    }

    /// The output at `x`.
    #[must_use]
    pub fn at(&self, x: f64) -> T {
        let mut lo = &self.first;
        let Some(mut hi) = self.rest.first() else {
            return lo.1.clone();
        };
        if x < lo.0 && !self.extend_start {
            return lo.1.clone();
        }
        // The segment holding `x`: the last one when `x` is past the end.
        for stop in self.rest.iter().skip(1) {
            if x <= hi.0 {
                break;
            }
            lo = hi;
            hi = stop;
        }
        if x > hi.0 && !self.extend_end {
            return hi.1.clone();
        }
        let span = hi.0 - lo.0;
        if span <= 0.0 {
            return if x < lo.0 { lo.1.clone() } else { hi.1.clone() };
        }
        lo.1.lerp(&hi.1, (x - lo.0) / span)
    }
}

// ---------------------------------------------------------------------------
// Animated: a value that moves
// ---------------------------------------------------------------------------

/// What an [`Animated`] keeps beside its value: bookkeeping no view reads.
struct Slot<T> {
    /// Where the value is headed, or the value itself at rest.
    target: T,
    /// Counts every animation step and every `set` that took the value over. A step whose
    /// number is no longer current has been displaced and stops writing.
    claim: u64,
    /// The flight `animate_to` started, to stop when the next one begins.
    flight: Option<Playing>,
    /// The window whose frames move the value.
    clock: FrameClock,
}

/// A reactive value that animations move: a number with its velocity, or anything else with a
/// [`Lerp`] (a color, a point, a rect, a `Vec` of them).
///
/// Reading it is a tracked read, so whatever reads it follows the motion and nothing else
/// re-runs. It goes straight into any modifier that takes a reactive value:
///
/// ```rust,ignore
/// let fade = Animated::new(1.0);
/// let lift = Animated::new(0.0);
/// label(res::str::title())
///     .opacity(fade)
///     .translation(0.0, lift);
/// button(res::str::hide()).action(move || {
///     Animation::parallel([
///         Animation::timing(fade, 0.0, 220, Easing::EASE_IN_OUT),
///         Animation::spring(lift, -24.0, Spring::new(320.0, 22.0)),
///     ])
///     .start();
/// });
/// ```
///
/// The value is `Copy`, like a `Signal`, and lives in the reactive scope it was made in: when
/// that scope is disposed, animations moving it stop.
///
/// One owner at a time: the latest animation step or [`set`](Animated::set) to touch a value
/// owns it, and an earlier step on the same value ends without moving it further. A spring
/// that takes over reads the velocity the displaced step left, which is what makes a retarget
/// bend the motion.
pub struct Animated<T: Lerp + 'static = f64> {
    /// The value on screen.
    value: Signal<T>,
    /// Its speed in units per second. Kept for numbers; zero for everything else.
    velocity: Signal<f64>,
    /// Whether an animation is moving it.
    moving: Signal<bool>,
    /// The bookkeeping.
    slot: Signal<Slot<T>>,
}

#[expect(
    clippy::expl_impl_clone_on_copy,
    reason = "a derive would demand `T: Copy`, and the handle is `Copy` whatever it holds"
)]
impl<T: Lerp + 'static> Clone for Animated<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: Lerp + 'static> Copy for Animated<T> {}

impl<T: Lerp + std::fmt::Debug + 'static> std::fmt::Debug for Animated<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Animated")
            .field("value", &self.try_now())
            .finish_non_exhaustive()
    }
}

impl<T: Lerp + 'static> Animated<T> {
    /// A value at rest on `initial`, animating on the current page's window.
    #[must_use]
    pub fn new(initial: T) -> Self {
        Self::on(FrameClock::current(), initial)
    }

    /// A value at rest on `initial`, animating on `clock`'s window.
    #[must_use]
    pub fn on(clock: FrameClock, initial: T) -> Self {
        Animated {
            value: Signal::new(initial.clone()),
            velocity: Signal::new(0.0),
            moving: Signal::new(false),
            slot: Signal::new(Slot {
                target: initial,
                claim: 0,
                flight: None,
                clock,
            }),
        }
    }

    /// The value on screen now; tracked.
    #[must_use]
    pub fn get(self) -> T {
        self.value.get()
    }

    /// The value on screen now, without subscribing.
    #[must_use]
    pub fn get_untracked(self) -> T {
        self.value.get_untracked()
    }

    /// Where the value is headed (the value itself once it has arrived).
    #[must_use]
    pub fn target(self) -> T {
        self.slot.with_untracked(|slot| slot.target.clone())
    }

    /// Whether an animation is moving the value; tracked, so a control can disable itself
    /// while it is.
    #[must_use]
    pub fn is_animating(self) -> bool {
        self.moving.get()
    }

    /// Puts the value at rest on `value`, taking it from any animation moving it.
    pub fn set(self, value: T) {
        self.claim(Some(value.clone()));
        if let Some(flight) = self.take_flight() {
            flight.stop();
        }
        self.value.set(value);
        self.velocity.set_if_changed(0.0);
        self.moving.set_if_changed(false);
    }

    /// Animates from the value on screen to `target` under `spec`: its delay, curve or spring,
    /// repeats and autoreverse, as [`AnimSpec::sample`] defines them. A call while an earlier
    /// one is in flight starts from the value on screen, so the motion bends instead of
    /// jumping. Shorthand for starting [`Animation::tween`].
    pub fn animate_to(self, target: T, spec: AnimSpec) -> Playing {
        if let Some(flight) = self.take_flight() {
            flight.stop();
        }
        let playing = Animation::tween(self, target, spec).start();
        if !playing.is_finished() {
            let flight = playing.clone();
            self.slot.update(|slot| slot.flight = Some(flight));
        }
        playing
    }

    /// A reactive read of the value mapped through `input` → `output`
    /// ([`Interpolation`]), clamped at both ends: a closure for any modifier that takes a
    /// reactive value. Colors blend in OKLab.
    ///
    /// ```rust,ignore
    /// let scroll = Animated::new(0.0);
    /// header
    ///     .opacity(scroll.interpolate([0.0, 120.0], [1.0, 0.0]))
    ///     .background(scroll.interpolate([0.0, 120.0], [Color::WHITE, accent]));
    /// ```
    pub fn interpolate<U: Lerp + 'static, const N: usize>(
        self,
        input: [f64; N],
        output: [U; N],
    ) -> impl Fn() -> U + Clone + 'static
    where
        T: Into<f64>,
    {
        self.interpolate_with(Interpolation::new(input, output))
    }

    /// A reactive read of the value mapped through `map`, for a map that extends past its
    /// ends or is shared.
    pub fn interpolate_with<U: Lerp + 'static>(
        self,
        map: Interpolation<U>,
    ) -> impl Fn() -> U + Clone + 'static
    where
        T: Into<f64>,
    {
        let map = Rc::new(map);
        move || map.at(self.get().into())
    }

    /// The value, when its scope is still alive; untracked.
    fn try_now(self) -> Option<T> {
        day_reactive::untrack(|| self.value.try_get())
    }

    /// Takes the value over and answers the new claim number. `target` also records where
    /// it is now headed.
    fn claim(self, target: Option<T>) -> u64 {
        let mut claim = 0;
        self.slot.update(|slot| {
            slot.claim += 1;
            if let Some(target) = target {
                slot.target = target;
            }
            claim = slot.claim;
        });
        claim
    }

    /// The flight `animate_to` last started, taken out of the slot.
    fn take_flight(self) -> Option<Playing> {
        let mut flight = None;
        self.slot.update(|slot| flight = slot.flight.take());
        flight
    }

    /// The window whose frames move the value, while its scope is alive.
    fn clock(self) -> Option<FrameClock> {
        day_reactive::untrack(|| self.slot.try_with(|slot| slot.clock))
    }
}

impl Animated<f64> {
    /// The value's speed in units per second; tracked. A spring and a decay start from it.
    #[must_use]
    pub fn velocity(self) -> f64 {
        self.velocity.get()
    }

    /// Gives the value a speed without moving it: the velocity a finger released it with,
    /// before a [`spring`](Animation::spring) or a [`decay`](Animation::decay) takes it on.
    pub fn set_velocity(self, velocity: f64) {
        self.velocity.set(velocity);
    }
}

/// One animation step's hold on a value: it writes while its claim is current.
struct Hold<T: Lerp + 'static> {
    /// The value held.
    value: Animated<T>,
    /// The claim this step made.
    claim: u64,
}

impl<T: Lerp + 'static> Hold<T> {
    /// Takes `value` for a step headed to `target`, starting from where it stands. `None`
    /// when the value's scope is gone.
    fn take(value: Animated<T>, target: Option<T>) -> Option<(Self, T)> {
        let from = value.try_now()?;
        let claim = value.claim(target);
        value.moving.set_if_changed(true);
        Some((Hold { value, claim }, from))
    }

    /// Whether the step still owns the value: `Some(true)` to write, `Some(false)` when
    /// displaced, `None` when the value is gone.
    fn owns(&self) -> Option<bool> {
        day_reactive::untrack(|| self.value.slot.try_with(|slot| slot.claim == self.claim))
    }

    /// Writes the value mid-flight, keeping a number's velocity from the change.
    fn write(&self, next: T, dt: f64) {
        if dt > 0.0
            && let Some(next) = (&next as &dyn Any).downcast_ref::<f64>()
            && let Some(prev) = self
                .value
                .try_now()
                .as_ref()
                .and_then(|p| (p as &dyn Any).downcast_ref::<f64>().copied())
        {
            self.value.velocity.set((next - prev) / dt);
        }
        self.value.value.set(next);
    }

    /// Puts the value at rest on `end` and gives it up.
    fn land(&self, end: T) {
        self.value.value.set(end);
        self.value.velocity.set_if_changed(0.0);
        self.release();
    }

    /// Gives the value up where it stands, if the step still owns it.
    fn release(&self) {
        if self.owns() == Some(true) {
            self.value.moving.set_if_changed(false);
        }
    }
}

// ---------------------------------------------------------------------------
// Animation: steps and their composition
// ---------------------------------------------------------------------------

/// How a frame left a step.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Flow {
    /// It has more to do.
    Running,
    /// It ended, with this many seconds of the frame unused: the step after it starts that
    /// far in, so a sequence or a loop keeps exact time.
    Ended(f64),
    /// A value it moves was disposed with its scope: the whole animation stops.
    Gone,
}

/// One step on one value (or one closure): the leaves of an [`Animation`].
trait Leaf {
    /// The step, started now. `None` when its value is gone.
    fn begin(&self) -> Option<Box<dyn LeafRun>>;
    /// Puts the step's value where the step would leave it.
    fn finish(&self);
    /// Whether the step never ends by itself.
    fn endless(&self) -> bool {
        false
    }
    /// A closure that puts the step's value back where it is now.
    fn snapshot(&self) -> Option<Box<dyn Fn()>> {
        None
    }
    /// The window whose frames should drive the step.
    fn clock(&self) -> Option<FrameClock> {
        None
    }
}

/// A [`Leaf`] in flight.
trait LeafRun {
    /// Advances by `dt` seconds.
    fn advance(&mut self, dt: f64) -> Flow;
    /// Gives up the value where it stands: the animation was stopped.
    fn release(&self);
}

/// A snapshot closure for a value: restores it and its velocity.
fn snapshot_of<T: Lerp + 'static>(value: Animated<T>) -> Option<Box<dyn Fn()>> {
    let at = value.try_now()?;
    let velocity = day_reactive::untrack(|| value.velocity.try_get())?;
    Some(Box::new(move || {
        value.set(at.clone());
        value.velocity.set_if_changed(velocity);
    }))
}

/// `value` to `to` over `secs` along `easing`.
struct TimingLeaf<T: Lerp + 'static> {
    value: Animated<T>,
    to: T,
    secs: f64,
    easing: Easing,
}

impl<T: Lerp + 'static> Leaf for TimingLeaf<T> {
    fn begin(&self) -> Option<Box<dyn LeafRun>> {
        let (hold, from) = Hold::take(self.value, Some(self.to.clone()))?;
        Some(Box::new(TimingRun {
            hold,
            from,
            to: self.to.clone(),
            secs: self.secs,
            easing: self.easing,
            elapsed: 0.0,
        }))
    }
    fn finish(&self) {
        self.value.set(self.to.clone());
    }
    fn snapshot(&self) -> Option<Box<dyn Fn()>> {
        snapshot_of(self.value)
    }
    fn clock(&self) -> Option<FrameClock> {
        self.value.clock()
    }
}

struct TimingRun<T: Lerp + 'static> {
    hold: Hold<T>,
    from: T,
    to: T,
    secs: f64,
    easing: Easing,
    elapsed: f64,
}

impl<T: Lerp + 'static> LeafRun for TimingRun<T> {
    fn advance(&mut self, dt: f64) -> Flow {
        self.elapsed += dt;
        let left = self.elapsed - self.secs;
        match self.hold.owns() {
            None => Flow::Gone,
            // Displaced: the step keeps its time, so what follows it is not brought forward.
            Some(false) if left >= 0.0 => Flow::Ended(left.min(dt)),
            Some(false) => Flow::Running,
            Some(true) if left >= 0.0 => {
                self.hold.land(self.to.clone());
                Flow::Ended(left.min(dt))
            }
            Some(true) => {
                let t = self.easing.value(self.elapsed / self.secs);
                self.hold.write(self.from.lerp(&self.to, t), dt);
                Flow::Running
            }
        }
    }
    fn release(&self) {
        self.hold.release();
    }
}

/// `value` to `to` under an [`AnimSpec`]'s whole timeline.
struct TweenLeaf<T: Lerp + 'static> {
    value: Animated<T>,
    to: T,
    spec: AnimSpec,
}

impl<T: Lerp + 'static> Leaf for TweenLeaf<T> {
    fn begin(&self) -> Option<Box<dyn LeafRun>> {
        let (hold, from) = Hold::take(self.value, Some(self.to.clone()))?;
        Some(Box::new(TweenRun {
            hold,
            from,
            to: self.to.clone(),
            spec: self.spec,
            elapsed: 0.0,
        }))
    }
    fn finish(&self) {
        self.value.set(self.to.clone());
    }
    fn endless(&self) -> bool {
        self.spec.repeats_forever()
    }
    fn snapshot(&self) -> Option<Box<dyn Fn()>> {
        snapshot_of(self.value)
    }
    fn clock(&self) -> Option<FrameClock> {
        self.value.clock()
    }
}

struct TweenRun<T: Lerp + 'static> {
    hold: Hold<T>,
    from: T,
    to: T,
    spec: AnimSpec,
    elapsed: f64,
}

impl<T: Lerp + 'static> LeafRun for TweenRun<T> {
    fn advance(&mut self, dt: f64) -> Flow {
        self.elapsed += dt;
        let sample = self.spec.sample(self.elapsed);
        let left = self
            .spec
            .total_secs()
            .map_or(0.0, |total| (self.elapsed - total).clamp(0.0, dt));
        match self.hold.owns() {
            None => Flow::Gone,
            Some(false) if sample.done => Flow::Ended(left),
            Some(false) => Flow::Running,
            Some(true) if sample.done => {
                // The value rests on its target whatever the last cycle did (rule 5 of the
                // `AnimSpec` contract).
                self.hold.land(self.to.clone());
                Flow::Ended(left)
            }
            Some(true) => {
                self.hold
                    .write(self.from.lerp(&self.to, sample.progress), dt);
                Flow::Running
            }
        }
    }
    fn release(&self) {
        self.hold.release();
    }
}

/// `value` sprung to `to` from its own velocity.
struct SpringLeaf {
    value: Animated<f64>,
    to: f64,
    spring: Spring,
}

impl Leaf for SpringLeaf {
    fn begin(&self) -> Option<Box<dyn LeafRun>> {
        let (hold, from) = Hold::take(self.value, Some(self.to))?;
        let velocity = day_reactive::untrack(|| self.value.velocity.try_get())?;
        // The largest excursion the spring can make from here: its distance, or what its
        // speed carries it to. Rest is a thousandth of that.
        let mass = self.spring.mass.max(1e-6);
        let omega0 = (self.spring.stiffness.max(1e-6) / mass).sqrt();
        let reach = (self.to - from).abs().max(velocity.abs() / omega0);
        Some(Box::new(SpringRun {
            hold,
            to: self.to,
            spring: self.spring,
            rest: (reach * 1e-3).max(f64::EPSILON),
            rest_speed: (reach * omega0 * 1e-3).max(f64::EPSILON),
        }))
    }
    fn finish(&self) {
        self.value.set(self.to);
    }
    fn snapshot(&self) -> Option<Box<dyn Fn()>> {
        snapshot_of(self.value)
    }
    fn clock(&self) -> Option<FrameClock> {
        self.value.clock()
    }
}

struct SpringRun {
    hold: Hold<f64>,
    to: f64,
    spring: Spring,
    /// How close counts as arrived.
    rest: f64,
    /// How slow counts as at rest.
    rest_speed: f64,
}

impl LeafRun for SpringRun {
    fn advance(&mut self, dt: f64) -> Flow {
        match self.hold.owns() {
            None => return Flow::Gone,
            Some(false) => return Flow::Ended(0.0),
            Some(true) => {}
        }
        let value = self.hold.value;
        let (Some(at), Some(velocity)) = (
            value.try_now(),
            day_reactive::untrack(|| value.velocity.try_get()),
        ) else {
            return Flow::Gone;
        };
        let mut state = SpringState {
            value: at,
            velocity,
        };
        self.spring.step(&mut state, self.to, dt);
        if (state.value - self.to).abs() < self.rest && state.velocity.abs() < self.rest_speed {
            self.hold.land(self.to);
            return Flow::Ended(0.0);
        }
        value.value.set(state.value);
        value.velocity.set(state.velocity);
        Flow::Running
    }
    fn release(&self) {
        self.hold.release();
    }
}

/// `value` coasting from its own velocity until it stops.
struct DecayLeaf {
    value: Animated<f64>,
    decay: Decay,
}

impl DecayLeaf {
    /// The value and its velocity, while its scope is alive.
    fn state(&self) -> Option<SpringState> {
        Some(SpringState {
            value: self.value.try_now()?,
            velocity: day_reactive::untrack(|| self.value.velocity.try_get())?,
        })
    }
}

impl Leaf for DecayLeaf {
    fn begin(&self) -> Option<Box<dyn LeafRun>> {
        let (hold, _) = Hold::take(self.value, self.state().map(|s| self.decay.rest(s)))?;
        Some(Box::new(DecayRun {
            hold,
            decay: self.decay,
        }))
    }
    fn finish(&self) {
        if let Some(state) = self.state() {
            self.value.set(self.decay.rest(state));
        }
    }
    fn snapshot(&self) -> Option<Box<dyn Fn()>> {
        snapshot_of(self.value)
    }
    fn clock(&self) -> Option<FrameClock> {
        self.value.clock()
    }
}

struct DecayRun {
    hold: Hold<f64>,
    decay: Decay,
}

impl LeafRun for DecayRun {
    fn advance(&mut self, dt: f64) -> Flow {
        match self.hold.owns() {
            None => return Flow::Gone,
            Some(false) => return Flow::Ended(0.0),
            Some(true) => {}
        }
        let value = self.hold.value;
        let (Some(at), Some(velocity)) = (
            value.try_now(),
            day_reactive::untrack(|| value.velocity.try_get()),
        ) else {
            return Flow::Gone;
        };
        let mut state = SpringState {
            value: at,
            velocity,
        };
        self.decay.step(&mut state, dt);
        if self.decay.stopped(state) {
            self.hold.land(state.value);
            return Flow::Ended(0.0);
        }
        value.value.set(state.value);
        value.velocity.set(state.velocity);
        Flow::Running
    }
    fn release(&self) {
        self.hold.release();
    }
}

/// A closure called with each frame's sample of an [`AnimSpec`].
struct DriveLeaf {
    spec: AnimSpec,
    step: Rc<RefCell<dyn FnMut(AnimSample)>>,
}

impl DriveLeaf {
    /// Calls the closure, unless it is already running (a step that restarts its own
    /// animation).
    fn call(step: &Rc<RefCell<dyn FnMut(AnimSample)>>, sample: AnimSample) {
        if let Ok(mut step) = step.try_borrow_mut() {
            step(sample);
        }
    }
}

impl Leaf for DriveLeaf {
    fn begin(&self) -> Option<Box<dyn LeafRun>> {
        Some(Box::new(DriveRun {
            spec: self.spec,
            step: self.step.clone(),
            elapsed: 0.0,
        }))
    }
    fn finish(&self) {
        let end = self.spec.total_secs().unwrap_or(0.0);
        Self::call(
            &self.step,
            AnimSample {
                done: true,
                ..self.spec.sample(end)
            },
        );
    }
    fn endless(&self) -> bool {
        self.spec.repeats_forever()
    }
}

struct DriveRun {
    spec: AnimSpec,
    step: Rc<RefCell<dyn FnMut(AnimSample)>>,
    elapsed: f64,
}

impl LeafRun for DriveRun {
    fn advance(&mut self, dt: f64) -> Flow {
        self.elapsed += dt;
        let sample = self.spec.sample(self.elapsed);
        DriveLeaf::call(&self.step, sample);
        if sample.done {
            let total = self.spec.total_secs().unwrap_or(self.elapsed);
            Flow::Ended((self.elapsed - total).clamp(0.0, dt))
        } else {
            Flow::Running
        }
    }
    fn release(&self) {}
}

/// The shape of an [`Animation`].
#[derive(Clone)]
enum Node {
    /// One step.
    Leaf(Rc<dyn Leaf>),
    /// Nothing, for this many seconds.
    Delay(f64),
    /// Its parts one after another.
    Sequence(Vec<Node>),
    /// Its parts together, ending with the last of them.
    Parallel(Vec<Node>),
    /// Its part again each time it ends: this many times, or without end.
    Repeat(Box<Node>, Option<u32>),
}

impl Node {
    /// Whether the animation never ends by itself.
    fn endless(&self) -> bool {
        match self {
            Node::Leaf(leaf) => leaf.endless(),
            // Played no times, a repeat ends at once whatever it holds.
            Node::Delay(_) | Node::Repeat(_, Some(0)) => false,
            Node::Sequence(parts) | Node::Parallel(parts) => parts.iter().any(Node::endless),
            Node::Repeat(part, times) => times.is_none() || part.endless(),
        }
    }

    /// Puts every value where the animation would leave it, in the order it would.
    fn finish(&self) {
        match self {
            Node::Leaf(leaf) => leaf.finish(),
            // Played no times, a repeat leaves every value where it is.
            Node::Delay(_) | Node::Repeat(_, Some(0)) => {}
            Node::Sequence(parts) | Node::Parallel(parts) => parts.iter().for_each(Node::finish),
            Node::Repeat(part, _) => part.finish(),
        }
    }

    /// Visits every leaf, in order.
    fn leaves(&self, visit: &mut dyn FnMut(&dyn Leaf)) {
        match self {
            Node::Leaf(leaf) => visit(leaf.as_ref()),
            Node::Delay(_) => {}
            Node::Sequence(parts) | Node::Parallel(parts) => {
                for part in parts {
                    part.leaves(visit);
                }
            }
            Node::Repeat(part, _) => part.leaves(visit),
        }
    }

    /// The animation, started now. `None` when a value it would move first is gone, and then
    /// nothing it holds stays claimed: a group lets go of the parts it had already started.
    fn begin(&self) -> Option<Run> {
        Some(match self {
            Node::Leaf(leaf) => Run::Leaf(leaf.begin()?),
            Node::Delay(secs) => Run::Delay(*secs),
            Node::Sequence(parts) => Run::Sequence {
                parts: parts.clone(),
                at: 0,
                current: match parts.first() {
                    Some(first) => Some(Box::new(first.begin()?)),
                    None => None,
                },
            },
            Node::Parallel(parts) => {
                let mut runs = Vec::with_capacity(parts.len());
                for part in parts {
                    let Some(run) = part.begin() else {
                        for (run, _) in &runs {
                            Run::release(run);
                        }
                        return None;
                    };
                    runs.push((run, false));
                }
                Run::Parallel(runs)
            }
            // Played no times: a pause of no length, which never touches a value.
            Node::Repeat(_, Some(0)) => Run::Delay(0.0),
            Node::Repeat(part, times) => Run::Repeat {
                part: (**part).clone(),
                times: *times,
                done: 0,
                current: Box::new(part.begin()?),
            },
        })
    }
}

/// How many passes that take no time an endless repeat plays in one frame before it waits for
/// the next.
const INSTANT_PASSES_PER_FRAME: u32 = 64;

/// A [`Node`] in flight.
enum Run {
    /// A step on its way.
    Leaf(Box<dyn LeafRun>),
    /// A pause, with the seconds left of it.
    Delay(f64),
    /// A sequence at one of its parts. A part starts when the one before it ends, so it reads
    /// its value where that one left it.
    Sequence {
        parts: Vec<Node>,
        at: usize,
        current: Option<Box<Run>>,
    },
    /// A group, each part with whether it has ended.
    Parallel(Vec<(Run, bool)>),
    /// A loop around one part.
    Repeat {
        part: Node,
        times: Option<u32>,
        done: u32,
        current: Box<Run>,
    },
}

impl Run {
    /// Advances by `dt` seconds.
    fn advance(&mut self, dt: f64) -> Flow {
        match self {
            Run::Leaf(run) => run.advance(dt),
            Run::Delay(left) => {
                *left -= dt;
                if *left <= 0.0 {
                    Flow::Ended((-*left).min(dt))
                } else {
                    Flow::Running
                }
            }
            Run::Sequence { parts, at, current } => {
                let mut dt = dt;
                loop {
                    let Some(run) = current else {
                        return Flow::Ended(dt);
                    };
                    match run.advance(dt) {
                        Flow::Ended(left) => dt = left,
                        flow => return flow,
                    }
                    *at += 1;
                    *current = match parts.get(*at) {
                        Some(next) => match next.begin() {
                            Some(run) => Some(Box::new(run)),
                            None => return Flow::Gone,
                        },
                        None => None,
                    };
                }
            }
            Run::Parallel(runs) => {
                let mut left = dt;
                for (run, ended) in runs.iter_mut().filter(|(_, ended)| !*ended) {
                    match run.advance(dt) {
                        Flow::Running => {}
                        Flow::Ended(unused) => {
                            *ended = true;
                            left = left.min(unused);
                        }
                        Flow::Gone => return Flow::Gone,
                    }
                }
                if runs.iter().all(|(_, ended)| *ended) {
                    Flow::Ended(left)
                } else {
                    Flow::Running
                }
            }
            Run::Repeat {
                part,
                times,
                done,
                current,
            } => {
                let mut dt = dt;
                // Every pass that fits in `dt` plays now, so a late frame (a stall, an app
                // coming back) catches up instead of losing time. A finite repeat ends however
                // short its passes are, and hands what is left of `dt` to what follows it. Only
                // an endless repeat of passes that take no time at all could spin here for ever;
                // a run of those yields to the next frame (an endless run has no later steps
                // to lose the time to).
                let mut instant = 0;
                loop {
                    let before = dt;
                    match current.advance(dt) {
                        Flow::Ended(left) => {
                            *done = done.saturating_add(1);
                            if times.is_some_and(|times| *done >= times) {
                                return Flow::Ended(left);
                            }
                            match part.begin() {
                                Some(run) => **current = run,
                                None => return Flow::Gone,
                            }
                            if left <= 0.0 {
                                return Flow::Running;
                            }
                            if left < before || times.is_some() {
                                instant = 0;
                            } else {
                                instant += 1;
                                if instant >= INSTANT_PASSES_PER_FRAME {
                                    return Flow::Running;
                                }
                            }
                            dt = left;
                        }
                        flow => return flow,
                    }
                }
            }
        }
    }

    /// Gives up every value a running step holds, where it stands.
    fn release(&self) {
        match self {
            Run::Leaf(run) => run.release(),
            Run::Delay(_) => {}
            Run::Sequence { current, .. } => {
                if let Some(run) = current {
                    run.release();
                }
            }
            Run::Parallel(runs) => runs
                .iter()
                .filter(|(_, ended)| !*ended)
                .for_each(|(run, _)| run.release()),
            Run::Repeat { current, .. } => current.release(),
        }
    }
}

/// Motion for [`Animated`] values: one step, or a composition of several. Building one moves
/// nothing; [`start`](Animation::start) plays it, and it can be started again.
///
/// ```rust,ignore
/// let lifts: Vec<Animated> = (0..5).map(|_| Animated::new(0.0)).collect();
/// let entrance = Animation::stagger(
///     70,
///     lifts.iter().map(|&lift| {
///         Animation::sequence([
///             Animation::timing(lift, -46.0, 220, Easing::Power(Ease::Out, 2.0)),
///             Animation::spring(lift, 0.0, Spring::new(320.0, 11.0)),
///         ])
///     }),
/// );
/// let playing = entrance.start();
/// day::task(async move {
///     if playing.await == Outcome::Finished {
///         status.set(Status::Arrived);
///     }
/// });
/// ```
///
/// Under reduced motion ([`reduce_motion`](crate::reduce_motion)) a finite animation puts every
/// value on its end state at once, also when the setting flips mid-flight; one that never ends
/// by itself (a [`looped`](Animation::looped) pulse, the sign that work is going on) keeps
/// running. An animation stops when a value it moves is disposed with its scope, so a page
/// needs no cleanup of its own.
#[derive(Clone)]
#[must_use = "an animation does nothing until it is started"]
pub struct Animation {
    node: Node,
}

impl Animation {
    /// Moves `value` to `to` over `ms` milliseconds along `easing`, from wherever it is when
    /// the step starts.
    pub fn timing<T: Lerp + 'static>(value: Animated<T>, to: T, ms: u32, easing: Easing) -> Self {
        Self::leaf(TimingLeaf {
            value,
            to,
            secs: f64::from(ms) / 1000.0,
            easing,
        })
    }

    /// Moves `value` to `to` as `spec` says: its delay, its curve or spring, its repeats and
    /// autoreverse ([`AnimSpec::sample`]). The frame-driven twin of
    /// [`with_animation`](crate::with_animation): a canvas value moved with the spec a native
    /// property is animated with moves in step with it. A spec that repeats forever never ends.
    pub fn tween<T: Lerp + 'static>(value: Animated<T>, to: T, spec: AnimSpec) -> Self {
        Self::leaf(TweenLeaf { value, to, spec })
    }

    /// Springs `value` to `to`, starting at the velocity it has, until it comes to rest. A
    /// spring that takes over from another step, or follows
    /// [`set_velocity`](Animated::set_velocity), carries that speed on.
    pub fn spring(value: Animated<f64>, to: f64, spring: Spring) -> Self {
        Self::leaf(SpringLeaf { value, to, spring })
    }

    /// Lets `value` coast from the velocity it has until it stops, as a flung scroll view does.
    pub fn decay(value: Animated<f64>, decay: Decay) -> Self {
        Self::leaf(DecayLeaf { value, decay })
    }

    /// Calls `step` with each frame's [`AnimSample`] of `spec`, the last one `done`: for a
    /// transition that is more than one value (a chart blending two states). Under reduced
    /// motion `step` is called once, with the final sample.
    pub fn drive(spec: AnimSpec, step: impl FnMut(AnimSample) + 'static) -> Self {
        Self::leaf(DriveLeaf {
            spec,
            step: Rc::new(RefCell::new(step)),
        })
    }

    /// Does nothing for `ms` milliseconds.
    pub fn delay(ms: u32) -> Self {
        Animation {
            node: Node::Delay(f64::from(ms) / 1000.0),
        }
    }

    /// Runs `steps` one after another. Each starts from where its value stands when its turn
    /// comes, and on the frame the step before it ends.
    pub fn sequence(steps: impl IntoIterator<Item = Animation>) -> Self {
        Animation {
            node: Node::Sequence(steps.into_iter().map(|step| step.node).collect()),
        }
    }

    /// Runs `steps` together and ends with the last of them.
    pub fn parallel(steps: impl IntoIterator<Item = Animation>) -> Self {
        Animation {
            node: Node::Parallel(steps.into_iter().map(|step| step.node).collect()),
        }
    }

    /// Runs `steps` together, each starting `gap_ms` milliseconds after the one before it.
    pub fn stagger(gap_ms: u32, steps: impl IntoIterator<Item = Animation>) -> Self {
        let gap = f64::from(gap_ms) / 1000.0;
        Animation {
            node: Node::Parallel(
                steps
                    .into_iter()
                    .enumerate()
                    .map(|(i, step)| {
                        if i == 0 {
                            step.node
                        } else {
                            Node::Sequence(vec![Node::Delay(gap * i as f64), step.node])
                        }
                    })
                    .collect(),
            ),
        }
    }

    /// Plays the animation `times` times in a row.
    pub fn repeat(self, times: u32) -> Self {
        Animation {
            node: Node::Repeat(Box::new(self.node), Some(times)),
        }
    }

    /// Plays the animation again each time it ends, until it is stopped.
    pub fn looped(self) -> Self {
        Animation {
            node: Node::Repeat(Box::new(self.node), None),
        }
    }

    /// Whether the animation never ends by itself.
    #[must_use]
    pub fn is_endless(&self) -> bool {
        self.node.endless()
    }

    /// Puts every value where the animation would leave it, without playing it. An endless
    /// animation's values are left where one pass would.
    pub fn jump_to_end(&self) {
        day_reactive::batch(|| self.node.finish());
    }

    /// Plays the animation on the window of the first value it moves (the current page's
    /// window when it moves none).
    pub fn start(&self) -> Playing {
        let mut clock = None;
        self.node
            .leaves(&mut |leaf| clock = clock.or_else(|| leaf.clock()));
        self.start_on(clock.unwrap_or_else(FrameClock::current))
    }

    /// Plays the animation on `clock`'s frames.
    pub fn start_on(&self, clock: FrameClock) -> Playing {
        let mut restore = Vec::new();
        self.node
            .leaves(&mut |leaf| restore.extend(leaf.snapshot()));
        let shared = Rc::new(Shared {
            outcome: Cell::new(None),
            handle: RefCell::new(None),
            run: RefCell::new(None),
            wakers: RefCell::new(Vec::new()),
            listeners: RefCell::new(Vec::new()),
            restore,
        });
        let playing = Playing {
            shared: shared.clone(),
        };
        let endless = self.node.endless();
        if crate::reduce_motion_now() && !endless {
            self.jump_to_end();
            shared.end(Outcome::Finished);
            return playing;
        }
        let Some(run) = day_reactive::batch(|| self.node.begin()) else {
            shared.end(Outcome::Stopped);
            return playing;
        };
        *shared.run.borrow_mut() = Some(run);
        let node = self.node.clone();
        let guard = Guard(shared.clone());
        let handle = clock.subscribe(move |frame| {
            let shared = &guard.0;
            if shared.outcome.get().is_some() {
                return ControlFlow::Break(());
            }
            // The setting is read per frame, so one flipped mid-flight lands the animation too.
            let flow = if crate::reduce_motion_now() && !endless {
                node.finish();
                Flow::Ended(0.0)
            } else {
                shared
                    .run
                    .try_borrow_mut()
                    .ok()
                    .and_then(|mut run| {
                        run.as_mut()
                            .map(|run| run.advance(frame.delta.as_secs_f64()))
                    })
                    .unwrap_or(Flow::Gone)
            };
            match flow {
                // A step may have stopped the animation it belongs to.
                Flow::Running if shared.outcome.get().is_none() => {
                    return ControlFlow::Continue(());
                }
                Flow::Running => shared.cleanup(),
                Flow::Ended(_) => shared.end(Outcome::Finished),
                Flow::Gone => shared.end(Outcome::Stopped),
            }
            ControlFlow::Break(())
        });
        // Ended already only if `subscribe` ran the callback inline, which no clock does.
        if shared.outcome.get().is_none() {
            *shared.handle.borrow_mut() = Some(handle);
        }
        playing
    }

    /// An animation of one step.
    fn leaf(leaf: impl Leaf + 'static) -> Self {
        Animation {
            node: Node::Leaf(Rc::new(leaf)),
        }
    }
}

// ---------------------------------------------------------------------------
// Playing: an animation in flight
// ---------------------------------------------------------------------------

/// How an animation's run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// It played to its end (or landed there at once under reduced motion).
    Finished,
    /// It was stopped first: by [`Playing::stop`] or [`Playing::reset`], or because a value
    /// it moved or its window went away.
    Stopped,
}

/// A callback for the end of a run.
type Listener = Box<dyn FnOnce(Outcome)>;

/// What a running animation and its handles share.
struct Shared {
    /// How the run ended, once it has.
    outcome: Cell<Option<Outcome>>,
    /// The frame subscription driving it.
    handle: RefCell<Option<FrameHandle>>,
    /// The steps in flight.
    run: RefCell<Option<Run>>,
    /// Tasks awaiting the end.
    wakers: RefCell<Vec<Waker>>,
    /// Callbacks for the end.
    listeners: RefCell<Vec<Listener>>,
    /// Puts each value back where it was when the run started.
    restore: Vec<Box<dyn Fn()>>,
}

impl Shared {
    /// Ends the run, once: releases what it held, wakes what awaited it, calls its listeners.
    fn end(&self, outcome: Outcome) {
        if self.outcome.get().is_some() {
            return;
        }
        self.outcome.set(Some(outcome));
        self.cleanup();
    }

    /// Lets go of the steps and the frame subscription, then tells everyone waiting. Skipped
    /// while a step of this very run is advancing (it stopped its own animation): the frame
    /// callback calls it again once the step returns.
    fn cleanup(&self) {
        let Some(outcome) = self.outcome.get() else {
            return;
        };
        let Ok(mut run) = self.run.try_borrow_mut() else {
            return;
        };
        if let Some(run) = run.take() {
            day_reactive::batch(|| run.release());
        }
        drop(run);
        // Dropping the handle cancels the subscription.
        let handle = self.handle.borrow_mut().take();
        drop(handle);
        for waker in std::mem::take(&mut *self.wakers.borrow_mut()) {
            waker.wake();
        }
        for listener in std::mem::take(&mut *self.listeners.borrow_mut()) {
            listener(outcome);
        }
    }
}

/// Ends a run whose frame subscription was dropped under it (its window closed), so nothing
/// awaits it for ever.
struct Guard(Rc<Shared>);

impl Drop for Guard {
    fn drop(&mut self) {
        // At thread teardown the reactive runtime may already be gone; nothing is waiting then.
        if crate::frame::alive() {
            self.0.end(Outcome::Stopped);
        }
    }
}

/// A started [`Animation`]: stop it, reset it, ask how it went, or await its end.
///
/// The animation does not depend on the handle: dropping it leaves the animation running, and
/// it stops by itself when a value it moves is disposed. Awaiting the handle answers
/// [`Outcome::Finished`] when the animation played to its end and [`Outcome::Stopped`] when
/// something ended it first.
#[derive(Clone)]
pub struct Playing {
    shared: Rc<Shared>,
}

impl Playing {
    /// Stops the animation where it stands; its values keep what they show. Answers whether
    /// it was still running.
    #[expect(
        clippy::must_use_candidate,
        reason = "stopping is the point; whether it was running is incidental"
    )]
    pub fn stop(&self) -> bool {
        let running = self.is_running();
        self.shared.end(Outcome::Stopped);
        running
    }

    /// Stops the animation and puts every value it moves back where it was when it started.
    pub fn reset(&self) {
        self.stop();
        day_reactive::batch(|| self.shared.restore.iter().for_each(|restore| restore()));
    }

    /// Whether the animation is still playing.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.shared.outcome.get().is_none()
    }

    /// Whether the animation played to its end.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.shared.outcome.get() == Some(Outcome::Finished)
    }

    /// How the run ended, or `None` while it plays.
    #[must_use]
    pub fn outcome(&self) -> Option<Outcome> {
        self.shared.outcome.get()
    }

    /// Calls `f` when the run ends, with how; at once when it already has. The callback form
    /// of awaiting the handle.
    pub fn on_end(&self, f: impl FnOnce(Outcome) + 'static) {
        match self.shared.outcome.get() {
            Some(outcome) => f(outcome),
            None => self.shared.listeners.borrow_mut().push(Box::new(f)),
        }
    }
}

impl std::fmt::Debug for Playing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Playing")
            .field("outcome", &self.shared.outcome.get())
            .finish()
    }
}

impl Future for Playing {
    type Output = Outcome;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Outcome> {
        if let Some(outcome) = self.shared.outcome.get() {
            return Poll::Ready(outcome);
        }
        self.shared.wakers.borrow_mut().push(cx.waker().clone());
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_curve_starts_at_zero_and_ends_at_one() {
        let curves = [
            Easing::Linear,
            Easing::EASE,
            Easing::EASE_IN_OUT,
            Easing::Power(Ease::Out, 2.0),
            Easing::Power(Ease::InOut, 3.0),
            Easing::Sine(Ease::In),
            Easing::Circle(Ease::Out),
            Easing::Exponential(Ease::InOut),
            Easing::Back(Ease::Out, 1.70158),
            Easing::ElasticOut(1.0),
            Easing::BounceOut,
        ];
        for curve in curves {
            assert!(curve.value(0.0).abs() < 1e-3, "{curve:?} at 0");
            assert!((curve.value(1.0) - 1.0).abs() < 1e-3, "{curve:?} at 1");
        }
    }

    #[test]
    fn back_out_overshoots_and_linear_bezier_is_linear() {
        let peak = Easing::Back(Ease::Out, 1.70158)
            .samples(101)
            .into_iter()
            .fold(0.0, f64::max);
        assert!(peak > 1.05 && peak < 1.2, "{peak}");
        let straight = Easing::Bezier(0.25, 0.25, 0.75, 0.75);
        assert!((straight.value(0.3) - 0.3).abs() < 1e-3);
    }

    #[test]
    fn a_spring_settles_on_its_target_and_a_loose_one_overshoots() {
        let firm = Spring::new(400.0, 30.0);
        let mut state = SpringState::default();
        for _ in 0..240 {
            firm.step(&mut state, 100.0, 1.0 / 60.0);
        }
        assert!(firm.settled(state, 100.0), "{state:?}");

        let peak = Spring::new(180.0, 8.0)
            .response(1.8, 200)
            .into_iter()
            .fold(0.0, f64::max);
        assert!(peak > 1.2, "{peak}");
    }

    #[test]
    fn a_decay_stops_at_a_finite_distance() {
        let decay = Decay { rate: 0.992 };
        let mut state = SpringState {
            value: 0.0,
            velocity: 1000.0,
        };
        for _ in 0..600 {
            decay.step(&mut state, 1.0 / 60.0);
        }
        assert!(decay.stopped(state));
        // 1000 pt/s losing 0.8% per ms covers 1000 / (-ln 0.992 · 1000) points.
        assert!((state.value - 124.5).abs() < 2.0, "{}", state.value);
    }

    // --- The driver --------------------------------------------------------------------------

    use crate::tree::RNode;
    use day_spec::{Color, FrameStamp};
    use std::collections::VecDeque;

    /// A frame source the test fires by hand, 60 frames a second from a time of its own.
    struct Frames {
        pending: Rc<RefCell<VecDeque<day_spec::FrameCallback>>>,
        now: Cell<f64>,
    }

    impl Frames {
        fn new() -> Self {
            let pending: Rc<RefCell<VecDeque<day_spec::FrameCallback>>> = Rc::default();
            let q = pending.clone();
            crate::frame::install_frame_requester(move |_, cb| {
                q.borrow_mut().push_back(cb);
                Box::new(|| {})
            });
            Frames {
                pending,
                now: Cell::new(10.0),
            }
        }

        fn clock() -> FrameClock {
            FrameClock::for_root(RNode::default())
        }

        /// Delivers one frame `secs` after the last. Answers whether one was wanted.
        fn step(&self, secs: f64) -> bool {
            self.now.set(self.now.get() + secs);
            // Requests of cancelled subscriptions stay queued in this fake; deliver them all.
            let due: Vec<_> = self.pending.borrow_mut().drain(..).collect();
            let wanted = !due.is_empty();
            for cb in due {
                cb(FrameStamp::new(self.now.get()));
            }
            wanted
        }

        /// Runs frames of a sixtieth until `secs` have passed.
        fn run(&self, secs: f64) {
            for _ in 0..(secs * 60.0).round() as usize {
                self.step(1.0 / 60.0);
            }
        }

        fn idle() -> bool {
            crate::frame_consumer_count() == 0
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn a_timing_moves_its_value_and_releases_the_clock_on_arrival() {
        let frames = Frames::new();
        let v = Animated::on(Frames::clock(), 0.0);
        let playing = Animation::timing(v, 100.0, 100, Easing::Linear).start();
        assert!(v.is_animating() && playing.is_running());
        assert!(close(v.target(), 100.0));
        frames.step(0.0); // the first frame is the start: elapsed 0
        assert!(close(v.get_untracked(), 0.0));
        frames.step(0.05);
        assert!(close(v.get_untracked(), 50.0));
        assert!(close(v.velocity.get_untracked(), 1000.0));
        frames.step(0.06);
        assert!(close(v.get_untracked(), 100.0));
        assert_eq!(playing.outcome(), Some(Outcome::Finished));
        assert!(!v.is_animating());
        assert!(Frames::idle(), "no frames wanted after arrival");
    }

    #[test]
    fn animate_to_retargets_from_the_value_on_screen() {
        let frames = Frames::new();
        let v = Animated::on(Frames::clock(), 0.0);
        let first = v.animate_to(100.0, AnimSpec::linear(100));
        frames.step(0.0);
        frames.step(0.05);
        assert!(close(v.get_untracked(), 50.0));
        let second = v.animate_to(0.0, AnimSpec::linear(100));
        assert_eq!(first.outcome(), Some(Outcome::Stopped));
        frames.step(0.01); // the new flight's first frame
        assert!(close(v.get_untracked(), 50.0));
        frames.step(0.05);
        assert!(close(v.get_untracked(), 25.0));
        frames.step(0.06);
        assert!(close(v.get_untracked(), 0.0));
        assert!(second.is_finished() && !v.is_animating() && Frames::idle());
    }

    #[test]
    fn a_tween_follows_its_spec_and_rests_on_the_target() {
        let frames = Frames::new();
        let v = Animated::on(Frames::clock(), 0.0);
        // Forward and back: the last cycle ends on the start, the value rests on the target.
        let spec = AnimSpec::linear(100).delay(100).repeat(1, true);
        let playing = Animation::tween(v, 10.0, spec).start();
        frames.step(0.0);
        frames.step(0.05);
        assert!(close(v.get_untracked(), 0.0), "held through the delay");
        frames.step(0.1); // 0.15: half way forward
        assert!(close(v.get_untracked(), 5.0));
        frames.step(0.125); // 0.275: three quarters of the way back
        assert!(close(v.get_untracked(), 2.5));
        frames.step(0.05);
        assert!(close(v.get_untracked(), 10.0));
        assert!(playing.is_finished());
    }

    #[test]
    fn a_sequence_runs_in_order_and_carries_time_across_its_steps() {
        let frames = Frames::new();
        let (a, b) = (
            Animated::on(Frames::clock(), 0.0),
            Animated::on(Frames::clock(), 0.0),
        );
        let playing = Animation::sequence([
            Animation::timing(a, 1.0, 100, Easing::Linear),
            Animation::delay(100),
            Animation::timing(b, 1.0, 100, Easing::Linear),
        ])
        .start();
        frames.step(0.0);
        frames.step(0.15);
        assert!(close(a.get_untracked(), 1.0) && close(b.get_untracked(), 0.0));
        assert!(!b.is_animating());
        // 0.25 in: the delay ended at 0.2 and the last step is half way, not at its start.
        frames.step(0.1);
        assert!(close(b.get_untracked(), 0.5), "{}", b.get_untracked());
        frames.step(0.06);
        assert!(playing.is_finished() && close(b.get_untracked(), 1.0));
    }

    #[test]
    fn a_stagger_starts_each_step_a_gap_after_the_one_before() {
        let frames = Frames::new();
        let values: Vec<Animated> = (0..3).map(|_| Animated::on(Frames::clock(), 0.0)).collect();
        let playing = Animation::stagger(
            100,
            values
                .iter()
                .map(|&v| Animation::timing(v, 1.0, 200, Easing::Linear)),
        )
        .start();
        frames.step(0.0);
        frames.step(0.2);
        let at: Vec<f64> = values.iter().map(|v| v.get_untracked()).collect();
        assert!(
            close(at[0], 1.0) && close(at[1], 0.5) && close(at[2], 0.0),
            "{at:?}"
        );
        frames.step(0.2);
        assert!(playing.is_finished());
        assert!(values.iter().all(|v| close(v.get_untracked(), 1.0)));
    }

    #[test]
    fn repeat_plays_a_number_of_times_and_looped_never_ends() {
        let frames = Frames::new();
        let v = Animated::on(Frames::clock(), 0.0);
        let there_and_back = Animation::sequence([
            Animation::timing(v, 1.0, 100, Easing::Linear),
            Animation::timing(v, 0.0, 100, Easing::Linear),
        ]);
        assert!(!there_and_back.is_endless());
        let twice = there_and_back.clone().repeat(2).start();
        frames.step(0.0);
        frames.step(0.25); // second pass, a quarter in
        assert!(close(v.get_untracked(), 0.5), "{}", v.get_untracked());
        assert!(twice.is_running());
        frames.step(0.2);
        assert!(twice.is_finished() && close(v.get_untracked(), 0.0));

        let pulse = there_and_back.looped();
        assert!(pulse.is_endless());
        let playing = pulse.start();
        frames.run(3.0);
        assert!(playing.is_running());
        assert!(playing.stop());
        assert_eq!(playing.outcome(), Some(Outcome::Stopped));
        assert!(!v.is_animating() && Frames::idle());
        assert!(!playing.stop(), "stopping twice is harmless");
    }

    #[test]
    fn a_spring_settles_carries_velocity_and_a_decay_coasts_to_its_rest() {
        let frames = Frames::new();
        let v = Animated::on(Frames::clock(), 0.0);
        let playing = Animation::spring(v, 100.0, Spring::new(180.0, 8.0)).start();
        let mut peak = 0.0_f64;
        frames.step(0.0);
        for _ in 0..600 {
            frames.step(1.0 / 60.0);
            peak = peak.max(v.get_untracked());
        }
        assert!(peak > 110.0, "a loose spring overshoots: {peak}");
        assert!(playing.is_finished() && close(v.get_untracked(), 100.0));
        assert!(close(v.velocity.get_untracked(), 0.0));

        // A release velocity away from the target carries the value past where it stood.
        v.set(0.0);
        v.set_velocity(-2000.0);
        let back = Animation::spring(v, 0.0, Spring::new(180.0, 26.0)).start();
        frames.step(0.0);
        frames.step(1.0 / 60.0);
        assert!(v.get_untracked() < -10.0);
        frames.run(5.0);
        assert!(back.is_finished() && close(v.get_untracked(), 0.0));

        let decay = Decay { rate: 0.992 };
        v.set(0.0);
        v.set_velocity(1000.0);
        let rest = decay.rest(SpringState {
            value: 0.0,
            velocity: 1000.0,
        });
        assert!((rest - 124.5).abs() < 0.1, "{rest}");
        let fling = Animation::decay(v, decay).start();
        assert!(close(v.target(), rest));
        frames.run(10.0);
        assert!(fling.is_finished());
        assert!(
            (v.get_untracked() - rest).abs() < 1.0,
            "{}",
            v.get_untracked()
        );
    }

    #[test]
    fn the_latest_step_owns_a_value_and_set_takes_it_from_any() {
        let frames = Frames::new();
        let v = Animated::on(Frames::clock(), 0.0);
        let slow = Animation::timing(v, 100.0, 1000, Easing::Linear).start();
        frames.step(0.0);
        frames.step(0.1);
        let quick = Animation::timing(v, -10.0, 100, Easing::Linear).start();
        frames.step(0.0);
        frames.step(0.2);
        assert!(quick.is_finished() && close(v.get_untracked(), -10.0));
        // The displaced step keeps its time and ends without touching the value again.
        frames.run(1.0);
        assert!(slow.is_finished() && close(v.get_untracked(), -10.0));

        let again = Animation::timing(v, 100.0, 1000, Easing::Linear).start();
        frames.step(0.0);
        frames.step(0.1);
        v.set(7.0);
        assert!(!v.is_animating());
        frames.run(1.5);
        assert!(again.is_finished() && close(v.get_untracked(), 7.0));
    }

    #[test]
    fn reset_puts_values_back_and_jump_to_end_lands_them() {
        let frames = Frames::new();
        let (a, b) = (
            Animated::on(Frames::clock(), 1.0),
            Animated::on(Frames::clock(), 2.0),
        );
        let animation = Animation::parallel([
            Animation::timing(a, 10.0, 100, Easing::Linear),
            Animation::sequence([
                Animation::delay(50),
                Animation::spring(b, 20.0, Spring::default()),
            ]),
        ]);
        let playing = animation.start();
        frames.step(0.0);
        frames.step(0.08);
        assert!(a.get_untracked() > 1.0 && b.get_untracked() > 2.0);
        playing.reset();
        assert_eq!(playing.outcome(), Some(Outcome::Stopped));
        assert!(close(a.get_untracked(), 1.0) && close(b.get_untracked(), 2.0));
        assert!(Frames::idle());
        animation.jump_to_end();
        assert!(close(a.get_untracked(), 10.0) && close(b.get_untracked(), 20.0));
    }

    #[test]
    fn an_animation_stops_when_a_value_it_moves_is_disposed() {
        let frames = Frames::new();
        let scope = day_reactive::Scope::root().create_child();
        let v = scope.enter(|| Animated::on(Frames::clock(), 0.0));
        let kept = Animated::on(Frames::clock(), 0.0);
        let playing = Animation::parallel([
            Animation::timing(v, 1.0, 1000, Easing::Linear),
            Animation::timing(kept, 1.0, 1000, Easing::Linear),
        ])
        .looped()
        .start();
        frames.step(0.0);
        frames.step(0.1);
        scope.dispose();
        frames.step(0.1);
        assert_eq!(playing.outcome(), Some(Outcome::Stopped));
        assert!(!kept.is_animating() && Frames::idle());
        // Starting on a value that is already gone ends at once.
        let late = Animation::timing(v, 1.0, 100, Easing::Linear).start_on(Frames::clock());
        assert_eq!(late.outcome(), Some(Outcome::Stopped));
    }

    #[test]
    fn a_group_that_cannot_start_lets_go_of_what_it_started() {
        let _frames = Frames::new();
        let scope = day_reactive::Scope::root().create_child();
        let gone = scope.enter(|| Animated::on(Frames::clock(), 0.0));
        scope.dispose();
        let step = |v: Animated<f64>| Animation::timing(v, 1.0, 100, Easing::Linear);
        // The value that is gone at each position of a group, and inside a nested one.
        for gone_at in 0..3 {
            let live: Vec<Animated<f64>> =
                (0..2).map(|_| Animated::on(Frames::clock(), 0.0)).collect();
            let mut parts: Vec<Animation> = live.iter().map(|v| step(*v)).collect();
            parts.insert(gone_at, step(gone));
            let nested = Animation::parallel([
                Animation::sequence([step(live[0]), step(live[1])]),
                Animation::parallel(parts),
            ]);
            for playing in [
                nested.start(),
                Animation::parallel([step(live[1]), step(gone)]).start(),
            ] {
                assert_eq!(playing.outcome(), Some(Outcome::Stopped), "at {gone_at}");
                assert!(
                    live.iter()
                        .all(|v| !v.is_animating() && close(v.get_untracked(), 0.0)),
                    "a live value stayed claimed (gone at {gone_at})"
                );
            }
        }
        assert!(Frames::idle());
    }

    #[test]
    fn a_repeat_of_zero_plays_nothing_and_a_late_frame_plays_every_pass_it_covers() {
        let frames = Frames::new();
        let v = Animated::on(Frames::clock(), 0.0);
        let none = Animation::timing(v, 10.0, 100, Easing::Linear).repeat(0);
        assert!(!none.is_endless());
        let playing = none.start();
        frames.step(0.0);
        frames.step(0.2);
        assert_eq!(playing.outcome(), Some(Outcome::Finished));
        assert!(close(v.get_untracked(), 0.0) && !v.is_animating());

        // Ten passes of 10 ms, all inside one frame that came 200 ms late.
        let ten = Animation::delay(10).repeat(10).start_on(Frames::clock());
        frames.step(0.0);
        frames.step(0.2);
        assert_eq!(ten.outcome(), Some(Outcome::Finished));

        // A thousand passes of 1 ms fit in one stalled frame of two seconds.
        let ticks = Animated::on(Frames::clock(), 0.0);
        let counted = Animation::sequence([
            Animation::delay(1),
            Animation::timing(ticks, 1.0, 0, Easing::Linear),
        ])
        .repeat(1000)
        .start();
        frames.step(0.0);
        frames.step(2.0);
        assert_eq!(counted.outcome(), Some(Outcome::Finished));

        // A hundred passes that take no time end within the frame and leave the rest of it to
        // the step after them.
        let after = Animation::sequence([Animation::delay(0).repeat(100), Animation::delay(100)])
            .start_on(Frames::clock());
        frames.step(0.0);
        frames.step(0.2);
        assert_eq!(after.outcome(), Some(Outcome::Finished));

        // An endless pass that takes no time at all yields to the next frame instead of spinning.
        let forever = Animation::delay(0).looped().start_on(Frames::clock());
        frames.step(0.0);
        frames.step(0.1);
        assert!(forever.is_running());
        assert!(forever.stop() && Frames::idle());
    }

    #[test]
    fn the_handle_resolves_for_callbacks_and_for_tasks() {
        let frames = Frames::new();
        let v = Animated::on(Frames::clock(), 0.0);
        let playing = Animation::timing(v, 1.0, 100, Easing::Linear).start();
        let heard = Rc::new(Cell::new(None));
        let h = heard.clone();
        playing.on_end(move |outcome| h.set(Some(outcome)));
        let mut task = Box::pin(playing.clone());
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        assert!(task.as_mut().poll(&mut cx).is_pending());
        frames.run(0.2);
        assert_eq!(heard.get(), Some(Outcome::Finished));
        assert_eq!(task.as_mut().poll(&mut cx), Poll::Ready(Outcome::Finished));
        // A listener added after the end hears at once.
        let late = Rc::new(Cell::new(false));
        let l = late.clone();
        playing.on_end(move |_| l.set(true));
        assert!(late.get());
    }

    #[test]
    fn every_write_of_a_frame_reaches_a_binding_in_one_pass() {
        let frames = Frames::new();
        let values: Vec<Animated> = (0..4).map(|_| Animated::on(Frames::clock(), 0.0)).collect();
        let runs = Rc::new(Cell::new(0));
        let (r, read) = (runs.clone(), values.clone());
        day_reactive::Effect::new(move || {
            // One view reading four animated values, as a transform of them would.
            let _sum: f64 = read.iter().map(|v| v.get()).sum();
            r.set(r.get() + 1);
        });
        day_reactive::flush_sync();
        let before = runs.get();
        let playing = Animation::parallel(
            values
                .iter()
                .map(|&v| Animation::timing(v, 1.0, 1000, Easing::Linear)),
        )
        .start();
        frames.step(0.0);
        let started = runs.get();
        assert!(started <= before + 2);
        for _ in 0..10 {
            frames.step(1.0 / 60.0);
        }
        // Ten frames moved four values each; the reader ran once per frame.
        assert_eq!(runs.get() - started, 10);
        playing.stop();
    }

    #[test]
    fn interpolation_clamps_extends_and_blends_colors() {
        let fade = Interpolation::new([0.0, 120.0], [1.0, 0.0]);
        assert!(close(fade.at(60.0), 0.5));
        assert!(close(fade.at(-30.0), 1.0) && close(fade.at(500.0), 0.0));
        let open = fade.clone().extend();
        assert!(close(open.at(-60.0), 1.5) && close(open.at(240.0), -1.0));
        let one_way = Interpolation::new([0.0, 1.0], [0.0, 10.0]).extend_end();
        assert!(close(one_way.at(-1.0), 0.0) && close(one_way.at(2.0), 20.0));

        let steps = Interpolation::new([0.0, 1.0, 3.0], [0.0, 10.0, 30.0]);
        assert!(close(steps.at(0.5), 5.0) && close(steps.at(2.0), 20.0));
        assert!(close(steps.at(1.0), 10.0) && close(steps.at(9.0), 30.0));
        assert!(close(Interpolation::new([4.0], [7.0]).at(100.0), 7.0));

        let (blue, yellow) = (Color::rgb(0.0, 0.0, 1.0), Color::rgb(1.0, 1.0, 0.0));
        let heat = Interpolation::new([0.0, 1.0], [blue, yellow]);
        assert_eq!(heat.at(0.0), blue);
        assert_eq!(heat.at(1.0), yellow);
        assert_eq!(heat.at(0.5), crate::tween::mix_oklab(blue, yellow, 0.5));
        assert_eq!(heat.at(7.0), yellow);

        let _frames = Frames::new();
        let scroll = Animated::on(Frames::clock(), 60.0);
        let opacity = scroll.interpolate([0.0, 120.0], [1.0, 0.0]);
        assert!(close(opacity(), 0.5));
        scroll.set(120.0);
        assert!(close(opacity(), 0.0));
    }

    #[test]
    fn a_spring_and_its_curve_describe_the_same_motion() {
        let spring = Spring::from_response(0.5, 0.4);
        let Curve::Spring { response, damping } = spring.curve() else {
            panic!("a spring's curve is a spring");
        };
        assert!(close(response, 0.5) && close(damping, 0.4));
        // The stepped spring and the closed form agree along the way.
        let spec = AnimSpec::spring(0.5, 0.4);
        let mut state = SpringState::default();
        for i in 1..=60 {
            spring.step(&mut state, 1.0, 0.01);
            let exact = spec.curve.fraction(f64::from(i) * 0.01, 0.0);
            assert!(
                (state.value - exact).abs() < 0.01,
                "{i}: {} {exact}",
                state.value
            );
        }
    }
}
