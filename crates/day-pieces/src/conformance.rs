// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The built-in pieces' conformance cases, gathered (docs/testing.md). Each piece's cases live
//! in a `mod conformance` at the end of the file that defines it, next to the constructors they
//! prove; the attribute discovers them automatically. This module holds cases that belong
//! to no piece and builds the conformance app's browsing page.
//! Compiled only under the `conformance` feature, so a shipping app links none of it.

use day_core::conformance::{Case, Drive};
use day_core::*;
use day_reactive::Signal;

use crate::*;

/// The cases that belong to no single piece.
mod general {
    use super::*;

    /// A headless test: no page, app state checked on the app's own thread, in its own process.
    #[day_macros::test(day_core)]
    fn headless_sanity() -> Case {
        Case::headless().run(|t: Drive| async move {
            let count = Signal::new(1i64);
            count.update(|n| *n += 1);
            t.check_eq(count.get(), 2)?;
            day_core::sleep(1).await;
            t.check_eq(count.get(), 2)
        })
    }

    /// The mock runs posts, timers and blocking jobs inline on the test thread (and the web
    /// has no threads), so the cases below assert less about time and threads there.
    fn synchronous_backend() -> bool {
        day_core::backend_name() == Some("mock-mock")
    }

    /// Poll `done` between short sleeps until it holds or a few seconds pass: an emulator in
    /// CI is slow, so no case judges a timer against a tight ceiling.
    async fn settle(
        t: &Drive,
        what: &str,
        done: impl Fn() -> bool,
    ) -> day_core::conformance::TestResult {
        for _ in 0..400 {
            if done() {
                return Ok(());
            }
            day_core::sleep(20).await;
        }
        t.check(done(), what)
    }

    /// `day::sleep` is the toolkit's own timer (docs/async.md): it resolves after at least its
    /// delay, back on the UI thread where the task reads and writes signals as before.
    #[day_macros::test(day_core)]
    fn sleep_resolves_after_its_delay() -> Case {
        Case::headless().run(|t: Drive| async move {
            let thread = std::thread::current().id();
            let count = Signal::new(0i64);
            // Exercise both sides of Windows' timer tick boundaries and repeat the longer
            // delay: a single sample can miss a WM_TIMER delivered before its deadline.
            for ms in [1, 10, 15, 16, 31, 120, 120, 120] {
                #[cfg(not(target_arch = "wasm32"))]
                let started = std::time::Instant::now();
                day_core::sleep(ms).await;
                #[cfg(not(target_arch = "wasm32"))]
                if !synchronous_backend() {
                    // The one millisecond of slack covers the timer's clock and `Instant`
                    // ticking apart; the backend must not round down to a system tick.
                    let slept = started.elapsed();
                    t.check(
                        slept >= std::time::Duration::from_millis(u64::from(ms - 1)),
                        &format!("a {ms} ms sleep resolved after {slept:?}"),
                    )?;
                }
            }
            t.check(
                std::thread::current().id() == thread,
                "the task resumed on another thread",
            )?;
            count.set(1);
            t.check_eq(count.get(), 1)
        })
    }

    /// A zero delay still yields: the rest of the task runs on a later turn, after the code
    /// that followed `task(..)`.
    #[day_macros::test(day_core)]
    fn sleep_zero_yields_the_turn() -> Case {
        Case::headless().run(|t: Drive| async move {
            let flag = Signal::new(false);
            let seen = Signal::new(None::<bool>);
            day_core::task(async move {
                day_core::sleep(0).await;
                seen.set(Some(flag.get()));
            });
            // Runs after the task's eager first poll parked it on the timer.
            flag.set(true);
            settle(&t, "the zero sleep never resolved", move || {
                seen.get().is_some()
            })
            .await?;
            if synchronous_backend() {
                return Ok(());
            }
            t.check_eq(seen.get(), Some(true))
        })
    }

    /// Timers resolve in delay order whatever order they were armed in, and many at once.
    #[day_macros::test(day_core)]
    fn sleeps_resolve_in_delay_order() -> Case {
        Case::headless().run(|t: Drive| async move {
            let order = Signal::new(Vec::<u32>::new());
            for ms in [200u32, 10, 100, 50] {
                day_core::task(async move {
                    day_core::sleep(ms).await;
                    order.update(|o| o.push(ms));
                });
            }
            let done = Signal::new(0usize);
            for i in 0..64u32 {
                day_core::task(async move {
                    day_core::sleep(1 + (i * 7) % 80).await;
                    done.update(|d| *d += 1);
                });
            }
            settle(&t, "not every sleep resolved", move || {
                order.with(std::vec::Vec::len) == 4 && done.get() == 64
            })
            .await?;
            if synchronous_backend() {
                return Ok(());
            }
            t.check_eq(order.get(), vec![10, 50, 100, 200])
        })
    }

    /// Aborting a task parked on a sleep drops the sleep: the timer may still fire natively,
    /// but nothing resumes.
    #[day_macros::test(day_core)]
    fn an_aborted_task_never_wakes_from_its_sleep() -> Case {
        Case::headless().run(|t: Drive| async move {
            let fired = Signal::new(false);
            let handle = day_core::task(async move {
                day_core::sleep(60).await;
                fired.set(true);
            });
            handle.abort();
            day_core::sleep(250).await;
            if synchronous_backend() {
                // Inline timers resolved the sleep before the abort could; nothing to judge.
                return Ok(());
            }
            t.check(!fired.get(), "an aborted task resumed after its sleep")
        })
    }

    /// `spawn_blocking` runs its job on a pool thread and resolves back on the UI thread, where
    /// the value lands in a signal like any other.
    #[day_macros::test(day_core)]
    fn a_blocking_job_runs_off_the_ui_thread_and_resolves_on_it() -> Case {
        Case::headless().run(|t: Drive| async move {
            let ui = std::thread::current().id();
            let answer = Signal::new(0i64);
            let got = day_core::spawn_blocking(move || {
                let elsewhere = std::thread::current().id() != ui;
                (elsewhere, (1..=9i64).product::<i64>())
            })
            .await;
            let (elsewhere, value) = got.map_err(|e| day_core::conformance::Fail(e.to_string()))?;
            t.check(
                std::thread::current().id() == ui,
                "the await resumed off the UI thread",
            )?;
            t.check_eq(elsewhere, !day_core::blocking_inline())?;
            answer.set(value);
            t.check_eq(answer.get(), 362_880)
        })
    }

    /// A job that panics is an error on the awaiting side, and the pool keeps serving. Not on
    /// the web, where a panic aborts the whole instance before anything could catch it.
    #[day_macros::test(day_core)]
    fn a_panicking_blocking_job_is_an_error() -> Case {
        Case::headless().run(|t: Drive| async move {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let failed: Result<(), _> =
                    day_core::spawn_blocking(|| panic!("the job failed on purpose")).await;
                t.check_eq(failed, Err(day_core::BlockingError::Panicked))?;
            }
            let next = day_core::spawn_blocking(|| 7).await;
            t.check_eq(next, Ok(7))
        })
    }

    /// Dropping the future before the job started cancels the job: it never runs.
    #[day_macros::test(day_core)]
    fn a_blocking_job_dropped_before_it_starts_never_runs() -> Case {
        Case::headless().run(|t: Drive| async move {
            use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
            use std::sync::{Arc, Mutex, mpsc};
            let ran = Arc::new(AtomicBool::new(false));
            if day_core::blocking_inline() {
                // Inline jobs run at the first poll; a future never polled never runs.
                let marker = ran.clone();
                drop(day_core::spawn_blocking(move || {
                    marker.store(true, Ordering::SeqCst);
                }));
                let _ = day_core::spawn_blocking(|| ()).await;
                return t.check(!ran.load(Ordering::SeqCst), "a cancelled inline job ran");
            }
            // Hold every pool thread behind a latch so the marker has to queue.
            let limit = day_core::blocking_limit();
            let (release, held) = mpsc::channel::<()>();
            let held = Arc::new(Mutex::new(held));
            let started = Arc::new(AtomicUsize::new(0));
            let holders: Vec<_> = (0..limit)
                .map(|_| {
                    let (held, started) = (held.clone(), started.clone());
                    day_core::spawn_blocking(move || {
                        started.fetch_add(1, Ordering::SeqCst);
                        let _ = held.lock().map(|h| h.recv());
                    })
                })
                .collect();
            settle(&t, "the pool never filled", move || {
                started.load(Ordering::SeqCst) >= limit
            })
            .await?;
            let marker = ran.clone();
            drop(day_core::spawn_blocking(move || {
                marker.store(true, Ordering::SeqCst);
            }));
            for _ in 0..limit {
                let _ = release.send(());
            }
            for holder in holders {
                t.check_eq(holder.await, Ok(()))?;
            }
            // The holders are through, so whatever queued behind them has drained too.
            t.check_eq(day_core::spawn_blocking(|| ()).await, Ok(()))?;
            t.check(!ran.load(Ordering::SeqCst), "a cancelled job ran")
        })
    }

    /// Jobs beyond the thread limit queue and every one completes.
    #[day_macros::test(day_core)]
    fn blocking_jobs_beyond_the_limit_all_complete() -> Case {
        Case::headless().run(|t: Drive| async move {
            let n = day_core::blocking_limit().max(1) * 6;
            let jobs: Vec<_> = (0..n)
                .map(|i| {
                    day_core::spawn_blocking(move || {
                        if !day_core::blocking_inline() {
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                        i
                    })
                })
                .collect();
            let mut sum = 0;
            for job in jobs {
                sum += job
                    .await
                    .map_err(|e| day_core::conformance::Fail(e.to_string()))?;
            }
            t.check_eq(sum, (0..n).sum::<usize>())
        })
    }
}

/// Animation cases (docs/tween.md "The animation contract"): what a toolkit's animator does
/// with an `AnimSpec`, read mid-flight through `assert_displayed`, and the frame-driven
/// `day::motion` driver bound to native properties.
///
/// A case cannot know how long a device took to get from one step to the next, so none of
/// them asserts "at 0.5 s the opacity is 0.5". Each keeps a stopwatch on the window's frame
/// clock, started by the frame after the change, reads how long the animation has really been
/// running, and asks the spec ([`AnimSpec::sample`](day_spec::AnimSpec::sample)) what should
/// be showing then, give or take [`JITTER`] of time. The moments each case aims for are chosen
/// so that an animator ignoring the field under test is far outside that window.
mod motion {
    use std::cell::Cell;
    use std::ops::ControlFlow;
    use std::rc::Rc;

    use day_core::conformance::{DisplayedExpect, TestResult};
    use day_core::motion::{Animated, Animation, Easing, Outcome, Spring};
    use day_spec::{AnimSpec, Cap, Color, Point, Support};

    use super::*;

    /// How far, in seconds, the native animator's clock may be from the case's stopwatch: a
    /// frame or two before the animator starts, a frame before the read.
    const JITTER: f64 = 0.1;
    /// How far a read value may be from the spec's, as a share of the distance traveled, on
    /// top of what [`JITTER`] allows.
    const SLACK: f64 = 0.06;

    /// Animations land at once under reduced motion, so there is nothing in flight to read.
    fn motion_allowed() -> Support {
        if day_core::reduce_motion_now() {
            Support::Unsupported
        } else {
            Support::Native
        }
    }

    /// Seconds since a change, counted in the window's frame timestamps.
    #[derive(Clone, Default)]
    struct Stopwatch {
        now: Rc<Cell<f64>>,
        start: Rc<Cell<Option<f64>>>,
        armed: Rc<Cell<bool>>,
    }

    impl Stopwatch {
        /// Follows the current page's frames for as long as the page is mounted.
        fn run(&self) {
            let watch = self.clone();
            day_core::frame::subscribe(move |frame| {
                let now = frame.timestamp.as_secs_f64();
                watch.now.set(now);
                if watch.armed.replace(false) {
                    watch.start.set(Some(now));
                }
                ControlFlow::Continue(())
            })
            .in_scope();
        }

        /// Starts counting at the next frame: the one the change made now is committed with.
        fn mark(&self) {
            self.start.set(None);
            self.armed.set(true);
        }

        fn elapsed(&self) -> f64 {
            self.start.get().map_or(0.0, |start| self.now.get() - start)
        }

        /// Waits until about `secs` after the mark.
        async fn until(&self, d: &Drive, secs: f64) -> TestResult {
            let left = secs - self.elapsed();
            if left > 0.0 {
                d.pause(left).await?;
            }
            Ok(())
        }

        /// What a value animated from `from` to `to` under `spec` shows now, as the middle of
        /// what the spec allows within [`JITTER`] and how far from it a read may be.
        fn expect(&self, spec: AnimSpec, from: f64, to: f64) -> (f64, f64) {
            let at = |elapsed: f64| {
                let sample = spec.sample(elapsed.max(0.0));
                if sample.done {
                    to
                } else {
                    from + (to - from) * sample.progress
                }
            };
            let elapsed = self.elapsed();
            let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
            for i in 0..=16 {
                let v = at(elapsed - JITTER + JITTER * f64::from(i) / 8.0);
                lo = lo.min(v);
                hi = hi.max(v);
            }
            // Never tighter than a hundredth: a value at rest is read back through a float.
            let floor = 0.01 * from.abs().max(to.abs()).max(1.0);
            (
                f64::midpoint(lo, hi),
                ((hi - lo) / 2.0 + (to - from).abs() * SLACK).max(floor),
            )
        }

        /// The opacity `id` shows is what `spec` says for a fade from `from` to `to`.
        async fn opacity(
            &self,
            d: &Drive,
            id: &str,
            spec: AnimSpec,
            from: f64,
            to: f64,
        ) -> TestResult {
            self.opacity_within(d, id, spec, (from, to), 0.0).await
        }

        /// [`Self::opacity`] with `extra` more room, for a fade whose start is itself only
        /// known to within the jitter.
        async fn opacity_within(
            &self,
            d: &Drive,
            id: &str,
            spec: AnimSpec,
            (from, to): (f64, f64),
            extra: f64,
        ) -> TestResult {
            let (opacity, tolerance) = self.expect(spec, from, to);
            let tolerance = tolerance + extra;
            d.assert_displayed(
                id,
                DisplayedExpect {
                    opacity: Some(opacity),
                    tolerance: Some(tolerance),
                    ..Default::default()
                },
            )
            .await
        }
    }

    /// A page with a button that fades a square out under `spec`, and the stopwatch its
    /// drive reads. The square is `anim_box`, the button `anim_go`.
    fn fade(spec: AnimSpec) -> (Case, Stopwatch) {
        let watch = Stopwatch::default();
        let w = watch.clone();
        let case = Case::new()
            .proves_cap(Cap::Animation)
            .proves_duty("set_opacity")
            .proves_duty("read_displayed")
            .requires(Cap::Animation)
            .requires_support("motion that is not reduced", motion_allowed)
            .timeout(60.0)
            .page(move || {
                let faded = Signal::new(false);
                let w = w.clone();
                w.run();
                column((
                    button("Fade")
                        .action(move || {
                            w.mark();
                            with_animation(spec, || faded.set(true));
                        })
                        .id(day_core::id!(anim_go)),
                    rectangle()
                        .fill(Color::rgb(1.0, 0.0, 0.0))
                        .frame(80.0, 40.0)
                        .id(day_core::id!(anim_box))
                        .opacity(move || if faded.get() { 0.0 } else { 1.0 }),
                ))
            });
        (case, watch)
    }

    /// Drives a [`fade`] page: taps the button and checks the opacity at each of `moments`.
    fn fade_at(spec: AnimSpec, moments: &'static [f64]) -> Case {
        let (case, watch) = fade(spec);
        case.drive(move |d: Drive| {
            let watch = watch.clone();
            async move {
                watch.opacity(&d, "anim_box", spec, 1.0, 1.0).await?;
                d.tap("anim_go").await?;
                for &at in moments {
                    watch.until(&d, at).await?;
                    watch.opacity(&d, "anim_box", spec, 1.0, 0.0).await?;
                }
                Ok(())
            }
        })
    }

    /// `delay_ms` holds the start value: half way through a 900 ms delay the square is still
    /// opaque (an animator that ignores the delay is three quarters gone by then), half way
    /// through the 600 ms that follow it is half faded, and then it is gone.
    #[day_macros::test(day_core)]
    fn animation_delay_holds_the_start() -> Case {
        fade_at(AnimSpec::linear(600).delay(900), &[0.45, 1.2, 1.9])
    }

    /// `repeat` plays the cycle again from the start: in the middle of the second and of the
    /// third 800 ms cycle the square is half faded, where an animator that played once has
    /// long left it transparent.
    #[day_macros::test(day_core)]
    fn animation_repeat_plays_every_cycle() -> Case {
        fade_at(
            AnimSpec::linear(800).repeat(2, false),
            &[0.4, 1.2, 2.0, 2.8],
        )
    }

    /// `autoreverse` plays every second cycle backwards: 300 ms into the second cycle the
    /// square is on its way back and mostly transparent still, where an animator that
    /// restarts the cycle shows it mostly opaque; the third cycle runs forward again and the
    /// square ends transparent.
    #[day_macros::test(day_core)]
    fn animation_autoreverse_plays_back() -> Case {
        fade_at(
            AnimSpec::linear(800).repeat(2, true),
            &[0.4, 1.1, 1.4, 2.0, 2.8],
        )
    }

    /// A spec that repeats forever keeps going until the property is changed without an
    /// animation, which cancels it and applies at once.
    #[day_macros::test(day_core)]
    fn animation_repeats_forever_until_set() -> Case {
        let spec = AnimSpec::linear(600).repeat_forever(true);
        let watch = Stopwatch::default();
        let w = watch.clone();
        Case::new()
            .proves_cap(Cap::Animation)
            .requires(Cap::Animation)
            .requires_support("motion that is not reduced", motion_allowed)
            .timeout(60.0)
            .page(move || {
                let level = Signal::new(1.0);
                let w = w.clone();
                w.run();
                column((
                    button("Pulse")
                        .action(move || {
                            w.mark();
                            with_animation(spec, || level.set(0.0));
                        })
                        .id(day_core::id!(anim_go)),
                    button("Hold")
                        .action(move || level.set(0.5))
                        .id(day_core::id!(anim_hold)),
                    rectangle()
                        .fill(Color::rgb(1.0, 0.0, 0.0))
                        .frame(80.0, 40.0)
                        .id(day_core::id!(anim_box))
                        .opacity(level),
                ))
            })
            .drive(move |d: Drive| {
                let watch = watch.clone();
                async move {
                    d.tap("anim_go").await?;
                    // The second cycle runs backwards; the fifth is still running.
                    for at in [0.3, 1.05, 2.7] {
                        watch.until(&d, at).await?;
                        watch.opacity(&d, "anim_box", spec, 1.0, 0.0).await?;
                    }
                    d.tap("anim_hold").await?;
                    let held = DisplayedExpect {
                        opacity: Some(0.5),
                        ..Default::default()
                    };
                    d.assert_displayed("anim_box", held.clone()).await?;
                    d.pause(0.4).await?;
                    d.assert_displayed("anim_box", held).await
                }
            })
    }

    /// A change made while another is in flight starts from the value on screen: half way
    /// through a fade out, a fade back in starts from half opaque rather than from nothing.
    #[day_macros::test(day_core)]
    fn animation_retargets_from_the_value_shown() -> Case {
        let spec = AnimSpec::linear(1000);
        let watch = Stopwatch::default();
        let w = watch.clone();
        // What the square showed when the second change was made.
        let turned_at = Rc::new(Cell::new(1.0));
        let t = turned_at.clone();
        Case::new()
            .proves_cap(Cap::Animation)
            .requires(Cap::Animation)
            .requires_support("motion that is not reduced", motion_allowed)
            .timeout(60.0)
            .page(move || {
                let level = Signal::new(1.0);
                let (w, t) = (w.clone(), t.clone());
                w.run();
                column((
                    button("Out")
                        .action({
                            let w = w.clone();
                            move || {
                                w.mark();
                                with_animation(spec, || level.set(0.0));
                            }
                        })
                        .id(day_core::id!(anim_out)),
                    button("In")
                        .action(move || {
                            t.set(1.0 - spec.sample(w.elapsed()).progress);
                            w.mark();
                            with_animation(spec, || level.set(1.0));
                        })
                        .id(day_core::id!(anim_in)),
                    rectangle()
                        .fill(Color::rgb(1.0, 0.0, 0.0))
                        .frame(80.0, 40.0)
                        .id(day_core::id!(anim_box))
                        .opacity(level),
                ))
            })
            .drive(move |d: Drive| {
                let (watch, turned_at) = (watch.clone(), turned_at.clone());
                async move {
                    d.tap("anim_out").await?;
                    watch.until(&d, 0.5).await?;
                    watch.opacity(&d, "anim_box", spec, 1.0, 0.0).await?;
                    d.tap("anim_in").await?;
                    // A quarter of the way back from where it turned: an animator that
                    // restarted from transparent is a quarter opaque, not past the turn.
                    // Where it turned is known to within the jitter of a one-second fade.
                    let ends = (turned_at.get(), 1.0);
                    watch.until(&d, 0.25).await?;
                    watch
                        .opacity_within(&d, "anim_box", spec, ends, JITTER)
                        .await?;
                    watch.until(&d, 1.3).await?;
                    watch
                        .opacity_within(&d, "anim_box", spec, ends, JITTER)
                        .await
                }
            })
    }

    /// A spring is the physical spring its response and damping describe: a loose one passes
    /// its target by almost half the distance, swings back short of it, and comes to rest on
    /// it. An ease curve stretched over the response never leaves the two ends.
    #[day_macros::test(day_core)]
    fn animation_spring_overshoots_and_settles() -> Case {
        let spec = AnimSpec::spring(0.9, 0.25);
        let watch = Stopwatch::default();
        let w = watch.clone();
        Case::new()
            .proves_cap(Cap::Animation)
            .proves_duty("set_transform")
            .requires(Cap::Animation)
            .requires_support("motion that is not reduced", motion_allowed)
            .timeout(60.0)
            .page(move || {
                let moved = Signal::new(false);
                let w = w.clone();
                w.run();
                column((
                    button("Spring")
                        .action(move || {
                            w.mark();
                            with_animation(spec, || moved.set(true));
                        })
                        .id(day_core::id!(anim_go)),
                    rectangle()
                        .fill(Color::rgb(0.0, 0.0, 1.0))
                        .frame(40.0, 40.0)
                        .id(day_core::id!(anim_box))
                        .translation(move || if moved.get() { 100.0 } else { 0.0 }, 0.0),
                ))
            })
            .drive(move |d: Drive| {
                let watch = watch.clone();
                async move {
                    d.tap("anim_go").await?;
                    // The first peak (144 points), the first trough (80), and at rest.
                    for at in [0.465, 0.93, spec.duration_secs() + 0.3] {
                        watch.until(&d, at).await?;
                        let (tx, tolerance) = watch.expect(spec, 0.0, 100.0);
                        d.assert_displayed(
                            "anim_box",
                            DisplayedExpect {
                                translation: Some(Point::new(tx, 0.0)),
                                tolerance: Some(tolerance),
                                ..Default::default()
                            },
                        )
                        .await?;
                    }
                    Ok(())
                }
            })
    }

    /// An animated frame change passes through the sizes in between.
    #[day_macros::test(day_core)]
    fn animation_frame_passes_between() -> Case {
        let spec = AnimSpec::linear(1200);
        let watch = Stopwatch::default();
        let w = watch.clone();
        Case::new()
            .proves_cap(Cap::Animation)
            .proves_duty("set_frame")
            .requires(Cap::Animation)
            .requires_support("motion that is not reduced", motion_allowed)
            .timeout(60.0)
            .page(move || {
                let wide = Signal::new(false);
                let w = w.clone();
                w.run();
                column((
                    button("Widen")
                        .action(move || {
                            w.mark();
                            with_animation(spec, || wide.set(true));
                        })
                        .id(day_core::id!(anim_go)),
                    rectangle()
                        .fill(Color::rgb(0.0, 0.5, 0.0))
                        .id(day_core::id!(anim_box))
                        .width(move || if wide.get() { 240.0 } else { 80.0 })
                        .height(40.0),
                ))
                .align(HAlign::Leading)
            })
            .drive(move |d: Drive| {
                let watch = watch.clone();
                async move {
                    d.tap("anim_go").await?;
                    for at in [0.6, 1.6] {
                        watch.until(&d, at).await?;
                        let (width, tolerance) = watch.expect(spec, 80.0, 240.0);
                        d.assert_displayed(
                            "anim_box",
                            DisplayedExpect {
                                width: Some(width),
                                tolerance: Some(tolerance),
                                ..Default::default()
                            },
                        )
                        .await?;
                    }
                    d.assert_size("anim_box", 240.0, 40.0).await
                }
            })
    }

    /// The sRGB channel value of a linear-light one.
    fn srgb(linear: f64) -> f64 {
        if linear <= 0.003_130_8 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        }
    }

    /// An animated background color passes through the colors in between. From black to
    /// white every channel rises together; how fast depends on the space the animator blends
    /// in, so the case accepts anything from an OKLab blend (the darkest) to a linear-light
    /// one (the brightest) and rejects a color that is already at either end.
    #[day_macros::test(day_core)]
    fn animation_background_passes_between() -> Case {
        let spec = AnimSpec::linear(2500);
        let watch = Stopwatch::default();
        let w = watch.clone();
        Case::new()
            .proves_cap(Cap::Animation)
            .proves_modifier("background")
            .requires(Cap::Animation)
            .requires_support("motion that is not reduced", motion_allowed)
            .timeout(60.0)
            .page(move || {
                let lit = Signal::new(false);
                let w = w.clone();
                w.run();
                column((
                    button("Light")
                        .action(move || {
                            w.mark();
                            with_animation(spec, || lit.set(true));
                        })
                        .id(day_core::id!(anim_go)),
                    label("")
                        .frame(80.0, 40.0)
                        .id(day_core::id!(anim_box))
                        .background(move || {
                            if lit.get() {
                                Color::WHITE
                            } else {
                                Color::BLACK
                            }
                        }),
                ))
            })
            .drive(move |d: Drive| {
                let watch = watch.clone();
                async move {
                    d.tap("anim_go").await?;
                    watch.until(&d, 1.05).await?;
                    let progress = |shift: f64| {
                        let sample = spec.sample((watch.elapsed() + shift).max(0.0));
                        if sample.done { 1.0 } else { sample.progress }
                    };
                    // OKLab lightness is the cube root of linear light.
                    let darkest = srgb(progress(-JITTER).powi(3));
                    let brightest = srgb(progress(JITTER));
                    let gray = f64::midpoint(darkest, brightest);
                    d.assert_displayed(
                        "anim_box",
                        DisplayedExpect {
                            background: Some(Color::rgb(gray, gray, gray)),
                            tolerance: Some((brightest - darkest) / 2.0 + SLACK),
                            ..Default::default()
                        },
                    )
                    .await?;
                    watch.until(&d, 2.9).await?;
                    d.assert_displayed(
                        "anim_box",
                        DisplayedExpect {
                            background: Some(Color::WHITE),
                            ..Default::default()
                        },
                    )
                    .await
                }
            })
    }

    /// A composed `day::motion` animation runs on the window's frames and moves the native
    /// properties bound to its values: a fade and a lift in sequence end where they say, and
    /// the handle reports that it finished.
    #[day_macros::test(day_core)]
    fn motion_sequence_moves_bound_properties() -> Case {
        Case::new()
            .proves_duty("request_frame")
            .timeout(60.0)
            .page(|| {
                let fade = Animated::new(1.0);
                let lift = Animated::new(0.0);
                let status = Signal::new("idle");
                column((
                    button("Play")
                        .action(move || {
                            status.set("playing");
                            Animation::sequence([
                                Animation::parallel([
                                    Animation::timing(fade, 0.25, 200, Easing::EASE_IN_OUT),
                                    Animation::timing(lift, -30.0, 200, Easing::EASE),
                                ]),
                                Animation::delay(50),
                                Animation::spring(lift, 12.0, Spring::new(320.0, 30.0)),
                            ])
                            .start()
                            .on_end(move |outcome| {
                                status.set(if outcome == Outcome::Finished {
                                    "finished"
                                } else {
                                    "stopped"
                                });
                            });
                        })
                        .id(day_core::id!(motion_play)),
                    label(move || status.get().to_owned()).id(day_core::id!(motion_status)),
                    label(move || format!("{:.2} {:.1}", fade.get(), lift.get()))
                        .id(day_core::id!(motion_values)),
                    rectangle()
                        .fill(Color::rgb(0.0, 0.0, 1.0))
                        .frame(40.0, 40.0)
                        .id(day_core::id!(motion_box))
                        .opacity(fade)
                        .translation(0.0, lift),
                ))
            })
            .drive(|d: Drive| async move {
                d.assert_text("motion_values", "1.00 0.0").await?;
                d.tap("motion_play").await?;
                d.assert_text("motion_status", "finished").await?;
                d.assert_text("motion_values", "0.25 12.0").await?;
                d.assert_displayed(
                    "motion_box",
                    DisplayedExpect {
                        opacity: Some(0.25),
                        translation: Some(Point::new(0.0, 12.0)),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// A looped animation runs until it is stopped, and a reset puts its values back where
    /// they were when it started.
    #[day_macros::test(day_core)]
    fn motion_loop_stops_and_resets() -> Case {
        Case::new()
            .timeout(60.0)
            .page(|| {
                let pulse = Animated::new(1.0);
                let status = Signal::new("idle");
                let playing = Rc::new(Cell::new(None));
                let (stop, reset) = (playing.clone(), playing.clone());
                column((
                    button("Loop")
                        .action(move || {
                            let run = Animation::sequence([
                                Animation::timing(pulse, 0.3, 150, Easing::Linear),
                                Animation::timing(pulse, 0.8, 150, Easing::Linear),
                            ])
                            .looped()
                            .start();
                            run.on_end(move |outcome| {
                                status.set(if outcome == Outcome::Stopped {
                                    "stopped"
                                } else {
                                    "finished"
                                });
                            });
                            status.set("looping");
                            playing.set(Some(run));
                        })
                        .id(day_core::id!(motion_loop)),
                    button("Stop")
                        .action(move || {
                            let run = stop.take();
                            if let Some(run) = &run {
                                run.stop();
                            }
                            stop.set(run);
                        })
                        .id(day_core::id!(motion_stop)),
                    button("Reset")
                        .action(move || {
                            if let Some(run) = reset.take() {
                                run.reset();
                            }
                        })
                        .id(day_core::id!(motion_reset)),
                    label(move || status.get().to_owned()).id(day_core::id!(motion_status)),
                    label(move || {
                        if pulse.is_animating() {
                            "moving".to_owned()
                        } else {
                            format!("{:.2}", pulse.get())
                        }
                    })
                    .id(day_core::id!(motion_value)),
                    rectangle()
                        .fill(Color::rgb(1.0, 0.0, 0.0))
                        .frame(40.0, 40.0)
                        .id(day_core::id!(motion_box))
                        .opacity(pulse),
                ))
            })
            .drive(|d: Drive| async move {
                d.assert_text("motion_value", "1.00").await?;
                d.tap("motion_loop").await?;
                d.assert_text("motion_value", "moving").await?;
                // Several passes later it is still going.
                d.pause(1.0).await?;
                d.assert_text("motion_status", "looping").await?;
                d.tap("motion_stop").await?;
                d.assert_text("motion_status", "stopped").await?;
                d.tap("motion_reset").await?;
                d.assert_text("motion_value", "1.00").await?;
                d.assert_displayed(
                    "motion_box",
                    DisplayedExpect {
                        opacity: Some(1.0),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// An interpolation maps one animated value onto an opacity and a color, clamped at the
    /// ends of its range.
    #[day_macros::test(day_core)]
    fn motion_interpolation_follows_its_value() -> Case {
        Case::new()
            .requires(Cap::Snapshot)
            .page(|| {
                let scroll = Animated::new(0.0);
                column((
                    button("Half")
                        .action(move || scroll.set(60.0))
                        .id(day_core::id!(motion_half)),
                    button("Past")
                        .action(move || scroll.set(500.0))
                        .id(day_core::id!(motion_past)),
                    label(move || {
                        let fade = scroll.interpolate([0.0, 120.0], [1.0, 0.2]);
                        format!("{:.2}", fade())
                    })
                    .id(day_core::id!(motion_value)),
                    label("")
                        .frame(80.0, 40.0)
                        .background(scroll.interpolate(
                            [0.0, 120.0],
                            [Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0)],
                        ))
                        .id(day_core::id!(motion_swatch)),
                ))
            })
            .drive(|d: Drive| async move {
                d.assert_text("motion_value", "1.00").await?;
                d.sample_pixel("motion_swatch", 0.5, 0.5, "#ff0000").await?;
                d.tap("motion_half").await?;
                d.assert_text("motion_value", "0.60").await?;
                d.tap("motion_past").await?;
                d.assert_text("motion_value", "0.20").await?;
                d.sample_pixel("motion_swatch", 0.5, 0.5, "#0000ff").await
            })
    }
}

/// The conformance app's content: a nav over every registered GUI case, one route per case,
/// so a person can open any case on any toolkit and look at it. A run does not go through it:
/// the app roots it in [`test_host`](crate::test_host), which shows each driven case's page
/// alone while it runs. An app with its own tests can browse them the same way.
///
/// Split, never tabs: a split nav builds a case's page when it is first opened, where tabs
/// (what a phone picks for a flat nav) keep every page resident and build them all at launch.
/// Since cases hold navigation, covers and pieces of their own, that kept a CI phone's launch
/// transitioning past the run's first `wait_idle`.
#[must_use]
pub fn test_pages() -> impl Piece {
    let selection = Signal::new(String::new());
    nav(selection)
        .presentation(day_spec::props::NavPresentation::Split)
        .items(nav_items(
            || {
                day_core::conformance::cases()
                    .into_iter()
                    .filter(|c| c.kind() == day_core::conformance::TestKind::Gui)
                    .map(|c| c.name().to_owned())
                    .collect::<Vec<String>>()
            },
            |name: &String| item(name.clone(), name.clone()),
        ))
        .destination(
            |key: &String| match day_core::conformance::case_named(key) {
                Some(case) => guarded_page(case),
                None => label(format!("no test {key:?}")).any(),
            },
        )
}
