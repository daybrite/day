// ---------------------------------------------------------------------------
// AppKit: an NSHostingView created by this crate's Swift shim (apple/swift/DaySwiftUI.swift → the
// generated DayPieces SwiftPM package, statically linked into the cargo binary by `day build`).
// Rust calls the shim's flat C ABI and wraps the returned +1-retained NSView. The provider class
// the shim resolves comes from the app's own Swift sources — zero project-file edits.
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::CString;
use std::os::raw::{c_char, c_void};

use day_appkit::AppKit;
use day_spec::NodeId;
use objc2::rc::Retained;
use objc2_app_kit::NSView;

unsafe extern "C" {
    fn day_swiftui_make(
        name: *const c_char,
        params: *const c_char,
        state_key: *const c_char,
    ) -> *mut c_void;
    fn day_swiftui_update(view: *mut c_void, params: *const c_char);
}

fn make(_backend: &mut AppKit, p: &SwiftUiProps, _id: NodeId) -> Retained<NSView> {
    let name = CString::new(p.name.as_str()).unwrap_or_default();
    let params = p.params.as_deref().and_then(|s| CString::new(s).ok());
    let params_ptr = params.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
    // With a state key the shim returns the RETAINED hosting view from a prior mount (its SwiftUI
    // state intact, the new params applied) instead of creating a fresh one.
    let key = p.state_key.as_deref().and_then(|s| CString::new(s).ok());
    let key_ptr = key.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
    // The shim returns a +1-retained hosting view (never null — a missing provider hosts a visible
    // error view instead); we take ownership.
    let ptr = unsafe { day_swiftui_make(name.as_ptr(), params_ptr, key_ptr) };
    unsafe { Retained::from_raw(ptr.cast::<NSView>()) }.expect("DaySwiftUI hosting view")
}

fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &SwiftUiPatch) {
    match patch {
        // The stored NSView IS the hosting view; the shim casts the pointer back, re-invokes the
        // provider's body with the new JSON, and replaces the rootView.
        SwiftUiPatch::Params(json) => {
            let Ok(params) = CString::new(json.as_str()) else {
                return;
            };
            let ptr = (&**h as *const NSView) as *mut c_void;
            unsafe { day_swiftui_update(ptr, params.as_ptr()) };
        }
    }
}

// `name` is set once at build; only `params` patches.
day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: SwiftUiProps, patch: SwiftUiPatch, make: make, update: update,
    measure: day_pieces::fill_measure);
