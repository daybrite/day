//! Tweaks (docs/tweaks.md): JNI access to the `android.view.View` behind a Day-created piece.
//!
//! `with_native` clones the `GlobalRef` out of the realized tree and attaches a `JNIEnv` (the
//! crate's own `with_env` helper), so `f` can call any View method:
//!
//! ```ignore
//! use day_android::AndroidExt;
//! use day_android::jni::objects::JValue;
//! label("selectable").android(|view, env| {
//!     let _ = env.call_method(view, "setTextIsSelectable", "(Z)V", &[JValue::Bool(1)]);
//! });
//! ```
//!
//! Day may re-apply *managed* properties on its next patch; unmanaged properties are stable.
//! After a size-affecting change, call `day_core::invalidate_size(node)`.

use day_core::RNode;
use day_pieces::Decorate;
use jni::JNIEnv;
use jni::objects::GlobalRef;

/// Run `f` with the native View's `GlobalRef` and an attached `JNIEnv`. `None` when the node is
/// layout-only or disposed.
pub fn with_native<R>(node: RNode, f: impl FnOnce(&GlobalRef, &mut JNIEnv) -> R) -> Option<R> {
    let h = day_core::with_tree(|t| t.node_handle_any(node))?
        .downcast::<crate::AHandle>()
        .ok()?;
    Some(crate::with_env(|env| f(&h.0, env)))
}

/// The Android tweak modifier: runs once at mount, after the widget exists (docs/tweaks.md).
pub trait AndroidExt: Decorate + Sized {
    fn android(self, f: impl FnOnce(&GlobalRef, &mut JNIEnv) + 'static) -> day_core::AnyPiece {
        self.tweak(move |n| {
            let _ = with_native(n, f);
        })
    }
}

impl<P: Decorate> AndroidExt for P {}
