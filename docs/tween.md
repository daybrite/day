---
title: "Motion and tweening"
description: "Animated values, composed animations, interpolation and the AnimSpec contract every toolkit keeps."
---

<!-- Copyright © The Daybrite Project
     SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Motion and tweening

Day animates in two ways, and they share one vocabulary.

- **The platform animates.** A change made inside
  [`with_animation`](https://docs.rs/day-core/latest/day_core/fn.with_animation.html) or under
  [`.animation(..)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.animation)
  reaches the toolkit with an [`AnimSpec`](https://docs.rs/day-spec/latest/day_spec/struct.AnimSpec.html), and the toolkit plays it on
  its own animator. [The animation contract](#the-animation-contract) says exactly what it plays.
- **Day animates.** `day::motion` steps values on the window's [frame clock](frames.md), and
  views or a [canvas](canvas.md) read them. This is the path for a staggered entrance, a spring
  that starts at the speed a finger released it with, a value that drives several properties
  through an interpolation, and anything a canvas draws.

```rust,ignore
use day::motion::{Animated, Animation, Ease, Easing, Spring};

let fade = Animated::new(1.0);
let lift = Animated::new(0.0);
column((
    label(res::str::title()).opacity(fade).translation(0.0, lift),
    button(res::str::hide()).action(move || {
        Animation::parallel([
            Animation::timing(fade, 0.0, 220, Easing::EASE_IN_OUT),
            Animation::spring(lift, -24.0, Spring::new(320.0, 22.0)),
        ])
        .start();
    }),
))
```

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Animated

[`Animated<T>`](https://docs.rs/day-core/latest/day_core/motion/struct.Animated.html) is a reactive value that animations move. `T` is
a number by default and can be anything with a [`Lerp`](#lerp): a color, a point, a rect, a
`Vec` of them. It is the one animated-value type in Day.

- [`get()`](https://docs.rs/day-core/latest/day_core/motion/struct.Animated.html#method.get) is a tracked read, so whatever reads the
  value follows its motion and nothing else re-runs. An `Animated<T>` is also accepted wherever a
  modifier takes a reactive `T`: [`.opacity(fade)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.opacity), [`.translation(0.0, lift)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.translation),
  [`.scale(zoom)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.scale), [`.rotation(turn)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.rotation), [`.background(tint)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.background).
- A number carries a velocity ([`velocity()`](https://docs.rs/day-core/latest/day_core/motion/struct.Animated.html#method.velocity),
  [`set_velocity(v)`](https://docs.rs/day-core/latest/day_core/motion/struct.Animated.html#method.set_velocity)). A spring or a decay
  starts from it, which is what makes a retarget bend the motion and lets a gesture hand over
  its release speed.
- [`set(value)`](https://docs.rs/day-core/latest/day_core/motion/struct.Animated.html#method.set) puts the value at rest and takes it
  from any animation moving it. [`target()`](https://docs.rs/day-core/latest/day_core/motion/struct.Animated.html#method.target) is
  where it is headed, and [`is_animating()`](https://docs.rs/day-core/latest/day_core/motion/struct.Animated.html#method.is_animating)
  is a tracked read of whether something is moving it.
- [`animate_to(target, spec)`](https://docs.rs/day-core/latest/day_core/motion/struct.Animated.html#method.animate_to) is the short
  form for one value and one `AnimSpec`: a canvas bar that glides to each new height. Calling it
  again mid-flight starts from the value on screen.
- The value is `Copy`, like a `Signal`, and belongs to the reactive scope it was made in.
  [`Animated::new`](https://docs.rs/day-core/latest/day_core/motion/struct.Animated.html#method.new) animates on the current page's
  window; [`Animated::on(clock, ..)`](https://docs.rs/day-core/latest/day_core/motion/struct.Animated.html#method.on) names one.

One owner at a time: the latest animation step or `set` to touch a value owns it, and an earlier
step on the same value ends without moving it further.

## Animation

An [`Animation`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html) is one step or a composition of steps.
Building one moves nothing, and it can be started any number of times.

| Step | What it does |
|---|---|
| [`Animation::timing(value, to, ms, easing)`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.timing) | moves `value` to `to` over `ms` milliseconds along an [`Easing`](#easing-springs-and-decay) |
| [`Animation::tween(value, to, spec)`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.tween) | moves `value` to `to` as an `AnimSpec` says: delay, curve or spring, repeats, autoreverse |
| [`Animation::spring(value, to, spring)`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.spring) | springs a number to `to` from the velocity it has, until it rests |
| [`Animation::decay(value, decay)`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.decay) | lets a number coast from the velocity it has until it stops |
| [`Animation::drive(spec, step)`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.drive) | calls `step` with each frame's [`AnimSample`](https://docs.rs/day-spec/latest/day_spec/struct.AnimSample.html) of `spec`, for a transition that is more than one value |
| [`Animation::delay(ms)`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.delay) | waits |

| Composition | What it does |
|---|---|
| [`Animation::sequence(steps)`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.sequence) | runs the steps one after another; each starts from where its value stands when its turn comes |
| [`Animation::parallel(steps)`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.parallel) | runs them together and ends with the last |
| [`Animation::stagger(gap_ms, steps)`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.stagger) | runs them together, each starting `gap_ms` after the one before |
| [`.repeat(times)`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.repeat) | plays the animation `times` times in a row |
| [`.looped()`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.looped) | plays it again each time it ends, until it is stopped |

```rust,ignore
let lifts: Vec<Animated> = (0..5).map(|_| Animated::new(0.0)).collect();
let entrance = Animation::stagger(
    70,
    lifts.iter().map(|&lift| {
        Animation::sequence([
            Animation::timing(lift, -46.0, 220, Easing::Power(Ease::Out, 2.0)),
            Animation::spring(lift, 0.0, Spring::new(320.0, 11.0)),
        ])
    }),
);
```

A step that ends part-way through a frame hands the rest of the frame to the step after it, so a
sequence, a stagger and a loop keep exact time however the frames fall.

### Playing

[`start()`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.start) plays the animation on the window of
the first value it moves and answers a [`Playing`](https://docs.rs/day-core/latest/day_core/motion/struct.Playing.html) handle;
[`start_on(clock)`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.start_on) names the window.

- [`stop()`](https://docs.rs/day-core/latest/day_core/motion/struct.Playing.html#method.stop) stops it where it stands.
- [`reset()`](https://docs.rs/day-core/latest/day_core/motion/struct.Playing.html#method.reset) stops it and puts every value back
  where it was when it started.
- Awaiting the handle answers an [`Outcome`](https://docs.rs/day-core/latest/day_core/motion/enum.Outcome.html): `Finished` when the
  animation played to its end, `Stopped` when something ended it first.
  [`on_end(f)`](https://docs.rs/day-core/latest/day_core/motion/struct.Playing.html#method.on_end) is the callback form.
- [`jump_to_end()`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.jump_to_end) on the animation puts
  every value on its end state without playing.

```rust,ignore
let playing = entrance.start();
day::task(async move {
    if playing.await == Outcome::Finished {
        status.set(Status::Arrived);
    }
});
```

The animation does not depend on the handle: dropping it leaves the animation running.

### What the framework handles

- **Reduced motion** ([accessibility](accessibility.md): the user's system setting, or a run that
  forced it with `--fast` / `DAY_REDUCE_MOTION=1`). A finite animation puts every value on its
  end state at once and reports `Finished`, also when the setting flips mid-flight. An animation
  that never ends by itself (a [`looped()`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.looped) pulse, a spec that repeats forever) keeps running: a
  spinner is the one sign that work is going on. Check
  [`day::reduce_motion()`](https://docs.rs/day-app/latest/day/fn.reduce_motion.html) before
  starting a loop that is decoration only.
- **Scope cleanup.** An animation stops, with `Stopped`, when a value it moves is disposed with
  its scope or its window closes. A page needs no cleanup of its own.
- **One pass per frame.** Every animation of a window advances inside one batch per display
  frame, so a view bound to several animated values rebuilds its `Transform` once per frame and
  the toolkit receives one `set_transform`, however many values moved. The window requests no
  frames once nothing is animating.

## Interpolation

[`Interpolation::new(input, output)`](https://docs.rs/day-core/latest/day_core/motion/struct.Interpolation.html) maps one range onto
another through any number of stops: `[0, 120]` of scroll onto `[1, 0]` of opacity. The output is
anything with a `Lerp`, so the same call maps onto colors, which blend in OKLab. Outside the
input range the map clamps; [`.extend()`](https://docs.rs/day-core/latest/day_core/motion/struct.Interpolation.html#method.extend)
lets the outer segments run on (`extend_start` and `extend_end` choose a side).

[`Animated::interpolate(input, output)`](https://docs.rs/day-core/latest/day_core/motion/struct.Animated.html#method.interpolate) is
the reactive form: a closure for any modifier that takes a reactive value, so one animated number
drives several properties.

```rust,ignore
let scroll = Animated::new(0.0);
header
    .opacity(scroll.interpolate([0.0, 120.0], [1.0, 0.0]))
    .background(scroll.interpolate([0.0, 120.0], [Color::WHITE, accent]));
```

## The animation contract

An [`AnimSpec`](https://docs.rs/day-spec/latest/day_spec/struct.AnimSpec.html) means the same thing on every toolkit and on the
frame-driven path. A change from the value on screen (the start) to the new value (the target)
plays as follows.

| Field | Meaning |
|---|---|
| `delay_ms` | The start value is held this long before the first cycle. Served once, never between repeats. |
| `duration_ms` | The length of one cycle. Zero applies the target at once (after the delay). |
| `curve` | The shape of one cycle: `Linear` is `t`, `EaseIn` is `t²`, `EaseOut` is `1 - (1 - t)²`, `EaseInOut` is `t²(3 - 2t)`, and `Spring` is a physical spring. |
| `repeat` | Cycles beyond the first. `u32::MAX` repeats until the property is next changed. |
| `autoreverse` | Every second cycle plays backwards, from the target to the start. |

- **Rest.** When a finite run ends the property rests on the target, whatever the last cycle did.
  A run that ends on a backward cycle (autoreverse with an odd `repeat`) is followed by a jump to
  the target; use an even `repeat` to end on a forward one.
- **Spring.** `Curve::Spring { response, damping }` is the spring of SwiftUI's
  `.spring(response:dampingFraction:)`: `response` is the period of its undamped oscillation in
  seconds and `damping` its damping ratio (1 is critically damped, below 1 overshoots). Its
  position is the closed-form response [`Curve::fraction`](https://docs.rs/day-spec/latest/day_spec/enum.Curve.html#method.fraction)
  evaluates. [`AnimSpec::spring`](https://docs.rs/day-spec/latest/day_spec/struct.AnimSpec.html#method.spring) sets `duration_ms` to
  the spring's settle time ([`Curve::settle_secs`](https://docs.rs/day-spec/latest/day_spec/enum.Curve.html#method.settle_secs): the
  time after which it stays within a thousandth of its travel), and the cycle ends exactly on its
  end value. A spring is therefore longer than its response:
[`spring(0.4, 0.8)`](https://docs.rs/day-spec/latest/day_spec/struct.AnimSpec.html#method.spring) settles in
  about 0.59 s.
- **Retarget.** A new change to a property in flight starts from the value on screen. A change
  with no animation cancels the run and applies at once.
- **What animates.** Frame, opacity, transform, and a container's background color, whose
  in-between colors lie between the two ends in every channel.

[`AnimSpec::sample(elapsed)`](https://docs.rs/day-spec/latest/day_spec/struct.AnimSpec.html#method.sample) is this timeline as a
function: the progress `elapsed` seconds after the change and whether the run is over.
[`total_secs()`](https://docs.rs/day-spec/latest/day_spec/struct.AnimSpec.html#method.total_secs) is the run's length, or `None` for
a spec that repeats forever. [`Animation::tween`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.tween) and
[`Animated::animate_to`](https://docs.rs/day-core/latest/day_core/motion/struct.Animated.html#method.animate_to) run on it, so a canvas
value moved with the spec a native property is animated with moves in step with it.

A toolkit plays the contract on its own animator. Where the platform has a spring animator it
takes [`Curve::spring_constants`](https://docs.rs/day-spec/latest/day_spec/enum.Curve.html#method.spring_constants) (stiffness and
damping for a mass of 1); everywhere else it plays
[`AnimSpec::keyframes()`](https://docs.rs/day-spec/latest/day_spec/struct.AnimSpec.html#method.keyframes), one forward cycle sampled
sixty times a second, through a keyframe animation or a custom interpolator, so every toolkit
draws the same spring.
[`day::motion::Spring::from_response(response, damping)`](https://docs.rs/day-core/latest/day_core/motion/struct.Spring.html#method.from_response)
is the same spring for the frame-driven path, and
[`spring.curve()`](https://docs.rs/day-core/latest/day_core/motion/struct.Spring.html#method.curve) converts back.

### Toolkit support

`Cap::Animation` says whether the toolkit animates at all; where it is `Unsupported` an animated
change applies at once. The conformance cases `animation-delay-holds-the-start`,
`animation-repeat-plays-every-cycle`, `animation-autoreverse-plays-back`,
`animation-repeats-forever-until-set`, `animation-retargets-from-the-value-shown`,
`animation-spring-overshoots-and-settles`, `animation-frame-passes-between` and
`animation-background-passes-between` read each property mid-flight through
[`assert_displayed`](testing.md#interpret-a-result) and compare it with
[`AnimSpec::sample`](https://docs.rs/day-spec/latest/day_spec/struct.AnimSpec.html#method.sample).

| Toolkit | Delay | Repeat and autoreverse | Spring | Background color | Mid-flight read-back | Limitations |
|---|---|---|---|---|---|---|
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | yes | yes | `CASpringAnimation` from macOS 14; Day's keyframes for an over-damped spring on macOS 13 and for a spring that scales or turns | yes, on the view's layer for the run | opacity, frame, translation, background | A gradient fill, and a change to or from no fill, apply at once. |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | yes | yes | `CASpringAnimation`; Day's keyframes for a background color | yes | opacity, frame, translation, background | A change to or from no color applies at once. |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | yes | yes | the physical spring, sampled each frame | yes, blended per sRGB channel | opacity, frame, translation, background | Day steps every animated property on the frame clock, since GTK has no property animator, so animations run on the main thread and stall with it. A spring's opacity overshoot is clipped to 0 to 1. A fill animation rewrites the surface's CSS each frame. An unmapped widget, a list row's content and a stacked navigation host take a new frame at once. A transform interpolates translation, scale and rotation one component at a time, in a straight line. |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | yes | yes | Day's keyframes | yes | opacity, frame, translation, background | A host whose frame Qt owns (navigation, suite, split, inspector, list) takes a new frame at once. A transform that draws outside the widget is clipped to the parent widget's rect. |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | yes | yes | Day's keyframes in a storyboard | yes | opacity, frame, translation, background | |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | yes | yes | Day's keyframes through an interpolator | yes | opacity, frame, translation, background | A view's first placement applies at once. |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | yes | yes; a finite run plays as one curve over all its cycles | Day's spring as a custom curve | yes | none: the C API reads back the values set, not the values shown | A change to or from no background applies at once. |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | yes | yes | Day's keyframes in a Web Animation | yes | opacity, frame, translation, background | A background from no fill applies at once. |

## Lerp

`Lerp` is how a value moves between two states: [`a.lerp(&b, t)`](https://docs.rs/day-core/latest/day_core/tween/fn.lerp.html) is `a` at `t = 0` and `b` at
`t = 1`. It is implemented for `f64`, `f32`, `Point`, `Size`, `Rect`, `Color`, `Ink`, `Angle`, tuples of two and
three, arrays and `Vec`s (element-wise over the common prefix, the target's tail taken as is), and
`Option` (interpolated when both are present, switched at the midpoint otherwise). `t` may leave
`0..=1`: an under-damped spring overshoots, and a number follows it past the target and back.

Colors blend in OKLab, alpha linearly. The midpoint of blue and yellow is a clean mid-lightness
color rather than sRGB's dim gray, and a sequential scale's in-between colors read as in-between
values. The endpoints are returned exactly. An `Ink` blends each appearance separately, so a pair
mid-flight still follows an appearance switch, and a role is resolved against both appearances
first; the endpoints are the inks themselves ([color.md](color.md) "Inks in motion"). An `Angle` moves
along the number line, not the short way round: normalize first for the short arc. [`mix_oklab(a, b, t)`](https://docs.rs/day-core/latest/day_core/tween/fn.mix_oklab.html) is the same blend as a function.

[`stagger(spec, index, count, spread_ms)`](https://docs.rs/day-core/latest/day_core/tween/fn.stagger.html) delays an `AnimSpec` for item `index` of `count` so the
first starts on time and the last `spread_ms` later: one spec per bar of a chart, for a
`with_animation` or a `tween` each.

## Pairs

When what changed is a collection (the bars of a chart, the cards on a table, the tiles of a
board), the first question is which item became which. [`Pairs::by_key(from, to, key, absent)`](https://docs.rs/day-core/latest/day_core/tween/struct.Pairs.html#method.by_key)
answers it once per change:

```rust,ignore
use day::tween::{Change, Pairs};

let pairs = Pairs::by_key(&before, &after, |bar| bar.name.clone(), |bar, change| match change {
    Change::Entering => bar.at_baseline(), // grows from zero
    _ => bar.at_baseline().faded(),        // shrinks away
});
// Each frame:
let bars = pairs.at(progress);
```

- An item whose key is on both sides is `Kept` and moves from where it was to where it is. Keys may
  repeat: the *n*th item with a key on one side matches the *n*th on the other, so identical tokens
  pair up in order.
- An item only in `to` is `Entering`, and `absent(item, Change::Entering)` says where it starts. One
  only in `from` is `Leaving`, and `absent(item, Change::Leaving)` says where it goes. That stand-in
  is the whole meaning of arriving and departing (grown from a baseline, slid in beside a
  neighbor, faded) and it is the caller's to choose.
- The result is in `to`'s order, then the leaving items in `from`'s order.

[`at(t)`](https://docs.rs/day-core/latest/day_core/tween/struct.Pairs.html#method.at) interpolates every item with its `Lerp`. [`at_with(t, f)`](https://docs.rs/day-core/latest/day_core/tween/struct.Pairs.html#method.at_with) hands each `Pair` and `t` to a
closure, for items whose in-between needs more context than their two ends. [`at_each(progress, f)`](https://docs.rs/day-core/latest/day_core/tween/struct.Pairs.html#method.at_each)
gives each item its own `t`, for sequences that do not move in lockstep. [`settled()`](https://docs.rs/day-core/latest/day_core/tween/struct.Pairs.html#method.settled) is the
arriving state without the leaving items: what to draw once the transition is over.

When the caller's own rules already know what moved where (a sliding-tile game knows which two
tiles merged into which cell, which no key match could say), it collects its `Pair`s directly:
`Pairs` implements `FromIterator<Pair<T>>`.

## Composite transitions

A chart, a graph or a map changes many values at once, and what they mean decides how they should
move. day-piece-charts is the worked example: its `animate.rs` pairs marks and axis ticks across
two resolved charts with [`Pairs::by_key`](https://docs.rs/day-core/latest/day_core/tween/struct.Pairs.html#method.by_key), keyed by what they encode, interpolates values in data space while the scales' domains move,
re-stacks stacked marks every frame, and gives entering and leaving marks a meaning (a bar grows
from its baseline, a line's new samples unfold along the old line). It drives the whole blend
with one [`Animation::drive`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.drive) per change and reads [`current_anim()`](https://docs.rs/day-core/latest/day_core/fn.current_anim.html) so that a change made inside
[`with_animation`](https://docs.rs/day-core/latest/day_core/fn.with_animation.html) animates with the caller's spec, the same contract native widgets keep.

## Easing, springs and decay

`Animation` steps are built from three small values, and an app can also step them itself:

- **A curve the native animators do not carry.** [`Easing`](https://docs.rs/day-core/latest/day_core/motion/enum.Easing.html) maps progress to eased progress for
  linear, a CSS cubic Bézier, power, sine, circle, exponential, back, elastic and bounce curves,
  in, out or both ways where that applies.
  [`Easing::Back(Ease::Out, 1.70158).value(t)`](https://docs.rs/day-core/latest/day_core/motion/enum.Easing.html#method.value) overshoots by a
  tenth before it settles; [`samples(n)`](https://docs.rs/day-core/latest/day_core/motion/enum.Easing.html#method.samples) gives the curve as
  points for plotting it.
- **A spring that keeps its velocity.** [`Spring`](https://docs.rs/day-core/latest/day_core/motion/struct.Spring.html) has a stiffness, a damping and a mass, and
  [`spring.step(&mut state, target, dt)`](https://docs.rs/day-core/latest/day_core/motion/struct.Spring.html#method.step) advances a [`SpringState`](https://docs.rs/day-core/latest/day_core/motion/struct.SpringState.html) (a value and its velocity)
  by one frame. Because the velocity lives in the state, a spring started with the speed a finger
  released a view at carries that speed on, and a new target bends the motion instead of
  restarting it. `settled` says when to stop asking for frames.
- **A fling that coasts.** [`Decay`](https://docs.rs/day-core/latest/day_core/motion/struct.Decay.html) loses a fixed share of the velocity per millisecond, as a scroll
  view decelerates.

They are plain numbers: nothing in them touches a view or a clock.
[`Animation::timing`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.timing),
[`Animation::spring`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.spring) and
[`Animation::decay`](https://docs.rs/day-core/latest/day_core/motion/struct.Animation.html#method.decay) are the usual way to run them. A simulation with rules
of its own (a drag that springs back only past a threshold, bodies that collide) steps them from
a [`FrameClock`](frames.md) subscription with the frame's `delta` and writes the result into an
`Animated` or a signal:

```rust
use day::motion::{Spring, SpringState};

let spring = Spring::new(180.0, 11.0);
let mut x = SpringState { value: start, velocity: release_velocity };
FrameClock::current().subscribe(move |frame| {
    spring.step(&mut x, 0.0, frame.delta.as_secs_f64());
    offset.set(x.value);
    if spring.settled(x, 0.0) { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
});
```

Motion stepped this way is the app's own: read
[`day::reduce_motion()`](https://docs.rs/day-core/latest/day_core/fn.reduce_motion.html) and write the target at
once instead of starting the subscription.

Tests live in `day-core/src/motion.rs` (the driver on a manual frame source: timing, sequences
that carry time across steps, stagger, repeat and loop, springs and decay with velocity,
ownership of a value, reset, scope disposal, the awaitable handle, one binding pass per frame,
interpolation), `day-spec` (`anim_tests`: the timeline across delay, repeat, autoreverse and
springs, settle time, keyframes) and `day-core/src/tween.rs` (interpolation and OKLab endpoints,
staggering, and `Pairs`: classification and order, the stand-in callback, repeated keys, empty
sides, permutations, sampling past the end, per-item progress, many-to-one pairs collected
directly, and a randomized check that every item of either side is accounted for exactly once).
