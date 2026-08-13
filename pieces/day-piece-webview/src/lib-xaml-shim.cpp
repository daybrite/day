// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The web-view piece's OWN C++/WinRT shim — parallel to src/lib-qt-shim.cpp.
//
// day-xaml hosts UWP system XAML (winrt::Windows::UI::Xaml, base Windows SDK, no WinAppSDK) inside a
// Win32 window via XAML Islands. The system-XAML web view, Windows.UI.Xaml.Controls.WebView (EdgeHTML),
// is UNSUPPORTED in that host: it renders blank, never raises NavigationCompleted, and crashes on
// navigation. The supported engine is WebView2, hosted here in WINDOWLESS / VISUAL-HOSTING mode — the
// same technique the official XAML WebView2 controls use internally:
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

// Seams exported by day-xaml-sys (already linked into the app). The host HWND is the composition
// controller's parentWindow (for DPI / IME / input association) — the page still renders windowless.
extern "C" void *day_xaml_box(void *iinspectable_abi);
extern "C" void *day_xaml_unbox(void *handle);
extern "C" void *day_xaml_host_hwnd();

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
    // Inline mode (docs/webview.md): the URL prefix of the bundled site under the virtual host
    // (empty = remote mode) and the callback a cancelled external navigation reports through.
    std::wstring inline_prefix;
    void (*link_cb)(uint64_t, const char *){};
};

// The virtual host the exe-relative assets tree is mapped under for inline sites. `.example` is
// reserved for exactly this use, the same convention Microsoft's own WebView2 docs model.
static const wchar_t *kDayAssetsHost = L"day-assets.example";

static std::map<void *, WebViewCtx *> g_webviews;

static WebViewCtx *find_ctx(void *handle) {
    auto it = g_webviews.find(handle);
    return it == g_webviews.end() ? nullptr : it->second;
}

// ---- JavaScript evaluation (docs/webview-eval.md) --------------------------
//
// One file-static callback shared by every web view; each reply carries its own node id, so the
// Rust side registers it once rather than per view (the Qt shim's arrangement).
static void (*g_eval_cb)(uint64_t, uint64_t, const char *) = nullptr;

// A reply in the 0x1F-separated form the Rust front-end decodes. Only for ENGINE failures — a
// script that merely throws is caught by the JS wrapper and arrives as an ordinary value.
static std::string eval_engine_error(const char *name, const char *message) {
    std::string s = "0";
    s.push_back('\x1f');
    s += name;
    s.push_back('\x1f');
    s += message;
    return s;
}

/// Deliver exactly one reply for `req`. Every path out of an eval MUST reach this, or the Rust
/// future waits forever: WebView2 releases pending handlers on `Close()`, so a dropped callback
/// is a stranded request rather than a late one.
static void eval_reply(uint64_t id, uint64_t req, std::string const &payload) {
    if (g_eval_cb)
        g_eval_cb(id, req, payload.c_str());
}

/// Decode a JSON string literal into the string it denotes — the fallback path's job.
///
/// `ExecuteScript` hands back the result AS JSON, so the wrapper's string return arrives quoted
/// and escaped, and the front-end wants the string itself. `` is the separator the whole
/// protocol is built on, so `\u` decoding (surrogate pairs included) is required, not optional.
/// Returns false when the text is not a JSON string at all, which is an engine-level failure.
static bool json_string_to_utf8(std::wstring const &json, std::string &out) {
    if (json.size() < 2 || json.front() != L'"' || json.back() != L'"')
        return false;
    std::wstring w;
    w.reserve(json.size());
    for (size_t i = 1; i + 1 < json.size(); ++i) {
        wchar_t c = json[i];
        if (c != L'\\') {
            w.push_back(c);
            continue;
        }
        if (++i + 1 > json.size())
            return false;
        switch (json[i]) {
        case L'"': w.push_back(L'"'); break;
        case L'\\': w.push_back(L'\\'); break;
        case L'/': w.push_back(L'/'); break;
        case L'b': w.push_back(L'\b'); break;
        case L'f': w.push_back(L'\f'); break;
        case L'n': w.push_back(L'\n'); break;
        case L'r': w.push_back(L'\r'); break;
        case L't': w.push_back(L'\t'); break;
        case L'u': {
            if (i + 4 >= json.size())
                return false;
            unsigned v = 0;
            for (int k = 1; k <= 4; ++k) {
                wchar_t d = json[i + k];
                v <<= 4;
                if (d >= L'0' && d <= L'9') v |= unsigned(d - L'0');
                else if (d >= L'a' && d <= L'f') v |= unsigned(d - L'a' + 10);
                else if (d >= L'A' && d <= L'F') v |= unsigned(d - L'A' + 10);
                else return false;
            }
            i += 4;
            // A surrogate pair is two \u escapes; UTF-16 is wchar_t's own encoding on Windows, so
            // both halves are pushed as-is and the conversion below pairs them.
            w.push_back(static_cast<wchar_t>(v));
            break;
        }
        default:
            return false;
        }
    }
    int len = WideCharToMultiByte(CP_UTF8, 0, w.c_str(), static_cast<int>(w.size()), nullptr, 0,
                                  nullptr, nullptr);
    out.assign(static_cast<size_t>(len), '\0');
    if (len)
        WideCharToMultiByte(CP_UTF8, 0, w.c_str(), static_cast<int>(w.size()), out.data(), len,
                            nullptr, nullptr);
    return true;
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

                            if (c2->webview && !c2->inline_prefix.empty()) {
                                // Inline mode: map the exe-relative assets tree under the virtual
                                // host BEFORE the first Navigate, then police top-level
                                // navigations against the site prefix — leaving ones are
                                // CANCELLED and reported (the Rust side runs the LinkPolicy;
                                // events are enqueue-only, so the verdict can't come back here).
                                wrl::ComPtr<ICoreWebView2_3> wv3;
                                if (SUCCEEDED(c2->webview->QueryInterface(IID_PPV_ARGS(&wv3))) &&
                                    wv3) {
                                    wchar_t exe[MAX_PATH]{};
                                    GetModuleFileNameW(nullptr, exe, MAX_PATH);
                                    std::wstring assets(exe);
                                    size_t slash = assets.find_last_of(L"\\/");
                                    if (slash != std::wstring::npos)
                                        assets.resize(slash);
                                    assets += L"\\assets";
                                    wv3->SetVirtualHostNameToFolderMapping(
                                        kDayAssetsHost, assets.c_str(),
                                        COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_ALLOW);
                                }
                                EventRegistrationToken navTok{};
                                c2->webview->add_NavigationStarting(
                                    wrl::Callback<ICoreWebView2NavigationStartingEventHandler>(
                                        [handle](ICoreWebView2 *,
                                                 ICoreWebView2NavigationStartingEventArgs *args)
                                            -> HRESULT {
                                            auto *cn = find_ctx(handle);
                                            if (!cn || cn->inline_prefix.empty())
                                                return S_OK;
                                            LPWSTR uri = nullptr;
                                            if (FAILED(args->get_Uri(&uri)) || !uri)
                                                return S_OK;
                                            std::wstring u(uri);
                                            CoTaskMemFree(uri);
                                            const bool inside =
                                                u.rfind(cn->inline_prefix, 0) == 0 ||
                                                u == L"about:blank";
                                            if (!inside) {
                                                args->put_Cancel(TRUE);
                                                if (cn->link_cb) {
                                                    std::string s = to_utf8(winrt::hstring{u});
                                                    cn->link_cb(cn->id, s.c_str());
                                                }
                                            }
                                            return S_OK;
                                        })
                                        .Get(),
                                    &navTok);
                                // window.open / target=_blank: external by definition — no new
                                // window exists in day's tree, so report and swallow.
                                EventRegistrationToken winTok{};
                                c2->webview->add_NewWindowRequested(
                                    wrl::Callback<ICoreWebView2NewWindowRequestedEventHandler>(
                                        [handle](ICoreWebView2 *,
                                                 ICoreWebView2NewWindowRequestedEventArgs *args)
                                            -> HRESULT {
                                            auto *cn = find_ctx(handle);
                                            if (!cn || cn->inline_prefix.empty())
                                                return S_OK;
                                            args->put_Handled(TRUE);
                                            LPWSTR uri = nullptr;
                                            if (SUCCEEDED(args->get_Uri(&uri)) && uri) {
                                                std::string s =
                                                    to_utf8(winrt::hstring{std::wstring(uri)});
                                                if (cn->link_cb)
                                                    cn->link_cb(cn->id, s.c_str());
                                                CoTaskMemFree(uri);
                                            }
                                            return S_OK;
                                        })
                                        .Get(),
                                    &winTok);
                            }
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

void *day_webview_xaml_new(const char *url, uint64_t id, void (*cb)(uint64_t, const char *),
                           const char *inline_root, const char *inline_start,
                           void (*link_cb)(uint64_t, const char *)) {
    // The boxed element day lays out: a transparent (hit-testable) Border carrying a faint URL label.
    // The browser's render visual is spliced in as the Border's child visual and covers the label;
    // if the WebView2 Runtime is absent, the label remains as the graceful, no-crash fallback.
    const bool inlined = inline_root && *inline_root;
    std::wstring first = hs(url ? url : "").c_str();
    std::wstring prefix;
    if (inlined) {
        // The bundled site browses under the virtual host mapped at controller creation.
        prefix = std::wstring(L"https://") + kDayAssetsHost + L"/" + hs(inline_root).c_str() + L"/";
        first = prefix + hs(inline_start ? inline_start : "index.html").c_str();
    }
    WUXC::Border placeholder;
    placeholder.Background(WUXM::SolidColorBrush(WUI::Color{0, 0, 0, 0})); // transparent but hit-testable
    WUXC::TextBlock label;
    label.Text(winrt::hstring{first});
    label.Margin(WUX::Thickness{8, 8, 8, 8});
    label.Opacity(0.6);
    placeholder.Child(label);
    void *handle = day_xaml_box(winrt::get_abi(placeholder));

    auto *c = new WebViewCtx{};
    c->parent = reinterpret_cast<HWND>(day_xaml_host_hwnd());
    c->placeholder = placeholder;
    c->id = id;
    c->cb = cb;
    c->pending_url = first;
    c->inline_prefix = prefix;
    c->link_cb = link_cb;
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

void day_webview_xaml_load(void *handle, const char *url) {
    auto *c = find_ctx(handle);
    if (!c)
        return;
    std::wstring w = hs(url).c_str();
    if (c->webview)
        c->webview->Navigate(w.c_str());
    else
        c->pending_url = w;
}
void day_webview_xaml_back(void *handle) {
    auto *c = find_ctx(handle);
    if (c && c->webview) {
        BOOL can = FALSE;
        c->webview->get_CanGoBack(&can);
        if (can)
            c->webview->GoBack();
    }
}
void day_webview_xaml_forward(void *handle) {
    auto *c = find_ctx(handle);
    if (c && c->webview) {
        BOOL can = FALSE;
        c->webview->get_CanGoForward(&can);
        if (can)
            c->webview->GoForward();
    }
}
void day_webview_xaml_set_eval_cb(void (*cb)(uint64_t, uint64_t, const char *)) { g_eval_cb = cb; }

// Evaluate `script` (already wrapped by the Rust front-end) and reply exactly once.
//
// Two paths, and the better one is worth the probe: `ExecuteScriptWithResult` on ICoreWebView2_21
// reports failures at the ENGINE level, so it catches the one case the JS wrapper structurally
// cannot — a syntax error, where the wrapper and the user's script compile as a single unit and
// the wrapper's own `try` never runs. The interface arrived in SDK 1.0.2277.86 (this crate pins
// 1.0.3179.45, so the header is always present) but the RUNTIME is not guaranteed, so a failed
// query falls back to plain `ExecuteScript` + the wrapper's own error reporting.
//
// Handlers run on the creating UI thread, serially and never re-entrantly, so touching
// `g_eval_cb` from them needs no synchronization. They capture PODs only: `Close()` releases
// pending handlers, so a handler must never assume its web view still exists.
void day_webview_xaml_eval(void *handle, uint64_t req, const char *script) {
    auto *c = find_ctx(handle);
    if (!c || !c->webview) {
        // The WebView2 Runtime is absent, or the view is gone. Still a reply — the Rust future is
        // resolved only by one arriving.
        eval_reply(c ? c->id : 0, req, eval_engine_error("Error", "no engine"));
        return;
    }
    const uint64_t id = c->id;
    const std::wstring js = hs(script).c_str();

    wrl::ComPtr<ICoreWebView2_21> wv21;
    if (SUCCEEDED(c->webview.As(&wv21)) && wv21) {
        HRESULT hr = wv21->ExecuteScriptWithResult(
            js.c_str(),
            wrl::Callback<ICoreWebView2ExecuteScriptWithResultCompletedHandler>(
                [id, req](HRESULT code, ICoreWebView2ExecuteScriptResult *res) -> HRESULT {
                    if (FAILED(code) || !res) {
                        eval_reply(id, req, eval_engine_error("Error", "no result (page discarded)"));
                        return S_OK;
                    }
                    BOOL ok = FALSE;
                    res->get_Succeeded(&ok);
                    if (ok) {
                        // The wrapper always evaluates to a STRING, so ask for it directly rather
                        // than re-parsing ResultAsJson — the same shape Qt's QVariant and
                        // WebKit's NSString hand back.
                        LPWSTR str = nullptr;
                        BOOL is_string = FALSE;
                        if (SUCCEEDED(res->TryGetResultAsString(&str, &is_string)) && is_string &&
                            str) {
                            std::string utf8;
                            int len = WideCharToMultiByte(CP_UTF8, 0, str, -1, nullptr, 0, nullptr,
                                                          nullptr);
                            if (len > 1) {
                                utf8.assign(static_cast<size_t>(len - 1), '\0');
                                WideCharToMultiByte(CP_UTF8, 0, str, -1, utf8.data(), len - 1,
                                                    nullptr, nullptr);
                            }
                            CoTaskMemFree(str);
                            eval_reply(id, req, utf8);
                            return S_OK;
                        }
                        if (str)
                            CoTaskMemFree(str);
                        eval_reply(id, req,
                                   eval_engine_error("Error", "result was not a string"));
                        return S_OK;
                    }
                    // Engine-level failure — in practice a syntax error, since anything the script
                    // threw at run time was caught by the wrapper and returned as a value.
                    // Documented trap: get_Exception returns S_OK even when it yields nothing, so
                    // the out-pointer is what decides, not the HRESULT.
                    wrl::ComPtr<ICoreWebView2ScriptException> ex;
                    res->get_Exception(&ex);
                    if (!ex) {
                        eval_reply(id, req, eval_engine_error("Error", "script failed"));
                        return S_OK;
                    }
                    LPWSTR name = nullptr, message = nullptr;
                    ex->get_Name(&name);
                    ex->get_Message(&message);
                    auto narrow = [](LPWSTR w, const char *fallback) {
                        if (!w)
                            return std::string(fallback);
                        int len = WideCharToMultiByte(CP_UTF8, 0, w, -1, nullptr, 0, nullptr, nullptr);
                        std::string s;
                        if (len > 1) {
                            s.assign(static_cast<size_t>(len - 1), '\0');
                            WideCharToMultiByte(CP_UTF8, 0, w, -1, s.data(), len - 1, nullptr, nullptr);
                        }
                        return s.empty() ? std::string(fallback) : s;
                    };
                    const std::string n = narrow(name, "SyntaxError");
                    const std::string m = narrow(message, "script failed");
                    if (name)
                        CoTaskMemFree(name);
                    if (message)
                        CoTaskMemFree(message);
                    eval_reply(id, req, eval_engine_error(n.c_str(), m.c_str()));
                    return S_OK;
                })
                .Get());
        if (SUCCEEDED(hr))
            return;
        // The call itself was refused — fall through to the older path rather than stranding it.
    }

    HRESULT hr = c->webview->ExecuteScript(
        js.c_str(),
        wrl::Callback<ICoreWebView2ExecuteScriptCompletedHandler>(
            [id, req](HRESULT code, LPCWSTR json) -> HRESULT {
                if (FAILED(code) || !json) {
                    eval_reply(id, req, eval_engine_error("Error", "no result (page discarded)"));
                    return S_OK;
                }
                // This path returns the result AS JSON, so the wrapper's string arrives quoted
                // and escaped — including the `` separators the protocol rides on.
                std::string inner;
                if (!json_string_to_utf8(json, inner)) {
                    eval_reply(id, req, eval_engine_error("Error", "result was not a string"));
                    return S_OK;
                }
                eval_reply(id, req, inner);
                return S_OK;
            })
            .Get());
    if (FAILED(hr))
        eval_reply(id, req, eval_engine_error("Error", "evaluation was refused"));
}

void day_webview_xaml_stop(void *handle) {
    auto *c = find_ctx(handle);
    if (c && c->webview)
        c->webview->Stop();
}
void day_webview_xaml_reload(void *handle) {
    auto *c = find_ctx(handle);
    if (c && c->webview)
        c->webview->Reload();
}

} // extern "C"
