//! Tweaks (docs/tweaks.md): access to the WinRT `UIElement` behind a Day-created piece.
//!
//! WinUI is driven through a C++/WinRT shim, and the modern `windows` crate ships no
//! `Windows.UI.Xaml` bindings — so the tweak surface is the element's **borrowed ABI pointer**
//! (`IUIElement*`, via the shim's `day_winui_unbox` seam), paired with the concrete native
//! **class name** Day realized for the node (e.g. `"Slider"`). Rust can't introspect the opaque
//! pointer, so the class is the metadata that lets your C++/WinRT `try_as` the right control — pass
//! it across the FFI and guard the cast. Calling XAML methods means a few lines of your own
//! C++/WinRT, compiled exactly the way `day-piece-picker` compiles its own WinUI shim (see
//! `pieces/day-piece-picker/src/lib-winui-shim.cpp` and the recipe in docs/tweaks.md):
//!
//! ```ignore
//! // src/my_tweak.cpp — link against the same WindowsApp.lib day-winui-sys already links:
//! //   extern "C" void my_slider_ticks(void* abi, const char* cls, double freq) {
//! //       if (!cls || std::strcmp(cls, "Slider") != 0) return;   // told what it is
//! //       winrt::Windows::UI::Xaml::UIElement e{ nullptr };
//! //       winrt::copy_from_abi(e, abi);   // AddRef for the duration of this call
//! //       auto s = e.as<winrt::Windows::UI::Xaml::Controls::Slider>();
//! //       s.TickFrequency(freq);
//! //       s.TickPlacement(winrt::Windows::UI::Xaml::Controls::Primitives::TickPlacement::BottomRight);
//! //   }
//! use day_winui::WinUiExt;
//! slider(v).winui_raw(|abi, class| {
//!     let cls = std::ffi::CString::new(class).unwrap();
//!     unsafe { my_slider_ticks(abi, cls.as_ptr(), 10.0) };
//! });
//! ```
//!
//! Contract: the pointer is BORROWED (not AddRef'd) and valid only while Day's handle lives —
//! `copy_from_abi` in your C++ if you retain it, never Release a ref you didn't take, main
//! thread only. After a size-affecting change, call `day_core::invalidate_size(node)`.

use std::os::raw::c_void;

use day_core::RNode;
use day_pieces::Decorate;
use day_spec::{PieceKind, kinds};

/// The XAML control class Day's shim (`day-winui-sys`) realizes for `kind`. `""` for
/// container/layout kinds with no single leaf control.
fn class_for_kind(kind: Option<PieceKind>) -> &'static str {
    match kind {
        Some(kinds::LABEL) => "TextBlock",
        Some(kinds::BUTTON) => "Button",
        Some(kinds::TOGGLE) => "ToggleSwitch",
        Some(kinds::SLIDER) => "Slider",
        Some(kinds::TEXT_FIELD) => "TextBox",
        Some(kinds::PROGRESS) => "ProgressBar",
        _ => "",
    }
}

/// The borrowed `IUIElement*` ABI pointer behind `node` and its native class name. `None` when the
/// node is layout-only or disposed.
pub fn with_native_raw(node: RNode) -> Option<(*mut c_void, &'static str)> {
    let (handle, kind) = day_core::with_tree(|t| (t.node_handle_any(node), t.node_kind(node)));
    let h = handle?.downcast::<crate::Handle>().ok()?;
    let abi = unsafe { day_winui_sys::day_winui_unbox(h.0) };
    (!abi.is_null()).then_some((abi, class_for_kind(kind)))
}

/// The WinUI tweak modifier: runs once at mount with the borrowed ABI pointer and its class name
/// (docs/tweaks.md).
pub trait WinUiExt: Decorate + Sized {
    fn winui_raw(self, f: impl FnOnce(*mut c_void, &str) + 'static) -> day_core::AnyPiece {
        self.tweak(move |n| {
            if let Some((abi, class)) = with_native_raw(n) {
                f(abi, class);
            }
        })
    }
}

impl<P: Decorate> WinUiExt for P {}
