// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! HarmonyOS: the User Authentication Kit's system dialog, through the ArkTS arm in `bridge.rs`.
//!
//! The kit has no C API in the public SDK (no header under `sysroot/usr/include` names it), so
//! both halves are ArkTS: `getAvailableStatus` for the probes, and
//! `getUserAuthInstance(...).start()` for the dialog, whose `onResult` completes the prompt.
//! `ohos.permission.ACCESS_BIOMETRIC` (`system_grant`, no reason) comes from the portable
//! `biometrics` permission this crate declares.
//!
//! # The token
//!
//! A passed prompt returns the kit's `AuthToken` (up to 1024 bytes). It is bound to the
//! `challenge` the prompt was given, and a Universal Keystore key generated with user
//! authentication accepts it once, which is how `day-part-keychain` reads a
//! `Protection::UserPresence` item here: the keystore issues a 32-byte challenge, this prompt
//! turns it into a token, the keystore finishes the decryption with the token. The arm hands the
//! token back with one leading byte naming the kit's `UserAuthType` that passed.
//!
//! # Modalities
//!
//! The dialog shows the modalities it is given, and refuses a list naming one the device lacks.
//! The arm therefore probes face and fingerprint (and the PIN, when the prompt allows the device
//! credential) with `getAvailableStatus` and lists only the ones the device has an executor for,
//! enrolled or not; an enrolled-nothing device then fails at `start()` with `NOT_ENROLLED`, which
//! is the answer. Every call runs on the JS thread; the completion arrives there too.

use crate::bridge::{
    authenticate_native_async, authenticate_native_support, cancel_prompt, probe_availability,
    probe_biometry,
};
use crate::{Availability, BiometricError, Biometry, Method, OnDone, Prompt, Support, Verified};

/// `userAuth.UserAuthType`.
const TYPE_PIN: i32 = 1;
const TYPE_FACE: i32 = 2;
const TYPE_FINGERPRINT: i32 = 4;

/// `userAuth.UserAuthResultCode` and the kit's own error codes.
const CODE_FAIL: i32 = 12_500_001;
const CODE_GENERAL: i32 = 12_500_002;
const CODE_CANCELED: i32 = 12_500_003;
const CODE_TIMEOUT: i32 = 12_500_004;
const CODE_TYPE_NOT_SUPPORT: i32 = 12_500_005;
const CODE_TRUST_LEVEL_NOT_SUPPORT: i32 = 12_500_006;
const CODE_BUSY: i32 = 12_500_007;
const CODE_INVALID_PARAMETERS: i32 = 12_500_008;
const CODE_LOCKED: i32 = 12_500_009;
const CODE_NOT_ENROLLED: i32 = 12_500_010;
const CODE_CANCELED_FROM_WIDGET: i32 = 12_500_011;
const CODE_PIN_EXPIRED: i32 = 12_500_013;
/// `ohos.permission.ACCESS_BIOMETRIC` is missing from the app's `module.json5`.
const CODE_PERMISSION: i32 = 201;

/// The kit takes a challenge of at most this many bytes.
const MAX_CHALLENGE: usize = 32;

pub fn support() -> Support {
    // Reached where `day build` staged the ArkTS module.
    if authenticate_native_support() == Support::Unsupported {
        Support::Unsupported
    } else {
        Support::Native
    }
}

/// `getAvailableStatus` for one modality at ATL1: `Ok` when it can authenticate now, else the
/// kit's code saying why not.
fn probe(kind: i32) -> Result<(), i32> {
    match probe_availability(kind) {
        Ok(0) => Ok(()),
        Ok(code) => Err(code),
        Err(_) => Err(CODE_TYPE_NOT_SUPPORT),
    }
}

pub fn availability() -> Availability {
    if support() == Support::Unsupported {
        return Availability::Unsupported;
    }
    let face = probe(TYPE_FACE);
    let finger = probe(TYPE_FINGERPRINT);
    if face.is_ok() || finger.is_ok() {
        return Availability::Available;
    }
    let codes = [face, finger].map(|r| r.err().unwrap_or(0));
    if codes.contains(&CODE_PERMISSION) {
        // Not declared: the prompt cannot be shown at all, whatever the hardware.
        return Availability::Unsupported;
    }
    if codes.contains(&CODE_LOCKED) {
        return Availability::Lockout;
    }
    if codes.contains(&CODE_NOT_ENROLLED) {
        return Availability::NotEnrolled;
    }
    if codes.contains(&CODE_PIN_EXPIRED) {
        return Availability::PasscodeNotSet;
    }
    if codes
        .iter()
        .all(|c| *c == CODE_TYPE_NOT_SUPPORT || *c == CODE_TRUST_LEVEL_NOT_SUPPORT)
    {
        return Availability::NoHardware;
    }
    Availability::Unsupported
}

pub fn biometry() -> Biometry {
    if support() == Support::Unsupported {
        return Biometry::None;
    }
    match probe_biometry() {
        Ok(1) => Biometry::Fingerprint,
        Ok(2) => Biometry::Face,
        Ok(3) => Biometry::Unknown,
        _ => Biometry::None,
    }
}

/// The kit's result code, in the crate's vocabulary.
fn error_from_code(code: i32) -> BiometricError {
    match code {
        CODE_FAIL | CODE_TIMEOUT => BiometricError::Failed,
        CODE_CANCELED => BiometricError::UserCancelled,
        CODE_TYPE_NOT_SUPPORT | CODE_TRUST_LEVEL_NOT_SUPPORT => BiometricError::NoHardware,
        CODE_BUSY => BiometricError::Busy,
        CODE_INVALID_PARAMETERS => BiometricError::InvalidPrompt,
        CODE_LOCKED => BiometricError::Lockout,
        CODE_NOT_ENROLLED => BiometricError::NotEnrolled,
        CODE_CANCELED_FROM_WIDGET => BiometricError::UserFallback,
        CODE_PIN_EXPIRED => BiometricError::PasscodeNotSet,
        CODE_GENERAL => BiometricError::Platform {
            code,
            message: "general user authentication error".to_string(),
        },
        CODE_PERMISSION => BiometricError::Platform {
            code,
            message: "ohos.permission.ACCESS_BIOMETRIC is not declared".to_string(),
        },
        other => BiometricError::Platform {
            code: other,
            message: String::new(),
        },
    }
}

/// The arm's failure message is the kit's code as decimal text.
fn error_from_bridge(e: day_bridge::Error) -> BiometricError {
    match e {
        day_bridge::Error::Unsupported => BiometricError::Unsupported,
        day_bridge::Error::Foreign(message) => match message.trim().parse::<i32>() {
            Ok(code) => error_from_code(code),
            Err(_) => BiometricError::Platform { code: 0, message },
        },
        day_bridge::Error::Encoding => BiometricError::Platform {
            code: 0,
            message: "bridge encoding error".to_string(),
        },
        day_bridge::Error::Runtime => BiometricError::NotInteractive,
    }
}

pub fn authenticate(prompt: Prompt, on_done: OnDone) {
    if prompt.challenge_bytes().len() > MAX_CHALLENGE {
        on_done(Err(BiometricError::InvalidPrompt));
        return;
    }
    // Either channel alone is a complete answer (docs/bridge.md "Callbacks"): an arm that fails
    // to start has already fired the callback with its error, so the return is not read.
    let _ = authenticate_native_async(
        prompt.challenge_bytes(),
        prompt.reason(),
        prompt.fallback_title_text().unwrap_or(""),
        prompt.allows_device_credential(),
        move |answer| {
            on_done(match answer {
                Ok(bytes) => {
                    let method = match bytes.first().copied().map(i32::from) {
                        Some(TYPE_PIN) => Method::DeviceCredential,
                        Some(TYPE_FACE) | Some(TYPE_FINGERPRINT) => Method::Biometric,
                        _ => Method::Unknown,
                    };
                    Ok(Verified {
                        method,
                        token: bytes.get(1..).map(<[u8]>::to_vec).unwrap_or_default(),
                    })
                }
                Err(e) => Err(error_from_bridge(e)),
            })
        },
    );
}

pub fn cancel() {
    let _ = cancel_prompt();
}
