//! Ambient animation intent (§8.4). [`with_animation`] sets a thread-local `AnimSpec` for the
//! duration of a state mutation; the tree's `patch` / layout's `set_frame` read it (via
//! `Tree::resolve_anim`) and hand it to the backend as *backend-executed* animation intent — Day
//! passes intent, the toolkit animates. A node-scoped `.animation(anim)` stores its own `AnimSpec`
//! on the node (`NodeData::implicit_anim`); the ambient one wins when both are present.

use std::cell::Cell;

use day_spec::AnimSpec;

thread_local! {
    static CURRENT_ANIM: Cell<Option<AnimSpec>> = const { Cell::new(None) };
}

/// The ambient animation set by the innermost enclosing [`with_animation`], if any.
pub fn current_anim() -> Option<AnimSpec> {
    CURRENT_ANIM.with(|c| c.get())
}

/// Restores the previous ambient animation on drop (panic-safe).
struct Restore(Option<AnimSpec>);
impl Drop for Restore {
    fn drop(&mut self) {
        CURRENT_ANIM.with(|c| c.set(self.0));
    }
}

/// Run `f` with `spec` as the ambient animation, restoring the previous value afterwards.
pub(crate) fn with_current_anim<R>(spec: AnimSpec, f: impl FnOnce() -> R) -> R {
    let prev = CURRENT_ANIM.with(|c| c.replace(Some(spec)));
    let _restore = Restore(prev);
    f()
}

/// Explicitly animate every state change made in `f` — Day's equivalent of SwiftUI's
/// `withAnimation`. The mutation runs inside a [`day_reactive::batch`]; that batch's synchronous
/// fixpoint drain (bindings → `patch`, plus the turn-end layout → `set_frame`) executes while
/// `spec` is ambient, so the resulting native updates carry the animation intent and the toolkit
/// animates them on its own compositor. Nesting overrides; the previous ambient restores after.
///
/// Edge case: if called from *inside* an in-progress drain (rare — mutating within a reactive
/// effect), the batch defers to the ongoing drain and the intent is not captured; the change then
/// applies instantly. This matches SwiftUI's transaction boundaries.
pub fn with_animation<R>(spec: AnimSpec, f: impl FnOnce() -> R) -> R {
    with_current_anim(spec, || {
        // Coalesce the writes, then force the drain to run NOW — while `spec` is still ambient —
        // rather than deferring to the enclosing batch's close (event dispatch runs handlers inside
        // a batch, so that close happens after this scope ends and the intent would be lost).
        let r = day_reactive::batch(f);
        day_reactive::flush_now();
        r
    })
}
