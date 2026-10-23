// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Windows: DPAPI over one file per item, with Windows Hello in front of the items that need the
//! person.
//!
//! Windows has two stores an app could use and neither fits whole. The Credential Manager caps a
//! secret at 2560 bytes and loses concurrent updates; the Credential Locker holds twenty entries
//! per app and roams them through the Microsoft account. DPAPI (`CryptProtectData`) has no cap,
//! binds the bytes to the signed-in Windows account on this machine (another account, or another
//! machine, cannot decrypt them), adds its own integrity check, and needs no store: the protected
//! bytes go in a file of this crate's own (`record.rs`) under `%LOCALAPPDATA%`, where Windows
//! keeps the app's other private data.
//!
//! # Protection
//!
//! `WhenUnlocked` and `AfterFirstUnlock` both map to a plain DPAPI blob: Windows unlocks the
//! account's keys at sign-in and keeps them until sign-out, so there is no finer level to offer.
//! `UserPresence` records the level in the item and reads it only after Windows Hello verifies the
//! person ([`day_part_biometrics`]). DPAPI itself does not know about the check, so the item's
//! bytes are as decryptable as any other by code running as this user; this is why
//! [`protection_support`] reports `Emulated` for that level, where Apple, Android and HarmonyOS
//! report `Native`. A `set` at that level checks that Hello is set up first, so no item is ever
//! stored that nothing could read.
//!
//! `CRYPTPROTECT_UI_FORBIDDEN` is passed on both calls: the prompt-based DPAPI flow is deprecated
//! and this crate never wants one. No optional entropy and no description, so a blob is
//! self-describing and nothing else has to be stored to open it.

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};

use crate::record;
use crate::{Entry, KeychainError, OnDone, Protection, Support};
use day_part_biometrics::{Availability, BiometricError, Prompt};

/// The store directory: `DAY_DATA_DIR` when the CLI set one (tests, a host that relocates data),
/// else `%LOCALAPPDATA%\<app id>\day-keychain`. Local, not roaming: a DPAPI blob made on this
/// machine is unreadable on another.
fn store_dir() -> Result<std::path::PathBuf, KeychainError> {
    if let Some(dir) = std::env::var_os("DAY_DATA_DIR")
        && !dir.is_empty()
    {
        return Ok(std::path::PathBuf::from(dir).join("day-keychain"));
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .filter(|d| !d.is_empty())
        .or_else(|| std::env::var_os("APPDATA").filter(|d| !d.is_empty()))
        .ok_or(KeychainError::Unavailable)?;
    Ok(std::path::PathBuf::from(base)
        .join(crate::app_id())
        .join("day-keychain"))
}

/// Copy a DPAPI output blob into Rust memory and free it.
fn take_blob(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if blob.pbData.is_null() {
        return Vec::new();
    }
    // SAFETY: a successful DPAPI call hands back a LocalAlloc'd buffer of `cbData` bytes, which is
    // copied and then freed exactly once.
    unsafe {
        let bytes = std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec();
        LocalFree(Some(HLOCAL(blob.pbData.cast())));
        bytes
    }
}

fn protect(plain: &[u8]) -> Result<Vec<u8>, KeychainError> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(plain.len()).map_err(|_| KeychainError::TooLarge)?,
        pbData: plain.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `input` points at `plain`, live for the call; `output` receives an OS-allocated
    // buffer that `take_blob` frees. No description, entropy, reserved pointer or prompt struct.
    unsafe {
        CryptProtectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    }
    .map_err(platform_error)?;
    Ok(take_blob(output))
}

fn unprotect(blob: &[u8]) -> Result<Vec<u8>, KeychainError> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(blob.len()).map_err(|_| KeychainError::Unreadable)?,
        pbData: blob.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: as in `protect`; the description out-parameter is declined (None).
    let result = unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    match result {
        Ok(()) => Ok(take_blob(output)),
        // Any failure to open a blob that was ours is the same fact to the caller: a blob from
        // another account, a reset password, or a damaged file. The item stays until removed.
        Err(_) => Err(KeychainError::Unreadable),
    }
}

fn platform_error(e: windows::core::Error) -> KeychainError {
    // ERROR_NO_SUCH_LOGON_SESSION (1312): a network logon with no credential set.
    if e.code().0 & 0xFFFF == 1312 {
        return KeychainError::Unavailable;
    }
    KeychainError::Platform(e.message())
}

/// `Availability`, as the reason a `UserPresence` item cannot be stored or read here.
fn presence_error(a: Availability) -> KeychainError {
    match a {
        Availability::Available => KeychainError::Unsupported,
        // Hello is set up in Settings; until then nothing can verify the person.
        Availability::NotEnrolled | Availability::PasscodeNotSet => KeychainError::PasscodeNotSet,
        Availability::Lockout => KeychainError::Lockout,
        Availability::Busy => KeychainError::Locked,
        Availability::NoHardware | Availability::DisabledByPolicy | Availability::Unsupported => {
            KeychainError::Unsupported
        }
    }
}

fn biometric_error(e: BiometricError) -> KeychainError {
    match e {
        BiometricError::UserCancelled
        | BiometricError::SystemCancelled
        | BiometricError::UserFallback => KeychainError::Cancelled,
        BiometricError::Failed => KeychainError::AuthFailed,
        BiometricError::Lockout | BiometricError::LockoutPermanent => KeychainError::Lockout,
        BiometricError::NotEnrolled | BiometricError::PasscodeNotSet => {
            KeychainError::PasscodeNotSet
        }
        BiometricError::Busy => KeychainError::Locked,
        BiometricError::Unsupported
        | BiometricError::NoHardware
        | BiometricError::DisabledByPolicy
        | BiometricError::NotInteractive
        | BiometricError::InvalidPrompt => KeychainError::Unsupported,
        BiometricError::Platform { code, message } => {
            KeychainError::Platform(format!("Windows Hello: {message} ({code})"))
        }
    }
}

pub fn support() -> Support {
    Support::Native
}

pub fn protection_support(protection: Protection) -> Support {
    match protection {
        Protection::UserPresence => match day_part_biometrics::availability() {
            Availability::Unsupported
            | Availability::NoHardware
            | Availability::DisabledByPolicy => Support::Unsupported,
            _ => Support::Emulated,
        },
        _ => Support::Native,
    }
}

pub fn backend() -> &'static str {
    "dpapi"
}

pub fn set(key: &str, secret: &[u8], protection: Protection) -> Result<(), KeychainError> {
    if protection == Protection::UserPresence {
        // Never store what nothing could read: Hello has to be ready before the item exists.
        match day_part_biometrics::availability() {
            Availability::Available => {}
            other => return Err(presence_error(other)),
        }
    }
    let dir = store_dir()?;
    let blob = protect(secret)?;
    record::write(&dir, key, protection, &blob)
}

pub fn get(key: &str) -> Result<Vec<u8>, KeychainError> {
    let item = record::read(&store_dir()?, key)?;
    if item.protection == Protection::UserPresence {
        return Err(KeychainError::NeedsPresence);
    }
    unprotect(&item.payload)
}

pub fn get_with_presence(key: String, reason: String, on_done: OnDone) {
    let item = match store_dir().and_then(|dir| record::read(&dir, &key)) {
        Ok(item) => item,
        Err(e) => {
            on_done(Err(e));
            return;
        }
    };
    if item.protection != Protection::UserPresence {
        on_done(unprotect(&item.payload));
        return;
    }
    // The Hello window is owned by the active window, which `authenticate` captures on this
    // thread; the verification and the decryption that follows run on its worker.
    day_part_biometrics::authenticate(Prompt::new(reason), move |verified| {
        on_done(match verified {
            Ok(_) => unprotect(&item.payload),
            Err(e) => Err(biometric_error(e)),
        });
    });
}

pub fn remove(key: &str) -> Result<(), KeychainError> {
    record::remove(&store_dir()?, key)
}

pub fn contains(key: &str) -> Result<bool, KeychainError> {
    Ok(record::exists(&store_dir()?, key))
}

pub fn entries() -> Result<Vec<Entry>, KeychainError> {
    record::entries(&store_dir()?)
}
