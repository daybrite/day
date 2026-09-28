---
title: "Tweening"
description: "Interpolation, timing and frame-driven transitions for canvas animations."
---

<!-- Copyright © The Daybrite Project
     SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Tweening

Native widgets animate through `with_animation`: Day hands the toolkit an `AnimSpec` and the
toolkit animates on its own compositor. A canvas has no compositor to hand anything to. It
re-records a display list, so something has to decide, frame by frame, what the in-between picture
is. `day::tween` is that something, in parts that work on their own or together.

```rust,ignore
use day::tween::{Lerp, Tweened};

let height = Tweened::new(0.0);
button("Grow").action(move || height.animate_to(120.0, AnimSpec::spring(0.4, 0.8)));
canvas(move |d, _| d.fill(Shape::Rect(Rect::new(0.0, 0.0, 40.0, height.get())), BLUE));
```

## Lerp

`Lerp` is how a value moves between two states: `a.lerp(&b, t)` is `a` at `t = 0` and `b` at
`t = 1`. It is implemented for `f64`, `f32`, `Point`, `Size`, `Rect`, `Color`, tuples of two and
three, arrays and `Vec`s (element-wise over the common prefix, the target's tail taken as is), and
`Option` (interpolated when both are present, switched at the midpoint otherwise). `t` may leave
`0..=1`: an under-damped spring overshoots, and a number follows it past the target and back.

Colors blend in OKLab, alpha linearly. The midpoint of blue and yellow is a clean mid-lightness
color rather than sRGB's dim gray, and a sequential scale's in-between colors read as in-between
values. The endpoints are returned exactly. `mix_oklab(a, b, t)` is the same blend as a function.

## Timing

`Timing::new(spec).sample(elapsed)` turns an `AnimSpec` into progress: the delay holds the start,
easing curves run for `duration_ms`, a spring runs its analytic step response and ends at
`duration_ms` (which `AnimSpec::spring` sets to its response, the same duration every native
backend gives a spring), and `repeat` and `autoreverse` cycle it. Each `Sample` says whether it is
the last one; the last one is the exact end value, so nothing is left a spring's residue away from
its target. `total()` is the run's length, or `None` for a spec that repeats forever.

`stagger(spec, index, count, spread_ms)` delays item `index` of `count` so the first starts on
time and the last `spread_ms` later: bars rising left to right, points arriving along a line.

## animate

`animate(clock, timing, step)` calls `step(sample)` once per display frame of a
[`FrameClock`](frames.md) until the timing is done, then stops, and the window requests no more
frames on its behalf. Elapsed time is measured from the first frame delivered rather than from the
call, so a transition started while the app is busy begins on screen instead of part-way through.
It returns the `FrameHandle`: keep it, or call `in_scope()`, since dropping it cancels the
transition where it stands.

## Tweened

`Tweened<T: Lerp>` is a reactive value that glides to each new target. `get()` is tracked, so a
canvas reading it re-records on every frame of its transition and at no other time.
`animate_to(target, spec)` starts from the value on screen, so retargeting mid-flight bends the
motion rather than snapping it; `set(value)` jumps. `Tweened::new` animates on the current page's
window; `Tweened::on(clock, ..)` names one.

## Pairs

When what changed is a collection (the bars of a chart, the cards on a table, the tiles of a
board), the first question is which item became which. `Pairs::by_key(from, to, key, absent)`
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

`at(t)` interpolates every item with its `Lerp`. `at_with(t, f)` hands each `Pair` and `t` to a
closure, for items whose in-between needs more context than their two ends. `at_each(progress, f)`
gives each item its own `t`, for sequences that do not move in lockstep. `settled()` is the
arriving state without the leaving items: what to draw once the transition is over.

When the caller's own rules already know what moved where (a sliding-tile game knows which two
tiles merged into which cell, which no key match could say), it collects its `Pair`s directly:
`Pairs` implements `FromIterator<Pair<T>>`.

## Composite transitions

A chart, a graph or a map changes many values at once, and what they mean decides how they should
move. day-piece-charts is the worked example: its `animate.rs` pairs marks and axis ticks across
two resolved charts with `Pairs::by_key`, keyed by what they encode, interpolates values in data space while the scales' domains move,
re-stacks stacked marks every frame, and gives entering and leaving marks a meaning (a bar grows
from its baseline, a line's new samples unfold along the old line). It drives the whole blend
with one `animate` call per change and reads `current_anim()` so that a change made inside
`with_animation` animates with the caller's spec, the same contract native widgets keep.

Tests live in `day-core/src/tween.rs`: timing across delay, repeat, autoreverse and springs,
staggering, interpolation and OKLab endpoints, a `Tweened` driven by a manual frame source
through a retarget to arrival with no frame requested afterwards, and `Pairs`: classification and
order, the stand-in callback, repeated keys, empty sides, permutations, sampling past the end,
per-item progress, many-to-one pairs collected directly, and a randomized check that every item of
either side is accounted for exactly once.
