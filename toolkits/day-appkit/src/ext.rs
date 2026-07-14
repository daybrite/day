//! Tweaks (docs/tweaks.md): typed access to the `NSView` behind a Day-created piece.
//!
//! `with_native` clones the retained handle out of the realized tree (a retain, not a transfer)
//! and hands it to `f` together with the concrete native **class name** and the `MainThreadMarker`
//! AppKit calls want. The class is the realized view's runtime class (`object_getClass`), so a
//! tweak can branch on it instead of guessing — this matters when Day realizes a piece with a
//! *conditional* backing (e.g. a plain `label` as `NSTextField`, a rich-text one as `NSTextView`).
//! Downcast to the concrete class for widget-specific API:
//!
//! ```ignore
//! use day_appkit::AppKitExt;
//! button("Save").appkit(|view, class, _mtm| {
//!     // `class` is "NSButton" here; match on it when a piece has more than one backing.
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

/// Run `f` with the native `NSView` behind `node`, its runtime class name, and the
/// `MainThreadMarker`. `None` when the node is layout-only or disposed (or, defensively, off the
/// main thread).
pub fn with_native<R>(
    node: RNode,
    f: impl FnOnce(&Retained<NSView>, &str, MainThreadMarker) -> R,
) -> Option<R> {
    let mtm = MainThreadMarker::new()?;
    let h = day_core::with_tree(|t| t.node_handle_any(node))?
        .downcast::<crate::Handle>()
        .ok()?;
    // The view's actual runtime class (object_getClass); classes are 'static in the objc runtime.
    let class = h.class().name().to_str().unwrap_or("");
    Some(f(&h, class, mtm))
}

/// The AppKit tweak modifier: runs once at mount, after the widget exists (docs/tweaks.md).
pub trait AppKitExt: Decorate + Sized {
    fn appkit(
        self,
        f: impl FnOnce(&Retained<NSView>, &str, MainThreadMarker) + 'static,
    ) -> day_core::AnyPiece {
        self.tweak(move |n| {
            let _ = with_native(n, f);
        })
    }
}

impl<P: Decorate> AppKitExt for P {}
