// ---------------------------------------------------------------------------
// UIKit: a LottieAnimationView from airbnb/lottie-ios, created by this crate's Swift shim
// (ios/swift/DayLottie.swift → the generated DayPieces SwiftPM package). Rust calls the shim's flat
// C ABI and wraps the returned +1-retained UIView. The lottie-ios SwiftPM dependency is declared in
// this crate's [package.metadata.day.ios]; `day build` links it into the app — zero .xcodeproj edits.
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::CString;
use std::os::raw::{c_char, c_void};

use day_spec::{NodeId, Renderer};
use day_uikit::Uikit;
use linkme::distributed_slice;
use objc2::rc::Retained;
use objc2_ui_kit::UIView;

unsafe extern "C" {
    fn day_lottie_new(name: *const c_char, looping: bool, autoplay: bool) -> *mut c_void;
}

fn make(_backend: &mut Uikit, props: &dyn std::any::Any, _id: NodeId) -> Retained<UIView> {
    let p = props.downcast_ref::<LottieProps>().unwrap();
    let name = CString::new(p.name.as_str()).unwrap_or_default();
    // The shim returns a +1-retained LottieAnimationView (a UIView subclass); we take ownership.
    let ptr = unsafe { day_lottie_new(name.as_ptr(), p.looping, p.autoplay) };
    unsafe { Retained::from_raw(ptr.cast::<UIView>()) }.expect("LottieAnimationView")
}

fn update(_backend: &mut Uikit, _h: &Retained<UIView>, _patch: &dyn std::any::Any) {
    // No patches: the animation is configured once at build (name/looping/autoplay).
}

#[distributed_slice(day_uikit::RENDERERS)]
static LOTTIE_UIKIT: fn() -> Renderer<Uikit> = || Renderer {
    kind: KIND,
    make,
    update,
    measure: None,
};
