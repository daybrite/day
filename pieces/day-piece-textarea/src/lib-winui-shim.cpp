// The textarea piece's OWN C++/WinRT shim — parallel to src/lib-qt-shim.cpp. A multi-line TextBox
// (AcceptsReturn = true, TextWrapping = Wrap, a native PlaceholderText), boxed into a Day handle via the
// day_winui_box/day_winui_unbox seam that day-winui-sys exports, so this piece carries its own WinUI
// native code with ZERO edits to day's toolkit crates.
//
// TextChanged reports edits back to Rust as a UTF-8 C string (valid only during the callback; Rust
// copies it). Programmatic Text(...) re-fires TextChanged, but the front-end's bind only re-patches on a
// real change and guards the echo, so there is no runaway loop.
//
// Windows-only; compiled by build.rs (like the Qt shim) and linked alongside day-winui-sys.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>

#include <windows.h>

#include <cstdint>
#include <string>

using namespace winrt;
namespace WUX = winrt::Windows::UI::Xaml;
namespace WUXC = winrt::Windows::UI::Xaml::Controls;

// The boxing seam, exported by day-winui-sys (already linked into the app).
extern "C" void *day_winui_box(void *iinspectable_abi);
extern "C" void *day_winui_unbox(void *handle);

static winrt::hstring hs(const char *s) {
    if (!s || !*s)
        return winrt::hstring{};
    int len = MultiByteToWideChar(CP_UTF8, 0, s, -1, nullptr, 0);
    if (len <= 1)
        return winrt::hstring{};
    std::wstring w(static_cast<size_t>(len - 1), L'\0');
    MultiByteToWideChar(CP_UTF8, 0, s, -1, w.data(), len);
    return winrt::hstring{w};
}

static std::string to_utf8(winrt::hstring const &h) {
    if (h.empty())
        return std::string{};
    int len = WideCharToMultiByte(CP_UTF8, 0, h.c_str(), -1, nullptr, 0, nullptr, nullptr);
    if (len <= 1)
        return std::string{};
    std::string s(static_cast<size_t>(len - 1), '\0');
    WideCharToMultiByte(CP_UTF8, 0, h.c_str(), -1, s.data(), len, nullptr, nullptr);
    return s;
}

extern "C" {

void *day_textarea_winui_new(const char *placeholder, const char *initial, uint64_t id,
                             void (*cb)(uint64_t, const char *)) {
    WUXC::TextBox box;
    box.AcceptsReturn(true);
    box.TextWrapping(WUX::TextWrapping::Wrap);
    box.PlaceholderText(hs(placeholder));
    if (initial && *initial)
        box.Text(hs(initial));
    box.TextChanged([id, cb](winrt::Windows::Foundation::IInspectable const &sender,
                             WUXC::TextChangedEventArgs const &) {
        if (auto tb = sender.try_as<WUXC::TextBox>()) {
            std::string t = to_utf8(tb.Text());
            cb(id, t.c_str());
        }
    });
    return day_winui_box(winrt::get_abi(box));
}

void day_textarea_winui_set_text(void *handle, const char *text) {
    WUX::UIElement e{nullptr};
    winrt::copy_from_abi(e, day_winui_unbox(handle));
    if (auto box = e.try_as<WUXC::TextBox>()) {
        auto nt = hs(text);
        if (box.Text() != nt)
            box.Text(nt);
    }
}

} // extern "C"
