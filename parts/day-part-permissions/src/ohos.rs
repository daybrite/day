// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! HarmonyOS: `OH_AT_CheckSelfPermission` for the answer, and the host page's ArkTS prompter for
//! the question.
//!
//! The check is a direct FFI call into `libability_access_control.so`. The REQUEST has no native
//! C API — `requestPermissionsFromUser` needs a `UIAbilityContext`, reachable only from ArkTS —
//! so it goes through `day_arkui_request_permissions`, a seam day-arkui exports to the shim's
//! ArkTS-registered prompter (docs/permissions.md). It is found by `dlsym` at call time, so this
//! crate keeps no link-time dependency on that toolkit: a HarmonyOS app always carries it, and a
//! build that somehow does not answers the request with the current status instead of failing
//! to link.
//!
//! HarmonyOS has no "asked and refused once" state a native call can read: a denied permission is
//! simply not held, and the system remembers a refusal itself (a second `requestPermissionsFromUser`
//! after a denial resolves without a dialog). So `status` answers `Prompt` for anything not held,
//! `can_prompt` is true, and the request's own answer is where a refusal becomes `Denied`.

use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::sync::Mutex;

use crate::{Gate, Permission, Status, merge};

#[link(name = "ability_access_control")]
unsafe extern "C" {
    fn OH_AT_CheckSelfPermission(permission: *const c_char) -> bool;
}

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// The seam's C signature (see `day_arkui::day_arkui_request_permissions`).
type RequestFn = unsafe extern "C" fn(u64, *const c_char, extern "C" fn(u64, u64)) -> c_int;

/// The permission names one portable permission stands for, in the order they are requested
/// (which is the order the grant mask answers in). Notifications has no runtime permission on
/// HarmonyOS — enabling them is a `notificationManager` call — so it has none.
fn native_ids(perm: Permission) -> Vec<&'static str> {
    match perm {
        Permission::Location => vec![
            "ohos.permission.APPROXIMATELY_LOCATION",
            "ohos.permission.LOCATION",
        ],
        Permission::LocationAlways => vec!["ohos.permission.LOCATION_IN_BACKGROUND"],
        Permission::Camera => vec!["ohos.permission.CAMERA"],
        Permission::Microphone => vec!["ohos.permission.MICROPHONE"],
        Permission::Notifications => Vec::new(),
        Permission::Photos => vec!["ohos.permission.READ_IMAGEVIDEO"],
        Permission::Motion => vec!["ohos.permission.ACTIVITY_MOTION"],
        Permission::Raw(name) => vec![name],
    }
}

fn held(name: &str) -> bool {
    let Ok(c) = CString::new(name) else {
        return false;
    };
    unsafe { OH_AT_CheckSelfPermission(c.as_ptr()) }
}

pub fn gate(perm: Permission) -> Gate {
    if native_ids(perm).is_empty() {
        Gate::Absent
    } else {
        Gate::Prompts
    }
}

pub fn status(perm: Permission) -> Status {
    let ids = native_ids(perm);
    if ids.is_empty() {
        return Status::Unsupported;
    }
    ids.iter()
        .map(|id| {
            if held(id) {
                Status::Granted
            } else {
                Status::Prompt
            }
        })
        .fold(Status::Unknown, merge)
}

pub fn status_async(perm: Permission, on_done: Box<dyn FnOnce(Status) + Send>) {
    on_done(status(perm));
}

pub fn can_prompt(perm: Permission) -> bool {
    status(perm) == Status::Prompt
}

pub fn should_show_rationale(_perm: Permission) -> bool {
    false
}

/// One outstanding prompt: what was asked, and who is waiting.
type Pending = HashMap<u64, (Vec<&'static str>, Box<dyn FnOnce(Status) + Send>)>;

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);
static NEXT_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn pending_lock() -> std::sync::MutexGuard<'static, Option<Pending>> {
    match PENDING.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// The seam, looked up once. `None` when this process carries no day-arkui.
fn request_fn() -> Option<RequestFn> {
    let name = c"day_arkui_request_permissions";
    let sym = unsafe { dlsym(std::ptr::null_mut(), name.as_ptr()) };
    if sym.is_null() {
        return None;
    }
    // SAFETY: the symbol is day-arkui's `#[no_mangle] extern "C"` function of exactly this
    // signature; nothing else exports that name.
    Some(unsafe { std::mem::transmute::<*mut c_void, RequestFn>(sym) })
}

/// The prompter's answer: bit `i` of `mask` is set when the `i`th requested name was granted.
extern "C" fn on_result(token: u64, mask: u64) {
    let entry = pending_lock().as_mut().and_then(|m| m.remove(&token));
    let Some((ids, on_done)) = entry else {
        return; // already answered
    };
    let status = ids
        .iter()
        .enumerate()
        .map(|(i, _)| {
            if mask & (1 << i) != 0 {
                Status::Granted
            } else {
                Status::Denied
            }
        })
        .fold(Status::Unknown, merge);
    on_done(status);
}

pub fn request(perm: Permission, on_done: Box<dyn FnOnce(Status) + Send>) {
    let ids = native_ids(perm);
    if ids.is_empty() {
        on_done(status(perm));
        return;
    }
    let Some(ask) = request_fn() else {
        on_done(status(perm));
        return;
    };
    let Ok(joined) = CString::new(ids.join("\u{1f}")) else {
        on_done(status(perm));
        return;
    };
    let token = NEXT_TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    pending_lock()
        .get_or_insert_with(HashMap::new)
        .insert(token, (ids, on_done));
    let sent = unsafe { ask(token, joined.as_ptr(), on_result) };
    if sent == 0 {
        // No prompter is registered, so the answer will never come — resolve with what is held
        // rather than leave the caller waiting.
        let entry = pending_lock().as_mut().and_then(|m| m.remove(&token));
        if let Some((_, cb)) = entry {
            cb(status(perm));
        }
    }
}

pub fn open_settings(_perm: Permission) -> bool {
    false
}
