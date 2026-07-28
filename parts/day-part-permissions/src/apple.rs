//! iOS and macOS: the per-framework authorization APIs.
//!
//! Apple has no permission *system* to query — each capability is gated by its own framework, with
//! its own status enum and its own request call. The four we share here are `CLLocationManager`,
//! `AVCaptureDevice`, `PHPhotoLibrary` and `UNUserNotificationCenter`; motion and the settings deep
//! link differ per OS and live in `ios.rs` / `macos.rs`.
//!
//! # Why raw `objc2`, with no framework wrapper crates
//!
//! Everything below is a handful of class methods with scalar or block arguments, so this file uses
//! `AnyClass::get` + `msg_send!` rather than pulling `objc2-core-location`, `objc2-av-foundation`,
//! `objc2-photos` and `objc2-user-notifications` into the graph. `day-part-network` sets the
//! precedent (raw `SystemConfiguration` FFI, zero wrapper crates). The frameworks still have to be
//! LINKED: cargo does that on macOS through the `#[link]` blocks below, and on iOS through
//! `[package.metadata.day.ios].frameworks`, because xcodebuild ignores Rust link metadata.
//!
//! # Two hazards this file is built around
//!
//! 1. **An unbundled process must not touch `UNUserNotificationCenter`** — `currentNotificationCenter`
//!    aborts with "bundleProxyForCurrentProcess is nil". `cargo test` and `examples/` are both
//!    unbundled, so every notification path is behind [`is_bundled`].
//! 2. **Requesting without the matching `Info.plist` usage description terminates the process**
//!    (TCC kills you; it is not an exception you can catch). Nothing here can prevent that — the
//!    `[permissions]` declaration in Day.toml is what generates the key, and `day lint` is the
//!    backstop.
//!
//! # Location is polled, not delegated
//!
//! `CLLocationManager` reports authorization changes to a delegate, which only fires with a live run
//! loop. A part must work in a plain `main` and under `cargo test` (docs/async.md rule 3), so the
//! request path here polls `authorizationStatus` on a background thread instead of installing a
//! delegate — which also keeps this file free of `define_class!` and its unsafe.

use std::ffi::CString;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{msg_send, sel};

use super::{Gate, Permission, Status, from_apple_status};

#[cfg(target_os = "ios")]
#[path = "ios.rs"]
mod os;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod os;

// Link the frameworks whose classes this file looks up. Empty extern blocks are enough: the
// attribute emits the `-framework` flag, and every call goes through the Objective-C runtime.
#[link(name = "CoreLocation", kind = "framework")]
unsafe extern "C" {}
#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {}
#[link(name = "Photos", kind = "framework")]
unsafe extern "C" {}
#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}

/// `AVMediaTypeVideo` / `AVMediaTypeAudio` — the four-character media-type codes, spelled out so
/// this file needs no framework constant (and no crate to import it from).
const MEDIA_VIDEO: &str = "vide";
const MEDIA_AUDIO: &str = "soun";

/// Look a class up without panicking: a framework that failed to link leaves us reporting
/// `Unsupported` rather than aborting the app.
fn class(name: &str) -> Option<&'static AnyClass> {
    let c = CString::new(name).ok()?;
    AnyClass::get(&c)
}

/// An autoreleased `NSString` from a Rust `&str`.
fn nsstring(s: &str) -> Option<Retained<AnyObject>> {
    let cls = class("NSString")?;
    let c = CString::new(s).ok()?;
    // SAFETY: `stringWithUTF8String:` takes a NUL-terminated UTF-8 pointer, which `c` is, and
    // returns an autoreleased NSString that `Retained` retains.
    unsafe { msg_send![cls, stringWithUTF8String: c.as_ptr()] }
}

/// Wrap a one-shot completion so it can live in an Objective-C block.
///
/// `RcBlock` requires `Fn` (a block may be invoked any number of times), while our callbacks are
/// `FnOnce`. The mutex holds the callback until the first invocation and drops it after, so a
/// framework that calls back twice is harmless instead of a double-move.
fn once(cb: Box<dyn FnOnce(Status) + Send>) -> impl Fn(Status) + 'static {
    let cell = std::sync::Mutex::new(Some(cb));
    move |s| {
        let taken = match cell.lock() {
            Ok(mut g) => g.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        if let Some(cb) = taken {
            cb(s);
        }
    }
}

/// Whether this process has a bundle identifier. TCC and `UNUserNotificationCenter` both require
/// one; a bare `cargo run`/`cargo test` binary has none, and touching notifications there aborts.
fn is_bundled() -> bool {
    let Some(cls) = class("NSBundle") else {
        return false;
    };
    // SAFETY: `+mainBundle` is always safe; `-bundleIdentifier` returns nil for an unbundled
    // process, which is exactly the case being detected.
    unsafe {
        let bundle: *mut AnyObject = msg_send![cls, mainBundle];
        if bundle.is_null() {
            return false;
        }
        let ident: *mut AnyObject = msg_send![bundle, bundleIdentifier];
        !ident.is_null()
    }
}

/// A fresh `CLLocationManager`. Creating one does not prompt; only `request…Authorization` does.
fn location_manager() -> Option<Retained<AnyObject>> {
    let cls = class("CLLocationManager")?;
    // SAFETY: `+new` returns a +1 reference, which `Retained` owns.
    unsafe { msg_send![cls, new] }
}

/// The raw `CLAuthorizationStatus`, or `None` if CoreLocation isn't linked.
///
/// Read through the CLASS method: it is thread-safe and allocation-free, which matters because the
/// request path polls it from a background thread, and `CLLocationManager` instances want a thread
/// with a run loop. The instance property (iOS 14+/macOS 11+) is the fallback if a future OS drops
/// the deprecated class method.
fn location_raw() -> Option<i32> {
    let cls = class("CLLocationManager")?;
    // SAFETY: both forms take no arguments and return CLAuthorizationStatus (an i32 enum); the
    // metaclass check keeps a missing class method from raising instead of falling back.
    unsafe {
        if cls.metaclass().responds_to(sel!(authorizationStatus)) {
            return Some(msg_send![cls, authorizationStatus]);
        }
        let mgr = location_manager()?;
        Some(msg_send![&*mgr, authorizationStatus])
    }
}

fn location_status() -> Status {
    // CoreLocation: 0 notDetermined, 1 restricted, 2 denied, 3 authorizedAlways,
    // 4 authorizedWhenInUse — the shared mapping already treats 3 and 4 as granted.
    match location_raw() {
        Some(raw) => from_apple_status(i64::from(raw)),
        None => Status::Unsupported,
    }
}

/// Whether the granted authorization covers background use (`authorizedAlways` = 3).
fn location_is_always() -> bool {
    location_raw() == Some(3)
}

fn capture_status(media: &str) -> Status {
    let (Some(cls), Some(m)) = (class("AVCaptureDevice"), nsstring(media)) else {
        return Status::Unsupported;
    };
    // SAFETY: `+authorizationStatusForMediaType:` takes an NSString and returns an NSInteger.
    unsafe {
        let raw: isize = msg_send![cls, authorizationStatusForMediaType: &*m];
        from_apple_status(raw as i64)
    }
}

fn photos_status() -> Status {
    let Some(cls) = class("PHPhotoLibrary") else {
        return Status::Unsupported;
    };
    // SAFETY: prefer the access-level form (iOS 14 / macOS 11); 2 = PHAccessLevelReadWrite. Both
    // are class methods, so the probe goes through the metaclass.
    unsafe {
        if cls
            .metaclass()
            .responds_to(sel!(authorizationStatusForAccessLevel:))
        {
            let raw: isize = msg_send![cls, authorizationStatusForAccessLevel: 2isize];
            return from_apple_status(raw as i64);
        }
        let raw: isize = msg_send![cls, authorizationStatus];
        from_apple_status(raw as i64)
    }
}

/// The last notification authorization this process observed. `UNUserNotificationCenter` has no
/// synchronous accessor, so `status()` reports this cache and primes it on first use.
fn notifications_cached() -> &'static std::sync::atomic::AtomicI64 {
    static CACHE: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);
    &CACHE
}

fn notifications_status() -> Status {
    if !is_bundled() {
        return Status::Unsupported;
    }
    match notifications_cached().load(std::sync::atomic::Ordering::Relaxed) {
        -1 => {
            // Prime in the background and answer `Unknown` this once.
            notifications_query(Box::new(|_| {}));
            Status::Unknown
        }
        raw => from_apple_status(raw),
    }
}

/// Read the real notification settings, caching the result. The completion runs on an internal
/// queue — this is the one Apple status with no synchronous form.
fn notifications_query(on_done: Box<dyn FnOnce(Status) + Send>) {
    if !is_bundled() {
        on_done(Status::Unsupported);
        return;
    }
    let Some(cls) = class("UNUserNotificationCenter") else {
        on_done(Status::Unsupported);
        return;
    };
    let done = once(on_done);
    let block = RcBlock::new(move |settings: *mut AnyObject| {
        let status = if settings.is_null() {
            Status::Unknown
        } else {
            // SAFETY: the completion hands us a UNNotificationSettings; its authorizationStatus is
            // an NSInteger. 0 notDetermined, 1 denied, 2 authorized, 3 provisional, 4 ephemeral.
            let raw: isize = unsafe { msg_send![settings, authorizationStatus] };
            match raw {
                0 => Status::Prompt,
                1 => Status::Denied,
                // authorized | provisional | ephemeral — all three may post notifications.
                2..=4 => Status::Granted,
                _ => Status::Unknown,
            }
        };
        notifications_cached().store(
            match status {
                Status::Prompt => 0,
                Status::Restricted => 1,
                Status::Denied => 2,
                Status::Granted => 3,
                _ => -1,
            },
            std::sync::atomic::Ordering::Relaxed,
        );
        done(status);
    });
    // SAFETY: `+currentNotificationCenter` is safe once bundled (checked above), and
    // `-getNotificationSettingsWithCompletionHandler:` takes a block of exactly this signature.
    unsafe {
        let center: *mut AnyObject = msg_send![cls, currentNotificationCenter];
        if center.is_null() {
            return;
        }
        let _: () = msg_send![center, getNotificationSettingsWithCompletionHandler: &*block];
    }
}

// ---------------------------------------------------------------------------
// The part's per-OS contract
// ---------------------------------------------------------------------------

pub fn gate(perm: Permission) -> Gate {
    match perm {
        Permission::Location | Permission::LocationAlways => {
            if class("CLLocationManager").is_some() {
                Gate::Prompts
            } else {
                Gate::Absent
            }
        }
        Permission::Camera | Permission::Microphone => {
            if class("AVCaptureDevice").is_some() {
                Gate::Prompts
            } else {
                Gate::Absent
            }
        }
        Permission::Photos => {
            if class("PHPhotoLibrary").is_some() {
                Gate::Prompts
            } else {
                Gate::Absent
            }
        }
        // Unbundled processes cannot reach the notification center at all.
        Permission::Notifications => {
            if is_bundled() && class("UNUserNotificationCenter").is_some() {
                Gate::Prompts
            } else {
                Gate::Absent
            }
        }
        Permission::Motion => os::motion_gate(),
        Permission::Raw(_) => Gate::Absent,
    }
}

pub fn status(perm: Permission) -> Status {
    if gate(perm) == Gate::Absent {
        return Status::Unsupported;
    }
    match perm {
        Permission::Location => location_status(),
        // "Always" is granted only by the authorizedAlways tier; whenInUse is not enough.
        Permission::LocationAlways => match location_status() {
            Status::Granted if !location_is_always() => Status::Prompt,
            other => other,
        },
        Permission::Camera => capture_status(MEDIA_VIDEO),
        Permission::Microphone => capture_status(MEDIA_AUDIO),
        Permission::Notifications => notifications_status(),
        Permission::Photos => photos_status(),
        Permission::Motion => os::motion_status(),
        Permission::Raw(_) => Status::Unsupported,
    }
}

pub fn status_async(perm: Permission, on_done: Box<dyn FnOnce(Status) + Send>) {
    // Notifications is the only Apple status without a synchronous accessor.
    if perm == Permission::Notifications {
        notifications_query(on_done);
        return;
    }
    on_done(status(perm));
}

pub fn can_prompt(perm: Permission) -> bool {
    // Apple never re-prompts: once the user has answered, only Settings can change it.
    status(perm) == Status::Prompt
}

pub fn should_show_rationale(_perm: Permission) -> bool {
    // No Apple equivalent — a denial here is final, so there is no "ask again" to explain.
    false
}

pub fn request(perm: Permission, on_done: Box<dyn FnOnce(Status) + Send>) {
    match perm {
        Permission::Camera => request_capture(MEDIA_VIDEO, on_done),
        Permission::Microphone => request_capture(MEDIA_AUDIO, on_done),
        Permission::Photos => request_photos(on_done),
        Permission::Notifications => request_notifications(on_done),
        Permission::Location | Permission::LocationAlways => request_location(perm, on_done),
        Permission::Motion => os::request_motion(on_done),
        Permission::Raw(_) => on_done(Status::Unsupported),
    }
}

fn request_capture(media: &'static str, on_done: Box<dyn FnOnce(Status) + Send>) {
    let (Some(cls), Some(m)) = (class("AVCaptureDevice"), nsstring(media)) else {
        on_done(Status::Unsupported);
        return;
    };
    let done = once(on_done);
    let block = RcBlock::new(move |granted: Bool| {
        done(if granted.as_bool() {
            Status::Granted
        } else {
            Status::Denied
        });
    });
    // SAFETY: the completion block's signature matches `void (^)(BOOL)`.
    unsafe {
        let _: () = msg_send![cls, requestAccessForMediaType: &*m, completionHandler: &*block];
    }
}

fn request_photos(on_done: Box<dyn FnOnce(Status) + Send>) {
    let Some(cls) = class("PHPhotoLibrary") else {
        on_done(Status::Unsupported);
        return;
    };
    let done = once(on_done);
    let block = RcBlock::new(move |raw: isize| {
        done(from_apple_status(raw as i64));
    });
    // SAFETY: both forms take `void (^)(PHAuthorizationStatus)`; 2 = PHAccessLevelReadWrite.
    unsafe {
        if cls
            .metaclass()
            .responds_to(sel!(requestAuthorizationForAccessLevel:handler:))
        {
            let _: () =
                msg_send![cls, requestAuthorizationForAccessLevel: 2isize, handler: &*block];
        } else {
            let _: () = msg_send![cls, requestAuthorization: &*block];
        }
    }
}

fn request_notifications(on_done: Box<dyn FnOnce(Status) + Send>) {
    if !is_bundled() {
        on_done(Status::Unsupported);
        return;
    }
    let Some(cls) = class("UNUserNotificationCenter") else {
        on_done(Status::Unsupported);
        return;
    };
    let done = once(on_done);
    let block = RcBlock::new(move |granted: Bool, _err: *mut AnyObject| {
        let status = if granted.as_bool() {
            Status::Granted
        } else {
            Status::Denied
        };
        notifications_cached().store(
            if granted.as_bool() { 3 } else { 2 },
            std::sync::atomic::Ordering::Relaxed,
        );
        done(status);
    });
    // Alert | Badge | Sound — the conventional default set. A finer-grained options API is a
    // follow-up; docs/permissions.md records the choice.
    const OPTIONS: usize = (1 << 0) | (1 << 1) | (1 << 2);
    // SAFETY: the block matches `void (^)(BOOL, NSError *)`.
    unsafe {
        let center: *mut AnyObject = msg_send![cls, currentNotificationCenter];
        if center.is_null() {
            return;
        }
        let _: () = msg_send![
            center,
            requestAuthorizationWithOptions: OPTIONS,
            completionHandler: &*block
        ];
    }
}

/// Ask CoreLocation, then poll for the answer.
///
/// The authorization result arrives at a delegate, which needs a live run loop; polling keeps this
/// working in a plain `main` and under `cargo test` (docs/async.md rule 3). The prompt itself is
/// modal to the user, not to us, so a 120 s cap simply stops the thread if they walk away — the
/// next `status()` still reports whatever they eventually chose.
fn request_location(perm: Permission, on_done: Box<dyn FnOnce(Status) + Send>) {
    let Some(mgr) = location_manager() else {
        on_done(Status::Unsupported);
        return;
    };
    let before = location_status();
    // SAFETY: both selectors take no arguments and return void; they prompt at most once.
    unsafe {
        if perm == Permission::LocationAlways {
            let _: () = msg_send![&*mgr, requestAlwaysAuthorization];
        } else {
            let _: () = msg_send![&*mgr, requestWhenInUseAuthorization];
        }
    }
    // CoreLocation cancels a prompt whose manager is deallocated, and `Retained` is not `Send`, so
    // it cannot ride the polling thread. Leak this one manager for the process lifetime: it is a
    // few dozen bytes, at most one is ever created here, and it keeps the dialog alive.
    std::mem::forget(mgr);

    // Poll for the user's answer — `location_raw` reads a thread-safe class method, so no
    // CoreLocation object is touched off the main thread.
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        loop {
            let now = status(perm);
            if now != before && now != Status::Prompt {
                on_done(now);
                return;
            }
            if std::time::Instant::now() >= deadline {
                on_done(now);
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
}

pub fn open_settings(perm: Permission) -> bool {
    os::open_settings(perm)
}
