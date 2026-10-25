// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Android: the platform prompt in `android.hardware.biometrics` (API 28 and later), through the
//! Java arm in `bridge.rs`. Below API 28 there is no prompt an app may show without the
//! deprecated fingerprint API and its own permission, so this crate reports `Unsupported` there
//! (Day's floor is API 24). This file is the Rust reading of the arm's answers.

use crate::bridge::{authenticate_native_async, cancel_prompt, probe_availability, probe_biometry};
use crate::{Availability, BiometricError, Biometry, Method, OnDone, Prompt, Support, Verified};

/// The first API level with the platform prompt.
const MIN_SDK: i32 = 28;

pub fn support() -> Support {
    if day_platform_android::sdk_int() >= MIN_SDK {
        Support::Native
    } else {
        Support::Unsupported
    }
}

/// The arm's availability code (`bridge.rs`, "The wire").
fn availability_from_code(code: i32) -> Availability {
    match code {
        0 => Availability::Available,
        1 => Availability::NotEnrolled,
        2 => Availability::NoHardware,
        4 => Availability::Busy,
        5 => Availability::DisabledByPolicy,
        6 => Availability::PasscodeNotSet,
        _ => Availability::Unsupported,
    }
}

pub fn availability() -> Availability {
    if support() == Support::Unsupported {
        return Availability::Unsupported;
    }
    probe_availability(0).map_or(Availability::Unsupported, availability_from_code)
}

pub fn biometry() -> Biometry {
    if support() == Support::Unsupported {
        return Biometry::None;
    }
    match probe_biometry() {
        Ok(0) | Err(_) => Biometry::None,
        Ok(1) => Biometry::Fingerprint,
        Ok(2) => Biometry::Face,
        Ok(4) => Biometry::Iris,
        // Several sensors: the prompt picks, the app is told "a biometric".
        Ok(_) => Biometry::Unknown,
    }
}

/// The prompt's `BIOMETRIC_ERROR_*` code, plus the arm's own numbers, in the crate's vocabulary.
fn error_from_code(code: i32) -> BiometricError {
    match code {
        1000 => BiometricError::UserFallback, // the negative button
        3000 => BiometricError::Unsupported,
        1 => BiometricError::Busy, // HW_UNAVAILABLE: the sensor is busy or asleep
        2 | 3 => BiometricError::Failed, // UNABLE_TO_PROCESS, TIMEOUT
        5 => BiometricError::SystemCancelled, // CANCELED
        7 => BiometricError::Lockout,
        9 => BiometricError::LockoutPermanent,
        10 => BiometricError::UserCancelled,
        11 => BiometricError::NotEnrolled,      // NO_BIOMETRICS
        12 => BiometricError::NoHardware,       // HW_NOT_PRESENT
        14 => BiometricError::PasscodeNotSet,   // NO_DEVICE_CREDENTIAL
        15 => BiometricError::DisabledByPolicy, // SECURITY_UPDATE_REQUIRED
        other => BiometricError::Platform {
            code: other,
            message: String::new(),
        },
    }
}

/// The arm fails with a code as decimal text, or an exception's description.
fn error_from_bridge(e: day_bridge::Error) -> BiometricError {
    match e {
        day_bridge::Error::Unsupported => BiometricError::Unsupported,
        day_bridge::Error::Runtime => BiometricError::NotInteractive,
        day_bridge::Error::Foreign(message) => match message.trim().parse::<i32>() {
            Ok(code) => error_from_code(code),
            Err(_) => BiometricError::Platform { code: 0, message },
        },
        day_bridge::Error::Encoding => BiometricError::Platform {
            code: 0,
            message: "bridge encoding error".to_string(),
        },
    }
}

/// The prompt's authentication type, the first byte of a pass.
fn method_from_kind(kind: Option<u8>) -> Method {
    match kind {
        Some(1) => Method::DeviceCredential,
        Some(2) => Method::Biometric,
        _ => Method::Unknown,
    }
}

pub fn authenticate(prompt: Prompt, on_done: OnDone) {
    if support() == Support::Unsupported {
        on_done(Err(BiometricError::Unsupported));
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
                Ok(bytes) => Ok(Verified {
                    method: method_from_kind(bytes.first().copied()),
                    token: Vec::new(),
                }),
                Err(e) => Err(error_from_bridge(e)),
            })
        },
    );
}

pub fn cancel() {
    let _ = cancel_prompt();
}
