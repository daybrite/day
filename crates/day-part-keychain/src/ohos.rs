// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! HarmonyOS: the Universal Keystore (HUKS) over one file per item in the app sandbox.
//!
//! The Asset Store Kit keeps items but caps a secret at 1 KiB; the Universal Keystore keeps keys,
//! encrypts any size, and binds a key to user authentication. This arm takes the keystore: three
//! AES-256-GCM keys it generates on first use (one per [`Protection`] level), and a file per item
//! (`record.rs`) under the app's files directory whose payload is `nonce ‖ ciphertext ‖ tag`.
//! The item's key name is the cipher's associated data, so a payload moved to another item's
//! file does not decrypt. Everything is C through `libhuks_ndk.z.so` (the public NDK), except the
//! user-authentication dialog, which the kit offers to ArkTS only and `day-part-biometrics`
//! provides.
//!
//! # The three keys
//!
//! | level | storage level | user authentication |
//! |---|---|---|
//! | `WhenUnlocked` | ECE (usable only while the device is unlocked) | none |
//! | `AfterFirstUnlock` | CE (usable from the first unlock until reboot) | none |
//! | `UserPresence` | ECE | PIN, face or fingerprint, for decryption only |
//!
//! A `UserPresence` key is generated with `KEY_AUTH_PURPOSE = DECRYPT`, so a `set` encrypts
//! without a dialog and only a read asks. Its access type is `INVALID_CLEAR_PASSWORD`: removing the
//! lock-screen password invalidates the key, and every item under it reads as
//! [`KeychainError::Unreadable`] afterwards, which is what the other platforms do too. A device
//! with no lock-screen password cannot generate it, and a `set` fails with
//! [`KeychainError::PasscodeNotSet`].
//!
//! # Reading a protected item
//!
//! `InitSession` on the protected key returns a 32-byte challenge. The challenge goes into the
//! prompt ([`Prompt::challenge`]); the dialog's token, bound to it, goes back into
//! `FinishSession` as `AUTH_TOKEN`, which decrypts. The dialog runs on the JS thread and its
//! completion arrives there, which is where the decryption finishes.

use std::ffi::{CStr, c_char};
use std::path::PathBuf;

use crate::record;
use crate::{Entry, KeychainError, OnDone, Protection, Support};
use day_part_biometrics::{BiometricError, Prompt};

#[link(name = "huks_ndk.z")]
unsafe extern "C" {
    fn OH_Huks_GenerateKeyItem(
        alias: *const Blob,
        params: *const ParamSet,
        out: *mut ParamSet,
    ) -> HuksResult;
    fn OH_Huks_IsKeyItemExist(alias: *const Blob, params: *const ParamSet) -> HuksResult;
    fn OH_Huks_InitSession(
        alias: *const Blob,
        params: *const ParamSet,
        handle: *mut Blob,
        token: *mut Blob,
    ) -> HuksResult;
    fn OH_Huks_FinishSession(
        handle: *const Blob,
        params: *const ParamSet,
        input: *const Blob,
        output: *mut Blob,
    ) -> HuksResult;
    fn OH_Huks_AbortSession(handle: *const Blob, params: *const ParamSet) -> HuksResult;
    fn OH_Huks_InitParamSet(set: *mut *mut ParamSet) -> HuksResult;
    fn OH_Huks_AddParams(set: *mut ParamSet, params: *const Param, count: u32) -> HuksResult;
    fn OH_Huks_BuildParamSet(set: *mut *mut ParamSet) -> HuksResult;
    fn OH_Huks_FreeParamSet(set: *mut *mut ParamSet);
}

/// `OH_Huks_Blob`.
#[repr(C)]
struct Blob {
    size: u32,
    data: *mut u8,
}

impl Blob {
    fn of(bytes: &[u8]) -> Blob {
        Blob {
            size: bytes.len() as u32,
            data: bytes.as_ptr().cast_mut(),
        }
    }
    fn of_mut(bytes: &mut [u8]) -> Blob {
        Blob {
            size: bytes.len() as u32,
            data: bytes.as_mut_ptr(),
        }
    }
}

/// `OH_Huks_Param`'s value union.
#[repr(C)]
union ParamValue {
    bool_param: bool,
    int32_param: i32,
    uint32_param: u32,
    uint64_param: u64,
    blob: std::mem::ManuallyDrop<Blob>,
}

/// `OH_Huks_Param`.
#[repr(C)]
struct Param {
    tag: u32,
    value: ParamValue,
}

impl Param {
    fn uint(tag: u32, value: u32) -> Param {
        Param {
            tag,
            value: ParamValue {
                uint32_param: value,
            },
        }
    }
    fn bytes(tag: u32, bytes: &[u8]) -> Param {
        Param {
            tag,
            value: ParamValue {
                blob: std::mem::ManuallyDrop::new(Blob::of(bytes)),
            },
        }
    }
}

/// `OH_Huks_ParamSet`: opaque here, built through the helpers.
#[repr(C)]
struct ParamSet {
    _private: [u8; 0],
}

/// `OH_Huks_Result`, returned by value.
#[repr(C)]
struct HuksResult {
    error_code: i32,
    error_msg: *const c_char,
    data: *mut u8,
}

impl HuksResult {
    fn check(self) -> Result<(), KeychainError> {
        if self.error_code == 0 {
            return Ok(());
        }
        Err(huks_error(self.error_code, self.error_msg))
    }
}

/// `OH_Huks_Tag` values (type bits included).
const TAG_UINT: u32 = 2 << 28;
const TAG_BYTES: u32 = 5 << 28;
const TAG_ALGORITHM: u32 = TAG_UINT | 1;
const TAG_PURPOSE: u32 = TAG_UINT | 2;
const TAG_KEY_SIZE: u32 = TAG_UINT | 3;
const TAG_PADDING: u32 = TAG_UINT | 5;
const TAG_BLOCK_MODE: u32 = TAG_UINT | 6;
const TAG_ASSOCIATED_DATA: u32 = TAG_BYTES | 8;
const TAG_NONCE: u32 = TAG_BYTES | 9;
const TAG_USER_AUTH_TYPE: u32 = TAG_UINT | 304;
const TAG_AUTH_TOKEN: u32 = TAG_BYTES | 306;
const TAG_KEY_AUTH_ACCESS_TYPE: u32 = TAG_UINT | 307;
const TAG_CHALLENGE_TYPE: u32 = TAG_UINT | 309;
const TAG_KEY_AUTH_PURPOSE: u32 = TAG_UINT | 311;
const TAG_AUTH_STORAGE_LEVEL: u32 = TAG_UINT | 316;
const TAG_AE_TAG: u32 = TAG_BYTES | 10009;

/// Algorithm parameters.
const ALG_AES: u32 = 20;
const PURPOSE_ENCRYPT: u32 = 1;
const PURPOSE_DECRYPT: u32 = 2;
const AES_KEY_SIZE_256: u32 = 256;
const PADDING_NONE: u32 = 0;
const MODE_GCM: u32 = 32;
const STORAGE_LEVEL_CE: u32 = 1;
const STORAGE_LEVEL_ECE: u32 = 2;
const USER_AUTH_FINGERPRINT: u32 = 1 << 0;
const USER_AUTH_FACE: u32 = 1 << 1;
const USER_AUTH_PIN: u32 = 1 << 2;
const AUTH_ACCESS_INVALID_CLEAR_PASSWORD: u32 = 1 << 0;
const CHALLENGE_TYPE_NORMAL: u32 = 0;

/// Error codes.
const ERR_FEATURE_NOT_SUPPORTED: i32 = 12_000_001;
const ERR_CRYPTO_FAIL: i32 = 12_000_006;
const ERR_KEY_AUTH_PERMANENTLY_INVALIDATED: i32 = 12_000_007;
const ERR_KEY_AUTH_VERIFY_FAILED: i32 = 12_000_008;
const ERR_KEY_AUTH_TIME_OUT: i32 = 12_000_009;
const ERR_ITEM_NOT_EXIST: i32 = 12_000_011;
const ERR_DEVICE_PASSWORD_UNSET: i32 = 12_000_016;

/// AES-GCM sizes.
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
/// A session handle is a `uint64_t`; a challenge is 32 bytes.
const HANDLE_LEN: usize = 8;
const CHALLENGE_LEN: usize = 32;

fn huks_error(code: i32, message: *const c_char) -> KeychainError {
    match code {
        ERR_ITEM_NOT_EXIST => KeychainError::Unreadable,
        ERR_KEY_AUTH_PERMANENTLY_INVALIDATED => KeychainError::Unreadable,
        ERR_CRYPTO_FAIL => KeychainError::Unreadable,
        ERR_KEY_AUTH_VERIFY_FAILED | ERR_KEY_AUTH_TIME_OUT => KeychainError::AuthFailed,
        ERR_DEVICE_PASSWORD_UNSET => KeychainError::PasscodeNotSet,
        ERR_FEATURE_NOT_SUPPORTED => KeychainError::Unsupported,
        _ => {
            let text = if message.is_null() {
                String::new()
            } else {
                // SAFETY: HUKS hands back a NUL-terminated static message or null.
                unsafe { CStr::from_ptr(message) }
                    .to_string_lossy()
                    .into_owned()
            };
            KeychainError::Platform(format!("HUKS {code}: {text}"))
        }
    }
}

/// A built `OH_Huks_ParamSet`, freed on drop.
struct Params(*mut ParamSet);

impl Params {
    fn build(params: &[Param]) -> Result<Params, KeychainError> {
        let mut set: *mut ParamSet = std::ptr::null_mut();
        // SAFETY: the three helpers allocate, fill and serialize the set; every blob parameter
        // points into memory the caller keeps alive for as long as the `Params` lives.
        unsafe {
            OH_Huks_InitParamSet(&mut set).check()?;
            let added = OH_Huks_AddParams(set, params.as_ptr(), params.len() as u32).check();
            let built = added.and_then(|()| OH_Huks_BuildParamSet(&mut set).check());
            if let Err(e) = built {
                OH_Huks_FreeParamSet(&mut set);
                return Err(e);
            }
        }
        Ok(Params(set))
    }
}

impl Drop for Params {
    fn drop(&mut self) {
        // SAFETY: built by `OH_Huks_BuildParamSet`, freed once.
        unsafe { OH_Huks_FreeParamSet(&mut self.0) };
    }
}

/// The keystore alias for a level (at most 64 bytes).
fn alias(protection: Protection) -> &'static [u8] {
    match protection {
        Protection::WhenUnlocked => b"dev.daybrite.day.keychain.unlocked.v1",
        Protection::AfterFirstUnlock => b"dev.daybrite.day.keychain.firstunlock.v1",
        Protection::UserPresence => b"dev.daybrite.day.keychain.presence.v1",
    }
}

fn storage_level(protection: Protection) -> u32 {
    match protection {
        Protection::AfterFirstUnlock => STORAGE_LEVEL_CE,
        Protection::WhenUnlocked | Protection::UserPresence => STORAGE_LEVEL_ECE,
    }
}

/// The parameters every cipher session names, plus the storage level.
fn cipher_params(purpose: u32, protection: Protection) -> Vec<Param> {
    vec![
        Param::uint(TAG_ALGORITHM, ALG_AES),
        Param::uint(TAG_PURPOSE, purpose),
        Param::uint(TAG_KEY_SIZE, AES_KEY_SIZE_256),
        Param::uint(TAG_PADDING, PADDING_NONE),
        Param::uint(TAG_BLOCK_MODE, MODE_GCM),
        Param::uint(TAG_AUTH_STORAGE_LEVEL, storage_level(protection)),
    ]
}

/// Whether the key for `protection` exists.
fn key_exists(protection: Protection) -> Result<bool, KeychainError> {
    let params = Params::build(&[Param::uint(
        TAG_AUTH_STORAGE_LEVEL,
        storage_level(protection),
    )])?;
    let alias = Blob::of(alias(protection));
    // SAFETY: the alias and the set are live for the call.
    let result = unsafe { OH_Huks_IsKeyItemExist(&alias, params.0) };
    match result.error_code {
        0 => Ok(true),
        ERR_ITEM_NOT_EXIST => Ok(false),
        code => Err(huks_error(code, result.error_msg)),
    }
}

/// Generate the key for `protection`. Only a write creates keys: a read that finds none has
/// nothing to decrypt with, and says so.
fn ensure_key(protection: Protection) -> Result<(), KeychainError> {
    if key_exists(protection)? {
        return Ok(());
    }
    let mut params = cipher_params(PURPOSE_ENCRYPT | PURPOSE_DECRYPT, protection);
    if protection == Protection::UserPresence {
        params.extend([
            Param::uint(
                TAG_USER_AUTH_TYPE,
                USER_AUTH_FINGERPRINT | USER_AUTH_FACE | USER_AUTH_PIN,
            ),
            Param::uint(TAG_KEY_AUTH_ACCESS_TYPE, AUTH_ACCESS_INVALID_CLEAR_PASSWORD),
            Param::uint(TAG_CHALLENGE_TYPE, CHALLENGE_TYPE_NORMAL),
            Param::uint(TAG_KEY_AUTH_PURPOSE, PURPOSE_DECRYPT),
        ]);
    }
    let params = Params::build(&params)?;
    let alias = Blob::of(alias(protection));
    // SAFETY: the alias and the set are live for the call; a persistent key takes no out set.
    unsafe { OH_Huks_GenerateKeyItem(&alias, params.0, std::ptr::null_mut()) }.check()
}

/// Twelve bytes from the OS random device.
fn nonce() -> Result<[u8; NONCE_LEN], KeychainError> {
    use std::io::Read;
    let mut out = [0u8; NONCE_LEN];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut out))
        .map_err(|e| KeychainError::Platform(format!("random: {e}")))?;
    Ok(out)
}

/// `InitSession` → `FinishSession` in one go, with no user authentication.
fn run_session(
    protection: Protection,
    params: &Params,
    input: &[u8],
    output_len: usize,
) -> Result<Vec<u8>, KeychainError> {
    let alias = Blob::of(alias(protection));
    let mut handle_bytes = [0u8; HANDLE_LEN];
    let mut handle = Blob::of_mut(&mut handle_bytes);
    // SAFETY: every blob points at live memory sized as stated; a key without user
    // authentication fills no token, so none is asked for.
    unsafe { OH_Huks_InitSession(&alias, params.0, &mut handle, std::ptr::null_mut()) }.check()?;
    finish(&handle, params, input, output_len)
}

fn finish(
    handle: &Blob,
    params: &Params,
    input: &[u8],
    output_len: usize,
) -> Result<Vec<u8>, KeychainError> {
    let mut out = vec![0u8; output_len.max(1)];
    let mut out_blob = Blob::of_mut(&mut out);
    let in_blob = Blob::of(input);
    // SAFETY: the handle came from `InitSession`; the output blob's size is its capacity on
    // entry and the bytes written on return.
    let result = unsafe { OH_Huks_FinishSession(handle, params.0, &in_blob, &mut out_blob) };
    if let Err(e) = result.check() {
        // The session is consumed by a failed finish on most builds; aborting a dead handle is a
        // harmless no-op and keeps a live one from leaking on the others.
        let _ = unsafe { OH_Huks_AbortSession(handle, params.0) };
        return Err(e);
    }
    out.truncate(out_blob.size as usize);
    Ok(out)
}

fn encrypt(protection: Protection, key: &str, plain: &[u8]) -> Result<Vec<u8>, KeychainError> {
    let nonce = nonce()?;
    let mut params = cipher_params(PURPOSE_ENCRYPT, protection);
    params.push(Param::bytes(TAG_NONCE, &nonce));
    params.push(Param::bytes(TAG_ASSOCIATED_DATA, key.as_bytes()));
    let params = Params::build(&params)?;
    let sealed = run_session(protection, &params, plain, plain.len() + TAG_LEN)?;
    let mut payload = Vec::with_capacity(NONCE_LEN + sealed.len());
    payload.extend_from_slice(&nonce);
    payload.extend_from_slice(&sealed);
    Ok(payload)
}

/// Split a payload into the cipher parameters and the ciphertext it decrypts.
fn decrypt_params(
    protection: Protection,
    key: &str,
    payload: &[u8],
) -> Result<(Vec<Param>, std::ops::Range<usize>), KeychainError> {
    if payload.len() < NONCE_LEN + TAG_LEN {
        return Err(KeychainError::Unreadable);
    }
    let (nonce, rest) = payload.split_at(NONCE_LEN);
    let (ciphertext, tag) = rest.split_at(rest.len() - TAG_LEN);
    let mut params = cipher_params(PURPOSE_DECRYPT, protection);
    params.push(Param::bytes(TAG_NONCE, nonce));
    params.push(Param::bytes(TAG_ASSOCIATED_DATA, key.as_bytes()));
    params.push(Param::bytes(TAG_AE_TAG, tag));
    Ok((params, NONCE_LEN..NONCE_LEN + ciphertext.len()))
}

fn decrypt(protection: Protection, key: &str, payload: &[u8]) -> Result<Vec<u8>, KeychainError> {
    let (params, range) = decrypt_params(protection, key, payload)?;
    let built = Params::build(&params)?;
    let ciphertext = &payload[range];
    run_session(protection, &built, ciphertext, ciphertext.len())
}

/// The store directory: the app's files directory, which the ArkTS host exports as `DAY_DATA_DIR`.
fn store_dir() -> Result<PathBuf, KeychainError> {
    let dir = std::env::var_os("DAY_DATA_DIR")
        .filter(|d| !d.is_empty())
        .ok_or(KeychainError::Unavailable)?;
    Ok(PathBuf::from(dir).join("day-keychain"))
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
            KeychainError::Platform(format!("user authentication: {message} ({code})"))
        }
    }
}

pub fn support() -> Support {
    Support::Native
}

pub fn protection_support(protection: Protection) -> Support {
    match protection {
        Protection::UserPresence if day_part_biometrics::support() == Support::Unsupported => {
            Support::Unsupported
        }
        _ => Support::Native,
    }
}

pub fn backend() -> &'static str {
    "huks"
}

pub fn set(key: &str, secret: &[u8], protection: Protection) -> Result<(), KeychainError> {
    let dir = store_dir()?;
    ensure_key(protection)?;
    let payload = encrypt(protection, key, secret)?;
    record::write(&dir, key, protection, &payload)
}

pub fn get(key: &str) -> Result<Vec<u8>, KeychainError> {
    let item = record::read(&store_dir()?, key)?;
    if item.protection == Protection::UserPresence {
        return Err(KeychainError::NeedsPresence);
    }
    decrypt(item.protection, key, &item.payload)
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
        on_done(decrypt(item.protection, &key, &item.payload));
        return;
    }
    // Open the session first: it issues the challenge the dialog binds its token to. The
    // parameter set points into `key` and the payload, so it is built again (from the same owned
    // bytes) when the answer arrives, instead of crossing to the completion as raw pointers.
    let payload = item.payload;
    let (handle_bytes, challenge) = match open_presence_session(&key, &payload) {
        Ok(opened) => opened,
        Err(e) => {
            on_done(Err(e));
            return;
        }
    };
    day_part_biometrics::authenticate(Prompt::new(reason).challenge(challenge), move |verified| {
        on_done(finish_presence_session(
            &key,
            &payload,
            handle_bytes,
            verified,
        ));
    });
}

/// `InitSession` on the protected key: the session handle and the challenge it issued.
fn open_presence_session(
    key: &str,
    payload: &[u8],
) -> Result<([u8; HANDLE_LEN], Vec<u8>), KeychainError> {
    let (params, _) = decrypt_params(Protection::UserPresence, key, payload)?;
    let built = Params::build(&params)?;
    let alias_blob = Blob::of(alias(Protection::UserPresence));
    let mut handle_bytes = [0u8; HANDLE_LEN];
    let mut challenge = [0u8; CHALLENGE_LEN];
    let mut handle = Blob::of_mut(&mut handle_bytes);
    let mut challenge_blob = Blob::of_mut(&mut challenge);
    // SAFETY: every blob points at live memory sized as stated.
    unsafe { OH_Huks_InitSession(&alias_blob, built.0, &mut handle, &mut challenge_blob) }
        .check()?;
    let challenge_len = (challenge_blob.size as usize).min(CHALLENGE_LEN);
    Ok((handle_bytes, challenge[..challenge_len].to_vec()))
}

/// `FinishSession` with the dialog's token, or `AbortSession` when there is none.
fn finish_presence_session(
    key: &str,
    payload: &[u8],
    mut handle_bytes: [u8; HANDLE_LEN],
    verified: Result<day_part_biometrics::Verified, BiometricError>,
) -> Result<Vec<u8>, KeychainError> {
    let (mut params, range) = decrypt_params(Protection::UserPresence, key, payload)?;
    let handle = Blob::of_mut(&mut handle_bytes);
    match verified {
        Ok(v) => {
            params.push(Param::bytes(TAG_AUTH_TOKEN, &v.token));
            let set = Params::build(&params)?;
            let ciphertext = &payload[range];
            finish(&handle, &set, ciphertext, ciphertext.len())
        }
        Err(e) => {
            if let Ok(set) = Params::build(&params) {
                // SAFETY: the handle is the live session opened for this read; aborting releases it.
                let _ = unsafe { OH_Huks_AbortSession(&handle, set.0) };
            }
            Err(biometric_error(e))
        }
    }
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
