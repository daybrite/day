// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Opt-in presentation policy for functional walkthroughs: fast mode is reduced motion forced
//! on (docs/accessibility.md), nothing more. This is not a virtual clock: timers, network
//! requests, physics and media retain their real timing.

static FORCED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Whether the launch forced reduced motion: `DAY_TEST_FAST=1` (`day launch --fast`) or
/// `DAY_REDUCE_MOTION=1`, the override that lets a script or a screenshot run the reduced
/// variant without the system setting. Neither is read after launch, and the user's own
/// setting is OR-ed in by [`crate::reduce_motion_now`].
pub(crate) fn forced_reduce_motion() -> bool {
    *FORCED.get_or_init(|| {
        let on = |key: &str| std::env::var(key).as_deref() == Ok("1");
        on("DAY_TEST_FAST") || on("DAY_REDUCE_MOTION")
    })
}

/// Skip decorative motion, applying its destination state immediately: the reduced-motion
/// gate ([`crate::reduce_motion_now`]), which fast mode forces on. Kept under this name for
/// the backends that ask it before animating a native transition.
pub fn fast_animations() -> bool {
    crate::reduce_motion_now()
}

/// Browser hosts have no process environment. Initialize once before constructing the UI.
#[cfg(target_arch = "wasm32")]
pub fn init_fast_animations(enabled: bool) {
    let _ = FORCED.set(enabled);
}

#[cfg(test)]
mod tests {
    use crate::frame::{FrameClock, install_frame_requester};
    use crate::tween::{Timing, Tweened, animate};
    use day_spec::{AnimSpec, FrameStamp};
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};

    #[test]
    fn fast_motion_preserves_endpoints_and_continuous_clocks() {
        // Exercise both startup policies in isolated processes, without racing other
        // tests over a process-global environment variable or the cached policy.
        if std::env::var_os("DAY_MOTION_TEST_CHILD").is_none() {
            for fast in ["0", "1"] {
                assert!(
                    std::process::Command::new(std::env::current_exe().unwrap())
                        .args([
                            "--exact",
                            "testing::tests::fast_motion_preserves_endpoints_and_continuous_clocks"
                        ])
                        .env("DAY_MOTION_TEST_CHILD", "1")
                        .env("DAY_TEST_FAST", fast)
                        .status()
                        .unwrap()
                        .success()
                );
            }
            return;
        }
        let fast = super::fast_animations();
        let pending = Rc::new(RefCell::new(VecDeque::new()));
        let q = pending.clone();
        install_frame_requester(move |_, cb| {
            q.borrow_mut().push_back(cb);
            Box::new(|| {})
        });
        let clock = FrameClock::for_root(crate::RNode::default());
        let tween = Tweened::on(clock, 0.0);
        tween.animate_to(42.0, AnimSpec::linear(1000));
        assert_eq!(tween.get_untracked(), if fast { 42.0 } else { 0.0 });
        let samples = Rc::new(RefCell::new(Vec::new()));
        let s = samples.clone();
        let finite = animate(
            clock,
            Timing::new(AnimSpec {
                repeat: 1,
                autoreverse: true,
                delay_ms: 500,
                ..AnimSpec::linear(1000)
            }),
            move |sample| s.borrow_mut().push(sample),
        );
        let continuous = clock.subscribe(|_| std::ops::ControlFlow::Continue(()));
        let forever = animate(
            clock,
            Timing::new(AnimSpec {
                repeat: u32::MAX,
                ..AnimSpec::linear(1000)
            }),
            |sample| assert!(!sample.done),
        );
        let instant = Rc::new(RefCell::new(None));
        let i = instant.clone();
        let _instant = animate(
            clock,
            Timing::new(AnimSpec {
                repeat: 1,
                autoreverse: true,
                ..AnimSpec::linear(0)
            }),
            move |sample| *i.borrow_mut() = Some(sample),
        );
        let cb = pending.borrow_mut().pop_front().unwrap();
        cb(FrameStamp::new(1.0));
        assert_eq!(samples.borrow()[0].done, fast);
        assert_eq!(samples.borrow()[0].progress, 0.0); // autoreverse ends at its start
        assert_eq!(finite.is_active(), !fast);
        assert!(continuous.is_active() && forever.is_active());
        let instant = instant.borrow().unwrap();
        assert!(instant.done);
        assert_eq!(instant.progress, 1.0); // zero-duration timing always applies its target
    }
}
