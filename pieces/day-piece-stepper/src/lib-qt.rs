// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Qt: this crate's OWN shim (src/lib-qt-shim.cpp) — a `QDoubleSpinBox`, Qt's real
// field-with-arrows control. The value crosses the flat C ABI as a double both ways.
// ---------------------------------------------------------------------------

use super::*;
use std::os::raw::{c_int, c_void};

use day_qt::{Qt, QtHandle};
use day_spec::{NodeId, Proposal, Size};

unsafe extern "C" {
    fn day_stepper_new(
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        decimals: c_int,
        id: u64,
        cb: extern "C" fn(u64, f64),
    ) -> *mut c_void;
    fn day_stepper_set(h: *mut c_void, value: f64);
    // From day-qt-sys (already linked into the binary):
    fn day_qt_size_hint(w: *mut c_void, out_w: *mut f64, out_h: *mut f64);
}

extern "C" fn on_value(id: u64, v: f64) {
    day_qt::emit(NodeId(id), Event::custom(VALUE_TAG, v.to_string()));
}

fn make(_backend: &mut Qt, p: &StepperProps, id: NodeId) -> QtHandle {
    QtHandle(unsafe {
        day_stepper_new(
            p.value,
            p.min,
            p.max,
            p.step,
            p.decimals as c_int,
            id.0,
            on_value,
        )
    })
}

fn update(_backend: &mut Qt, h: &QtHandle, patch: &StepperPatch) {
    let StepperPatch::SetValue(v) = patch;
    unsafe { day_stepper_set(h.0, *v) };
}

fn measure(_backend: &mut Qt, h: &QtHandle, _p: Proposal) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_qt_size_hint(h.0, &mut w, &mut hh) };
    Size::new(w.max(96.0), hh.max(24.0))
}

day_pieces::renderer!(day_qt::RENDERERS, Qt,
    kind: KIND, props: StepperProps, patch: StepperPatch,
    make: make, update: update, measure: measure);
