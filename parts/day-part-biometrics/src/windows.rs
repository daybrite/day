// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Windows: Windows Hello through `UserConsentVerifier`.
//!
//! `CheckAvailabilityAsync` answers [`availability`]; the dialog itself comes from the desktop
//! interop interface `IUserConsentVerifierInterop::RequestVerificationForWindowAsync`, which takes
//! the window handle the dialog is owned by. The WinRT `RequestVerificationAsync` method is for
//! packaged UWP apps and has no window to attach to in a desktop process, so it is not used.
//!
//! # The window handle
//!
//! The handle is captured on the calling thread, where `GetActiveWindow` knows which of this
//! thread's windows is active (a Day action runs on the UI thread, so that is the app's window).
//! A call from another thread falls back to the foreground window when it belongs to this
//! process. With no handle at all the prompt fails with [`BiometricError::NotInteractive`]; there
//! is nothing to attach a dialog to.
//!
//! # The worker thread
//!
//! The verification is awaited with `join()`, which blocks, so it runs on a worker thread with
//! its own multithreaded apartment, the idiom `day-part-sensors` uses; the UI thread keeps pumping
//! while the Hello window is up. The availability check runs first, because a request on a machine
//! with no Hello credential does not fail, it waits.
//!
//! Hello never says which credential passed (PIN, face, fingerprint), so a pass reports
//! [`Method::Unknown`] and [`biometry`] reports [`Biometry::Unknown`] when anything is set up.

use std::sync::Mutex;

use windows::Security::Credentials::UI::{
    UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::System::WinRT::{
    IUserConsentVerifierInterop, RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::core::{HSTRING, factory};
use windows_future::IAsyncOperation;

use crate::{Availability, BiometricError, Biometry, Method, OnDone, Prompt, Support, Verified};

/// A WinRT apartment for the worker thread; `RoUninitialize`d on drop.
struct Apartment(bool);

impl Apartment {
    fn new() -> Self {
        // SAFETY: `RoInitialize` has no preconditions; the matching uninitialize is in `Drop`.
        Self(unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful `RoInitialize` above, on the same thread.
            unsafe { RoUninitialize() };
        }
    }
}

/// The verification in flight, so [`cancel`] can reach it.
static ACTIVE: Mutex<Option<IAsyncOperation<UserConsentVerificationResult>>> = Mutex::new(None);

fn active() -> std::sync::MutexGuard<'static, Option<IAsyncOperation<UserConsentVerificationResult>>>
{
    match ACTIVE.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// A window handle that crosses to the worker thread.
#[derive(Clone, Copy)]
struct Window(isize);

/// The window the dialog is owned by: this thread's active window, else the foreground window
/// when it is ours. `None` when neither exists.
fn owner_window() -> Option<Window> {
    // SAFETY: both calls take no arguments and return a handle or null; `GetWindowThreadProcessId`
    // writes the owning process id into the out-parameter it is given.
    unsafe {
        let active = GetActiveWindow();
        if !active.is_invalid() {
            return Some(Window(active.0 as isize));
        }
        let foreground = GetForegroundWindow();
        if foreground.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(foreground, Some(&mut pid));
        (pid == GetCurrentProcessId()).then_some(Window(foreground.0 as isize))
    }
}

fn check_availability() -> Result<UserConsentVerifierAvailability, windows::core::Error> {
    UserConsentVerifier::CheckAvailabilityAsync()?.join()
}

/// `UserConsentVerifierAvailability`, in the crate's vocabulary.
fn availability_from(a: UserConsentVerifierAvailability) -> Availability {
    match a.0 {
        0 => Availability::Available,
        1 => Availability::NoHardware,
        2 => Availability::NotEnrolled,
        3 => Availability::DisabledByPolicy,
        4 => Availability::Busy,
        _ => Availability::Unsupported,
    }
}

/// The error a prompt fails with when the verifier is not available: the same facts as
/// [`availability_from`], as a reason not to show the dialog.
fn unavailable_error(a: UserConsentVerifierAvailability) -> BiometricError {
    match a.0 {
        1 => BiometricError::NoHardware,
        2 => BiometricError::NotEnrolled,
        3 => BiometricError::DisabledByPolicy,
        4 => BiometricError::Busy,
        code => BiometricError::Platform {
            code,
            message: "the consent verifier is not available".to_string(),
        },
    }
}

/// `UserConsentVerificationResult`, in the crate's vocabulary.
fn outcome_from(r: UserConsentVerificationResult) -> Result<Verified, BiometricError> {
    match r.0 {
        0 => Ok(Verified {
            method: Method::Unknown,
            token: Vec::new(),
        }),
        1 => Err(BiometricError::NoHardware),
        2 => Err(BiometricError::NotEnrolled),
        3 => Err(BiometricError::DisabledByPolicy),
        4 => Err(BiometricError::Busy),
        // RetriesExhausted: ten misses, and Hello refuses this request and every retry of it.
        5 => Err(BiometricError::LockoutPermanent),
        6 => Err(BiometricError::UserCancelled),
        code => Err(BiometricError::Platform {
            code,
            message: "unexpected consent verification result".to_string(),
        }),
    }
}

fn platform_error(e: windows::core::Error) -> BiometricError {
    BiometricError::Platform {
        code: e.code().0,
        message: e.message(),
    }
}

pub fn support() -> Support {
    Support::Native
}

pub fn availability() -> Availability {
    let _apartment = Apartment::new();
    match check_availability() {
        Ok(a) => availability_from(a),
        Err(_) => Availability::Unsupported,
    }
}

pub fn biometry() -> Biometry {
    // Hello reports that a credential exists, never which kind.
    match availability() {
        Availability::Available => Biometry::Unknown,
        _ => Biometry::None,
    }
}

pub fn authenticate(prompt: Prompt, on_done: OnDone) {
    let Some(window) = owner_window() else {
        on_done(Err(BiometricError::NotInteractive));
        return;
    };
    let message = HSTRING::from(prompt.reason());
    let spawned = std::thread::Builder::new()
        .name("day-biometrics".into())
        .spawn(move || {
            let _apartment = Apartment::new();
            on_done(verify(window, &message));
        });
    if let Err(e) = spawned {
        // The thread never started, so the completion above was never moved out of it; the
        // closure (and the callback inside it) was dropped with the error, and nothing can be
        // answered any more. Record the failure where a developer will see it.
        eprintln!("day-part-biometrics: could not start the prompt thread: {e}");
    }
}

/// The whole verification, on the worker thread: availability first, then the owned dialog.
fn verify(window: Window, message: &HSTRING) -> Result<Verified, BiometricError> {
    let availability = check_availability().map_err(platform_error)?;
    if availability.0 != 0 {
        return Err(unavailable_error(availability));
    }
    let interop: IUserConsentVerifierInterop =
        factory::<UserConsentVerifier, IUserConsentVerifierInterop>().map_err(platform_error)?;
    // SAFETY: `window` is a live top-level window of this process, captured on the thread that
    // owns it; the HSTRING outlives the call (the OS copies the message).
    let op: IAsyncOperation<UserConsentVerificationResult> = unsafe {
        interop.RequestVerificationForWindowAsync(HWND(window.0 as *mut core::ffi::c_void), message)
    }
    .map_err(platform_error)?;
    *active() = Some(op.clone());
    let result = op.join();
    *active() = None;
    match result {
        Ok(r) => outcome_from(r),
        // A cancelled operation reports E_ABORT (0x80004004) from `join`; that is `cancel` at work.
        Err(e) if e.code().0 == 0x8000_4004_u32 as i32 => Err(BiometricError::SystemCancelled),
        Err(e) => Err(platform_error(e)),
    }
}

pub fn cancel() {
    if let Some(op) = active().as_ref() {
        let _ = op.Cancel();
    }
}
