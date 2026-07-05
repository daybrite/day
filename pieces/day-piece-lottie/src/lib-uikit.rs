// ---------------------------------------------------------------------------
// UIKit: a LottieAnimationView from airbnb/lottie-ios, created by this crate's Swift shim
// (ios/swift/DayLottie.swift → the generated DayPieces SwiftPM package). Rust calls the shim's flat
// C ABI and wraps the returned +1-retained UIView. The lottie-ios SwiftPM dependency is declared in
// this crate's [package.metadata.day.ios]; `day build` links it into the app — zero .xcodeproj edits.
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::CString;
use std::os::raw::{c_char, c_void};

use day_spec::NodeId;
use day_uikit::Uikit;
use objc2::rc::Retained;
use objc2_ui_kit::UIView;

unsafe extern "C" {
    fn day_lottie_new(
        name: *const c_char,
        looping: bool,
        autoplay: bool,
        speed: f64,
    ) -> *mut c_void;
    fn day_lottie_set_speed(view: *mut c_void, speed: f64);
}

fn make(_backend: &mut Uikit, p: &LottieProps, _id: NodeId) -> Retained<UIView> {
    let name = CString::new(p.name.as_str()).unwrap_or_default();
    // The shim returns a +1-retained LottieAnimationView (a UIView subclass); we take ownership.
    let ptr = unsafe { day_lottie_new(name.as_ptr(), p.looping, p.autoplay, p.speed) };
    unsafe { Retained::from_raw(ptr.cast::<UIView>()) }.expect("LottieAnimationView")
}

fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &LottiePatch) {
    match patch {
        // The stored UIView IS the LottieAnimationView; the shim casts the pointer back to set speed.
        LottiePatch::Speed(s) => {
            let ptr = (&**h as *const UIView) as *mut c_void;
            unsafe { day_lottie_set_speed(ptr, *s) };
        }
    }
}

// name/looping/autoplay are set once at build; only `speed` patches.
day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
    kind: KIND, props: LottieProps, patch: LottiePatch, make: make, update: update,
    measure: day_pieces::fill_measure);
