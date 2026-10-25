// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
use day_spec::sidetable::SideTable;
use day_spec::{Point, ffi_guard, transfer::*};
std::thread_local! {
    static SOURCES: SideTable<Source> = SideTable::new();
    static TARGETS: SideTable<Target> = SideTable::new();
}
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn day_dom_drag_source(el: u32);
    fn day_dom_drop_target(el: u32);
    fn day_dom_drag_offer(p: *const u8, n: usize);
}
pub fn source(el: u32, source: Source) {
    SOURCES.with(|t| t.insert(el as usize, source));
    unsafe {
        day_dom_drag_source(el);
    }
}
pub fn target(el: u32, target: Target) {
    TARGETS.with(|t| t.insert(el as usize, target));
    unsafe {
        day_dom_drop_target(el);
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_drag_prepare(el: u32, x: f64, y: f64) {
    ffi_guard::contain((), || {
        if let Some(packet) = SOURCES
            .with(|t| t.get(el as usize))
            .and_then(|f| f(Point::new(x, y)))
            .and_then(|o| o.encode())
        {
            unsafe {
                day_dom_drag_offer(packet.as_ptr(), packet.len());
            }
        }
    });
}
// Each callback consumes the allocation created by the trusted JS shim.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn day_dom_drag_accept(
    el: u32,
    x: f64,
    y: f64,
    p: *mut u8,
    n: usize,
    local: bool,
) -> bool {
    let bytes = unsafe { Vec::from_raw_parts(p, n, n) };
    ffi_guard::contain(false, || {
        let at = Location {
            position: Point::new(x, y),
            local,
            allowed: vec![Operation::Copy],
            types: String::from_utf8_lossy(&bytes)
                .split('\n')
                .map(str::to_owned)
                .collect(),
        };
        TARGETS
            .with(|t| t.get(el as usize))
            .is_some_and(|t| t.proposal(&at) == Operation::Copy)
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn day_dom_drag_receive(
    el: u32,
    x: f64,
    y: f64,
    p: *mut u8,
    n: usize,
    local: bool,
) -> bool {
    let bytes = unsafe { Vec::from_raw_parts(p, n, n) };
    ffi_guard::contain(false, || {
        let Some(offer) = Offer::decode(&bytes) else {
            return false;
        };
        TARGETS.with(|t| t.get(el as usize)).is_some_and(|t| {
            t.deliver(
                Location {
                    position: Point::new(x, y),
                    local,
                    allowed: vec![Operation::Copy],
                    types: offer.types(),
                },
                offer,
            )
        })
    })
}
