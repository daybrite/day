// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Android: `Context.checkSelfPermission`, and `requestPermissions` through this crate's OWN Java
//! shim (`android/java/…/DayPermissions.java`) — staged into the app's Gradle build by `day build`
//! through `[package.metadata.day.android]`, exactly like the UI pieces but registering no renderer.
//!
//! One portable permission can map to SEVERAL native ids (location is fine + coarse; photos on API
//! 33+ is images + video), so every query folds the per-id answers with [`crate::merge`] and every
//! request submits the whole set in one array — which is also what makes Android show its
//! precise/approximate location dialog.
//!
//! Permission lists cross JNI as one `\u{1f}`-joined string and answers come back as a bitmask, so
//! neither side needs `jobjectArray` plumbing. The Rust side already knows which ids it asked for,
//! keyed by token.

use std::collections::HashMap;
use std::sync::Mutex;

use day_android::DayEnv;
use day_android::jni::objects::JValue;
use day_android::with_env;

use crate::{Gate, Permission, Status, classify_android, merge};

const CLASS: &str = "dev/daybrite/day/permissions/DayPermissions";
/// The separator DayPermissions.java splits on (day_spec's C-ABI convention).
const SEP: char = '\u{1f}';

type Pending = HashMap<
    i64,
    (
        Permission,
        Vec<&'static str>,
        Box<dyn FnOnce(Status) + Send>,
    ),
>;

/// `Build.VERSION.SDK_INT`, read once. 0 when the shim is unreachable, which makes every
/// version-gated branch below take its most conservative arm.
fn sdk_int() -> i32 {
    static CACHE: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| {
        with_env(|env| {
            env.dcall_static(CLASS, "sdkInt", "()I", &[])
                .ok()
                .and_then(|v| v.i().ok())
                .unwrap_or(0)
        })
    })
}

/// The native permission ids a portable permission maps to on THIS device's API level.
///
/// An empty list means "nothing to ask for here" — the capability is either ungated (motion below
/// API 29) or handled by a non-permission API (notifications below API 33).
fn native_ids(perm: Permission) -> Vec<&'static str> {
    let sdk = sdk_int();
    match perm {
        // Requesting both is what produces the precise/approximate choice; granting either is
        // location access, which `merge`'s Granted-wins precedence already expresses.
        Permission::Location => vec![
            "android.permission.ACCESS_FINE_LOCATION",
            "android.permission.ACCESS_COARSE_LOCATION",
        ],
        // API 29 introduced the separate background permission; below that, foreground location
        // already covers background use, so the foreground answer IS the answer.
        Permission::LocationAlways => {
            if sdk >= 29 {
                vec!["android.permission.ACCESS_BACKGROUND_LOCATION"]
            } else {
                vec![
                    "android.permission.ACCESS_FINE_LOCATION",
                    "android.permission.ACCESS_COARSE_LOCATION",
                ]
            }
        }
        Permission::Camera => vec!["android.permission.CAMERA"],
        Permission::Microphone => vec!["android.permission.RECORD_AUDIO"],
        Permission::Notifications => {
            if sdk >= 33 {
                vec!["android.permission.POST_NOTIFICATIONS"]
            } else {
                Vec::new() // areNotificationsEnabled() is the whole story below 33
            }
        }
        Permission::Photos => {
            if sdk >= 33 {
                vec![
                    "android.permission.READ_MEDIA_IMAGES",
                    "android.permission.READ_MEDIA_VIDEO",
                ]
            } else {
                vec!["android.permission.READ_EXTERNAL_STORAGE"]
            }
        }
        Permission::Motion => {
            if sdk >= 29 {
                vec!["android.permission.ACTIVITY_RECOGNITION"]
            } else {
                Vec::new() // install-time before API 29
            }
        }
        Permission::Raw(name) => vec![name],
    }
}

fn call_bool(method: &str, perm: &str) -> bool {
    with_env(|env| {
        let Ok(s) = env.new_string(perm) else {
            return false;
        };
        env.dcall_static(
            CLASS,
            method,
            "(Ljava/lang/String;)Z",
            &[JValue::Object(&s)],
        )
        .ok()
        .and_then(|v| v.z().ok())
        .unwrap_or(false)
    })
}

fn is_granted(perm: &str) -> bool {
    with_env(|env| {
        let Ok(s) = env.new_string(perm) else {
            return false;
        };
        env.dcall_static(
            CLASS,
            "check",
            "(Ljava/lang/String;)I",
            &[JValue::Object(&s)],
        )
        .ok()
        .and_then(|v| v.i().ok())
        .unwrap_or(0)
            == 1
    })
}

fn notifications_enabled() -> bool {
    with_env(|env| {
        env.dcall_static(CLASS, "notificationsEnabled", "()Z", &[])
            .ok()
            .and_then(|v| v.z().ok())
            .unwrap_or(false)
    })
}

/// The status of ONE native id, from the three probes [`classify_android`] folds.
fn id_status(id: &str) -> Status {
    classify_android(
        is_granted(id),
        call_bool("isDeclared", id),
        call_bool("shouldShowRationale", id),
    )
}

// ---------------------------------------------------------------------------
// The part's per-OS contract
// ---------------------------------------------------------------------------

pub fn gate(perm: Permission) -> Gate {
    match perm {
        // The user can still switch notifications off in Settings below API 33 — the OS keeps a
        // consent record even though no dialog exists — so this is `Prompts` with `can_prompt`
        // answering false, not `Ungated`.
        Permission::Notifications => Gate::Prompts,
        // Motion below API 29 is install-time: nothing gates it at runtime.
        Permission::Motion if sdk_int() < 29 => Gate::Ungated,
        _ => Gate::Prompts,
    }
}

pub fn status(perm: Permission) -> Status {
    if perm == Permission::Notifications && sdk_int() < 33 {
        return if notifications_enabled() {
            Status::Granted
        } else {
            Status::Denied
        };
    }
    let ids = native_ids(perm);
    if ids.is_empty() {
        return Status::Granted; // ungated on this API level
    }
    ids.iter()
        .map(|id| id_status(id))
        .reduce(merge)
        .unwrap_or(Status::Unsupported)
}

pub fn status_async(perm: Permission, on_done: Box<dyn FnOnce(Status) + Send>) {
    // Every Android probe is synchronous.
    on_done(status(perm));
}

pub fn can_prompt(perm: Permission) -> bool {
    // Below API 33 notifications can only be changed in Settings; on API 30+ so can background
    // location, which the system refuses to prompt for in-app.
    if perm == Permission::Notifications && sdk_int() < 33 {
        return false;
    }
    if perm == Permission::LocationAlways && sdk_int() >= 30 {
        return false;
    }
    // `Prompt` covers both never-asked and permanently-denied (Day keeps no state to tell them
    // apart) — asking in the permanent case simply resolves `Denied` without a dialog.
    matches!(status(perm), Status::Prompt | Status::Denied) && gate(perm) == Gate::Prompts
}

pub fn should_show_rationale(perm: Permission) -> bool {
    native_ids(perm)
        .iter()
        .any(|id| call_bool("shouldShowRationale", id))
}

/// In-flight requests: token → (permission, the native ids asked for, the completion).
static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

fn next_token() -> i64 {
    static NEXT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn pending_lock() -> std::sync::MutexGuard<'static, Option<Pending>> {
    match PENDING.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub fn request(perm: Permission, on_done: Box<dyn FnOnce(Status) + Send>) {
    let ids = native_ids(perm);
    if ids.is_empty() {
        // Nothing to ask for on this API level: notifications below 33 and motion below 29 are
        // settled by the current state.
        on_done(status(perm));
        return;
    }
    let token = next_token();
    pending_lock()
        .get_or_insert_with(HashMap::new)
        .insert(token, (perm, ids.clone(), on_done));

    let joined = ids.join(&SEP.to_string());
    let launched = with_env(|env| {
        let Ok(s) = env.new_string(&joined) else {
            return false;
        };
        env.dcall_static(
            CLASS,
            "request",
            "(JLjava/lang/String;)V",
            &[JValue::Long(token), JValue::Object(&s)],
        )
        .is_ok()
    });
    if !launched {
        // The shim never got the call, so nativeResult will never fire — resolve here instead of
        // leaving the future pending forever.
        let entry = pending_lock().as_mut().and_then(|m| m.remove(&token));
        if let Some((perm, _, cb)) = entry {
            cb(status(perm));
        }
    }
}

pub fn open_settings(_perm: Permission) -> bool {
    // Android has one destination: this app's details page. `perm` cannot narrow it further.
    with_env(|env| {
        env.dcall_static(CLASS, "openSettings", "()Z", &[])
            .ok()
            .and_then(|v| v.z().ok())
            .unwrap_or(false)
    })
}

/// The Java shim's result callback: bit `i` of `granted_mask` is set when the `i`th requested id
/// was granted.
///
/// The final status folds the mask (authoritative for granted-ness, straight from the dialog) with
/// fresh `isDeclared`/`shouldShowRationale` probes, since a fresh denial flips the rationale flag.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_daybrite_day_permissions_DayPermissions_nativeResult(
    _env: day_android::jni::EnvUnowned<'_>,
    _class: day_android::jni::objects::JClass<'_>,
    token: day_android::jni::sys::jlong,
    granted_mask: day_android::jni::sys::jlong,
) {
    let entry = pending_lock().as_mut().and_then(|m| m.remove(&token));
    let Some((_perm, ids, on_done)) = entry else {
        return; // already answered
    };
    let status = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let granted = i < 64 && (granted_mask & (1i64 << i)) != 0;
            classify_android(
                granted,
                call_bool("isDeclared", id),
                call_bool("shouldShowRationale", id),
            )
        })
        .reduce(merge)
        .unwrap_or(Status::Unsupported);
    on_done(status);
}
