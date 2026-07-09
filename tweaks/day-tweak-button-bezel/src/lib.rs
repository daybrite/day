//! day-tweak-button-bezel — the smallest possible packaged tweak (docs/tweaks.md): symbolic
//! constants for AppKit's `NSButton` bezel styles, applied through `day_appkit::with_native`.
//!
//! ```ignore
//! use day_tweak_button_bezel::{Bezel, ButtonBezelTweak};
//! button("Save").bezel(Bezel::Toolbar)
//! ```
//!
//! The tweak crate owns the platform gate: on every toolkit except AppKit, `.bezel(…)` is a
//! documented no-op passthrough, so app code needs no `#[cfg]`. Bezel style is an UNMANAGED
//! property (Day never patches it), so it survives Day's own updates to the button.

use day_pieces::Decorate;

/// The bezel to give an AppKit button (modern `NSBezelStyle` names, macOS 11+).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bezel {
    /// The standard push button (the default look).
    Push,
    /// A push button that grows vertically with its content.
    FlexiblePush,
    /// The borderless toolbar-item style.
    Toolbar,
    /// The recessed accessory-bar style.
    AccessoryBar,
    /// The unbordered accessory-bar action style.
    AccessoryBarAction,
    /// A circular bezel (single-character or image buttons).
    Circular,
    /// The badge style (inline counts).
    Badge,
    /// A small square bezel that tiles cleanly.
    SmallSquare,
}

#[cfg(feature = "appkit")]
fn ns_bezel(b: Bezel) -> objc2_app_kit::NSBezelStyle {
    use objc2_app_kit::NSBezelStyle as S;
    match b {
        Bezel::Push => S::Push,
        Bezel::FlexiblePush => S::FlexiblePush,
        Bezel::Toolbar => S::Toolbar,
        Bezel::AccessoryBar => S::AccessoryBar,
        Bezel::AccessoryBarAction => S::AccessoryBarAction,
        Bezel::Circular => S::Circular,
        Bezel::Badge => S::Badge,
        Bezel::SmallSquare => S::SmallSquare,
    }
}

/// `.bezel(…)` on any piece whose native widget is an `NSButton` (i.e. `button(…)`).
pub trait ButtonBezelTweak: Decorate + Sized {
    #[allow(unused_variables)]
    fn bezel(self, bezel: Bezel) -> day_core::AnyPiece {
        #[cfg(feature = "appkit")]
        {
            self.tweak(move |n| {
                let _ = day_appkit::with_native(n, |view, _mtm| {
                    if let Some(btn) = view.downcast_ref::<objc2_app_kit::NSButton>() {
                        btn.setBezelStyle(ns_bezel(bezel));
                        // Bezels have different intrinsic metrics — let layout re-measure.
                        day_core::invalidate_size(n);
                    }
                });
            })
        }
        #[cfg(not(feature = "appkit"))]
        {
            // Documented no-op on non-AppKit toolkits: the button stays stock.
            self.tweak(|_| {})
        }
    }
}

impl<P: Decorate> ButtonBezelTweak for P {}
