// The remote-image piece's OWN C++/WinRT shim — parallel to src/lib-qt-shim.cpp. day-winui hosts the
// UWP system XAML (winrt::Windows::UI::Xaml, from the base Windows SDK — no WinAppSDK). A circle clip
// uses an Ellipse filled with an ImageBrush (a true circular avatar); a rounded/plain image uses a
// Border (CornerRadius) hosting an Image. The root element is boxed into a day handle via the
// `day_winui_box`/`day_winui_unbox` seam day-winui-sys exports, so this piece carries its own WinUI
// native code with ZERO edits to day's toolkit crates. Bytes are decoded into a BitmapImage from an
// InMemoryRandomAccessStream.
//
// WRITTEN BLIND (no Windows host here) and NOT verified — best-effort so the winui build links in CI.
// Caveats: the byte→BitmapImage decode blocks on StoreAsync().get(), which on an STA UI thread can
// stall; and clearing (None) on the Ellipse path drops the placeholder brush. Both are acceptable
// for the CI-only winui backend and are noted in the crate's caveats. Everything is wrapped in
// try/catch so an unexpected throw degrades to an empty element rather than crashing the app.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Storage.Streams.h>
#include <winrt/Windows.UI.Xaml.Controls.h>
#include <winrt/Windows.UI.Xaml.Media.Imaging.h>
#include <winrt/Windows.UI.Xaml.Media.h>
#include <winrt/Windows.UI.Xaml.Shapes.h>
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.h>

#include <cstdint>

using namespace winrt;
namespace WUX = winrt::Windows::UI::Xaml;
namespace WUXC = winrt::Windows::UI::Xaml::Controls;
namespace WUXM = winrt::Windows::UI::Xaml::Media;
namespace WUXMI = winrt::Windows::UI::Xaml::Media::Imaging;
namespace WUXS = winrt::Windows::UI::Xaml::Shapes;
namespace WSS = winrt::Windows::Storage::Streams;

// The boxing seam, exported by day-winui-sys (already linked into the app).
extern "C" void *day_winui_box(void *iinspectable_abi);
extern "C" void *day_winui_unbox(void *handle);

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

extern "C" {

void *day_remote_image_new(int clip, double radius, int mode, double r, double g, double b,
                           double a) {
    try {
        WUXM::Stretch stretch = (mode == 1) ? WUXM::Stretch::UniformToFill : WUXM::Stretch::Uniform;
        WUXM::SolidColorBrush placeholder{color_of(r, g, b, a)};
        if (clip == 1) {
            // Circle → an Ellipse; the placeholder brush shows until bytes arrive.
            WUXS::Ellipse e;
            e.Fill(placeholder);
            return day_winui_box(winrt::get_abi(e));
        }
        // Rounded / plain → a Border (CornerRadius) hosting an Image over the placeholder.
        WUXC::Border bd;
        if (clip == 2) {
            WUX::CornerRadius cr;
            cr.TopLeft = cr.TopRight = cr.BottomLeft = cr.BottomRight = radius;
            bd.CornerRadius(cr);
        }
        bd.Background(placeholder);
        WUXC::Image img;
        img.Stretch(stretch);
        bd.Child(img);
        return day_winui_box(winrt::get_abi(bd));
    } catch (...) {
        WUXC::Border empty;
        return day_winui_box(winrt::get_abi(empty));
    }
}

void day_remote_image_set_bytes(void *handle, const uint8_t *data, uint64_t len) {
    try {
        WUX::UIElement e{nullptr};
        winrt::copy_from_abi(e, day_winui_unbox(handle));
        WUXMI::BitmapImage bmp = decode(data, len);
        if (auto ell = e.try_as<WUXS::Ellipse>()) {
            if (bmp) {
                WUXM::ImageBrush ib;
                ib.ImageSource(bmp);
                ib.Stretch(WUXM::Stretch::UniformToFill);
                ell.Fill(ib);
            }
            // (Clearing an Ellipse back to the placeholder brush is not supported — see caveats.)
        } else if (auto bd = e.try_as<WUXC::Border>()) {
            if (auto img = bd.Child().try_as<WUXC::Image>()) {
                img.Source(bmp); // nullptr clears → the Border's placeholder background shows
            }
        }
    } catch (...) {
    }
}

} // extern "C"
