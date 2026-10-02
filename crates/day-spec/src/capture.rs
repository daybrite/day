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
