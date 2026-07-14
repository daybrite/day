//! Tweaks (docs/tweaks.md): RAW access to the `ArkUI_NodeHandle` behind a Day-created piece.
//!
//! ArkUI is driven through the NDK C API, and the stored handle IS the NDK node pointer — so the
//! tweak surface is that pointer, paired with the concrete native **node type name** Day realized
//! for the node (e.g. `"Slider"`, matching `ARKUI_NODE_SLIDER`). Rust can't introspect the opaque
//! handle, so the class is the metadata that lets your C++ act on the right node type — pass it
//! across the FFI and guard. Use the pointer with your own `ArkUI_NativeNodeAPI_1` calls (declare
//! the NDK functions you need `extern "C"`, or add a small C file like `day-arkui-sys` does;
//! recipe in docs/tweaks.md).
//!
//! Contract: the node is owned by Day — never dispose it, never reparent it, main thread only,
//! don't hold the pointer past the call (capture a `NativeRef` and re-resolve instead). After a
//! size-affecting change, call `day_core::invalidate_size(node)`.

use std::os::raw::c_void;

use day_core::RNode;
use day_pieces::Decorate;
use day_spec::{PieceKind, kinds};

/// The ArkUI node type Day's shim (`day-arkui-sys`) realizes for `kind`, named after the
/// `ARKUI_NODE_*` constant. `""` for container/layout kinds with no single leaf node type.
fn class_for_kind(kind: Option<PieceKind>) -> &'static str {
    match kind {
        Some(kinds::LABEL) => "Text",
        Some(kinds::BUTTON) => "Button",
        Some(kinds::TOGGLE) => "Toggle",
        Some(kinds::SLIDER) => "Slider",
        Some(kinds::TEXT_FIELD) => "TextInput",
        Some(kinds::PROGRESS) => "Progress",
        _ => "",
    }
}

/// The raw `ArkUI_NodeHandle` behind `node` and its node type name. `None` when the node is
/// layout-only or disposed.
pub fn with_native_raw(node: RNode) -> Option<(*mut c_void, &'static str)> {
    let (handle, kind) = day_core::with_tree(|t| (t.node_handle_any(node), t.node_kind(node)));
    let h = handle?.downcast::<crate::AHandle>().ok()?;
    Some((h.0, class_for_kind(kind)))
}

/// The ArkUI tweak modifier: runs once at mount with the raw node handle and its type name
/// (docs/tweaks.md).
pub trait ArkUiExt: Decorate + Sized {
    fn arkui_raw(self, f: impl FnOnce(*mut c_void, &str) + 'static) -> day_core::AnyPiece {
        self.tweak(move |n| {
            if let Some((w, class)) = with_native_raw(n) {
                f(w, class);
            }
        })
    }
}

impl<P: Decorate> ArkUiExt for P {}
