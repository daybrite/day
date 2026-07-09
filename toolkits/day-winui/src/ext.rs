//! Tweaks (docs/tweaks.md): access to the WinRT `UIElement` behind a Day-created piece.
//!
//! WinUI is driven through a C++/WinRT shim, and the modern `windows` crate ships no
//! `Windows.UI.Xaml` bindings — so the tweak surface is the element's **borrowed ABI pointer**
//! (`IUIElement*`, via the shim's `day_winui_unbox` seam), and calling XAML methods on it means a
//! few lines of your own C++/WinRT, compiled exactly the way `day-piece-picker` compiles its own
//! WinUI shim (see `pieces/day-piece-picker/src/lib-winui-shim.cpp` and the recipe in
//! docs/tweaks.md):
//!
//! ```ignore
//! // src/my_tweak.cpp — link against the same WindowsApp.lib day-winui-sys already links:
//! //   extern "C" void my_slider_ticks(void* abi, double freq) {
//! //       winrt::Windows::UI::Xaml::UIElement e{ nullptr };
//! //       winrt::copy_from_abi(e, abi);   // AddRef for the duration of this call
//! //       auto s = e.as<winrt::Windows::UI::Xaml::Controls::Slider>();
//! //       s.TickFrequency(freq);
//! //       s.TickPlacement(winrt::Windows::UI::Xaml::Controls::Primitives::TickPlacement::BottomRight);
//! //   }
//! use day_winui::WinUiExt;
//! slider(v).winui_raw(|abi| unsafe { my_slider_ticks(abi, 10.0) });
//! ```
//!
//! Contract: the pointer is BORROWED (not AddRef'd) and valid only while Day's handle lives —
//! `copy_from_abi` in your C++ if you retain it, never Release a ref you didn't take, main
//! thread only. After a size-affecting change, call `day_core::invalidate_size(node)`.

use std::os::raw::c_void;

use day_core::RNode;
use day_pieces::Decorate;

/// The borrowed `IUIElement*` ABI pointer behind `node`. `None` when the node is layout-only or
/// disposed.
pub fn with_native_raw(node: RNode) -> Option<*mut c_void> {
    let h = day_core::with_tree(|t| t.node_handle_any(node))?
        .downcast::<crate::Handle>()
        .ok()?;
    let abi = unsafe { day_winui_sys::day_winui_unbox(h.0) };
    (!abi.is_null()).then_some(abi)
}

/// The WinUI tweak modifier: runs once at mount with the borrowed ABI pointer (docs/tweaks.md).
pub trait WinUiExt: Decorate + Sized {
    fn winui_raw(self, f: impl FnOnce(*mut c_void) + 'static) -> day_core::AnyPiece {
        self.tweak(move |n| {
            if let Some(abi) = with_native_raw(n) {
                f(abi);
            }
        })
    }
}

impl<P: Decorate> WinUiExt for P {}
