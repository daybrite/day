// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The XAML half of day-tweak-slider-tickmarks — the bring-your-own-C++/WinRT tweak recipe
// (docs/tweaks.md): the borrowed IUIElement* from `day_xaml::with_native_raw` is copied into a
// C++/WinRT reference (AddRef for the duration of the call), cast to the concrete Slider, and
// configured. WindowsApp.lib is already linked by day-xaml-sys.
//
// `cls` is the native class name Day realized for the node (here "Slider"). We confirm it before
// touching the element — `try_as` is already a checked cast, but the class check lets the tweak
// no-op cleanly (and cheaply) when applied to something that isn't a Slider.

#include <winrt/Windows.Foundation.h>
// One shim, two XAML stacks (docs/winui.md): windows-xaml builds it against system XAML,
// windows-winui (DAY_WINUI) against WinUI 3. The namespaces differ; the controls do not.
#ifdef DAY_WINUI
#define DAY_XAML_NS winrt::Microsoft::UI::Xaml
#else
#define DAY_XAML_NS winrt::Windows::UI::Xaml
#endif
#ifdef DAY_WINUI
#include <winrt/Microsoft.UI.Xaml.h>
#else
#include <winrt/Windows.UI.Xaml.h>
#endif
#ifdef DAY_WINUI
#include <winrt/Microsoft.UI.Xaml.Controls.h>
#else
#include <winrt/Windows.UI.Xaml.Controls.h>
#endif
#ifdef DAY_WINUI
#include <winrt/Microsoft.UI.Xaml.Controls.Primitives.h>
#else
#include <winrt/Windows.UI.Xaml.Controls.Primitives.h>
#endif
#include <cstring>

namespace WUX = DAY_XAML_NS;
namespace WUXC = DAY_XAML_NS::Controls;
namespace WUXCP = DAY_XAML_NS::Controls::Primitives;

extern "C" void day_tweak_slider_ticks_xaml(void* abi, const char* cls, int count, int position, int snap) {
    if (!cls || std::strcmp(cls, "Slider") != 0) return;
    try {
        WUX::UIElement e{ nullptr };
        winrt::copy_from_abi(e, abi); // AddRef; released when `e` drops at scope exit
        auto s = e.try_as<WUXC::Slider>();
        if (!s) return;
        double range = s.Maximum() - s.Minimum();
        if (count > 1 && range > 0) s.TickFrequency(range / (count - 1));
        switch (position) {
            case 1: s.TickPlacement(WUXCP::TickPlacement::TopLeft); break;
            case 2: s.TickPlacement(WUXCP::TickPlacement::Outside); break;
            default: s.TickPlacement(WUXCP::TickPlacement::BottomRight); break;
        }
        // SliderSnapsTo lives in Controls.Primitives (like TickPlacement), not plain Controls.
        s.SnapsTo(snap ? WUXCP::SliderSnapsTo::Ticks : WUXCP::SliderSnapsTo::StepValues);
    } catch (...) {
        // Best-effort side effect on one element — a degraded element must not abort the app
        // (same guard rationale as day-xaml-sys's FFI seam).
    }
}
