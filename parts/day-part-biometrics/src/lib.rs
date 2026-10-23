// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-part-biometrics: the platform's own "prove it is you" prompt, as one Rust API. No UI of its
//! own; any Rust code can depend on this crate, ask what the device can do, and put the system's
//! authentication dialog in front of the person.
//!
//! ```no_run
//! use day_part_biometrics::{Prompt, availability, Availability};
//!
//! if availability() == Availability::Available {
//!     day_part_biometrics::authenticate(Prompt::new("Unlock your notes"), |outcome| match outcome {
//!         Ok(verified) => println!("verified by {:?}", verified.method),
//!         Err(e) => println!("not verified: {e}"),
//!     });
//! }
//! ```
//!
//! Platform selection is `#[cfg(target_os)]`: iOS and macOS use LocalAuthentication, Android the
//! platform `BiometricPrompt` (API 28 and later), Windows the Windows Hello consent verifier, and
//! HarmonyOS the User Authentication Kit's dialog. Desktop Linux and the web have no API an
//! application may use for this, so [`support`] reports `Unsupported` there and every prompt
//! fails with [`BiometricError::Unsupported`].
//!
//! # What a successful prompt proves
//!
//! A `Verified` answer means the OS showed its dialog and the person passed it: a biometric match,
//! or the device credential (passcode, PIN, pattern, account password) when the prompt allows one.
//! The answer is a fact about the moment, not a key: nothing is unlocked by it except what the app
//! chooses to do next. Where the platform can bind a stored secret to the same check, that binding
//! is `day-part-keychain`'s `Protection::UserPresence`, which uses this crate's prompt on the
//! platforms that need one (Windows Hello and HarmonyOS; Apple and Android bind inside their
//! keychain and keystore).
//!
//! # One prompt at a time
//!
//! Every platform shows one authentication dialog at a time. A second [`authenticate`] while one
//! is on screen fails at once with [`BiometricError::Busy`] rather than queuing behind the first.
//! [`cancel`] dismisses the prompt on screen; its caller then hears
//! [`BiometricError::SystemCancelled`].
//!
//! # Threads
//!
//! Completions run where the platform delivers them (a LocalAuthentication reply queue, an
//! Android executor, a WinRT thread-pool thread, the HarmonyOS JS thread), which is why the
//! callback is `Send`. [`authenticate_future`] is the door back to the UI thread: awaited under
//! `day::task`, the code after the `.await` writes signals directly. A callback user captures a
//! `Setter` instead. Nothing here posts to a main loop, so the crate works in a plain `main` and
//! under `cargo test` (docs/async.md rule 3).

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

pub use day_bridge::Support;

/// The kind of biometric sensor the device offers for app prompts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Biometry {
    /// No biometric sensor: a prompt can still succeed with the device credential.
    None,
    /// A fingerprint reader (Touch ID, Android fingerprint, a Windows Hello fingerprint reader).
    Fingerprint,
    /// Face recognition (Face ID, Android face unlock, Windows Hello face).
    Face,
    /// An iris scanner (Android).
    Iris,
    /// A sensor the platform has but does not describe to apps, or several kinds at once.
    /// Windows Hello never says which credential a person set up; Apple's Optic ID reports here.
    Unknown,
}

/// Whether a prompt can succeed right now, and if not, why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// A biometric is enrolled and ready; a prompt will ask for it.
    Available,
    /// The device has a sensor but the person has not enrolled anything. A prompt that allows the
    /// device credential can still succeed.
    NotEnrolled,
    /// The device has no biometric sensor. A prompt that allows the device credential can still
    /// succeed where the platform offers one (Apple and Android do, HarmonyOS does with a PIN
    /// enrolled, Windows Hello needs a PIN set up).
    NoHardware,
    /// No passcode, PIN or password is set on the device, so nothing can be verified at all.
    PasscodeNotSet,
    /// Too many failed attempts: the biometric is locked until the device credential is entered.
    Lockout,
    /// A device policy (Windows Group Policy, an Android device admin) disables biometrics.
    DisabledByPolicy,
    /// The sensor is in use by another request.
    Busy,
    /// This platform has no API an app can use (desktop Linux, the web, Android below API 28).
    Unsupported,
}

/// What a person is asked to do, and how the dialog may let them do it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    reason: String,
    fallback_title: Option<String>,
    device_credential: bool,
    challenge: Vec<u8>,
}

impl Prompt {
    /// A prompt that explains, in the person's language, why the app asks. The text is shown in
    /// the system dialog: as the reason under the Face ID glyph on Apple platforms, the title of
    /// the Android sheet, the message of the Windows Hello window, the title of the HarmonyOS
    /// dialog. An empty reason is rejected with [`BiometricError::InvalidPrompt`], because Apple
    /// aborts on one and the others show a blank dialog. Localize it with `res::str`.
    pub fn new(reason: impl Into<String>) -> Self {
        Prompt {
            reason: reason.into(),
            fallback_title: None,
            device_credential: true,
            challenge: Vec::new(),
        }
    }

    /// The label of the button that leads away from the biometric. With the device credential
    /// allowed it is the "use passcode" button (Apple), with it refused it is the negative button
    /// that dismisses the dialog (Android), and the platforms with no such button ignore it.
    pub fn fallback_title(mut self, title: impl Into<String>) -> Self {
        self.fallback_title = Some(title.into());
        self
    }

    /// Whether the dialog offers the device credential (passcode, PIN, pattern, password) as a way
    /// to pass. `true` by default, and what most apps want: a person whose finger is wet can still
    /// get in. `false` asks for a biometric only; a device with none enrolled then fails with
    /// [`BiometricError::NotEnrolled`]. Windows Hello decides this itself and ignores the flag.
    pub fn device_credential(mut self, allowed: bool) -> Self {
        self.device_credential = allowed;
        self
    }

    /// Bytes the platform binds the result to. HarmonyOS issues an authentication token for the
    /// challenge it is given (32 bytes at most) and a key bound to the same challenge accepts that
    /// token once; `day-part-keychain` passes the challenge its keystore produced. Every other
    /// platform ignores the challenge and returns an empty token.
    pub fn challenge(mut self, challenge: impl Into<Vec<u8>>) -> Self {
        self.challenge = challenge.into();
        self
    }

    /// The reason shown in the dialog.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The fallback button's label, when one was given.
    pub fn fallback_title_text(&self) -> Option<&str> {
        self.fallback_title.as_deref()
    }

    /// Whether the device credential may pass the prompt.
    pub fn allows_device_credential(&self) -> bool {
        self.device_credential
    }

    /// The challenge bytes, empty unless [`Prompt::challenge`] set them.
    pub fn challenge_bytes(&self) -> &[u8] {
        &self.challenge
    }
}

/// How the person passed the prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// A biometric match.
    Biometric,
    /// The device passcode, PIN, pattern or account password.
    DeviceCredential,
    /// The platform did not say (macOS and Windows report a pass without the method).
    Unknown,
}

/// A passed prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    /// How the person passed.
    pub method: Method,
    /// The platform's authentication token, where one exists: HarmonyOS returns the User
    /// Authentication Kit's token (up to 1024 bytes), bound to the prompt's challenge and
    /// accepted once by a key generated with the same binding. Empty on every other platform.
    pub token: Vec<u8>,
}

/// Why a prompt did not pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BiometricError {
    /// This platform has no API an app can use, or the OS is too old for it.
    Unsupported,
    /// The device has no biometric sensor and the prompt refused the device credential.
    NoHardware,
    /// Nothing is enrolled and the prompt refused the device credential.
    NotEnrolled,
    /// No passcode, PIN or password is set on the device, so nothing can be verified.
    PasscodeNotSet,
    /// Too many failed biometric attempts; the device credential clears the lockout.
    Lockout,
    /// Biometrics are locked until the device is unlocked with its credential (Android's
    /// permanent lockout, Windows Hello's exhausted retries).
    LockoutPermanent,
    /// The person dismissed the dialog.
    UserCancelled,
    /// The OS or the app dismissed the dialog: the app went to the background, [`cancel`] was
    /// called, another prompt took over.
    SystemCancelled,
    /// The person chose the fallback button of a prompt with no device credential to fall back
    /// to. The app shows its own alternative, if it has one.
    UserFallback,
    /// The attempt did not match and the dialog closed (Apple closes after several misses; Android
    /// reports each miss to the dialog and this only when the dialog ends on one).
    Failed,
    /// Another prompt is on screen, from this app or from another.
    Busy,
    /// A device policy disables biometrics.
    DisabledByPolicy,
    /// The prompt was asked for where no dialog can be shown: no window, no foreground app.
    NotInteractive,
    /// The [`Prompt`] was unusable: an empty reason, or a challenge longer than the platform takes.
    InvalidPrompt,
    /// A platform error outside the vocabulary above, with the platform's code and message.
    Platform { code: i32, message: String },
}

impl fmt::Display for BiometricError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BiometricError::Unsupported => write!(
                f,
                "biometric authentication is not available on this platform"
            ),
            BiometricError::NoHardware => write!(f, "this device has no biometric sensor"),
            BiometricError::NotEnrolled => write!(f, "no biometric is enrolled on this device"),
            BiometricError::PasscodeNotSet => {
                write!(f, "the device has no passcode, PIN or password")
            }
            BiometricError::Lockout => write!(f, "biometrics are locked after too many attempts"),
            BiometricError::LockoutPermanent => write!(
                f,
                "biometrics are locked until the device is unlocked with its credential"
            ),
            BiometricError::UserCancelled => write!(f, "the prompt was dismissed"),
            BiometricError::SystemCancelled => write!(f, "the prompt was cancelled"),
            BiometricError::UserFallback => write!(f, "the fallback button was chosen"),
            BiometricError::Failed => write!(f, "authentication failed"),
            BiometricError::Busy => write!(f, "another authentication prompt is on screen"),
            BiometricError::DisabledByPolicy => write!(f, "biometrics are disabled by policy"),
            BiometricError::NotInteractive => write!(f, "no window to show the prompt from"),
            BiometricError::InvalidPrompt => write!(f, "the prompt has no reason text"),
            BiometricError::Platform { code, message } => {
                if message.is_empty() {
                    write!(f, "platform error {code}")
                } else {
                    write!(f, "platform error {code}: {message}")
                }
            }
        }
    }
}

impl std::error::Error for BiometricError {}

/// What this target's implementation promises: `Native` where the platform has a prompt an app
/// may show, `Unsupported` where it has none (desktop Linux, the web). This is API presence, not a
/// live hardware probe; [`availability`] is the probe.
pub fn support() -> Support {
    imp::support()
}

/// Whether a prompt can succeed right now, asked of the platform.
pub fn availability() -> Availability {
    imp::availability()
}

/// The kind of sensor the device offers. [`Biometry::None`] where there is none or the platform
/// reports `Unsupported`.
pub fn biometry() -> Biometry {
    imp::biometry()
}

/// Show the system's authentication dialog and report how it ended, exactly once, on the
/// platform's thread. Fails at once, through the same callback, when the prompt is unusable, a
/// prompt is already on screen, or this platform has none.
pub fn authenticate(
    prompt: Prompt,
    on_done: impl FnOnce(Result<Verified, BiometricError>) + Send + 'static,
) {
    if prompt.reason.trim().is_empty() {
        on_done(Err(BiometricError::InvalidPrompt));
        return;
    }
    if imp::support() == Support::Unsupported {
        on_done(Err(BiometricError::Unsupported));
        return;
    }
    if IN_FLIGHT.swap(true, Ordering::AcqRel) {
        on_done(Err(BiometricError::Busy));
        return;
    }
    imp::authenticate(
        prompt,
        Box::new(move |outcome| {
            IN_FLIGHT.store(false, Ordering::Release);
            on_done(outcome);
        }),
    );
}

/// [`authenticate`] as a future. Dropping it before the answer arrives keeps the dialog on screen
/// (the person still has to settle it) but discards the answer; call [`cancel`] to dismiss it.
pub fn authenticate_future(
    prompt: Prompt,
) -> impl std::future::Future<Output = Result<Verified, BiometricError>> + Send {
    let (tx, rx) = day_async::oneshot();
    authenticate(prompt, move |outcome| tx.send(outcome));
    async move { rx.await.unwrap_or(Err(BiometricError::SystemCancelled)) }
}

/// Dismiss the prompt on screen, if any. Its caller hears [`BiometricError::SystemCancelled`].
pub fn cancel() {
    if IN_FLIGHT.load(Ordering::Acquire) {
        imp::cancel();
    }
}

/// Whether a prompt from this crate is on screen.
pub fn is_prompting() -> bool {
    IN_FLIGHT.load(Ordering::Acquire)
}

/// The one-prompt guard. Set by [`authenticate`], cleared when its completion runs.
static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// The completion every per-OS implementation receives.
type OnDone = Box<dyn FnOnce(Result<Verified, BiometricError>) + Send>;

// ---------------------------------------------------------------------------
// Per-OS implementations. Each exposes:
//   fn support() -> Support
//   fn availability() -> Availability
//   fn biometry() -> Biometry
//   fn authenticate(prompt: Prompt, on_done: OnDone)   // answers exactly once
//   fn cancel()
// ---------------------------------------------------------------------------

// The one declaration the Java and ArkTS arms implement; the two files below read its answers.
#[cfg(any(target_os = "android", target_env = "ohos"))]
mod bridge;

#[cfg(any(target_os = "ios", target_os = "macos"))]
#[path = "apple.rs"]
mod imp;

#[cfg(target_os = "android")]
#[path = "android.rs"]
mod imp;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;

#[cfg(target_env = "ohos")]
#[path = "ohos.rs"]
mod imp;

// Desktop Linux (GTK and Qt alike) and the web: no API an application may use. fprintd is claimed
// by the login stack and answers a bare yes/no with nothing bound to it; polkit authenticates on
// the system's behalf and never tells the app how. The browser's WebAuthn is a credential
// protocol with a server, not a local prompt.
#[cfg(not(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "android",
    target_os = "windows",
    target_env = "ohos"
)))]
#[path = "unsupported.rs"]
mod imp;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_builder_keeps_what_it_is_given() {
        let p = Prompt::new("Unlock")
            .fallback_title("Use passcode")
            .device_credential(false)
            .challenge(vec![1, 2, 3]);
        assert_eq!(p.reason(), "Unlock");
        assert_eq!(p.fallback_title_text(), Some("Use passcode"));
        assert!(!p.allows_device_credential());
        assert_eq!(p.challenge_bytes(), &[1, 2, 3]);
        let d = Prompt::new("x");
        assert!(d.allows_device_credential());
        assert!(d.fallback_title_text().is_none());
        assert!(d.challenge_bytes().is_empty());
    }

    /// An empty reason never reaches the platform: Apple aborts on one, the others show a blank
    /// dialog. The answer comes through the callback, on the calling thread, at once.
    #[test]
    fn empty_reason_is_rejected_before_any_platform_call() {
        let (tx, rx) = std::sync::mpsc::channel();
        authenticate(Prompt::new("   "), move |r| {
            let _ = tx.send(r);
        });
        assert_eq!(rx.recv().ok(), Some(Err(BiometricError::InvalidPrompt)));
        assert!(!is_prompting());
    }

    /// The probes answer on every host, including headless CI, and agree with each other: a
    /// target that reports no API has no availability and no sensor.
    #[test]
    fn probes_never_panic_and_agree() {
        let s = support();
        let a = availability();
        let b = biometry();
        if s == Support::Unsupported {
            assert_eq!(a, Availability::Unsupported);
            assert_eq!(b, Biometry::None);
        }
    }

    /// A target with no API answers `Unsupported` through the callback and leaves no prompt
    /// recorded as in flight; `cancel` on nothing is a no-op.
    #[test]
    fn unsupported_target_fails_cleanly() {
        if support() != Support::Unsupported {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        authenticate(Prompt::new("Unlock"), move |r| {
            let _ = tx.send(r);
        });
        assert_eq!(rx.recv().ok(), Some(Err(BiometricError::Unsupported)));
        assert!(!is_prompting());
        cancel();
    }

    #[test]
    fn errors_display_without_panicking() {
        let all = [
            BiometricError::Unsupported,
            BiometricError::NoHardware,
            BiometricError::NotEnrolled,
            BiometricError::PasscodeNotSet,
            BiometricError::Lockout,
            BiometricError::LockoutPermanent,
            BiometricError::UserCancelled,
            BiometricError::SystemCancelled,
            BiometricError::UserFallback,
            BiometricError::Failed,
            BiometricError::Busy,
            BiometricError::DisabledByPolicy,
            BiometricError::NotInteractive,
            BiometricError::InvalidPrompt,
            BiometricError::Platform {
                code: 7,
                message: String::new(),
            },
            BiometricError::Platform {
                code: 7,
                message: "x".into(),
            },
        ];
        for e in all {
            assert!(!e.to_string().is_empty());
        }
    }
}
