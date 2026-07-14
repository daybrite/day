//! day-tweak-label-selectable — a mid-size packaged tweak (docs/tweaks.md): make a label's text
//! user-selectable, on the three toolkits with three different access tiers — objc2 (AppKit),
//! gtk4-rs (GTK), and JNI (Android). Everywhere else, `.selectable()` is a documented no-op.
//!
//! ```ignore
//! use day_tweak_label_selectable::LabelSelectableTweak;
//! label("You can copy this").selectable()
//! ```
//!
//! Selectability is an UNMANAGED property (Day never patches it), so it survives Day's own text
//! updates to the label. Selection visuals and shortcuts are the platform's own.

use day_core::RNode;
use day_pieces::Decorate;

fn apply(node: RNode) {
    #[cfg(feature = "appkit")]
    {
        // Day labels are non-editable NSTextFields; selectable text is one setter away.
        let _ = day_appkit::with_native(node, |view, _mtm| {
            if let Some(field) = view.downcast_ref::<objc2_app_kit::NSTextField>() {
                field.setSelectable(true);
            }
        });
    }
    #[cfg(feature = "gtk")]
    {
        use gtk4::prelude::*;
        let _ = day_gtk::with_native(node, |w| {
            if let Some(l) = w.downcast_ref::<gtk4::Label>() {
                l.set_selectable(true);
            }
        });
    }
    #[cfg(all(feature = "widget", target_os = "android"))]
    {
        // Day labels are android.widget.TextView; JNI through the ext module's attached env.
        use day_android::jni::objects::JValue;
        use day_android::DayEnv;
        let _ = day_android::with_native(node, |view, env| {
            let _ = env.dcall(view, "setTextIsSelectable", "(Z)V", &[JValue::Bool(true)]);
        });
    }
    #[cfg(not(any(
        feature = "appkit",
        feature = "gtk",
        all(feature = "widget", target_os = "android")
    )))]
    let _ = node; // documented no-op on toolkits this tweak doesn't cover
}

/// `.selectable()` on any piece whose native widget is a text label (i.e. `label(…)`).
pub trait LabelSelectableTweak: Decorate + Sized {
    fn selectable(self) -> day_core::AnyPiece {
        self.tweak(apply)
    }
}

impl<P: Decorate> LabelSelectableTweak for P {}
