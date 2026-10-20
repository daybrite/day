// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The viewport owns its placeholder, image and clip. Both paints use the same clip; circles
// are centered with diameter min(width, height), including after a resize. The image is
// explicitly constrained to the viewport so UniformToFill crops at its center.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Storage.Streams.h>
// One shim, two XAML stacks (docs/winui.md): windows-xaml builds it against system XAML,
// windows-winui (DAY_WINUI) against WinUI 3. The namespaces differ; the controls do not.
#ifdef DAY_WINUI
#define DAY_XAML_NS winrt::Microsoft::UI::Xaml
#else
#define DAY_XAML_NS winrt::Windows::UI::Xaml
#endif
#ifdef DAY_WINUI
#include <winrt/Microsoft.UI.Xaml.Controls.h>
#include <winrt/Microsoft.UI.Xaml.Hosting.h>
#include <winrt/Microsoft.UI.Composition.h>
#else
#include <winrt/Windows.UI.Xaml.Controls.h>
#include <winrt/Windows.UI.Xaml.Hosting.h>
#include <winrt/Windows.UI.Composition.h>
#endif
#ifdef DAY_WINUI
#include <winrt/Microsoft.UI.Xaml.Media.Imaging.h>
#else
#include <winrt/Windows.UI.Xaml.Media.Imaging.h>
#endif
#ifdef DAY_WINUI
#include <winrt/Microsoft.UI.Xaml.Media.h>
#else
#include <winrt/Windows.UI.Xaml.Media.h>
#endif
#ifdef DAY_WINUI
#include <winrt/Microsoft.UI.Xaml.Shapes.h>
#else
#include <winrt/Windows.UI.Xaml.Shapes.h>
#endif
#ifdef DAY_WINUI
#include <winrt/Microsoft.UI.Xaml.h>
#else
#include <winrt/Windows.UI.Xaml.h>
#endif
#include <winrt/Windows.UI.h>

#include <cstdint>
#include <algorithm>
#include <cmath>

using namespace winrt;
namespace WUX = DAY_XAML_NS;
namespace WUXC = DAY_XAML_NS::Controls;
namespace WUXM = DAY_XAML_NS::Media;
namespace WUXMI = DAY_XAML_NS::Media::Imaging;
namespace WUXS = DAY_XAML_NS::Shapes;
namespace WSS = winrt::Windows::Storage::Streams;

// The boxing functions, exported by day-xaml-sys (already linked into the app).
extern "C" void *day_xaml_box(void *iinspectable_abi);
extern "C" void *day_xaml_unbox(void *handle);

static winrt::Windows::UI::Color color_of(double r, double g, double b, double a) {
    auto u8 = [](double v) -> uint8_t {
        if (v < 0.0)
            v = 0.0;
        if (v > 1.0)
            v = 1.0;
        return static_cast<uint8_t>(v * 255.0 + 0.5);
    };
    winrt::Windows::UI::Color c;
    c.A = u8(a);
    c.R = u8(r);
    c.G = u8(g);
    c.B = u8(b);
    return c;
}

// Decode encoded bytes into a BitmapImage (blocking). Returns nullptr on empty/failure.
static WUXMI::BitmapImage decode(const uint8_t *data, uint64_t len) {
    if (!data || len == 0)
        return nullptr;
    try {
        WSS::InMemoryRandomAccessStream stream;
        WSS::DataWriter writer(stream);
        writer.WriteBytes(winrt::array_view<uint8_t const>(data, data + len));
        writer.StoreAsync().get();
        writer.DetachStream();
        stream.Seek(0);
        WUXMI::BitmapImage bmp;
        bmp.SetSource(stream);
        return bmp;
    } catch (...) {
        return nullptr;
    }
}

static void layout_remote(WUXC::Canvas const& host, int clip, double radius) {
    double w = host.Width(), h = host.Height();
    if (!std::isfinite(w) || !std::isfinite(h) || w < 0 || h < 0) return;
    auto img = host.Children().GetAt(0).as<WUXC::Image>();
    img.Width(w);
    img.Height(h);
    auto visual = DAY_XAML_NS::Hosting::ElementCompositionPreview::GetElementVisual(host);
    auto compositor = visual.Compositor();
    auto geometry = compositor.CreateRoundedRectangleGeometry();
    double cw = w, ch = h;
    if (clip == 1) {
        cw = ch = (std::min)(w, h);
        radius = cw / 2;
    } else if (clip != 2) {
        radius = 0;
    }
    radius = (std::max)(0.0, (std::min)(radius, (std::min)(cw, ch) / 2));
    geometry.Size({static_cast<float>(cw), static_cast<float>(ch)});
    geometry.Offset({static_cast<float>((w - cw) / 2), static_cast<float>((h - ch) / 2)});
    geometry.CornerRadius({static_cast<float>(radius), static_cast<float>(radius)});
    visual.Clip(compositor.CreateGeometricClip(geometry));
}

extern "C" {

void *day_remote_image_new(int clip, double radius, int mode, double r, double g, double b,
                           double a) {
    try {
        WUXC::Canvas host;
        host.Background(WUXM::SolidColorBrush{color_of(r, g, b, a)});
        WUXC::Image img;
        img.Stretch(mode == 1 ? WUXM::Stretch::UniformToFill : WUXM::Stretch::Uniform);
        host.Children().Append(img);
        // Day assigns Width/Height directly. Observe those assignments as well as native
        // layout, so the clip cannot lag behind a resize while waiting for SizeChanged.
        auto changed = [clip, radius](WUX::DependencyObject const& sender,
                                      WUX::DependencyProperty const&) {
            layout_remote(sender.as<WUXC::Canvas>(), clip, radius);
        };
        host.RegisterPropertyChangedCallback(WUX::FrameworkElement::WidthProperty(), changed);
        host.RegisterPropertyChangedCallback(WUX::FrameworkElement::HeightProperty(), changed);
        return day_xaml_box(winrt::get_abi(host));
    } catch (...) {
        WUXC::Border empty;
        return day_xaml_box(winrt::get_abi(empty));
    }
}

void day_remote_image_set_bytes(void *handle, const uint8_t *data, uint64_t len) {
    try {
        WUX::UIElement e{nullptr};
        winrt::copy_from_abi(e, day_xaml_unbox(handle));
        if (auto host = e.try_as<WUXC::Canvas>()) {
            auto img = host.Children().GetAt(0).as<WUXC::Image>();
            // Empty and corrupt data clear the image, exposing the retained placeholder.
            img.Source(decode(data, len));
        }
    } catch (...) {
    }
}

} // extern "C"
