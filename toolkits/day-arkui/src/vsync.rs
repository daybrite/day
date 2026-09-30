// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The frame clock (§8.4, docs/frames.md) on `OH_NativeVSync` (`ohos_sys::vsync`).
//!
//! Native VSync may arrive off the JS/UI thread. Only an integer ticket crosses threads, then
//! resolves on the UI loop, so cancellation can destroy the source without leaving a dangling
//! Rust closure pointer in a callback already queued by the system.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_longlong, c_void};
use std::rc::Rc;

use ohos_sys::vsync::{
    OH_NativeVSync, OH_NativeVSync_Create, OH_NativeVSync_Destroy, OH_NativeVSync_RequestFrame,
};

/// The frame callback: `(token, timestamp in seconds)`.
pub type Deliver = fn(u64, f64);

struct Source {
    vsync: *mut OH_NativeVSync,
    token: u64,
}

impl Drop for Source {
    fn drop(&mut self) {
        // SAFETY: a source this module created.
        unsafe { OH_NativeVSync_Destroy(self.vsync) };
    }
}

thread_local! {
    /// Host node pointer → its live VSync source.
    static SOURCES: RefCell<HashMap<usize, Rc<RefCell<Source>>>> = RefCell::new(HashMap::new());
    /// Ticket → (host, callback) for requests not yet delivered.
    static PENDING: RefCell<HashMap<u64, (usize, Deliver)>> = RefCell::new(HashMap::new());
}

fn deliver(token: u64, timestamp: c_longlong) {
    let Some((host, cb)) = PENDING.with(|p| p.borrow_mut().remove(&token)) else {
        return;
    };
    let Some(source) = SOURCES.with(|s| s.borrow().get(&host).cloned()) else {
        return;
    };
    source.borrow_mut().token = 0;
    cb(token, timestamp as f64 / 1e9);
    // The callback may have requested the next frame; release the source only when it didn't.
    if source.borrow().token == 0 {
        SOURCES.with(|s| s.borrow_mut().remove(&host));
    }
}

unsafe extern "C" fn native_vsync(timestamp: c_longlong, data: *mut c_void) {
    let token = data as usize as u64;
    crate::main_thread::post(Box::new(move || deliver(token, timestamp)));
}

/// Ask for the next frame for `host`; `cb` runs on the UI thread with `token` and the frame's
/// timestamp. False when the platform refused.
pub fn request_frame(host: usize, token: u64, cb: Deliver) -> bool {
    let source = SOURCES.with(|s| {
        let mut s = s.borrow_mut();
        if let Some(existing) = s.get(&host) {
            return Some(existing.clone());
        }
        // SAFETY: a name for the connection; null when the platform has no VSync.
        let vsync = unsafe { OH_NativeVSync_Create(c"Day".as_ptr(), 3) };
        if vsync.is_null() {
            return None;
        }
        let source = Rc::new(RefCell::new(Source { vsync, token: 0 }));
        s.insert(host, source.clone());
        Some(source)
    });
    let Some(source) = source else {
        return false;
    };
    source.borrow_mut().token = token;
    PENDING.with(|p| p.borrow_mut().insert(token, (host, cb)));
    // SAFETY: a live source; the callback carries only the ticket.
    let rc = unsafe {
        OH_NativeVSync_RequestFrame(
            source.borrow().vsync,
            Some(native_vsync),
            token as usize as *mut c_void,
        )
    };
    if rc != 0 {
        PENDING.with(|p| p.borrow_mut().remove(&token));
        SOURCES.with(|s| s.borrow_mut().remove(&host));
        return false;
    }
    true
}

pub fn cancel_frame(token: u64) {
    let Some((host, _)) = PENDING.with(|p| p.borrow_mut().remove(&token)) else {
        return;
    };
    SOURCES.with(|s| s.borrow_mut().remove(&host));
}
