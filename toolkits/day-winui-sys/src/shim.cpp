// day-winui-sys — C++/WinRT XAML Islands shim (DESIGN.md §9).
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

#include <string>
#include <limits>
#include <cstdio>
#include <cstdlib>
#include <cmath>
#include <vector>

#include <winrt/base.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Storage.Streams.h>
#include <winrt/Windows.System.h>
#include <winrt/Windows.UI.h>
#include <winrt/Windows.UI.Text.h>
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>
#include <winrt/Windows.UI.Xaml.Controls.Primitives.h>
#include <winrt/Windows.UI.Xaml.Input.h>
#include <winrt/Windows.UI.Xaml.Media.h>
#include <winrt/Windows.UI.Xaml.Media.Imaging.h>
#include <winrt/Windows.UI.Xaml.Shapes.h>
#include <winrt/Windows.UI.Xaml.Hosting.h>
#include <winrt/Windows.UI.Xaml.Automation.h>
#include <winrt/Windows.UI.Xaml.Markup.h>
#include <winrt/Windows.UI.Xaml.Interop.h>

#include <windows.ui.xaml.hosting.desktopwindowxamlsource.h>
#include <DispatcherQueue.h>
#include <robuffer.h> // IBufferByteAccess — raw pixels out of a WinRT IBuffer

using namespace winrt;
namespace WF = winrt::Windows::Foundation;
namespace WUI = winrt::Windows::UI;
namespace WUX = winrt::Windows::UI::Xaml;
namespace WUXC = winrt::Windows::UI::Xaml::Controls;
namespace WUXCP = winrt::Windows::UI::Xaml::Controls::Primitives;
namespace WUXM = winrt::Windows::UI::Xaml::Media;
namespace WUXSh = winrt::Windows::UI::Xaml::Shapes;
namespace WUXH = winrt::Windows::UI::Xaml::Hosting;
namespace WUXIn = winrt::Windows::UI::Xaml::Input;
namespace WS = winrt::Windows::System;

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
    if (r > half) r = half;
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

struct DayApp : WUX::ApplicationT<DayApp, WUXMk::IXamlMetadataProvider> {
    WUXH::WindowsXamlManager manager{ nullptr };
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
};

static AppWindow* g_app = nullptr;

static const UINT WM_DAY_POST = WM_APP + 1;
struct PostMsg { void (*cb)(void*); void* data; };
// Day's window-resize report (single window, v1 — like g_app). UNVERIFIED on a live
// Windows host; mirrors the Qt shim's DayWindow::resizeEvent contract.
static void (*g_resize_cb)(int, int) = nullptr;
// Lifecycle (docs/lifecycle.md): codes match day_spec::Lifecycle order (2=DidBecomeActive,
// 3=WillResignActive, 7=WillTerminate).
static void (*g_lifecycle_cb)(int) = nullptr;

static LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    switch (msg) {
    case WM_SIZE:
        if (g_app && g_app->island) {
            RECT rc; GetClientRect(hwnd, &rc);
            SetWindowPos(g_app->island, nullptr, 0, 0, rc.right, rc.bottom, SWP_SHOWWINDOW);
            if (g_resize_cb) g_resize_cb(rc.right, rc.bottom);
        }
        return 0;
    case WM_ACTIVATE:
        // Window gained/lost foreground focus → active / resign-active.
        if (g_lifecycle_cb) g_lifecycle_cb(LOWORD(wp) == WA_INACTIVE ? 3 : 2);
        break; // let DefWindowProc handle focus normally
    case WM_CLOSE:
        // About to close (menu Quit posts WM_CLOSE too) → terminate, then destroy.
        if (g_lifecycle_cb) g_lifecycle_cb(7);
        break;
    case WM_DAY_POST: {
        auto p = reinterpret_cast<PostMsg*>(lp);
        if (p) { p->cb(p->data); delete p; }
        return 0;
    }
    case WM_DESTROY:
        PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

extern "C" void day_winui_set_lifecycle_cb(void (*cb)(int)) { g_lifecycle_cb = cb; }

extern "C" {

void* day_winui_window_new(const char* title, int w, int h) try {
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

    WNDCLASSW wc{};
    wc.lpfnWndProc = WndProc;
    wc.hInstance = GetModuleHandleW(nullptr);
    wc.lpszClassName = L"day_winui_host";
    wc.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    wc.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
    RegisterClassW(&wc);

    DWORD style = WS_OVERLAPPEDWINDOW; // resizable; WM_SIZE reflows the island + day tree
    RECT r{ 0, 0, w, h };
    AdjustWindowRect(&r, style, FALSE);
    HWND host = CreateWindowExW(0, L"day_winui_host", hs(title).c_str(), style,
                                CW_USEDEFAULT, CW_USEDEFAULT, r.right - r.left, r.bottom - r.top,
                                nullptr, nullptr, wc.hInstance, nullptr);

    WUXH::DesktopWindowXamlSource source;
    auto interop = source.as<::IDesktopWindowXamlSourceNative>();
    interop->AttachToWindow(host);
    HWND island = nullptr;
    interop->get_WindowHandle(&island);
    RECT rc; GetClientRect(host, &rc);
    SetWindowPos(island, nullptr, 0, 0, rc.right, rc.bottom, SWP_SHOWWINDOW);

    WUXC::Canvas root;
    source.Content(root);

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
    g_app = aw;
    return aw;
} catch (winrt::hresult_error const& e) {
    std::string msg = u8(e.message());
    std::fprintf(stderr, "day-winui: XAML Islands init failed: hr=0x%08X %s\n",
                 static_cast<unsigned>(e.code().value), msg.c_str());
    std::fflush(stderr);
    return nullptr;
} catch (...) {
    std::fprintf(stderr, "day-winui: XAML Islands init failed (unknown C++ exception)\n");
    std::fflush(stderr);
    return nullptr;
}

void* day_winui_window_root(void* win) {
    auto app = reinterpret_cast<AppWindow*>(win);
    return boxh(app->root);
}

void day_winui_window_on_resize(void* win, void (*cb)(int, int)) {
    (void)win; // single window (v1)
    g_resize_cb = cb;
}
void day_winui_window_show(void* win) {
    auto app = reinterpret_cast<AppWindow*>(win);
    ShowWindow(app->host, SW_SHOWNORMAL);
    UpdateWindow(app->host);
}

void day_winui_run(void* win) {
    auto app = reinterpret_cast<AppWindow*>(win);
    auto interop2 = app->source.as<::IDesktopWindowXamlSourceNative2>();
    MSG msg{};
    while (GetMessageW(&msg, nullptr, 0, 0)) {
        BOOL handled = FALSE;
        if (interop2) interop2->PreTranslateMessage(&msg, &handled);
        if (!handled) {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

void day_winui_post(void (*cb)(void*), void* data) {
    if (g_app && g_app->host) {
        PostMessageW(g_app->host, WM_DAY_POST, 0, reinterpret_cast<LPARAM>(new PostMsg{ cb, data }));
    }
}

// ---- containers ----

void* day_winui_container_new() { WUXC::Canvas c; return boxh(c); }
void* day_winui_scroll_new() { WUXC::Canvas c; return boxh(c); } // MVP: no scrolling
void* day_winui_canvas_new() { WUXC::Canvas c; return boxh(c); }

void day_winui_canvas_set_ops(void* h, const double* nums, int n, const char* texts_joined) {
    auto canvas = elem(h).try_as<WUXC::Canvas>();
    if (!canvas) return;
    canvas.Children().Clear();

    std::vector<std::string> texts;
    {
        std::string all = texts_joined ? texts_joined : "";
        size_t start = 0;
        while (start <= all.size()) {
            size_t nl = all.find('\n', start);
            texts.push_back(all.substr(start, nl == std::string::npos ? std::string::npos : nl - start));
            if (nl == std::string::npos) break;
            start = nl + 1;
        }
    }
    size_t ti = 0;
    std::vector<WUXM::Matrix> stack;
    WUXM::Matrix cur = mat_identity();
    const double DEG = 3.14159265358979323846 / 180.0;

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
        case 2: {
            WUXSh::Path p;
            if (k == 2) {
                p.Data(rounded_rect_geo(a, b, c, d, e));
            } else {
                WUXM::RectangleGeometry rg;
                rg.Rect(WF::Rect{ (float)a, (float)b, (float)c, (float)d });
                p.Data(rg);
            }
            if (k == 1) {
                p.Stroke(brush_bits(col));
                p.StrokeThickness(g);
            } else {
                p.Fill(brush_bits(col));
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
                p.Fill(brush_bits(col));
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
        }
    }
}

// Recycling-list host: a real ScrollViewer whose Content is a Canvas that holds the row cells
// (day positions each cell by absolute frame). `out_content` receives a handle to that Canvas so
// the Rust side can add/position cells; the list drives scrolling via the content's extent.
void* day_winui_list_new(void** out_content) {
    WUXC::ScrollViewer sv;
    sv.HorizontalScrollBarVisibility(WUXC::ScrollBarVisibility::Disabled);
    sv.VerticalScrollBarVisibility(WUXC::ScrollBarVisibility::Auto);
    WUXC::Canvas content;
    sv.Content(content);
    if (out_content) *out_content = boxh(content);
    return boxh(sv);
}
void day_winui_list_set_content_size(void* content, int w, int h) {
    if (auto fe = elem(content).try_as<FrameworkElement>()) {
        fe.Width(static_cast<double>(w));
        fe.Height(static_cast<double>(h));
    }
}

// Navigation sidebar item list (docs/navigation.md): a single-select ListView of route titles.
// The NAV host + pages are plain Canvases; day-core's NavLayout positions the sidebar/detail
// split, so no native split control is needed. Items are '\n'-joined (titles have no newlines).
void* day_winui_navlist_new(unsigned long long id, void (*cb)(unsigned long long, int)) {
    WUXC::ListView lv;
    lv.SelectionMode(WUXC::ListViewSelectionMode::Single);
    lv.SelectionChanged([id, cb](WF::IInspectable const& s, WUXC::SelectionChangedEventArgs const&) {
        cb(id, s.as<WUXC::ListView>().SelectedIndex());
    });
    return boxh(lv);
}
void day_winui_navlist_set_items(void* w, const char* items_joined) {
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
void day_winui_navlist_set_selected(void* w, int idx) {
    auto lv = elem(w).try_as<WUXC::ListView>();
    if (lv && lv.SelectedIndex() != idx) lv.SelectedIndex(idx);
}

void day_winui_container_set_bg(void* h, unsigned int argb) {
    if (auto p = elem(h).try_as<WUXC::Panel>())
        p.Background(WUXM::SolidColorBrush(color_argb(argb)));
}

// ---- label ----

void* day_winui_label_new(const char* text) {
    WUXC::TextBlock t;
    t.Text(hs(text));
    t.TextWrapping(WUX::TextWrapping::Wrap);
    return boxh(t);
}
void day_winui_label_set_text(void* h, const char* t) {
    if (auto tb = elem(h).try_as<WUXC::TextBlock>()) tb.Text(hs(t));
}
void day_winui_label_set_font(void* h, double pt, int weight, int italic) {
    if (auto tb = elem(h).try_as<WUXC::TextBlock>()) {
        // FontSize scales with the OS text-scale-factor (accessibility "Text size"); WinUI applies it.
        tb.FontSize(pt);
        // `weight` is a numeric font weight (100–900); build a FontWeight directly.
        winrt::Windows::UI::Text::FontWeight w;
        w.Weight = static_cast<uint16_t>(weight > 0 ? weight : 400);
        tb.FontWeight(w);
        tb.FontStyle(italic ? WUI::Text::FontStyle::Italic : WUI::Text::FontStyle::Normal);
    }
}

// ---- button ----

void* day_winui_button_new(const char* title, unsigned long long id, void (*cb)(unsigned long long)) {
    WUXC::Button b;
    b.Content(winrt::box_value(hs(title)));
    b.Click([id, cb](WF::IInspectable const&, WUX::RoutedEventArgs const&) { cb(id); });
    return boxh(b);
}
void day_winui_button_set_title(void* h, const char* t) {
    if (auto b = elem(h).try_as<WUXC::Button>()) b.Content(winrt::box_value(hs(t)));
}

// ---- toggle (ToggleSwitch) ----

void* day_winui_toggle_new(int on, unsigned long long id, void (*cb)(unsigned long long, int)) {
    WUXC::ToggleSwitch t;
    t.IsOn(on != 0);
    t.OnContent(winrt::box_value(winrt::hstring{}));
    t.OffContent(winrt::box_value(winrt::hstring{}));
    t.Toggled([id, cb](WF::IInspectable const& s, WUX::RoutedEventArgs const&) {
        cb(id, s.as<WUXC::ToggleSwitch>().IsOn() ? 1 : 0);
    });
    return boxh(t);
}
void day_winui_toggle_set(void* h, int on) {
    if (auto t = elem(h).try_as<WUXC::ToggleSwitch>())
        if (t.IsOn() != (on != 0)) t.IsOn(on != 0);
}

// ---- slider (integer positions 0..1000; Rust maps the f64 range) ----

void* day_winui_slider_new(int value, unsigned long long id, void (*cb)(unsigned long long, int)) {
    WUXC::Slider s;
    s.Minimum(0);
    s.Maximum(1000);
    s.StepFrequency(1);
    s.Value(value);
    s.ValueChanged([id, cb](WF::IInspectable const& sender,
                            WUXCP::RangeBaseValueChangedEventArgs const&) {
        cb(id, static_cast<int>(sender.as<WUXC::Slider>().Value()));
    });
    return boxh(s);
}
void day_winui_slider_set(void* h, int value) {
    if (auto s = elem(h).try_as<WUXC::Slider>())
        if (static_cast<int>(s.Value()) != value) s.Value(value);
}

// ---- progress (determinate ProgressBar 0..1000, or indeterminate ProgressRing) ----

void* day_winui_progress_new(int determinate, int value) {
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
void day_winui_progress_set(void* h, int value) {
    if (auto b = elem(h).try_as<WUXC::ProgressBar>())
        if (static_cast<int>(b.Value()) != value) b.Value(value);
}

// ---- tabs (docs/tabs.md): a Pivot owns its page content ----

void* day_winui_tabs_new(unsigned long long id, void (*cb)(unsigned long long, int)) {
    WUXC::Pivot p;
    p.SelectionChanged([id, cb](winrt::Windows::Foundation::IInspectable const& s,
                                WUXC::SelectionChangedEventArgs const&) {
        cb(id, s.as<WUXC::Pivot>().SelectedIndex());
    });
    return boxh(p);
}
void day_winui_tabs_add_page(void* tabs, void* page, const char* title, int index) {
    auto p = elem(tabs).as<WUXC::Pivot>();
    WUXC::PivotItem item;
    item.Header(winrt::box_value(hs(title)));
    item.Content(elem(page));
    auto items = p.Items();
    if (index < 0 || static_cast<uint32_t>(index) >= items.Size()) items.Append(item);
    else items.InsertAt(static_cast<uint32_t>(index), item);
}
void day_winui_tabs_set_current(void* tabs, int index) {
    elem(tabs).as<WUXC::Pivot>().SelectedIndex(index);
}
void day_winui_tabs_content_size(void* tabs, double* w, double* h) {
    auto p = elem(tabs).as<WUXC::Pivot>();
    *w = p.ActualWidth();
    double ah = p.ActualHeight();
    *h = ah > 48 ? ah - 48 : ah; // subtract the header strip
}

// ---- textbox ----

void* day_winui_textbox_new(const char* text, const char* placeholder, unsigned long long id,
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
void day_winui_textbox_set_text(void* h, const char* t) {
    if (auto tb = elem(h).try_as<WUXC::TextBox>()) {
        auto nt = hs(t);
        if (tb.Text() != nt) tb.Text(nt);
    }
}
void day_winui_textbox_set_placeholder(void* h, const char* t) {
    if (auto tb = elem(h).try_as<WUXC::TextBox>()) tb.PlaceholderText(hs(t));
}

// ---- divider / image ----

void* day_winui_divider_new() {
    WUXC::Border b;
    b.Height(1);
    b.Background(WUXM::SolidColorBrush(color_argb(0xff'c8c8c8u)));
    return boxh(b);
}

void* day_winui_image_new(const char* uri) {
    WUXC::Image img;
    if (uri && *uri) {
        try {
            WUXM::Imaging::BitmapImage bmp{ WF::Uri{ hs(uri) } };
            img.Source(bmp);
        } catch (...) {}
    }
    return boxh(img);
}

// ---- combo box ----

static void combo_fill(WUXC::ComboBox const& cb, const char* items_joined, int selected) {
    cb.Items().Clear();
    std::string all = items_joined ? items_joined : "";
    size_t start = 0;
    while (start <= all.size()) {
        size_t nl = all.find('\n', start);
        std::string item = all.substr(start, nl == std::string::npos ? std::string::npos : nl - start);
        if (!(item.empty() && all.empty())) {
            WUXC::ComboBoxItem it;
            it.Content(winrt::box_value(hs(item.c_str())));
            cb.Items().Append(it);
        }
        if (nl == std::string::npos) break;
        start = nl + 1;
    }
    cb.SelectedIndex(selected);
}

void* day_winui_combo_new(const char* items_joined, int selected, unsigned long long id,
                          void (*cb)(unsigned long long, int)) {
    WUXC::ComboBox box;
    combo_fill(box, items_joined, selected);
    box.SelectionChanged([id, cb](WF::IInspectable const& s, WUXC::SelectionChangedEventArgs const&) {
        cb(id, s.as<WUXC::ComboBox>().SelectedIndex());
    });
    return boxh(box);
}
void day_winui_combo_set_items(void* h, const char* items_joined) {
    if (auto box = elem(h).try_as<WUXC::ComboBox>()) combo_fill(box, items_joined, box.SelectedIndex());
}
void day_winui_combo_set_selected(void* h, int idx) {
    if (auto box = elem(h).try_as<WUXC::ComboBox>())
        if (box.SelectedIndex() != idx) box.SelectedIndex(idx);
}

// ---- tree / geometry / props ----

void day_winui_add_child(void* parent, void* child) {
    guard([&] {
        if (auto p = elem(parent).try_as<WUXC::Panel>()) p.Children().Append(elem(child));
    });
}
void day_winui_remove_child(void* parent, void* child) {
    guard([&] {
        if (auto p = elem(parent).try_as<WUXC::Panel>()) {
            uint32_t idx = 0;
            if (p.Children().IndexOf(elem(child), idx)) p.Children().RemoveAt(idx);
        }
    });
}
void day_winui_delete(void* h) { delete reinterpret_cast<Node*>(h); }

// External-piece handle seam (docs/picker.md): box any WinRT UI element into a day handle, and
// borrow it back — so an external piece can carry its OWN native WinUI shim (like the Qt shims)
// without duplicating the private `Node` wrapper. The element crosses as a WinRT ABI pointer
// (`get_abi`, a stable COM interface pointer), which day-winui-sys owns the boxing for.
void* day_winui_box(void* iinspectable_abi) {
    WF::IInspectable insp{ nullptr };
    winrt::copy_from_abi(insp, iinspectable_abi); // AddRefs the incoming element
    return boxh(insp.as<UIElement>());
}
void* day_winui_unbox(void* handle) {
    return winrt::get_abi(elem(handle)); // borrowed IUIElement* (piece copy_from_abi's to own a ref)
}

void day_winui_set_geometry(void* h, int x, int y, int width, int height) {
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

void day_winui_measure(void* h, double aw, double ah, double* ow, double* oh) {
    *ow = 0; // sane defaults if a degraded element throws mid-measure (guard swallows it)
    *oh = 0;
    guard([&] {
        float fw = aw < 0 ? std::numeric_limits<float>::infinity() : static_cast<float>(aw);
        float fh = ah < 0 ? std::numeric_limits<float>::infinity() : static_cast<float>(ah);
        auto& e = elem(h);
        e.Measure(WF::Size{ fw, fh });
        auto d = e.DesiredSize();
        if (d.Width == 0 && d.Height == 0) {
            // day measures during its synchronous initial layout, before the island's first async
            // layout pass has applied control templates (so templated controls report 0). Force a
            // synchronous layout to expand templates, then re-measure. Runs at most once (the first
            // zero-measure lays out the whole tree; later measures are already non-zero).
            if (auto fe = e.try_as<FrameworkElement>()) fe.UpdateLayout();
            e.Measure(WF::Size{ fw, fh });
            d = e.DesiredSize();
        }
        *ow = d.Width;
        *oh = d.Height;
    });
}

void day_winui_set_enabled(void* h, int enabled) {
    guard([&] {
        if (auto c = elem(h).try_as<WUXC::Control>()) c.IsEnabled(enabled != 0);
    });
}

void day_winui_set_visible(void* h, int visible) {
    guard([&] {
        elem(h).Visibility(visible ? WUX::Visibility::Visible : WUX::Visibility::Collapsed);
    });
}

// The element's laid-out size (points). Used by the list to size cells to its viewport width.
void day_winui_widget_size(void* h, double* ow, double* oh) {
    *ow = 0;
    *oh = 0;
    guard([&] {
        if (auto fe = elem(h).try_as<FrameworkElement>()) {
            *ow = fe.ActualWidth();
            *oh = fe.ActualHeight();
        }
    });
}

void day_winui_set_name(void* h, const char* name) {
    guard([&] { WUX::Automation::AutomationProperties::SetAutomationId(elem(h), hs(name)); });
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
int day_winui_snapshot_png(void* win, const char* path) try {
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

static void add_accel(WUXC::MenuFlyoutItem const& item, int key, int mods) {
    if (key == 0) return;
    WUXIn::KeyboardAccelerator ka;
    ka.Key(static_cast<WS::VirtualKey>(key));
    auto m = WS::VirtualKeyModifiers::None;
    if (mods & 1) m |= WS::VirtualKeyModifiers::Control;
    if (mods & 2) m |= WS::VirtualKeyModifiers::Shift;
    if (mods & 4) m |= WS::VirtualKeyModifiers::Menu;
    ka.Modifiers(m);
    item.KeyboardAccelerators().Append(ka);
}

// Append the flat menu spec into a MenuFlyoutItemBase collection (a MenuFlyout / MenuFlyoutSubItem /
// MenuBarItem Items()), tracking submenu depth with a stack.
static void build_menu_items(WF::Collections::IVector<WUXC::MenuFlyoutItemBase> root,
                             const std::string& spec) {
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
            item.IsEnabled(!(f.size() > 5 && f[5] == "0"));
            int key = f.size() > 3 ? std::atoi(f[3].c_str()) : 0;
            int mods = f.size() > 4 ? std::atoi(f[4].c_str()) : 0;
            add_accel(item, key, mods);
            if (kind == "A") {
                unsigned long long aid = f.size() > 1 ? std::strtoull(f[1].c_str(), nullptr, 10) : 0;
                item.Click([aid](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
                    if (g_menu_cb) g_menu_cb(aid);
                });
            } else {
                int role = f.size() > 2 ? std::atoi(f[2].c_str()) : -1;
                if (role == 8) { // Quit
                    item.Click([](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
                        if (g_app && g_app->host) PostMessageW(g_app->host, WM_CLOSE, 0, 0);
                    });
                }
            }
            cur.Append(item);
        }
    }
}

extern "C" void day_winui_set_menu_cb(void (*cb)(unsigned long long)) { g_menu_cb = cb; }

extern "C" void day_winui_set_context_menu(void* h, const char* spec) try {
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

extern "C" void day_winui_set_app_menu(void* win, const char* spec) try {
    auto app = reinterpret_cast<AppWindow*>(win);
    if (!app || !app->root) return;
    // Remove any prior MenuBar we docked (named "day_menubar"). `Children()` returns the
    // UIElementCollection by value (a projection over the real collection), so bind it by value —
    // a non-const reference can't bind to that rvalue (C2440) — mutations still hit the real one.
    auto kids = app->root.Children();
    for (uint32_t i = 0; i < kids.Size(); ++i) {
        if (auto fe = kids.GetAt(i).try_as<FrameworkElement>()) {
            if (fe.Name() == L"day_menubar") { kids.RemoveAt(i); break; }
        }
    }
    if (!spec || !*spec) return;
    WUXC::MenuBar bar;
    bar.Name(L"day_menubar");
    // Top-level "S" groups become MenuBarItems; a bare item wraps in an unnamed MenuBarItem.
    auto lines = split_lines(spec);
    size_t i = 0;
    while (i < lines.size()) {
        if (lines[i].empty()) { ++i; continue; }
        auto f = split_tabs(lines[i]);
        std::string kind = f.size() > 0 ? f[0] : "";
        if (kind == "S") {
            std::string label = f.size() > 6 ? f[6] : "";
            WUXC::MenuBarItem mbi;
            mbi.Title(hs(label.c_str()));
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
            build_menu_items(mbi.Items(), inner);
            bar.Items().Append(mbi);
        } else {
            WUXC::MenuBarItem mbi;
            mbi.Title(hs(""));
            build_menu_items(mbi.Items(), lines[i] + "\n");
            bar.Items().Append(mbi);
            ++i;
        }
    }
    WUXC::Canvas::SetLeft(bar, 0);
    WUXC::Canvas::SetTop(bar, 0);
    app->root.Children().Append(bar);
} catch (...) {
}
