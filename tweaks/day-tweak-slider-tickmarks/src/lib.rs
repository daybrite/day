//! day-tweak-slider-tickmarks — the full-range packaged-tweak example (docs/tweaks.md): native
//! tick marks (and, where the platform supports it, snap-to-tick) on Day's `slider(…)`, across
//! six toolkits, exercising every tweak access tier:
//!
//! | toolkit | mechanism | count | position | snap |
//! |---|---|---|---|---|
//! | AppKit  | objc2 (`NSSlider` tick API)                  | ✓ | Above/Below | ✓ |
//! | GTK     | gtk4-rs (`Scale::add_mark`)                  | ✓ | ✓ (incl. Both) | ✗ (no native snap) |
//! | Android | JNI (Material `Slider` step size)            | ✓ | ✗ (Material draws its own) | always on (Material snaps with steps) |
//! | Qt      | own C++ (`QSlider` ticks; docs/tweaks.md recipe) | ✓ | ✓ | ✗ (no native snap) |
//! | WinUI   | own C++/WinRT (`Slider` TickFrequency/SnapsTo)   | ✓ | ✓ | ✓ |
//! | ArkUI   | own C++ against the NDK (`NODE_SLIDER_STEP`) | ✓ | ✗ | always on (steps snap) |
//! | UIKit   | — `UISlider` has NO native tick API: documented no-op | | | |
//!
//! ```ignore
//! use day_tweak_slider_tickmarks::{SliderTickmarksTweak, TickPosition, Tickmarks};
//! slider(volume).range(0.0..=100.0).tickmarks(Tickmarks::count(11).snap(true))
//! ```
//!
//! Tick configuration is UNMANAGED (Day never patches it), so it survives Day's own value
//! updates. Where a column says ✗ or "always on", that's the platform's reality — the tweak
//! reports it here instead of faking it.

use day_core::RNode;
use day_pieces::Decorate;

/// Where tick marks are drawn, on toolkits that let you choose.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TickPosition {
    #[default]
    Below,
    Above,
    /// Both sides (GTK, Qt); falls back to Below where only one side is supported (AppKit).
    Both,
}

/// The tick configuration: how many marks, whether values snap to them, where they draw.
#[derive(Clone, Copy, Debug)]
pub struct Tickmarks {
    pub count: u32,
    pub snap: bool,
    pub position: TickPosition,
}

impl Tickmarks {
    /// `count` marks (≥ 2: one at each end plus evenly spaced between), no snapping, below.
    pub fn count(count: u32) -> Self {
        Tickmarks {
            count: count.max(2),
            snap: false,
            position: TickPosition::Below,
        }
    }
    pub fn snap(mut self, snap: bool) -> Self {
        self.snap = snap;
        self
    }
    pub fn position(mut self, position: TickPosition) -> Self {
        self.position = position;
        self
    }
}

/// `.tickmarks(…)` on any piece whose native widget is a slider (i.e. `slider(…)`).
pub trait SliderTickmarksTweak: Decorate + Sized {
    #[allow(unused_variables)]
    fn tickmarks(self, ticks: Tickmarks) -> day_core::AnyPiece {
        self.tweak(move |n| apply(n, ticks))
    }
}

impl<P: Decorate> SliderTickmarksTweak for P {}

#[allow(unused_variables)]
fn apply(node: RNode, t: Tickmarks) {
    #[cfg(feature = "appkit")]
    {
        use objc2_app_kit::{NSSlider, NSTickMarkPosition};
        let _ = day_appkit::with_native(node, |view, _mtm| {
            if let Some(s) = view.downcast_ref::<NSSlider>() {
                s.setNumberOfTickMarks(t.count as isize);
                s.setAllowsTickMarkValuesOnly(t.snap);
                s.setTickMarkPosition(match t.position {
                    TickPosition::Above => NSTickMarkPosition::Above,
                    _ => NSTickMarkPosition::Below, // Both unsupported on AppKit
                });
                // Tick marks add to the control's intrinsic height.
                day_core::invalidate_size(node);
            }
        });
    }
    #[cfg(feature = "gtk")]
    {
        use gtk4::prelude::*;
        let _ = day_gtk::with_native(node, |w| {
            if let Some(scale) = w.downcast_ref::<gtk4::Scale>() {
                let (lo, hi) = (scale.adjustment().lower(), scale.adjustment().upper());
                scale.clear_marks();
                for i in 0..t.count {
                    let v = lo + (hi - lo) * f64::from(i) / f64::from(t.count - 1);
                    match t.position {
                        TickPosition::Below => scale.add_mark(v, gtk4::PositionType::Bottom, None),
                        TickPosition::Above => scale.add_mark(v, gtk4::PositionType::Top, None),
                        TickPosition::Both => {
                            scale.add_mark(v, gtk4::PositionType::Bottom, None);
                            scale.add_mark(v, gtk4::PositionType::Top, None);
                        }
                    }
                }
                day_core::invalidate_size(node);
            }
        });
        // snap: GtkScale has no native snap-to-marks — documented, not emulated.
    }
    #[cfg(all(feature = "widget", target_os = "android"))]
    {
        // Material Slider: a step size yields visible ticks AND snapping (Material always snaps
        // when stepped — `snap: false` is not honorable here, per the table above).
        use day_android::jni::objects::JValue;
        let _ = day_android::with_native(node, |view, env| {
            let lo = env
                .call_method(view, "getValueFrom", "()F", &[])
                .and_then(|v| v.f())
                .unwrap_or(0.0);
            let hi = env
                .call_method(view, "getValueTo", "()F", &[])
                .and_then(|v| v.f())
                .unwrap_or(1.0);
            let step = (hi - lo) / (t.count.saturating_sub(1).max(1)) as f32;
            let _ = env.call_method(view, "setStepSize", "(F)V", &[JValue::Float(step)]);
            // A stepped Material slider requires EVERY value to sit on the step grid — it
            // hard-crashes at the next layout pass otherwise (BaseSlider.validateValues). Snap
            // the current value now. NOTE: a programmatic setValue does NOT notify the bound
            // Signal (fromUser=false), so an off-grid initial value diverges from the widget
            // until the next user interaction — start stepped sliders on a grid value.
            let value = env
                .call_method(view, "getValue", "()F", &[])
                .and_then(|v| v.f())
                .unwrap_or(lo);
            let snapped = (lo + ((value - lo) / step).round() * step).clamp(lo, hi);
            if snapped != value {
                let _ = env.call_method(view, "setValue", "(F)V", &[JValue::Float(snapped)]);
            }
            let _ = env.call_method(view, "setTickVisible", "(Z)V", &[JValue::Bool(1)]);
        });
    }
    #[cfg(feature = "qt")]
    {
        // Own C++ against the raw QWidget* (src/ticks-qt.cpp) — the bring-your-own recipe.
        // day-qt sliders use a fixed 0..1000 integer range (day-qt-sys shim).
        unsafe extern "C" {
            fn day_tweak_slider_ticks_qt(
                w: *mut std::os::raw::c_void,
                interval: std::os::raw::c_int,
                position: std::os::raw::c_int,
            );
        }
        if let Some(w) = day_qt::with_native_raw(node) {
            let interval = (1000 / (t.count.saturating_sub(1).max(1))) as std::os::raw::c_int;
            let pos = match t.position {
                TickPosition::Below => 0,
                TickPosition::Above => 1,
                TickPosition::Both => 2,
            };
            unsafe { day_tweak_slider_ticks_qt(w, interval, pos) };
            day_core::invalidate_size(node);
        }
        // snap: QSlider has no native snap-to-ticks — documented, not emulated.
    }
    #[cfg(all(feature = "winui", windows))]
    {
        // Own C++/WinRT against the borrowed ABI pointer (src/ticks-winui.cpp).
        unsafe extern "C" {
            fn day_tweak_slider_ticks_winui(
                abi: *mut std::os::raw::c_void,
                count: std::os::raw::c_int,
                position: std::os::raw::c_int,
                snap: std::os::raw::c_int,
            );
        }
        if let Some(abi) = day_winui::with_native_raw(node) {
            let pos = match t.position {
                TickPosition::Below => 0,
                TickPosition::Above => 1,
                TickPosition::Both => 2,
            };
            unsafe {
                day_tweak_slider_ticks_winui(
                    abi,
                    t.count as std::os::raw::c_int,
                    pos,
                    t.snap as std::os::raw::c_int,
                )
            };
            day_core::invalidate_size(node);
        }
    }
    #[cfg(all(feature = "arkui", target_env = "ohos"))]
    {
        // Own C++ against the NDK node handle (src/ticks-arkui.cpp). ArkUI's stepped slider
        // always snaps; `NODE_SLIDER_STEP` is a percentage of the range.
        unsafe extern "C" {
            fn day_tweak_slider_ticks_arkui(
                node: *mut std::os::raw::c_void,
                step_percent: f32,
                show: std::os::raw::c_int,
            );
        }
        if let Some(h) = day_arkui::with_native_raw(node) {
            let step = 100.0 / (t.count.saturating_sub(1).max(1)) as f32;
            unsafe { day_tweak_slider_ticks_arkui(h, step, 1) };
        }
    }
    #[cfg(not(any(
        feature = "appkit",
        feature = "gtk",
        all(feature = "widget", target_os = "android"),
        feature = "qt",
        all(feature = "winui", windows),
        all(feature = "arkui", target_env = "ohos")
    )))]
    let _ = node; // UIKit (and mock): documented no-op — UISlider has no native tick API.
}
