//! HarmonyOS: `OH_AT_CheckSelfPermission`, and an ArkTS seam for the request.
//!
//! **This arm is not implemented yet.** Every query reports [`Gate::Absent`] /
//! [`Status::Unsupported`] and every request resolves the same way, so an app linking this crate on
//! HarmonyOS today gets an honest "no" rather than a wrong "yes". The check is a direct FFI call into `libability_access_control.so`. The REQUEST has no
//! native C API — `requestPermissionsFromUser` needs a `UIAbilityContext`, reachable only from
//! ArkTS — so it will go through a `day-arkui-sys` seam mirroring `registerFilePicker`, found by
//! `dlsym` so this crate keeps no dependency on that toolkit.

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
