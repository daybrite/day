// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-piece-colorpicker's OWN C++/WinRT shim — parallel to src/lib-qt-shim.cpp. A swatch `Button`
// whose `Flyout` holds the system `ColorPicker` (Windows.UI.Xaml.Controls, since Windows 10 1703 —
// the spectrum, the channel sliders, the hex field and the alpha channel all come from the OS).
// Elements are boxed into Day handles via the day_xaml_box/day_xaml_unbox seam day-xaml-sys
// exports, so no day toolkit crate is touched.
//
// The swatch is a `Border` filled with a `SolidColorBrush` and captioned with its own hex, which is
// the arrangement `set` walks back down: Button → Content(Border) → Background(SolidColorBrush)
// and Child(TextBlock); and Button → Flyout → Content(ColorPicker). Keeping the state IN the visual
// tree is what lets this shim avoid a pointer-keyed side map (and the address-reuse hazard that
// comes with one) entirely.
//
// Windows-only; compiled by build.rs, built in CI, NOT verified locally. docs/colorpicker.md lists
// what a check on Windows has to confirm.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h> // IVector methods — else C3779
#include <winrt/Windows.UI.h>
#include <winrt/Windows.UI.Text.h>
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>
#include <winrt/Windows.UI.Xaml.Controls.Primitives.h>
#include <winrt/Windows.UI.Xaml.Media.h>

#include <windows.h>

#include <cmath>
#include <cstdint>
#include <string>

using namespace winrt;
namespace WF = winrt::Windows::Foundation;
namespace WU = winrt::Windows::UI;
namespace WUX = winrt::Windows::UI::Xaml;
namespace WUXC = winrt::Windows::UI::Xaml::Controls;
namespace WUXM = winrt::Windows::UI::Xaml::Media;

// The boxing seam, exported by day-xaml-sys (already linked into the app).
extern "C" void *day_xaml_box(void *iinspectable_abi);
extern "C" void *day_xaml_unbox(void *handle);

static uint8_t toByte(double v) {
    if (v < 0.0) v = 0.0;
    if (v > 1.0) v = 1.0;
    return static_cast<uint8_t>(std::lround(v * 255.0));
}

static WU::Color toWinColor(double r, double g, double b, double a) {
    return WU::ColorHelper::FromArgb(toByte(a), toByte(r), toByte(g), toByte(b));
}

static bool sameColor(WU::Color const &x, WU::Color const &y) {
    return x.A == y.A && x.R == y.R && x.G == y.G && x.B == y.B;
}

/// UTF-8 → UTF-16, the same helper every other xaml shim in the tree carries.
static std::wstring wide(const char *s) {
    if (!s || !*s) return std::wstring();
    const int len = MultiByteToWideChar(CP_UTF8, 0, s, -1, nullptr, 0);
    if (len <= 1) return std::wstring();
    std::wstring w(static_cast<size_t>(len - 1), L'\0');
    MultiByteToWideChar(CP_UTF8, 0, s, -1, w.data(), len);
    return w;
}

static std::wstring hexOf(WU::Color const &c, bool withAlpha) {
    wchar_t buf[16];
    if (withAlpha && c.A != 255) {
        swprintf(buf, 16, L"#%02x%02x%02x%02x", c.R, c.G, c.B, c.A);
    } else {
        swprintf(buf, 16, L"#%02x%02x%02x", c.R, c.G, c.B);
    }
    return std::wstring(buf);
}

// Paint the swatch: fill the border, and flip the caption color with the fill's Rec. 709
// luminance so the hex stays readable on a near-black and a near-white pick alike.
static void paintSwatch(WUXC::Border const &border, WU::Color const &c, bool withAlpha) {
    border.Background(WUXM::SolidColorBrush(c));
    if (auto text = border.Child().try_as<WUXC::TextBlock>()) {
        const double lum = (0.2126 * c.R + 0.7152 * c.G + 0.0722 * c.B) / 255.0;
        text.Foreground(WUXM::SolidColorBrush(lum > 0.55 ? WU::Colors::Black() : WU::Colors::White()));
        text.Text(hexOf(c, withAlpha));
    }
}

extern "C" {

void *day_colorpicker_xaml_new(double r, double g, double b, double a, int with_alpha,
                               const char *title, uint64_t id,
                               void (*cb)(uint64_t, double, double, double, double)) {
    const bool alpha = with_alpha != 0;
    const WU::Color value = toWinColor(r, g, b, a);

    WUXC::TextBlock caption;
    caption.Text(hexOf(value, alpha));
    caption.FontFamily(WUXM::FontFamily(L"Consolas"));

    WUXC::Border swatch;
    swatch.CornerRadius(WUX::CornerRadius{4, 4, 4, 4});
    swatch.Padding(WUX::Thickness{10, 4, 10, 4});
    swatch.Child(caption);
    paintSwatch(swatch, value, alpha);

    WUXC::ColorPicker picker;
    picker.IsAlphaEnabled(alpha);
    picker.IsHexInputVisible(true);
    picker.Color(value);
    picker.ColorChanged([id, cb, swatch, alpha](WUXC::ColorPicker const &,
                                                WUXC::ColorChangedEventArgs const &args) {
        const WU::Color c = args.NewColor();
        paintSwatch(swatch, c, alpha);
        cb(id, c.R / 255.0, c.G / 255.0, c.B / 255.0, c.A / 255.0);
    });

    WUXC::Flyout flyout;
    flyout.Content(picker);

    WUXC::Button button;
    button.Content(swatch);
    button.Padding(WUX::Thickness{0, 0, 0, 0});
    button.Flyout(flyout);
    const std::wstring heading = wide(title);
    if (!heading.empty()) {
        // A flyout has no title slot, so the heading rides the button's tooltip — the one place a
        // XAML flyout button can carry explanatory text without inventing chrome for it.
        WUXC::ToolTipService::SetToolTip(button, WF::PropertyValue::CreateString(heading));
    }
    return day_xaml_box(winrt::get_abi(button));
}

void day_colorpicker_xaml_set(void *handle, double r, double g, double b, double a) {
    WF::IInspectable e{nullptr};
    winrt::copy_from_abi(e, day_xaml_unbox(handle));
    auto button = e.try_as<WUXC::Button>();
    if (!button) return;
    const WU::Color value = toWinColor(r, g, b, a);
    if (auto flyout = button.Flyout().try_as<WUXC::Flyout>()) {
        if (auto picker = flyout.Content().try_as<WUXC::ColorPicker>()) {
            // Setting `Color` raises `ColorChanged`, so writing back the value that just arrived
            // FROM the picker would round-trip forever. Comparing first is the whole guard.
            if (sameColor(picker.Color(), value)) return;
            picker.Color(value);
        }
    }
    if (auto swatch = button.Content().try_as<WUXC::Border>()) {
        paintSwatch(swatch, value, value.A != 255);
    }
}

} // extern "C"
