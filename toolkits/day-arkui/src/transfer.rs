// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
use crate::AHandle;
use day_spec::{Point, transfer::*};
use day_spec::{ffi_guard, sidetable::SideTable};
use std::ffi::{CStr, c_char, c_void};
day_core::tls_group! {
    static SOURCES: SideTable<Source> = SideTable::new();
    static TARGETS: SideTable<Target> = SideTable::new();
}
unsafe extern "C" {
    fn day_arkui_drag_source(
        h: *mut c_void,
        prepare: extern "C" fn(*mut c_void, f64, f64, *mut usize) -> *mut u8,
        free: extern "C" fn(*mut u8, usize),
    );
    fn day_arkui_drop_target(
        h: *mut c_void,
        accept: extern "C" fn(*mut c_void, f64, f64, *const c_char, bool) -> bool,
        receive: extern "C" fn(*mut c_void, f64, f64, *const u8, usize, bool) -> bool,
    );
}
extern "C" fn prepare(h: *mut c_void, x: f64, y: f64, len: *mut usize) -> *mut u8 {
    ffi_guard::contain(std::ptr::null_mut(), || {
        let Some(offer) = SOURCES
            .with(|t| t.get(h as usize))
            .and_then(|f| f(Point::new(x, y)))
        else {
            return std::ptr::null_mut();
        };
        let Some(packet) = offer.encode() else {
            return std::ptr::null_mut();
        };
        unsafe {
            *len = packet.len();
        }
        Box::into_raw(packet.into_boxed_slice()).cast()
    })
}
extern "C" fn free(p: *mut u8, n: usize) {
    unsafe {
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(p, n)));
    }
}
extern "C" fn accept(h: *mut c_void, x: f64, y: f64, types: *const c_char, local: bool) -> bool {
    ffi_guard::contain(false, || {
        let types = unsafe { CStr::from_ptr(types) }
            .to_string_lossy()
            .split('\n')
            .map(str::to_owned)
            .collect();
        TARGETS.with(|t| t.get(h as usize)).is_some_and(|t| {
            t.proposal(&Location {
                local,
                position: Point::new(x, y),
                types,
                allowed: vec![Operation::Copy],
            }) == Operation::Copy
        })
    })
}
extern "C" fn receive(h: *mut c_void, x: f64, y: f64, p: *const u8, n: usize, local: bool) -> bool {
    ffi_guard::contain(false, || {
        if n > MAX_BYTES {
            return false;
        }
        let Some(offer) = Offer::decode(unsafe { std::slice::from_raw_parts(p, n) }) else {
            return false;
        };
        TARGETS.with(|t| t.get(h as usize)).is_some_and(|t| {
            t.deliver(
                Location {
                    local,
                    position: Point::new(x, y),
                    types: offer.types(),
                    allowed: vec![Operation::Copy],
                },
                offer,
            )
        })
    })
}
pub fn source(h: &AHandle, source: Source) {
    SOURCES.with(|t| t.insert(h.0 as usize, source));
    unsafe {
        day_arkui_drag_source(h.0, prepare, free);
    }
}
pub fn target(h: &AHandle, target: Target) {
    TARGETS.with(|t| t.insert(h.0 as usize, target));
    unsafe {
        day_arkui_drop_target(h.0, accept, receive);
    }
}
