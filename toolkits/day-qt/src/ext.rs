//! Tweaks (docs/tweaks.md): RAW access to the `QWidget*` behind a Day-created piece.
//!
//! Qt is a C++ toolkit behind a C shim, so there is no typed Rust widget to hand out — the tweak
//! surface is the raw `QWidget*` itself, paired with the concrete native **class name** Day
//! realized for the node (e.g. `"QSlider"`). Rust can't introspect the opaque pointer, so the
//! class is the piece of metadata that lets your C++ cast it *knowingly* — pass it across the FFI
//! and guard the cast. Calling Qt methods on the pointer means writing a few lines of your own
//! C++, compiled the same way `day-qt-sys` compiles its shim (`cc` + `pkg-config`, see the recipe
//! in docs/tweaks.md):
//!
//! ```ignore
//! // build.rs: cc::Build::new().cpp(true).file("src/my_tweak.cpp") + Qt6Widgets cflags
//! // src/my_tweak.cpp:
//! //   extern "C" void my_slider_ticks(void* w, const char* cls, int interval) {
//! //       if (!w || !cls || std::strcmp(cls, "QSlider") != 0) return;  // told what it is
//! //       auto* s = static_cast<QSlider*>(w);
//! //       s->setTickPosition(QSlider::TicksBelow); s->setTickInterval(interval);
//! //   }
//! use day_qt::QtExt;
//! slider(v).qt_raw(|w, class| {
//!     let cls = std::ffi::CString::new(class).unwrap();
//!     unsafe { my_slider_ticks(w, cls.as_ptr(), 10) };
//! });
//! ```
//!
//! Contract: the pointer is owned by Day/Qt — never `delete` it, never reparent it, use it only
//! on the main thread, and don't hold it past the call (capture a `NativeRef` and re-resolve
//! instead). After a size-affecting change, call `day_core::invalidate_size(node)`.

use std::os::raw::c_void;

use day_core::RNode;
use day_pieces::Decorate;
use day_spec::{PieceKind, kinds};

/// The Qt widget class Day's shim (`day-qt-sys`) realizes for `kind`. `""` for container/layout
/// kinds with no single leaf widget.
fn class_for_kind(kind: Option<PieceKind>) -> &'static str {
    match kind {
        Some(kinds::LABEL) => "QLabel",
        Some(kinds::BUTTON) => "QPushButton",
        Some(kinds::TOGGLE) => "QCheckBox",
        Some(kinds::SLIDER) => "QSlider",
        Some(kinds::TEXT_FIELD) => "QLineEdit",
        Some(kinds::PROGRESS) => "QProgressBar",
        _ => "",
    }
}

/// The raw `QWidget*` behind `node` and its native class name. `None` when the node is layout-only
/// or disposed.
pub fn with_native_raw(node: RNode) -> Option<(*mut c_void, &'static str)> {
    let (handle, kind) = day_core::with_tree(|t| (t.node_handle_any(node), t.node_kind(node)));
    let h = handle?.downcast::<crate::Handle>().ok()?;
    Some((h.0, class_for_kind(kind)))
}

/// The Qt tweak modifier: runs once at mount with the raw `QWidget*` and its class name
/// (docs/tweaks.md).
pub trait QtExt: Decorate + Sized {
    fn qt_raw(self, f: impl FnOnce(*mut c_void, &str) + 'static) -> day_core::AnyPiece {
        self.tweak(move |n| {
            if let Some((w, class)) = with_native_raw(n) {
                f(w, class);
            }
        })
    }
}

impl<P: Decorate> QtExt for P {}
