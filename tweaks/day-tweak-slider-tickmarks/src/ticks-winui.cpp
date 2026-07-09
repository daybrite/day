// The WinUI half of day-tweak-slider-tickmarks — the bring-your-own-C++/WinRT tweak recipe
// (docs/tweaks.md): the borrowed IUIElement* from `day_winui::with_native_raw` is copied into a
// C++/WinRT reference (AddRef for the duration of the call), cast to the concrete Slider, and
// configured. WindowsApp.lib is already linked by day-winui-sys.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>
#include <winrt/Windows.UI.Xaml.Controls.Primitives.h>

namespace WUX = winrt::Windows::UI::Xaml;
namespace WUXC = winrt::Windows::UI::Xaml::Controls;
namespace WUXCP = winrt::Windows::UI::Xaml::Controls::Primitives;

extern "C" void day_tweak_slider_ticks_winui(void* abi, int count, int position, int snap) {
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
        s.SnapsTo(snap ? WUXC::SliderSnapsTo::Ticks : WUXC::SliderSnapsTo::StepValues);
    } catch (...) {
        // Best-effort side effect on one element — a degraded element must not abort the app
        // (same guard rationale as day-winui-sys's FFI seam).
    }
}
