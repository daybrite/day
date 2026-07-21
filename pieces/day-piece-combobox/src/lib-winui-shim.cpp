// The combo piece's OWN C++/WinRT shim — parallel to src/lib-qt-shim.cpp. An EDITABLE ComboBox
// (IsEditable, Windows 10 1809+): the platform's real combo box (free text + a dropdown of
// items), boxed into a Day handle via the day_winui_box/day_winui_unbox seam that day-winui-sys
// exports, so this piece carries its own WinUI native code with ZERO edits to day's toolkit
// crates.
//
// Change paths back to Rust (each reports the CURRENT text as UTF-8, valid only during the
// callback; Rust copies it):
//   - SelectionChanged → the picked item's string (immediate);
//   - TextSubmitted (Enter) and LostFocus → the free-form text. XAML's ComboBox exposes no
//     per-keystroke text event, so free-form entry commits on those two — a documented
//     divergence from the per-keystroke backends.
// Programmatic setters never echo: Text(...) fires none of the three, and an items swap only
// fires SelectionChanged with nothing selected, which is dropped.
//
// Windows-only; compiled by build.rs (like the Qt shim) and linked alongside day-winui-sys.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h> // IObservableVector Clear/Append on box.Items() — else C3779
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>
#include <winrt/Windows.UI.Xaml.Controls.Primitives.h> // ComboBox's Selector members (SelectedItem / SelectionChanged)

#include <windows.h>

#include <cstdint>
#include <string>

using namespace winrt;
namespace WF = winrt::Windows::Foundation;
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

// Replace the dropdown's items ('\n'-joined). Items are plain boxed strings, so an editable
// combo's Text tracks a pick automatically.
static void combo_fill(WUXC::ComboBox const &box, const char *items_joined) {
    box.Items().Clear();
    std::string all = items_joined ? items_joined : "";
    size_t start = 0;
    while (start <= all.size()) {
        size_t nl = all.find('\n', start);
        std::string item =
            (nl == std::string::npos) ? all.substr(start) : all.substr(start, nl - start);
        if (!item.empty())
            box.Items().Append(winrt::box_value(hs(item.c_str())));
        if (nl == std::string::npos)
            break;
        start = nl + 1;
    }
}

extern "C" {

void *day_combo_winui_new(const char *items_joined, const char *text, const char *placeholder,
                          uint64_t id, void (*cb)(uint64_t, const char *)) {
    WUXC::ComboBox box;
    box.IsEditable(true);
    box.PlaceholderText(hs(placeholder));
    combo_fill(box, items_joined);
    if (text && *text)
        box.Text(hs(text));
    box.SelectionChanged([id, cb](WF::IInspectable const &sender,
                                  WUXC::SelectionChangedEventArgs const &) {
        auto b = sender.as<WUXC::ComboBox>();
        auto sel = b.SelectedItem();
        if (!sel)
            return; // an items swap deselects — not a user change
        if (auto v = sel.try_as<WF::IPropertyValue>()) {
            std::string t = to_utf8(v.GetString());
            cb(id, t.c_str());
        }
    });
    box.TextSubmitted([id, cb](WUXC::ComboBox const &,
                               WUXC::ComboBoxTextSubmittedEventArgs const &args) {
        std::string t = to_utf8(args.Text());
        cb(id, t.c_str());
    });
    box.LostFocus([id, cb](WF::IInspectable const &sender, WUX::RoutedEventArgs const &) {
        if (auto b = sender.try_as<WUXC::ComboBox>()) {
            std::string t = to_utf8(b.Text());
            cb(id, t.c_str());
        }
    });
    return day_winui_box(winrt::get_abi(box));
}

void day_combo_winui_set_items(void *handle, const char *items_joined) {
    WUX::UIElement e{nullptr};
    winrt::copy_from_abi(e, day_winui_unbox(handle));
    if (auto box = e.try_as<WUXC::ComboBox>()) {
        auto keep = box.Text(); // the text is the value; it survives the list swap
        combo_fill(box, items_joined);
        if (box.Text() != keep)
            box.Text(keep);
    }
}

void day_combo_winui_set_text(void *handle, const char *text) {
    WUX::UIElement e{nullptr};
    winrt::copy_from_abi(e, day_winui_unbox(handle));
    if (auto box = e.try_as<WUXC::ComboBox>()) {
        auto nt = hs(text);
        if (box.Text() != nt)
            box.Text(nt);
    }
}

} // extern "C"
