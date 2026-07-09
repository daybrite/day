//! Tweaks (docs/tweaks.md): RAW access to the `QWidget*` behind a Day-created piece.
//!
//! Qt is a C++ toolkit behind a C shim, so there is no typed Rust widget to hand out — the tweak
//! surface is the raw `QWidget*` itself. Calling Qt methods on it means writing a few lines of
//! your own C++, compiled the same way `day-qt-sys` compiles its shim (`cc` + `pkg-config`, see
//! the recipe in docs/tweaks.md):
//!
//! ```ignore
//! // build.rs: cc::Build::new().cpp(true).file("src/my_tweak.cpp") + Qt6Widgets cflags
//! // src/my_tweak.cpp:
//! //   extern "C" void my_slider_ticks(void* w, int interval) {
//! //       auto* s = static_cast<QSlider*>(w);
//! //       s->setTickPosition(QSlider::TicksBelow); s->setTickInterval(interval);
//! //   }
//! use day_qt::QtExt;
//! slider(v).qt_raw(|w| unsafe { my_slider_ticks(w, 10) });
//! ```
//!
//! Contract: the pointer is owned by Day/Qt — never `delete` it, never reparent it, use it only
//! on the main thread, and don't hold it past the call (capture a `NativeRef` and re-resolve
//! instead). After a size-affecting change, call `day_core::invalidate_size(node)`.

use std::os::raw::c_void;

use day_core::RNode;
use day_pieces::Decorate;

/// The raw `QWidget*` behind `node`. `None` when the node is layout-only or disposed.
pub fn with_native_raw(node: RNode) -> Option<*mut c_void> {
    let h = day_core::with_tree(|t| t.node_handle_any(node))?
        .downcast::<crate::Handle>()
        .ok()?;
    Some(h.0)
}

/// The Qt tweak modifier: runs once at mount with the raw `QWidget*` (docs/tweaks.md).
pub trait QtExt: Decorate + Sized {
    fn qt_raw(self, f: impl FnOnce(*mut c_void) + 'static) -> day_core::AnyPiece {
        self.tweak(move |n| {
            if let Some(w) = with_native_raw(n) {
                f(w);
            }
        })
    }
}

impl<P: Decorate> QtExt for P {}
