//! Tweaks (docs/tweaks.md): typed access to the `UIView` behind a Day-created piece.
//!
//! Same shape as day-appkit's ext: `with_native` clones the retained handle (a retain, not a
//! transfer) and hands it to `f` with the concrete native **class name** and the
//! `MainThreadMarker`. The class is the realized view's runtime class (`object_getClass`), so a
//! tweak can branch on it — this matters for a piece with a *conditional* backing (e.g. a plain
//! `label` as `UILabel`, a link-bearing one as `UITextView`). Downcast for widget-specific API:
//!
//! ```ignore
//! use day_uikit::UiKitExt;
//! label("selectable").uikit(|view, class, _mtm| {
//!     match class {
//!         "UILabel" => { if let Some(l) = view.downcast_ref::<objc2_ui_kit::UILabel>() { /* … */ } }
//!         "UITextView" => { /* the rich/link backing */ }
//!         _ => {}
//!     }
//! });
//! ```
//!
//! Day may re-apply *managed* properties on its next patch; unmanaged properties are stable.
//! After a size-affecting change, call `day_core::invalidate_size(node)`.

use day_core::RNode;
use day_pieces::Decorate;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_ui_kit::UIView;

/// Run `f` with the native `UIView` behind `node`, its runtime class name, and the
/// `MainThreadMarker`. `None` when the node is layout-only or disposed (or, defensively, off the
/// main thread).
pub fn with_native<R>(
    node: RNode,
    f: impl FnOnce(&Retained<UIView>, &str, MainThreadMarker) -> R,
) -> Option<R> {
    let mtm = MainThreadMarker::new()?;
    let h = day_core::with_tree(|t| t.node_handle_any(node))?
        .downcast::<crate::Handle>()
        .ok()?;
    // The view's actual runtime class (object_getClass); classes are 'static in the objc runtime.
    let class = h.class().name().to_str().unwrap_or("");
    Some(f(&h, class, mtm))
}

/// The UIKit tweak modifier: runs once at mount, after the widget exists (docs/tweaks.md).
pub trait UiKitExt: Decorate + Sized {
    fn uikit(
        self,
        f: impl FnOnce(&Retained<UIView>, &str, MainThreadMarker) + 'static,
    ) -> day_core::AnyPiece {
        self.tweak(move |n| {
            let _ = with_native(n, f);
        })
    }
}

impl<P: Decorate> UiKitExt for P {}
