// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! iOS and macOS: the keychain, as generic-password items.
//!
//! Every item is a `kSecClassGenericPassword` with the app id as its service and the key as its
//! account, which is the pair the keychain treats as the item's identity. The data protection
//! keychain (`kSecUseDataProtectionKeychain`) is asked for everywhere: it is the only keychain on
//! iOS, and on macOS it is the one with per-item access control and no legacy ACL dialogs. Items
//! are `ThisDeviceOnly`: the keys that protect them never leave the device, and nothing here is
//! backed up or synchronized.
//!
//! # macOS without an entitlement
//!
//! The data protection keychain needs the `keychain-access-groups` entitlement, which a signed
//! app has and an unsigned development binary (`cargo run`, `day launch` on a dev build) does
//! not: every call answers `errSecMissingEntitlement`. This arm probes that once and falls back to
//! the login keychain, the file-based store the same calls address without the flag.
//! [`backend`] then says `login-keychain`, [`support`] says `Emulated`, and
//! [`Protection::UserPresence`] is `Unsupported` there (access control exists only in the data
//! protection keychain). A signed build never takes this path.
//!
//! # Reading a protected item
//!
//! The synchronous [`get`] passes `kSecUseAuthenticationUI = Fail`, so the keychain refuses with
//! `errSecInteractionNotAllowed` instead of showing anything; an attribute-only query (which needs
//! no authentication) then tells a protected item (`NeedsPresence`) from a locked device
//! (`Locked`). [`get_with_presence`] hands the keychain an `LAContext` carrying the reason, and
//! the system shows its dialog during `SecItemCopyMatching`, which is why that call runs on a
//! thread of its own. The `UserPresence` access control is satisfied by a biometric or the
//! passcode, and is not invalidated by a change of enrolment.

use std::ptr::NonNull;
use std::sync::OnceLock;

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_core_foundation::{
    CFArray, CFBoolean, CFData, CFDictionary, CFMutableDictionary, CFRetained, CFString, CFType,
};
use objc2_foundation::NSString;
use objc2_security::{
    SecAccessControl, SecAccessControlCreateFlags, SecItemAdd, SecItemCopyMatching, SecItemDelete,
    kSecAttrAccessControl, kSecAttrAccessible, kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
    kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly, kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
    kSecAttrAccount, kSecAttrLabel, kSecAttrService, kSecClass, kSecClassGenericPassword,
    kSecMatchLimit, kSecMatchLimitAll, kSecMatchLimitOne, kSecReturnAttributes, kSecReturnData,
    kSecUseAuthenticationContext, kSecUseDataProtectionKeychain, kSecValueData,
};

use crate::{Entry, KeychainError, OnDone, Protection, Support};

// Link the frameworks the calls come from. On iOS the app links them through
// `[package.metadata.day.ios].frameworks` instead, since xcodebuild ignores this.
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {}
#[link(name = "LocalAuthentication", kind = "framework")]
unsafe extern "C" {}

/// `OSStatus` values from `SecBase.h`.
const ERR_SUCCESS: i32 = 0;
const ERR_USER_CANCELED: i32 = -128;
const ERR_AUTH_FAILED: i32 = -25293;
const ERR_DUPLICATE_ITEM: i32 = -25299;
const ERR_ITEM_NOT_FOUND: i32 = -25300;
const ERR_DATA_TOO_LARGE: i32 = -25302;
const ERR_INTERACTION_NOT_ALLOWED: i32 = -25308;
const ERR_MISSING_ENTITLEMENT: i32 = -34018;
const ERR_NOT_AVAILABLE: i32 = -25291;

type Query = CFRetained<CFMutableDictionary<CFString, CFType>>;

/// Whether the data protection keychain answers in this process (see the module note). Probed
/// once: an entitlement does not change while the process runs.
///
/// The probe is a write, because only a write is refused: a read of a missing item answers
/// `errSecItemNotFound` from either keychain. The one-byte probe item is removed at once.
fn data_protection() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| {
        if cfg!(target_os = "ios") {
            return true;
        }
        let probe = base_query(Some(PROBE_KEY), true);
        // SAFETY: the `kSec*` statics are the framework's constant strings; the data is retained.
        unsafe {
            probe.set(kSecValueData, &CFData::from_bytes(b"probe"));
            probe.set(
                kSecAttrAccessible,
                kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
            );
        }
        // SAFETY: the dictionary is live for both calls; no result is asked for.
        let status = unsafe { SecItemAdd(as_dictionary(&probe), std::ptr::null_mut()) };
        if status == ERR_SUCCESS || status == ERR_DUPLICATE_ITEM {
            let _ = unsafe { SecItemDelete(as_dictionary(&base_query(Some(PROBE_KEY), true))) };
        }
        status != ERR_MISSING_ENTITLEMENT
    })
}

/// The account of the item [`data_protection`] writes and removes to learn which keychain
/// answers. Never listed by [`entries`]: it exists for one call.
const PROBE_KEY: &str = "dev.daybrite.day.keychain.probe";

/// The dictionary every call starts from: the class, this app's service, the key when given, and
/// the data protection flag when that keychain is in use.
fn base_query(key: Option<&str>, data_protection: bool) -> Query {
    let query = CFMutableDictionary::<CFString, CFType>::empty();
    // SAFETY: the `kSec*` statics are the framework's constant strings, valid for the process.
    unsafe {
        query.set(kSecClass, kSecClassGenericPassword);
        query.set(kSecAttrService, &CFString::from_str(&crate::app_id()));
        if let Some(key) = key {
            query.set(kSecAttrAccount, &CFString::from_str(key));
        }
        if data_protection {
            query.set(kSecUseDataProtectionKeychain, CFBoolean::new(true));
        }
    }
    query
}

/// The typed dictionary as the untyped one the Security calls take.
fn as_dictionary(query: &Query) -> &CFDictionary {
    let ptr: *const CFMutableDictionary<CFString, CFType> = &**query;
    // SAFETY: a CFMutableDictionary is a CFDictionary, and the type parameters are a Rust-side
    // annotation over the same opaque object.
    unsafe { &*ptr.cast::<CFDictionary>() }
}

/// `SecItemCopyMatching`, with the result retained when the status is success.
fn copy_matching(query: &Query) -> Result<Option<CFRetained<CFType>>, i32> {
    let mut result: *const CFType = std::ptr::null();
    // SAFETY: the dictionary is live for the call; `result` receives a +1 reference on success,
    // owned by the `CFRetained` built from it, or stays null.
    let status = unsafe { SecItemCopyMatching(as_dictionary(query), &mut result) };
    if status != ERR_SUCCESS {
        return Err(status);
    }
    Ok(NonNull::new(result.cast_mut()).map(|p| unsafe { CFRetained::from_raw(p) }))
}

/// An `OSStatus` in the crate's vocabulary.
fn status_error(status: i32) -> KeychainError {
    match status {
        ERR_ITEM_NOT_FOUND => KeychainError::NotFound,
        ERR_USER_CANCELED => KeychainError::Cancelled,
        ERR_AUTH_FAILED => KeychainError::AuthFailed,
        ERR_INTERACTION_NOT_ALLOWED => KeychainError::Locked,
        ERR_DATA_TOO_LARGE => KeychainError::TooLarge,
        ERR_NOT_AVAILABLE => KeychainError::Unavailable,
        ERR_MISSING_ENTITLEMENT => KeychainError::Platform(
            "the data protection keychain needs the keychain-access-groups entitlement".into(),
        ),
        other => KeychainError::Platform(format!("OSStatus {other}")),
    }
}

/// The level an item's attributes record.
fn protection_of(attributes: &CFDictionary<CFString, CFType>) -> Protection {
    // SAFETY: the `kSec*` statics are the framework's constant strings.
    unsafe {
        if attributes.contains_key(kSecAttrAccessControl) {
            return Protection::UserPresence;
        }
        let accessible = attributes
            .get(kSecAttrAccessible)
            .and_then(|v| v.downcast::<CFString>().ok())
            .map(|s| s.to_string());
        if accessible.as_deref()
            == Some(&kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly.to_string())
        {
            Protection::AfterFirstUnlock
        } else {
            Protection::WhenUnlocked
        }
    }
}

/// The attributes of the item under `key`, which the keychain hands out without authentication
/// even for a protected item. `Ok(None)` when there is no such item.
fn attributes_of(key: &str) -> Result<Option<Protection>, KeychainError> {
    let query = base_query(Some(key), data_protection());
    unsafe {
        query.set(kSecReturnAttributes, CFBoolean::new(true));
        query.set(kSecMatchLimit, kSecMatchLimitOne);
    }
    match copy_matching(&query) {
        Ok(Some(found)) => {
            // SAFETY: an attributes query answers a dictionary; the cast only names its key and
            // value types.
            let attrs = found
                .downcast::<CFDictionary>()
                .map_err(|_| KeychainError::Platform("attributes are not a dictionary".into()))?;
            let attrs =
                unsafe { CFRetained::cast_unchecked::<CFDictionary<CFString, CFType>>(attrs) };
            Ok(Some(protection_of(&attrs)))
        }
        Ok(None) => Ok(None),
        Err(ERR_ITEM_NOT_FOUND) => Ok(None),
        Err(status) => Err(status_error(status)),
    }
}

pub fn support() -> Support {
    if data_protection() {
        Support::Native
    } else {
        Support::Emulated
    }
}

pub fn protection_support(protection: Protection) -> Support {
    match (protection, data_protection()) {
        (_, true) => Support::Native,
        // The login keychain has no per-item access control and no accessibility classes.
        (Protection::UserPresence, false) => Support::Unsupported,
        (_, false) => Support::Emulated,
    }
}

pub fn backend() -> &'static str {
    if data_protection() {
        "data-protection-keychain"
    } else {
        "login-keychain"
    }
}

pub fn set(key: &str, secret: &[u8], protection: Protection) -> Result<(), KeychainError> {
    let data_protection = data_protection();
    if protection == Protection::UserPresence && !data_protection {
        return Err(KeychainError::Unsupported);
    }
    // Replace rather than update: an item's access control cannot be changed in place, and a
    // delete-then-add gives every level the same path.
    match remove(key) {
        Ok(()) | Err(KeychainError::NotFound) => {}
        Err(e) => return Err(e),
    }
    let attributes = base_query(Some(key), data_protection);
    // SAFETY: the `kSec*` statics are the framework's constant strings; every value set here is a
    // retained CF object that outlives the call.
    unsafe {
        attributes.set(kSecValueData, &CFData::from_bytes(secret));
        attributes.set(
            kSecAttrLabel,
            &CFString::from_str(&format!("{}: {key}", crate::app_id())),
        );
        if data_protection {
            match protection {
                Protection::WhenUnlocked => {
                    attributes.set(
                        kSecAttrAccessible,
                        kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
                    );
                }
                Protection::AfterFirstUnlock => {
                    attributes.set(
                        kSecAttrAccessible,
                        kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
                    );
                }
                Protection::UserPresence => {
                    // Biometrics or the passcode, each time, on a device that has a passcode;
                    // `kSecAttrAccessible` must not be set alongside the control.
                    let control = SecAccessControl::with_flags(
                        None,
                        kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly,
                        SecAccessControlCreateFlags::UserPresence,
                        std::ptr::null_mut(),
                    )
                    .ok_or_else(|| {
                        KeychainError::Platform("SecAccessControlCreateWithFlags failed".into())
                    })?;
                    attributes.set(kSecAttrAccessControl, &control);
                }
            }
        }
    }
    // SAFETY: the dictionary is live for the call; no result is asked for.
    let status = unsafe { SecItemAdd(as_dictionary(&attributes), std::ptr::null_mut()) };
    match status {
        ERR_SUCCESS => Ok(()),
        // A passcode-only access class cannot be written on a device without a passcode.
        ERR_AUTH_FAILED if protection == Protection::UserPresence => {
            Err(KeychainError::PasscodeNotSet)
        }
        ERR_DUPLICATE_ITEM => Err(KeychainError::Platform(
            "the item reappeared during replace".into(),
        )),
        other => Err(status_error(other)),
    }
}

/// Read the secret of `key` with `query`'s authentication settings applied.
fn read_data(query: &Query) -> Result<Vec<u8>, KeychainError> {
    unsafe {
        query.set(kSecReturnData, CFBoolean::new(true));
        query.set(kSecMatchLimit, kSecMatchLimitOne);
    }
    match copy_matching(query) {
        Ok(Some(found)) => found
            .downcast::<CFData>()
            .map(|data| data.to_vec())
            .map_err(|_| KeychainError::Unreadable),
        Ok(None) => Err(KeychainError::NotFound),
        Err(status) => Err(status_error(status)),
    }
}

/// A fresh `LAContext` whose `interactionNotAllowed` is set: the keychain then refuses a
/// protected item with `errSecInteractionNotAllowed` instead of showing its dialog (the
/// replacement for the deprecated `kSecUseAuthenticationUI`).
fn non_interactive_context() -> Option<Retained<AnyObject>> {
    let cls = AnyClass::get(c"LAContext")?;
    // SAFETY: `+new` returns a +1 reference; `interactionNotAllowed` is a readwrite BOOL property.
    unsafe {
        let ctx: Retained<AnyObject> = msg_send![cls, new];
        let _: () = msg_send![&*ctx, setInteractionNotAllowed: true];
        Some(ctx)
    }
}

/// Attach `context` to `query`, for the duration of the caller's use of both.
fn use_context(query: &Query, context: &Retained<AnyObject>) {
    let ptr: *const AnyObject = Retained::as_ptr(context);
    // SAFETY: an Objective-C object is a CFTypeRef (toll-free bridging), and the caller keeps
    // `context` alive for as long as the dictionary entry is read.
    unsafe { query.set(kSecUseAuthenticationContext, &*ptr.cast::<CFType>()) };
}

pub fn get(key: &str) -> Result<Vec<u8>, KeychainError> {
    let query = base_query(Some(key), data_protection());
    // Never a dialog from the synchronous call: the keychain refuses instead.
    let context = non_interactive_context();
    if let Some(ctx) = &context {
        use_context(&query, ctx);
    }
    match read_data(&query) {
        Err(KeychainError::Locked) => match attributes_of(key)? {
            Some(Protection::UserPresence) => Err(KeychainError::NeedsPresence),
            Some(_) => Err(KeychainError::Locked),
            None => Err(KeychainError::NotFound),
        },
        other => other,
    }
}

/// A fresh `LAContext` whose `localizedReason` the keychain's dialog shows.
fn authentication_context(reason: &str) -> Option<Retained<AnyObject>> {
    let cls = AnyClass::get(c"LAContext")?;
    // SAFETY: `+new` returns a +1 reference; `localizedReason` is a readwrite NSString property.
    unsafe {
        let ctx: Retained<AnyObject> = msg_send![cls, new];
        let reason = NSString::from_str(reason);
        let _: () = msg_send![&*ctx, setLocalizedReason: &*reason];
        Some(ctx)
    }
}

pub fn get_with_presence(key: String, reason: String, on_done: OnDone) {
    let spawned = std::thread::Builder::new()
        .name("day-keychain".into())
        .spawn(move || {
            let query = base_query(Some(&key), data_protection());
            // The context stays alive until the read returns: the dialog is attached to it.
            let context = authentication_context(&reason);
            if let Some(ctx) = &context {
                use_context(&query, ctx);
            }
            on_done(read_data(&query));
        });
    if let Err(e) = spawned {
        eprintln!("day-part-keychain: could not start the read thread: {e}");
    }
}

pub fn remove(key: &str) -> Result<(), KeychainError> {
    let query = base_query(Some(key), data_protection());
    // SAFETY: the dictionary is live for the call.
    let status = unsafe { SecItemDelete(as_dictionary(&query)) };
    match status {
        ERR_SUCCESS => Ok(()),
        other => Err(status_error(other)),
    }
}

pub fn contains(key: &str) -> Result<bool, KeychainError> {
    attributes_of(key).map(|found| found.is_some())
}

pub fn entries() -> Result<Vec<Entry>, KeychainError> {
    let query = base_query(None, data_protection());
    unsafe {
        query.set(kSecReturnAttributes, CFBoolean::new(true));
        query.set(kSecMatchLimit, kSecMatchLimitAll);
    }
    let found = match copy_matching(&query) {
        Ok(Some(found)) => found,
        Ok(None) | Err(ERR_ITEM_NOT_FOUND) => return Ok(Vec::new()),
        Err(status) => return Err(status_error(status)),
    };
    let array = found
        .downcast::<CFArray>()
        .map_err(|_| KeychainError::Platform("the match list is not an array".into()))?;
    // SAFETY: a match-all attributes query answers an array of attribute dictionaries; the cast
    // names their types.
    let array =
        unsafe { CFRetained::cast_unchecked::<CFArray<CFDictionary<CFString, CFType>>>(array) };
    let mut out = Vec::new();
    for attrs in array.iter() {
        // SAFETY: the account attribute is the framework's constant key.
        let key = unsafe { attrs.get(kSecAttrAccount) }
            .and_then(|v| v.downcast::<CFString>().ok())
            .map(|s| s.to_string());
        if let Some(key) = key {
            out.push(Entry {
                key,
                protection: protection_of(&attrs),
            });
        }
    }
    Ok(out)
}
