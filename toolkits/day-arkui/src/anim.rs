// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Backend-executed animation (§8.4) through ArkUI's `animateTo` (`ohos_sys::arkui`
//! `native_animate`): the attribute changes `apply` makes inside it animate from their current
//! values on ArkUI's own compositor.

// Node handles are opaque runtime tokens (see node.rs), never dereferenced here.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::{self, NonNull};

use day_spec::{AnimSpec, Curve};
use ohos_sys::arkui::native_animate::{
    ArkUI_AnimateCompleteCallback, ArkUI_AnimateOption, ArkUI_CurveHandle,
    ArkUI_NativeAnimateAPI_1, OH_ArkUI_AnimateOption_Create, OH_ArkUI_AnimateOption_Dispose,
    OH_ArkUI_AnimateOption_SetCurve, OH_ArkUI_AnimateOption_SetDelay,
    OH_ArkUI_AnimateOption_SetDuration, OH_ArkUI_AnimateOption_SetICurve,
    OH_ArkUI_AnimateOption_SetIterations, OH_ArkUI_AnimateOption_SetPlayMode,
    OH_ArkUI_Curve_CreateCustomCurve, OH_ArkUI_Curve_DisposeCurve,
};
use ohos_sys::arkui::native_interface::{
    ArkUI_NativeAPIVariantKind, OH_ArkUI_QueryModuleInterfaceByName,
};
use ohos_sys::arkui::native_node::OH_ArkUI_GetContextByNode;
use ohos_sys::arkui::native_type::{
    ArkUI_AnimationCurve, ArkUI_AnimationPlayMode, ArkUI_ContextCallback, ArkUI_FinishCallbackType,
};

use crate::node::Handle;

thread_local! {
    static API: Cell<Option<NonNull<ArkUI_NativeAnimateAPI_1>>> = const { Cell::new(None) };
}

fn api() -> Option<&'static ArkUI_NativeAnimateAPI_1> {
    API.with(|slot| {
        if let Some(p) = slot.get() {
            // SAFETY: a process-lifetime table inside ArkUI.
            return Some(unsafe { &*p.as_ptr() });
        }
        // SAFETY: a name lookup with a valid C string.
        let raw = unsafe {
            OH_ArkUI_QueryModuleInterfaceByName(
                ArkUI_NativeAPIVariantKind::ARKUI_NATIVE_ANIMATE,
                c"ArkUI_NativeAnimateAPI_1".as_ptr(),
            )
        };
        let p = NonNull::new(raw.cast::<ArkUI_NativeAnimateAPI_1>())?;
        slot.set(Some(p));
        // SAFETY: as above.
        Some(unsafe { &*p.as_ptr() })
    })
}

/// One animation in flight: the apply closure (run exactly once) and what to release when the
/// animation ends.
struct Animation {
    apply: Option<Box<dyn FnOnce()>>,
    option: *mut ArkUI_AnimateOption,
    curve: ArkUI_CurveHandle,
    spring: Option<Box<SpringCurve>>,
}

impl Animation {
    fn run(&mut self) {
        if let Some(apply) = self.apply.take() {
            apply();
        }
    }
}

impl Drop for Animation {
    fn drop(&mut self) {
        // SAFETY: objects this animation created.
        unsafe {
            if !self.option.is_null() {
                OH_ArkUI_AnimateOption_Dispose(self.option);
            }
            if !self.curve.is_null() {
                OH_ArkUI_Curve_DisposeCurve(self.curve);
            }
        }
    }
}

unsafe extern "C" fn on_update(data: *mut c_void) {
    // SAFETY: the Animation box `animate` leaked; alive until `on_done`.
    let a = unsafe { &mut *data.cast::<Animation>() };
    day_spec::ffi_guard::contain((), || a.run());
}

unsafe extern "C" fn on_done(data: *mut c_void) {
    // SAFETY: the box leaked by `animate`, reclaimed exactly once here.
    drop(unsafe { Box::from_raw(data.cast::<Animation>()) });
}

/// A spring [`Curve`] sampled as an ArkUI custom curve over `secs`. `end` is the spring's value
/// at `secs`, just short of 1 for a spring still ringing then; the curve adds the remainder
/// linearly so it lands exactly on 1, since ArkUI jumps to the target value at the end of an
/// animation whose curve stops elsewhere.
struct SpringCurve {
    curve: Curve,
    secs: f64,
    end: f64,
}

unsafe extern "C" fn spring_curve(fraction: f32, data: *mut c_void) -> f32 {
    // SAFETY: the SpringCurve the animation owns for its lifetime.
    let s = unsafe { &*data.cast::<SpringCurve>() };
    let x = f64::from(fraction).clamp(0.0, 1.0);
    (s.curve.fraction(x * s.secs, s.secs) + (1.0 - s.end) * x) as f32
}

/// Run `apply`'s attribute changes under `anim`: inside `animateTo`, which interpolates each
/// changed attribute from its current value, or instantly when `anim` is `None`, zero-length,
/// the node has no UI context yet, or ArkUI refuses. `apply` runs exactly once either way.
///
/// Easing curves map to ArkUI's built-in ones; a spring becomes a custom curve evaluating Day's
/// own analytic spring over the duration, so its overshoot and timing match every other
/// backend. `iterations` < 0 repeats forever.
pub fn animate(node: Handle, anim: Option<&AnimSpec>, apply: impl FnOnce() + 'static) {
    let Some(a) = anim.filter(|a| a.duration_ms > 0) else {
        apply();
        return;
    };
    let mut animation = Box::new(Animation {
        apply: Some(Box::new(apply)),
        option: ptr::null_mut(),
        curve: ptr::null_mut(),
        spring: None,
    });
    // SAFETY: the node's context is read; every object created is owned by `animation`.
    let ctx = if node.is_null() {
        ptr::null_mut()
    } else {
        unsafe { OH_ArkUI_GetContextByNode(node) }
    };
    let Some(api) = api().filter(|_| !ctx.is_null()) else {
        animation.run();
        return;
    };
    let iterations = if a.repeat == u32::MAX {
        -1
    } else {
        i32::try_from(a.repeat).map_or(i32::MAX, |r| r.saturating_add(1))
    };
    // SAFETY: options and curves created here belong to `animation`, released in its Drop.
    let rc = unsafe {
        let option = OH_ArkUI_AnimateOption_Create();
        animation.option = option;
        OH_ArkUI_AnimateOption_SetDuration(option, a.duration_ms.min(i32::MAX as u32) as i32);
        OH_ArkUI_AnimateOption_SetDelay(option, a.delay_ms.min(i32::MAX as u32) as i32);
        OH_ArkUI_AnimateOption_SetIterations(option, iterations);
        OH_ArkUI_AnimateOption_SetPlayMode(
            option,
            if a.autoreverse {
                ArkUI_AnimationPlayMode::ARKUI_ANIMATION_PLAY_MODE_ALTERNATE
            } else {
                ArkUI_AnimationPlayMode::ARKUI_ANIMATION_PLAY_MODE_NORMAL
            },
        );
        match a.curve {
            Curve::Spring { .. } => {
                let secs = f64::from(a.duration_ms) / 1000.0;
                let spring = Box::new(SpringCurve {
                    curve: a.curve,
                    secs,
                    end: a.curve.fraction(secs, secs),
                });
                let data = ptr::from_ref(&*spring) as *mut c_void;
                animation.spring = Some(spring);
                animation.curve = OH_ArkUI_Curve_CreateCustomCurve(data, Some(spring_curve));
                if animation.curve.is_null() {
                    OH_ArkUI_AnimateOption_SetCurve(
                        option,
                        ArkUI_AnimationCurve::ARKUI_CURVE_EASE_IN_OUT,
                    );
                } else {
                    OH_ArkUI_AnimateOption_SetICurve(option, animation.curve);
                }
            }
            curve => {
                let c = match curve {
                    Curve::Linear => ArkUI_AnimationCurve::ARKUI_CURVE_LINEAR,
                    Curve::EaseIn => ArkUI_AnimationCurve::ARKUI_CURVE_EASE_IN,
                    Curve::EaseOut => ArkUI_AnimationCurve::ARKUI_CURVE_EASE_OUT,
                    _ => ArkUI_AnimationCurve::ARKUI_CURVE_EASE_IN_OUT,
                };
                OH_ArkUI_AnimateOption_SetCurve(option, c);
            }
        }
        let raw = Box::into_raw(animation);
        let mut update = ArkUI_ContextCallback {
            userData: raw.cast(),
            callback: Some(on_update),
        };
        let mut done = ArkUI_AnimateCompleteCallback {
            type_: ArkUI_FinishCallbackType::ARKUI_FINISH_CALLBACK_REMOVED,
            callback: Some(on_done),
            userData: raw.cast(),
        };
        let rc = match api.animateTo {
            Some(f) => f(ctx, option, &mut update, &mut done),
            None => -1,
        };
        if rc != 0 {
            // Not animated: apply instantly, and nothing will call `done`, so free here.
            let mut animation = Box::from_raw(raw);
            animation.run();
            drop(animation);
        }
        rc
    };
    let _ = rc;
}
