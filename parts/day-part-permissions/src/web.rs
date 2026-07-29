//! The browser: `navigator.permissions` plus the per-API request calls.
//!
//! **This arm is not implemented yet.** Every query reports [`Gate::Absent`] /
//! [`Status::Unsupported`] and every request resolves the same way, so an app linking this crate on
//! the web today gets an honest "no" rather than a wrong "yes". The next step is a `day_dom_perm_*` block in `crates/day-cli/resources/web/shim.js` (the
//! `day-part-prefs` / `day-part-http` pattern) with a live `change`-event cache, and the matching
//! `extern "C"` imports here. Note that `DeviceMotionEvent.requestPermission()` needs a live user
//! activation, which day-dom preserves because it dispatches `click` synchronously into wasm.

use crate::{Gate, Permission, Status};

pub fn gate(_perm: Permission) -> Gate {
    Gate::Absent
}

pub fn status(_perm: Permission) -> Status {
    Status::Unsupported
}

pub fn status_async(_perm: Permission, on_done: Box<dyn FnOnce(Status) + Send>) {
    on_done(Status::Unsupported);
}

pub fn can_prompt(_perm: Permission) -> bool {
    false
}

pub fn should_show_rationale(_perm: Permission) -> bool {
    false
}

pub fn request(_perm: Permission, on_done: Box<dyn FnOnce(Status) + Send>) {
    on_done(Status::Unsupported);
}

pub fn open_settings(_perm: Permission) -> bool {
    false
}
