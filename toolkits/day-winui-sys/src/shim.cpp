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

#include <string>
#include <limits>
#include <cstdio>

#include <winrt/base.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.System.h>
#include <winrt/Windows.UI.h>
#include <winrt/Windows.UI.Text.h>
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>
#include <winrt/Windows.UI.Xaml.Controls.Primitives.h>
#include <winrt/Windows.UI.Xaml.Media.h>
#include <winrt/Windows.UI.Xaml.Media.Imaging.h>
#include <winrt/Windows.UI.Xaml.Hosting.h>
#include <winrt/Windows.UI.Xaml.Automation.h>
#include <winrt/Windows.UI.Xaml.Markup.h>
#include <winrt/Windows.UI.Xaml.Interop.h>

#include <windows.ui.xaml.hosting.desktopwindowxamlsource.h>
#include <DispatcherQueue.h>

using namespace winrt;
namespace WF = winrt::Windows::Foundation;
namespace WUI = winrt::Windows::UI;
namespace WUX = winrt::Windows::UI::Xaml;
namespace WUXC = winrt::Windows::UI::Xaml::Controls;
namespace WUXCP = winrt::Windows::UI::Xaml::Controls::Primitives;
namespace WUXM = winrt::Windows::UI::Xaml::Media;
namespace WUXH = winrt::Windows::UI::Xaml::Hosting;

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

static LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    switch (msg) {
    case WM_SIZE:
        if (g_app && g_app->island) {
            RECT rc; GetClientRect(hwnd, &rc);
            SetWindowPos(g_app->island, nullptr, 0, 0, rc.right, rc.bottom, SWP_SHOWWINDOW);
        }
        return 0;
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

    DWORD style = WS_OVERLAPPEDWINDOW & ~(WS_THICKFRAME | WS_MAXIMIZEBOX); // fixed size (MVP)
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
void day_winui_label_set_font(void* h, double pt, int bold) {
    if (auto tb = elem(h).try_as<WUXC::TextBlock>()) {
        tb.FontSize(pt);
        tb.FontWeight(bold ? WUI::Text::FontWeights::SemiBold() : WUI::Text::FontWeights::Normal());
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
    if (auto p = elem(parent).try_as<WUXC::Panel>()) p.Children().Append(elem(child));
}
void day_winui_remove_child(void* parent, void* child) {
    if (auto p = elem(parent).try_as<WUXC::Panel>()) {
        uint32_t idx = 0;
        if (p.Children().IndexOf(elem(child), idx)) p.Children().RemoveAt(idx);
    }
}
void day_winui_delete(void* h) { delete reinterpret_cast<Node*>(h); }

void day_winui_set_geometry(void* h, int x, int y, int width, int height) {
    auto& e = elem(h);
    WUXC::Canvas::SetLeft(e, static_cast<double>(x));
    WUXC::Canvas::SetTop(e, static_cast<double>(y));
    if (auto fe = e.try_as<FrameworkElement>()) {
        fe.Width(static_cast<double>(width));
        fe.Height(static_cast<double>(height));
    }
}

void day_winui_measure(void* h, double aw, double ah, double* ow, double* oh) {
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
}

void day_winui_set_enabled(void* h, int enabled) {
    if (auto c = elem(h).try_as<WUXC::Control>()) c.IsEnabled(enabled != 0);
}

void day_winui_set_name(void* h, const char* name) {
    WUX::Automation::AutomationProperties::SetAutomationId(elem(h), hs(name));
}

} // extern "C"
