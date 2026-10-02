// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Opt-in presentation policy for functional walkthroughs. This is not a virtual clock:
//! timers, network requests, physics and media retain their real timing.

static FAST_ANIMATIONS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Skip decorative motion, applying its destination state immediately.
/// Set `DAY_TEST_FAST=1` before launch (`day launch --fast`). Ordinary runs are unchanged.
pub fn fast_animations() -> bool {
    *FAST_ANIMATIONS.get_or_init(|| std::env::var("DAY_TEST_FAST").as_deref() == Ok("1"))
}

/// Browser hosts have no process environment. Initialize once before constructing the UI.
#[cfg(target_arch = "wasm32")]
pub fn init_fast_animations(enabled: bool) {
    let _ = FAST_ANIMATIONS.set(enabled);
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
