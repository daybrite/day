// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// XAML: this crate's C++/WinRT shim (src/lib-xaml-shim.cpp) wrapping a
// ProgressRing (Microsoft.UI.Xaml on WinUI, Windows.UI.Xaml on legacy XAML), boxed into a
// day handle via day-xaml-sys's `day_xaml_box`/`day_xaml_unbox` functions (like the
// media/picker/webview xaml pieces). `IsActive` maps to `.animating`; `.large` sets Width/Height.
// Windows-only; built and exercised by native conformance tests.
// ---------------------------------------------------------------------------

use day_spec::props::{ProgressPatch as ActivityPatch, ProgressProps as ActivityProps};
use std::os::raw::{c_int, c_void};

use crate::{WinHandle, Xaml};
use day_spec::NodeId;

unsafe extern "C" {
    fn day_activity_xaml_new(large: c_int, animating: c_int) -> *mut c_void;
    fn day_activity_xaml_set_animating(w: *mut c_void, on: c_int);
}

pub(crate) fn make(_backend: &mut Xaml, p: &ActivityProps, _id: NodeId) -> WinHandle {
    WinHandle(unsafe { day_activity_xaml_new(p.large as c_int, p.animating as c_int) })
}

pub(crate) fn update(_backend: &mut Xaml, h: &WinHandle, patch: &ActivityPatch) {
    match patch {
        ActivityPatch::Value(_) => {}
        ActivityPatch::Animating(on) => unsafe {
            day_activity_xaml_set_animating(h.0, *on as c_int)
        },
    }
}
