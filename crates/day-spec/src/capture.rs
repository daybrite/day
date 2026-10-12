// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Screenshot checkpoints. A revision identifies one ordered request, not a timer or
//! an image hash. Completion of an older request cannot release a newer checkpoint.

/// How a toolkit establishes freshness for a screenshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// The main loop must run before the checkpoint can complete.
    Pending,
    /// The toolkit completed its render checkpoint. The external capture still owns
    /// synchronization with the compositor (and must not return a cached image).
    Ready,
    /// The toolkit's synchronous snapshot operation itself establishes freshness.
    /// This does not promise that an external whole-screen capture is ready.
    OnCapture,
}

/// UI-thread-owned state for an asynchronous capture checkpoint. Clones share state.
/// Keep one per native window. Callbacks may outlive a timed-out request: `complete`
/// deliberately ignores them after another revision has started.
#[derive(Clone, Default)]
pub struct Fence(std::rc::Rc<std::cell::RefCell<State>>);
#[derive(Default)]
struct State {
    revision: Option<u32>,
    result: Option<Result<(), String>>,
}
impl Fence {
    /// Start once; repeated polls of the same revision must not request another frame.
    pub fn begin(&self, revision: u32) -> bool {
        let mut s = self.0.borrow_mut();
        if s.revision == Some(revision) {
            return false;
        }
        *s = State {
            revision: Some(revision),
            result: None,
        };
        true
    }
    pub fn complete(&self, revision: u32, result: Result<(), String>) {
        let mut s = self.0.borrow_mut();
        if s.revision == Some(revision) && s.result.is_none() {
            s.result = Some(result);
        }
    }
    pub fn poll(&self) -> Result<Readiness, String> {
        match self.0.borrow().result.as_ref() {
            None => Ok(Readiness::Pending),
            Some(Ok(())) => Ok(Readiness::Ready),
            Some(Err(e)) => Err(e.clone()),
        }
    }
}

/// Whether a host that presents frames of its own has every frame it owes on screen.
///
/// A renderer that paints outside the toolkit's frame — an offscreen engine presented through
/// a canvas, its frames arriving after an asynchronous read-back and decode — is invisible to
/// the toolkit's checkpoint: the checkpoint sees a committed frame, not that the host's latest
/// frame was in it. A capture taken then shows the canvas as it was, or empty. Such a host
/// registers one hold ([`register_hold`]); the dayscript capture steps wait, bounded by the
/// step's timeout, while any hold answers `false`, before they request the checkpoint at all.
/// Called on the UI thread, so the hold may read UI-thread state.
pub type Hold = fn() -> bool;

static HOLDS: std::sync::Mutex<Vec<Hold>> = std::sync::Mutex::new(Vec::new());

/// Register a capture hold for the life of the process. Register once per host: every
/// registration is consulted on every capture.
pub fn register_hold(hold: Hold) {
    HOLDS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(hold);
}

/// Whether every registered hold has settled. Holds run outside the registry's lock, so a
/// hold may register another.
pub fn holds_settled() -> bool {
    let holds = HOLDS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    holds.iter().all(|hold| hold())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checkpoints_do_not_rearm_or_accept_late_completions() {
        let f = Fence::default();
        assert!(f.begin(1));
        assert!(!f.begin(1));
        assert_eq!(f.poll(), Ok(Readiness::Pending));
        assert!(f.begin(2));
        f.clone().complete(1, Ok(()));
        assert_eq!(f.poll(), Ok(Readiness::Pending));
        f.complete(2, Ok(()));
        assert_eq!(f.poll(), Ok(Readiness::Ready));
        assert!(!f.begin(2));
    }
    #[test]
    fn a_hold_defers_captures_until_it_settles() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static SETTLED: AtomicBool = AtomicBool::new(false);
        // Other tests in this process may have registered nothing; before any hold, settled.
        assert!(holds_settled());
        register_hold(|| SETTLED.load(Ordering::Relaxed));
        assert!(!holds_settled());
        SETTLED.store(true, Ordering::Relaxed);
        assert!(holds_settled());
    }
    #[test]
    fn errors_and_windows_are_independent() {
        let a = Fence::default();
        let b = Fence::default();
        a.begin(7);
        b.begin(7);
        a.complete(7, Err("window closed".into()));
        a.complete(7, Ok(()));
        assert_eq!(a.poll(), Err("window closed".into()));
        assert_eq!(b.poll(), Ok(Readiness::Pending));
        a.begin(8);
        a.complete(8, Ok(()));
        assert_eq!(a.poll(), Ok(Readiness::Ready));
    }
}
