//! Tweaks (docs/tweaks.md): typed access to the `NSView` behind a Day-created piece.
//!
//! `with_native` clones the retained handle out of the realized tree (a retain, not a transfer)
//! and hands it to `f` together with the `MainThreadMarker` AppKit calls want. Downcast to the
//! concrete class for widget-specific API:
//!
//! ```ignore
//! use day_appkit::AppKitExt;
//! button("Save").appkit(|view, _mtm| {
//!     if let Some(btn) = view.downcast_ref::<objc2_app_kit::NSButton>() {
//!         unsafe { btn.setBezelStyle(objc2_app_kit::NSBezelStyle::Toolbar) };
//!     }
//! })
//! ```
//!
//! Day may re-apply *managed* properties (title, value, enabled, frame, a11y) on its next patch;
//! unmanaged properties are stable. After a call that changes the widget's intrinsic size, call
//! `day_core::invalidate_size(node)` (the `.tweak` docs cover the rules).

use day_core::RNode;
use day_pieces::Decorate;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::NSView;

/// Run `f` with the native `NSView` behind `node`. `None` when the node is layout-only or
/// disposed (or, defensively, off the main thread).
pub fn with_native<R>(
    node: RNode,
    f: impl FnOnce(&Retained<NSView>, MainThreadMarker) -> R,
) -> Option<R> {
    let mtm = MainThreadMarker::new()?;
    let h = day_core::with_tree(|t| t.node_handle_any(node))?
        .downcast::<crate::Handle>()
        .ok()?;
    Some(f(&h, mtm))
}

/// The AppKit tweak modifier: runs once at mount, after the widget exists (docs/tweaks.md).
pub trait AppKitExt: Decorate + Sized {
    fn appkit(
        self,
        f: impl FnOnce(&Retained<NSView>, MainThreadMarker) + 'static,
    ) -> day_core::AnyPiece {
        self.tweak(move |n| {
            let _ = with_native(n, f);
        })
    }
}

impl<P: Decorate> AppKitExt for P {}
