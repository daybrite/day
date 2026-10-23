// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// XAML: this crate's C++/WinRT shim (src/lib-xaml-shim.cpp) wrapping a
// ProgressRing (Microsoft.UI.Xaml on WinUI, Windows.UI.Xaml on legacy XAML), boxed into a
// day handle via day-xaml-sys's `day_xaml_box`/`day_xaml_unbox` functions (like the
// media/picker/webview xaml pieces). `IsActive` maps to `.animating`; `.large` sets Width/Height.
// Windows-only; built and exercised by native conformance tests.
// ---------------------------------------------------------------------------

use super::*;
use std::os::raw::{c_int, c_void};

use day_spec::{NodeId, Proposal, Size};
use day_xaml::{WinHandle, Xaml};

unsafe extern "C" {
    fn day_activity_xaml_new(large: c_int, animating: c_int) -> *mut c_void;
    fn day_activity_xaml_set_animating(w: *mut c_void, on: c_int);
}

fn make(_backend: &mut Xaml, p: &ActivityProps, _id: NodeId) -> WinHandle {
    WinHandle(unsafe { day_activity_xaml_new(p.large as c_int, p.animating as c_int) })
}

fn update(_backend: &mut Xaml, h: &WinHandle, patch: &ActivityPatch) {
    match patch {
        ActivityPatch::Animating(on) => unsafe {
            day_activity_xaml_set_animating(h.0, *on as c_int)
        },
    }
}

fn measure(_backend: &mut Xaml, h: &WinHandle, _proposal: Proposal) -> Size {
    // The generic XAML leaf measure fills the parent's proposal. A ProgressRing
    // instead hugs its native size; stretching only its width distorts the animation.
    let mut width = 0.0;
    let mut height = 0.0;
    unsafe { day_xaml_sys::day_xaml_measure(h.0, -1.0, -1.0, &mut width, &mut height) };
    Size::new(width.ceil(), height.ceil())
}

day_pieces::renderer!(day_xaml::RENDERERS, Xaml,
    kind: KIND, props: ActivityProps, patch: ActivityPatch,
    make: make, update: update, measure: measure);
