// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The activity piece's C++/WinRT shim, parallel to src/lib-qt-shim.cpp. day-xaml hosts the UWP
// system XAML (DAY_XAML_NS, from the base Windows SDK, not WinAppSDK), so the matching
// spinner is Windows.UI.Xaml.Controls.ProgressRing, whose `IsActive` runs/stops the animation. The
// element is boxed into a day handle via the `day_xaml_box`/`day_xaml_unbox` functions day-xaml-sys
// exports (zero edits to day's toolkit crates), exactly like the media/picker/webview shims.
//
// Written blind (no Windows host here); Windows-only, compiled by build.rs and linked alongside
// day-xaml-sys. ProgressRing is core system XAML so construction can't fail like EdgeHTML, but
// creation still degrades to a TextBlock on any unexpected throw so the app keeps running.

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

#include <windows.h>

using namespace winrt;
namespace WUX = DAY_XAML_NS;
namespace WUXC = DAY_XAML_NS::Controls;

// The boxing functions, exported by day-xaml-sys (already linked into the app).
extern "C" void *day_xaml_box(void *iinspectable_abi);
extern "C" void *day_xaml_unbox(void *handle);

static WUXC::ProgressRing ring_of(void *handle) {
    WUX::UIElement e{nullptr};
    winrt::copy_from_abi(e, day_xaml_unbox(handle));
    if (auto r = e.try_as<WUXC::ProgressRing>())
        return r;
    return nullptr;
}

extern "C" {

void *day_activity_xaml_new(int large, int animating) {
    try {
        WUXC::ProgressRing ring;
        ring.IsActive(animating != 0);
        if (large) {
            ring.Width(48.0);
            ring.Height(48.0);
        }
#ifdef DAY_WINUI
        // System XAML's template sizes the ring itself (20, or the 48 set above); WinUI 3's draws
        // it with an AnimatedVisualPlayer that measures to nothing before the control loads, which
        // is when Day measures it, so the spinner laid out with no frame at all. The floor goes on
        // MinWidth/MinHeight because day-xaml's measure clears Width/Height to Auto first.
        const double side = large ? 48.0 : 20.0;
        ring.MinWidth(side);
        ring.MinHeight(side);
#endif
        return day_xaml_box(winrt::get_abi(ring));
    } catch (...) {
        // Any unexpected failure degrades to a placeholder so the app still runs and screenshots.
        WUXC::TextBlock tb;
        tb.Text(winrt::hstring{L"…"});
        return day_xaml_box(winrt::get_abi(tb));
    }
}

void day_activity_xaml_set_animating(void *handle, int on) {
    try {
        if (auto r = ring_of(handle))
            r.IsActive(on != 0);
    } catch (...) {
    }
}

} // extern "C"
