// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The iOS half of the Apple arm: CoreMotion activity, and the app's own Settings page.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{msg_send, sel};

use super::{class, nsstring};
use crate::{Gate, Permission, Status, from_apple_status};

/// `CMMotionActivityManager` gates motion and fitness ACTIVITY (step counts, activity
/// classification) — not the raw accelerometer/gyroscope, which need no permission at all
/// (docs/sensors.md).
pub fn motion_gate() -> Gate {
    if class("CMMotionActivityManager").is_some() {
        Gate::Prompts
    } else {
        Gate::Absent
    }
}

pub fn motion_status() -> Status {
    let Some(cls) = class("CMMotionActivityManager") else {
        return Status::Unsupported;
    };
    // SAFETY: `+authorizationStatus` (iOS 11+) takes no arguments and returns an NSInteger;
    // the metaclass probe keeps an older OS from raising.
    unsafe {
        if !cls.metaclass().responds_to(sel!(authorizationStatus)) {
            return Status::Unknown;
        }
        let raw: isize = msg_send![cls, authorizationStatus];
        from_apple_status(raw as i64)
    }
}

/// CoreMotion has no request API. The prompt appears when the app first QUERIES activity data, so
/// this starts a one-shot historical query and then reads the resulting authorization.
pub fn request_motion(on_done: Box<dyn FnOnce(Status) + Send>) {
    let (Some(cls), Some(date_cls)) = (class("CMMotionActivityManager"), class("NSDate")) else {
        on_done(Status::Unsupported);
        return;
    };
    // SAFETY: `+new` returns +1; `queryActivityStartingFromDate:toDate:toQueue:withHandler:` needs a
    // non-nil queue, and `+[NSOperationQueue mainQueue]` is always available.
    let started = unsafe {
        let mgr: Option<Retained<AnyObject>> = msg_send![cls, new];
        let Some(mgr) = mgr else {
            on_done(Status::Unsupported);
            return;
        };
        let from: *mut AnyObject = msg_send![date_cls, distantPast];
        let to: *mut AnyObject = msg_send![date_cls, date];
        let queue_cls = class("NSOperationQueue");
        let queue: *mut AnyObject = match queue_cls {
            Some(q) => msg_send![q, mainQueue],
            None => std::ptr::null_mut(),
        };
        if queue.is_null() {
            false
        } else {
            let handler =
                block2::RcBlock::new(|_activities: *mut AnyObject, _err: *mut AnyObject| {});
            let _: () = msg_send![
                &*mgr,
                queryActivityStartingFromDate: from,
                toDate: to,
                toQueue: queue,
                withHandler: &*handler
            ];
            // The manager must outlive the query; it is a few dozen bytes and at most one is made.
            std::mem::forget(mgr);
            true
        }
    };
    if !started {
        on_done(Status::Unsupported);
        return;
    }
    // Poll for the answer — the same reasoning as location: a delegate/handler needs a run loop,
    // which a plain `main` or `cargo test` does not have.
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            let now = motion_status();
            if now != Status::Prompt || std::time::Instant::now() >= deadline {
                on_done(now);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
}

/// iOS offers exactly one destination — this app's own page in Settings. There is no per-permission
/// anchor, so `perm` is unused.
pub fn open_settings(_perm: Permission) -> bool {
    let (Some(app_cls), Some(url_cls), Some(s)) = (
        class("UIApplication"),
        class("NSURL"),
        // `UIApplicationOpenSettingsURLString`'s value, spelled out so this file needs no UIKit
        // constant (and no crate to import it from).
        nsstring("app-settings:"),
    ) else {
        return false;
    };
    // SAFETY: `+sharedApplication` is nil in an app-extension process (checked); `-openURL:…` takes
    // an options dictionary and a completion block, both of which may be nil/null.
    unsafe {
        let url: *mut AnyObject = msg_send![url_cls, URLWithString: &*s];
        if url.is_null() {
            return false;
        }
        let app: *mut AnyObject = msg_send![app_cls, sharedApplication];
        if app.is_null() {
            return false;
        }
        let _: () = msg_send![
            app,
            openURL: url,
            options: std::ptr::null_mut::<AnyObject>(),
            completionHandler: std::ptr::null_mut::<AnyObject>()
        ];
        true
    }
}
