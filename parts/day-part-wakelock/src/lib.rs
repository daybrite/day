// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-part-wakelock — HEADLESS: keep the screen on while something is showing.
//!
//! ```no_run
//! let lock = day_part_wakelock::keep_screen_on();
//! // The screen stays on, whatever the device's auto-lock setting, until the lock is dropped.
//! drop(lock);
//! ```
//!
//! Locks nest: the screen may sleep again once the last one is dropped. In a Day app, tie a lock to
//! the page's scope so a page that closes releases it:
//!
//! ```ignore
//! let lock = day_part_wakelock::keep_screen_on();
//! day_reactive::Scope::current().on_cleanup(move || drop(lock));
//! ```
//!
//! | Platform | Mechanism |
//! |---|---|
//! | iOS | `UIApplication.isIdleTimerDisabled` |
//! | macOS | an IOKit power assertion, `PreventUserIdleDisplaySleep` |
//! | Windows | `SetThreadExecutionState(ES_CONTINUOUS \| ES_DISPLAY_REQUIRED)` |
//! | Android | `FLAG_KEEP_SCREEN_ON` on the activity's window |
//! | HarmonyOS | `Window.setWindowKeepScreenOn` |
//! | web | the Screen Wake Lock API, taken again each time the page becomes visible |
//! | Linux | none yet: [`is_supported`] is false and a lock does nothing |
//!
//! Take and drop locks on the UI thread, where a Day app's code runs: UIKit and Windows tie the
//! setting to it, so [`ScreenLock`] is not `Send`. Nothing here returns an error or panics; a
//! platform that cannot keep the screen on simply lets it sleep.

use std::marker::PhantomData;
use std::sync::{Mutex, MutexGuard};

/// How many [`ScreenLock`]s are alive. One process-wide lock rather than a thread-local: each
/// `thread_local!` costs Android a pthread key, and bionic has only 128 for the whole process.
static HELD: Mutex<u32> = Mutex::new(0);

fn held() -> MutexGuard<'static, u32> {
    // A counter cannot be left half-written, so a poisoned lock is still usable.
    HELD.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether this platform can keep the screen on.
pub fn is_supported() -> bool {
    imp::supported()
}

/// Whether a [`ScreenLock`] is alive anywhere in the app.
pub fn is_held() -> bool {
    *held() > 0
}

/// Keep the screen on until the returned lock is dropped.
#[must_use = "the screen may sleep again as soon as the lock is dropped"]
pub fn keep_screen_on() -> ScreenLock {
    let mut n = held();
    *n += 1;
    if *n == 1 {
        imp::hold(true);
    }
    ScreenLock {
        _ui_thread: PhantomData,
    }
}

/// The screen stays on while this lives ([`keep_screen_on`]).
pub struct ScreenLock {
    /// Not `Send`: the platform setting is released on the thread that took it.
    _ui_thread: PhantomData<*const ()>,
}

impl Drop for ScreenLock {
    fn drop(&mut self) {
        let mut n = held();
        *n = n.saturating_sub(1);
        if *n == 0 {
            imp::hold(false);
        }
    }
}

#[cfg(target_os = "ios")]
mod imp {
    use objc2::MainThreadMarker;
    use objc2_ui_kit::UIApplication;

    pub fn supported() -> bool {
        true
    }

    pub fn hold(on: bool) {
        // UIApplication is main-thread only. Day runs app code there; a call from anywhere else is
        // skipped rather than risking UIKit off its thread.
        if let Some(mtm) = MainThreadMarker::new() {
            UIApplication::sharedApplication(mtm).setIdleTimerDisabled(on);
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::{c_char, c_void};
    use std::sync::Mutex;

    type CFStringRef = *const c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            text: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(value: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(
            kind: CFStringRef,
            level: u32,
            name: CFStringRef,
            id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }

    const UTF8: u32 = 0x0800_0100;
    /// `kIOPMAssertionLevelOn`.
    const LEVEL_ON: u32 = 255;

    /// The assertion while one is held; `pmset -g assertions` lists it under the app's process.
    static ASSERTION: Mutex<Option<u32>> = Mutex::new(None);

    pub fn supported() -> bool {
        true
    }

    pub fn hold(on: bool) {
        let mut held = ASSERTION.lock().unwrap_or_else(|e| e.into_inner());
        if on && held.is_none() {
            // SAFETY: both strings are NUL-terminated literals; each CFString made here is released
            // here, and IOKit copies what it keeps. `id` is written only on success.
            unsafe {
                let kind = CFStringCreateWithCString(
                    std::ptr::null(),
                    c"PreventUserIdleDisplaySleep".as_ptr(),
                    UTF8,
                );
                let name = CFStringCreateWithCString(
                    std::ptr::null(),
                    c"Keeping the screen on while the app shows something".as_ptr(),
                    UTF8,
                );
                let mut id = 0u32;
                if !kind.is_null()
                    && !name.is_null()
                    && IOPMAssertionCreateWithName(kind, LEVEL_ON, name, &mut id) == 0
                {
                    *held = Some(id);
                }
                for s in [kind, name] {
                    if !s.is_null() {
                        CFRelease(s);
                    }
                }
            }
        } else if !on && let Some(id) = held.take() {
            // SAFETY: `id` came from a successful IOPMAssertionCreateWithName and is released once.
            unsafe {
                IOPMAssertionRelease(id);
            }
        }
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::System::Power::{
        ES_CONTINUOUS, ES_DISPLAY_REQUIRED, SetThreadExecutionState,
    };

    pub fn supported() -> bool {
        true
    }

    pub fn hold(on: bool) {
        let state = if on {
            ES_CONTINUOUS | ES_DISPLAY_REQUIRED
        } else {
            ES_CONTINUOUS
        };
        // SAFETY: a plain Win32 call with documented flags. The state belongs to the calling
        // thread, which is why ScreenLock is not Send.
        unsafe {
            SetThreadExecutionState(state);
        }
    }
}

#[cfg(any(
    target_os = "android",
    all(target_os = "linux", target_env = "ohos"),
    target_arch = "wasm32"
))]
mod imp {
    use day_bridge::Support;

    pub fn supported() -> bool {
        super::ready_native_support() != Support::Unsupported
            && super::ready_native().unwrap_or(false)
    }

    pub fn hold(on: bool) {
        super::hold_native(on);
    }
}

#[cfg(not(any(
    target_os = "ios",
    target_os = "macos",
    windows,
    target_os = "android",
    all(target_os = "linux", target_env = "ohos"),
    target_arch = "wasm32"
)))]
mod imp {
    pub fn supported() -> bool {
        false
    }

    pub fn hold(_on: bool) {}
}

day_bridge::bridge! {
    // The contract the foreign arms implement.
    #[day_bridge::declare]
    extern "day" {
        /// Keep the screen on, or let it sleep again.
        fn hold_native(on: bool);
        /// Whether this platform can keep the screen on.
        fn ready_native() -> Result<bool, day_bridge::Error>;
    }

    // Android: a window flag, which holds only while that window is showing, so leaving the app
    // lets the screen sleep with no bookkeeping here. `DayBridge.ctx` is the running activity.
    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import android.app.Activity;
            import android.content.Context;
            import android.view.WindowManager;
            import dev.daybrite.day.bridge.DayBridge;
        "#,
        body = r#"
            public static void hold_native(boolean on) {
                Context ctx = DayBridge.ctx;
                if (!(ctx instanceof Activity)) {
                    return;
                }
                Activity activity = (Activity) ctx;
                // Window flags belong to the UI thread.
                DayBridge.main.post(() -> {
                    if (on) {
                        activity.getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                    } else {
                        activity.getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                    }
                });
            }

            public static boolean ready_native() {
                return DayBridge.ctx instanceof Activity;
            }
        "#,
    );

    // HarmonyOS: the window's keep-screen-on switch, like Android's flag.
    #[day_bridge::impl(arkts, platforms = [ohos])]
    arkts!(
        prelude = r#"
            import { window } from '@kit.ArkUI';
            import { common } from '@kit.AbilityKit';
        "#,
        body = r#"
            export function hold_native(on: boolean): void {
                const ctx = getContext() as common.UIAbilityContext;
                window.getLastWindow(ctx).then((win: window.Window) => {
                    return win.setWindowKeepScreenOn(on);
                }).catch((e: Error) => {
                    console.warn(`day-part-wakelock: ${e.message}`);
                });
            }

            export function ready_native(): boolean {
                return true;
            }
        "#,
    );

    // The web: the Screen Wake Lock API. The browser releases the lock whenever the page is hidden,
    // so it is taken again each time the page comes back while the app still wants it.
    #[day_bridge::impl(js, platforms = [web])]
    js!(r#"
        let dayLock = null;
        let dayWanted = false;
        let dayListening = false;

        async function dayTake() {
            if (!dayWanted || dayLock !== null || document.visibilityState !== "visible") {
                return;
            }
            try {
                const lock = await navigator.wakeLock.request("screen");
                if (!dayWanted) {
                    lock.release();
                    return;
                }
                dayLock = lock;
                lock.addEventListener("release", () => {
                    if (dayLock === lock) {
                        dayLock = null;
                    }
                });
            } catch (e) {
                console.warn(`day-part-wakelock: ${e}`);
            }
        }

        export function hold_native(on) {
            if (!("wakeLock" in navigator)) {
                return;
            }
            dayWanted = Boolean(on);
            if (!dayListening) {
                dayListening = true;
                document.addEventListener("visibilitychange", () => {
                    dayTake();
                });
            }
            if (dayWanted) {
                dayTake();
            } else if (dayLock !== null) {
                dayLock.release();
                dayLock = null;
            }
        }

        export function ready_native() {
            return "wakeLock" in navigator;
        }
    "#);

    // Everywhere else, and the arm `cargo test` and day-mock compile against: the screen sleeps as
    // usual.
    #[day_bridge::impl(rust, platforms = [other])]
    fn hold_native(_on: bool) {}

    #[day_bridge::impl(rust, platforms = [other])]
    fn ready_native() -> Result<bool, day_bridge::Error> {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_nest() {
        assert!(!is_held());
        let first = keep_screen_on();
        let second = keep_screen_on();
        assert!(is_held());
        drop(first);
        assert!(is_held(), "the second lock still holds the screen");
        drop(second);
        assert!(!is_held());
    }
}
