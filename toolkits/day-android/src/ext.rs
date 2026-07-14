//! Tweaks (docs/tweaks.md): JNI access to the `android.view.View` behind a Day-created piece.
//!
//! `with_native` hands `f` the View's global reference, the concrete native **class name**, and an
//! attached `Env` (the crate's own `with_env` helper), so `f` can call any View method. jni 0.22
//! takes typed method names and signatures, so use the `jni_str!` / `jni_sig!` compile-time macros
//! (or parse at runtime):
//!
//! ```ignore
//! use day_android::AndroidExt;
//! use day_android::jni::{objects::JValue, signature::RuntimeMethodSignature, strings::JNIString};
//! label("selectable").android(|view, class, env| {
//!     // `class` is "android.widget.TextView" here.
//!     let sig = "(Z)V".parse::<RuntimeMethodSignature>().unwrap();
//!     let _ = env.call_method(view, JNIString::from("setTextIsSelectable"),
//!                             (&sig).into(), &[JValue::Bool(true)]);
//! });
//! ```
//!
//! The class is the Java class Day's `DayBridge` factory realizes for the node's kind. Day may
//! re-apply *managed* properties on its next patch; unmanaged properties are stable. After a
//! size-affecting change, call `day_core::invalidate_size(node)`.

use day_core::RNode;
use day_pieces::Decorate;
use day_spec::{PieceKind, kinds};
use jni::Env;
use jni::objects::{Global, JObject};

/// The Java class `DayBridge` realizes for `kind` (mirrors `DayBridge.make*`). `""` for kinds
/// whose stored handle is a container/wrapper rather than a single leaf widget.
fn class_for_kind(kind: Option<PieceKind>) -> &'static str {
    match kind {
        Some(kinds::LABEL) => "android.widget.TextView",
        Some(kinds::BUTTON) => "com.google.android.material.button.MaterialButton",
        Some(kinds::SLIDER) => "com.google.android.material.slider.Slider",
        Some(kinds::DIVIDER) => "com.google.android.material.divider.MaterialDivider",
        _ => "",
    }
}

/// Run `f` with the native View's global reference, its Java class name, and an attached `Env`.
/// `None` when the node is layout-only or disposed.
pub fn with_native<R>(
    node: RNode,
    f: impl FnOnce(&Global<JObject<'static>>, &str, &mut Env) -> R,
) -> Option<R> {
    let (handle, kind) = day_core::with_tree(|t| (t.node_handle_any(node), t.node_kind(node)));
    let h = handle?.downcast::<crate::AHandle>().ok()?;
    let class = class_for_kind(kind);
    Some(crate::with_env(|env| f(&h.0, class, env)))
}

/// The Android tweak modifier: runs once at mount, after the widget exists (docs/tweaks.md).
pub trait AndroidExt: Decorate + Sized {
    fn android(
        self,
        f: impl FnOnce(&Global<JObject<'static>>, &str, &mut Env) + 'static,
    ) -> day_core::AnyPiece {
        self.tweak(move |n| {
            let _ = with_native(n, f);
        })
    }
}

impl<P: Decorate> AndroidExt for P {}
