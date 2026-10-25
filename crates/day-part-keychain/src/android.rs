// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Android: files in the app's no-backup directory, encrypted with keys the Android Keystore
//! holds, through an inline Java arm (docs/bridge.md).
//!
//! The Keystore keeps keys, not items, so the arm keeps one file per item under
//! `getNoBackupFilesDir()/day-keychain/` (never carried by Auto Backup: a restore to another
//! device would bring ciphertext nothing can open), in the record layout `record.rs` describes.
//!
//! # The two keys
//!
//! `WhenUnlocked` and `AfterFirstUnlock` items are AES-256-GCM under one keystore key, with the
//! item's key name as the associated data. The Keystore unlocks with the first sign-in and the
//! two levels are alike here.
//!
//! `UserPresence` items are AES-256-GCM under a fresh per-item key, which an RSA key pair in the
//! Keystore wraps with OAEP. Encrypting with the public key needs no prompt, so `set` never shows
//! one; the private key is generated with user authentication required for every use, and the
//! only way to use it is through a cipher the system prompt authorized (`CryptoObject`). On API 30
//! and later the device credential is accepted alongside a biometric; on API 28 and 29 the prompt
//! is biometric-only, because the platform does not combine a credential with a `CryptoObject`
//! there. A change of biometric enrollment does not invalidate the key (the item is bound to the
//! person's device credential as much as to a fingerprint); removing the screen lock does, and
//! the item then reads as `Unreadable`. Below API 28 there is no prompt that can authorize a key,
//! so the level is `Unsupported`.
//!
//! Keys are created by `set`, never by a read: a wiped keystore reads as `Unreadable` rather than
//! silently starting over.
//!
//! # The wire
//!
//! Every answer is a status byte, followed by the payload where there is one (see `Status`):
//! `get_native` returns `status ‖ secret`, `get_presence_native` completes with the same, and
//! the others return the status alone. `entries_native` returns `<level digit><key>` entries
//! joined by U+001F. An exception in the arm fails the call with its message.

use crate::bridge_glue::{
    get_presence_native_async, has_item, list_items, read, remove_item, store, support_level,
};
use crate::{Entry, KeychainError, OnDone, Protection, Support};

/// The arm's status codes.
mod status {
    pub const OK: u8 = 0;
    pub const NOT_FOUND: u8 = 1;
    pub const NEEDS_PRESENCE: u8 = 2;
    pub const UNREADABLE: u8 = 3;
    pub const UNSUPPORTED: u8 = 4;
    pub const CANCELLED: u8 = 5;
    pub const AUTH_FAILED: u8 = 6;
    pub const PASSCODE_NOT_SET: u8 = 7;
    pub const LOCKOUT: u8 = 8;
    pub const UNAVAILABLE: u8 = 10;
    pub const TOO_LARGE: u8 = 11;
    pub const PLATFORM: u8 = 20;
}

/// A status byte (and the message a platform error carries) in the crate's vocabulary.
fn error_from_status(code: u8, message: &[u8]) -> KeychainError {
    match code {
        status::NOT_FOUND => KeychainError::NotFound,
        status::NEEDS_PRESENCE => KeychainError::NeedsPresence,
        status::UNREADABLE => KeychainError::Unreadable,
        status::UNSUPPORTED => KeychainError::Unsupported,
        status::CANCELLED => KeychainError::Cancelled,
        status::AUTH_FAILED => KeychainError::AuthFailed,
        status::PASSCODE_NOT_SET => KeychainError::PasscodeNotSet,
        status::LOCKOUT => KeychainError::Lockout,
        status::UNAVAILABLE => KeychainError::Unavailable,
        status::TOO_LARGE => KeychainError::TooLarge,
        status::PLATFORM => KeychainError::Platform(String::from_utf8_lossy(message).into_owned()),
        other => KeychainError::Platform(format!("keystore status {other}")),
    }
}

/// A status-only answer.
fn check(code: i32) -> Result<(), KeychainError> {
    match u8::try_from(code) {
        Ok(status::OK) => Ok(()),
        Ok(code) => Err(error_from_status(code, &[])),
        Err(_) => Err(KeychainError::Platform(format!("keystore status {code}"))),
    }
}

/// A `status ‖ payload` answer.
fn unwrap_payload(mut bytes: Vec<u8>) -> Result<Vec<u8>, KeychainError> {
    match bytes.first().copied() {
        Some(status::OK) => {
            bytes.remove(0);
            Ok(bytes)
        }
        Some(code) => Err(error_from_status(code, &bytes[1..])),
        None => Err(KeychainError::Platform("empty keystore answer".into())),
    }
}

fn bridge_error(e: day_bridge::Error) -> KeychainError {
    match e {
        day_bridge::Error::Unsupported => KeychainError::Unsupported,
        day_bridge::Error::Runtime => KeychainError::Unavailable,
        day_bridge::Error::Foreign(message) => KeychainError::Platform(message),
        day_bridge::Error::Encoding => KeychainError::Platform("bridge encoding error".into()),
    }
}

pub fn support() -> Support {
    Support::Native
}

pub fn protection_support(protection: Protection) -> Support {
    match support_level(i32::from(protection.code())) {
        Ok(0) => Support::Native,
        _ => Support::Unsupported,
    }
}

pub fn backend() -> &'static str {
    "android-keystore"
}

pub fn set(key: &str, secret: &[u8], protection: Protection) -> Result<(), KeychainError> {
    store(key, secret, i32::from(protection.code()))
        .map_err(bridge_error)
        .and_then(check)
}

pub fn get(key: &str) -> Result<Vec<u8>, KeychainError> {
    read(key).map_err(bridge_error).and_then(unwrap_payload)
}

pub fn get_with_presence(key: String, reason: String, on_done: OnDone) {
    // Either channel alone is a complete answer (docs/bridge.md "Callbacks"): an arm that fails
    // to start has already fired the callback with its error, so the return is not read.
    let _ = get_presence_native_async(&key, &reason, move |answer| {
        on_done(answer.map_err(bridge_error).and_then(unwrap_payload));
    });
}

pub fn remove(key: &str) -> Result<(), KeychainError> {
    remove_item(key).map_err(bridge_error).and_then(check)
}

pub fn contains(key: &str) -> Result<bool, KeychainError> {
    match has_item(key).map_err(bridge_error)? {
        0 => Ok(false),
        1 => Ok(true),
        code => check(code).map(|()| false),
    }
}

pub fn entries() -> Result<Vec<Entry>, KeychainError> {
    let listing = list_items().map_err(bridge_error)?;
    let mut out = Vec::new();
    for item in listing.split('\u{1f}').filter(|s| !s.is_empty()) {
        let mut chars = item.chars();
        let level = chars
            .next()
            .and_then(|c| c.to_digit(10))
            .and_then(|d| u8::try_from(d).ok());
        if let Some(protection) = level.and_then(Protection::from_code) {
            out.push(Entry {
                key: chars.as_str().to_string(),
                protection,
            });
        }
    }
    Ok(out)
}
