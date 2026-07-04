// The web-view piece's OWN C++/WinRT shim — parallel to src/lib-qt-shim.cpp. day-winui hosts the UWP
// system XAML (winrt::Windows::UI::Xaml, from the base Windows SDK — no WinAppSDK), so the matching
// web view is Windows.UI.Xaml.Controls.WebView. The element is boxed into a day handle via the
// `day_winui_box`/`day_winui_unbox` seam day-winui-sys exports (zero edits to day's toolkit crates).
//
// Written blind (no Windows host here); Windows-only, compiled by build.rs and linked alongside
// day-winui-sys. Creation + navigation are wrapped in try/catch: EdgeHTML WebView can be unavailable
// in an unpackaged Win32 XAML host, so on failure we degrade to a TextBlock showing the URL rather
// than crashing — the app still runs and produces a screenshot.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>

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
        return {};
    int len = WideCharToMultiByte(CP_UTF8, 0, h.c_str(), -1, nullptr, 0, nullptr, nullptr);
    if (len <= 1)
        return {};
    std::string s(static_cast<size_t>(len - 1), '\0');
    WideCharToMultiByte(CP_UTF8, 0, h.c_str(), -1, s.data(), len, nullptr, nullptr);
    return s;
}

static void navigate(WUXC::WebView const &wv, const char *url) {
    if (!url || !*url)
        return;
    try {
        wv.Navigate(WF::Uri{hs(url)});
    } catch (...) {
    }
}

static WUXC::WebView as_webview(void *handle) {
    WUX::UIElement e{nullptr};
    winrt::copy_from_abi(e, day_winui_unbox(handle));
    return e.try_as<WUXC::WebView>();
}

extern "C" {

void *day_webview_winui_new(const char *url, uint64_t id, void (*cb)(uint64_t, const char *)) {
    try {
        WUXC::WebView wv;
        // Report the current URL back on each completed navigation (matches the AppKit/Qt/GTK path).
        wv.NavigationCompleted(
            [id, cb](WUXC::WebView const &, WUXC::WebViewNavigationCompletedEventArgs const &args) {
                try {
                    if (auto uri = args.Uri()) {
                        std::string s = to_utf8(uri.ToString());
                        cb(id, s.c_str());
                    }
                } catch (...) {
                }
            });
        navigate(wv, url);
        return day_winui_box(winrt::get_abi(wv));
    } catch (...) {
        // EdgeHTML WebView unavailable in this host — degrade to a label so the app still runs.
        WUXC::TextBlock tb;
        tb.Text(hs(url ? url : ""));
        return day_winui_box(winrt::get_abi(tb));
    }
}

void day_webview_winui_load(void *handle, const char *url) {
    if (auto wv = as_webview(handle))
        navigate(wv, url);
}
void day_webview_winui_back(void *handle) {
    if (auto wv = as_webview(handle))
        if (wv.CanGoBack())
            wv.GoBack();
}
void day_webview_winui_forward(void *handle) {
    if (auto wv = as_webview(handle))
        if (wv.CanGoForward())
            wv.GoForward();
}
void day_webview_winui_stop(void *handle) {
    if (auto wv = as_webview(handle))
        wv.Stop();
}
void day_webview_winui_reload(void *handle) {
    if (auto wv = as_webview(handle))
        wv.Refresh();
}

} // extern "C"
