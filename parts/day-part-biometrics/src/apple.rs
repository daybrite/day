// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! iOS and macOS: LocalAuthentication's `LAContext`.
//!
//! One class and four selectors: `canEvaluatePolicy:error:` answers [`availability`],
//! `biometryType` answers [`biometry`], `evaluatePolicy:localizedReason:reply:` shows the dialog,
//! and `invalidate` dismisses it. Raw `objc2` + `block2`, as day-part-permissions does for its
//! authorization calls: a wrapper crate for one class is not worth the graph.
//!
//! # A fresh context per prompt
//!
//! A context that has evaluated once answers the next `evaluatePolicy:` from its cached result,
//! without showing anything. Every prompt therefore gets a new `LAContext`, which is kept alive
//! (in [`ACTIVE`]) until its reply arrives, so [`cancel`] can `invalidate` it and so the runtime
//! does not release it under a dialog still on screen.
//!
//! # Policies
//!
//! `LAPolicyDeviceOwnerAuthenticationWithBiometrics` (1) asks for a biometric and nothing else; its
//! fallback button reports [`BiometricError::UserFallback`]. `LAPolicyDeviceOwnerAuthentication` (2)
//! falls back to the passcode or account password on its own, which is what
//! [`Prompt::device_credential`] selects. [`availability`] always asks about policy 1, because
//! the question is whether a biometric is ready; a device without one can still pass policy 2.
//!
//! The reply block runs on a LocalAuthentication queue, which is where the callback is invoked;
//! the crate's contract makes it `Send` for that reason.

use std::sync::Mutex;

use block2::RcBlock;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2_foundation::{NSError, NSString};

use crate::{Availability, BiometricError, Biometry, Method, OnDone, Prompt, Support, Verified};

// Link the framework the class comes from. An empty extern block is enough: the attribute emits
// the `-framework` flag, and every call goes through the Objective-C runtime. On iOS the app links
// it through `[package.metadata.day.ios].frameworks` instead, since xcodebuild ignores this.
#[link(name = "LocalAuthentication", kind = "framework")]
unsafe extern "C" {}

/// `LAPolicyDeviceOwnerAuthenticationWithBiometrics`.
const POLICY_BIOMETRICS: isize = 1;
/// `LAPolicyDeviceOwnerAuthentication`: biometrics, then the passcode or account password.
const POLICY_ANY: isize = 2;

/// `LAError` codes, from `LAError.h`.
const LA_AUTHENTICATION_FAILED: isize = -1;
const LA_USER_CANCEL: isize = -2;
const LA_USER_FALLBACK: isize = -3;
const LA_SYSTEM_CANCEL: isize = -4;
const LA_PASSCODE_NOT_SET: isize = -5;
const LA_BIOMETRY_NOT_AVAILABLE: isize = -6;
const LA_BIOMETRY_NOT_ENROLLED: isize = -7;
const LA_BIOMETRY_LOCKOUT: isize = -8;
const LA_APP_CANCEL: isize = -9;
const LA_NOT_INTERACTIVE: isize = -1004;

/// `LABiometryType`.
const BIOMETRY_NONE: isize = 0;
const BIOMETRY_TOUCH_ID: isize = 1;
const BIOMETRY_FACE_ID: isize = 2;

/// The context of the prompt on screen, as a raw +1 pointer so the slot is `Send`. Taken and
/// released when the reply arrives; `invalidate`d by [`cancel`].
static ACTIVE: Mutex<Option<ContextPtr>> = Mutex::new(None);

/// A retained `LAContext` the reply closure owns across threads.
struct ContextPtr(*mut AnyObject);

// SAFETY: LAContext is documented as usable from any thread, and this pointer is only ever
// dereferenced for `invalidate` (thread-safe) and for the final release, which happens once.
unsafe impl Send for ContextPtr {}

fn active() -> std::sync::MutexGuard<'static, Option<ContextPtr>> {
    match ACTIVE.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Look the class up without panicking: a framework that failed to link leaves us reporting
/// `Unsupported` rather than aborting the app.
fn context_class() -> Option<&'static AnyClass> {
    AnyClass::get(c"LAContext")
}

/// A fresh `LAContext`, or `None` where LocalAuthentication is not linked.
fn new_context() -> Option<Retained<AnyObject>> {
    let cls = context_class()?;
    // SAFETY: `+new` returns a +1 reference, which `Retained` owns.
    unsafe { msg_send![cls, new] }
}

/// `canEvaluatePolicy:error:` on a fresh context: `Ok` when the policy can be evaluated now, else
/// the `LAError` code saying why not.
fn can_evaluate(ctx: &AnyObject, policy: isize) -> Result<(), isize> {
    // SAFETY: the selector takes an NSInteger policy and an out NSError**, which objc2 turns into
    // a `Result` for a method returning BOOL with a trailing `error:` argument.
    let r: Result<(), Retained<NSError>> =
        unsafe { msg_send![ctx, canEvaluatePolicy: policy, error: _] };
    r.map_err(|e| e.code())
}

pub fn support() -> Support {
    if context_class().is_some() {
        Support::Native
    } else {
        Support::Unsupported
    }
}

pub fn availability() -> Availability {
    let Some(ctx) = new_context() else {
        return Availability::Unsupported;
    };
    match can_evaluate(&ctx, POLICY_BIOMETRICS) {
        Ok(()) => Availability::Available,
        Err(LA_PASSCODE_NOT_SET) => Availability::PasscodeNotSet,
        Err(LA_BIOMETRY_NOT_AVAILABLE) => Availability::NoHardware,
        Err(LA_BIOMETRY_NOT_ENROLLED) => Availability::NotEnrolled,
        Err(LA_BIOMETRY_LOCKOUT) => Availability::Lockout,
        // -1004 is the no-UI case (a context marked non-interactive, a daemon); nothing else is
        // documented for this call, so anything left is the framework declining.
        Err(_) => Availability::Unsupported,
    }
}

pub fn biometry() -> Biometry {
    let Some(ctx) = new_context() else {
        return Biometry::None;
    };
    // `biometryType` is defined only after `canEvaluatePolicy:` ran on the same context; the
    // answer is valid whether or not that call succeeded (a not-enrolled Face ID device still
    // reports Face ID).
    let _ = can_evaluate(&ctx, POLICY_BIOMETRICS);
    // SAFETY: `biometryType` is a readonly NSInteger property.
    let kind: isize = unsafe { msg_send![&*ctx, biometryType] };
    match kind {
        BIOMETRY_NONE => Biometry::None,
        BIOMETRY_TOUCH_ID => Biometry::Fingerprint,
        BIOMETRY_FACE_ID => Biometry::Face,
        // Optic ID (4), or a kind newer than this table.
        _ => Biometry::Unknown,
    }
}

/// Wrap a one-shot completion so it can live in an Objective-C block.
///
/// `RcBlock` requires `Fn` (a block may be invoked any number of times), while the completion is
/// `FnOnce`. The mutex holds it until the first invocation and drops it after, so a framework that
/// calls back twice is harmless instead of a double-move.
fn once(cb: OnDone) -> impl Fn(Result<Verified, BiometricError>) + 'static {
    let cell = Mutex::new(Some(cb));
    move |outcome| {
        let taken = match cell.lock() {
            Ok(mut g) => g.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        if let Some(cb) = taken {
            cb(outcome);
        }
    }
}

/// The error the reply carries, in the crate's vocabulary.
fn map_error(error: *mut NSError) -> BiometricError {
    if error.is_null() {
        return BiometricError::Failed;
    }
    // SAFETY: the reply block's contract hands a live NSError for the duration of the call.
    let error = unsafe { &*error };
    match error.code() {
        LA_AUTHENTICATION_FAILED => BiometricError::Failed,
        LA_USER_CANCEL => BiometricError::UserCancelled,
        LA_USER_FALLBACK => BiometricError::UserFallback,
        LA_SYSTEM_CANCEL | LA_APP_CANCEL => BiometricError::SystemCancelled,
        LA_PASSCODE_NOT_SET => BiometricError::PasscodeNotSet,
        LA_BIOMETRY_NOT_AVAILABLE => BiometricError::NoHardware,
        LA_BIOMETRY_NOT_ENROLLED => BiometricError::NotEnrolled,
        LA_BIOMETRY_LOCKOUT => BiometricError::Lockout,
        LA_NOT_INTERACTIVE => BiometricError::NotInteractive,
        code => BiometricError::Platform {
            code: code as i32,
            message: error.localizedDescription().to_string(),
        },
    }
}

pub fn authenticate(prompt: Prompt, on_done: OnDone) {
    let Some(ctx) = new_context() else {
        on_done(Err(BiometricError::Unsupported));
        return;
    };
    let policy = if prompt.allows_device_credential() {
        POLICY_ANY
    } else {
        POLICY_BIOMETRICS
    };
    if let Some(title) = prompt.fallback_title_text() {
        let title = NSString::from_str(title);
        // SAFETY: `localizedFallbackTitle` is a readwrite NSString property.
        let _: () = unsafe { msg_send![&*ctx, setLocalizedFallbackTitle: &*title] };
    }
    let reason = NSString::from_str(prompt.reason());

    let settle = once(on_done);
    let method = if policy == POLICY_BIOMETRICS {
        Method::Biometric
    } else {
        // Policy 2 passes on a biometric or the credential and the reply does not say which.
        Method::Unknown
    };
    let reply = RcBlock::new(move |success: Bool, error: *mut NSError| {
        // Release the context this reply belongs to. A newer prompt cannot be on screen (the
        // crate allows one at a time), so the slot is ours.
        if let Some(ContextPtr(ptr)) = active().take() {
            // SAFETY: the pointer came from `Retained::into_raw` below and is released once, here.
            drop(unsafe { Retained::from_raw(ptr) });
        }
        if success.as_bool() {
            settle(Ok(Verified {
                method,
                token: Vec::new(),
            }));
        } else {
            settle(Err(map_error(error)));
        }
    });

    // Park the context so `cancel` can reach it, and so it outlives this function: the dialog
    // holds a weak reference and would end with `LAErrorAppCancel` if the last strong one went.
    let raw = Retained::into_raw(ctx);
    *active() = Some(ContextPtr(raw));
    // SAFETY: `raw` is a live +1 LAContext parked above; the selector takes an NSInteger policy,
    // an NSString reason and a `void (^)(BOOL, NSError *)` block, which `reply` is.
    unsafe {
        let _: () =
            msg_send![raw, evaluatePolicy: policy, localizedReason: &*reason, reply: &*reply];
    }
}

pub fn cancel() {
    if let Some(ContextPtr(ptr)) = active().as_ref() {
        // SAFETY: the parked pointer is a live LAContext until its reply releases it; `invalidate`
        // is documented as callable from any thread and makes the pending reply fail with -9.
        let _: () = unsafe { msg_send![*ptr, invalidate] };
    }
}
