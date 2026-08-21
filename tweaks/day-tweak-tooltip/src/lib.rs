// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-tweak-tooltip — a mid-size packaged tweak (docs/tweaks.md): give any piece a native help
//! tooltip, on the three toolkits with a native tooltip affordance and three different access
//! tiers — objc2 (AppKit), gtk4-rs (GTK), and JNI (Android). Everywhere else, `.tooltip(…)` is a
//! documented no-op.
//!
//! ```ignore
//! use day_tweak_tooltip::TooltipTweak;
//! button("Save").tooltip("Save your changes (⌘S)")
//! ```
//!
//! The tooltip shows on hover (macOS/GTK) or long-press (Android). It is an UNMANAGED property
//! (Day never patches it), so it survives Day's own updates to the widget.

use day_pieces::Decorate;

#[cfg(any(
    feature = "appkit",
    feature = "gtk",
    all(feature = "mdc", target_os = "android")
))]
fn apply(node: day_core::RNode, text: &str) {
    #[cfg(feature = "appkit")]
    {
        // `setToolTip:` is on NSView, so this works for any piece's backing view — no downcast.
        let _ = day_appkit::with_native(node, |view, _class, _mtm| {
            view.setToolTip(Some(&objc2_foundation::NSString::from_str(text)));
        });
    }
    #[cfg(feature = "gtk")]
    {
        use gtk4::prelude::*;
        // `set_tooltip_text` is on GtkWidget — every backing widget accepts it.
        let _ = day_gtk::with_native(node, |w, _class| w.set_tooltip_text(Some(text)));
    }
    #[cfg(all(feature = "mdc", target_os = "android"))]
    {
        use day_android::DayEnv;
        use day_android::jni::objects::{JObject, JValue};
        let _ = day_android::with_native(node, |view, _class, env| {
            // View.setTooltipText(CharSequence) — API 26+ (a String IS a CharSequence). Below 26
            // the call throws and the `let _ =` swallows it: a no-op, not a crash.
            if let Ok(js) = env.new_string(text) {
                let s = JObject::from(js);
                let _ = env.dcall(
                    view,
                    "setTooltipText",
                    "(Ljava/lang/CharSequence;)V",
                    &[JValue::Object(&s)],
                );
            }
        });
    }
}

/// `.tooltip(text)` on any piece — a native hover (desktop) / long-press (Android) help string.
pub trait TooltipTweak: Decorate + Sized {
    #[allow(unused_variables)]
    fn tooltip(self, text: impl Into<String>) -> day_pieces::Decorated<Self> {
        let text = text.into();
        #[cfg(any(
            feature = "appkit",
            feature = "gtk",
            all(feature = "mdc", target_os = "android")
        ))]
        {
            self.tweak(move |n| apply(n, &text))
        }
        #[cfg(not(any(
            feature = "appkit",
            feature = "gtk",
            all(feature = "mdc", target_os = "android")
        )))]
        {
            // Documented no-op on toolkits this tweak doesn't cover.
            let _ = text;
            self.tweak(|_| {})
        }
    }
}

impl<P: Decorate> TooltipTweak for P {}
