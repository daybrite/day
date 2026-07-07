// The activity piece's OWN C++/WinRT shim — parallel to src/lib-qt-shim.cpp. day-winui hosts the UWP
// system XAML (winrt::Windows::UI::Xaml, from the base Windows SDK — no WinAppSDK), so the matching
// spinner is Windows.UI.Xaml.Controls.ProgressRing, whose `IsActive` runs/stops the animation. The
// element is boxed into a day handle via the `day_winui_box`/`day_winui_unbox` seam day-winui-sys
// exports (zero edits to day's toolkit crates), exactly like the media/picker/webview shims.
//
// Written blind (no Windows host here); Windows-only, compiled by build.rs and linked alongside
// day-winui-sys. ProgressRing is core system XAML so construction can't fail like EdgeHTML, but
// creation still degrades to a TextBlock on any unexpected throw so the app keeps running.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>

#include <windows.h>

using namespace winrt;
namespace WUX = winrt::Windows::UI::Xaml;
namespace WUXC = winrt::Windows::UI::Xaml::Controls;

// The boxing seam, exported by day-winui-sys (already linked into the app).
extern "C" void *day_winui_box(void *iinspectable_abi);
extern "C" void *day_winui_unbox(void *handle);

static WUXC::ProgressRing ring_of(void *handle) {
    WUX::UIElement e{nullptr};
    winrt::copy_from_abi(e, day_winui_unbox(handle));
    if (auto r = e.try_as<WUXC::ProgressRing>())
        return r;
    return nullptr;
}

extern "C" {

void *day_activity_winui_new(int large, int animating) {
    try {
        WUXC::ProgressRing ring;
        ring.IsActive(animating != 0);
        if (large) {
            ring.Width(48.0);
            ring.Height(48.0);
        }
        return day_winui_box(winrt::get_abi(ring));
    } catch (...) {
        // Any unexpected failure — degrade to a placeholder so the app still runs and screenshots.
        WUXC::TextBlock tb;
        tb.Text(winrt::hstring{L"…"});
        return day_winui_box(winrt::get_abi(tb));
    }
}

void day_activity_winui_set_animating(void *handle, int on) {
    try {
        if (auto r = ring_of(handle))
            r.IsActive(on != 0);
    } catch (...) {
    }
}

} // extern "C"
