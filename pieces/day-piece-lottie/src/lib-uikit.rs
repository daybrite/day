// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

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
        path: *const c_char,
        looping: bool,
        autoplay: bool,
        speed: f64,
    ) -> *mut c_void;
    fn day_lottie_set_speed(view: *mut c_void, speed: f64);
}

fn make(_backend: &mut Uikit, p: &LottieProps, _id: NodeId) -> Retained<UIView> {
    let name = CString::new(p.name.as_str()).unwrap_or_default();
    // `lottie("hello")` means `resource/assets/hello.json`, which `day build` stages into the
    // bundle's `assets/` (the same file the Android arm reads through AAssetManager). Resolve it
    // to a path here: lottie-ios's by-name initializer searches the bundle ROOT, so without this
    // an app would have to add a second copy of the file to its Xcode project by hand. An empty
    // path leaves the shim on that by-name path, which is what a project doing so still uses.
    let path = day_spec::resource::resolve_asset_file(&format!("{}.json", p.name))
        .and_then(|p| CString::new(p.to_string_lossy().as_ref()).ok())
        .unwrap_or_default();
    // The shim returns a +1-retained LottieAnimationView (a UIView subclass); we take ownership.
    let ptr = unsafe {
        day_lottie_new(
            name.as_ptr(),
            path.as_ptr(),
            p.looping,
            p.autoplay,
            p.speed,
        )
    };
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
