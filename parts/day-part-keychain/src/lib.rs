// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-part-keychain: small secrets in the platform's own secure store, by key. No UI; any Rust
//! code can depend on this crate and keep a session token, a password or a key where the OS keeps
//! its own.
//!
//! ```no_run
//! use day_part_keychain::{Protection, Secret};
//!
//! day_part_keychain::set("session", &Secret::from("eyJhbGciOi…"), Protection::WhenUnlocked)?;
//! let token = day_part_keychain::get("session")?;
//! println!("{} bytes", token.len());
//! # Ok::<(), day_part_keychain::KeychainError>(())
//! ```
//!
//! Platform selection is `#[cfg(target_os)]`: the data protection keychain on iOS and macOS, the
//! Android Keystore over files in the app's no-backup directory, DPAPI over files under
//! `%LOCALAPPDATA%` on Windows, the Secret Service (GNOME Keyring, KDE's ksecretd, KeePassXC)
//! through libsecret on desktop Linux, and the Universal Keystore over files in the app sandbox
//! on HarmonyOS. The web has no store a page may rely on for a secret, so [`support`] reports
//! `Unsupported` there and every call fails with [`KeychainError::Unsupported`].
//!
//! # What an item is
//!
//! An item is a key (a short string the app chooses) and a [`Secret`] (up to [`MAX_SECRET_LEN`]
//! bytes), stored for this app on this device under a [`Protection`] level. Items belong to the
//! app that wrote them: each platform scopes them by the app's identity (the bundle, the package,
//! the user's profile and the app id). They survive a reinstall where the platform keeps keychain
//! data across one (Apple) and are removed with the app elsewhere; none of them is backed up or
//! synchronized, because the keys that encrypt them never leave the device.
//!
//! # Reading an item that needs the person
//!
//! A [`Protection::UserPresence`] item is readable only after the device verifies the person: a
//! biometric or the device credential, in the OS's own dialog. [`get`] is synchronous and never
//! shows that dialog; on such an item it returns [`KeychainError::NeedsPresence`], and the app
//! calls [`get_async`] or [`get_future`], which show the dialog and resolve with the secret. The
//! other protection levels resolve without a dialog in either form.
//!
//! # Threads
//!
//! The synchronous calls run on the caller's thread and finish in milliseconds on every platform
//! but Linux, where a locked keyring makes the Secret Service show its unlock dialog during the
//! call; UI code there prefers [`get_async`], which works on a thread of its own. Completions run
//! where the platform delivers them (a worker thread, an Android executor, the HarmonyOS JS
//! thread), which is why the callback is `Send`; [`get_future`] awaited under `day::task`
//! resumes on the UI thread. Nothing here posts to a main loop, so the crate works in a plain
//! `main` and under `cargo test` (docs/async.md rule 3).

use std::fmt;

pub use day_bridge::Support;

/// The longest key, in UTF-8 bytes. Long enough for a reverse-DNS name with an account after it,
/// short enough for every platform's identifier and file-name limits.
pub const MAX_KEY_LEN: usize = 128;

/// The largest secret, in bytes. A keychain holds credentials, not documents: tokens, passwords,
/// keys and the occasional certificate all fit, and every platform store below stays fast at this
/// size.
pub const MAX_SECRET_LEN: usize = 64 * 1024;

/// Bytes that are wiped when dropped.
///
/// A `Secret` owns its bytes and zeroes them (with volatile writes the optimizer cannot remove)
/// when it goes away, so a token read from the keychain does not linger in freed memory. It is
/// built from a string or bytes, read back as either, compared in constant time, and prints as
/// `Secret(<n> bytes)` so a stray `{:?}` never logs it.
pub struct Secret(Vec<u8>);

impl Secret {
    /// A secret from raw bytes.
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Secret(bytes.into())
    }

    /// The bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The bytes as text, when they are valid UTF-8.
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    /// The number of bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no bytes.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<&str> for Secret {
    fn from(s: &str) -> Self {
        Secret(s.as_bytes().to_vec())
    }
}

impl From<String> for Secret {
    fn from(s: String) -> Self {
        Secret(s.into_bytes())
    }
}

impl From<&[u8]> for Secret {
    fn from(b: &[u8]) -> Self {
        Secret(b.to_vec())
    }
}

impl From<Vec<u8>> for Secret {
    fn from(b: Vec<u8>) -> Self {
        Secret(b)
    }
}

impl Clone for Secret {
    fn clone(&self) -> Self {
        Secret(self.0.clone())
    }
}

impl PartialEq for Secret {
    /// Constant time in the length of the longer secret: a comparison against a stored value
    /// leaks nothing about where the first differing byte is.
    fn eq(&self, other: &Self) -> bool {
        let mut diff = (self.0.len() ^ other.0.len()) as u8;
        let longer = self.0.len().max(other.0.len());
        for i in 0..longer {
            let a = self.0.get(i).copied().unwrap_or(0);
            let b = other.0.get(i).copied().unwrap_or(0);
            diff |= a ^ b;
        }
        diff == 0
    }
}

impl Eq for Secret {}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret({} bytes)", self.0.len())
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

/// Zero a buffer, including the capacity past its length, with writes the optimizer keeps.
pub(crate) fn wipe(bytes: &mut Vec<u8>) {
    for b in bytes.iter_mut() {
        // SAFETY: `b` is a live, aligned `u8` inside the vector.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    for slot in bytes.spare_capacity_mut() {
        slot.write(0);
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

/// When an item may be read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Protection {
    /// While the device is unlocked, which is whenever the app's UI is on screen. The default,
    /// and right for a session token. Apple keeps the item unreadable while the device is locked;
    /// HarmonyOS likewise (the ECE storage level); the other platforms keep it readable whenever
    /// the app runs, since their keystores unlock with the first sign-in.
    #[default]
    WhenUnlocked,
    /// From the first unlock after boot until the next reboot, locked screen included. For work the
    /// app does in the background (a sync, a notification handler) that needs the secret while the
    /// screen is locked. Apple and HarmonyOS (the CE storage level) distinguish this from
    /// [`WhenUnlocked`](Protection::WhenUnlocked); the other platforms treat the two alike.
    AfterFirstUnlock,
    /// Only after the device verifies the person, every time: a biometric or the device credential
    /// in the OS's own dialog ([`get_async`]). Writing needs no dialog. Where the OS can bind the
    /// item to the check it does (Apple, Android, HarmonyOS: the key that decrypts the item is
    /// usable only after the check), and where it cannot (Windows) the check gates the read and
    /// [`protection_support`] says `Emulated`. The device must have a passcode, PIN or password;
    /// without one [`set`] fails with [`KeychainError::PasscodeNotSet`]. Not available on Linux.
    UserPresence,
}

// The file-backed stores and the Android arm record the level as a byte; the keychain and the
// Secret Service keep it as attributes of their own.
#[cfg(any(
    target_os = "windows",
    target_env = "ohos",
    target_os = "android",
    test
))]
impl Protection {
    /// The one-byte code the file-backed stores and the Android arm record.
    pub(crate) fn code(self) -> u8 {
        match self {
            Protection::WhenUnlocked => 0,
            Protection::AfterFirstUnlock => 1,
            Protection::UserPresence => 2,
        }
    }

    /// The inverse of [`Protection::code`].
    pub(crate) fn from_code(code: u8) -> Option<Protection> {
        match code {
            0 => Some(Protection::WhenUnlocked),
            1 => Some(Protection::AfterFirstUnlock),
            2 => Some(Protection::UserPresence),
            _ => None,
        }
    }
}

/// One stored item, without its secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The key the item was stored under.
    pub key: String,
    /// The level it was stored at.
    pub protection: Protection,
}

/// Why a call did not do what it was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeychainError {
    /// The key is empty, longer than [`MAX_KEY_LEN`] bytes, or contains a control character.
    InvalidKey,
    /// The reason text for a user-presence read is empty.
    InvalidReason,
    /// The secret is longer than [`MAX_SECRET_LEN`] bytes.
    TooLarge,
    /// No item has this key.
    NotFound,
    /// The item needs the person present and this was the synchronous [`get`]; use
    /// [`get_async`] or [`get_future`].
    NeedsPresence,
    /// The person dismissed the verification dialog.
    Cancelled,
    /// The person did not pass the verification dialog.
    AuthFailed,
    /// Biometrics are locked after too many attempts; the device credential clears it.
    Lockout,
    /// The item needs a device passcode, PIN or password and none is set.
    PasscodeNotSet,
    /// The item exists but cannot be read: its key was invalidated (the device credential was
    /// removed, a protected key's biometric enrolment changed), its file is damaged, or it was
    /// written by another user account. The item is left in place; [`remove`] clears it.
    Unreadable,
    /// The store is locked and cannot be unlocked now: a Linux keyring whose unlock dialog was
    /// dismissed, an Apple item read while the device is locked.
    Locked,
    /// The store is not reachable: no Secret Service on this Linux session, no logon session on
    /// Windows, no app context on Android.
    Unavailable,
    /// This platform has no store, or this [`Protection`] level has no realization here.
    Unsupported,
    /// A platform error outside the vocabulary above, with the platform's message.
    Platform(String),
}

impl fmt::Display for KeychainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeychainError::InvalidKey => {
                write!(f, "the key is empty, too long or has control characters")
            }
            KeychainError::InvalidReason => write!(f, "the reason text is empty"),
            KeychainError::TooLarge => {
                write!(f, "the secret is larger than {MAX_SECRET_LEN} bytes")
            }
            KeychainError::NotFound => write!(f, "no item with this key"),
            KeychainError::NeedsPresence => write!(
                f,
                "the item needs the person present; read it with get_async"
            ),
            KeychainError::Cancelled => write!(f, "the verification was dismissed"),
            KeychainError::AuthFailed => write!(f, "the verification did not pass"),
            KeychainError::Lockout => write!(f, "biometrics are locked after too many attempts"),
            KeychainError::PasscodeNotSet => {
                write!(f, "the device has no passcode, PIN or password")
            }
            KeychainError::Unreadable => write!(f, "the item exists but cannot be read"),
            KeychainError::Locked => write!(f, "the store is locked"),
            KeychainError::Unavailable => write!(f, "the store is not reachable"),
            KeychainError::Unsupported => write!(f, "not supported on this platform"),
            KeychainError::Platform(message) => write!(f, "platform error: {message}"),
        }
    }
}

impl std::error::Error for KeychainError {}

/// What this target's store promises: `Native` where the platform keeps the secret in its own
/// store, `Emulated` where the store is a stand-in with weaker guarantees (an unsigned macOS
/// development build's login keychain), `Unsupported` where there is none (the web, a Linux
/// session with no Secret Service).
pub fn support() -> Support {
    imp::support()
}

/// Whether one [`Protection`] level is realized here: `Native` where the OS binds the item to it,
/// `Emulated` where the crate gates the read without a binding (Windows user presence),
/// `Unsupported` where nothing can honor it (Linux user presence, every level on the web).
pub fn protection_support(protection: Protection) -> Support {
    imp::protection_support(protection)
}

/// The store this process writes to, as a short name for diagnostics and support notes:
/// `data-protection-keychain`, `login-keychain`, `android-keystore`, `dpapi`, `secret-service`,
/// `huks`, or `none`.
pub fn backend() -> &'static str {
    imp::backend()
}

/// Store `secret` under `key` at `protection`, replacing any item with that key (whatever its
/// level). Never shows a dialog.
pub fn set(key: &str, secret: &Secret, protection: Protection) -> Result<(), KeychainError> {
    check_key(key)?;
    if secret.len() > MAX_SECRET_LEN {
        return Err(KeychainError::TooLarge);
    }
    imp::set(key, secret.as_bytes(), protection)
}

/// The item under `key`, without a dialog. A [`Protection::UserPresence`] item answers
/// [`KeychainError::NeedsPresence`]; use [`get_async`] or [`get_future`] for it.
pub fn get(key: &str) -> Result<Secret, KeychainError> {
    check_key(key)?;
    imp::get(key).map(Secret)
}

/// The item under `key`, showing the OS's verification dialog with `reason` when the item needs
/// the person present, and answering exactly once, on the platform's thread. An item at another
/// level resolves without a dialog.
pub fn get_async(
    key: &str,
    reason: &str,
    on_done: impl FnOnce(Result<Secret, KeychainError>) + Send + 'static,
) {
    if let Err(e) = check_key(key) {
        on_done(Err(e));
        return;
    }
    if reason.trim().is_empty() {
        on_done(Err(KeychainError::InvalidReason));
        return;
    }
    imp::get_with_presence(
        key.to_string(),
        reason.to_string(),
        Box::new(move |outcome| on_done(outcome.map(Secret))),
    );
}

/// [`get_async`] as a future.
pub fn get_future(
    key: &str,
    reason: &str,
) -> impl std::future::Future<Output = Result<Secret, KeychainError>> + Send {
    let (tx, rx) = day_async::oneshot();
    get_async(key, reason, move |outcome| tx.send(outcome));
    async move { rx.await.unwrap_or(Err(KeychainError::Cancelled)) }
}

/// Remove the item under `key`. [`KeychainError::NotFound`] when there is none.
pub fn remove(key: &str) -> Result<(), KeychainError> {
    check_key(key)?;
    imp::remove(key)
}

/// Whether an item exists under `key`, without reading it and without a dialog.
pub fn contains(key: &str) -> Result<bool, KeychainError> {
    check_key(key)?;
    imp::contains(key)
}

/// Every item this app stored, with its level, without the secrets and without a dialog.
pub fn entries() -> Result<Vec<Entry>, KeychainError> {
    let mut all = imp::entries()?;
    all.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(all)
}

/// The key rules every platform shares: non-empty, at most [`MAX_KEY_LEN`] bytes, printable.
fn check_key(key: &str) -> Result<(), KeychainError> {
    if key.is_empty() || key.len() > MAX_KEY_LEN || key.chars().any(char::is_control) {
        return Err(KeychainError::InvalidKey);
    }
    Ok(())
}

/// The completion every per-OS implementation receives for a user-presence read.
type OnDone = Box<dyn FnOnce(Result<Vec<u8>, KeychainError>) + Send>;

/// The application identity items are scoped by: the `[app].id` from `Day.toml`, which `day
/// build`/`day launch` export as `DAY_APP_ID` (a dev launch sets it in the process environment; a
/// packaged build bakes it at compile time). A binary built outside the CLI shares one id. The
/// Android and HarmonyOS stores live in the app's own sandbox and need no id of their own.
#[cfg(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos"))
))]
pub(crate) fn app_id() -> String {
    std::env::var("DAY_APP_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| option_env!("DAY_APP_ID").map(str::to_owned))
        .unwrap_or_else(|| "dev.daybrite.day".to_string())
}

// ---------------------------------------------------------------------------
// Shared pieces of the file-backed stores (Windows, HarmonyOS, the Android arm's layout) and the
// Linux arm. Compiled only where used, plus under test.
// ---------------------------------------------------------------------------

#[cfg(any(all(target_os = "linux", not(target_env = "ohos")), test))]
mod base64;
#[cfg(any(target_os = "windows", target_env = "ohos", test))]
mod record;
#[cfg(any(target_os = "windows", target_env = "ohos", test))]
mod sha256;

// ---------------------------------------------------------------------------
// Per-OS implementations. Each exposes:
//   fn support() -> Support
//   fn protection_support(Protection) -> Support
//   fn backend() -> &'static str
//   fn set(key: &str, secret: &[u8], protection: Protection) -> Result<(), KeychainError>
//   fn get(key: &str) -> Result<Vec<u8>, KeychainError>          // NeedsPresence, never a dialog
//   fn get_with_presence(key: String, reason: String, on_done: OnDone)
//   fn remove(key: &str) -> Result<(), KeychainError>
//   fn contains(key: &str) -> Result<bool, KeychainError>
//   fn entries() -> Result<Vec<Entry>, KeychainError>
// ---------------------------------------------------------------------------

// The Android arm's declaration and Java (docs/bridge.md); `android.rs` reads its answers.
#[cfg(target_os = "android")]
mod bridge_glue;

#[cfg(any(target_os = "ios", target_os = "macos"))]
#[path = "apple.rs"]
mod imp;

#[cfg(target_os = "android")]
#[path = "android.rs"]
mod imp;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
#[path = "linux.rs"]
mod imp;

#[cfg(target_env = "ohos")]
#[path = "ohos.rs"]
mod imp;

// The web, and anything else: no store a page may rely on. A browser's storage is readable by
// any script on the origin and cleared with site data, which is not a keychain.
#[cfg(not(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "android",
    target_os = "windows",
    target_os = "linux",
    target_env = "ohos"
)))]
#[path = "unsupported.rs"]
mod imp;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_reads_back_and_hides_in_debug() {
        let s = Secret::from("hunter2");
        assert_eq!(s.as_bytes(), b"hunter2");
        assert_eq!(s.as_str(), Some("hunter2"));
        assert_eq!(s.len(), 7);
        assert!(!s.is_empty());
        assert_eq!(format!("{s:?}"), "Secret(7 bytes)");
        let bytes = Secret::new(vec![0xff, 0xfe]);
        assert_eq!(bytes.as_str(), None);
    }

    #[test]
    fn secret_equality_is_by_bytes() {
        assert_eq!(Secret::from("a"), Secret::from("a"));
        assert_ne!(Secret::from("a"), Secret::from("b"));
        assert_ne!(Secret::from("a"), Secret::from("ab"));
        assert_eq!(Secret::from(""), Secret::new(Vec::new()));
    }

    /// The wipe clears the bytes and the spare capacity: what `Drop` does, observed through the
    /// helper it calls.
    #[test]
    fn wipe_zeroes_length_and_capacity() {
        let mut v = Vec::with_capacity(8);
        v.extend_from_slice(b"abc");
        wipe(&mut v);
        assert_eq!(v, vec![0, 0, 0]);
        // SAFETY: the capacity was fully written by `wipe`, so reading it is reading initialized
        // zeroes; the test only observes, it does not change the length.
        let spare: Vec<u8> = v
            .spare_capacity_mut()
            .iter()
            .map(|s| unsafe { s.assume_init() })
            .collect();
        assert!(spare.iter().all(|b| *b == 0));
    }

    #[test]
    fn protection_codes_round_trip() {
        for p in [
            Protection::WhenUnlocked,
            Protection::AfterFirstUnlock,
            Protection::UserPresence,
        ] {
            assert_eq!(Protection::from_code(p.code()), Some(p));
        }
        assert_eq!(Protection::from_code(9), None);
        assert_eq!(Protection::default(), Protection::WhenUnlocked);
    }

    #[test]
    fn key_rules_are_enforced_before_the_platform() {
        assert_eq!(check_key(""), Err(KeychainError::InvalidKey));
        assert_eq!(check_key("a\nb"), Err(KeychainError::InvalidKey));
        assert_eq!(
            check_key(&"k".repeat(MAX_KEY_LEN + 1)),
            Err(KeychainError::InvalidKey)
        );
        assert_eq!(check_key(&"k".repeat(MAX_KEY_LEN)), Ok(()));
        assert_eq!(check_key("dev.daybrite.showcase/session"), Ok(()));
        assert_eq!(get("").err(), Some(KeychainError::InvalidKey));
        assert_eq!(remove("\t").err(), Some(KeychainError::InvalidKey));
        assert_eq!(contains("").err(), Some(KeychainError::InvalidKey));
        let big = Secret::new(vec![0u8; MAX_SECRET_LEN + 1]);
        assert_eq!(
            set("k", &big, Protection::WhenUnlocked),
            Err(KeychainError::TooLarge)
        );
    }

    /// The argument checks answer through the callback, at once, on the calling thread.
    #[test]
    fn async_argument_errors_come_through_the_callback() {
        let (tx, rx) = std::sync::mpsc::channel();
        get_async("", "Unlock", move |r| {
            let _ = tx.send(r);
        });
        assert_eq!(rx.recv().ok(), Some(Err(KeychainError::InvalidKey)));
        let (tx, rx) = std::sync::mpsc::channel();
        get_async("k", "  ", move |r| {
            let _ = tx.send(r);
        });
        assert_eq!(rx.recv().ok(), Some(Err(KeychainError::InvalidReason)));
    }

    /// The probes answer on every host, including headless CI, and agree: a target with no store
    /// supports no protection level, and names its backend `none`.
    #[test]
    fn probes_never_panic_and_agree() {
        let s = support();
        let _ = backend();
        for p in [
            Protection::WhenUnlocked,
            Protection::AfterFirstUnlock,
            Protection::UserPresence,
        ] {
            let ps = protection_support(p);
            if s == Support::Unsupported {
                assert_eq!(ps, Support::Unsupported);
            }
        }
        if s == Support::Unsupported {
            assert_eq!(backend(), "none");
        }
    }

    /// A round trip through the host's real store, where one answers. CI runners without a
    /// store (headless Linux has no Secret Service) report `Unavailable`, which is the honest
    /// answer and ends the test early; a host with a store must hold and give back the bytes.
    #[test]
    fn round_trip_where_the_host_has_a_store() {
        if support() == Support::Unsupported {
            return;
        }
        let key = format!("day-part-keychain.test.{}", std::process::id());
        let secret = Secret::from("the quick brown fox");
        match set(&key, &secret, Protection::WhenUnlocked) {
            Ok(()) => {}
            Err(KeychainError::Unavailable | KeychainError::Locked) => return,
            Err(e) => panic!("set: {e}"),
        }
        assert_eq!(get(&key).as_ref(), Ok(&secret));
        assert_eq!(contains(&key), Ok(true));
        assert!(entries().unwrap_or_default().iter().any(|e| e.key == key));
        let replaced = Secret::from("jumps over");
        set(&key, &replaced, Protection::WhenUnlocked).unwrap_or_else(|e| panic!("set again: {e}"));
        assert_eq!(get(&key).as_ref(), Ok(&replaced));
        assert_eq!(remove(&key), Ok(()));
        assert_eq!(remove(&key), Err(KeychainError::NotFound));
        assert_eq!(get(&key), Err(KeychainError::NotFound));
        assert_eq!(contains(&key), Ok(false));
    }

    #[test]
    fn errors_display_without_panicking() {
        let all = [
            KeychainError::InvalidKey,
            KeychainError::InvalidReason,
            KeychainError::TooLarge,
            KeychainError::NotFound,
            KeychainError::NeedsPresence,
            KeychainError::Cancelled,
            KeychainError::AuthFailed,
            KeychainError::Lockout,
            KeychainError::PasscodeNotSet,
            KeychainError::Unreadable,
            KeychainError::Locked,
            KeychainError::Unavailable,
            KeychainError::Unsupported,
            KeychainError::Platform("x".into()),
        ];
        for e in all {
            assert!(!e.to_string().is_empty());
        }
    }
}
