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
    all(feature = "material", target_os = "android")
))]
fn apply(node: day_core::RNode, text: &str) {
    #[cfg(feature = "appkit")]
    {
        // `setToolTip:` is on NSView, so this works for any piece's backing view — no downcast.
        let _ = day_toolkit_appkit::with_native(node, |view, _class, _mtm| {
            view.setToolTip(Some(&objc2_foundation::NSString::from_str(text)));
        });
    }
    #[cfg(feature = "gtk")]
    {
        use gtk4::prelude::*;
        // `set_tooltip_text` is on GtkWidget — every backing widget accepts it.
        let _ = day_toolkit_gtk::with_native(node, |w, _class| w.set_tooltip_text(Some(text)));
    }
    #[cfg(all(feature = "material", target_os = "android"))]
    {
        // View.setTooltipText(CharSequence) arrived in API 26; below it there is nothing to set.
        if day_toolkit_material::sdk_int() >= 26 {
            let _ = day_toolkit_material::with_native(node, |view, _class, env| {
                day_toolkit_material::java_in(env, |env| {
                    let text = day_toolkit_material::jni::objects::JString::from_str(env, text)?;
                    day_toolkit_material::downcast::<android::View>(env, view)?
                        .setTooltipText(env, &text)
                });
            });
        }
    }
}

/// `android.view.View`'s tooltip setter, the one call this tweak makes (a String IS a
/// CharSequence).
#[cfg(all(feature = "material", target_os = "android"))]
mod android {
    day_toolkit_material::java_class! {
        pub View => android.view.View,
        since = 26,
        methods {
            fn setTooltipText(text: java.lang.CharSequence),
        },
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
            all(feature = "material", target_os = "android")
        ))]
        {
            self.tweak(move |n| apply(n, &text))
        }
        #[cfg(not(any(
            feature = "appkit",
            feature = "gtk",
            all(feature = "material", target_os = "android")
        )))]
        {
            // Documented no-op on toolkits this tweak doesn't cover.
            let _ = text;
            self.tweak(|_| {})
        }
    }
}

impl<P: Decorate> TooltipTweak for P {}
