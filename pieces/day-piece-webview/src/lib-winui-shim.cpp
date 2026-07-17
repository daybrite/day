// The web-view piece's OWN C++/WinRT shim — parallel to src/lib-qt-shim.cpp.
//
// day-winui hosts UWP system XAML (winrt::Windows::UI::Xaml, base Windows SDK, no WinAppSDK) inside a
// Win32 window via XAML Islands. The system-XAML web view, Windows.UI.Xaml.Controls.WebView (EdgeHTML),
// is UNSUPPORTED in that host: it renders blank, never raises NavigationCompleted, and crashes on
// navigation. The supported engine is WebView2, hosted here in WINDOWLESS / VISUAL-HOSTING mode — the
// same technique the official WinUI WebView2 controls use internally:
//
//   * make() boxes a plain XAML Border (transparent, hit-testable, with a faint URL label) as the day
//     handle. day lays it out like any leaf.
//   * A CoreWebView2CompositionController renders the page into a Windows.UI.Composition Visual instead
//     of its own HWND. We splice that visual into the XAML tree with ElementCompositionPreview::
//     SetElementChildVisual(Border, visual) — so the web view is a REAL node in the XAML visual tree:
//     correct z-order, clipping, DPI and layout, no separate window to track, no airspace.
//   * Input: a raw child HWND over the XAML island gets no mouse input, because the island's
//     ContentIsland InputSite owns pointer input for its whole surface. Windowless hosting turns that
//     around — the InputSite delivers pointer events to the Border (XAML), and we FORWARD them to the
//     controller's SendMouseInput. So clicks/scroll/drag work, routed through XAML's own input.
//   * The browser's lifetime follows the Border's tree membership (Unloaded → detach visual + Close).
//   * If the WebView2 Runtime is absent, controller creation fails and the Border's URL label remains
//     as a graceful, no-crash fallback.
//
// WebView2LoaderStatic.lib is statically linked by build.rs (no DLL to bundle); the WebView2 Runtime
// is a system-wide install present on Windows 11 and the CI runners.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.UI.h>             // Color (transparent, hit-testable Border background)
#include <winrt/Windows.UI.Composition.h> // Compositor, ContainerVisual (the render target)
#include <winrt/Windows.UI.Input.h>       // PointerPoint(Properties), PointerUpdateKind
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>
#include <winrt/Windows.UI.Xaml.Hosting.h> // ElementCompositionPreview (splice visual into the tree)
#include <winrt/Windows.UI.Xaml.Input.h>   // PointerRoutedEventArgs
#include <winrt/Windows.UI.Xaml.Media.h>

#include <windows.h>
#include <wrl.h>
#include <wrl/event.h>
#include <WebView2.h>
#include <WebView2EnvironmentOptions.h>

#include <cmath>
#include <cstdint>
#include <map>
#include <string>

using namespace winrt;
namespace WF = winrt::Windows::Foundation;
namespace WUI = winrt::Windows::UI;
namespace WUC = winrt::Windows::UI::Composition;
namespace WUInput = winrt::Windows::UI::Input;
namespace WUX = winrt::Windows::UI::Xaml;
namespace WUXC = winrt::Windows::UI::Xaml::Controls;
namespace WUXH = winrt::Windows::UI::Xaml::Hosting;
namespace WUXI = winrt::Windows::UI::Xaml::Input;
namespace WUXM = winrt::Windows::UI::Xaml::Media;
namespace wrl = Microsoft::WRL;

// Seams exported by day-winui-sys (already linked into the app). The host HWND is the composition
// controller's parentWindow (for DPI / IME / input association) — the page still renders windowless.
extern "C" void *day_winui_box(void *iinspectable_abi);
extern "C" void *day_winui_unbox(void *handle);
extern "C" void *day_winui_host_hwnd();

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

// Per-web-view state. Keyed by the day handle (the boxed Border) so async callbacks and later
// operations find it, and a callback that outlives teardown is a safe no-op (find returns null).
struct WebViewCtx {
    HWND parent{}; // host window — the composition controller's parentWindow
    wrl::ComPtr<ICoreWebView2CompositionController> compositionController; // SendMouseInput, visual
    wrl::ComPtr<ICoreWebView2Controller> controller; // Bounds, IsVisible, focus, Close (same object)
    wrl::ComPtr<ICoreWebView2> webview;
    WUXC::Border placeholder{nullptr}; // XAML host element; the render visual is its child
    WUC::ContainerVisual rootVisual{nullptr};
    uint64_t id{};
    void (*cb)(uint64_t, const char *){};
    std::wstring pending_url; // navigated once the controller is ready
    double scale{1.0};        // DIP → physical-pixel factor (host-window DPI / 96), the rasterization scale
};

static std::map<void *, WebViewCtx *> g_webviews;

static WebViewCtx *find_ctx(void *handle) {
    auto it = g_webviews.find(handle);
    return it == g_webviews.end() ? nullptr : it->second;
}

// Match the render visual + controller Bounds to the Border's current size. BoundsMode is
// UseRasterizationScale, so Bounds/visual are in DIPs and RasterizationScale carries the DPI. The
// visual follows the element's position/clipping/transforms automatically (it is a child of the
// element's own composition visual) — only the size needs syncing, and only on resize.
static void sync_size(WebViewCtx *c) {
    if (!c->controller || !c->placeholder)
        return;
    double w = c->placeholder.ActualWidth(), h = c->placeholder.ActualHeight();
    bool show = w > 0 && h > 0;
    if (c->rootVisual)
        c->rootVisual.Size({static_cast<float>(w), static_cast<float>(h)});
    RECT b{0, 0, static_cast<LONG>(std::lround(w)), static_cast<LONG>(std::lround(h))};
    try {
        c->controller->put_Bounds(b);
        c->controller->put_IsVisible(show ? TRUE : FALSE);
    } catch (...) {
    }
}

// Modifier/button state (COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS) for a pointer event.
static COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS vkeys_of(WUInput::PointerPointProperties const &props) {
    uint32_t v = COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_NONE;
    if (props.IsLeftButtonPressed())
        v |= COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_LEFT_BUTTON;
    if (props.IsRightButtonPressed())
        v |= COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_RIGHT_BUTTON;
    if (props.IsMiddleButtonPressed())
        v |= COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_MIDDLE_BUTTON;
    if (GetKeyState(VK_CONTROL) < 0)
        v |= COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_CONTROL;
    if (GetKeyState(VK_SHIFT) < 0)
        v |= COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_SHIFT;
    return static_cast<COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS>(v);
}

// The pointer position relative to the Border, in DIPs — the WebView2's Bounds coordinate space.
static POINT point_of(WebViewCtx *c, WUXI::PointerRoutedEventArgs const &e) {
    auto pos = e.GetCurrentPoint(c->placeholder).Position();
    return POINT{static_cast<LONG>(std::lround(pos.X)), static_cast<LONG>(std::lround(pos.Y))};
}

// Wire the Border's XAML pointer events to the composition controller. This is the crux of windowless
// hosting: XAML's input site delivers pointer input to the Border, and we forward it to the browser.
static void wire_input(void *handle) {
    auto *c = find_ctx(handle);
    if (!c)
        return;
    auto &pl = c->placeholder;

    pl.PointerMoved([handle](WF::IInspectable const &, WUXI::PointerRoutedEventArgs const &e) {
        auto *cc = find_ctx(handle);
        if (!cc || !cc->compositionController)
            return;
        auto pp = e.GetCurrentPoint(cc->placeholder);
        cc->compositionController->SendMouseInput(COREWEBVIEW2_MOUSE_EVENT_KIND_MOVE,
                                                  vkeys_of(pp.Properties()), 0, point_of(cc, e));
        e.Handled(true);
    });

    pl.PointerPressed([handle](WF::IInspectable const &, WUXI::PointerRoutedEventArgs const &e) {
        auto *cc = find_ctx(handle);
        if (!cc || !cc->compositionController)
            return;
        cc->placeholder.CapturePointer(e.Pointer()); // keep move/up during a drag outside the element
        if (cc->controller)
            cc->controller->MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        auto pp = e.GetCurrentPoint(cc->placeholder);
        COREWEBVIEW2_MOUSE_EVENT_KIND kind;
        switch (pp.Properties().PointerUpdateKind()) {
        case WUInput::PointerUpdateKind::LeftButtonPressed:
            kind = COREWEBVIEW2_MOUSE_EVENT_KIND_LEFT_BUTTON_DOWN;
            break;
        case WUInput::PointerUpdateKind::RightButtonPressed:
            kind = COREWEBVIEW2_MOUSE_EVENT_KIND_RIGHT_BUTTON_DOWN;
            break;
        case WUInput::PointerUpdateKind::MiddleButtonPressed:
            kind = COREWEBVIEW2_MOUSE_EVENT_KIND_MIDDLE_BUTTON_DOWN;
            break;
        default:
            return;
        }
        cc->compositionController->SendMouseInput(kind, vkeys_of(pp.Properties()), 0, point_of(cc, e));
        e.Handled(true);
    });

    pl.PointerReleased([handle](WF::IInspectable const &, WUXI::PointerRoutedEventArgs const &e) {
        auto *cc = find_ctx(handle);
        if (!cc || !cc->compositionController)
            return;
        auto pp = e.GetCurrentPoint(cc->placeholder);
        COREWEBVIEW2_MOUSE_EVENT_KIND kind;
        switch (pp.Properties().PointerUpdateKind()) {
        case WUInput::PointerUpdateKind::LeftButtonReleased:
            kind = COREWEBVIEW2_MOUSE_EVENT_KIND_LEFT_BUTTON_UP;
            break;
        case WUInput::PointerUpdateKind::RightButtonReleased:
            kind = COREWEBVIEW2_MOUSE_EVENT_KIND_RIGHT_BUTTON_UP;
            break;
        case WUInput::PointerUpdateKind::MiddleButtonReleased:
            kind = COREWEBVIEW2_MOUSE_EVENT_KIND_MIDDLE_BUTTON_UP;
            break;
        default:
            kind = COREWEBVIEW2_MOUSE_EVENT_KIND_LEFT_BUTTON_UP;
            break;
        }
        cc->compositionController->SendMouseInput(kind, vkeys_of(pp.Properties()), 0, point_of(cc, e));
        cc->placeholder.ReleasePointerCapture(e.Pointer());
        e.Handled(true);
    });

    pl.PointerWheelChanged([handle](WF::IInspectable const &, WUXI::PointerRoutedEventArgs const &e) {
        auto *cc = find_ctx(handle);
        if (!cc || !cc->compositionController)
            return;
        auto props = e.GetCurrentPoint(cc->placeholder).Properties();
        auto kind = props.IsHorizontalMouseWheel() ? COREWEBVIEW2_MOUSE_EVENT_KIND_HORIZONTAL_WHEEL
                                                    : COREWEBVIEW2_MOUSE_EVENT_KIND_WHEEL;
        cc->compositionController->SendMouseInput(kind, vkeys_of(props),
                                                  static_cast<UINT32>(props.MouseWheelDelta()),
                                                  point_of(cc, e));
        e.Handled(true); // don't let a parent ScrollViewer also scroll
    });

    pl.PointerExited([handle](WF::IInspectable const &, WUXI::PointerRoutedEventArgs const &e) {
        auto *cc = find_ctx(handle);
        if (!cc || !cc->compositionController)
            return;
        cc->compositionController->SendMouseInput(COREWEBVIEW2_MOUSE_EVENT_KIND_LEAVE,
                                                  COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_NONE, 0,
                                                  POINT{0, 0});
    });
}

static void destroy_ctx(void *handle) {
    auto it = g_webviews.find(handle);
    if (it == g_webviews.end())
        return;
    WebViewCtx *c = it->second;
    if (c->placeholder) {
        try {
            WUXH::ElementCompositionPreview::SetElementChildVisual(c->placeholder, nullptr);
        } catch (...) {
        }
    }
    if (c->controller) {
        try {
            c->controller->Close();
        } catch (...) {
        }
    }
    g_webviews.erase(it);
    delete c;
}

static std::wstring user_data_folder() {
    wchar_t buf[MAX_PATH]{};
    DWORD n = GetTempPathW(MAX_PATH, buf);
    std::wstring p(buf, n);
    p += L"day-webview2";
    return p;
}

// Kick off async WebView2 creation: environment → composition controller → attach the render visual,
// wire input + NavigationCompleted, size, navigate. All callbacks run on the UI thread (WebView2 posts
// to the creating thread), so touching XAML / composition / g_webviews here is safe.
static void create_webview2(void *handle) {
    auto *c = find_ctx(handle);
    if (!c)
        return;
    std::wstring udf = user_data_folder();
    auto options = wrl::Make<CoreWebView2EnvironmentOptions>();
    if (options)
        options->put_AdditionalBrowserArguments(L"--disable-features=CalculateNativeWinOcclusion");
    CreateCoreWebView2EnvironmentWithOptions(
        nullptr, udf.c_str(), options.Get(),
        wrl::Callback<ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler>(
            [handle](HRESULT r, ICoreWebView2Environment *env) -> HRESULT {
                auto *cc = find_ctx(handle);
                if (!cc || FAILED(r) || !env)
                    return S_OK;
                wrl::ComPtr<ICoreWebView2Environment3> env3;
                if (FAILED(env->QueryInterface(IID_PPV_ARGS(&env3))) || !env3)
                    return S_OK;
                env3->CreateCoreWebView2CompositionController(
                    cc->parent,
                    wrl::Callback<ICoreWebView2CreateCoreWebView2CompositionControllerCompletedHandler>(
                        [handle](HRESULT r2, ICoreWebView2CompositionController *comp) -> HRESULT {
                            auto *c2 = find_ctx(handle);
                            if (!c2 || FAILED(r2) || !comp)
                                return S_OK;
                            c2->compositionController = comp;
                            c2->compositionController.As(&c2->controller); // same object, base interface
                            if (!c2->controller)
                                return S_OK;
                            c2->controller->get_CoreWebView2(c2->webview.GetAddressOf());

                            // Logical (DIP) bounds scaled by the window DPI — crisp at any scale.
                            wrl::ComPtr<ICoreWebView2Controller3> c3;
                            if (SUCCEEDED(c2->controller.As(&c3)) && c3) {
                                c3->put_BoundsMode(COREWEBVIEW2_BOUNDS_MODE_USE_RASTERIZATION_SCALE);
                                c3->put_ShouldDetectMonitorScaleChanges(FALSE);
                                c3->put_RasterizationScale(c2->scale);
                            }

                            // Splice the browser's render visual into the Border's XAML visual.
                            auto elemVisual =
                                WUXH::ElementCompositionPreview::GetElementVisual(c2->placeholder);
                            auto compositor = elemVisual.Compositor();
                            c2->rootVisual = compositor.CreateContainerVisual();
                            c2->compositionController->put_RootVisualTarget(
                                reinterpret_cast<::IUnknown *>(winrt::get_abi(c2->rootVisual)));
                            WUXH::ElementCompositionPreview::SetElementChildVisual(c2->placeholder,
                                                                                  c2->rootVisual);

                            if (c2->webview) {
                                // Report the settled URL back so the app's URL bar follows navigation.
                                EventRegistrationToken tok{};
                                c2->webview->add_NavigationCompleted(
                                    wrl::Callback<ICoreWebView2NavigationCompletedEventHandler>(
                                        [handle](ICoreWebView2 *wv,
                                                 ICoreWebView2NavigationCompletedEventArgs *)
                                            -> HRESULT {
                                            auto *c3n = find_ctx(handle);
                                            if (!c3n)
                                                return S_OK;
                                            LPWSTR src = nullptr;
                                            if (SUCCEEDED(wv->get_Source(&src)) && src) {
                                                std::string s = to_utf8(winrt::hstring{src});
                                                if (c3n->cb)
                                                    c3n->cb(c3n->id, s.c_str());
                                                CoTaskMemFree(src);
                                            }
                                            return S_OK;
                                        })
                                        .Get(),
                                    &tok);
                            }
                            wire_input(handle);
                            sync_size(c2);
                            if (c2->webview && !c2->pending_url.empty())
                                c2->webview->Navigate(c2->pending_url.c_str());
                            return S_OK;
                        })
                        .Get());
                return S_OK;
            })
            .Get());
    // A failed HRESULT here (e.g. no WebView2 Runtime) leaves the Border's URL label as the fallback.
}

extern "C" {

void *day_webview_winui_new(const char *url, uint64_t id, void (*cb)(uint64_t, const char *)) {
    // The boxed element day lays out: a transparent (hit-testable) Border carrying a faint URL label.
    // The browser's render visual is spliced in as the Border's child visual and covers the label;
    // if the WebView2 Runtime is absent, the label remains as the graceful, no-crash fallback.
    WUXC::Border placeholder;
    placeholder.Background(WUXM::SolidColorBrush(WUI::Color{0, 0, 0, 0})); // transparent but hit-testable
    WUXC::TextBlock label;
    label.Text(hs(url ? url : ""));
    label.Margin(WUX::Thickness{8, 8, 8, 8});
    label.Opacity(0.6);
    placeholder.Child(label);
    void *handle = day_winui_box(winrt::get_abi(placeholder));

    auto *c = new WebViewCtx{};
    c->parent = reinterpret_cast<HWND>(day_winui_host_hwnd());
    c->placeholder = placeholder;
    c->id = id;
    c->cb = cb;
    c->pending_url = hs(url).c_str();
    UINT dpi = c->parent ? GetDpiForWindow(c->parent) : 96;
    c->scale = (dpi ? dpi : 96) / 96.0;
    g_webviews[handle] = c;

    // Keep the render visual + Bounds matched to the Border as it resizes.
    placeholder.SizeChanged([handle](WF::IInspectable const &, WUX::SizeChangedEventArgs const &) {
        if (auto *cc = find_ctx(handle))
            sync_size(cc);
    });
    // Tie the browser to the element's tree membership: day removing the node raises Unloaded.
    placeholder.Unloaded(
        [handle](WF::IInspectable const &, WUX::RoutedEventArgs const &) { destroy_ctx(handle); });

    create_webview2(handle);
    return handle;
}

void day_webview_winui_load(void *handle, const char *url) {
    auto *c = find_ctx(handle);
    if (!c)
        return;
    std::wstring w = hs(url).c_str();
    if (c->webview)
        c->webview->Navigate(w.c_str());
    else
        c->pending_url = w;
}
void day_webview_winui_back(void *handle) {
    auto *c = find_ctx(handle);
    if (c && c->webview) {
        BOOL can = FALSE;
        c->webview->get_CanGoBack(&can);
        if (can)
            c->webview->GoBack();
    }
}
void day_webview_winui_forward(void *handle) {
    auto *c = find_ctx(handle);
    if (c && c->webview) {
        BOOL can = FALSE;
        c->webview->get_CanGoForward(&can);
        if (can)
            c->webview->GoForward();
    }
}
void day_webview_winui_stop(void *handle) {
    auto *c = find_ctx(handle);
    if (c && c->webview)
        c->webview->Stop();
}
void day_webview_winui_reload(void *handle) {
    auto *c = find_ctx(handle);
    if (c && c->webview)
        c->webview->Reload();
}

} // extern "C"
