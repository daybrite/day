//! Tweaks (docs/tweaks.md): RAW access to the `ArkUI_NodeHandle` behind a Day-created piece.
//!
//! ArkUI is driven through the NDK C API, and the stored handle IS the NDK node pointer — so the
//! tweak surface is that pointer, usable with your own `ArkUI_NativeNodeAPI_1` calls (declare the
//! NDK functions you need `extern "C"`, or add a small C file like `day-arkui-sys` does; recipe
//! in docs/tweaks.md).
//!
//! Contract: the node is owned by Day — never dispose it, never reparent it, main thread only,
//! don't hold the pointer past the call (capture a `NativeRef` and re-resolve instead). After a
//! size-affecting change, call `day_core::invalidate_size(node)`.

use std::os::raw::c_void;

use day_core::RNode;
use day_pieces::Decorate;

/// The raw `ArkUI_NodeHandle` behind `node`. `None` when the node is layout-only or disposed.
pub fn with_native_raw(node: RNode) -> Option<*mut c_void> {
    let h = day_core::with_tree(|t| t.node_handle_any(node))?
        .downcast::<crate::AHandle>()
        .ok()?;
    Some(h.0)
}

/// The ArkUI tweak modifier: runs once at mount with the raw node handle (docs/tweaks.md).
pub trait ArkUiExt: Decorate + Sized {
    fn arkui_raw(self, f: impl FnOnce(*mut c_void) + 'static) -> day_core::AnyPiece {
        self.tweak(move |n| {
            if let Some(w) = with_native_raw(n) {
                f(w);
            }
        })
    }
}

impl<P: Decorate> ArkUiExt for P {}
