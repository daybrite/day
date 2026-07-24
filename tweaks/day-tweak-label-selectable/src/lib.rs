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
        // Day realizes a plain `label` as a non-editable NSTextField today. Branch on the realized
        // `class` (the metadata the ext hands us) rather than assuming one type: if a future
        // rich-text / link-bearing label backs onto an NSTextView, add an arm here — the tweak
        // won't silently mis-cast, it just no-ops on a backing it doesn't recognize.
        let _ = day_appkit::with_native(node, |view, class, _mtm| {
            if class == "NSTextField"
                && let Some(field) = view.downcast_ref::<objc2_app_kit::NSTextField>()
            {
                field.setSelectable(true);
            }
        });
    }
    #[cfg(feature = "gtk")]
    {
        use gtk4::prelude::*;
        // `class` is "GtkLabel"; the downcast is the guard for this single-backing piece.
        let _ = day_gtk::with_native(node, |w, _class| {
            if let Some(l) = w.downcast_ref::<gtk4::Label>() {
                l.set_selectable(true);
            }
        });
    }
    #[cfg(all(feature = "mdc", target_os = "android"))]
    {
        // Day labels are android.widget.TextView (`class`); JNI through the ext's attached env.
        use day_android::DayEnv;
        use day_android::jni::objects::JValue;
        let _ = day_android::with_native(node, |view, _class, env| {
            let _ = env.dcall(view, "setTextIsSelectable", "(Z)V", &[JValue::Bool(true)]);
        });
    }
    #[cfg(not(any(
        feature = "appkit",
        feature = "gtk",
        all(feature = "mdc", target_os = "android")
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
