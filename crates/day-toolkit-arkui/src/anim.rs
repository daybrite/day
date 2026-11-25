// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Backend-executed animation (§8.4) through ArkUI's `animateTo` (`ohos_sys::arkui`
//! `native_animate`): the attribute changes `apply` makes inside it animate from their current
//! values on ArkUI's own compositor.

// Node handles are opaque runtime tokens (see node.rs), never dereferenced here.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::rc::Rc;

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

day_reactive::tls_slots! {
    anim;
    static API: Cell<Option<NonNull<ArkUI_NativeAnimateAPI_1>>> = const { Cell::new(None) };
    // The (node, channel) pairs a repeat-forever run is playing on: the next set of that
    // property must stop it (docs/tween.md rule 7).
    static FOREVER: RefCell<HashSet<(usize, Channel)>> = RefCell::new(HashSet::new());
}

/// The animated property a call sets, so a later set of the same property can stop a run that
/// repeats for ever without touching the node's other animations.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    Opacity,
    Transform,
    Frame,
    Background,
}

/// Forget node `n`'s endless runs: it is being disposed.
pub(crate) fn forget(n: Handle) {
    FOREVER.with(|f| f.borrow_mut().retain(|(node, _)| *node != n as usize));
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
    apply: Option<Rc<dyn Fn()>>,
    option: *mut ArkUI_AnimateOption,
    curve: ArkUI_CurveHandle,
    run_curve: Option<Box<RunCurve>>,
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

/// A whole run as one ArkUI custom curve over `cycles` cycles: each cycle follows the spec's
/// curve (Day's physical spring included, docs/tween.md rule 6), backwards on every second
/// cycle when the run autoreverses, and the end is exactly 1. Finite repeats play this way
/// rather than through ArkUI's iterations: an alternating run with an even cycle count leaves
/// ArkUI's attribute on the start value, where rule 5 wants the target.
struct RunCurve {
    spec: AnimSpec,
    cycles: u32,
}

unsafe extern "C" fn run_curve(fraction: f32, data: *mut c_void) -> f32 {
    // SAFETY: the RunCurve the animation owns for its lifetime.
    let s = unsafe { &*data.cast::<RunCurve>() };
    crate::paint::run_progress(
        |x| s.spec.cycle_progress(x),
        s.spec.autoreverse,
        s.cycles,
        f64::from(fraction),
    ) as f32
}

/// Run `apply`'s attribute changes under `anim`: inside `animateTo`, which interpolates each
/// changed attribute from its current value, or instantly when `anim` is `None`, zero-length,
/// the node has no UI context yet, or ArkUI refuses. `apply` runs exactly once either way.
///
/// Easing curves map to ArkUI's built-in ones; a spring becomes a custom curve evaluating Day's
/// own analytic spring over the duration, so its overshoot and timing match every other
/// backend. `iterations` < 0 repeats forever.
///
/// A set on a `channel` that a repeat-forever run is playing on replaces that run: ArkUI keeps
/// an endless `animateTo` going through a plain attribute write, so an unanimated set runs as a
/// zero-duration `animateTo`, ArkUI's way to stop a running animation at a new value.
pub fn animate(
    node: Handle,
    channel: Channel,
    anim: Option<&AnimSpec>,
    apply: impl Fn() + 'static,
) {
    let key = (node as usize, channel);
    let endless = anim.is_some_and(|a| a.duration_ms > 0 && a.repeat == u32::MAX);
    let was_endless = FOREVER.with(|f| {
        let mut f = f.borrow_mut();
        if endless {
            !f.insert(key)
        } else {
            f.remove(&key)
        }
    });
    let cut = AnimSpec::linear(0);
    let anim = match anim.filter(|a| a.duration_ms > 0) {
        Some(a) => a,
        None if was_endless => &cut,
        None => {
            apply();
            return;
        }
    };
    let a = anim;
    let mut animation = Box::new(Animation {
        apply: Some(Rc::new(apply)),
        option: ptr::null_mut(),
        curve: ptr::null_mut(),
        run_curve: None,
    });
    let ctx = if node.is_null() {
        ptr::null_mut()
    } else {
        // SAFETY: `node` is a non-null handle the caller holds live, on the UI thread; the call
        // only reads its context. Every object created below is owned by `animation`.
        unsafe { OH_ArkUI_GetContextByNode(node) }
    };
    let Some(api) = api().filter(|_| !ctx.is_null()) else {
        animation.run();
        return;
    };
    // A finite run is one pass of a run curve over all its cycles; only an endless one uses
    // ArkUI's iterations (-1) and alternate play mode.
    let endless = a.repeat == u32::MAX;
    let cycles = if endless {
        1
    } else {
        a.repeat.saturating_add(1)
    };
    let duration = u64::from(a.duration_ms) * u64::from(cycles);
    let duration = i32::try_from(duration).unwrap_or(i32::MAX);
    // SAFETY: options and curves created here belong to `animation`, released in its Drop.
    let rc = unsafe {
        let option = OH_ArkUI_AnimateOption_Create();
        animation.option = option;
        OH_ArkUI_AnimateOption_SetDuration(option, duration);
        OH_ArkUI_AnimateOption_SetDelay(option, a.delay_ms.min(i32::MAX as u32) as i32);
        OH_ArkUI_AnimateOption_SetIterations(option, if endless { -1 } else { 1 });
        OH_ArkUI_AnimateOption_SetPlayMode(
            option,
            if endless && a.autoreverse {
                ArkUI_AnimationPlayMode::ARKUI_ANIMATION_PLAY_MODE_ALTERNATE
            } else {
                ArkUI_AnimationPlayMode::ARKUI_ANIMATION_PLAY_MODE_NORMAL
            },
        );
        match a.curve {
            _ if matches!(a.curve, Curve::Spring { .. }) || cycles > 1 => {
                let curve = Box::new(RunCurve { spec: *a, cycles });
                let data = ptr::from_ref(&*curve) as *mut c_void;
                animation.run_curve = Some(curve);
                animation.curve = OH_ArkUI_Curve_CreateCustomCurve(data, Some(run_curve));
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
            Some(f) => f(ctx, option, &raw mut update, &raw mut done),
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
