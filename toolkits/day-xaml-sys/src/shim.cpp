// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-xaml-sys — C++/WinRT XAML Islands shim (DESIGN.md §9).
//
// Hosts the Windows.UI.Xaml control set inside a Win32 host window via
// DesktopWindowXamlSource, and exposes a flat C ABI mirroring day-qt-sys. day owns layout:
// containers are XAML Canvases and children are positioned by absolute frame
// (Canvas.Left/Top + Width/Height). Native events call back into Rust by node id.

#define UNICODE
#define _UNICODE
#include <windows.h>
#undef GetCurrentTime // windows.h macro clashes with Windows.UI.Xaml.Media.Animation
#include <gdiplus.h>  // PNG encoding for window snapshots
#include <dwmapi.h>   // dark title bar opt-in (DwmSetWindowAttribute)
#ifndef DWMWA_USE_IMMERSIVE_DARK_MODE
#define DWMWA_USE_IMMERSIVE_DARK_MODE 20 // absent from pre-20H1 SDK headers
#endif
// Mica (docs/navigation.md). Absent from SDKs older than 22621; the call then fails and the
// window keeps its opaque ground, so defining them here costs nothing on older targets.
#ifndef DWMWA_SYSTEMBACKDROP_TYPE
#define DWMWA_SYSTEMBACKDROP_TYPE 38
#endif
#ifndef DWMSBT_MAINWINDOW
#define DWMSBT_MAINWINDOW 2 // the Mica backdrop a document-shaped app asks for
#endif

#include <string>
#include <limits>
#include <algorithm> // std::sort — radial-gradient stop ordering
#include <cwctype>   // towupper / iswalpha — menu access keys
#include <cwchar>    // wcscmp — WM_SETTINGCHANGE area name
#include <charconv>
#include <cstdio>
#include <cstdlib>
#include <cmath>
#include <vector>
#include <map>
#include <functional> // OEM menu-accelerator dispatch (see add_accel)
#include <fstream> // read local image bytes for BitmapImage.SetSource (file:// URIs don't load)
#include <shobjidl_core.h> // IInitializeWithWindow — parents WinRT file pickers to the host HWND

#include <winrt/base.h>
// DataPackage/RequestedOperation for list drag-to-reorder (docs/list.md): the projection's
// consume definitions live here — without this, any method call on DataPackage trips C3779
// ("a function that returns 'auto' cannot be used before it is defined").
#include <winrt/Windows.ApplicationModel.DataTransfer.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Storage.Streams.h>
#include <winrt/Windows.System.h>
#include <winrt/Windows.UI.h>
#include <winrt/Windows.UI.Input.h> // HoldingState (long-press gesture)
#include <winrt/Windows.UI.Text.h>
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>
#include <winrt/Windows.UI.Xaml.Documents.h> // Typography
#include <winrt/Windows.UI.Xaml.Controls.Primitives.h>
#include <winrt/Windows.UI.Xaml.Input.h>
#include <winrt/Windows.UI.Xaml.Media.h>
// Storyboard / DoubleAnimation / ColorAnimation — backend-executed animation (DESIGN.md §8.4).
// This is the header the `#undef GetCurrentTime` at the top of this file exists for.
#include <winrt/Windows.UI.Xaml.Media.Animation.h>
#include <winrt/Windows.UI.Xaml.Media.Imaging.h>
#include <winrt/Windows.UI.Xaml.Shapes.h>
#include <winrt/Windows.UI.Xaml.Hosting.h>
#include <winrt/Windows.UI.Composition.h> // rounded corner clip (see day_xaml_container_set_corner)
#include <winrt/Windows.UI.Xaml.Automation.h>
#include <winrt/Windows.UI.Xaml.Markup.h>
#include <winrt/Windows.UI.Xaml.Interop.h>
#include <winrt/Windows.Storage.h>         // StorageFile (file-picker results)
#include <winrt/Windows.Storage.Pickers.h> // FileOpenPicker / FileSavePicker

#include <windows.ui.xaml.hosting.desktopwindowxamlsource.h>
#include <DispatcherQueue.h>
#include <robuffer.h> // IBufferByteAccess — raw pixels out of a WinRT IBuffer

using namespace winrt;
namespace WF = winrt::Windows::Foundation;
namespace WUI = winrt::Windows::UI;
namespace WUX = winrt::Windows::UI::Xaml;
namespace WUXC = winrt::Windows::UI::Xaml::Controls;
namespace WUXCP = winrt::Windows::UI::Xaml::Controls::Primitives;
namespace WUXD = winrt::Windows::UI::Xaml::Documents; // Typography (numeral alignment)
namespace WUXM = winrt::Windows::UI::Xaml::Media;
namespace WUXMA = winrt::Windows::UI::Xaml::Media::Animation;
namespace WUXSh = winrt::Windows::UI::Xaml::Shapes;
namespace WUXH = winrt::Windows::UI::Xaml::Hosting;
namespace WUComp = winrt::Windows::UI::Composition;
namespace WUXIn = winrt::Windows::UI::Xaml::Input;
namespace WUIIn = winrt::Windows::UI::Input;
namespace WS = winrt::Windows::System;
namespace WSt = winrt::Windows::Storage;
namespace WStP = winrt::Windows::Storage::Pickers;
namespace WSS = winrt::Windows::Storage::Streams;

using WUX::UIElement;
using WUX::FrameworkElement;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

static winrt::hstring hs(const char* s) {
    if (!s || !*s) return winrt::hstring{};
    int len = MultiByteToWideChar(CP_UTF8, 0, s, -1, nullptr, 0);
    if (len <= 1) return winrt::hstring{};
    std::wstring w(static_cast<size_t>(len - 1), L'\0');
    MultiByteToWideChar(CP_UTF8, 0, s, -1, w.data(), len);
    return winrt::hstring{ w };
}

static std::string u8(winrt::hstring const& h) {
    if (h.empty()) return {};
    int len = WideCharToMultiByte(CP_UTF8, 0, h.c_str(), -1, nullptr, 0, nullptr, nullptr);
    if (len <= 1) return {};
    std::string s(static_cast<size_t>(len - 1), '\0');
    WideCharToMultiByte(CP_UTF8, 0, h.c_str(), -1, s.data(), len, nullptr, nullptr);
    return s;
}

// cppwinrt projected types delete `operator new`, so a bare `new UIElement(e)` is illegal.
// A plain wrapper struct owns the WinRT reference on the heap; delete releases it.
struct Node {
    UIElement e;
    explicit Node(UIElement const& x) : e(x) {}
};
static void* boxh(UIElement const& e) { return new Node(e); }
static UIElement& elem(void* h) { return reinterpret_cast<Node*>(h)->e; }

// A WinRT HRESULT thrown out of an element-mutating FFI entry point would unwind through Rust's
// `extern "C"` post-trampoline (run_posted) — a foreign unwind through a non-unwindable frame
// aborts the whole process ("panic in a function that cannot unwind"). Those entry points are all
// best-effort side effects on one element (layout, a11y id, visibility…), so a failure on a
// degraded element must be swallowed, not fatal. Motivating case: the EdgeHTML WebView
// (day-piece-webview) is a zombie on a headless CI host — its backing browser process never
// starts, so it throws on *every* interaction (SetAutomationId, Canvas.SetTop, …). Wrapping the
// FFI seam lets that page degrade to blank instead of taking the whole app down. Keep this OUT of
// element-creating entry points (`*_new`) — a null handle there would just crash Rust later.
template <typename F> static void guard(F&& f) {
    try {
        f();
    } catch (...) {
    }
}

// Pump the message loop until a WinRT async op completes (bounded). RenderTargetBitmap's async
// work runs on this UI thread, so a blocking .get() would deadlock — we must pump. (Templates
// can't live in the extern "C" block, hence file scope here.)
template <typename TOp>
static void pump_until_complete(TOp const& op) {
    auto done = std::make_shared<bool>(false);
    op.Completed([done](auto&&, auto&&) { *done = true; });
    MSG msg{};
    ULONGLONG start = GetTickCount64();
    // Snapshots run inside a day-core `with_tree` borrow; day's cross-thread post (WM_APP+1)
    // trampolines re-enter `with_tree` (e.g. a pending list-reload's bind_row). Excluding that
    // message from this nested pump leaves those closures queued for the real loop — the async
    // render completes via XAML's own messages, so nothing is lost.
    const UINT day_post = WM_APP + 1;
    while (!*done && GetTickCount64() - start < 5000) {
        if (PeekMessageW(&msg, nullptr, 0, day_post - 1, PM_REMOVE) ||
            PeekMessageW(&msg, nullptr, day_post + 1, 0xFFFFFFFF, PM_REMOVE)) {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        } else {
            Sleep(1);
        }
    }
}

// ---- canvas display list helpers (§11, docs/shapes.md) ----
// XAML is retained-mode, so each op becomes a Path/TextBlock child of the Canvas; the painter
// transform stack (Save/Restore/Concat) is folded into each element's RenderTransform (a
// MatrixTransform — same row-vector convention as day's Affine).
static WUXM::SolidColorBrush brush_bits(unsigned col) {
    WUI::Color c;
    c.A = static_cast<uint8_t>((col >> 24) & 0xff);
    c.R = static_cast<uint8_t>((col >> 16) & 0xff);
    c.G = static_cast<uint8_t>((col >> 8) & 0xff);
    c.B = static_cast<uint8_t>(col & 0xff);
    return WUXM::SolidColorBrush(c);
}
// System XAML (Windows.UI.Xaml.Media) ships LinearGradientBrush but no RadialGradientBrush — that
// type lives only in WinUI 2 (Microsoft.UI.Xaml), which this shim doesn't pull in. Synthesize the
// equivalent: rasterize the radial ramp into a unit-square WriteableBitmap and paint it through an
// ImageBrush with Stretch::Fill, which maps the unit square onto the shape's bounds — so the circle
// stretches to an ellipse in non-square bounds, matching LinearGradientBrush's RelativeToBoundingBox.
static WUI::Color radial_color_at(std::vector<std::pair<float, WUI::Color>> const& stops, double t) {
    if (stops.empty()) return WUI::Color{};
    if (t <= stops.front().first) return stops.front().second;
    if (t >= stops.back().first) return stops.back().second;
    for (size_t i = 1; i < stops.size(); ++i) {
        if (t <= stops[i].first) {
            double span = stops[i].first - stops[i - 1].first;
            double u = span > 0.0 ? (t - stops[i - 1].first) / span : 0.0;
            WUI::Color a = stops[i - 1].second, b = stops[i].second;
            auto mix = [u](uint8_t x, uint8_t y) {
                return static_cast<uint8_t>(std::lround(x + (y - x) * u));
            };
            WUI::Color c;
            c.A = mix(a.A, b.A);
            c.R = mix(a.R, b.R);
            c.G = mix(a.G, b.G);
            c.B = mix(a.B, b.B);
            return c;
        }
    }
    return stops.back().second;
}
static WUXM::ImageBrush make_radial_brush(double cx, double cy, double radius,
                                          std::vector<std::pair<float, WUI::Color>> stops) {
    std::sort(stops.begin(), stops.end(),
              [](auto const& l, auto const& r) { return l.first < r.first; });
    const int N = 256;
    WUXM::Imaging::WriteableBitmap wb(N, N);
    uint8_t* px = nullptr;
    wb.PixelBuffer().as<::Windows::Storage::Streams::IBufferByteAccess>()->Buffer(&px);
    for (int y = 0; y < N; ++y) {
        double v = static_cast<double>(y) / (N - 1);
        for (int x = 0; x < N; ++x) {
            double u = static_cast<double>(x) / (N - 1);
            double du = radius > 0.0 ? (u - cx) / radius : 0.0;
            double dv = radius > 0.0 ? (v - cy) / radius : 0.0;
            double t = std::sqrt(du * du + dv * dv);
            WUI::Color c = radial_color_at(stops, t > 1.0 ? 1.0 : t);
            // WriteableBitmap's PixelBuffer is premultiplied BGRA8.
            size_t o = (static_cast<size_t>(y) * N + x) * 4;
            px[o + 0] = static_cast<uint8_t>(c.B * c.A / 255);
            px[o + 1] = static_cast<uint8_t>(c.G * c.A / 255);
            px[o + 2] = static_cast<uint8_t>(c.R * c.A / 255);
            px[o + 3] = c.A;
        }
    }
    wb.Invalidate();
    WUXM::ImageBrush ib;
    ib.ImageSource(wb);
    ib.Stretch(WUXM::Stretch::Fill);
    return ib;
}
static WUXM::Matrix mat_identity() {
    WUXM::Matrix m{};
    m.M11 = 1;
    m.M22 = 1;
    return m;
}
// Row-vector affine product "apply x, then y" (p' = p·x·y).
static WUXM::Matrix mat_mul(WUXM::Matrix const& x, WUXM::Matrix const& y) {
    WUXM::Matrix r{};
    r.M11 = x.M11 * y.M11 + x.M12 * y.M21;
    r.M12 = x.M11 * y.M12 + x.M12 * y.M22;
    r.M21 = x.M21 * y.M11 + x.M22 * y.M21;
    r.M22 = x.M21 * y.M12 + x.M22 * y.M22;
    r.OffsetX = x.OffsetX * y.M11 + x.OffsetY * y.M21 + y.OffsetX;
    r.OffsetY = x.OffsetX * y.M12 + x.OffsetY * y.M22 + y.OffsetY;
    return r;
}
static void place_shape(WUXC::Canvas const& canvas, WUXSh::Shape const& p, WUXM::Matrix const& cur) {
    WUXC::Canvas::SetLeft(p, 0);
    WUXC::Canvas::SetTop(p, 0);
    WUXM::MatrixTransform mt;
    mt.Matrix(cur);
    p.RenderTransform(mt);
    canvas.Children().Append(p);
}
// Windows.UI.Xaml.Media.RectangleGeometry has no corner radius (unlike WPF), so build a rounded
// rect as a path of 4 lines + 4 quarter-arcs.
static WUXM::PathGeometry rounded_rect_geo(double a, double b, double c, double d, double r) {
    double half = (c < d ? c : d) / 2.0; // (windows.h defines min/max macros — avoid std::min)
    if (half < 0) half = 0;
    if (r > half) r = half;
    if (r < 0) r = 0;
    auto pt = [](double x, double y) { return WF::Point{ (float)x, (float)y }; };
    auto line = [&](double x, double y) {
        WUXM::LineSegment s;
        s.Point(pt(x, y));
        return s;
    };
    auto corner = [&](double x, double y) {
        WUXM::ArcSegment s;
        s.Point(pt(x, y));
        s.Size(WF::Size{ (float)r, (float)r });
        s.SweepDirection(WUXM::SweepDirection::Clockwise);
        return s;
    };
    WUXM::PathFigure fig;
    fig.StartPoint(pt(a + r, b));
    fig.IsClosed(true);
    auto segs = fig.Segments();
    segs.Append(line(a + c - r, b));
    segs.Append(corner(a + c, b + r));
    segs.Append(line(a + c, b + d - r));
    segs.Append(corner(a + c - r, b + d));
    segs.Append(line(a + r, b + d));
    segs.Append(corner(a, b + d - r));
    segs.Append(line(a, b + r));
    segs.Append(corner(a + r, b));
    WUXM::PathGeometry pg;
    pg.Figures().Append(fig);
    return pg;
}

static WUI::Color color_argb(unsigned int argb) {
    WUI::Color c{};
    c.A = static_cast<uint8_t>((argb >> 24) & 0xff);
    c.R = static_cast<uint8_t>((argb >> 16) & 0xff);
    c.G = static_cast<uint8_t>((argb >> 8) & 0xff);
    c.B = static_cast<uint8_t>(argb & 0xff);
    return c;
}

// ---------------------------------------------------------------------------
// XAML application: instantiating an Application sets Application::Current, which is what
// loads the framework's default control styles/templates. Without it, templated controls
// (Button/Slider/ToggleSwitch/TextBox) render blank while TextBlock still works. The App also
// owns the WindowsXamlManager. (This is the self-contained analogue of the Windows Community
// Toolkit's XamlApplication — no external component needed for system XAML.)
// ---------------------------------------------------------------------------

namespace WUXMk = winrt::Windows::UI::Xaml::Markup;
namespace WUXI = winrt::Windows::UI::Xaml::Interop;

// DAY_THEME env: 0 = unset (follow the system), 1 = light, 2 = dark.
static int day_theme_env() {
    char theme[16]{};
    DWORD n = GetEnvironmentVariableA("DAY_THEME", theme, sizeof(theme));
    if (n == 0 || n >= sizeof(theme)) return 0;
    if (strcmp(theme, "dark") == 0) return 2;
    if (strcmp(theme, "light") == 0) return 1;
    return 0;
}

// The DAY_THEME force in effect for this process (0 = unset/follow system, 1 = light, 2 = dark),
// captured at window creation. Read by the code-behind theme-brush fills below, which resolve per
// the SYSTEM theme and would otherwise mis-color a forced scheme.
static int g_forced_theme = 0;

struct DayApp : WUX::ApplicationT<DayApp, WUXMk::IXamlMetadataProvider> {
    WUXH::WindowsXamlManager manager{ nullptr };
    // DAY_THEME is forced PER-ELEMENT (ElementTheme on the root Canvas — see day_xaml_window_new),
    // NOT via Application::RequestedTheme: the app-level setter is unsupported under XAML Islands and
    // aborts island init (a fail-fast, not a catchable throw), which makes the app exit before the
    // dayscript engine binds its socket — the walkthrough runner then can't connect (§14).
    DayApp() { manager = WUXH::WindowsXamlManager::InitializeForCurrentThread(); }

    // IXamlMetadataProvider — no custom XAML types to describe.
    WUXMk::IXamlType GetXamlType(WUXI::TypeName const&) { return nullptr; }
    WUXMk::IXamlType GetXamlType(winrt::hstring const&) { return nullptr; }
    winrt::com_array<WUXMk::XmlnsDefinition> GetXmlnsDefinitions() { return {}; }
};

// ---------------------------------------------------------------------------
// window / islands state (single window, v1)
// ---------------------------------------------------------------------------

struct AppWindow {
    HWND host{};
    HWND island{};
    WUXH::DesktopWindowXamlSource source{ nullptr };
    WUX::Application app{ nullptr }; // keeps Application::Current + WindowsXamlManager alive
    void* dqc{}; // DispatcherQueueController — kept alive, never released
    WUXC::Canvas root{ nullptr };
    // Day's tree mounts into `content`, a child canvas offset BELOW any docked MenuBar —
    // both used to share `root` at (0,0), overlapping day's header with File/Edit/View.
    WUXC::Canvas content{ nullptr };
    WUXC::MenuBar menubar{ nullptr };
    // The window toolbar (docs/toolbars.md), docked under the menu bar; `content` clears both.
    WUXC::CommandBar toolbar{ nullptr };
    // Focus parking spot (docs/focus.md): system XAML has no "focus nothing", so resigning
    // hands focus to this invisible non-tab-stop control.
    WUXC::ContentControl focus_sink{ nullptr };
};

static AppWindow* g_app = nullptr;

// The primary window's close, reported to day-core so it can apply the close policy — the same
// deferred teardown a secondary window gets (docs/windows.md).
static void (*g_primary_closed)() = nullptr;

// The open secondary windows (docs/windows.md), keyed by host HWND. A map of POINTERS needs only
// the forward declaration, so it lives up here where the window procedures can both reach it.
struct SecWindow;
static std::map<HWND, SecWindow*> g_sec_windows;

static const UINT WM_DAY_POST = WM_APP + 1;
struct PostMsg { void (*cb)(void*); void* data; };
// Day's window-resize report (single window, v1 — like g_app). UNVERIFIED on a live
// Windows host; mirrors the Qt shim's DayWindow::resizeEvent contract.
static void (*g_resize_cb)(int, int) = nullptr;
// Minimum CLIENT size (points) from WindowOptions.min_size; 0 = no minimum. Single-window v1.
static int g_min_w = 0, g_min_h = 0;
// Lifecycle (docs/lifecycle.md): codes match day_spec::Lifecycle order (2=DidBecomeActive,
// 3=WillResignActive, 7=WillTerminate).
static void (*g_lifecycle_cb)(int) = nullptr;

// Reserve the docked chrome's strips: size the MenuBar and the toolbar CommandBar to the window
// width, stack them at the top, offset day's content canvas below BOTH, and report the REMAINING
// client size to day-core (XAML works in DIPs; the resize report and client rect are physical px
// — convert via the window DPI). Either strip may be absent, and they are installed in either
// order, so both offsets are recomputed here rather than where a bar is created.
/// Lay one window's docked strips out and report the client size that REMAINS for day's content.
///
/// Split out of the primary's relayout so a secondary window (docs/windows.md) lays its chrome
/// out by the same rules: the two windows differ only in how they report the remaining size —
/// the primary through the global resize callback, a secondary through its node id — so that is
/// what stays with each caller and everything else lives here.
static void layout_window_chrome(HWND host, WUXC::Canvas const& content,
                                 WUXC::MenuBar const& menubar, WUXC::CommandBar const& toolbar,
                                 int* out_w, int* out_h) {
    RECT rc; GetClientRect(host, &rc);
    double scale = GetDpiForWindow(host) / 96.0;
    if (scale <= 0) scale = 1.0;
    double mh_dip = 0;
    if (menubar) {
        menubar.Measure(WF::Size{ std::numeric_limits<float>::infinity(),
                                  std::numeric_limits<float>::infinity() });
        mh_dip = menubar.DesiredSize().Height;
        menubar.Width(rc.right / scale);
    }
    double th_dip = 0;
    if (toolbar) {
        // Measured at the real width: a CommandBar lays its commands out against the width it
        // gets, and an infinite proposal would not give the height it will actually draw at.
        toolbar.Measure(WF::Size{ static_cast<float>(rc.right / scale),
                                  std::numeric_limits<float>::infinity() });
        th_dip = toolbar.DesiredSize().Height;
        toolbar.Width(rc.right / scale);
        WUXC::Canvas::SetTop(toolbar, mh_dip);
    }
    if (content) WUXC::Canvas::SetTop(content, mh_dip + th_dip);
    int chrome_px = static_cast<int>(std::lround((mh_dip + th_dip) * scale));
    if (out_w) *out_w = rc.right;
    if (out_h) *out_h = rc.bottom > chrome_px ? rc.bottom - chrome_px : 0;
}

// Reserve the docked chrome's strips on the PRIMARY window and report the remaining client size
// to day-core (XAML works in DIPs; the resize report and client rect are physical px). Either
// strip may be absent, and they are installed in either order, so both offsets are recomputed
// here rather than where a bar is created.
static void day_xaml_relayout_chrome(AppWindow* app) {
    if (!app || !app->host) return;
    int w = 0, h = 0;
    layout_window_chrome(app->host, app->content, app->menubar, app->toolbar, &w, &h);
    if (g_resize_cb) g_resize_cb(w, h);
}

/// Effective light/dark: a DAY_THEME force wins, else the system's CURRENT setting.
///
/// Read from the registry rather than from `Application::RequestedTheme()`. That property is
/// resolved when the XAML app object is constructed and does NOT track a later system flip under
/// Islands — the XAML tree still re-themes itself (its brushes are theme resources), so the
/// property looks right up to the moment you ask it during a change, which is exactly when the
/// title bar needs the new value. `AppsUseLightTheme` is what the ImmersiveColorSet broadcast is
/// announcing, so it is both current and authoritative.
static bool effective_dark() try {
    if (g_forced_theme) return g_forced_theme == 2;
    DWORD light = 1, size = sizeof(light);
    if (RegGetValueW(HKEY_CURRENT_USER,
                     L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
                     L"AppsUseLightTheme", RRF_RT_REG_DWORD, nullptr, &light, &size)
        == ERROR_SUCCESS) {
        return light == 0;
    }
    // Absent on a machine that never set it: fall back to what XAML resolved at startup.
    return WUX::Application::Current().RequestedTheme() == WUX::ApplicationTheme::Dark;
} catch (...) {
    return false;
}

/// Match the Win32 title bar to the effective theme. `DWMWA_USE_IMMERSIVE_DARK_MODE` is a
/// ONE-SHOT attribute, not a binding: a window opened in light mode keeps a light title bar over
/// a dark app until this is set again, which is why WM_SETTINGCHANGE re-runs it. The XAML tree
/// needs no such help — its brushes are theme resources and follow on their own.
static void apply_dark_titlebar(HWND host) {
    BOOL dark = effective_dark() ? TRUE : FALSE;
    DwmSetWindowAttribute(host, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark, sizeof(dark));
}

/// True for the broadcast Windows sends when the light/dark setting flips.
static bool is_color_scheme_change(LPARAM lp) {
    auto area = reinterpret_cast<const wchar_t*>(lp);
    return area && std::wcscmp(area, L"ImmersiveColorSet") == 0;
}

static LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    switch (msg) {
    case WM_SIZE:
        if (g_app && g_app->island) {
            RECT rc; GetClientRect(hwnd, &rc);
            SetWindowPos(g_app->island, nullptr, 0, 0, rc.right, rc.bottom, SWP_SHOWWINDOW);
            day_xaml_relayout_chrome(g_app);
        }
        return 0;
    case WM_GETMINMAXINFO:
        // Enforce WindowOptions.min_size: convert the min CLIENT size to a window size (add the
        // frame) and clamp the minimum track size.
        if (g_min_w > 0 || g_min_h > 0) {
            RECT r{ 0, 0, g_min_w, g_min_h };
            AdjustWindowRectEx(&r, static_cast<DWORD>(GetWindowLongW(hwnd, GWL_STYLE)), FALSE,
                               static_cast<DWORD>(GetWindowLongW(hwnd, GWL_EXSTYLE)));
            auto* mmi = reinterpret_cast<MINMAXINFO*>(lp);
            mmi->ptMinTrackSize.x = r.right - r.left;
            mmi->ptMinTrackSize.y = r.bottom - r.top;
            return 0;
        }
        break;
    case WM_ACTIVATE:
        // Window gained/lost foreground focus → active / resign-active.
        if (g_lifecycle_cb) g_lifecycle_cb(LOWORD(wp) == WA_INACTIVE ? 3 : 2);
        break; // let DefWindowProc handle focus normally
    case WM_SETFOCUS:
        // Hand keyboard focus to the ISLAND. The host is a bare Win32 frame with nothing
        // focusable in it, so without this the caret sits on the host and XAML never sees a
        // keystroke unless a click put focus inside the island first. That is what made the
        // menu bar unreachable from the keyboard: AccessKeyManager only enters access-key
        // display mode (Alt, then Alt+letter) for a focused XAML tree, and Alt on the host went
        // to DefWindowProc, which opens the window's system menu instead.
        if (g_app && g_app->island) {
            SetFocus(g_app->island);
            return 0;
        }
        break;
    case WM_SETTINGCHANGE:
        // The user flipped Windows between light and dark while the app was running.
        if (is_color_scheme_change(lp)) apply_dark_titlebar(hwnd);
        break;
    case WM_CLOSE:
        // Report the close and STOP: no destroy here, no lifecycle-terminate. This window is
        // an ordinary primary window (docs/windows.md close policy), so day-core decides what
        // its closing means — tear down just this window while another primary is open, or end
        // the app — and drives the destroy back through the released root handle. Exactly the
        // deferred teardown a secondary window already gets.
        if (g_primary_closed) g_primary_closed();
        return 0;
    case WM_DAY_POST: {
        auto p = reinterpret_cast<PostMsg*>(lp);
        if (p) { p->cb(p->data); delete p; }
        return 0;
    }
    case WM_DESTROY:
        // NOT the end of the app any more (docs/windows.md close policy): this window is an
        // ordinary primary window, and day-core ends the process — through `day_xaml_quit` —
        // when the LAST primary closes. Quitting here would take the other windows with it.
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

extern "C" void day_xaml_set_lifecycle_cb(void (*cb)(int)) { g_lifecycle_cb = cb; }

extern "C" void day_xaml_set_primary_closed_cb(void (*cb)()) { g_primary_closed = cb; }

// Destroy the primary window's host, once day-core has torn its content down (the released
// root handle is the signal). The counterpart of day_xaml_window_destroy2 for window zero.
extern "C" void day_xaml_destroy_primary() {
    if (g_app && g_app->host) {
        HWND h = g_app->host;
        g_app->host = nullptr;
        DestroyWindow(h);
    }
}

// Open a URL in the system's default handler (browser for http(s), mail app for mailto:, ...).
// Fire and forget — the IAsyncOperation is discarded; an invalid URI throws and is swallowed.
// Backs the `link` piece.
extern "C" void day_xaml_open_url(const char* url) try {
    winrt::Windows::System::Launcher::LaunchUriAsync(
        winrt::Windows::Foundation::Uri(hs(url)));
} catch (...) {}

// App-menu shortcuts on OEM keys, which never reach XAML's accelerator table (see add_accel):
// day_xaml_run's message loop matches them itself. Only the app menu fills this — a context
// menu's shortcuts are labels, and its items come and go with the flyout.
struct OemAccel {
    int key;
    int mods;
    std::function<void()> fire;
};
static std::vector<OemAccel> g_oem_accels;

extern "C" {

void* day_xaml_window_new(const char* title, int w, int h, int min_w, int min_h) try {
    g_min_w = min_w;
    g_min_h = min_h;
    winrt::init_apartment(winrt::apartment_type::single_threaded);

    // XAML requires a DispatcherQueue on the UI thread. Load the flat export dynamically to
    // avoid needing the CoreMessaging import library.
    void* dqc = nullptr;
    if (HMODULE lib = LoadLibraryW(L"CoreMessaging.dll")) {
        using PFN = HRESULT(WINAPI*)(DispatcherQueueOptions,
                                     ABI::Windows::System::IDispatcherQueueController**);
        if (auto fn = reinterpret_cast<PFN>(GetProcAddress(lib, "CreateDispatcherQueueController"))) {
            DispatcherQueueOptions opt{ sizeof(DispatcherQueueOptions),
                                        DQTYPE_THREAD_CURRENT, DQTAT_COM_NONE };
            ABI::Windows::System::IDispatcherQueueController* c = nullptr;
            fn(opt, &c);
            dqc = c;
        }
    }

    // Application must exist before controls so default styles resolve; its ctor also inits
    // the WindowsXamlManager for this thread.
    auto app = winrt::make<DayApp>();
    // Effective light/dark: a DAY_THEME force wins, else the app's system-resolved theme (the
    // RequestedTheme getter reports the system value when nothing was forced). Drives the Win32
    // chrome (transient client brush + dark title bar) and, via g_forced_theme, the code-behind
    // theme-brush fills. The theme itself is forced per-element on the root below, not app-wide.
    g_forced_theme = day_theme_env(); // 0 unset, 1 light, 2 dark
    bool app_dark = g_forced_theme == 2 ||
        (g_forced_theme == 0 &&
         WUX::Application::Current().RequestedTheme() == WUX::ApplicationTheme::Dark);

    WNDCLASSW wc{};
    wc.lpfnWndProc = WndProc;
    wc.hInstance = GetModuleHandleW(nullptr);
    wc.lpszClassName = L"day_xaml_host";
    wc.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    // Transient fill behind the island (resize gaps, pre-first-frame): Win32 has no
    // theme-adaptive client brush, so pick the stock object nearest the XAML page ground.
    wc.hbrBackground = app_dark ? reinterpret_cast<HBRUSH>(GetStockObject(BLACK_BRUSH))
                                : reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
    RegisterClassW(&wc);

    DWORD style = WS_OVERLAPPEDWINDOW; // resizable; WM_SIZE reflows the island + day tree
    RECT r{ 0, 0, w, h };
    AdjustWindowRect(&r, style, FALSE);
    HWND host = CreateWindowExW(0, L"day_xaml_host", hs(title).c_str(), style,
                                CW_USEDEFAULT, CW_USEDEFAULT, r.right - r.left, r.bottom - r.top,
                                nullptr, nullptr, wc.hInstance, nullptr);

    WUXH::DesktopWindowXamlSource source;
    auto interop = source.as<::IDesktopWindowXamlSourceNative>();
    interop->AttachToWindow(host);
    HWND island = nullptr;
    interop->get_WindowHandle(&island);
    RECT rc; GetClientRect(host, &rc);
    SetWindowPos(island, nullptr, 0, 0, rc.right, rc.bottom, SWP_SHOWWINDOW);

    // Dark title bars are opt-in for Win32 windows; match the app theme (no-op pre-1809).
    // WM_SETTINGCHANGE re-runs this when the system theme flips.
    apply_dark_titlebar(host);

    // Mica: the Windows 11 window material, and the counterpart of the sidebar material AppKit
    // supplies (docs/navigation.md). DWMSBT_MAINWINDOW is the backdrop a document-shaped app
    // uses; the call fails harmlessly on Windows 10 and on 11 before 22621, where the attribute
    // does not exist — which is exactly why the RESULT decides what happens next. Mica only
    // shows through content that is transparent, so the grounding below is skipped when (and
    // only when) the backdrop was accepted; if it was refused, the opaque ground stays and the
    // window looks as it always has.
    bool mica = false;
    {
        DWORD backdrop = DWMSBT_MAINWINDOW;
        mica = SUCCEEDED(DwmSetWindowAttribute(host, DWMWA_SYSTEMBACKDROP_TYPE, &backdrop,
                                               sizeof(backdrop)));
    }

    WUXC::Canvas root;
    // Force DAY_THEME PER-ELEMENT (islands-safe, unlike Application::RequestedTheme): ElementTheme
    // on the root cascades to every descendant control + its {ThemeResource} lookups, so the whole
    // tree renders in the forced scheme. Unset => Default (follows the system).
    switch (g_forced_theme) {
        case 2: root.RequestedTheme(WUX::ElementTheme::Dark); break;
        case 1: root.RequestedTheme(WUX::ElementTheme::Light); break;
    }
    source.Content(root);
    // Ground the island: a Canvas paints nothing itself, and the raw HWND behind it is white — under
    // a dark tree that white would ghost through. The named page-background brush resolves per the
    // SYSTEM theme, so it is only trustworthy when unforced; when DAY_THEME forces a scheme, ground
    // with a solid neutral matching it (the Fluent page-base color for that scheme).
    {
        // Mica accepted ⇒ leave the root transparent so the material shows through it.
        bool grounded = mica;
        if (!mica && g_forced_theme == 0) {
            auto res = WUX::Application::Current().Resources();
            auto key = winrt::box_value(winrt::hstring(L"ApplicationPageBackgroundThemeBrush"));
            if (res.HasKey(key)) {
                if (auto brush = res.Lookup(key).try_as<WUXM::Brush>()) {
                    root.Background(brush);
                    grounded = true;
                }
            }
        }
        if (!grounded)
            root.Background(WUXM::SolidColorBrush(
                color_argb(app_dark ? 0xFF'202020u : 0xFF'F3F3F3u)));
    }

    // Load the island NOW, before day builds the control tree. Controls added to a live,
    // loaded tree get their default styles/templates applied immediately, so day's first
    // (synchronous) Measure returns real sizes. Without this, templated controls measure to 0
    // and lay out invisible. Pump until the root's Loaded event fires (bounded).
    ShowWindow(host, SW_SHOWNORMAL);
    UpdateWindow(host);
    auto loaded = std::make_shared<bool>(false);
    auto token = root.Loaded([loaded](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
        *loaded = true;
    });
    {
        MSG msg{};
        ULONGLONG start = GetTickCount64();
        while (!*loaded && GetTickCount64() - start < 4000) {
            if (PeekMessageW(&msg, nullptr, 0, 0, PM_REMOVE)) {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            } else {
                Sleep(1);
            }
        }
    }
    root.Loaded(token);

    auto aw = new AppWindow();
    aw->host = host;
    aw->island = island;
    aw->source = source;
    aw->app = app;
    aw->dqc = dqc;
    aw->root = root;
    WUXC::Canvas content;
    WUXC::Canvas::SetLeft(content, 0);
    WUXC::Canvas::SetTop(content, 0);
    root.Children().Append(content);
    aw->content = content;
    // The focus sink: 1×1, transparent, off the visible canvas, never a tab stop (resign
    // flips IsTabStop around a programmatic Focus call, then removes it from tab order again).
    {
        WUXC::ContentControl sink;
        sink.Width(1);
        sink.Height(1);
        sink.Opacity(0);
        sink.IsTabStop(false);
        WUXC::Canvas::SetLeft(sink, -100);
        WUXC::Canvas::SetTop(sink, -100);
        root.Children().Append(sink);
        aw->focus_sink = sink;
    }
    g_app = aw;
    return aw;
} catch (winrt::hresult_error const& e) {
    std::string msg = u8(e.message());
    std::fprintf(stderr, "day-xaml: XAML Islands init failed: hr=0x%08X %s\n",
                 static_cast<unsigned>(e.code().value), msg.c_str());
    std::fflush(stderr);
    return nullptr;
} catch (...) {
    std::fprintf(stderr, "day-xaml: XAML Islands init failed (unknown C++ exception)\n");
    std::fflush(stderr);
    return nullptr;
}

void* day_xaml_window_root(void* win) {
    auto app = reinterpret_cast<AppWindow*>(win);
    return boxh(app->content);
}

void day_xaml_window_on_resize(void* win, void (*cb)(int, int)) {
    (void)win; // single window (v1)
    g_resize_cb = cb;
}
void day_xaml_window_show(void* win) {
    auto app = reinterpret_cast<AppWindow*>(win);
    ShowWindow(app->host, SW_SHOWNORMAL);
    UpdateWindow(app->host);
}

// --- secondary windows (docs/windows.md) --------------------------------------------------
// Each is its own Win32 host + DesktopWindowXamlSource island (multiple islands per thread
// are supported) carrying a day root Canvas. Events route to per-window callbacks keyed by
// the day node id; close HIDES (Rust drives destruction when day releases the content).
// Known v1 limit: the primary's message loop PreTranslateMessage targets the primary
// island only, so keyboard accelerators inside secondary islands may be degraded.

// Live toolbar item elements by id, for the targeted patches (search text, toggle state,
// enabled). Each install rebuilds its window's map; the map holds the only reference day keeps to
// those elements.
//
// Keyed BY WINDOW, because item ids are only unique within one toolbar: every window an app opens
// installs the same item list (the showcase's `register_new_window` re-runs its toolbar install),
// so a single id→element map would let the newest window's items answer patches aimed at an older
// window's — and clearing it per install would strand the older window's patches entirely.
// Declared here rather than beside the toolbar builder because a window's teardown drops its
// entry, and that runs further up this file.
using ToolbarElems = std::map<std::string, FrameworkElement>;
static std::map<void*, ToolbarElems> g_toolbar_elems;

struct SecWindow {
    HWND host{};
    HWND island{};
    WUXH::DesktopWindowXamlSource source{ nullptr };
    WUXC::Canvas root{ nullptr };
    // Day's tree mounts into `content`, a child canvas offset BELOW the docked chrome — the
    // primary window's arrangement, so a secondary window can carry a menu bar and a toolbar
    // instead of being a bare island (docs/windows.md).
    WUXC::Canvas content{ nullptr };
    WUXC::MenuBar menubar{ nullptr };
    WUXC::CommandBar toolbar{ nullptr };
    unsigned long long node = 0;
};
static void (*g_win_resized)(unsigned long long, int, int) = nullptr;
static void (*g_win_closed)(unsigned long long) = nullptr;
static void (*g_win_focused)(unsigned long long, int) = nullptr;

void day_xaml_set_window_events_cb(void (*resized)(unsigned long long, int, int),
                                   void (*closed)(unsigned long long),
                                   void (*focused)(unsigned long long, int)) {
    g_win_resized = resized;
    g_win_closed = closed;
    g_win_focused = focused;
}

/// The host HWND of the window `e` is in, or null. Each window is its own XAML island with its
/// own XamlRoot, which is what tells them apart — the same handle the sidebar toggle resolves by.
/// Used for the menu commands that act on THEIR window rather than the app.
static HWND host_for_element(WUX::FrameworkElement const& e) try {
    if (!e) return nullptr;
    auto root = e.XamlRoot();
    if (!root) return nullptr;
    if (g_app && g_app->root && g_app->root.XamlRoot() == root) return g_app->host;
    for (auto const& [hwnd, sw] : g_sec_windows) {
        if (sw && sw->root && sw->root.XamlRoot() == root) return hwnd;
    }
    return nullptr;
} catch (...) {
    return nullptr;
}

// Lay this window's chrome out and report the size that remains for day's content — the
// secondary counterpart of day_xaml_relayout_chrome, differing only in where the size goes.
static void relayout_sec_chrome(SecWindow* sw) {
    if (!sw || !sw->host) return;
    int w = 0, h = 0;
    layout_window_chrome(sw->host, sw->content, sw->menubar, sw->toolbar, &w, &h);
    if (g_win_resized) g_win_resized(sw->node, w, h);
}

static LRESULT CALLBACK SecWndProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    auto it = g_sec_windows.find(hwnd);
    SecWindow* sw = it == g_sec_windows.end() ? nullptr : it->second;
    switch (msg) {
    case WM_SIZE:
        if (sw && sw->island) {
            RECT rc; GetClientRect(hwnd, &rc);
            SetWindowPos(sw->island, nullptr, 0, 0, rc.right, rc.bottom, SWP_SHOWWINDOW);
            // Reports the size BELOW the chrome, not the whole client, or day would lay its
            // tree out under the menu bar.
            relayout_sec_chrome(sw);
        }
        return 0;
    case WM_ACTIVATE:
        if (sw && g_win_focused) g_win_focused(sw->node, LOWORD(wp) != WA_INACTIVE ? 1 : 0);
        break;
    case WM_CLOSE:
        // Confirm to day (its teardown is deferred); HIDE — no destroy here, no
        // lifecycle-terminate, no PostQuitMessage (those are primary-only semantics).
        if (sw && g_win_closed) g_win_closed(sw->node);
        ShowWindow(hwnd, SW_HIDE);
        return 0;
    case WM_SETFOCUS:
        // Same routing as the primary: the island, not the bare host frame, takes the keyboard.
        if (sw && sw->island) {
            SetFocus(sw->island);
            return 0;
        }
        break;
    case WM_SETTINGCHANGE:
        // Every window owns its own title bar, so each one re-applies the attribute.
        if (is_color_scheme_change(lp)) apply_dark_titlebar(hwnd);
        break;
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

void* day_xaml_window_new2(const char* title, int w, int h,
                           unsigned long long node, int fixed) try {
    bool app_dark = g_forced_theme == 2 ||
        (g_forced_theme == 0 &&
         WUX::Application::Current().RequestedTheme() == WUX::ApplicationTheme::Dark);
    static bool registered = false;
    if (!registered) {
        WNDCLASSW wc{};
        wc.lpfnWndProc = SecWndProc;
        wc.hInstance = GetModuleHandleW(nullptr);
        wc.lpszClassName = L"day_xaml_win2";
        wc.hCursor = LoadCursorW(nullptr, IDC_ARROW);
        wc.hbrBackground = app_dark ? reinterpret_cast<HBRUSH>(GetStockObject(BLACK_BRUSH))
                                    : reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
        RegisterClassW(&wc);
        registered = true;
    }
    // `fixed` is WindowKind::Preferences — a PANEL, not a second main window. docs/windows.md
    // has it "drop resize/minimize", and Windows' own settings dialogs add to that: owned by the
    // window they belong to, so they float above it and take no taskbar button of their own. An
    // owner is passed as hWndParent WITHOUT WS_CHILD, which is what makes it owned rather than
    // parented. A Normal window gets none of this — it is an independent main window.
    DWORD style = WS_OVERLAPPEDWINDOW;
    HWND owner = nullptr;
    if (fixed) {
        style &= ~(WS_THICKFRAME | WS_MAXIMIZEBOX | WS_MINIMIZEBOX);
        owner = g_app ? g_app->host : nullptr;
    }
    RECT r{ 0, 0, w, h };
    AdjustWindowRect(&r, style, FALSE);
    HWND host = CreateWindowExW(0, L"day_xaml_win2", hs(title).c_str(), style,
                                CW_USEDEFAULT, CW_USEDEFAULT, r.right - r.left,
                                r.bottom - r.top, owner, nullptr,
                                GetModuleHandleW(nullptr), nullptr);
    if (!host) return nullptr;

    WUXH::DesktopWindowXamlSource source;
    auto interop = source.as<::IDesktopWindowXamlSourceNative>();
    interop->AttachToWindow(host);
    HWND island = nullptr;
    interop->get_WindowHandle(&island);
    RECT rc; GetClientRect(host, &rc);
    SetWindowPos(island, nullptr, 0, 0, rc.right, rc.bottom, SWP_SHOWWINDOW);
    apply_dark_titlebar(host); // re-applied on WM_SETTINGCHANGE, as for the primary
    WUXC::Canvas root;
    switch (g_forced_theme) {
        case 2: root.RequestedTheme(WUX::ElementTheme::Dark); break;
        case 1: root.RequestedTheme(WUX::ElementTheme::Light); break;
    }
    source.Content(root);
    // Solid neutral ground matching the scheme (the primary's themed-brush path needs the
    // unforced system lookup; a solid is correct in both cases and keeps this path simple).
    root.Background(WUXM::SolidColorBrush(
        color_argb(app_dark ? 0xFF'202020u : 0xFF'F3F3F3u)));

    // Load the island before day builds (see day_xaml_window_new: unloaded templated
    // controls measure to 0). Bounded pump.
    ShowWindow(host, SW_SHOWNORMAL);
    UpdateWindow(host);
    // ACTIVATE it, which ShowWindow alone does not reliably do for a window created by a
    // background thread's request. Without this the new window appears while the window that
    // opened it keeps focus — so the next menu command goes to the OLD window. `File ▸ New
    // Window` then `File ▸ Close` read as "close the window I just opened" but closed the
    // PRIMARY, and closing the primary ends the app (above), taking the new window with it.
    SetForegroundWindow(host);
    auto loaded = std::make_shared<bool>(false);
    auto token = root.Loaded([loaded](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
        *loaded = true;
    });
    {
        MSG msg{};
        ULONGLONG start = GetTickCount64();
        while (!*loaded && GetTickCount64() - start < 2000) {
            if (PeekMessageW(&msg, nullptr, 0, 0, PM_REMOVE)) {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            } else {
                Sleep(1);
            }
        }
    }
    root.Loaded(token);

    // Day's tree goes in a child canvas, not in `root` — `root` also carries the docked menu bar
    // and toolbar, and relayout slides this one below them.
    WUXC::Canvas content;
    root.Children().Append(content);

    auto sw = new SecWindow();
    sw->host = host;
    sw->island = island;
    sw->source = source;
    sw->root = root;
    sw->content = content;
    sw->node = node;
    g_sec_windows[host] = sw;
    return sw;
} catch (...) {
    std::fprintf(stderr, "day-xaml: secondary window init failed\n");
    std::fflush(stderr);
    return nullptr;
}

void* day_xaml_window_content2(void* win) {
    return boxh(static_cast<SecWindow*>(win)->content);
}
void day_xaml_window_close2(void* win) {
    PostMessageW(static_cast<SecWindow*>(win)->host, WM_CLOSE, 0, 0);
}
void day_xaml_window_raise2(void* win) {
    HWND h = static_cast<SecWindow*>(win)->host;
    ShowWindow(h, SW_SHOWNORMAL);
    SetForegroundWindow(h);
}
void day_xaml_window_set_title2(void* win, const char* title) {
    SetWindowTextW(static_cast<SecWindow*>(win)->host, hs(title).c_str());
}
void day_xaml_window_destroy2(void* win) {
    auto sw = static_cast<SecWindow*>(win);
    g_sec_windows.erase(sw->host);
    // This window's toolbar items go with it, or the map grows by a full item set on every
    // open/close cycle and holds those elements alive for the process's life.
    g_toolbar_elems.erase(win);
    // Close the island BEFORE the host window goes. The DesktopWindowXamlSource owns a child
    // HWND of `host`, so DestroyWindow takes that window out from under XAML and leaves the
    // source to tear down an island whose HWND no longer exists — Close() first is the order
    // the API documents. This is the ONLY teardown that runs while the process keeps living:
    // the primary reaches its own destroy on the way out, where nothing outlives the mistake.
    if (sw->source) {
        try {
            sw->source.Close();
        } catch (...) {
        }
    }
    DestroyWindow(sw->host);
    delete sw;
}

// App icon (§18.2): title-bar + taskbar icon for the unbundled Win32 host window, loaded from the
// multi-size .ico that `day launch` resolves from the project's icons/windows/ (DAY_APP_ICON).
void day_xaml_set_app_icon(void* win, const char* ico_path) {
    auto app = reinterpret_cast<AppWindow*>(win);
    if (!app || !app->host || !ico_path || !*ico_path) return;
    // LR_DEFAULTSIZE picks the right frame from the multi-size .ico per use (32 big / 16 small).
    HICON big = (HICON)LoadImageW(nullptr, hs(ico_path).c_str(), IMAGE_ICON, 0, 0,
                                  LR_LOADFROMFILE | LR_DEFAULTSIZE);
    HICON small_ = (HICON)LoadImageW(nullptr, hs(ico_path).c_str(), IMAGE_ICON,
                                     GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON),
                                     LR_LOADFROMFILE);
    if (big) SendMessageW(app->host, WM_SETICON, ICON_BIG, (LPARAM)big);
    if (small_) SendMessageW(app->host, WM_SETICON, ICON_SMALL, (LPARAM)small_);
}

// Top-level host HWND, for a piece that needs the window handle behind the XAML island — the WebView2
// web view passes it as the composition controller's parentWindow (DPI / IME / input association),
// while the page renders windowless into a composition visual spliced into the XAML tree. Single
// window (v1), via g_app.
void* day_xaml_host_hwnd() { return g_app ? reinterpret_cast<void*>(g_app->host) : nullptr; }

void day_xaml_run(void* win) {
    auto app = reinterpret_cast<AppWindow*>(win);
    auto interop2 = app->source.as<::IDesktopWindowXamlSourceNative2>();
    MSG msg{};
    while (GetMessageW(&msg, nullptr, 0, 0)) {
        BOOL handled = FALSE;
        // Menu shortcuts XAML could not take as accelerators (OEM keys — see add_accel). Checked
        // ahead of PreTranslateMessage so the island cannot route the keystroke somewhere else
        // first; every registered combination carries a modifier, so plain typing is untouched.
        if (msg.message == WM_KEYDOWN || msg.message == WM_SYSKEYDOWN) {
            int mods = 0;
            if (GetKeyState(VK_CONTROL) & 0x8000) mods |= 1;
            if (GetKeyState(VK_SHIFT) & 0x8000) mods |= 2;
            if (GetKeyState(VK_MENU) & 0x8000) mods |= 4;
            for (auto const& a : g_oem_accels) {
                if (a.key == static_cast<int>(msg.wParam) && a.mods == mods) {
                    a.fire();
                    handled = TRUE;
                    break;
                }
            }
        }
        if (!handled && interop2) interop2->PreTranslateMessage(&msg, &handled);
        if (!handled) {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

// End the app (docs/windows.md close policy). day-core calls this once the last primary
// window has closed, having already disposed the rest and delivered WillTerminate — so this
// ends the message loop and nothing more. Deliberately NOT a WM_CLOSE to the primary: that
// would fire the terminate lifecycle a second time.
void day_xaml_quit() { PostQuitMessage(0); }

void day_xaml_post(void (*cb)(void*), void* data) {
    if (g_app && g_app->host) {
        PostMessageW(g_app->host, WM_DAY_POST, 0, reinterpret_cast<LPARAM>(new PostMsg{ cb, data }));
    }
}

// ---- containers ----

void* day_xaml_container_new() { WUXC::Canvas c; return boxh(c); }
// A vertical ScrollViewer wrapping a content Canvas (like the list host). day positions the
// content's children by absolute frame and reports the content extent via set_content_size; the
// ScrollViewer then clips + scrolls. Horizontal scrolling is disabled (day's scroll is vertical,
// matching the Qt/AppKit backends). out_content receives the inner Canvas — day adds children there.
void* day_xaml_scroll_new(void** out_content, int horizontal) {
    WUXC::ScrollViewer sv;
    if (horizontal) {
        sv.HorizontalScrollBarVisibility(WUXC::ScrollBarVisibility::Auto);
        sv.VerticalScrollBarVisibility(WUXC::ScrollBarVisibility::Disabled);
        sv.HorizontalScrollMode(WUXC::ScrollMode::Enabled);
        sv.VerticalScrollMode(WUXC::ScrollMode::Disabled);
    } else {
        sv.HorizontalScrollBarVisibility(WUXC::ScrollBarVisibility::Disabled);
        sv.VerticalScrollBarVisibility(WUXC::ScrollBarVisibility::Auto);
    }
    WUXC::Canvas content;
    sv.Content(content);
    if (out_content) *out_content = boxh(content);
    return boxh(sv);
}
void day_xaml_scroll_set_content_size(void* content, int w, int h) {
    if (auto fe = elem(content).try_as<FrameworkElement>()) {
        fe.Width(static_cast<double>(w));
        fe.Height(static_cast<double>(h));
    }
}
void day_xaml_scroll_offset(void* sv, double* out_x, double* out_y) {
    *out_x = 0;
    *out_y = 0;
    if (auto s = elem(sv).try_as<WUXC::ScrollViewer>()) {
        *out_x = s.HorizontalOffset();
        *out_y = s.VerticalOffset();
    }
}
void day_xaml_scroll_to(void* sv, int y, int h, int animated) {
    // Scroll the minimum to make [y, y+h] visible (NSScrollView scrollRectToVisible semantics).
    auto s = elem(sv).try_as<WUXC::ScrollViewer>();
    if (!s) return;
    double vh = s.ViewportHeight(), off = s.VerticalOffset(), target = off;
    if (y < off) target = y;
    else if (y + h > off + vh) target = y + h - vh;
    if (target != off) s.ChangeView(nullptr, target, nullptr, animated == 0);
}
void* day_xaml_canvas_new() { WUXC::Canvas c; return boxh(c); }

void day_xaml_canvas_set_ops(void* h, const double* nums, int n, const char* texts_joined) {
    auto canvas = elem(h).try_as<WUXC::Canvas>();
    if (!canvas) return;
    canvas.Children().Clear();

    std::vector<std::string> texts;
    {
        std::string all = texts_joined ? texts_joined : "";
        size_t start = 0;
        while (start <= all.size()) {
            size_t nl = all.find('\x1f', start);
            texts.push_back(all.substr(start, nl == std::string::npos ? std::string::npos : nl - start));
            if (nl == std::string::npos) break;
            start = nl + 1;
        }
    }
    size_t ti = 0;
    std::vector<WUXM::Matrix> stack;
    WUXM::Matrix cur = mat_identity();
    const double DEG = 3.14159265358979323846 / 180.0;
    // A decoded kind-14 record (set-gradient), consumed by the NEXT fill-shape record. XAML's
    // default MappingMode is RelativeToBoundingBox, so the encoded unit geometry maps onto each
    // shape's bounds with no extra math; make_radial_brush mirrors that with Stretch::Fill so both
    // gradient kinds turn naturally elliptical in non-square bounds, the shared rule.
    WUXM::Brush gradPending{ nullptr };
    auto fill_brush = [&](unsigned col) -> WUXM::Brush {
        if (gradPending) {
            WUXM::Brush b = gradPending;
            gradPending = nullptr;
            return b;
        }
        return brush_bits(col);
    };

    for (int i = 0; i + 8 < n; i += 9) {
        int k = static_cast<int>(nums[i]);
        double a = nums[i + 1], b = nums[i + 2], c = nums[i + 3], d = nums[i + 4];
        double e = nums[i + 5], f = nums[i + 6], g = nums[i + 7];
        unsigned col = static_cast<unsigned>(nums[i + 8]);
        switch (k) {
        case 8:
            stack.push_back(cur);
            break;
        case 9:
            if (!stack.empty()) {
                cur = stack.back();
                stack.pop_back();
            }
            break;
        case 10: {
            WUXM::Matrix m{};
            m.M11 = a;
            m.M12 = b;
            m.M21 = c;
            m.M22 = d;
            m.OffsetX = e;
            m.OffsetY = f;
            cur = mat_mul(m, cur);
            break;
        }
        case 0:
        case 1:
        case 2:
        case 13: {
            WUXSh::Path p;
            if (k == 2 || k == 13) {
                p.Data(rounded_rect_geo(a, b, c, d, e));
            } else {
                WUXM::RectangleGeometry rg;
                rg.Rect(WF::Rect{ (float)a, (float)b, (float)c, (float)d });
                p.Data(rg);
            }
            if (k == 1 || k == 13) {
                p.Stroke(brush_bits(col));
                p.StrokeThickness(g);
            } else {
                p.Fill(fill_brush(col));
            }
            place_shape(canvas, p, cur);
            break;
        }
        case 3:
        case 4: {
            WUXM::EllipseGeometry eg;
            eg.Center(WF::Point{ (float)(a + c / 2), (float)(b + d / 2) });
            eg.RadiusX(c / 2);
            eg.RadiusY(d / 2);
            WUXSh::Path p;
            p.Data(eg);
            if (k == 4) {
                p.Stroke(brush_bits(col));
                p.StrokeThickness(g);
            } else {
                p.Fill(fill_brush(col));
            }
            place_shape(canvas, p, cur);
            break;
        }
        case 5: { // stroke arc (e=start°, f=sweep°); clockwise, 0=+x, in screen (y-down) space
            double cx = a + c / 2, cy = b + d / 2, rx = c / 2, ry = d / 2;
            double s = e * DEG, en = (e + f) * DEG;
            WUXM::ArcSegment arc;
            arc.Point(WF::Point{ (float)(cx + rx * cos(en)), (float)(cy + ry * sin(en)) });
            arc.Size(WF::Size{ (float)rx, (float)ry });
            arc.IsLargeArc(fabs(f) > 180.0);
            arc.SweepDirection(f >= 0 ? WUXM::SweepDirection::Clockwise
                                      : WUXM::SweepDirection::Counterclockwise);
            WUXM::PathFigure fig;
            fig.StartPoint(WF::Point{ (float)(cx + rx * cos(s)), (float)(cy + ry * sin(s)) });
            fig.IsClosed(false);
            fig.Segments().Append(arc);
            WUXM::PathGeometry pg;
            pg.Figures().Append(fig);
            WUXSh::Path p;
            p.Data(pg);
            p.Stroke(brush_bits(col));
            p.StrokeThickness(g);
            p.StrokeStartLineCap(WUXM::PenLineCap::Round);
            p.StrokeEndLineCap(WUXM::PenLineCap::Round);
            place_shape(canvas, p, cur);
            break;
        }
        case 6: { // line
            WUXM::LineGeometry lg;
            lg.StartPoint(WF::Point{ (float)a, (float)b });
            lg.EndPoint(WF::Point{ (float)c, (float)d });
            WUXSh::Path p;
            p.Data(lg);
            p.Stroke(brush_bits(col));
            p.StrokeThickness(g);
            p.StrokeStartLineCap(WUXM::PenLineCap::Round);
            p.StrokeEndLineCap(WUXM::PenLineCap::Round);
            place_shape(canvas, p, cur);
            break;
        }
        case 7: { // text at (a,b); e=size, f=anchor (0 leading / 1 centered)
            std::string t = ti < texts.size() ? texts[ti++] : std::string();
            WUXC::TextBlock tb;
            tb.Text(hs(t.c_str()));
            tb.FontSize(e);
            tb.Foreground(brush_bits(col));
            // Fold the CTM into the anchor point (glyph rotation is a follow-up; the demos draw
            // upright text under an identity CTM).
            double px = a * cur.M11 + b * cur.M21 + cur.OffsetX;
            double py = a * cur.M12 + b * cur.M22 + cur.OffsetY;
            if (f > 0.5) {
                tb.Measure(WF::Size{ std::numeric_limits<float>::infinity(),
                                     std::numeric_limits<float>::infinity() });
                auto ds = tb.DesiredSize();
                px -= ds.Width / 2;
                py -= ds.Height / 2;
            }
            WUXC::Canvas::SetLeft(tb, px);
            WUXC::Canvas::SetTop(tb, py);
            canvas.Children().Append(tb);
            break;
        }
        case 11:
        case 12: { // polygon (11 fill / 12 stroke); points ride texts as "x,y x,y ..."
            std::string t = ti < texts.size() ? texts[ti++] : std::string();
            WUXM::PathFigure fig;
            fig.IsClosed(true);
            bool first = true;
            size_t pos = 0;
            while (pos < t.size()) {
                size_t sp = t.find(' ', pos);
                std::string pair = t.substr(pos, sp == std::string::npos ? std::string::npos : sp - pos);
                pos = sp == std::string::npos ? t.size() : sp + 1;
                size_t comma = pair.find(',');
                if (comma == std::string::npos || comma == 0) continue;
                // std::from_chars: locale-independent (atof honors LC_NUMERIC).
                float x = 0.0f, y = 0.0f;
                std::from_chars(pair.data(), pair.data() + comma, x);
                std::from_chars(pair.data() + comma + 1, pair.data() + pair.size(), y);
                if (first) {
                    fig.StartPoint(WF::Point{ x, y });
                    first = false;
                } else {
                    WUXM::LineSegment seg;
                    seg.Point(WF::Point{ x, y });
                    fig.Segments().Append(seg);
                }
            }
            if (!first) {
                WUXM::PathGeometry pg;
                pg.Figures().Append(fig);
                WUXSh::Path p;
                p.Data(pg);
                if (k == 12) {
                    p.Stroke(brush_bits(col));
                    p.StrokeThickness(g);
                } else {
                    p.Fill(fill_brush(col));
                }
                place_shape(canvas, p, cur);
            }
            break;
        }
        case 14: { // set-gradient (f = type): stops ride texts as "offset,aarrggbb offset,aarrggbb ..."
            std::string t = ti < texts.size() ? texts[ti++] : std::string();
            // Parse the shared stop format once, into whichever brush the type selects.
            auto parse_stops = [&](auto appendStop) {
                size_t pos = 0, count = 0;
                while (pos < t.size()) {
                    size_t sp = t.find(' ', pos);
                    std::string pair = t.substr(pos, sp == std::string::npos ? std::string::npos : sp - pos);
                    pos = sp == std::string::npos ? t.size() : sp + 1;
                    size_t comma = pair.find(',');
                    if (comma == std::string::npos || comma == 0) continue;
                    float off = 0.0f;
                    std::from_chars(pair.data(), pair.data() + comma, off);
                    unsigned bits = 0;
                    std::from_chars(pair.data() + comma + 1, pair.data() + pair.size(), bits, 16);
                    WUXM::GradientStop gs;
                    gs.Offset(off);
                    WUI::Color cc;
                    cc.A = static_cast<uint8_t>((bits >> 24) & 0xff);
                    cc.R = static_cast<uint8_t>((bits >> 16) & 0xff);
                    cc.G = static_cast<uint8_t>((bits >> 8) & 0xff);
                    cc.B = static_cast<uint8_t>(bits & 0xff);
                    gs.Color(cc);
                    appendStop(gs);
                    count++;
                }
                return count;
            };
            if ((int)f == 1) {
                std::vector<std::pair<float, WUI::Color>> stops;
                if (parse_stops([&](WUXM::GradientStop gs) {
                        stops.emplace_back(static_cast<float>(gs.Offset()), gs.Color());
                    }) >= 2)
                    gradPending = make_radial_brush(a, b, c, std::move(stops));
            } else {
                WUXM::LinearGradientBrush lgb;
                lgb.StartPoint(WF::Point{ (float)a, (float)b });
                lgb.EndPoint(WF::Point{ (float)c, (float)d });
                if (parse_stops([&](WUXM::GradientStop gs) { lgb.GradientStops().Append(gs); }) >= 2)
                    gradPending = lgb;
            }
            break;
        }
        }
    }
}

// Recycling-list host: a real ScrollViewer whose Content is a Canvas that holds the row cells
// (day positions each cell by absolute frame). `out_content` receives a handle to that Canvas so
// the Rust side can add/position cells; the list drives scrolling via the content's extent.
void* day_xaml_list_new(void** out_content) {
    WUXC::ScrollViewer sv;
    sv.HorizontalScrollBarVisibility(WUXC::ScrollBarVisibility::Disabled);
    sv.VerticalScrollBarVisibility(WUXC::ScrollBarVisibility::Auto);
    WUXC::Canvas content;
    sv.Content(content);
    if (out_content) *out_content = boxh(content);
    return boxh(sv);
}
void day_xaml_list_set_content_size(void* content, int w, int h) {
    if (auto fe = elem(content).try_as<FrameworkElement>()) {
        fe.Width(static_cast<double>(w));
        fe.Height(static_cast<double>(h));
    }
}

// --- emulated list drag-to-reorder (docs/list.md) ---
// The real WinRT drag pipeline (CanDrag / DragOver / Drop — the same visuals every Windows app
// gets) over the emulated Canvas list. The DECISIONS stay Rust's: every hovered slot is vetted
// synchronously through the can-move callback (the app's guard; a denied slot answers
// DataPackageOperation::None so the system shows the no-drop cursor live), and the drop commits
// through the move callback.
typedef int (*DayListCanMoveCb)(unsigned long long id, int from, int to);
typedef void (*DayListMoveCb)(unsigned long long id, int from, int to);

struct DayListDragState {
    int from = -1;
};
static std::map<unsigned long long, DayListDragState> g_list_drags; // keyed by day list node id

void day_xaml_list_enable_reorder(void* content, unsigned long long id, int row_h,
                                  DayListCanMoveCb can, DayListMoveCb mv) {
    auto canvas = elem(content).try_as<WUXC::Canvas>();
    if (!canvas || row_h <= 0) return;
    canvas.AllowDrop(true);
    canvas.DragOver([id, row_h, can](WF::IInspectable const& sender,
                                     WUX::DragEventArgs const& e) {
        auto& st = g_list_drags[id];
        if (st.from < 0) return;
        auto c = sender.try_as<WUX::UIElement>();
        if (!c) return;
        int slot = (int)(e.GetPosition(c).Y) / row_h;
        e.AcceptedOperation(
            can(id, st.from, slot) >= 0
                ? winrt::Windows::ApplicationModel::DataTransfer::DataPackageOperation::Move
                : winrt::Windows::ApplicationModel::DataTransfer::DataPackageOperation::None);
    });
    canvas.Drop([id, row_h, can, mv](WF::IInspectable const& sender,
                                     WUX::DragEventArgs const& e) {
        auto& st = g_list_drags[id];
        int from = st.from;
        st.from = -1;
        if (from < 0) return;
        auto c = sender.try_as<WUX::UIElement>();
        if (!c) return;
        int slot = (int)(e.GetPosition(c).Y) / row_h;
        int accepted = can(id, from, slot);
        if (accepted >= 0 && accepted != from) mv(id, from, accepted);
    });
}

void day_xaml_cell_drag(void* cell, unsigned long long id, int row) {
    auto el = elem(cell).try_as<WUX::UIElement>();
    if (!el) return;
    el.CanDrag(true);
    el.DragStarting([id, row](WUX::UIElement const&, WUX::DragStartingEventArgs const& args) {
        g_list_drags[id].from = row;
        args.Data().RequestedOperation(
            winrt::Windows::ApplicationModel::DataTransfer::DataPackageOperation::Move);
    });
}

// --- emulated list row selection (docs/list.md) ---
// A press on a list cell reports (list node, row, modifiers); the Rust side owns every selection
// DECISION (replace / toggle / extend) and calls back to paint the result, so the shim holds no
// selection state. modifiers: bit 0 = ctrl (toggle), bit 1 = shift (range).
typedef void (*DayRowClickCb)(unsigned long long id, int row, int modifiers);

// Defined with the container helpers below: a Canvas paints nothing itself, so its background —
// here, the selection fill — lives on a `Rectangle` shape kept as child[0], behind day's children.
static WUXSh::Rectangle ensure_bg_rect(WUXC::Canvas const& canvas);

// The emulated list's cell i shows row i for the cell's whole life (cells are created per row and
// never re-indexed), so the row is fixed at install time — the same invariant day_xaml_cell_drag
// relies on.
void day_xaml_list_cell_click(void* cell, unsigned long long id, int row, DayRowClickCb cb) {
    auto canvas = elem(cell).try_as<WUXC::Canvas>();
    if (!canvas) return;
    // An unpainted Canvas is not hit-testable (see day_xaml_enable_gesture), so a press would only
    // land where the row's own content is and miss its padding — a click just inside the row band
    // would do nothing. Giving the cell its background Rectangle up front — transparent, but
    // PAINTED — makes the whole band pressable, and it is the same rect the selection recolors.
    ensure_bg_rect(canvas).Fill(WUXM::SolidColorBrush(color_argb(0x00'000000u)));
    canvas.PointerPressed(
        [id, row, cb](WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& e) {
            auto mods = e.KeyModifiers();
            int m = 0;
            if ((mods & WS::VirtualKeyModifiers::Control) != WS::VirtualKeyModifiers::None) m |= 1;
            if ((mods & WS::VirtualKeyModifiers::Shift) != WS::VirtualKeyModifiers::None) m |= 2;
            cb(id, row, m);
        });
}

// Paint (or clear) one cell's selected treatment. The fill is the theme's list-selection accent
// where the app resources can be trusted; a DAY_THEME force makes them resolve for the SYSTEM
// scheme, so the forced case falls back to a translucent accent — alpha-over-ground, so it reads
// correctly in either scheme (the same reasoning as day_xaml_container_set_card).
void day_xaml_cell_set_selected(void* cell, int on) {
    auto canvas = elem(cell).try_as<WUXC::Canvas>();
    if (!canvas) return;
    auto rect = ensure_bg_rect(canvas);
    if (!on) {
        // Transparent, not null: the cell must stay hit-testable so the NEXT press still lands.
        rect.Fill(WUXM::SolidColorBrush(color_argb(0x00'000000u)));
        return;
    }
    if (g_forced_theme == 0) {
        auto res = WUX::Application::Current().Resources();
        auto key = winrt::box_value(winrt::hstring(L"SystemControlHighlightListAccentLowBrush"));
        if (res.HasKey(key)) {
            if (auto brush = res.Lookup(key).try_as<WUXM::Brush>()) {
                rect.Fill(brush);
                return;
            }
        }
    }
    rect.Fill(WUXM::SolidColorBrush(color_argb(0x66'0078D4u)));
}

// Navigation sidebar item list (docs/navigation.md): a single-select ListView of route titles.
// The NAV host + pages are plain Canvases; day-core's NavLayout positions the sidebar/detail
// split, so no native split control is needed. Items are '\n'-joined (titles have no newlines).
void* day_xaml_navlist_new(unsigned long long id, void (*cb)(unsigned long long, int)) {
    WUXC::ListView lv;
    lv.SelectionMode(WUXC::ListViewSelectionMode::Single);
    lv.SelectionChanged([id, cb](WF::IInspectable const& s, WUXC::SelectionChangedEventArgs const&) {
        cb(id, s.as<WUXC::ListView>().SelectedIndex());
    });
    return boxh(lv);
}
void day_xaml_navlist_set_items(void* w, const char* items_joined) {
    auto lv = elem(w).try_as<WUXC::ListView>();
    if (!lv) return;
    lv.Items().Clear();
    std::string all = items_joined ? items_joined : "";
    size_t start = 0;
    while (start <= all.size()) {
        size_t nl = all.find('\n', start);
        std::string item =
            all.substr(start, nl == std::string::npos ? std::string::npos : nl - start);
        if (!(item.empty() && all.empty())) lv.Items().Append(winrt::box_value(hs(item.c_str())));
        if (nl == std::string::npos) break;
        start = nl + 1;
    }
}
void day_xaml_navlist_set_selected(void* w, int idx) {
    auto lv = elem(w).try_as<WUXC::ListView>();
    if (lv && lv.SelectedIndex() != idx) lv.SelectedIndex(idx);
}

// --- native NavigationView (docs/navigation.md): the idiomatic Windows split navigation, as in
// the Settings app — a pane of selectable MenuItems, a prominent Header slot that names the
// current destination, an optional back button, and a Content region for the detail page. day
// owns the item list + selection (fed from NAV_MENU) and the header text (from NavPatch); the
// detail page is positioned by day into the returned content Canvas, whose SizeChanged reports
// the true content bounds back so day lays the page out to the NavigationView's actual region.
// system XAML has had NavigationView since Win10 1809, so no XAML-3 dependency is needed.
//
// The callbacks: sel_cb(id, index) on a user menu pick; size_cb(id, region, w, h) whenever a
// region reflows (region 0 = content / detail, 1 = pane header); back_cb(id) on the back button.
static constexpr double DAY_NAV_SIDEBAR_WIDTH = 240.0; // mirrors day_spec::NAV_SIDEBAR_WIDTH

// Every split NavigationView alive in this process, for the toolbar's sidebar toggle
// (docs/toolbars.md). Only a split host registers; a stack presentation has no sidebar to show
// or hide.
//
// A LIST, not the single global this started as: secondary windows (docs/windows.md) each build
// their own split nav, and the last one created would otherwise own the toggle for every window
// — so the primary window's toolbar button collapsed the SECOND window's sidebar and left its
// own alone. Each window is its own XAML island with its own XamlRoot, which is what tells them
// apart at click time.
static std::vector<WUXC::NavigationView> g_navviews;

// Show/hide one nav's pane. In PaneDisplayMode::Left the pane is ALWAYS expanded and IsPaneOpen
// is ignored, so hiding it means dropping to LeftMinimal — the mode whose pane is a hidden
// overlay behind a hamburger, which is what a Windows app's collapsed nav looks like.
static bool toggle_nav_pane(WUXC::NavigationView const& nv) {
    if (!nv) return false;
    if (nv.PaneDisplayMode() == WUXC::NavigationViewPaneDisplayMode::Left) {
        nv.PaneDisplayMode(WUXC::NavigationViewPaneDisplayMode::LeftMinimal);
        nv.IsPaneOpen(false);
    } else {
        nv.PaneDisplayMode(WUXC::NavigationViewPaneDisplayMode::Left);
        nv.OpenPaneLength(DAY_NAV_SIDEBAR_WIDTH);
    }
    return true;
}

// Drop navs whose window has gone. A closed island leaves its elements without a XamlRoot, which
// is the only liveness signal a NavigationView offers — without this the list grows across every
// open/close cycle and `day_xaml_toggle_sidebar` below could pick a dead one as "the primary".
static void prune_navviews() {
    g_navviews.erase(std::remove_if(g_navviews.begin(), g_navviews.end(),
                                    [](WUXC::NavigationView const& nv) {
                                        if (!nv) return true;
                                        try {
                                            return nv.XamlRoot() == nullptr;
                                        } catch (...) {
                                            return true;
                                        }
                                    }),
                     g_navviews.end());
}

// (A per-window `toggle_sidebar_near(origin)` lived here, resolving the nav from the clicked
// toolbar button's XamlRoot. The bar no longer draws a sidebar command — NavigationView's own
// PaneToggleButton is that affordance on Windows, and each window's built-in button already acts
// on its own pane — so nothing needs to resolve a window from a click any more.)

// The window-less entry point (day-xaml's `toggle_sidebar` duty, and dayscript's step): no click
// to locate a window from, so it drives the primary window's nav — the first one still alive,
// which is the one the app opened with.
extern "C" int day_xaml_toggle_sidebar() try {
    prune_navviews();
    if (g_navviews.empty()) return 0;
    return toggle_nav_pane(g_navviews.front()) ? 1 : 0;
} catch (...) {
    return 0;
}

void* day_xaml_nav_new(unsigned long long id,
                        void (*sel_cb)(unsigned long long, int),
                        void (*size_cb)(unsigned long long, int, int, int),
                        void (*back_cb)(unsigned long long),
                        void** out_content,
                        int stack) {
    WUXC::NavigationView nv;
    nv.IsSettingsVisible(false);
    nv.IsBackButtonVisible(WUXC::NavigationViewBackButtonVisible::Collapsed); // toggled per depth
    if (stack) {
        // A push/pop stack has no menu: collapse the pane to a thin strip that just carries the
        // back button + the current page title in the header. IsPaneOpen defaults to TRUE, and in
        // LeftMinimal mode an open pane is a ~320px OVERLAY that would sit on top of the content
        // (an empty gray sidebar hiding the page) — force it closed.
        nv.PaneDisplayMode(WUXC::NavigationViewPaneDisplayMode::LeftMinimal);
        nv.IsPaneToggleButtonVisible(false);
        nv.IsPaneOpen(false);
    } else {
        nv.PaneDisplayMode(WUXC::NavigationViewPaneDisplayMode::Left); // always-expanded sidebar
        nv.OpenPaneLength(DAY_NAV_SIDEBAR_WIDTH);
        // Only a split host is toggleable; a toolbar's sidebar button drives whichever of these
        // shares its window.
        g_navviews.push_back(nv);
    }

    // The detail host: a Canvas day positions the current page into (absolute frames). A Canvas has
    // no desired size, so stretch it to fill the NavigationView's content region.
    WUXC::Canvas content;
    content.HorizontalAlignment(WUX::HorizontalAlignment::Stretch);
    content.VerticalAlignment(WUX::VerticalAlignment::Stretch);
    nv.Content(content);
    content.SizeChanged(
        [id, size_cb](WF::IInspectable const& s, WUX::SizeChangedEventArgs const&) {
            auto fe = s.as<FrameworkElement>();
            size_cb(id, 0, static_cast<int>(fe.ActualWidth()), static_cast<int>(fe.ActualHeight()));
        });

    // User picked a menu item → report its index; day maps it back to the route via NAV_MENU.
    nv.SelectionChanged(
        [id, sel_cb](WUXC::NavigationView const& sender,
                     WUXC::NavigationViewSelectionChangedEventArgs const& args) {
            if (args.IsSettingsSelected()) return;
            auto item = args.SelectedItem();
            if (!item) return;
            uint32_t idx = 0;
            if (sender.MenuItems().IndexOf(item, idx)) sel_cb(id, static_cast<int>(idx));
        });
    nv.BackRequested([id, back_cb](WUXC::NavigationView const&,
                                   WUXC::NavigationViewBackRequestedEventArgs const&) {
        back_cb(id);
    });

    if (out_content) *out_content = boxh(content);
    return boxh(nv);
}

static std::vector<std::string> split_lines(const char* joined) {
    std::vector<std::string> out;
    std::string all = joined ? joined : "";
    if (all.empty()) return out;
    size_t start = 0;
    while (true) {
        size_t nl = all.find('\n', start);
        out.push_back(all.substr(start, nl == std::string::npos ? std::string::npos : nl - start));
        if (nl == std::string::npos) break;
        start = nl + 1;
    }
    return out;
}

// Rebuild the pane's MenuItems from '\n'-joined destination titles (route order). `icons_joined`
// is a PARALLEL '\n'-joined list of bundled icon FILE NAMES (empty entry = no icon), staged next to
// the exe under images/ so a monochrome BitmapIcon can load them via ms-appx and tint to the theme.
// Defined with the other glyph/tree entry points further down; declared here so the nav rows can
// build their icons out of the same geometry the in-page `vector(…)` pieces use.
void* day_xaml_vector_icon_new(const char* spec, unsigned int argb, int tinted, double box);
void day_xaml_delete(void* h);

// The nav pane's icon slot. Three parallel per-row lists, each one line per row:
//   `icons_joined`  the staged RASTER file name — the fallback when a row has no geometry
//   `geoms_joined`  the row's `.xamlgeom` spec with newlines escaped as \x1f (see day-xaml),
//                   empty when the glyph did not convert
//   `tints_joined`  an ARGB value, 0 meaning "no tint" — that row keeps the theme foreground
// A row draws as PathIcon geometry wherever it can, so the sidebar is vector at any DPI and its
// tint is a brush; only unconvertible art falls back to the monochrome raster.
void day_xaml_nav_set_items(void* navh, const char* items_joined, const char* icons_joined,
                            const char* geoms_joined, const char* tints_joined,
                            const char* badge_icons_joined, const char* badge_geoms_joined,
                            const char* badge_tints_joined) {
    guard([&] {
        auto nv = elem(navh).try_as<WUXC::NavigationView>();
        if (!nv) return;
        nv.MenuItems().Clear();
        auto titles = split_lines(items_joined);
        auto icons = split_lines(icons_joined);
        auto geoms = split_lines(geoms_joined ? geoms_joined : "");
        auto tints = split_lines(tints_joined ? tints_joined : "");
        auto badge_icons = split_lines(badge_icons_joined ? badge_icons_joined : "");
        auto badge_geoms = split_lines(badge_geoms_joined ? badge_geoms_joined : "");
        auto badge_tints = split_lines(badge_tints_joined ? badge_tints_joined : "");
        for (size_t i = 0; i < titles.size(); ++i) {
            WUXC::NavigationViewItem nvi;
            // The trailing status glyph (docs/navigation.md). NavigationViewItem has ONE Icon
            // slot and it is the leading one, so a badge has to ride the Content: a row with one
            // becomes [title][stretch][glyph] instead of a bare string. A row without one keeps
            // the plain boxed string, so nothing changes for the common case.
            void* badge_el = nullptr;
            unsigned int badge_argb =
                i < badge_tints.size()
                    ? static_cast<unsigned int>(std::strtoul(badge_tints[i].c_str(), nullptr, 10))
                    : 0u;
            bool badge_tinted = (badge_argb >> 24) != 0;
            if (i < badge_geoms.size() && !badge_geoms[i].empty()) {
                std::string spec = badge_geoms[i];
                for (auto& c : spec)
                    if (c == '\x1f') c = '\n';
                badge_el = day_xaml_vector_icon_new(spec.c_str(), badge_argb,
                                                    badge_tinted ? 1 : 0, 14.0);
            }
            bool has_badge_bitmap =
                !badge_el && i < badge_icons.size() && !badge_icons[i].empty();
            if (badge_el || has_badge_bitmap) {
                WUXC::Grid row;
                row.ColumnDefinitions().Append(WUXC::ColumnDefinition());
                WUXC::ColumnDefinition auto_col;
                // GridLengthHelper lives in Windows.UI.Xaml, not .Controls — GridLength is a
                // framework struct, and only its helper's statics can build one from C++.
                auto_col.Width(WUX::GridLengthHelper::Auto());
                row.ColumnDefinitions().Append(auto_col);
                WUXC::TextBlock label;
                label.Text(hs(titles[i].c_str()));
                label.VerticalAlignment(WUX::VerticalAlignment::Center);
                row.Children().Append(label);
                WUXC::Grid::SetColumn(label, 0);
                WUX::FrameworkElement glyph{ nullptr };
                if (badge_el) {
                    glyph = elem(badge_el).try_as<WUX::FrameworkElement>();
                } else {
                    WUXC::BitmapIcon bicon;
                    bicon.UriSource(
                        WF::Uri{ hs(("ms-appx:///images/" + badge_icons[i]).c_str()) });
                    bicon.ShowAsMonochrome(true);
                    if (badge_tinted)
                        bicon.Foreground(WUXM::SolidColorBrush(color_argb(badge_argb)));
                    bicon.Width(14.0);
                    bicon.Height(14.0);
                    glyph = bicon;
                }
                if (glyph) {
                    glyph.VerticalAlignment(WUX::VerticalAlignment::Center);
                    glyph.Margin(WUX::ThicknessHelper::FromLengths(8.0, 0.0, 0.0, 0.0));
                    row.Children().Append(glyph);
                    WUXC::Grid::SetColumn(glyph, 1);
                }
                if (badge_el) day_xaml_delete(badge_el); // the Grid owns it now
                nvi.Content(row);
            } else {
                nvi.Content(winrt::box_value(hs(titles[i].c_str())));
            }
            unsigned int argb =
                i < tints.size()
                    ? static_cast<unsigned int>(std::strtoul(tints[i].c_str(), nullptr, 10))
                    : 0u;
            // A fully transparent value is the "no tint" encoding, not a real colour.
            bool tinted = (argb >> 24) != 0;
            void* icon = nullptr;
            if (i < geoms.size() && !geoms[i].empty()) {
                std::string spec = geoms[i];
                for (auto& c : spec)
                    if (c == '\x1f') c = '\n';
                // 16 px is the NavigationViewItem icon box the default template lays out.
                icon = day_xaml_vector_icon_new(spec.c_str(), argb, tinted ? 1 : 0, 16.0);
            }
            if (icon) {
                if (auto ie = elem(icon).try_as<WUXC::IconElement>()) nvi.Icon(ie);
                day_xaml_delete(icon); // the NavigationViewItem owns it now
            } else if (i < icons.size() && !icons[i].empty()) {
                WUXC::BitmapIcon bicon;
                bicon.UriSource(WF::Uri{ hs(("ms-appx:///images/" + icons[i]).c_str()) });
                // ShowAsMonochrome makes the bitmap an alpha mask filled with Foreground, which
                // is what turns a staged raster glyph into a tintable icon (docs/vectors.md).
                bicon.ShowAsMonochrome(true); // tint to the pane foreground (theme-adaptive)
                if (tinted) bicon.Foreground(WUXM::SolidColorBrush(color_argb(argb)));
                nvi.Icon(bicon);
            }
            nv.MenuItems().Append(nvi);
        }
    });
}

// Programmatic highlight sync (idx < 0 = clear). SelectionChanged still fires, but day's route
// set is idempotent (show() no-ops on the same key), so no feedback loop.
void day_xaml_nav_set_selected(void* navh, int idx) {
    guard([&] {
        auto nv = elem(navh).try_as<WUXC::NavigationView>();
        if (!nv) return;
        auto items = nv.MenuItems();
        if (idx < 0 || static_cast<uint32_t>(idx) >= items.Size()) {
            nv.SelectedItem(nullptr);
            return;
        }
        auto want = items.GetAt(static_cast<uint32_t>(idx));
        if (nv.SelectedItem() != want) nv.SelectedItem(want);
    });
}

// The prominent page-title header (the current destination), shown at the top of the content area.
void day_xaml_nav_set_header(void* navh, const char* title) {
    guard([&] {
        auto nv = elem(navh).try_as<WUXC::NavigationView>();
        if (nv) nv.Header(winrt::box_value(hs(title)));
    });
}

// Custom pane header (day's sidebar header piece — logo + app title). A bare Canvas has no
// desired size, so the PaneHeader slot would collapse; day fixes its height via set_frame.
void day_xaml_nav_set_pane_header(void* navh, void* element) {
    guard([&] {
        auto nv = elem(navh).try_as<WUXC::NavigationView>();
        if (!nv) return;
        if (element) nv.PaneHeader(elem(element));
        else nv.PaneHeader(nullptr);
    });
}

// Show/hide the NavigationView back button (a stack shows it once a page is pushed, docs/navigation.md).
void day_xaml_nav_set_back_visible(void* navh, int visible) {
    guard([&] {
        auto nv = elem(navh).try_as<WUXC::NavigationView>();
        if (nv) {
            nv.IsBackButtonVisible(visible ? WUXC::NavigationViewBackButtonVisible::Visible
                                           : WUXC::NavigationViewBackButtonVisible::Collapsed);
        }
    });
}

// A container is a Canvas (day positions children by absolute frame). It has no rounded-corner clip
// of its own — Windows.UI.Xaml's RectangleGeometry can't round, and UIElement.Clip is rectangular
// only — so a background fill / rounded corner is drawn by a `Rectangle` SHAPE (which DOES carry
// RadiusX/RadiusY) kept as child[0], behind day's children, tracking the Canvas size via SizeChanged.
static WUXSh::Rectangle ensure_bg_rect(WUXC::Canvas const& canvas) {
    auto kids = canvas.Children();
    if (kids.Size() > 0)
        if (auto r = kids.GetAt(0).try_as<WUXSh::Rectangle>())
            if (r.Name() == L"day_bg") return r;
    WUXSh::Rectangle rect;
    rect.Name(L"day_bg");
    WUXC::Canvas::SetLeft(rect, 0);
    WUXC::Canvas::SetTop(rect, 0);
    rect.Width(canvas.ActualWidth());
    rect.Height(canvas.ActualHeight());
    // day sizes the container via set_geometry (Width/Height), which fires SizeChanged; grow with it.
    canvas.SizeChanged([rect](WF::IInspectable const&, WUX::SizeChangedEventArgs const& e) mutable {
        rect.Width(e.NewSize().Width);
        rect.Height(e.NewSize().Height);
    });
    kids.InsertAt(0, rect);
    return rect;
}

void day_xaml_container_set_bg(void* h, unsigned int argb) {
    if (auto c = elem(h).try_as<WUXC::Canvas>())
        ensure_bg_rect(c).Fill(WUXM::SolidColorBrush(color_argb(argb)));
}

// --- backend-executed animation (DESIGN.md §8.4) ---
// Day passes INTENT — a target value plus an AnimSpec — and the platform animates it; Day never
// ticks frames for native widgets. Here that is a Storyboard per channel, which XAML runs on its
// own compositor. `curve`: 0 linear, 1 ease-in, 2 ease-out, 3 ease-in-out, 4 spring (the shared
// encoding day-qt uses). `dur_ms <= 0` means "no animation" — set the value outright.

static WUXMA::EasingFunctionBase easing_for(int curve) {
    switch (curve) {
        case 1: {
            WUXMA::QuadraticEase e;
            e.EasingMode(WUXMA::EasingMode::EaseIn);
            return e;
        }
        case 2: {
            WUXMA::QuadraticEase e;
            e.EasingMode(WUXMA::EasingMode::EaseOut);
            return e;
        }
        case 3: {
            WUXMA::QuadraticEase e;
            e.EasingMode(WUXMA::EasingMode::EaseInOut);
            return e;
        }
        case 4: {
            // §8.4: a spring maps to a fixed-duration OVERSHOOT curve over exactly duration_ms, so
            // the timing matches the other toolkits rather than a physics settle time.
            WUXMA::BackEase e;
            e.EasingMode(WUXMA::EasingMode::EaseOut);
            e.Amplitude(0.35);
            return e;
        }
        default:
            return nullptr; // linear
    }
}

// One Storyboard per (element, property path). Re-animating a channel has to REPLACE the running
// storyboard: two live storyboards on one property fight, and a stopped one snaps its property
// back to the pre-animation value. Keyed by element+path so the channels stay independent.
static std::map<std::pair<void*, std::wstring>, WUXMA::Storyboard> g_anims;

static void stop_anim(void* key, std::wstring const& path) {
    auto it = g_anims.find({key, path});
    if (it != g_anims.end()) {
        it->second.Stop();
        g_anims.erase(it);
    }
}

// Animate one double property to `to`. `target` is the object the path is rooted at.
static void animate_double(void* key, WUX::DependencyObject const& target, std::wstring const& path,
                           double to, int dur_ms, int curve, bool dependent) {
    stop_anim(key, path);
    WUXMA::DoubleAnimation a;
    a.To(to);
    a.Duration(WUX::Duration(WF::TimeSpan{static_cast<int64_t>(dur_ms) * 10000}));
    a.EnableDependentAnimation(dependent);
    if (auto e = easing_for(curve)) a.EasingFunction(e);
    // FillBehavior::HoldEnd keeps the animated value after the run; without it the property snaps
    // back to its pre-animation value the moment the storyboard completes.
    a.FillBehavior(WUXMA::FillBehavior::HoldEnd);
    WUXMA::Storyboard sb;
    sb.Children().Append(a);
    WUXMA::Storyboard::SetTarget(a, target);
    WUXMA::Storyboard::SetTargetProperty(a, path);
    g_anims[{key, path}] = sb;
    sb.Begin();
}

// The CompositeTransform every transform channel animates through. Installed once per element;
// RenderTransformOrigin is the CENTRE, matching AppKit's layer anchor and Qt's painter transform
// (Day's anchor_x/anchor_y are 0.5/0.5 in practice and the other backends centre unconditionally).
static WUXM::CompositeTransform ensure_transform(WUX::UIElement const& el) {
    if (auto existing = el.RenderTransform().try_as<WUXM::CompositeTransform>()) return existing;
    WUXM::CompositeTransform t;
    el.RenderTransform(t);
    el.RenderTransformOrigin(WF::Point{0.5f, 0.5f});
    return t;
}

void day_xaml_set_opacity(void* h, double opacity, int dur_ms, int curve) {
    auto el = elem(h).try_as<WUX::UIElement>();
    if (!el) return;
    if (dur_ms <= 0) {
        stop_anim(h, L"Opacity");
        el.Opacity(opacity);
        return;
    }
    // Opacity is GPU-composited, so this runs independent of the UI thread.
    animate_double(h, el, L"Opacity", opacity, dur_ms, curve, false);
}

void day_xaml_set_transform(void* h, double tx, double ty, double sx, double sy, double rotate_deg,
                            int dur_ms, int curve) {
    auto el = elem(h).try_as<WUX::UIElement>();
    if (!el) return;
    auto t = ensure_transform(el);
    struct Channel { const wchar_t* path; double to; };
    const Channel channels[] = {
        {L"TranslateX", tx}, {L"TranslateY", ty},
        {L"ScaleX", sx},     {L"ScaleY", sy},
        {L"Rotation", rotate_deg},
    };
    for (auto const& ch : channels) {
        if (dur_ms <= 0) {
            stop_anim(h, ch.path);
            if (std::wstring(ch.path) == L"TranslateX") t.TranslateX(ch.to);
            else if (std::wstring(ch.path) == L"TranslateY") t.TranslateY(ch.to);
            else if (std::wstring(ch.path) == L"ScaleX") t.ScaleX(ch.to);
            else if (std::wstring(ch.path) == L"ScaleY") t.ScaleY(ch.to);
            else t.Rotation(ch.to);
        } else {
            // CompositeTransform channels are GPU-composited too — independent animations.
            animate_double(h, t, ch.path, ch.to, dur_ms, curve, false);
        }
    }
}

// Animated background fill. Unlike the transform channels a brush colour is NOT composited
// independently, so this one needs EnableDependentAnimation — it is a single box, not a per-frame
// layout cost. XAML is the only desktop backend that can tween this (DESIGN.md §8.4).
void day_xaml_container_animate_bg(void* h, unsigned int argb, int dur_ms, int curve) {
    auto canvas = elem(h).try_as<WUXC::Canvas>();
    if (!canvas) return;
    auto rect = ensure_bg_rect(canvas);
    auto brush = rect.Fill().try_as<WUXM::SolidColorBrush>();
    if (dur_ms <= 0 || !brush) {
        rect.Fill(WUXM::SolidColorBrush(color_argb(argb)));
        return;
    }
    stop_anim(h, L"BgColor");
    WUXMA::ColorAnimation a;
    a.To(color_argb(argb));
    a.Duration(WUX::Duration(WF::TimeSpan{static_cast<int64_t>(dur_ms) * 10000}));
    a.EnableDependentAnimation(true);
    if (auto e = easing_for(curve)) a.EasingFunction(e);
    a.FillBehavior(WUXMA::FillBehavior::HoldEnd);
    WUXMA::Storyboard sb;
    sb.Children().Append(a);
    WUXMA::Storyboard::SetTarget(a, brush);
    WUXMA::Storyboard::SetTargetProperty(a, L"Color");
    g_anims[{h, L"BgColor"}] = sb;
    sb.Begin();
}

// Emulated fullscreen cover (docs/cover.md): give the cover an OPAQUE page-background surface
// (the same grounding as the window root) so it occludes the content beneath it.
void day_xaml_cover_ground(void* h) {
    auto c = elem(h).try_as<WUXC::Canvas>();
    if (!c) return;
    auto rect = ensure_bg_rect(c);
    auto res = WUX::Application::Current().Resources();
    auto key = winrt::box_value(winrt::hstring(L"ApplicationPageBackgroundThemeBrush"));
    if (res.HasKey(key)) {
        if (auto brush = res.Lookup(key).try_as<WUXM::Brush>()) {
            rect.Fill(brush);
            return;
        }
    }
    bool dark = g_forced_theme == 2 ||
        (g_forced_theme == 0 &&
         WUX::Application::Current().RequestedTheme() == WUX::ApplicationTheme::Dark);
    rect.Fill(WUXM::SolidColorBrush(color_argb(dark ? 0xFF'202020u : 0xFF'F3F3F3u)));
}

// SurfaceRole::SectionCard: the grouped-card fill from the theme resources — resolved per the
// APP theme (a DAY_THEME force is applied app-wide, so this tracks it). Fallback where the
// resource set predates the card brush: a translucent neutral, which reads correctly over
// either scheme's page ground precisely because it is alpha-over-ground rather than a fixed
// opaque color.
void day_xaml_container_set_card(void* h, double radius) {
    if (auto c = elem(h).try_as<WUXC::Canvas>()) {
        auto r = ensure_bg_rect(c);
        bool filled = false;
        // The app-resource card brush resolves per the SYSTEM theme; only trust it when no
        // DAY_THEME force is active (else it mis-colors the forced scheme). The translucent-neutral
        // fallback is alpha-over-ground, so it reads correctly over either scheme's page ground.
        if (g_forced_theme == 0) {
            auto res = WUX::Application::Current().Resources();
            auto key = winrt::box_value(winrt::hstring(L"CardBackgroundFillColorDefaultBrush"));
            if (res.HasKey(key)) {
                if (auto brush = res.Lookup(key).try_as<WUXM::Brush>()) {
                    r.Fill(brush);
                    filled = true;
                }
            }
        }
        if (!filled) r.Fill(WUXM::SolidColorBrush(color_argb(0x14'808080u)));
        r.RadiusX(radius);
        r.RadiusY(radius);
    }
}

// Rounded corners for a container's background (login card, chat bubbles, avatar/badge discs).
// RadiusX/RadiusY live on the Rectangle SHAPE, not on RectangleGeometry.
void day_xaml_container_set_corner(void* h, double radius) {
    auto c = elem(h).try_as<WUXC::Canvas>();
    if (!c) return;
    // The container's OWN fill, for the case where this container also carries the background.
    auto r = ensure_bg_rect(c);
    r.RadiusX(radius);
    r.RadiusY(radius);

    // …and a real rounded CLIP over the children, which is what `ContainerProps.clips` asks for
    // and what the common case actually needs. `corner_radius(r)` is its own container in the
    // piece tree, wrapping the `background(c)` container rather than sharing one with it:
    //
    //   corner_radius container   (transparent, radius r, clips)
    //     └── background container  (opaque fill, square)
    //
    // so rounding only this container's own — invisible — rect left the child's square fill
    // painting over the top, and every `.background(…).corner_radius(…)` surface came out sharp.
    //
    // It has to be a COMPOSITION clip: XAML's `UIElement.Clip` is typed as RectangleGeometry,
    // which has no corner radius, so the rounded shape is unreachable from the XAML side.
    try {
        auto visual = WUXH::ElementCompositionPreview::GetElementVisual(c);
        if (!visual) return;
        auto compositor = visual.Compositor();
        auto geo = compositor.CreateRoundedRectangleGeometry();
        auto rf = static_cast<float>(radius);
        geo.CornerRadius({ rf, rf });
        geo.Size({ static_cast<float>(c.ActualWidth()), static_cast<float>(c.ActualHeight()) });
        visual.Clip(compositor.CreateGeometricClip(geo));
        // day sizes the container after realize, so the clip has to track it the way the
        // background rect does — a clip left at 0×0 would hide the whole subtree.
        c.SizeChanged([geo](WF::IInspectable const&, WUX::SizeChangedEventArgs const& e) mutable {
            geo.Size({ e.NewSize().Width, e.NewSize().Height });
        });
    } catch (...) {
        // CreateGeometricClip needs 1809+; on older builds the corners stay square, which is the
        // behaviour this backend already had.
    }
}

// ---- label ----

void* day_xaml_label_new(const char* text) {
    WUXC::TextBlock t;
    t.Text(hs(text));
    t.TextWrapping(WUX::TextWrapping::Wrap);
    return boxh(t);
}
void day_xaml_label_set_text(void* h, const char* t) {
    if (auto tb = elem(h).try_as<WUXC::TextBlock>()) tb.Text(hs(t));
}
// Make a label's text user-selectable (the `.selectable()` modifier, docs/text.md). try_as
// guards a non-TextBlock handle — a no-op rather than a bad cast.
void day_xaml_label_set_selectable(void* h, int on) {
    if (auto tb = elem(h).try_as<WUXC::TextBlock>()) tb.IsTextSelectionEnabled(on != 0);
}
void day_xaml_label_set_color(void* h, unsigned argb) {
    auto tb = elem(h).try_as<WUXC::TextBlock>();
    if (!tb) return;
    // Alpha 0 (Color::CLEAR / a `None` color token) = "no override" — restore the inherited default
    // foreground; otherwise paint the requested color.
    if ((argb >> 24) == 0)
        tb.ClearValue(WUXC::TextBlock::ForegroundProperty());
    else
        tb.Foreground(brush_bits(argb));
}
void day_xaml_label_set_font(void* h, double pt, int weight, int italic, int tabular) {
    if (auto tb = elem(h).try_as<WUXC::TextBlock>()) {
        // FontSize scales with the OS text-scale-factor (accessibility "Text size"); XAML applies it.
        tb.FontSize(pt);
        // `weight` is a numeric font weight (100–900); build a FontWeight directly.
        winrt::Windows::UI::Text::FontWeight w;
        w.Weight = static_cast<uint16_t>(weight > 0 ? weight : 400);
        tb.FontWeight(w);
        tb.FontStyle(italic ? WUI::Text::FontStyle::Italic : WUI::Text::FontStyle::Normal);
        // Tabular figures: XAML exposes them as the Typography attached property
        // NumeralAlignment, not as a font swap, so the face is untouched and only the digits
        // change metrics. Tabular == every digit on the same advance.
        //
        // FontNumeralAlignment sits in Windows.UI.Xaml, NOT in Windows.UI.Text beside the other
        // font primitives (FontStyle just above) — the typography enums are the XAML-side half of
        // the pair, matching the Typography attached property that consumes them. Naming the Text
        // namespace here compiles to a cascade rather than a clear error: the enum expression
        // fails first, so the two-argument setter is then reported as "does not take 1 arguments".
        WUXD::Typography::SetNumeralAlignment(
            tb, tabular ? WUX::FontNumeralAlignment::Tabular
                        : WUX::FontNumeralAlignment::Normal);
    }
}
// Bundled custom font (§18.4): `spec` is a FontFamily source string of the form
// "ms-appx:///fonts/<file>#<family>". Unpackaged system XAML rejects `file://`/absolute font
// locations (like BitmapImage), so the Rust side stages the font under `<exe>/fonts/` and hands us
// the `ms-appx:///` URI XAML resolves against the executable directory. An unresolved source leaves
// the inherited (default) font.
void day_xaml_label_set_font_family(void* h, const char* spec) {
    if (auto tb = elem(h).try_as<WUXC::TextBlock>()) {
        tb.FontFamily(WUXM::FontFamily(hs(spec)));
    }
}

// ---- button ----

void* day_xaml_button_new(const char* title, unsigned long long id, void (*cb)(unsigned long long)) {
    WUXC::Button b;
    b.Content(winrt::box_value(hs(title)));
    b.Click([id, cb](WF::IInspectable const&, WUX::RoutedEventArgs const&) { cb(id); });
    return boxh(b);
}

// ButtonStyleSpec::Prominent — the accent-filled style, where the resource set provides it.
void day_xaml_button_prominent(void* h) {
    auto b = elem(h).try_as<WUXC::Button>();
    if (!b) return;
    auto res = WUX::Application::Current().Resources();
    auto key = winrt::box_value(winrt::hstring(L"AccentButtonStyle"));
    if (res.HasKey(key)) {
        if (auto style = res.Lookup(key).try_as<WUX::Style>()) b.Style(style);
    }
}
void day_xaml_button_set_title(void* h, const char* t) {
    if (auto b = elem(h).try_as<WUXC::Button>()) b.Content(winrt::box_value(hs(t)));
}

// ---- toggle (ToggleSwitch) ----

void* day_xaml_toggle_new(int on, unsigned long long id, void (*cb)(unsigned long long, int)) {
    WUXC::ToggleSwitch t;
    t.IsOn(on != 0);
    t.OnContent(winrt::box_value(winrt::hstring{}));
    t.OffContent(winrt::box_value(winrt::hstring{}));
    t.Toggled([id, cb](WF::IInspectable const& s, WUX::RoutedEventArgs const&) {
        cb(id, s.as<WUXC::ToggleSwitch>().IsOn() ? 1 : 0);
    });
    return boxh(t);
}
void day_xaml_toggle_set(void* h, int on) {
    if (auto t = elem(h).try_as<WUXC::ToggleSwitch>())
        if (t.IsOn() != (on != 0)) t.IsOn(on != 0);
}

// ---- slider (native double range: day passes the app's f64 min/max/step/value straight through) ----

// `cb(id, value, committed)`: `committed != 0` marks the value the user settled on, as against
// the stream a drag produces (day-spec `Event::ValueCommitted`).
void* day_xaml_slider_new(double value, double min, double max, double step, unsigned long long id,
                           void (*cb)(unsigned long long, double, int)) {
    WUXC::Slider s;
    // XAML's Slider.Value/Minimum/Maximum/StepFrequency are all `double`, so day drives it in the
    // app's real units directly (like the GTK backend's Scale) — no 0..1000 integer-tick mapping.
    // That indirection only exists in the Qt backend because its QSlider is integer-only. Set the
    // bounds before the value so XAML doesn't clamp the value against the default 0..100 range.
    s.Minimum(min);
    s.Maximum(max);
    // StepFrequency governs keyboard/drag snapping; day passes the app's step (or a fine
    // 1/1000-of-range default, matching GTK) so the slider stays effectively continuous.
    if (step > 0.0) s.StepFrequency(step);
    // Day has the app render the value itself (the GTK backend likewise hides its native readout via
    // set_draw_value(false)), so suppress XAML's thumb tooltip rather than duplicate it.
    s.IsThumbToolTipEnabled(false);
    s.Value(value);
    s.ValueChanged([id, cb](WF::IInspectable const& sender,
                            WUXCP::RangeBaseValueChangedEventArgs const&) {
        cb(id, sender.as<WUXC::Slider>().Value(), 0);
    });
    // XAML gives a Slider no "drag ended" event. What it does give is the thumb's pointer
    // capture: the control takes it on press and loses it on release (or on the gesture being
    // cancelled), so PointerCaptureLost is the end of a drag. Keyboard steps never take capture,
    // so KeyUp covers those — arrow/Page keys move by StepFrequency and are settled on release.
    s.PointerCaptureLost([id, cb](WF::IInspectable const& sender, WUX::Input::PointerRoutedEventArgs const&) {
        cb(id, sender.as<WUXC::Slider>().Value(), 1);
    });
    s.KeyUp([id, cb](WF::IInspectable const& sender, WUX::Input::KeyRoutedEventArgs const&) {
        cb(id, sender.as<WUXC::Slider>().Value(), 1);
    });
    return boxh(s);
}
void day_xaml_slider_set(void* h, double value) {
    if (auto s = elem(h).try_as<WUXC::Slider>())
        if (s.Value() != value) s.Value(value);
}

// ---- progress (determinate ProgressBar 0..1000, or indeterminate ProgressRing) ----

void* day_xaml_progress_new(int determinate, int value) {
    if (determinate) {
        WUXC::ProgressBar b;
        b.Minimum(0);
        b.Maximum(1000);
        b.IsIndeterminate(false);
        b.Value(value);
        return boxh(b);
    }
    WUXC::ProgressRing r;
    r.IsActive(true);
    return boxh(r);
}
void day_xaml_progress_set(void* h, int value) {
    if (auto b = elem(h).try_as<WUXC::ProgressBar>())
        if (static_cast<int>(b.Value()) != value) b.Value(value);
}

// ---- tabs (docs/tabs.md): a Pivot owns its page content ----

void* day_xaml_tabs_new(unsigned long long id, void (*cb)(unsigned long long, int)) {
    WUXC::Pivot p;
    p.SelectionChanged([id, cb](winrt::Windows::Foundation::IInspectable const& s,
                                WUXC::SelectionChangedEventArgs const&) {
        cb(id, s.as<WUXC::Pivot>().SelectedIndex());
    });
    return boxh(p);
}
void day_xaml_tabs_add_page(void* tabs, void* page, const char* title, int index) {
    auto p = elem(tabs).as<WUXC::Pivot>();
    WUXC::PivotItem item;
    item.Header(winrt::box_value(hs(title)));
    item.Content(elem(page));
    auto items = p.Items();
    if (index < 0 || static_cast<uint32_t>(index) >= items.Size()) items.Append(item);
    else items.InsertAt(static_cast<uint32_t>(index), item);
}
void day_xaml_tabs_set_current(void* tabs, int index) {
    elem(tabs).as<WUXC::Pivot>().SelectedIndex(index);
}
void day_xaml_tabs_content_size(void* tabs, double* w, double* h) {
    auto p = elem(tabs).as<WUXC::Pivot>();
    *w = p.ActualWidth();
    double ah = p.ActualHeight();
    *h = ah > 48 ? ah - 48 : ah; // subtract the header strip
}

// ---- focus (docs/focus.md) ----
// Observe: kind 1 = gained, 0 = lost, 2 = submitted (Enter in a TextBox). System XAML has no
// global focus event, so each control reports its own GotFocus/LostFocus.
void day_xaml_enable_focus(void* h, unsigned long long id,
                            void (*cb)(unsigned long long, int)) try {
    auto c = elem(h).try_as<WUXC::Control>();
    if (!c) return;
    c.GotFocus([id, cb](WF::IInspectable const&, WUX::RoutedEventArgs const&) { cb(id, 1); });
    c.LostFocus([id, cb](WF::IInspectable const&, WUX::RoutedEventArgs const&) { cb(id, 0); });
    if (auto tb = c.try_as<WUXC::TextBox>()) {
        tb.KeyDown([id, cb](WF::IInspectable const&, WUXIn::KeyRoutedEventArgs const& a) {
            if (a.Key() == winrt::Windows::System::VirtualKey::Enter) cb(id, 2);
        });
    }
} catch (...) {}

// Drive: request programmatic focus, or resign it to the window's focus sink — only while
// this control still owns focus, so a stale release can't blur a sibling. (Programmatic
// focus draws no focus visual; that is system-XAML behavior, not a bug.)
void day_xaml_control_focus(void* h, int focused) try {
    auto c = elem(h).try_as<WUXC::Control>();
    if (!c) return;
    if (focused) {
        c.Focus(WUX::FocusState::Programmatic);
    } else if (c.FocusState() != WUX::FocusState::Unfocused && g_app && g_app->focus_sink) {
        auto sink = g_app->focus_sink;
        sink.IsTabStop(true);
        sink.Focus(WUX::FocusState::Programmatic);
        sink.IsTabStop(false);
    }
} catch (...) {}

// ---- textbox ----

void* day_xaml_textbox_new(const char* text, const char* placeholder, unsigned long long id,
                            void (*cb)(unsigned long long, const char*)) {
    WUXC::TextBox tb;
    tb.Text(hs(text));
    tb.PlaceholderText(hs(placeholder));
    tb.TextChanged([id, cb](WF::IInspectable const& s, WUXC::TextChangedEventArgs const&) {
        std::string str = u8(s.as<WUXC::TextBox>().Text());
        cb(id, str.c_str());
    });
    return boxh(tb);
}
void day_xaml_textbox_set_text(void* h, const char* t) {
    if (auto tb = elem(h).try_as<WUXC::TextBox>()) {
        auto nt = hs(t);
        if (tb.Text() != nt) tb.Text(nt);
    }
}
void day_xaml_textbox_set_placeholder(void* h, const char* t) {
    if (auto tb = elem(h).try_as<WUXC::TextBox>()) tb.PlaceholderText(hs(t));
}

// ---- divider / image ----

void* day_xaml_divider_new() {
    WUXC::Border b;
    b.Height(1);
    // The app-resource hairline brush resolves per the SYSTEM theme; only trust it when no
    // DAY_THEME force is active. Translucent-neutral fallback — alpha over the page ground reads
    // correctly in either scheme.
    bool filled = false;
    if (g_forced_theme == 0) {
        auto res = WUX::Application::Current().Resources();
        auto key = winrt::box_value(winrt::hstring(L"DividerStrokeColorDefaultBrush"));
        if (res.HasKey(key)) {
            if (auto brush = res.Lookup(key).try_as<WUXM::Brush>()) {
                b.Background(brush);
                filled = true;
            }
        }
    }
    if (!filled) b.Background(WUXM::SolidColorBrush(color_argb(0x33'808080u)));
    return boxh(b);
}

// Read a local file's bytes into an in-memory stream. The system-XAML BitmapImage can't load a
// `file://` / bare-path Uri (a UWP restriction that carries into XAML Islands), so bundled images
// must be fed as a stream via SetSource. `path` is a UTF-8 native path; open it wide so Unicode
// paths work. StoreAsync().get() commits the in-memory buffer (completes on a pool thread — no UI
// deadlock). Returns null on any failure.
static WSS::IRandomAccessStream read_file_stream(const char* path) {
    try {
        std::ifstream f(hs(path).c_str(), std::ios::binary); // hstring::c_str() is wchar_t* (MSVC)
        if (!f) return nullptr;
        std::vector<uint8_t> bytes((std::istreambuf_iterator<char>(f)),
                                   std::istreambuf_iterator<char>());
        if (bytes.empty()) return nullptr;
        WSS::InMemoryRandomAccessStream stream;
        WSS::DataWriter writer(stream);
        writer.WriteBytes(winrt::array_view<uint8_t const>(bytes.data(), bytes.data() + bytes.size()));
        writer.StoreAsync().get();
        writer.DetachStream();
        stream.Seek(0);
        return stream;
    } catch (...) {
        return nullptr;
    }
}

// ---- vector glyphs as real XAML geometry (docs/vectors.md) -------------------
//
// A `vector(…)` glyph draws as `Path` geometry, not as an image: the CLI converts the staged SVG
// into the shape list this parses (`build/day/vectors/xaml/<name>.xamlgeom`), so the glyph is
// resolution-INDEPENDENT — XAML rasterizes the geometry at whatever size the layout gives it,
// every frame, instead of scaling a 256 px cache. It is also what makes a tint a runtime
// composition: the colour is a Brush set on the shape when it is realized, so one staged glyph
// serves every tint at every size. Art the CLI could not convert (gradients, clips, embedded
// rasters) stages no geometry, and the callers below fall back to the raster.

/// One shape parsed out of a `.xamlgeom` spec.
struct GeomShape {
    std::string data;
    unsigned int fill = 0;   // 0 = no fill
    bool even_odd = false;
    unsigned int stroke = 0; // 0 = no stroke
    double stroke_width = 0;
    std::string cap, join;
};
struct Geom {
    double w = 0, h = 0;
    std::vector<GeomShape> shapes;
};

// "#AARRGGBB" → packed ARGB; "-" (or anything unparsable) → 0, the "absent" encoding.
static unsigned int geom_color(const std::string& s) {
    if (s.size() != 9 || s[0] != '#') return 0;
    return static_cast<unsigned int>(std::strtoul(s.c_str() + 1, nullptr, 16));
}

static Geom parse_geom(const char* spec) {
    Geom g;
    if (!spec) return g;
    std::string text(spec);
    size_t pos = 0;
    while (pos <= text.size()) {
        size_t nl = text.find('\n', pos);
        std::string line = text.substr(pos, nl == std::string::npos ? std::string::npos : nl - pos);
        if (nl == std::string::npos) pos = text.size() + 1; else pos = nl + 1;
        if (line.empty()) continue;
        if (line[0] == 'V') {
            std::sscanf(line.c_str(), "V %lf %lf", &g.w, &g.h);
        } else if (line[0] == 'P') {
            // The data is everything after the tab, so it can hold spaces and commas untouched.
            size_t tab = line.find('\t');
            if (tab == std::string::npos) continue;
            GeomShape sh;
            sh.data = line.substr(tab + 1);
            char fill[32] = { 0 }, stroke[32] = { 0 }, cap[16] = { 0 }, join[16] = { 0 };
            int eo = 0;
            double w = 0;
            if (std::sscanf(line.substr(0, tab).c_str(), "P %31s %d %31s %lf %15s %15s",
                            fill, &eo, stroke, &w, cap, join) >= 4) {
                sh.fill = geom_color(fill);
                sh.even_odd = eo != 0;
                sh.stroke = geom_color(stroke);
                sh.stroke_width = w;
                sh.cap = cap;
                sh.join = join;
                g.shapes.push_back(std::move(sh));
            }
        }
    }
    return g;
}

// Build a Geometry from the emitted path data.
//
// Assembled object by object rather than handed to XamlReader as `<Path Data="…"/>` markup: the
// island's Application answers XAML type resolution with its own IXamlMetadataProvider, and
// under it XamlReader::Load fails to produce a Path at all — which is what silently dropped
// every glyph to its raster. Nothing is lost by not going through the parser, since day-vector
// emits a grammar this side already knows: absolute `M`/`L`/`C`/`Z` only, so an unrecognized
// command means the two ends disagree and the glyph is refused rather than drawn wrong.
static WUXM::Geometry geometry_from_data(const std::string& data, bool even_odd) {
    try {
        WUXM::PathGeometry geo;
        // SVG's default fill rule is nonzero; XAML's is even-odd. Always stated, never implied.
        geo.FillRule(even_odd ? WUXM::FillRule::EvenOdd : WUXM::FillRule::Nonzero);
        auto figures = geo.Figures();
        WUXM::PathFigure fig{ nullptr };
        const char* p = data.c_str();
        auto num = [&p]() -> float {
            while (*p == ' ' || *p == ',') ++p;
            char* end = nullptr;
            float v = std::strtof(p, &end);
            if (end) p = end;
            return v;
        };
        while (*p) {
            char cmd = *p;
            if (cmd == ' ' || cmd == ',') { ++p; continue; }
            ++p;
            if (cmd == 'M') {
                fig = WUXM::PathFigure();
                float x = num(), y = num();
                fig.StartPoint(WF::Point{ x, y });
                fig.IsFilled(true);
                figures.Append(fig);
            } else if (cmd == 'L') {
                if (!fig) return nullptr;
                WUXM::LineSegment s;
                float x = num(), y = num();
                s.Point(WF::Point{ x, y });
                fig.Segments().Append(s);
            } else if (cmd == 'C') {
                if (!fig) return nullptr;
                WUXM::BezierSegment s;
                float x1 = num(), y1 = num(), x2 = num(), y2 = num(), x3 = num(), y3 = num();
                s.Point1(WF::Point{ x1, y1 });
                s.Point2(WF::Point{ x2, y2 });
                s.Point3(WF::Point{ x3, y3 });
                fig.Segments().Append(s);
            } else if (cmd == 'Z' || cmd == 'z') {
                if (fig) fig.IsClosed(true);
            } else {
                return nullptr;
            }
        }
        if (figures.Size() == 0) return nullptr;
        return geo;
    } catch (...) {
        return nullptr;
    }
}

// A glyph sized by the layout: the shapes go in a Canvas the size of the source viewport, and a
// Viewbox scales that to whatever frame day assigns — so one geometry serves every size.
// `tinted` replaces every authored paint with `argb`; otherwise the art keeps its own colours.
void* day_xaml_vector_new(const char* spec, int mode, unsigned int argb, int tinted) {
    Geom g = parse_geom(spec);
    if (g.shapes.empty() || g.w <= 0 || g.h <= 0) return nullptr;
    try {
        WUXC::Canvas canvas;
        canvas.Width(g.w);
        canvas.Height(g.h);
        bool any = false;
        for (auto const& sh : g.shapes) {
            auto geo = geometry_from_data(sh.data, sh.even_odd);
            if (!geo) continue;
            WUXSh::Path p;
            p.Data(geo);
            if (tinted) {
                // The tint composes over the geometry: fill where the art filled, stroke where
                // it stroked, so a glyph drawn as an outline stays an outline.
                if (sh.fill) p.Fill(WUXM::SolidColorBrush(color_argb(argb)));
                if (sh.stroke) p.Stroke(WUXM::SolidColorBrush(color_argb(argb)));
            } else {
                if (sh.fill) p.Fill(WUXM::SolidColorBrush(color_argb(sh.fill)));
                if (sh.stroke) p.Stroke(WUXM::SolidColorBrush(color_argb(sh.stroke)));
            }
            if (sh.stroke) {
                p.StrokeThickness(sh.stroke_width);
                if (sh.cap == "Round") p.StrokeStartLineCap(WUXM::PenLineCap::Round);
                else if (sh.cap == "Square") p.StrokeStartLineCap(WUXM::PenLineCap::Square);
                p.StrokeEndLineCap(p.StrokeStartLineCap());
                if (sh.join == "Round") p.StrokeLineJoin(WUXM::PenLineJoin::Round);
                else if (sh.join == "Bevel") p.StrokeLineJoin(WUXM::PenLineJoin::Bevel);
            }
            canvas.Children().Append(p);
            any = true;
        }
        if (!any) return nullptr;
        WUXC::Viewbox vb;
        vb.Child(canvas);
        // Matches the image content modes: 0=fit, 1=fill (crop), 2=stretch.
        vb.Stretch(mode == 2 ? WUXM::Stretch::Fill
                   : mode == 1 ? WUXM::Stretch::UniformToFill
                               : WUXM::Stretch::Uniform);
        return boxh(vb);
    } catch (...) {
        return nullptr;
    }
}

// The same geometry as a `PathIcon`, for the slots that demand an IconElement (nav rows).
// PathIcon draws its geometry in its OWN coordinates with no scaling of its own, so the source
// viewport is mapped onto `box` here; an untinted icon leaves Foreground alone and inherits the
// pane's theme colour, which is what keeps unstyled rows theme-adaptive.
void* day_xaml_vector_icon_new(const char* spec, unsigned int argb, int tinted, double box) {
    Geom g = parse_geom(spec);
    if (g.shapes.empty() || g.w <= 0 || g.h <= 0) return nullptr;
    // One Geometry for the icon: an IconElement carries a single colour anyway, so the shapes
    // concatenate into one figure list rather than losing their per-shape paints twice over.
    std::string all;
    bool even_odd = g.shapes.front().even_odd;
    for (auto const& sh : g.shapes) all += sh.data;
    try {
        auto geo = geometry_from_data(all, even_odd);
        if (!geo) return nullptr;
        double s = box / (g.w > g.h ? g.w : g.h);
        WUXM::ScaleTransform st;
        st.ScaleX(s);
        st.ScaleY(s);
        geo.Transform(st);
        WUXC::PathIcon icon;
        icon.Data(geo);
        if (tinted) icon.Foreground(WUXM::SolidColorBrush(color_argb(argb)));
        return boxh(icon);
    } catch (...) {
        return nullptr;
    }
}

// The raster fallback for art the CLI could not convert to geometry (gradients, clips, embedded
// rasters): ShowAsMonochrome turns the staged PNG into an alpha mask that Foreground fills.
// Lossier than the geometry path above — it tints a 256 px cache, not the glyph — which is why
// it is reached only when there is no geometry to draw. `icon_file` is the staged file NAME
// (BitmapIcon takes only a Uri, and unpackaged islands resolve `ms-appx:///images/<file>`
// against the exe directory); an empty name or transparent tint falls through to a plain Image.
void* day_xaml_image_tinted_new(const char* icon_file, int mode, unsigned int argb) {
    if (icon_file && *icon_file && (argb >> 24) != 0) {
        WUXC::BitmapIcon bicon;
        bicon.UriSource(WF::Uri{ hs((std::string("ms-appx:///images/") + icon_file).c_str()) });
        bicon.ShowAsMonochrome(true);
        bicon.Foreground(WUXM::SolidColorBrush(color_argb(argb)));
        return boxh(bicon);
    }
    return nullptr;
}

void* day_xaml_image_new(const char* uri, int mode) {
    WUXC::Image img;
    // Scaling (§18.3): 0=fit (Uniform), 1=fill (UniformToFill, cropped), 2=stretch (Fill).
    img.Stretch(mode == 2 ? WUXM::Stretch::Fill
                : mode == 1 ? WUXM::Stretch::UniformToFill
                            : WUXM::Stretch::Uniform);
    if (uri && *uri) {
        try {
            WUXM::Imaging::BitmapImage bmp;
            std::string s = uri;
            if (s.rfind("http://", 0) == 0 || s.rfind("https://", 0) == 0) {
                bmp.UriSource(WF::Uri{ hs(uri) }); // remote — BitmapImage loads http(s) directly
            } else if (auto stream = read_file_stream(uri)) {
                bmp.SetSource(stream); // local bundled file — feed the bytes, not a file:// Uri
            }
            img.Source(bmp);
        } catch (...) {}
    }
    return boxh(img);
}

// ---- tree / geometry / props ----

void day_xaml_add_child(void* parent, void* child) {
    guard([&] {
        if (auto p = elem(parent).try_as<WUXC::Panel>()) p.Children().Append(elem(child));
    });
}
void day_xaml_remove_child(void* parent, void* child) {
    guard([&] {
        if (auto p = elem(parent).try_as<WUXC::Panel>()) {
            uint32_t idx = 0;
            if (p.Children().IndexOf(elem(child), idx)) p.Children().RemoveAt(idx);
        }
    });
}
void day_xaml_delete(void* h) { delete reinterpret_cast<Node*>(h); }

// External-piece handle seam (docs/picker.md): box any WinRT UI element into a day handle, and
// borrow it back — so an external piece can carry its OWN native XAML shim (like the Qt shims)
// without duplicating the private `Node` wrapper. The element crosses as a WinRT ABI pointer
// (`get_abi`, a stable COM interface pointer), which day-xaml-sys owns the boxing for.
void* day_xaml_box(void* iinspectable_abi) {
    WF::IInspectable insp{ nullptr };
    winrt::copy_from_abi(insp, iinspectable_abi); // AddRefs the incoming element
    return boxh(insp.as<UIElement>());
}
void* day_xaml_unbox(void* handle) {
    return winrt::get_abi(elem(handle)); // borrowed IUIElement* (piece copy_from_abi's to own a ref)
}

void day_xaml_set_geometry(void* h, int x, int y, int width, int height) {
    guard([&] {
        auto& e = elem(h);
        WUXC::Canvas::SetLeft(e, static_cast<double>(x));
        WUXC::Canvas::SetTop(e, static_cast<double>(y));
        if (auto fe = e.try_as<FrameworkElement>()) {
            fe.Width(static_cast<double>(width));
            fe.Height(static_cast<double>(height));
        }
    });
}

void day_xaml_measure(void* h, double aw, double ah, double* ow, double* oh) {
    *ow = 0; // sane defaults if a degraded element throws mid-measure (guard swallows it)
    *oh = 0;
    guard([&] {
        float fw = aw < 0 ? std::numeric_limits<float>::infinity() : static_cast<float>(aw);
        float fh = ah < 0 ? std::numeric_limits<float>::infinity() : static_cast<float>(ah);
        auto& e = elem(h);
        // Measure with the explicit size back at Auto. Once day has framed an element,
        // `set_geometry` has stamped a Width/Height on it, and XAML derives `DesiredSize` from
        // THOSE rather than from the content — so a re-measure after the content changed reports
        // the size the PREVIOUS content wanted. That is what leaves a recycled list cell showing
        // "Row" alone: the label was framed for "Row 4", the shuffle rebinds it to "Row 487", the
        // stale narrow frame comes back from measure, and TextWrapping::Wrap folds the number onto
        // a second line outside the row band. The frame is day's to re-apply through set_geometry
        // right after, so clearing it here costs nothing; it is restored below either way.
        auto fe = e.try_as<FrameworkElement>();
        double saved_w = 0, saved_h = 0;
        bool had_w = false, had_h = false;
        const double kAuto = std::numeric_limits<double>::quiet_NaN();
        if (fe) {
            saved_w = fe.Width();
            saved_h = fe.Height();
            had_w = !std::isnan(saved_w);
            had_h = !std::isnan(saved_h);
            if (had_w) fe.Width(kAuto);
            if (had_h) fe.Height(kAuto);
        }
        e.Measure(WF::Size{ fw, fh });
        auto d = e.DesiredSize();
        // day measures during its synchronous initial layout, before the island's first async layout
        // pass has applied control templates (so a templated control reports 0). A PARTIAL zero counts
        // too: a Button measured this early reports its template HEIGHT but 0 width (its content isn't
        // laid out yet), which would leave it invisible on, e.g., a stack's root page (no other page's
        // fully-zero element triggers the pass). Force a synchronous layout to expand templates, then
        // re-measure. UpdateLayout can fire SizeChanged → day's nav resize → back into measure, so a
        // re-entrancy guard keeps the forced layout to one level (the first pass lays out the tree,
        // after which controls are non-zero).
        static bool s_forcing_layout = false;
        if ((d.Width == 0 || d.Height == 0) && !s_forcing_layout) {
            s_forcing_layout = true;
            if (fe) fe.UpdateLayout(); // the outer `fe` — re-querying here only shadowed it (C4456)
            s_forcing_layout = false;
            e.Measure(WF::Size{ fw, fh });
            d = e.DesiredSize();
        }
        // Put the frame back before returning: day re-applies it through set_geometry only when
        // its layout actually changes, so an element it leaves alone must keep the size it had.
        if (fe) {
            if (had_w) fe.Width(saved_w);
            if (had_h) fe.Height(saved_h);
        }
        *ow = d.Width;
        *oh = d.Height;
    });
}

void day_xaml_set_enabled(void* h, int enabled) {
    guard([&] {
        if (auto c = elem(h).try_as<WUXC::Control>()) c.IsEnabled(enabled != 0);
    });
}

void day_xaml_set_visible(void* h, int visible) {
    guard([&] {
        elem(h).Visibility(visible ? WUX::Visibility::Visible : WUX::Visibility::Collapsed);
    });
}

// The element's laid-out size (points). Used by the list to size cells to its viewport width.
void day_xaml_widget_size(void* h, double* ow, double* oh) {
    *ow = 0;
    *oh = 0;
    guard([&] {
        if (auto fe = elem(h).try_as<FrameworkElement>()) {
            *ow = fe.ActualWidth();
            *oh = fe.ActualHeight();
        }
    });
}

void day_xaml_set_name(void* h, const char* name) {
    guard([&] { WUX::Automation::AutomationProperties::SetAutomationId(elem(h), hs(name)); });
}

// Attach a native pointer recognizer to `h`, reporting to cb(id, phase, x, y, tx, ty) with `x,y` in
// the element's local space (docs/shapes.md). kind 0 = Tap, 1 = LongPress, 2 = Drag. Phase codes:
// 0 Tap, 1 Drag-Began, 2 Drag-Changed, 3 Drag-Ended, 4 LongPress (matching day-xaml's on_gesture).
// Routed events bubble from the shape's Path children up to the Canvas handle, so only the drawn
// shape is hit (a transparent Canvas isn't hit-testable) — the "hit-test the path" semantic.
void day_xaml_enable_gesture(void* h, unsigned long long id, int kind,
                              void (*cb)(unsigned long long, int, double, double, double,
                                         double)) try {
    auto el = elem(h);
    if (kind == 2) { // Drag → manipulation (translate only)
        el.ManipulationMode(WUXIn::ManipulationModes::TranslateX |
                            WUXIn::ManipulationModes::TranslateY);
        el.ManipulationStarted(
            [id, cb](WF::IInspectable const&, WUXIn::ManipulationStartedRoutedEventArgs const& a) {
                auto p = a.Position();
                cb(id, 1, p.X, p.Y, 0, 0);
            });
        el.ManipulationDelta(
            [id, cb](WF::IInspectable const&, WUXIn::ManipulationDeltaRoutedEventArgs const& a) {
                auto p = a.Position();
                auto t = a.Cumulative().Translation;
                cb(id, 2, p.X, p.Y, t.X, t.Y);
            });
        el.ManipulationCompleted(
            [id, cb](WF::IInspectable const&, WUXIn::ManipulationCompletedRoutedEventArgs const& a) {
                auto p = a.Position();
                auto t = a.Cumulative().Translation;
                cb(id, 3, p.X, p.Y, t.X, t.Y);
            });
    } else if (kind == 1) { // LongPress → Holding (touch/pen; fire once on Started)
        el.IsHoldingEnabled(true);
        el.Holding([id, cb, el](WF::IInspectable const&, WUXIn::HoldingRoutedEventArgs const& a) {
            if (a.HoldingState() == WUIIn::HoldingState::Started) {
                auto p = a.GetPosition(el);
                cb(id, 4, p.X, p.Y, 0, 0);
            }
        });
    } else { // Tap
        el.Tapped([id, cb, el](WF::IInspectable const&, WUXIn::TappedRoutedEventArgs const& a) {
            auto p = a.GetPosition(el);
            cb(id, 0, p.X, p.Y, 0, 0);
        });
    }
} catch (...) {
}

// ---- snapshot (PrintWindow → Gdiplus PNG) ----

static int png_encoder_clsid(CLSID* clsid) {
    UINT num = 0, size = 0;
    Gdiplus::GetImageEncodersSize(&num, &size);
    if (size == 0) return -1;
    auto info = reinterpret_cast<Gdiplus::ImageCodecInfo*>(malloc(size));
    if (!info) return -1;
    Gdiplus::GetImageEncoders(num, size, info);
    int result = -1;
    for (UINT i = 0; i < num; ++i) {
        if (wcscmp(info[i].MimeType, L"image/png") == 0) {
            *clsid = info[i].Clsid;
            result = static_cast<int>(i);
            break;
        }
    }
    free(info);
    return result;
}

// Snapshot via RenderTargetBitmap: renders the XAML visual tree straight to a bitmap,
// independent of whether the host window is visible/foreground/composed (so it works for a
// background-launched app and on headless CI, unlike PrintWindow). Pixels are BGRA8 — which is
// exactly Gdiplus PixelFormat32bppARGB's in-memory byte order. Returns 0 on success.
int day_xaml_snapshot_png(void* win, const char* path) try {
    auto app = reinterpret_cast<AppWindow*>(win);
    if (!app || !app->root) return 1;

    WUXM::Imaging::RenderTargetBitmap rtb;
    pump_until_complete(rtb.RenderAsync(app->root));
    int pw = rtb.PixelWidth(), ph = rtb.PixelHeight();
    if (pw <= 0 || ph <= 0) return 2;

    auto pixelsOp = rtb.GetPixelsAsync();
    pump_until_complete(pixelsOp);
    auto buffer = pixelsOp.GetResults();

    auto access = buffer.as<::Windows::Storage::Streams::IBufferByteAccess>();
    uint8_t* bytes = nullptr;
    access->Buffer(&bytes);
    if (!bytes || buffer.Length() < static_cast<uint32_t>(pw) * ph * 4) return 5;

    int rc_out = 3;
    ULONG_PTR token = 0;
    Gdiplus::GdiplusStartupInput si;
    if (Gdiplus::GdiplusStartup(&token, &si, nullptr) == Gdiplus::Ok) {
        {
            Gdiplus::Bitmap bitmap(pw, ph, PixelFormat32bppARGB);
            Gdiplus::Rect rect(0, 0, pw, ph);
            Gdiplus::BitmapData bd;
            if (bitmap.LockBits(&rect, Gdiplus::ImageLockModeWrite, PixelFormat32bppARGB, &bd) ==
                Gdiplus::Ok) {
                for (int y = 0; y < ph; ++y) {
                    memcpy(static_cast<uint8_t*>(bd.Scan0) + y * bd.Stride,
                           bytes + static_cast<size_t>(y) * pw * 4, static_cast<size_t>(pw) * 4);
                }
                bitmap.UnlockBits(&bd);
                CLSID clsid;
                if (png_encoder_clsid(&clsid) >= 0) {
                    std::wstring wpath = hs(path).c_str();
                    if (bitmap.Save(wpath.c_str(), &clsid, nullptr) == Gdiplus::Ok) rc_out = 0;
                }
            }
        } // bitmap destroyed before GdiplusShutdown
        Gdiplus::GdiplusShutdown(token);
    }
    return rc_out;
} catch (...) {
    return 9;
}

} // extern "C"

// ---- menus (docs/menus.md) ------------------------------------------------
// Context menus are MenuFlyouts set as a UIElement's ContextFlyout (right-click / press-hold);
// the app menu is a MenuBar docked at the top of the root Canvas. Both are built from the same
// tab/newline spec (kind \t id \t role \t key \t mods \t enabled \t label) so the Rust side only
// serializes the day-neutral tree once. Custom items fire g_menu_cb(id); roles carry the standard
// keyboard accelerator (and Quit closes the window). CI-built (no live Windows verification).

static void (*g_menu_cb)(unsigned long long) = nullptr;

static std::vector<std::string> split_tabs(const std::string& s) {
    std::vector<std::string> out;
    size_t p = 0;
    while (true) {
        size_t t = s.find('\t', p);
        if (t == std::string::npos) { out.push_back(s.substr(p)); break; }
        out.push_back(s.substr(p, t - p));
        p = t + 1;
    }
    return out;
}

static std::vector<std::string> split_lines(const std::string& s) {
    std::vector<std::string> out;
    size_t p = 0;
    while (true) {
        size_t nl = s.find('\n', p);
        out.push_back(s.substr(p, nl == std::string::npos ? std::string::npos : nl - p));
        if (nl == std::string::npos) break;
        p = nl + 1;
    }
    return out;
}

// `Windows.System.VirtualKey` names no member for the OEM punctuation codes (0xBA–0xC0,
// 0xDB–0xDF) — the enum stops naming values well before them — yet those are exactly the codes
// day's shortcut mapping emits for `,` `.` `-` `=` `/` (win_keycode). XAML derives a
// MenuFlyoutItem's shortcut text from the accelerator's key when the menu OPENS, and that
// generator fail-fasts on a value it cannot name: STATUS_STOWED_EXCEPTION (0xC000027B) inside
// Windows.UI.Xaml.dll, killing the process as the flyout appears rather than when the shortcut
// is ever pressed. `Ctrl+,` (the standard Settings/Preferences shortcut day auto-injects) means
// any app with a preferences item crashed on its first File-menu open.
//
// Naming the key ourselves keeps that generator out of the path; the accelerator itself is
// still registered, so the shortcut keeps working.
static const char* oem_key_name(int key) {
    switch (key) {
        case 0xBA: return ";";
        case 0xBB: return "=";
        case 0xBC: return ",";
        case 0xBD: return "-";
        case 0xBE: return ".";
        case 0xBF: return "/";
        case 0xC0: return "`";
        case 0xDB: return "[";
        case 0xDC: return "\\";
        case 0xDD: return "]";
        case 0xDE: return "'";
        default: return nullptr;
    }
}

static void add_accel(WUXC::MenuFlyoutItem const& item, int key, int mods,
                      std::function<void()> fire, bool global_accels) {
    if (key == 0) return;
    // An OEM key never becomes a KeyboardAccelerator: XAML fail-fasts while REGISTERING one
    // whose key it cannot name — supplying KeyboardAcceleratorTextOverride does NOT avoid it,
    // the accelerator object itself is the trigger. The item still shows the shortcut, and the
    // message loop in day_xaml_run dispatches it.
    const char* oem = oem_key_name(key);
    if (!oem) {
        WUXIn::KeyboardAccelerator ka;
        ka.Key(static_cast<WS::VirtualKey>(key));
        auto m = WS::VirtualKeyModifiers::None;
        if (mods & 1) m |= WS::VirtualKeyModifiers::Control;
        if (mods & 2) m |= WS::VirtualKeyModifiers::Shift;
        if (mods & 4) m |= WS::VirtualKeyModifiers::Menu;
        ka.Modifiers(m);
        item.KeyboardAccelerators().Append(ka);
        return;
    }
    // Windows' own modifier order, matching what XAML generates for the keys it can name.
    std::string text;
    if (mods & 1) text += "Ctrl+";
    if (mods & 2) text += "Shift+";
    if (mods & 4) text += "Alt+";
    text += oem;
    item.KeyboardAcceleratorTextOverride(hs(text.c_str()));
    // An unmodified OEM key is ordinary typing — claiming it globally would swallow every `,`
    // headed for a TextBox. Only a modified combination becomes a shortcut.
    if (global_accels && mods != 0 && fire) g_oem_accels.push_back({ key, mods, std::move(fire) });
}

// Append the flat menu spec into a MenuFlyoutItemBase collection (a MenuFlyout / MenuFlyoutSubItem /
// MenuBarItem Items()), tracking submenu depth with a stack.
static void build_menu_items(WF::Collections::IVector<WUXC::MenuFlyoutItemBase> root,
                             const std::string& spec, bool global_accels = false) {
    std::vector<WF::Collections::IVector<WUXC::MenuFlyoutItemBase>> stack;
    stack.push_back(root);
    for (auto const& line : split_lines(spec)) {
        if (line.empty()) continue;
        auto f = split_tabs(line);
        std::string kind = f.size() > 0 ? f[0] : "";
        std::string label = f.size() > 6 ? f[6] : "";
        auto cur = stack.back();
        if (kind == "-") {
            cur.Append(WUXC::MenuFlyoutSeparator{});
        } else if (kind == "S") {
            WUXC::MenuFlyoutSubItem sub;
            sub.Text(hs(label.c_str()));
            cur.Append(sub);
            stack.push_back(sub.Items());
        } else if (kind == "E") {
            if (stack.size() > 1) stack.pop_back();
        } else { // "A" action, "R" role
            WUXC::MenuFlyoutItem item;
            item.Text(hs(label.c_str()));
            bool enabled = !(f.size() > 5 && f[5] == "0");
            item.IsEnabled(enabled);
            int key = f.size() > 3 ? std::atoi(f[3].c_str()) : 0;
            int mods = f.size() > 4 ? std::atoi(f[4].c_str()) : 0;
            // What this item does — shared by the Click handler and, for an OEM shortcut, by
            // the message loop's own dispatch, so both paths can never drift apart.
            std::function<void()> fire;
            // `MenuRole::CloseWindow` closes the window the MENU is in, so unlike every other
            // role it needs the clicked item to say which that is — hence the sender-aware arm
            // below rather than a plain `fire`.
            bool closes_window = false;
            if (kind == "A") {
                unsigned long long aid = f.size() > 1 ? std::strtoull(f[1].c_str(), nullptr, 10) : 0;
                fire = [aid] { if (g_menu_cb) g_menu_cb(aid); };
            } else {
                int role = f.size() > 2 ? std::atoi(f[2].c_str()) : -1;
                if (role == 8) { // Quit — ends the app, whichever window it was chosen from
                    fire = [] { if (g_app && g_app->host) PostMessageW(g_app->host, WM_CLOSE, 0, 0); };
                }
                // Role 11 = CloseWindow. It had NO handler at all: a live, enabled File ▸ Close
                // that did nothing. That was survivable while only the primary window carried a
                // menu; now every window does, and a dead Close sits directly above Quit — which
                // DOES end the whole app — right where someone reaches to close one window.
                closes_window = role == 11;
            }
            if (fire || closes_window) {
                item.Click([fire, closes_window](WF::IInspectable const& s,
                                                 WUX::RoutedEventArgs const&) {
                    if (closes_window) {
                        // Same WM_CLOSE the title-bar X sends, so both routes tear the window
                        // down through one path (secondary: hide + day-side teardown; primary:
                        // lifecycle-terminate, then the app ends with its window).
                        if (auto fe = s.try_as<WUX::FrameworkElement>())
                            if (HWND h = host_for_element(fe)) PostMessageW(h, WM_CLOSE, 0, 0);
                        return;
                    }
                    if (fire) fire();
                });
            }
            // A disabled item must not fire from the keyboard either.
            add_accel(item, key, mods, enabled ? fire : std::function<void()>{}, global_accels);
            cur.Append(item);
        }
    }
}

extern "C" void day_xaml_set_menu_cb(void (*cb)(unsigned long long)) { g_menu_cb = cb; }

extern "C" void day_xaml_set_context_menu(void* h, const char* spec) try {
    if (!h) return;
    auto e = elem(h);
    if (!spec || !*spec) {
        e.ContextFlyout(nullptr);
        return;
    }
    WUXC::MenuFlyout fly;
    build_menu_items(fly.Items(), spec);
    e.ContextFlyout(fly);
} catch (...) {
}

/// Build the docked MenuBar for `root`, replacing any previous one; null for an empty spec.
///
/// Shared by the primary window and every secondary one (docs/windows.md): day's app menu has no
/// window parameter — it is one menu for the app, the macOS shape — but Windows draws it per
/// window, so the same spec is installed into each. `global_accels` is reserved for the primary:
/// the OEM shortcuts it registers dispatch app-level action ids, so registering them once is both
/// sufficient and what keeps a second window from clearing the first window's set.
/// The Alt-key letter for one menu title. Windows menu bars are reachable from the keyboard —
/// Alt lights the KeyTips, Alt+<letter> opens that menu — and XAML drives all of it from
/// `AccessKey`. Day's menu model carries no mnemonic (no `&File` convention), so derive one the
/// way a Win32 app conventionally would: the title's first letter that is still free. Dedup
/// matters more than the choice — two menus sharing a key leaves BOTH unreachable, and localized
/// titles collide readily ("Affichage"/"Aide" both want A).
static winrt::hstring pick_access_key(const std::string& label, std::vector<wchar_t>& taken) {
    std::wstring w{ hs(label.c_str()).c_str() };
    for (wchar_t c : w) {
        wchar_t up = static_cast<wchar_t>(std::towupper(c));
        if (!std::iswalpha(static_cast<wint_t>(up))) continue;
        if (std::find(taken.begin(), taken.end(), up) != taken.end()) continue;
        taken.push_back(up);
        return winrt::hstring{ std::wstring(1, up) };
    }
    return winrt::hstring{}; // every letter taken: no key rather than a stolen one
}

static WUXC::MenuBar install_menu_bar(WUXC::Canvas const& root, const char* spec,
                                      bool global_accels) {
    // Remove any prior MenuBar we docked (named "day_menubar"). `Children()` returns the
    // UIElementCollection by value (a projection over the real collection), so bind it by value —
    // a non-const reference can't bind to that rvalue (C2440) — mutations still hit the real one.
    auto kids = root.Children();
    for (uint32_t i = 0; i < kids.Size(); ++i) {
        if (auto fe = kids.GetAt(i).try_as<FrameworkElement>()) {
            if (fe.Name() == L"day_menubar") { kids.RemoveAt(i); break; }
        }
    }
    if (!spec || !*spec) return nullptr;
    WUXC::MenuBar bar;
    bar.Name(L"day_menubar");
    // Top-level "S" groups become MenuBarItems; a bare item wraps in an unnamed MenuBarItem.
    auto lines = split_lines(spec);
    std::vector<wchar_t> access_taken;
    size_t i = 0;
    while (i < lines.size()) {
        if (lines[i].empty()) { ++i; continue; }
        auto f = split_tabs(lines[i]);
        std::string kind = f.size() > 0 ? f[0] : "";
        if (kind == "S") {
            std::string label = f.size() > 6 ? f[6] : "";
            WUXC::MenuBarItem mbi;
            mbi.Title(hs(label.c_str()));
            if (auto key = pick_access_key(label, access_taken); !key.empty()) mbi.AccessKey(key);
            int depth = 1;
            std::string inner;
            ++i;
            while (i < lines.size() && depth > 0) {
                auto ff = split_tabs(lines[i]);
                std::string k = ff.empty() ? "" : ff[0];
                if (k == "S") depth++;
                else if (k == "E") { depth--; if (depth == 0) { ++i; break; } }
                inner += lines[i];
                inner += "\n";
                ++i;
            }
            build_menu_items(mbi.Items(), inner, global_accels);
            bar.Items().Append(mbi);
        } else {
            WUXC::MenuBarItem mbi;
            mbi.Title(hs(""));
            build_menu_items(mbi.Items(), lines[i] + "\n", global_accels);
            bar.Items().Append(mbi);
            ++i;
        }
    }
    WUXC::Canvas::SetLeft(bar, 0);
    WUXC::Canvas::SetTop(bar, 0);
    root.Children().Append(bar);
    return bar;
}

extern "C" void day_xaml_set_app_menu(void* win, const char* spec) try {
    auto app = reinterpret_cast<AppWindow*>(win);
    if (!app || !app->root) return;
    // The OEM shortcuts belong to the menu being replaced; drop them with it.
    g_oem_accels.clear();
    app->menubar = install_menu_bar(app->root, spec, true);
    day_xaml_relayout_chrome(app);
} catch (...) {
}

// The same app menu, docked in a secondary window (docs/windows.md).
extern "C" void day_xaml_window_set_menu2(void* win, const char* spec) try {
    auto sw = static_cast<SecWindow*>(win);
    if (!sw || !sw->root) return;
    sw->menubar = install_menu_bar(sw->root, spec, false);
    relayout_sec_chrome(sw);
} catch (...) {
}

// ---- window toolbar (docs/toolbars.md) ------------------------------------
// A Fluent CommandBar docked under the menu bar. `PrimaryCommands` — which the CommandBar
// template right-aligns — carries the AppBarButton / AppBarToggleButton / AppBarSeparator
// commands, and `Content` — which it left-aligns — carries the leading items in a horizontal
// StackPanel. That split is what the model's flexible space MEANS on Windows: items before it
// are leading Content, items after it are primary commands. A search field, a label or a fixed
// gap lands in Content whichever side it was written on, because system XAML's PrimaryCommands
// takes only ICommandBarElement — AppBarElementContainer, which would wrap an arbitrary control,
// is WinUI's and not in Windows.UI.Xaml.
//
// The spec is ONE flat blob, like the menu spec above. One line per item:
//   kind \t id \t action \t enabled \t on \t glyph \t image \t label \t tooltip \t text \t placeholder
// kinds: B button, T toggle, M menu, F search field, L label, `-` separator, `_` fixed space,
// `>` flexible space (the Content/PrimaryCommands split). `on` seeds a toggle and `text` a search
// field; `glyph` is a Segoe Fluent Icons code point in hex, `image` a bundled image FILE NAME.
// An `M` line is followed by that item's MENU spec — the same lines build_menu_items already
// parses — closed by an `X` line, so the sub-spec is sliced out here and handed straight to it.
// Buttons ride the same g_menu_cb rail as menu items; a toggle's state and a search field's text
// go through g_toolbar_cb. CI-built (no live Windows verification).

// kind 0 = toggle (`on`), kind 1 = search text (`text`).
static void (*g_toolbar_cb)(unsigned long long, int, int, const char*) = nullptr;

// Where the install in progress records its items. Set around a build, since the item
// construction below is many frames deep and threading a parameter through it all buys nothing
// on a single UI thread.
static ToolbarElems* g_toolbar_target = nullptr;

/// One window's live toolbar item, or null. `win` is the window token the install used — the
/// primary AppWindow* or a SecWindow* — so a patch reaches the item in the window that owns it.
static FrameworkElement find_toolbar_elem(void* win, const char* id) {
    auto w = g_toolbar_elems.find(win);
    if (w == g_toolbar_elems.end()) return nullptr;
    auto it = w->second.find(std::string(id ? id : ""));
    return it == w->second.end() ? nullptr : it->second;
}

// Completions for a search field (docs/search.md): AutoSuggestBox's own ItemsSource, so the popup,
// the keyboard handling and the Fluent styling are the platform's. The list is unit-separated
// (\x1f) because tabs and newlines are the spec's record separators. Defined ABOVE the toolbar
// install that seeds it — C++ resolves plain calls by declaration order, so a definition parked
// down with the patch entry point below would not be visible there.
static void day_xaml_fill_suggestions(WUXC::AutoSuggestBox const& box, std::string const& joined) {
    auto items = winrt::single_threaded_observable_vector<WF::IInspectable>();
    size_t start = 0;
    while (start <= joined.size() && !joined.empty()) {
        size_t sep = joined.find('\x1f', start);
        std::string one = joined.substr(start, sep == std::string::npos ? std::string::npos
                                                                        : sep - start);
        if (!one.empty()) items.Append(winrt::box_value(hs(one.c_str())));
        if (sep == std::string::npos) break;
        start = sep + 1;
    }
    box.ItemsSource(items);
}

// A programmatic IsChecked write is in flight. ToggleButton raises Checked/Unchecked
// synchronously from the setter, so a flag around the write keeps day's own value from echoing
// back through g_toolbar_cb. (The search field can't use one: AutoSuggestBox raises TextChanged
// asynchronously, which is why that handler filters on the change REASON instead.)
static bool g_toolbar_setting_checked = false;

extern "C" void day_xaml_set_toolbar_cb(void (*cb)(unsigned long long, int, int, const char*)) {
    g_toolbar_cb = cb;
}

// Segoe Fluent Icons is the Windows 11 icon font; Windows 10 ships its predecessor, Segoe MDL2
// Assets, which carries the same code points for every glyph day maps. A family the system does
// not have draws notdef boxes rather than falling back to a real glyph, so pick by the file the
// Win11 font installs. Resolved once — it cannot change while the process runs.
static winrt::hstring const& toolbar_icon_font() {
    static winrt::hstring family = [] {
        wchar_t dir[MAX_PATH]{};
        UINT n = GetWindowsDirectoryW(dir, MAX_PATH);
        std::wstring path = (n > 0 && n < MAX_PATH) ? std::wstring(dir) : std::wstring(L"C:\\Windows");
        path += L"\\Fonts\\SegoeIcons.ttf";
        return GetFileAttributesW(path.c_str()) != INVALID_FILE_ATTRIBUTES
                   ? winrt::hstring{ L"Segoe Fluent Icons" }
                   : winrt::hstring{ L"Segoe MDL2 Assets" };
    }();
    return family;
}

// An item's icon: the standard symbol's glyph, else a bundled image staged next to the exe (an
// ms-appx BitmapIcon, as the nav icons load), else none — an AppBarButton without an Icon shows
// its label alone, which is exactly what an unmapped symbol should look like.
static WUXC::IconElement toolbar_icon(const std::string& glyph, const std::string& image,
                                      const std::string& geom = std::string()) {
    if (!glyph.empty()) {
        // Every code point day maps is in the BMP private-use area: one UTF-16 unit.
        wchar_t ch = static_cast<wchar_t>(std::strtoul(glyph.c_str(), nullptr, 16));
        if (ch) {
            WUXC::FontIcon fi;
            fi.FontFamily(WUXM::FontFamily(toolbar_icon_font()));
            fi.Glyph(winrt::hstring{ std::wstring(1, ch) });
            return fi;
        }
    }
    // Vector before raster, as the nav pane does: a PathIcon is resolution-independent and takes
    // the bar's Foreground, so it themes itself. No tint channel here — a toolbar command is
    // monochrome chrome by definition, and the CommandBar's own foreground is the right colour in
    // both schemes and in the disabled/pressed visuals.
    if (!geom.empty()) {
        std::string spec = geom;
        // Undo both escapes day-xaml applied: this line format owns \t and \n, and a geometry
        // spec uses both (\n between shapes, \t before a shape's path data).
        for (auto& c : spec) {
            if (c == '\x1f') c = '\n';
            else if (c == '\x1e') c = '\t';
        }
        // 16 px is the AppBarButton icon box the default template lays out.
        if (void* h = day_xaml_vector_icon_new(spec.c_str(), 0u, 0, 16.0)) {
            auto ie = elem(h).try_as<WUXC::IconElement>();
            day_xaml_delete(h); // the command owns it now
            if (ie) return ie;
        }
    }
    if (!image.empty()) {
        WUXC::BitmapIcon bicon;
        bicon.UriSource(WF::Uri{ hs(("ms-appx:///images/" + image).c_str()) });
        bicon.ShowAsMonochrome(true); // tint to the bar foreground (theme-adaptive)
        return bicon;
    }
    return nullptr;
}

/// Build the docked CommandBar for `root`, replacing any previous one; null for an empty spec.
/// `elems` receives this window's id→element map for the targeted patches. Shared by the primary
/// window and every secondary one, exactly like `install_menu_bar`.
static WUXC::CommandBar install_toolbar_bar(WUXC::Canvas const& root, const char* spec,
                                            ToolbarElems* elems) {
    // Take out the bar a previous install docked (named "day_toolbar"), the same way the MenuBar
    // above is replaced; `Children()` is a projection returned by value, so bind it by value.
    auto kids = root.Children();
    for (uint32_t i = 0; i < kids.Size(); ++i) {
        if (auto fe = kids.GetAt(i).try_as<FrameworkElement>()) {
            if (fe.Name() == L"day_toolbar") { kids.RemoveAt(i); break; }
        }
    }
    if (elems) elems->clear();
    if (!spec || !*spec) return nullptr;
    g_toolbar_target = elems;

    WUXC::CommandBar bar;
    bar.Name(L"day_toolbar");
    // Labels beside the icons: the desktop CommandBar look, and it keeps the strip one row tall.
    bar.DefaultLabelPosition(WUXC::CommandBarDefaultLabelPosition::Right);
    bar.HorizontalContentAlignment(WUX::HorizontalAlignment::Left);
    bar.VerticalContentAlignment(WUX::VerticalAlignment::Center);

    WUXC::StackPanel lead;
    lead.Orientation(WUXC::Orientation::Horizontal);
    lead.VerticalAlignment(WUX::VerticalAlignment::Center);

    bool trailing = false; // past the first flexible space
    // A command: a primary command past the flexible space, a leading-panel child before it.
    // Either way it is remembered by id, which is what the targeted patches address.
    auto place_command = [&](FrameworkElement const& e, const std::string& id) {
        if (trailing) bar.PrimaryCommands().Append(e.as<WUXC::ICommandBarElement>());
        else lead.Children().Append(e);
        if (!id.empty() && g_toolbar_target) g_toolbar_target->insert_or_assign(id, e);
    };
    // A leading AppBarButton is outside the bar's own collections, so DefaultLabelPosition does
    // not reach it and it would draw its label UNDER the icon — two rows tall. Collapse the label
    // where there is an icon to show instead; without one the label is the only click target.
    auto compact = [&](WUXC::AppBarButton const& b, bool has_icon) {
        if (!trailing && has_icon) b.LabelPosition(WUXC::CommandBarLabelPosition::Collapsed);
    };

    auto lines = split_lines(spec);
    for (size_t i = 0; i < lines.size(); ++i) {
        if (lines[i].empty()) continue;
        auto f = split_tabs(lines[i]);
        auto fld = [&f](size_t n) { return f.size() > n ? f[n] : std::string(); };
        std::string kind = fld(0), id = fld(1);
        unsigned long long action = std::strtoull(fld(2).c_str(), nullptr, 10);
        bool enabled = fld(3) != "0";
        bool on = fld(4) == "1";
        std::string glyph = fld(5), image = fld(6), label = fld(7), tip = fld(8), text = fld(9),
                    placeholder = fld(10), suggestions = fld(11), geom = fld(12);
        bool has_icon = !glyph.empty() || !image.empty() || !geom.empty();

        if (kind == ">") {
            trailing = true;
        } else if (kind == "-") {
            if (trailing) {
                bar.PrimaryCommands().Append(
                    WUXC::AppBarSeparator{}.as<WUXC::ICommandBarElement>());
            } else {
                // AppBarSeparator sizes itself against the bar's own row, not against a
                // StackPanel, so a leading divider is a hairline of our own — the same
                // translucent neutral the divider piece falls back to, which reads in both
                // schemes.
                WUXC::Border rule;
                rule.Width(1);
                rule.Height(20);
                rule.Margin(WUX::Thickness{ 6, 0, 6, 0 });
                rule.Background(WUXM::SolidColorBrush(color_argb(0x33'808080u)));
                lead.Children().Append(rule);
            }
        } else if (kind == "_") {
            // A fixed gap. PrimaryCommands spaces its own commands and takes no filler element,
            // so a trailing gap has nothing to be.
            if (!trailing) {
                WUXC::Border gap;
                gap.Width(12);
                lead.Children().Append(gap);
            }
        } else if (kind == "F") {
            WUXC::AutoSuggestBox box;
            day_xaml_fill_suggestions(box, suggestions);
            box.Text(hs(text.c_str()));
            box.PlaceholderText(hs(placeholder.c_str()));
            box.QueryIcon(toolbar_icon("E721", "")); // the search glyph
            box.IsEnabled(enabled);
            box.Width(240); // a bar search field is sized, not stretched
            box.Margin(WUX::Thickness{ 4, 0, 4, 0 });
            if (action) {
                box.TextChanged([action](WUXC::AutoSuggestBox const& s,
                                         WUXC::AutoSuggestBoxTextChangedEventArgs const& a) {
                    // day writing the bound signal back raises this too; reporting that would
                    // echo the app's own value straight back at it.
                    if (a.Reason() != WUXC::AutoSuggestionBoxTextChangeReason::UserInput) return;
                    std::string str = u8(s.Text());
                    if (g_toolbar_cb) g_toolbar_cb(action, 1, 0, str.c_str());
                });
            }
            lead.Children().Append(box);
            if (!id.empty() && g_toolbar_target) g_toolbar_target->insert_or_assign(id, box);
        } else if (kind == "S") {
            // The sidebar toggle is REALIZED BY THE NAVIGATIONVIEW, not by a bar command: its
            // built-in PaneToggleButton is the hamburger Windows puts at the head of the pane,
            // and drawing our own beside it left the window with two stacked hamburgers doing the
            // same thing. A split nav in this window already shows one, so drop the bar copy.
            //
            // Only the ELEMENT goes: the item stays in day's toolbar model, so `toolbar:` in
            // dayscript still resolves it and still dispatches through the toggle-sidebar duty
            // (which drives `g_navviews` directly, never this button).
            //
            // Unconditional rather than "only when this window has a split nav": the toolbar can
            // be installed before the nav is realized, so the conditional would depend on order.
            // Nothing is lost by always dropping it — in a window with a split nav the built-in
            // button does the job, and in a window WITHOUT one this button drove nothing anyway
            // (`day_xaml_toggle_sidebar` no-ops on an empty `g_navviews`).
            continue;
        } else if (kind == "L") {
            WUXC::TextBlock caption;
            caption.Text(hs(label.c_str()));
            caption.VerticalAlignment(WUX::VerticalAlignment::Center);
            caption.Margin(WUX::Thickness{ 8, 0, 8, 0 });
            lead.Children().Append(caption);
            if (!id.empty() && g_toolbar_target) g_toolbar_target->insert_or_assign(id, caption);
        } else if (kind == "T") {
            WUXC::AppBarToggleButton toggle;
            toggle.Label(hs(label.c_str()));
            toggle.Icon(toolbar_icon(glyph, image, geom));
            toggle.IsEnabled(enabled);
            toggle.IsChecked(on);
            if (!trailing && has_icon)
                toggle.LabelPosition(WUXC::CommandBarLabelPosition::Collapsed);
            WUXC::ToolTipService::SetToolTip(toggle, winrt::box_value(hs(tip.c_str())));
            if (action) {
                toggle.Checked([action](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
                    if (g_toolbar_cb && !g_toolbar_setting_checked) g_toolbar_cb(action, 0, 1, "");
                });
                toggle.Unchecked([action](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
                    if (g_toolbar_cb && !g_toolbar_setting_checked) g_toolbar_cb(action, 0, 0, "");
                });
            }
            place_command(toggle, id);
        } else if (kind == "M") {
            // The item's own menu spec follows, closed by an `X` line: slice it out and let the
            // menu builder fill a MenuFlyout with it, so a toolbar menu and its menu-bar twin are
            // built by the same code.
            std::string inner;
            size_t j = i + 1;
            for (; j < lines.size(); ++j) {
                auto ff = split_tabs(lines[j]);
                if (!ff.empty() && ff[0] == "X") break;
                inner += lines[j];
                inner += "\n";
            }
            i = j; // resume after the terminator (or at the end of the spec)
            WUXC::AppBarButton button;
            button.Label(hs(label.c_str()));
            button.Icon(toolbar_icon(glyph, image, geom));
            button.IsEnabled(enabled);
            compact(button, has_icon);
            WUXC::ToolTipService::SetToolTip(button, winrt::box_value(hs(tip.c_str())));
            WUXC::MenuFlyout fly;
            build_menu_items(fly.Items(), inner);
            button.Flyout(fly);
            place_command(button, id);
        } else { // "B", and anything a later model adds: a plain command
            WUXC::AppBarButton button;
            button.Label(hs(label.c_str()));
            button.Icon(toolbar_icon(glyph, image, geom));
            button.IsEnabled(enabled);
            compact(button, has_icon);
            WUXC::ToolTipService::SetToolTip(button, winrt::box_value(hs(tip.c_str())));
            if (action) {
                button.Click([action](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
                    if (g_menu_cb) g_menu_cb(action);
                });
            }
            place_command(button, id);
        }
    }

    if (lead.Children().Size() > 0) bar.Content(lead);
    WUXC::Canvas::SetLeft(bar, 0);
    // The top offset (below the menu bar, if there is one) and the width are relayout's job —
    // the two bars are installed in either order.
    root.Children().Append(bar);
    g_toolbar_target = nullptr;
    return bar;
}

extern "C" void day_xaml_toolbar_set_suggestions(void* win, const char* id, const char* joined) try {
    auto e = find_toolbar_elem(win, id);
    if (auto box = e.try_as<WUXC::AutoSuggestBox>())
        day_xaml_fill_suggestions(box, std::string(joined ? joined : ""));
} catch (...) {
}

extern "C" void day_xaml_set_toolbar(void* win, const char* spec) try {
    auto app = reinterpret_cast<AppWindow*>(win);
    if (!app || !app->root) return;
    app->toolbar = install_toolbar_bar(app->root, spec, &g_toolbar_elems[win]);
    day_xaml_relayout_chrome(app);
} catch (...) {
    g_toolbar_target = nullptr;
}

// A secondary window's own toolbar (docs/toolbars.md): every window an app opens installs its
// own item list, and each one drives the window it is in.
extern "C" void day_xaml_window_set_toolbar2(void* win, const char* spec) try {
    auto sw = static_cast<SecWindow*>(win);
    if (!sw || !sw->root) return;
    sw->toolbar = install_toolbar_bar(sw->root, spec, &g_toolbar_elems[win]);
    relayout_sec_chrome(sw);
} catch (...) {
    g_toolbar_target = nullptr;
}

extern "C" void day_xaml_toolbar_set_text(void* win, const char* id, const char* text) try {
    auto e = find_toolbar_elem(win, id);
    if (!e) return;
    auto box = e.try_as<WUXC::AutoSuggestBox>();
    if (!box) return;
    // Write only a real change, exactly as day_xaml_textbox_set_text does: an identical Text
    // raises no TextChanged at all, so the app's caret and selection survive the sync.
    auto next = hs(text);
    if (box.Text() != next) box.Text(next);
} catch (...) {
}

extern "C" void day_xaml_toolbar_set_checked(void* win, const char* id, int on) try {
    auto e = find_toolbar_elem(win, id);
    if (!e) return;
    auto toggle = e.try_as<WUXC::AppBarToggleButton>();
    if (!toggle) return;
    auto current = toggle.IsChecked(); // IReference<bool>: null = indeterminate
    if (current && current.Value() == (on != 0)) return;
    g_toolbar_setting_checked = true;
    toggle.IsChecked(on != 0);
    g_toolbar_setting_checked = false;
} catch (...) {
    g_toolbar_setting_checked = false;
}

extern "C" void day_xaml_toolbar_set_enabled(void* win, const char* id, int on) try {
    auto e = find_toolbar_elem(win, id);
    if (!e) return;
    // A label item is a TextBlock, which has no enabled state — the cast fails and nothing
    // happens, which is the honest outcome for "disable a caption".
    if (auto control = e.try_as<WUXC::Control>()) control.IsEnabled(on != 0);
} catch (...) {
}

// ---------------------------------------------------------------------------
// present / dismiss (docs/dialogs.md)
// ---------------------------------------------------------------------------
// Native modals mirroring day-qt-sys's flat C ABI: a ContentDialog for alert/prompt, WinRT file
// pickers for open/save. Everything is ASYNC (ShowAsync / Pick*Async) so present() returns at once
// — the message loop keeps pumping and dayscript can `respond`/`dismiss` — with a per-`req` registry
// so dismiss() can close/cancel a still-open modal. Results flow back through
// g_present_cb(req, tag, index, text): tag 0 dismissed, 1 button@index, 2 text, 3 files(path) —
// matching day_spec::PresentResult::decode. Each entry is completed by the modal's OWN handler
// (which erases it and fires the cb once), like the Qt shim; dismiss only starts the close.
// present() bodies are try/catch: a WinRT throw must not cross into Rust's run_posted (would abort),
// and on failure we leave the request pending so a scripted `respond` can still resolve it.

static void (*g_present_cb)(uint64_t, int, long long, const char*) = nullptr;

struct DayPresent {
    WUXC::ContentDialog dialog{ nullptr };
    WF::IAsyncInfo op{ nullptr }; // file-picker async op — cancelable by dismiss
};
static std::map<uint64_t, DayPresent> g_presents;

// Split a unit-separator-joined (0x1f) list, dropping empty parts (buttons_joined / filters_joined).
static std::vector<std::string> split_units(const char* joined) {
    std::vector<std::string> out;
    std::string s = joined ? joined : "";
    size_t p = 0;
    while (true) {
        size_t u = s.find('\x1f', p);
        std::string part = s.substr(p, u == std::string::npos ? std::string::npos : u - p);
        if (!part.empty()) out.push_back(part);
        if (u == std::string::npos) break;
        p = u + 1;
    }
    return out;
}

extern "C" void day_xaml_set_present_cb(void (*cb)(uint64_t, int, long long, const char*)) {
    g_present_cb = cb;
}

extern "C" void day_xaml_present_dialog(uint64_t req, const char* title, const char* message,
                                         const char* buttons_joined, const char* roles_joined,
                                         void* win) try {
    (void)roles_joined; // ContentDialog styles by slot, not per-button role
    auto app = reinterpret_cast<AppWindow*>(win);
    if (!app || !app->root) { if (g_present_cb) g_present_cb(req, 0, 0, ""); return; }
    WUXC::ContentDialog dlg;
    dlg.XamlRoot(app->root.XamlRoot());
    if (title && *title) dlg.Title(winrt::box_value(hs(title)));
    if (message && *message) dlg.Content(winrt::box_value(hs(message)));
    // Map buttons in spec order to ContentDialog's Primary / Secondary / Close slots; the async
    // result maps back to the original index. Windows dialogs are conventionally <=3 buttons; any
    // beyond the third are dropped (the showcase's alert/confirm/delete/sheet all fit).
    auto labels = split_units(buttons_joined);
    int nbtns = static_cast<int>(labels.size());
    if (nbtns > 0) dlg.PrimaryButtonText(hs(labels[0].c_str()));
    if (nbtns > 1) dlg.SecondaryButtonText(hs(labels[1].c_str()));
    if (nbtns > 2) dlg.CloseButtonText(hs(labels[2].c_str()));

    g_presents[req].dialog = dlg;
    dlg.ShowAsync().Completed(
        [req, nbtns](WF::IAsyncOperation<WUXC::ContentDialogResult> const& a, WF::AsyncStatus st) {
            if (!g_presents.erase(req)) return; // already resolved (scripted respond) → drop
            if (!g_present_cb) return;
            auto res = (st == WF::AsyncStatus::Completed) ? a.GetResults()
                                                          : WUXC::ContentDialogResult::None;
            if (res == WUXC::ContentDialogResult::Primary && nbtns > 0) g_present_cb(req, 1, 0, "");
            else if (res == WUXC::ContentDialogResult::Secondary && nbtns > 1) g_present_cb(req, 1, 1, "");
            else if (res == WUXC::ContentDialogResult::None && nbtns > 2) g_present_cb(req, 1, 2, "");
            else g_present_cb(req, 0, 0, "");
        });
} catch (...) {
}

extern "C" void day_xaml_present_prompt(uint64_t req, const char* title, const char* message,
                                         const char* placeholder, const char* initial,
                                         const char* ok, const char* cancel, void* win) try {
    auto app = reinterpret_cast<AppWindow*>(win);
    if (!app || !app->root) { if (g_present_cb) g_present_cb(req, 0, 0, ""); return; }
    WUXC::ContentDialog dlg;
    dlg.XamlRoot(app->root.XamlRoot());
    if (title && *title) dlg.Title(winrt::box_value(hs(title)));
    WUXC::StackPanel panel;
    if (message && *message) {
        WUXC::TextBlock tb;
        tb.Text(hs(message));
        tb.Margin(WUX::Thickness{ 0, 0, 0, 8 });
        panel.Children().Append(tb);
    }
    WUXC::TextBox box;
    box.PlaceholderText(hs(placeholder));
    box.Text(hs(initial));
    panel.Children().Append(box);
    dlg.Content(panel);
    dlg.PrimaryButtonText(hs((ok && *ok) ? ok : "OK"));
    dlg.CloseButtonText(hs((cancel && *cancel) ? cancel : "Cancel"));

    g_presents[req].dialog = dlg;
    dlg.ShowAsync().Completed(
        [req, box](WF::IAsyncOperation<WUXC::ContentDialogResult> const& a, WF::AsyncStatus st) {
            if (!g_presents.erase(req)) return;
            if (!g_present_cb) return;
            if (st == WF::AsyncStatus::Completed &&
                a.GetResults() == WUXC::ContentDialogResult::Primary) {
                std::string txt = u8(box.Text());
                g_present_cb(req, 2, 0, txt.c_str());
            } else {
                g_present_cb(req, 0, 0, "");
            }
        });
} catch (...) {
}

// Report a completed file-picker op: tag 3 (files) with the chosen path, else tag 0 (dismissed).
static void finish_file(uint64_t req, WSt::StorageFile const& file) {
    if (!g_presents.erase(req)) return;
    if (!g_present_cb) return;
    if (file) {
        std::string p = u8(file.Path());
        g_present_cb(req, 3, 0, p.c_str());
    } else {
        g_present_cb(req, 0, 0, "");
    }
}

extern "C" void day_xaml_present_file_open(uint64_t req, const char* title,
                                            const char* filters_joined, void* win) try {
    (void)title; // FileOpenPicker has no title property in the WinRT API
    auto app = reinterpret_cast<AppWindow*>(win);
    WStP::FileOpenPicker picker;
    picker.SuggestedStartLocation(WStP::PickerLocationId::DocumentsLibrary);
    // FileOpenPicker requires >=1 FileTypeFilter or PickSingleFileAsync throws. Flatten day's named
    // filters ("Name|ext1,ext2") to a bare ".ext" list; no filters → "*" (all files).
    bool any = false;
    for (auto const& f : split_units(filters_joined)) {
        size_t bar = f.find('|');
        std::string exts = (bar == std::string::npos) ? "" : f.substr(bar + 1);
        size_t p = 0;
        while (p <= exts.size()) {
            size_t c = exts.find(',', p);
            std::string e = exts.substr(p, c == std::string::npos ? std::string::npos : c - p);
            if (!e.empty()) { picker.FileTypeFilter().Append(hs(("." + e).c_str())); any = true; }
            if (c == std::string::npos) break;
            p = c + 1;
        }
    }
    if (!any) picker.FileTypeFilter().Append(L"*");
    if (app && app->host) picker.as<::IInitializeWithWindow>()->Initialize(app->host);

    auto op = picker.PickSingleFileAsync();
    g_presents[req].op = op;
    op.Completed([req](WF::IAsyncOperation<WSt::StorageFile> const& a, WF::AsyncStatus st) {
        finish_file(req, (st == WF::AsyncStatus::Completed) ? a.GetResults() : nullptr);
    });
} catch (...) {
}

extern "C" void day_xaml_present_file_save(uint64_t req, const char* title, const char* suggested,
                                            const char* filters_joined, void* win) try {
    (void)title; // FileSavePicker has no title property in the WinRT API
    auto app = reinterpret_cast<AppWindow*>(win);
    WStP::FileSavePicker picker;
    picker.SuggestedStartLocation(WStP::PickerLocationId::DocumentsLibrary);
    // FileSavePicker requires >=1 FileTypeChoice (name → [".ext", ...]); no filters → "Any" → ".".
    bool any = false;
    for (auto const& f : split_units(filters_joined)) {
        size_t bar = f.find('|');
        std::string name = (bar == std::string::npos) ? f : f.substr(0, bar);
        std::string exts = (bar == std::string::npos) ? "" : f.substr(bar + 1);
        auto vec = winrt::single_threaded_vector<winrt::hstring>();
        size_t p = 0;
        while (p <= exts.size()) {
            size_t c = exts.find(',', p);
            std::string e = exts.substr(p, c == std::string::npos ? std::string::npos : c - p);
            if (!e.empty()) vec.Append(hs(("." + e).c_str()));
            if (c == std::string::npos) break;
            p = c + 1;
        }
        if (vec.Size() > 0) { picker.FileTypeChoices().Insert(hs(name.c_str()), vec); any = true; }
    }
    if (!any) {
        picker.FileTypeChoices().Insert(
            L"Any", winrt::single_threaded_vector<winrt::hstring>({ L"." }));
    }
    if (suggested && *suggested) picker.SuggestedFileName(hs(suggested));
    if (app && app->host) picker.as<::IInitializeWithWindow>()->Initialize(app->host);

    auto op = picker.PickSaveFileAsync();
    g_presents[req].op = op;
    op.Completed([req](WF::IAsyncOperation<WSt::StorageFile> const& a, WF::AsyncStatus st) {
        finish_file(req, (st == WF::AsyncStatus::Completed) ? a.GetResults() : nullptr);
    });
} catch (...) {
}

extern "C" void day_xaml_dismiss_present(uint64_t req) try {
    auto it = g_presents.find(req);
    if (it == g_presents.end()) return;
    // Start the close; the modal's own Completed handler erases the entry and fires the cb (which
    // day-core ignores, having already resolved). Matches the Qt shim's dismiss.
    if (it->second.dialog) it->second.dialog.Hide();
    else if (it->second.op) it->second.op.Cancel();
} catch (...) {
}
