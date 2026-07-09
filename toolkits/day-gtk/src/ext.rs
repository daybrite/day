//! Tweaks (docs/tweaks.md): typed access to the `gtk4::Widget` behind a Day-created piece.
//!
//! `with_native` clones the widget handle (a gobject ref) out of the realized tree. Downcast for
//! widget-specific API:
//!
//! ```ignore
//! use day_gtk::GtkExt;
//! use gtk4::prelude::*;
//! label("Legal text").gtk(|w| {
//!     if let Some(l) = w.downcast_ref::<gtk4::Label>() {
//!         l.set_selectable(true);
//!     }
//! });
//! ```
//!
//! Day may re-apply *managed* properties (title, value, enabled, frame, a11y) on its next patch;
//! unmanaged properties are stable. After a call that changes the widget's intrinsic size, call
//! `day_core::invalidate_size(node)`.

use day_core::RNode;
use day_pieces::Decorate;

/// Run `f` with the native `gtk4::Widget` behind `node`. `None` when the node is layout-only or
/// disposed.
pub fn with_native<R>(node: RNode, f: impl FnOnce(&gtk4::Widget) -> R) -> Option<R> {
    let h = day_core::with_tree(|t| t.node_handle_any(node))?
        .downcast::<crate::Handle>()
        .ok()?;
    Some(f(&h))
}

/// The GTK tweak modifier: runs once at mount, after the widget exists (docs/tweaks.md).
pub trait GtkExt: Decorate + Sized {
    fn gtk(self, f: impl FnOnce(&gtk4::Widget) + 'static) -> day_core::AnyPiece {
        self.tweak(move |n| {
            let _ = with_native(n, f);
        })
    }
}

impl<P: Decorate> GtkExt for P {}
