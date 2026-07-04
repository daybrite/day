// The picker piece's OWN C++/WinRT shim — parallel to src/lib-qt-shim.cpp. Three stylings behind a flat
// C ABI: 0 = menu (ComboBox), 1 = segmented (horizontal StackPanel of RadioButtons), 2 = inline
// (vertical StackPanel of RadioButtons). The native element is boxed into a day handle via the
// `day_winui_box`/`day_winui_unbox` seam that day-winui-sys exports, so this piece carries its own
// WinUI native code with ZERO edits to day's toolkit crates (the whole point of the seam).
//
// Windows-only; compiled by build.rs (like the Qt shim) and linked alongside day-winui-sys.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>

#include <windows.h>

#include <cstdint>
#include <string>
#include <vector>

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

static std::vector<std::string> split_items(const char *joined) {
    std::vector<std::string> out;
    std::string all = joined ? joined : "";
    size_t start = 0;
    while (true) {
        size_t nl = all.find('\n', start);
        std::string item = all.substr(start, nl == std::string::npos ? std::string::npos : nl - start);
        if (!item.empty())
            out.push_back(item);
        if (nl == std::string::npos)
            break;
        start = nl + 1;
    }
    return out;
}

extern "C" {

void *day_picker_winui_new(int style, const char *items_joined, int selected, uint64_t id,
                           void (*cb)(uint64_t, int)) {
    auto items = split_items(items_joined);
    if (style == 0) {
        WUXC::ComboBox box;
        for (auto &it : items) {
            WUXC::ComboBoxItem cbi;
            cbi.Content(winrt::box_value(hs(it.c_str())));
            box.Items().Append(cbi);
        }
        box.SelectedIndex(selected);
        box.SelectionChanged(
            [id, cb](WF::IInspectable const &s, WUXC::SelectionChangedEventArgs const &) {
                cb(id, s.as<WUXC::ComboBox>().SelectedIndex());
            });
        return day_winui_box(winrt::get_abi(box));
    }
    // Segmented (horizontal) / inline (vertical): RadioButtons sharing a per-instance GroupName so
    // they're mutually exclusive. `Checked` fires on user selection AND programmatic IsChecked, but
    // the front-end's bind only re-patches on a real change, so no runaway loop.
    WUXC::StackPanel panel;
    panel.Orientation(style == 1 ? WUXC::Orientation::Horizontal : WUXC::Orientation::Vertical);
    winrt::hstring group{std::to_wstring(id)}; // per-instance group ⇒ radios are mutually exclusive
    for (size_t i = 0; i < items.size(); i++) {
        WUXC::RadioButton rb;
        rb.Content(winrt::box_value(hs(items[i].c_str())));
        rb.GroupName(group);
        if (static_cast<int>(i) == selected)
            rb.IsChecked(true);
        int idx = static_cast<int>(i);
        rb.Checked([id, cb, idx](WF::IInspectable const &, WUX::RoutedEventArgs const &) {
            cb(id, idx);
        });
        panel.Children().Append(rb);
    }
    return day_winui_box(winrt::get_abi(panel));
}

void day_picker_winui_set_selected(void *handle, int idx) {
    WUX::UIElement e{nullptr};
    winrt::copy_from_abi(e, day_winui_unbox(handle));
    if (auto box = e.try_as<WUXC::ComboBox>()) {
        if (box.SelectedIndex() != idx)
            box.SelectedIndex(idx);
        return;
    }
    if (auto panel = e.try_as<WUXC::Panel>()) {
        auto kids = panel.Children();
        if (idx >= 0 && static_cast<uint32_t>(idx) < kids.Size())
            if (auto rb = kids.GetAt(idx).try_as<WUXC::RadioButton>())
                rb.IsChecked(true);
    }
}

} // extern "C"
