// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The built-in pieces' conformance cases, gathered (docs/testing.md). Each piece's cases live
//! in a `mod conformance` at the end of the file that defines it, next to the constructors they
//! prove; this module lists those modules for the targets with no link-time registry (wasm),
//! holds the cases that belong to no piece, and builds the conformance app's browsing page.
//! Compiled only under the `conformance` feature, so a shipping app links none of it.

use day_core::conformance::{Case, Drive, TestFn};
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
            #[cfg(not(target_arch = "wasm32"))]
            let started = std::time::Instant::now();
            day_core::sleep(120).await;
            #[cfg(not(target_arch = "wasm32"))]
            if !synchronous_backend() {
                // Every backend's timer fires at or after its delay; the one millisecond of
                // slack covers the timer's clock and `Instant` ticking apart.
                let slept = started.elapsed();
                t.check(
                    slept >= std::time::Duration::from_millis(119),
                    &format!("a 120 ms sleep resolved after {slept:?}"),
                )?;
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
                order.with(|o| o.len()) == 4 && done.get() == 64
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
                    marker.store(true, Ordering::SeqCst)
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
                marker.store(true, Ordering::SeqCst)
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

    day_core::tests! {
        headless_sanity,
        sleep_resolves_after_its_delay,
        sleep_zero_yields_the_turn,
        sleeps_resolve_in_delay_order,
        an_aborted_task_never_wakes_from_its_sleep,
        a_blocking_job_runs_off_the_ui_thread_and_resolves_on_it,
        a_panicking_blocking_job_is_an_error,
        a_blocking_job_dropped_before_it_starts_never_runs,
        blocking_jobs_beyond_the_limit_all_complete,
    }
}

/// Register every module's cases where there is no link-time registry (wasm); a no-op
/// elsewhere. A new piece module's `conformance` joins both lists below.
pub fn register_tests() {
    general::register_tests();
    crate::leaves::conformance::register_tests();
    crate::inputs::conformance::register_tests();
    crate::image::conformance::register_tests();
    crate::forms::conformance::register_tests();
    crate::canvas::conformance::register_tests();
    crate::containers::conformance::register_tests();
    crate::shapes::conformance::register_tests();
    crate::decorators::conformance::register_tests();
    crate::structure::conformance::register_tests();
    crate::nav::conformance::register_tests();
    crate::inspector::conformance::register_tests();
    crate::split::conformance::register_tests();
    crate::dialogs::conformance::register_tests();
    crate::toolbar::conformance::register_tests();
    crate::menus::conformance::register_tests();
    crate::tabs::conformance::register_tests();
}

/// Every module's roster as written, for the lint that holds it equal to the link-time slice.
pub fn roster() -> Vec<TestFn> {
    [
        general::roster(),
        crate::leaves::conformance::roster(),
        crate::inputs::conformance::roster(),
        crate::image::conformance::roster(),
        crate::forms::conformance::roster(),
        crate::canvas::conformance::roster(),
        crate::containers::conformance::roster(),
        crate::shapes::conformance::roster(),
        crate::decorators::conformance::roster(),
        crate::structure::conformance::roster(),
        crate::nav::conformance::roster(),
        crate::inspector::conformance::roster(),
        crate::split::conformance::roster(),
        crate::dialogs::conformance::roster(),
        crate::toolbar::conformance::roster(),
        crate::menus::conformance::roster(),
        crate::tabs::conformance::roster(),
    ]
    .concat()
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
pub fn test_pages() -> impl Piece {
    let selection = Signal::new(String::new());
    nav(selection)
        .presentation(day_spec::props::NavPresentation::Split)
        .items(
            || {
                day_core::conformance::cases()
                    .into_iter()
                    .filter(|c| c.kind() == day_core::conformance::TestKind::Gui)
                    .map(|c| c.name().to_owned())
                    .collect::<Vec<String>>()
            },
            |name: &String| item(name.clone(), name.clone()),
        )
        .destination(
            |key: &String| match day_core::conformance::case_named(key) {
                Some(case) => guarded_page(case),
                None => label(format!("no test {key:?}")).any(),
            },
        )
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    /// The roster (what wasm registers) names exactly the functions the link-time slice holds,
    /// under the same names.
    #[test]
    fn roster_matches_the_link_time_registry() {
        let entries = |tests: Vec<day_core::conformance::TestFn>| {
            let mut v: Vec<(String, usize)> = tests
                .iter()
                .map(|t| (t.name(), t.build_fn() as usize))
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            entries(day_core::conformance::TESTS.to_vec()),
            entries(super::roster()),
            "every #[day::test] here must also be in `day_core::tests!`"
        );
    }
}
