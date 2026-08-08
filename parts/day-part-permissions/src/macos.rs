// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The macOS half of the Apple arm: no CoreMotion, and a different settings deep link.

use objc2::msg_send;
use objc2::runtime::AnyObject;

use super::{class, nsstring};
use crate::{Gate, Permission, Status};

/// macOS has no `CMMotionActivityManager` — CoreMotion's activity APIs are iOS-only.
pub fn motion_gate() -> Gate {
    Gate::Absent
}

pub fn motion_status() -> Status {
    Status::Unsupported
}

pub fn request_motion(on_done: Box<dyn FnOnce(Status) + Send>) {
    on_done(Status::Unsupported);
}

/// The System Settings privacy pane for this permission. macOS accepts a per-anchor URL, so the
/// user lands on the right list rather than the top of Privacy & Security.
pub fn open_settings(perm: Permission) -> bool {
    let anchor = match perm {
        Permission::Location | Permission::LocationAlways => "Privacy_LocationServices",
        Permission::Camera => "Privacy_Camera",
        Permission::Microphone => "Privacy_Microphone",
        Permission::Photos => "Privacy_Photos",
        Permission::Notifications => "Privacy_Notifications",
        // Nothing on macOS gates motion, so there is no pane to open.
        Permission::Motion | Permission::Raw(_) => return false,
    };
    let url = format!("x-apple.systempreferences:com.apple.preference.security?{anchor}");
    let (Some(ws_cls), Some(url_cls), Some(s)) =
        (class("NSWorkspace"), class("NSURL"), nsstring(&url))
    else {
        return false;
    };
    // SAFETY: `+URLWithString:` takes an NSString and may return nil (checked); `-openURL:`
    // returns BOOL. Both are safe from any thread.
    unsafe {
        let url: *mut AnyObject = msg_send![url_cls, URLWithString: &*s];
        if url.is_null() {
            return false;
        }
        let ws: *mut AnyObject = msg_send![ws_cls, sharedWorkspace];
        if ws.is_null() {
            return false;
        }
        let ok: objc2::runtime::Bool = msg_send![ws, openURL: url];
        ok.as_bool()
    }
}
