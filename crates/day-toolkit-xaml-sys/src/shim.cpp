// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-toolkit-xaml-sys — C++/WinRT XAML Islands shim (DESIGN.md §9).
//
// Hosts the Windows.UI.Xaml control set inside a Win32 host window via
// DesktopWindowXamlSource, and exposes a flat C ABI mirroring day-toolkit-qt-sys. day owns layout:
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
#ifndef DWMSBT_TRANSIENTWINDOW
#define DWMSBT_TRANSIENTWINDOW 3 // Acrylic: WindowMaterial::Transient (docs/window-chrome.md)
#endif
#ifndef DWMSBT_TABBEDWINDOW
#define DWMSBT_TABBEDWINDOW 4 // Mica Alt: WindowMaterial::Sidebar
#endif
// Content protection (docs/windows.md "Window properties"). Absent from SDKs before 19041 (Windows
// 10 2004); on an older Windows the call fails and the shim falls back to WDA_MONITOR.
#ifndef WDA_EXCLUDEFROMCAPTURE
#define WDA_EXCLUDEFROMCAPTURE 0x00000011
#endif

#include <string>
#include <chrono> // monotonic deadlines for Win32 timers, which may signal early
#include <limits>
#include <algorithm> // std::sort — radial-gradient stop ordering
#include <array>      // the saved clip rect on the save/restore stack
#include <cwctype>   // towupper / iswalpha — menu access keys
#include <memory>    // shared_ptr — the segmented picker's group state, the textarea's flag
#include <cwchar>    // wcscmp — WM_SETTINGCHANGE area name
#include <charconv>
#include <cstdio>
#include <cstdlib>
#include <cstring> // _strdup (the font-list string, docs/fonts.md)
#include <cmath>
#include <vector>
#include <map>
#include <mutex>  // day_xaml_post's queue for posts that arrive before the window exists
#include <set>
#include <functional> // OEM menu-accelerator dispatch (see add_accel)
#include <fstream> // read local image bytes for BitmapImage.SetSource (file:// URIs don't load)
#include <shobjidl_core.h> // IInitializeWithWindow — parents WinRT file pickers to the host HWND
#include <propsys.h>        // IPropertyStore: a jump-list task's title (docs/deep-links.md)
#include <appmodel.h>       // GetCurrentPackageFullName: whether the process has an app identity

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
#include <winrt/Windows.UI.ViewManagement.h> // UISettings: the system accent color, reduce motion
#include <winrt/Windows.UI.Text.h>
#include <dwrite.h> // IDWriteFontCollection: the platform font list (docs/fonts.md)
#include <d2d1.h> // vector stroke expansion and geometry unions for PathIcon
#include <winrt/Windows.UI.Xaml.Interop.h> // TypeName: WinUI 3 still uses the system one
#include <winrt/Windows.Storage.h>         // StorageFile (file-picker results)
#include <winrt/Windows.Storage.Pickers.h> // FileOpenPicker / FileSavePicker
#include <robuffer.h> // IBufferByteAccess — raw pixels out of a WinRT IBuffer
#include <DispatcherQueue.h> // the system DispatcherQueue both stacks keep on the UI thread

// ONE shim, two XAML stacks. `windows-xaml` builds it against system XAML (Windows.UI.Xaml,
// hosted by XAML Islands); `windows-winui` (DAY_WINUI) against WinUI 3 (Microsoft.UI.Xaml, the
// Windows App SDK), hosted by that stack's own DesktopWindowXamlSource. The control set and
// nearly every API are the same under both namespaces, so the code below is written once
// against the aliases defined here; what differs (hosting, dispatch, bootstrapping, capture)
// is behind `#ifdef DAY_WINUI` where it happens.
#ifdef DAY_WINUI
#include <winrt/Microsoft.Graphics.Canvas.h>
#include <winrt/Microsoft.Graphics.Canvas.Brushes.h>
#include <winrt/Microsoft.Graphics.Canvas.Geometry.h>
#include <winrt/Microsoft.Graphics.Canvas.Text.h>
#include <winrt/Microsoft.Graphics.Canvas.UI.Xaml.h>
#include <winrt/Windows.Foundation.Numerics.h>
#include <wincodec.h>
#include <roapi.h>
#include <MddBootstrap.h>
#include <winrt/Microsoft.UI.h>
#include <winrt/Microsoft.UI.Interop.h>
#include <winrt/Microsoft.UI.Content.h>
#include <winrt/Microsoft.UI.Dispatching.h>
#include <winrt/Microsoft.UI.Input.h>
#include <winrt/Microsoft.UI.Text.h>
#include <winrt/Microsoft.UI.Composition.h>
#include <winrt/Microsoft.UI.Xaml.h>
#include <winrt/Microsoft.UI.Xaml.Controls.h>
#include <winrt/Microsoft.UI.Xaml.Documents.h>
#include <winrt/Microsoft.UI.Xaml.Controls.Primitives.h>
#include <winrt/Microsoft.UI.Xaml.Input.h>
#include <winrt/Microsoft.UI.Xaml.Media.h>
#include <winrt/Microsoft.UI.Xaml.Media.Animation.h>
#include <winrt/Microsoft.UI.Xaml.Media.Imaging.h>
#include <winrt/Microsoft.UI.Xaml.Shapes.h>
#include <winrt/Microsoft.UI.Xaml.Hosting.h>
#include <winrt/Microsoft.UI.Xaml.Automation.h>
// AutomationPeer / FrameworkElementAutomationPeer: screen-reader announcements and the heading,
// view and notification enums (docs/accessibility.md).
#include <winrt/Microsoft.UI.Xaml.Automation.Peers.h>
#include <winrt/Microsoft.UI.Xaml.Markup.h>
#include <winrt/Microsoft.UI.Xaml.XamlTypeInfo.h>
// Window capture (snapshot_wgc_png): Windows.Graphics.Capture reads the window back from DWM.
#include <d3d11.h>
#include <Windows.Graphics.Capture.Interop.h>
#include <windows.graphics.directx.direct3d11.interop.h>
#include <winrt/Windows.Graphics.h>
#include <winrt/Windows.Graphics.Capture.h>
#include <winrt/Windows.Graphics.DirectX.h>
#include <winrt/Windows.Graphics.DirectX.Direct3D11.h>
#define DAY_XAML_NS winrt::Microsoft::UI::Xaml
namespace WUComp = winrt::Microsoft::UI::Composition;
namespace WUIIn = winrt::Microsoft::UI::Input;
// FontWeights (the named weights); the FontWeight struct itself stays Windows.UI.Text.
namespace WUTextNames = winrt::Microsoft::UI::Text;
// A pointer's device type is Microsoft.UI.Input's; DynamicOverflowOrder is on ICommandBarElement itself.
using DayPointerDeviceType = winrt::Microsoft::UI::Input::PointerDeviceType;
using DayCommandBarElement = winrt::Microsoft::UI::Xaml::Controls::ICommandBarElement;
#else
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
#include <winrt/Windows.UI.Xaml.Automation.Peers.h> // see the WinUI branch above
#include <winrt/Windows.UI.Xaml.Markup.h>
#include <windows.ui.xaml.hosting.desktopwindowxamlsource.h>
#define DAY_XAML_NS winrt::Windows::UI::Xaml
namespace WUComp = winrt::Windows::UI::Composition;
namespace WUIIn = winrt::Windows::UI::Input;
namespace WUTextNames = winrt::Windows::UI::Text;
using DayPointerDeviceType = winrt::Windows::Devices::Input::PointerDeviceType;
using DayCommandBarElement = winrt::Windows::UI::Xaml::Controls::ICommandBarElement2;
#endif

using namespace winrt;
namespace WF = winrt::Windows::Foundation;
namespace WFC = winrt::Windows::Foundation::Collections; // IVectorView (multi-file picks)
namespace WUI = winrt::Windows::UI;
namespace WUX = DAY_XAML_NS;
namespace WUXC = DAY_XAML_NS::Controls;
namespace WUXCP = DAY_XAML_NS::Controls::Primitives;
namespace WUXD = DAY_XAML_NS::Documents; // Typography (numeral alignment)
namespace WUXM = DAY_XAML_NS::Media;
namespace WUXMA = DAY_XAML_NS::Media::Animation;
namespace WUXSh = DAY_XAML_NS::Shapes;
namespace WUXH = DAY_XAML_NS::Hosting;
namespace WUXIn = DAY_XAML_NS::Input;
namespace WUXA = DAY_XAML_NS::Automation;         // AutomationProperties (docs/accessibility.md)
namespace WUXAP = DAY_XAML_NS::Automation::Peers; // automation peers and their enums
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

// A multi-line TextBox reports its line separator as a CR, which is XAML's own spelling and
// nobody else's: every other toolkit hands Day an LF, and the bound signal is a plain Rust string
// an app splits on '\n'. Rewritten one code unit for one, so an offset into it still lands on the
// same character.
static std::string lf(std::string s) {
    for (char& c : s) {
        if (c == '\r' || c == '\v') c = '\n';
    }
    return s;
}

// cppwinrt projected types delete `operator new`, so a bare `new UIElement(e)` is illegal.
// A plain wrapper struct owns the WinRT reference on the heap; delete releases it.
struct Node {
    UIElement e;
    // The app gave this element an accessibility label (`day_xaml_set_a11y`): a button's title
    // change must then leave its automation name alone (`day_xaml_button_set_content`).
    bool app_named = false;
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

// Pump the message loop until `done()` answers true, or the bound elapses (best-effort: a caller
// that times out carries on with whatever state it has, because no capture or scroll is worth
// hanging the app over). Shares `pump_until_complete`'s WM_APP+1 exclusion, and for the same
// reason: these pumps run inside a day-core `with_tree` borrow that day's cross-thread post
// would re-enter. (A template, so it lives at file scope like its neighbor.)
template <typename F>
static void pump_until(F done, ULONGLONG timeout_ms) {
    MSG msg{};
    ULONGLONG start = GetTickCount64();
    const UINT day_post = WM_APP + 1;
    while (!done() && GetTickCount64() - start < timeout_ms) {
        if (PeekMessageW(&msg, nullptr, 0, day_post - 1, PM_REMOVE) ||
            PeekMessageW(&msg, nullptr, day_post + 1, 0xFFFFFFFF, PM_REMOVE)) {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        } else {
            Sleep(1);
        }
    }
}

// Native control brushes. Canvas drawing lives in canvas-winui.inc.
static WUXM::SolidColorBrush brush_bits(unsigned col) {
    WUI::Color c;
    c.A = static_cast<uint8_t>((col >> 24) & 0xff);
    c.R = static_cast<uint8_t>((col >> 16) & 0xff);
    c.G = static_cast<uint8_t>((col >> 8) & 0xff);
    c.B = static_cast<uint8_t>(col & 0xff);
    return WUXM::SolidColorBrush(c);
}
// DirectWrite font enumeration shared by native controls and canvas.
static std::string dw_utf8(const std::wstring& w) {
    if (w.empty()) return std::string();
    int n = WideCharToMultiByte(CP_UTF8, 0, w.c_str(), (int)w.size(), nullptr, 0, nullptr, nullptr);
    if (n <= 0) return std::string();
    std::string out((size_t)n, '\0');
    WideCharToMultiByte(CP_UTF8, 0, w.c_str(), (int)w.size(), &out[0], n, nullptr, nullptr);
    for (char& c : out) if (c == '\x1f' || c == '\x1e') c = ' ';
    return out;
}

// The en-us entry of a localized-strings set (else its first), as UTF-8.
static std::string dw_name(IDWriteLocalizedStrings* names) {
    if (!names || names->GetCount() == 0) return std::string();
    UINT32 idx = 0;
    BOOL exists = FALSE;
    if (FAILED(names->FindLocaleName(L"en-us", &idx, &exists)) || !exists) idx = 0;
    UINT32 len = 0;
    if (FAILED(names->GetStringLength(idx, &len))) return std::string();
    std::wstring buf((size_t)len + 1, L'\0');
    if (FAILED(names->GetString(idx, &buf[0], len + 1))) return std::string();
    buf.resize(len);
    return dw_utf8(buf);
}

extern "C" {

// The system font collection, family by family and font by font, in Day's list format
// (U+001E between families, U+001F between fields: family, then (face, CSS weight, italic
// 0/1) per face). Simulated (synthesized-bold/oblique) entries are skipped: they are what
// XAML does for any family anyway. Heap-allocated: release with day_xaml_string_free.
char* day_xaml_font_families(void) try {
    winrt::com_ptr<IDWriteFactory> factory;
    if (FAILED(DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED, __uuidof(IDWriteFactory),
                                   reinterpret_cast<IUnknown**>(factory.put())))) {
        return _strdup("");
    }
    winrt::com_ptr<IDWriteFontCollection> coll;
    if (FAILED(factory->GetSystemFontCollection(coll.put(), FALSE))) return _strdup("");
    std::string out;
    UINT32 n = coll->GetFontFamilyCount();
    for (UINT32 i = 0; i < n; ++i) {
        winrt::com_ptr<IDWriteFontFamily> fam;
        if (FAILED(coll->GetFontFamily(i, fam.put()))) continue;
        winrt::com_ptr<IDWriteLocalizedStrings> names;
        if (FAILED(fam->GetFamilyNames(names.put()))) continue;
        std::string family = dw_name(names.get());
        if (family.empty()) continue;
        if (!out.empty()) out.push_back('\x1e');
        out += family;
        UINT32 fc = fam->GetFontCount();
        for (UINT32 j = 0; j < fc; ++j) {
            winrt::com_ptr<IDWriteFont> font;
            if (FAILED(fam->GetFont(j, font.put()))) continue;
            if (font->GetSimulations() != DWRITE_FONT_SIMULATIONS_NONE) continue;
            winrt::com_ptr<IDWriteLocalizedStrings> faceNames;
            std::string face = SUCCEEDED(font->GetFaceNames(faceNames.put()))
                                   ? dw_name(faceNames.get())
                                   : std::string();
            int weight = (int)font->GetWeight();
            weight = ((weight + 50) / 100) * 100;
            if (weight < 100) weight = 100;
            if (weight > 900) weight = 900;
            out.push_back('\x1f');
            out += face;
            out.push_back('\x1f');
            out += std::to_string(weight);
            out.push_back('\x1f');
            out.push_back(font->GetStyle() != DWRITE_FONT_STYLE_NORMAL ? '1' : '0');
        }
    }
    return _strdup(out.c_str());
} catch (...) {
    return _strdup("");
}

// Release a string returned by day_xaml_font_families. Safe to call with null.
void day_xaml_string_free(char* p) { free(p); }

#ifndef DAY_WINUI
int day_xaml_measure_text(const char*, double, int, int, const char*, double*) { return 1; }
#endif

} // extern "C"



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

namespace WUXMk = DAY_XAML_NS::Markup;
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
/// The APP's own appearance override (docs/settings.md): 0 follow the system, 1 light, 2 dark.
/// Distinct from `g_forced_theme`, which is the DAY_THEME env force a screenshot run pins — that
/// one still wins, so a themed CI capture cannot be knocked off its scheme by the app's own picker.
static int g_app_override = 0;

/// True when the scheme in force is NOT the system's own — a DAY_THEME force or the app's
/// Appearance pick. The windows below consult it wherever they would otherwise take a color from
/// something that only ever speaks the SYSTEM scheme: the app's theme-brush resources, and Mica.
static bool theme_overridden() { return g_forced_theme != 0 || g_app_override != 0; }

struct DayApp : WUX::ApplicationT<DayApp, WUXMk::IXamlMetadataProvider> {
    WUXH::WindowsXamlManager manager{ nullptr };
    // DAY_THEME is forced PER-ELEMENT (ElementTheme on the root Canvas — see day_xaml_window_new),
    // NOT via Application::RequestedTheme: the app-level setter is unsupported under XAML Islands and
    // aborts island init (a fail-fast, not a catchable throw), which makes the app exit before the
    // dayscript engine binds its socket — the walkthrough runner then can't connect (§14).
#ifdef DAY_WINUI
    // WinUI 3 ships its control templates as a resource dictionary rather than in the OS, so the
    // Application has to merge XamlControlsResources, and answer for the controls' types so that
    // dictionary can resolve them (there is no XAML compiler here to generate a provider).
    WUX::XamlTypeInfo::XamlControlsXamlMetaDataProvider controls;
    DayApp() {
        manager = WUXH::WindowsXamlManager::InitializeForCurrentThread();
        Resources().MergedDictionaries().Append(WUXC::XamlControlsResources());
    }
    WUXMk::IXamlType GetXamlType(WUXI::TypeName const& t) { return controls.GetXamlType(t); }
    WUXMk::IXamlType GetXamlType(winrt::hstring const& n) { return controls.GetXamlType(n); }
    winrt::com_array<WUXMk::XmlnsDefinition> GetXmlnsDefinitions() {
        return controls.GetXmlnsDefinitions();
    }
#else
    DayApp() { manager = WUXH::WindowsXamlManager::InitializeForCurrentThread(); }

    // IXamlMetadataProvider — no custom XAML types to describe.
    WUXMk::IXamlType GetXamlType(WUXI::TypeName const&) { return nullptr; }
    WUXMk::IXamlType GetXamlType(winrt::hstring const&) { return nullptr; }
    winrt::com_array<WUXMk::XmlnsDefinition> GetXmlnsDefinitions() { return {}; }
#endif
};

#ifdef DAY_WINUI
// The Windows App SDK runtime, reached without linking any of its DLLs (docs/winui.md). An
// unpackaged app has to put the framework package on its own package graph
// before any Microsoft.UI type can activate, which is what the bootstrapper does; it is loaded
// from next to the exe (a packaged or staged app) and otherwise from the SDK cache the build
// used, whose path is compiled in (a development build).
static bool winui_bootstrap() {
    static int done = 0; // 0 not yet, 1 up, -1 failed
    if (done) return done > 0;
    done = -1;
    // Self-contained (what `day pack` ships, docs/winui.md "Packing"): the runtime sits beside
    // the exe and the exe's manifest registers its classes for registration-free activation.
    // Loading Microsoft.WindowsAppRuntime.dll first is the SDK's own self-contained start-up
    // (its UndockedRegFreeWinRT initializer); no framework package, no bootstrapper.
    ACTCTX_SECTION_KEYED_DATA activation{};
    activation.cbSize = sizeof(activation);
    const bool self_contained = FindActCtxSectionStringW(
        0, nullptr, ACTIVATION_CONTEXT_SECTION_WINRT_ACTIVATABLE_CLASSES,
        L"Microsoft.UI.Xaml.Application", &activation) != FALSE;
    wchar_t exe[MAX_PATH]{};
    DWORD n = GetModuleFileNameW(nullptr, exe, MAX_PATH);
    if (n > 0 && n < MAX_PATH) {
        std::wstring local(exe);
        local.resize(local.find_last_of(L"\\/") + 1);
        local += L"Microsoft.WindowsAppRuntime.dll";
        // DLL presence alone is not a deployment mode: a development build may share an
        // output directory with an earlier package. Only the activation manifest opts in.
        if (self_contained) {
            if (!LoadLibraryExW(local.c_str(), nullptr, 0)) {
                std::fprintf(stderr, "day-toolkit-winui: the self-contained manifest requires an app-local Windows App SDK runtime, but it failed to load\n");
                return false;
            }
            done = 1;
            return true;
        }
    }
    // A packaged app (MSIX) already has its framework dependency on the package graph.
    UINT32 len = 0;
    if (GetCurrentPackageFullName(&len, nullptr) == ERROR_INSUFFICIENT_BUFFER) {
        done = 1;
        return true;
    }
    HMODULE boot = LoadLibraryW(L"Microsoft.WindowsAppRuntime.Bootstrap.dll");
#ifdef DAY_WINAPPSDK_BOOTSTRAP_DLL
    if (!boot) boot = LoadLibraryW(DAY_WINAPPSDK_BOOTSTRAP_DLL);
#endif
    if (!boot) {
        std::fprintf(stderr, "day-toolkit-winui: Microsoft.WindowsAppRuntime.Bootstrap.dll not found\n");
        return false;
    }
    using Init = HRESULT(STDAPICALLTYPE*)(UINT32, PCWSTR, PACKAGE_VERSION,
                                          MddBootstrapInitializeOptions);
    auto init = reinterpret_cast<Init>(GetProcAddress(boot, "MddBootstrapInitialize2"));
    if (!init) return false;
    // The installed runtime of this major version at the release the build targets or newer;
    // ShowUI offers the runtime's download page when none is installed, instead of failing
    // silently.
    PACKAGE_VERSION min_version{};
    min_version.Version = DAY_WINAPPSDK_MIN_VERSION;
    HRESULT hr = init(DAY_WINAPPSDK_MAJOR_MINOR, L"", min_version,
                      MddBootstrapInitializeOptions_OnNoMatch_ShowUI);
    if (FAILED(hr)) {
        std::fprintf(stderr,
                     "day-toolkit-winui: the Windows App SDK runtime %u.%u is not installed (hr=0x%08X)\n",
                     DAY_WINAPPSDK_MAJOR_MINOR >> 16, DAY_WINAPPSDK_MAJOR_MINOR & 0xFFFF,
                     (unsigned)hr);
        return false;
    }
    done = 1;
    return true;
}

// ContentPreTranslateMessage: the island's keyboard routing (accelerators, Tab, access keys).
// A flat export of the runtime, found once the bootstrapper has put it on the package graph.
static BOOL winui_pretranslate(const MSG* msg) {
    using Fn = BOOL(__stdcall*)(const MSG*);
    static Fn fn = [] {
        HMODULE m = LoadLibraryW(L"Microsoft.UI.Windowing.Core.dll");
        return m ? reinterpret_cast<Fn>(GetProcAddress(m, "ContentPreTranslateMessage")) : nullptr;
    }();
    return fn ? fn(msg) : FALSE;
}

// Size and show a WinUI island in its host's client area.
static void winui_place_island(WUXH::DesktopWindowXamlSource const& source, HWND host) {
    RECT rc; GetClientRect(host, &rc);
    auto bridge = source.SiteBridge();
    bridge.MoveAndResize({ 0, 0, rc.right, rc.bottom });
    bridge.Show();
}

// Attach a WinUI island to `host`, returning the island's own child HWND (the bridge's window),
// which is what the rest of this file focuses and positions, exactly as it does system XAML's.
static HWND winui_attach_island(WUXH::DesktopWindowXamlSource const& source, HWND host) {
    source.Initialize(winrt::Microsoft::UI::GetWindowIdFromWindow(host));
    winui_place_island(source, host);
    return winrt::Microsoft::UI::GetWindowFromWindowId(source.SiteBridge().WindowId());
}

#endif

// ---------------------------------------------------------------------------
// window / islands state (single window, v1)
// ---------------------------------------------------------------------------

struct AppWindow {
    HWND host{};
    HWND island{};
#ifdef DAY_WINUI
    // Origin for the most recent primary capture: WGC includes visible window chrome;
    // the RenderTargetBitmap fallback contains only the XAML tree.
    double capture_origin[2]{};
#endif
    WUXH::DesktopWindowXamlSource source{ nullptr };
    WUX::Application app{ nullptr }; // keeps Application::Current + WindowsXamlManager alive
    void* dqc{}; // DispatcherQueueController — kept alive, never released
    WUXC::Canvas root{ nullptr };
    // Day's tree mounts into `content`, a child canvas offset below any docked MenuBar —
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

// --- window display state (docs/windows.md "Window properties") ----------------------------
// Minimized and maximized are the window's own (IsIconic / IsZoomed); fullscreen is Day's
// borderless kind, which Win32 has no state for: the window's frame styles come off, it covers
// its monitor, and the placement and styles it had are kept here to come back to. Codes are
// day_spec::WindowState's: 0 normal, 1 minimized, 2 maximized, 3 fullscreen.
struct WinShowState {
    int reported = 0;        // the last code sent to Day: reports go out on a change only
    bool fullscreen = false; // in Day's borderless fullscreen
    WINDOWPLACEMENT placement{ sizeof(WINDOWPLACEMENT) }; // where it was before fullscreen
    LONG_PTR style = 0;      // its GWL_STYLE before fullscreen
    LONG_PTR ex_style = 0;   // its GWL_EXSTYLE before fullscreen
};
static std::map<HWND, WinShowState> g_show_state;
// Day's report: (node, code). The primary reports as kPrimaryNode, day_spec::WINDOW_NODE's id.
static void (*g_win_state_cb)(unsigned long long, int) = nullptr;
static const unsigned long long kPrimaryNode = ~0ull;
// True while day_xaml_window_set_state works through a request: the restores it passes through on
// the way (minimized → maximized → normal) are not states to report, only where it ends up is.
static bool g_state_request = false;

// What the window shows as now: minimized wins (a fullscreen window can be minimized and comes
// back fullscreen), then Day's fullscreen, then zoomed.
static int window_state_now(HWND hwnd) {
    if (IsIconic(hwnd)) return 1;
    auto it = g_show_state.find(hwnd);
    if (it != g_show_state.end() && it->second.fullscreen) return 3;
    if (IsZoomed(hwnd)) return 2;
    return 0;
}

// Tell Day the window's state when it changed since the last report, or always with `force`:
// a request that did not land is answered with the state the window really is in, so Day's
// optimistic state signal settles back to it.
static void report_window_state(HWND hwnd, unsigned long long node, int state, bool force) {
    auto& st = g_show_state[hwnd];
    if (st.reported == state && !force) return;
    st.reported = state;
    if (g_win_state_cb) g_win_state_cb(node, state);
}

// WM_SIZE's view of the state: its wParam says minimized, maximized or restored; Day's fullscreen
// is restored as far as Win32 knows. The other two codes are about OTHER windows and say nothing.
static void report_size_state(HWND hwnd, unsigned long long node, WPARAM wp) {
    if (g_state_request) return;
    int state;
    switch (wp) {
    case SIZE_MINIMIZED: state = 1; break;
    case SIZE_MAXIMIZED: state = 2; break;
    case SIZE_RESTORED: state = window_state_now(hwnd); break;
    default: return;
    }
    report_window_state(hwnd, node, state, false);
}

// --- window chrome (docs/window-chrome.md) --------------------------------------------------
// What a window wears, from Day's WindowOptions: day-toolkit-xaml-sys's `DayXamlChrome`, field for field.
struct DayXamlChrome {
    int chrome;     // 0 standard, 1 overlay title bar, 2 frameless
    int background; // 0 opaque, 1 transparent, 2 a system material
    int material;   // with background 2: 0 window (Mica), 1 sidebar (Mica Alt), 2 transient (Acrylic)
    int shadow;     // 0 = no drop shadow (honored on a frameless window)
    int resizable;  // 0 = fixed size: no sizing border, no maximize
    int placement;  // 0 automatic, 1 centered, 2 at (x, y)
    double x, y;    // with placement 2: the outer frame's top-left, in points
};

// One window's chrome as the window procedures need it, by host HWND. Every Day window has an
// entry, standard ones included: the drag-region hit test needs the window's root and node too.
//
// Frameless keeps WS_CAPTION and the sizing border and answers WM_NCCALCSIZE with the whole window
// as client area, so DWM still draws the shadow and Aero Snap still works; the edges become
// sizing borders again in WM_NCHITTEST. Overlay is the documented "custom window frame using DWM"
// method: the client area starts at the window's top edge, the frame is extended down over the
// caption band, DWM draws (and DwmDefWindowProc hit-tests) the caption buttons, and the island is
// cut away around those buttons, because DWM draws them BEHIND client content.
struct WinChrome {
    int chrome = 0;
    bool resizable = true;
    bool shadow = true;
    bool material = false; // an explicit Material background whose backdrop DWM accepted
    bool hooked = false;   // the island's windows answer WM_NCHITTEST through chrome_hit_test
    unsigned long long node = 0;
    HWND island = nullptr;
    WUXC::Canvas root{ nullptr };
    double inset = -1; // the safe-area top last reported, DIPs; -1 = none yet
    // Window properties (docs/windows.md "Window properties").
    int docked_px = 0;      // the docked menu bar + toolbar above day's content, px
    bool closable = true;   // false: the close button and Alt+F4 do nothing
    bool skip_taskbar = false;
    int appearance = -1;    // this window's own scheme: -1 the app's, 0 light, 1 dark
    // Content-size limits in points; 0 = none.
    double min_w = 0, min_h = 0, max_w = 0, max_h = 0;
};
static std::map<HWND, WinChrome> g_chrome;
// Day's safe-area report for an overlay window: (node, top inset in DIPs); the primary as
// kPrimaryNode.
static void (*g_chrome_inset_cb)(unsigned long long, double) = nullptr;
// The elements `.window_drag_region()` marked (Toolkit::set_drag_region), in every window.
static std::vector<UIElement> g_drag_regions;
// The island's own windows, subclassed so a press on a sizing border or a drag region falls
// through to the host (HTTRANSPARENT), whose WM_NCHITTEST answers HTLEFT, HTCAPTION and the rest.
struct IslandHook { WNDPROC orig; HWND host; };
static std::map<HWND, IslandHook> g_island_hooks;

static double dip_scale(HWND h) {
    double s = GetDpiForWindow(h) / 96.0;
    return s > 0 ? s : 1.0;
}
// The sizing border's thickness, px: the frame plus the padding Windows adds around it.
static int frame_x(HWND h) {
    UINT dpi = GetDpiForWindow(h);
    return GetSystemMetricsForDpi(SM_CXFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
}
static int frame_y(HWND h) {
    UINT dpi = GetDpiForWindow(h);
    return GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
}
// How far down an overlay window's title bar reaches into its client area, px: the caption plus
// the top sizing border, whose band it also covers. A maximized window's border is off-screen
// (WM_NCCALCSIZE already moved the client top below it), and a fullscreen one has no bar at all.
static int overlay_band_px(HWND h) {
    if (window_state_now(h) == 3) return 0;
    int caption = GetSystemMetricsForDpi(SM_CYCAPTION, GetDpiForWindow(h));
    return IsZoomed(h) ? caption : caption + frame_y(h);
}
static int chrome_of(HWND h) {
    auto it = g_chrome.find(h);
    return it == g_chrome.end() ? 0 : it->second.chrome;
}
// The overlay band in DIPs, 0 for any other window: what layout_window_chrome keeps the docked
// strips below.
static double overlay_band_dip(HWND h) {
    return chrome_of(h) == 1 ? overlay_band_px(h) / dip_scale(h) : 0.0;
}

// Windows' build number, for the system backdrops (Windows 11 22621 and later).
static DWORD windows_build() {
    using Fn = LONG(WINAPI*)(OSVERSIONINFOW*);
    static DWORD build = [] {
        OSVERSIONINFOW v{};
        v.dwOSVersionInfoSize = sizeof(v);
        HMODULE nt = GetModuleHandleW(L"ntdll.dll");
        auto fn = nt ? reinterpret_cast<Fn>(GetProcAddress(nt, "RtlGetVersion")) : nullptr;
        return (fn && fn(&v) == 0) ? v.dwBuildNumber : DWORD{ 0 };
    }();
    return build;
}

// Extend DWM's frame into the client area as far as the chrome needs: all of it under a material
// (the backdrop shows through transparent content only there), the caption band of an overlay
// window (where DWM draws the caption buttons), one pixel all round a frameless window that keeps
// its shadow (DWM draws none for a window with no frame at all).
static void chrome_extend_frame(HWND h, WinChrome const& c) {
    MARGINS m{ 0, 0, 0, 0 };
    if (c.material) {
        m = MARGINS{ -1, -1, -1, -1 };
    } else if (c.chrome == 1) {
        m.cyTopHeight = overlay_band_px(h);
    } else if (c.chrome == 2 && c.shadow) {
        m = MARGINS{ 1, 1, 1, 1 };
    } else if (c.chrome == 0) {
        return; // a standard frame keeps DWM's defaults
    }
    DwmExtendFrameIntoClientArea(h, &m);
}

// Cut the caption buttons out of an overlay window's island, so the host's surface (painted
// black, which DWM's extended frame shows through) and DWM's buttons drawn on it stay visible and
// take their own clicks. The bounds come from DWM on Windows 11; before that, three standard
// buttons at the top right.
static void chrome_cut_caption_buttons(HWND h, WinChrome const& c) {
    if (c.chrome != 1 || !c.island) return;
    if (overlay_band_px(h) == 0) {
        SetWindowRgn(c.island, nullptr, TRUE); // fullscreen: no title bar, nothing to cut
        return;
    }
    RECT client; GetClientRect(h, &client);
    RECT buttons{};
    RECT win; GetWindowRect(h, &win);
    POINT origin{ 0, 0 }; ClientToScreen(h, &origin);
    if (SUCCEEDED(DwmGetWindowAttribute(h, DWMWA_CAPTION_BUTTON_BOUNDS, &buttons, sizeof(buttons)))) {
        // Window coordinates → client coordinates.
        OffsetRect(&buttons, win.left - origin.x, win.top - origin.y);
    } else {
        int bw = GetSystemMetricsForDpi(SM_CXSIZE, GetDpiForWindow(h));
        buttons = RECT{ client.right - 3 * bw, 0, client.right, overlay_band_px(h) };
    }
    HRGN all = CreateRectRgn(0, 0, client.right, client.bottom);
    HRGN cut = CreateRectRgn(buttons.left, buttons.top, buttons.right, buttons.bottom);
    if (all && cut && CombineRgn(all, all, cut, RGN_DIFF) != ERROR) {
        // The system owns the region from here on.
        if (SetWindowRgn(c.island, all, TRUE)) all = nullptr;
    }
    if (all) DeleteObject(all);
    if (cut) DeleteObject(cut);
}

// Tell Day how far the title bar reaches over an overlay window's content: its safe-area top.
static void report_overlay_inset(HWND h, double top_dip) {
    auto it = g_chrome.find(h);
    if (it == g_chrome.end() || it->second.chrome != 1 || !g_chrome_inset_cb) return;
    if (std::fabs(it->second.inset - top_dip) < 0.25) return;
    it->second.inset = top_dip;
    g_chrome_inset_cb(it->second.node, top_dip);
}

// What a press at `p` (DIPs on the window's root) reaches first: 1 a control, 2 a drag region's
// own background, 0 neither. Buttons, fields and every other Control keep their own clicks, also
// inside a region; text, images and empty background let the press through to what is below.
static int press_target(WinChrome const& c, WF::Point p) {
    // Topmost first; a region's descendants come before the region itself.
    for (auto const& hit : WUXM::VisualTreeHelper::FindElementsInHostCoordinates(p, c.root, true)) {
        for (auto const& e : g_drag_regions) {
            if (hit == e) return 2;
        }
        if (auto ctl = hit.try_as<WUXC::Control>()) {
            if (ctl.IsEnabled() && hit.IsHitTestVisible()) return 1;
        }
    }
    return 0;
}

// Whether `p` (DIPs on the window's root) is on a drag region's own background. The regions'
// bounds are checked first, so a window without one under the pointer never walks the tree.
static bool in_drag_region(WinChrome const& c, WF::Point p) {
    if (g_drag_regions.empty() || !c.root) return false;
    auto xr = c.root.XamlRoot();
    if (!xr) return false;
    for (auto const& e : g_drag_regions) {
        auto fe = e.try_as<FrameworkElement>();
        if (!fe || fe.XamlRoot() != xr || fe.Visibility() != WUX::Visibility::Visible) continue;
        WF::Rect r = fe.TransformToVisual(c.root).TransformBounds(
            WF::Rect{ 0, 0, static_cast<float>(fe.ActualWidth()),
                      static_cast<float>(fe.ActualHeight()) });
        if (p.X >= r.X && p.Y >= r.Y && p.X < r.X + r.Width && p.Y < r.Y + r.Height)
            return press_target(c, p) == 2;
    }
    return false;
}

// What a point is on a Day window, for WM_NCHITTEST: false leaves the answer to the window's
// default handling. `lp` carries the screen point, as WM_NCHITTEST's does.
static bool chrome_hit_test(HWND h, WPARAM wp, LPARAM lp, LRESULT* out) try {
    auto it = g_chrome.find(h);
    if (it == g_chrome.end()) return false;
    WinChrome const& c = it->second;
    if (c.chrome == 0 && g_drag_regions.empty()) return false;
    POINT pt{ static_cast<short>(LOWORD(lp)), static_cast<short>(HIWORD(lp)) };
    if (c.chrome == 1) {
        // DWM's caption buttons.
        LRESULT dwm = 0;
        if (DwmDefWindowProc(h, WM_NCHITTEST, wp, lp, &dwm)) {
            *out = dwm;
            return true;
        }
    }
    if (c.chrome != 2) {
        // The frame the window still has: all of it for a standard window, the sides and bottom
        // of an overlay one.
        LRESULT base = DefWindowProcW(h, WM_NCHITTEST, wp, lp);
        if (base != HTCLIENT) {
            *out = base;
            return true;
        }
    }
    RECT win; GetWindowRect(h, &win);
    bool sizable = c.resizable && !IsZoomed(h) && window_state_now(h) != 3;
    if (sizable && c.chrome != 0) {
        // The sizing borders inside the client area: all four edges of a frameless window, the
        // top edge of an overlay one (its others are still real frame, answered above).
        int fx = frame_x(h), fy = frame_y(h);
        bool top = pt.y < win.top + fy;
        bool bottom = c.chrome == 2 && pt.y >= win.bottom - fy;
        bool left = pt.x < win.left + fx;
        bool right = pt.x >= win.right - fx;
        if (c.chrome == 1 && !top) left = right = false;
        if (top && left) { *out = HTTOPLEFT; return true; }
        if (top && right) { *out = HTTOPRIGHT; return true; }
        if (bottom && left) { *out = HTBOTTOMLEFT; return true; }
        if (bottom && right) { *out = HTBOTTOMRIGHT; return true; }
        if (top) { *out = HTTOP; return true; }
        if (bottom) { *out = HTBOTTOM; return true; }
        if (left) { *out = HTLEFT; return true; }
        if (right) { *out = HTRIGHT; return true; }
    }
    POINT client = pt;
    ScreenToClient(h, &client);
    double s = dip_scale(h);
    WF::Point at{ static_cast<float>(client.x / s), static_cast<float>(client.y / s) };
    // An overlay window's title bar band still moves the window wherever the content puts no
    // control under it, as a transparent title bar does on macOS; a drag region moves it anywhere.
    bool caption = c.chrome == 1 && c.root && client.y < overlay_band_px(h) && press_target(c, at) != 1;
    if (caption || in_drag_region(c, at)) {
        // DefWindowProc's own title bar from here: the move loop with Aero Snap, double-click
        // to maximize, the window menu on a right click.
        *out = HTCAPTION;
        return true;
    }
    *out = HTCLIENT;
    return true;
} catch (...) {
    return false;
}

// WM_NCCALCSIZE for a frameless or overlay window: false leaves a standard one to the default.
static bool chrome_nccalcsize(HWND h, WPARAM wp, LPARAM lp, LRESULT* out) {
    int chrome = chrome_of(h);
    if (!wp || chrome == 0) return false;
    auto* p = reinterpret_cast<NCCALCSIZE_PARAMS*>(lp);
    if (chrome == 2) {
        // The whole window is client area. Maximized, Windows hangs the (now invisible) frame
        // off the monitor's edges; clamp to the work area so no content is lost there.
        if (IsZoomed(h)) {
            MONITORINFO mi{};
            mi.cbSize = sizeof(mi);
            if (GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONULL), &mi)) p->rgrc[0] = mi.rcWork;
        }
        *out = 0;
        return true;
    }
    // Overlay: the standard sides and bottom, and the client area up to the window's top edge.
    LONG top = p->rgrc[0].top;
    *out = DefWindowProcW(h, WM_NCCALCSIZE, wp, lp);
    p->rgrc[0].top = top;
    if (IsZoomed(h)) p->rgrc[0].top += frame_y(h);
    return true;
}

// Overlay and material windows paint their host black: the color DWM's extended frame shows
// through, which is what makes the caption buttons and the backdrop visible.
static bool chrome_erase(HWND h, HDC dc) {
    auto it = g_chrome.find(h);
    if (it == g_chrome.end() || (it->second.chrome != 1 && !it->second.material)) return false;
    RECT rc; GetClientRect(h, &rc);
    FillRect(dc, &rc, static_cast<HBRUSH>(GetStockObject(BLACK_BRUSH)));
    return true;
}

static LRESULT CALLBACK island_hook_proc(HWND w, UINT msg, WPARAM wp, LPARAM lp) {
    auto it = g_island_hooks.find(w);
    if (it == g_island_hooks.end()) return DefWindowProcW(w, msg, wp, lp);
    IslandHook hook = it->second;
    if (msg == WM_NCHITTEST) {
        LRESULT r = HTCLIENT;
        if (chrome_hit_test(hook.host, wp, lp, &r) && r != HTCLIENT) return HTTRANSPARENT;
    } else if (msg == WM_NCDESTROY) {
        g_island_hooks.erase(it);
        SetWindowLongPtrW(w, GWLP_WNDPROC, reinterpret_cast<LONG_PTR>(hook.orig));
    }
    return CallWindowProcW(hook.orig, w, msg, wp, lp);
}

static BOOL CALLBACK hook_island_window(HWND w, LPARAM host) {
    if (g_island_hooks.count(w)) return TRUE;
    auto orig = reinterpret_cast<WNDPROC>(
        SetWindowLongPtrW(w, GWLP_WNDPROC, reinterpret_cast<LONG_PTR>(island_hook_proc)));
    if (orig) g_island_hooks[w] = IslandHook{ orig, reinterpret_cast<HWND>(host) };
    return TRUE;
}

// Hook the island's windows (the island and whatever input windows XAML put inside it) once the
// window needs it: it has a custom frame, or some window has a drag region. Re-run on every
// WM_SIZE, which catches a window XAML creates later; an already hooked window is skipped.
static void chrome_hook_island(HWND h) {
    auto it = g_chrome.find(h);
    if (it == g_chrome.end()) return;
    if (it->second.chrome == 0 && g_drag_regions.empty()) return;
    it->second.hooked = true;
    EnumChildWindows(h, hook_island_window, reinterpret_cast<LPARAM>(h));
}

// The window was resized or changed state: re-extend the frame (the overlay band changes with
// maximize and DPI), re-cut the caption buttons, and catch any new island window.
static void chrome_on_size(HWND h) {
    auto it = g_chrome.find(h);
    if (it == g_chrome.end()) return;
    if (it->second.chrome != 0 || it->second.material) chrome_extend_frame(h, it->second);
    chrome_cut_caption_buttons(h, it->second);
    chrome_hook_island(h);
}

// The outer size for a client area of w×h under `style` and the chrome: a frameless window is all
// client, an overlay one loses only its caption band, a standard one adds the whole frame.
static void chrome_outer_size(DWORD style, int chrome, int w, int h, int* ow, int* oh) {
    if (chrome == 2) { *ow = w; *oh = h; return; }
    RECT r{ 0, 0, w, h };
    AdjustWindowRect(&r, style, FALSE);
    *ow = r.right - r.left;
    *oh = chrome == 1 ? r.bottom : r.bottom - r.top;
}

// The window styles for the chrome: resizable keeps the sizing border and the maximize box.
static DWORD chrome_style(DWORD style, DayXamlChrome const* opt) {
    if (opt && !opt->resizable) style &= ~(WS_THICKFRAME | WS_MAXIMIZEBOX);
    return style;
}

// Record a new window's chrome and apply what has to follow its creation: the frame
// recalculation the custom WM_NCCALCSIZE needs, the shadow, the backdrop. Returns whether an
// explicit material backdrop was accepted, which is when the window's root stays transparent.
static bool chrome_init(HWND h, unsigned long long node, DayXamlChrome const* opt) {
    WinChrome c;
    c.node = node;
    if (opt) {
        c.chrome = (opt->chrome == 1 || opt->chrome == 2) ? opt->chrome : 0;
        c.resizable = opt->resizable != 0;
        c.shadow = opt->shadow != 0;
    }
    if (opt && opt->background == 2) {
        DWORD backdrop = opt->material == 1   ? DWMSBT_TABBEDWINDOW
                         : opt->material == 2 ? DWMSBT_TRANSIENTWINDOW
                                              : DWMSBT_MAINWINDOW;
        c.material = SUCCEEDED(
            DwmSetWindowAttribute(h, DWMWA_SYSTEMBACKDROP_TYPE, &backdrop, sizeof(backdrop)));
    }
    bool material = c.material;
    g_chrome[h] = c;
    if (c.chrome == 2 && !c.shadow) {
        // No non-client rendering at all: no shadow, and no frame for DWM to round.
        DWMNCRENDERINGPOLICY policy = DWMNCRP_DISABLED;
        DwmSetWindowAttribute(h, DWMWA_NCRENDERING_POLICY, &policy, sizeof(policy));
    }
    if (c.chrome != 0 || c.material) {
        chrome_extend_frame(h, c);
        SetWindowPos(h, nullptr, 0, 0, 0, 0,
                     SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
    }
    return material;
}

// The island exists and has loaded: remember it and its root for hit-testing.
static void chrome_attach(HWND h, HWND island, WUXC::Canvas const& root) {
    auto it = g_chrome.find(h);
    if (it == g_chrome.end()) return;
    it->second.island = island;
    it->second.root = root;
    chrome_on_size(h);
}

// Where the window opens. Centered goes on the work area of the monitor showing the active
// window (the primary's when there is none); At is in points on the desktop, scaled by the
// system DPI. Automatic leaves the window manager's choice.
static void chrome_place(HWND h, DayXamlChrome const* opt) {
    if (!opt || opt->placement == 0) return;
    RECT r; GetWindowRect(h, &r);
    int x = 0, y = 0;
    if (opt->placement == 1) {
        HWND ref = GetForegroundWindow();
        if (!ref || ref == h) ref = g_app ? g_app->host : nullptr;
        HMONITOR mon = ref ? MonitorFromWindow(ref, MONITOR_DEFAULTTOPRIMARY)
                           : MonitorFromPoint(POINT{ 0, 0 }, MONITOR_DEFAULTTOPRIMARY);
        MONITORINFO mi{};
        mi.cbSize = sizeof(mi);
        if (!GetMonitorInfoW(mon, &mi)) return;
        x = mi.rcWork.left + ((mi.rcWork.right - mi.rcWork.left) - (r.right - r.left)) / 2;
        y = mi.rcWork.top + ((mi.rcWork.bottom - mi.rcWork.top) - (r.bottom - r.top)) / 2;
    } else {
        double s = GetDpiForSystem() / 96.0;
        x = static_cast<int>(std::lround(opt->x * s));
        y = static_cast<int>(std::lround(opt->y * s));
    }
    SetWindowPos(h, nullptr, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
}


// --- window properties, taskbar (docs/windows.md "Window properties", docs/status-item.md) ----

// Points on the desktop ↔ pixels: the system DPI, one scale for every position Day reads or sets,
// so a frame read back and set again lands where it was.
static double desktop_scale() {
    double s = GetDpiForSystem() / 96.0;
    return s > 0 ? s : 1.0;
}

// The taskbar's per-window interface (progress, overlay badge, tabs), created on first use.
// Calls before the window's button exists fail, so the state is kept and re-applied when
// Windows announces the button ("TaskbarButtonCreated").
static ITaskbarList3* taskbar_list() {
    static winrt::com_ptr<ITaskbarList3> list = [] {
        winrt::com_ptr<ITaskbarList3> p;
        if (FAILED(CoCreateInstance(__uuidof(TaskbarList), nullptr, CLSCTX_INPROC_SERVER,
                                    __uuidof(ITaskbarList3), p.put_void())) ||
            FAILED(p->HrInit()))
            p = nullptr;
        return p;
    }();
    return list.get();
}
static UINT taskbar_button_created_msg() {
    static UINT msg = RegisterWindowMessageW(L"TaskbarButtonCreated");
    return msg;
}

// The app's progress (Toolkit::set_app_progress) and badge (set_app_badge), on the primary's
// taskbar button: kept so they survive the button being re-created.
static int g_progress_kind = 0; // 0 none, 1 indeterminate, 2 value, 3 paused, 4 error
static double g_progress_value = 0;
static int g_badge_kind = 0;    // 0 none, 1 count, 2 dot
static unsigned g_badge_count = 0;

static void apply_progress() {
    ITaskbarList3* tb = taskbar_list();
    HWND h = g_app ? g_app->host : nullptr;
    if (!tb || !h) return;
    static const TBPFLAG states[] = { TBPF_NOPROGRESS, TBPF_INDETERMINATE, TBPF_NORMAL, TBPF_PAUSED,
                                      TBPF_ERROR };
    int k = g_progress_kind >= 0 && g_progress_kind <= 4 ? g_progress_kind : 0;
    tb->SetProgressState(h, states[k]);
    if (k >= 2) {
        double v = g_progress_value < 0 ? 0 : g_progress_value > 1 ? 1 : g_progress_value;
        tb->SetProgressValue(h, static_cast<ULONGLONG>(std::lround(v * 1000)), 1000);
    }
}

// A red disc at the small-icon size, with the count in white (99+ above 99) or nothing (a dot):
// the overlay Windows draws over the corner of the taskbar button.
static HICON badge_icon(int kind, unsigned count) {
    int size = GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem());
    HICON icon = nullptr;
    ULONG_PTR token = 0;
    Gdiplus::GdiplusStartupInput si;
    if (Gdiplus::GdiplusStartup(&token, &si, nullptr) != Gdiplus::Ok) return nullptr;
    {
        Gdiplus::Bitmap bmp(size, size, PixelFormat32bppARGB);
        Gdiplus::Graphics g(&bmp);
        g.SetSmoothingMode(Gdiplus::SmoothingModeAntiAlias);
        g.SetTextRenderingHint(Gdiplus::TextRenderingHintAntiAliasGridFit);
        g.Clear(Gdiplus::Color(0, 0, 0, 0));
        Gdiplus::SolidBrush red(Gdiplus::Color(255, 196, 43, 28));
        g.FillEllipse(&red, 0.0f, 0.0f, static_cast<Gdiplus::REAL>(size),
                      static_cast<Gdiplus::REAL>(size));
        if (kind == 1) {
            std::wstring text = count > 99 ? L"99+" : std::to_wstring(count);
            float em = size * (text.size() > 2 ? 0.45f : text.size() > 1 ? 0.55f : 0.7f);
            Gdiplus::FontFamily family(L"Segoe UI");
            Gdiplus::Font font(&family, em, Gdiplus::FontStyleBold, Gdiplus::UnitPixel);
            Gdiplus::StringFormat fmt;
            fmt.SetAlignment(Gdiplus::StringAlignmentCenter);
            fmt.SetLineAlignment(Gdiplus::StringAlignmentCenter);
            Gdiplus::SolidBrush white(Gdiplus::Color(255, 255, 255, 255));
            Gdiplus::RectF box(0.0f, 0.0f, static_cast<Gdiplus::REAL>(size),
                               static_cast<Gdiplus::REAL>(size));
            g.DrawString(text.c_str(), static_cast<INT>(text.size()), &font, box, &fmt, &white);
        }
        bmp.GetHICON(&icon);
    } // the bitmap goes before GdiplusShutdown
    Gdiplus::GdiplusShutdown(token);
    return icon;
}

static void apply_badge() {
    ITaskbarList3* tb = taskbar_list();
    HWND h = g_app ? g_app->host : nullptr;
    if (!tb || !h) return;
    if (g_badge_kind == 0) {
        tb->SetOverlayIcon(h, nullptr, nullptr);
        return;
    }
    HICON icon = badge_icon(g_badge_kind, g_badge_count);
    if (!icon) return;
    std::wstring what = g_badge_kind == 1 ? std::to_wstring(g_badge_count) : std::wstring(L"\u2022");
    tb->SetOverlayIcon(h, icon, what.c_str());
    DestroyIcon(icon); // the taskbar keeps its own copy
}

// A window's taskbar button (re)appeared: put back what Day had shown on it.
static void taskbar_button_created(HWND h) {
    if (g_app && h == g_app->host) {
        apply_progress();
        apply_badge();
    }
    auto it = g_chrome.find(h);
    if (it != g_chrome.end() && it->second.skip_taskbar)
        if (ITaskbarList3* tb = taskbar_list()) tb->DeleteTab(h);
}

// WM_GETMINMAXINFO for a window with Day limits: the content limits, in points, as outer track
// sizes (the frame and the docked chrome added back). False leaves the message to the default.
static bool window_minmax(HWND h, MINMAXINFO* mmi) {
    auto it = g_chrome.find(h);
    if (it == g_chrome.end()) return false;
    WinChrome const& c = it->second;
    if (c.min_w <= 0 && c.min_h <= 0 && c.max_w <= 0 && c.max_h <= 0) return false;
    RECT wr, cr;
    GetWindowRect(h, &wr);
    GetClientRect(h, &cr);
    int ex = (wr.right - wr.left) - cr.right;
    int ey = (wr.bottom - wr.top) - cr.bottom + c.docked_px;
    double s = dip_scale(h);
    auto px = [s](double pt) { return static_cast<LONG>(std::lround(pt * s)); };
    if (c.min_w > 0) mmi->ptMinTrackSize.x = px(c.min_w) + ex;
    if (c.min_h > 0) mmi->ptMinTrackSize.y = px(c.min_h) + ey;
    if (c.max_w > 0) mmi->ptMaxTrackSize.x = px(c.max_w) + ex;
    if (c.max_h > 0) mmi->ptMaxTrackSize.y = px(c.max_h) + ey;
    // Maximized too: Windows sizes a maximized window by ptMaxSize, not the track sizes, so a
    // window with a maximum would otherwise still fill the screen when zoomed. It keeps the work
    // area's top-left (ptMaxPosition).
    if (c.max_w > 0) mmi->ptMaxSize.x = (std::min)(mmi->ptMaxSize.x, mmi->ptMaxTrackSize.x);
    if (c.max_h > 0) mmi->ptMaxSize.y = (std::min)(mmi->ptMaxSize.y, mmi->ptMaxTrackSize.y);
    return true;
}

// Whether a WM_SYSCOMMAND is a close this window refuses (WindowChange::Closable(false)): the
// grayed close button already does nothing, and this stops Alt+F4 and the window menu too.
static bool close_refused(HWND h, WPARAM wp) {
    if ((wp & 0xFFF0) != SC_CLOSE) return false;
    auto it = g_chrome.find(h);
    return it != g_chrome.end() && !it->second.closable;
}

// The chrome's part of a Day window procedure, run first by both: DWM's caption buttons on an
// overlay window, the custom client area and hit-testing, the black ground. True with `out` set
// when the message is answered.
static bool chrome_wndproc(HWND h, UINT msg, WPARAM wp, LPARAM lp, LRESULT* out) {
    if (chrome_of(h) == 1 && DwmDefWindowProc(h, msg, wp, lp, out)) return true;
    if (msg == taskbar_button_created_msg()) {
        taskbar_button_created(h);
        return false;
    }
    switch (msg) {
    case WM_NCCALCSIZE: return chrome_nccalcsize(h, wp, lp, out);
    case WM_NCHITTEST: return chrome_hit_test(h, wp, lp, out);
    case WM_ERASEBKGND:
        if (!chrome_erase(h, reinterpret_cast<HDC>(wp))) return false;
        *out = 1;
        return true;
    case WM_GETMINMAXINFO:
        if (!window_minmax(h, reinterpret_cast<MINMAXINFO*>(lp))) return false;
        *out = 0;
        return true;
    case WM_SYSCOMMAND:
        if (!close_refused(h, wp)) return false;
        *out = 0;
        return true;
    case WM_NCACTIVATE:
        // A frameless window has no non-client area to repaint for the active state, and
        // repainting one anyway can flash a classic caption over the content. -1 asks
        // DefWindowProc to update the state without painting.
        if (chrome_of(h) != 2) return false;
        *out = DefWindowProcW(h, WM_NCACTIVATE, wp, -1);
        return true;
    }
    return false;
}

static const UINT WM_DAY_POST = WM_APP + 1;
struct PostMsg { void (*cb)(void*); void* data; };
// Posts made before the primary window exists. day_xaml_post runs on other threads (the dayscript
// engine's, a network callback's), and the engine starts listening as the app launches, before
// the window is up. A post then had no HWND to go to and was DROPPED, so the request behind it
// never answered: the first step of a walkthrough waited out its whole main-thread timeout on a
// cold start (WinUI's first launch after a build takes longest to open its window), then every
// later step passed. They wait here instead and go out, in order, once the window exists.
// The console display's power state (GUID_CONSOLE_DISPLAY_STATE: 0 off, 1 on, 2 dimmed), from
// the WM_POWERBROADCAST the primary window registers for. With the display off DWM stops
// composing, so a window capture reads a blank or stale surface; pump_until_presented wakes it.
static const GUID kDisplayStateGuid = {
    0x6fe69556, 0x704a, 0x47a0, { 0x8f, 0x24, 0xc2, 0x8d, 0x93, 0x6f, 0xda, 0x47 } };
static DWORD g_display_state = 1;
static std::mutex g_post_mutex;
static std::vector<PostMsg*> g_post_early;
static bool g_post_open = false; // the primary window has been created (posts go straight to it)
// The hidden shell window (docs/status-item.md, defined with the status items further down): it
// takes Day's posts once the primary window is gone, which an app kept running needs.
static HWND g_shell_hwnd = nullptr;
// The app's small icon (day_xaml_set_app_icon), what a status item without a glyph shows.
static HICON g_app_icon_small = nullptr;
// Day's window-resize report (single window, v1 — like g_app). UNVERIFIED on a live
// Windows host; mirrors the Qt shim's DayWindow::resizeEvent contract.
static void (*g_resize_cb)(int, int) = nullptr;
// Minimum CLIENT size (points) from WindowOptions.min_size; 0 = no minimum. Single-window v1.
static int g_min_w = 0, g_min_h = 0;
// Lifecycle (docs/lifecycle.md): codes match day_spec::Lifecycle order (2=DidBecomeActive,
// 3=WillResignActive, 7=WillTerminate).
static void (*g_lifecycle_cb)(int) = nullptr;

// Reserve the docked chrome's strips: size the MenuBar and the toolbar CommandBar to the window
// width, stack them at the top, offset day's content canvas below both, and report the REMAINING
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
    // An overlay title bar (docs/window-chrome.md) lies over the top of the client area: the
    // docked strips go below it, and day's content runs up under all of it, told how far down
    // through the safe-area top inset instead of being moved.
    bool overlay = chrome_of(host) == 1;
    double band_dip = overlay_band_dip(host);
    double mh_dip = 0;
    if (menubar) {
        menubar.Measure(WF::Size{ std::numeric_limits<float>::infinity(),
                                  std::numeric_limits<float>::infinity() });
        mh_dip = menubar.DesiredSize().Height;
        menubar.Width(rc.right / scale);
        WUXC::Canvas::SetTop(menubar, band_dip);
    }
    double th_dip = 0;
    if (toolbar) {
        // Measured at the real width: a CommandBar lays its commands out against the width it
        // gets, and an infinite proposal would not give the height it will actually draw at.
        toolbar.Measure(WF::Size{ static_cast<float>(rc.right / scale),
                                  std::numeric_limits<float>::infinity() });
        th_dip = toolbar.DesiredSize().Height;
        toolbar.Width(rc.right / scale);
        WUXC::Canvas::SetTop(toolbar, band_dip + mh_dip);
    }
    if (overlay) {
        if (auto it = g_chrome.find(host); it != g_chrome.end()) it->second.docked_px = 0;
        if (content) WUXC::Canvas::SetTop(content, 0);
        if (out_w) *out_w = rc.right;
        if (out_h) *out_h = rc.bottom;
        report_overlay_inset(host, band_dip + mh_dip + th_dip);
        return;
    }
    if (content) WUXC::Canvas::SetTop(content, mh_dip + th_dip);
    int chrome_px = static_cast<int>(std::lround((mh_dip + th_dip) * scale));
    if (auto it = g_chrome.find(host); it != g_chrome.end()) it->second.docked_px = chrome_px;
    if (out_w) *out_w = rc.right;
    if (out_h) *out_h = rc.bottom > chrome_px ? rc.bottom - chrome_px : 0;
}

// Reserve the docked chrome's strips on the PRIMARY window and report the remaining client size
// to day-core (XAML works in DIPs; the resize report and client rect are physical px). Either
// strip may be absent, and they are installed in either order, so both offsets are recomputed
// here rather than where a bar is created.
//
// The size goes to day in points (DIPs), which is what `Event::WindowResized` carries and what
// day's layout positions XAML elements in. It went out in pixels before, the same number at 100%
// scale and a layout too large for the window at any other.
static void day_xaml_relayout_chrome(AppWindow* app) {
    if (!app || !app->host) return;
    int w = 0, h = 0;
    layout_window_chrome(app->host, app->content, app->menubar, app->toolbar, &w, &h);
    double s = dip_scale(app->host);
    if (g_resize_cb)
        g_resize_cb(static_cast<int>(std::lround(w / s)), static_cast<int>(std::lround(h / s)));
}

/// Effective light/dark: a DAY_THEME force wins, else the system's current setting.
///
/// Read from the registry rather than from `Application::RequestedTheme()`. That property is
/// resolved when the XAML app object is constructed and does NOT track a later system flip under
/// Islands — the XAML tree still re-themes itself (its brushes are theme resources), so the
/// property looks right up to the moment you ask it during a change, which is exactly when the
/// title bar needs the new value. `AppsUseLightTheme` is what the ImmersiveColorSet broadcast is
/// announcing, so it is both current and authoritative.
static bool effective_dark() try {
    if (g_forced_theme) return g_forced_theme == 2;
    if (g_app_override) return g_app_override == 2;
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
    // A window with its own scheme (WindowChange::Appearance) keeps it over the app's.
    auto own = g_chrome.find(host);
    bool want = own != g_chrome.end() && own->second.appearance >= 0 ? own->second.appearance == 1
                                                                      : effective_dark();
    BOOL dark = want ? TRUE : FALSE;
    DwmSetWindowAttribute(host, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark, sizeof(dark));
}

/// Fired when the system light/dark setting flips, so day can re-evaluate its own palette
/// (docs/settings.md). Set once from Rust; null until then.
static void (*g_appearance_cb)() = nullptr;

/// Re-theme and re-ground every live window. Defined below, once `SecWindow` is a complete type;
/// declared here because the window procedure calls it on WM_SETTINGCHANGE. The `extern "C"`
/// matches the definition's — it lands inside this file's big C-linkage block, and a declaration
/// with C++ linkage would contradict it.
extern "C" {
static void apply_appearance_everywhere();
}

/// True for the broadcast Windows sends when the light/dark setting flips.
static bool is_color_scheme_change(LPARAM lp) {
    auto area = reinterpret_cast<const wchar_t*>(lp);
    return area && std::wcscmp(area, L"ImmersiveColorSet") == 0;
}

// --- pointer shape (the `.cursor()` modifier, docs/cursor.md) ---------------------------------
// UWP XAML hosted in a Win32 window has no per-element cursor property (that is WinUI 3's
// ProtectedCursor) and no CoreWindow to hand a PointerCursor to, so the shape is owned by the
// host: each decorated element records its code on PointerEntered, the nearest decorated
// ancestor's on PointerExited, and the host answers WM_SETCURSOR with the current one. The
// island's own DefWindowProc forwards WM_SETCURSOR to its parent first, which is what lets
// this window's answer stand.
static std::map<void*, int> g_cursor_codes;   // element ABI pointer -> code (1..=17)
static std::set<void*> g_cursor_hooked;       // elements with the enter/exit handlers attached
static int g_cursor_current = 0;              // what WM_SETCURSOR shows; 0 = the default arrow
static HCURSOR cursor_for_code(int code) {
    LPCWSTR id = IDC_ARROW;
    switch (code) {
    case 2: id = IDC_IBEAM; break;
    case 3: id = IDC_WAIT; break;
    case 4: id = IDC_CROSS; break;
    case 5: id = IDC_UPARROW; break;
    case 6: id = IDC_SIZENWSE; break;
    case 7: id = IDC_SIZENESW; break;
    case 8: id = IDC_SIZEWE; break;
    case 9: id = IDC_SIZENS; break;
    case 10: id = IDC_SIZEALL; break;
    case 11: id = IDC_NO; break;
    case 12: id = IDC_HAND; break;
    case 13: id = IDC_APPSTARTING; break;
    case 14: id = IDC_HELP; break;
    case 15: id = MAKEINTRESOURCEW(32671); break; // IDC_PIN (Windows 10 1809+)
    case 16: id = MAKEINTRESOURCEW(32672); break; // IDC_PERSON (Windows 10 1809+)
    default: break;
    }
    HCURSOR c = LoadCursorW(nullptr, id);
    return c ? c : LoadCursorW(nullptr, IDC_ARROW);
}
static void apply_cursor_now(int code) {
    g_cursor_current = code;
    if (code == 17) { SetCursor(nullptr); return; }
    SetCursor(cursor_for_code(code));
}
// The code in force for an element: its own, else the nearest decorated ancestor's, else 0.
static int cursor_code_over(UIElement const& e) {
    WUX::DependencyObject d = e;
    while (d) {
        auto it = g_cursor_codes.find(winrt::get_abi(d));
        if (it != g_cursor_codes.end()) return it->second;
        d = WUXM::VisualTreeHelper::GetParent(d);
    }
    return 0;
}

static LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    if (LRESULT r = 0; chrome_wndproc(hwnd, msg, wp, lp, &r)) return r;
    switch (msg) {
    case WM_SETCURSOR:
        if (g_cursor_current != 0 && LOWORD(lp) == HTCLIENT) {
            apply_cursor_now(g_cursor_current);
            return TRUE;
        }
        break;
    case WM_SIZE:
        if (g_app && g_app->island) {
            RECT rc; GetClientRect(hwnd, &rc);
#ifdef DAY_WINUI
            winui_place_island(g_app->source, hwnd);
#else
            SetWindowPos(g_app->island, nullptr, 0, 0, rc.right, rc.bottom, SWP_SHOWWINDOW);
#endif
            chrome_on_size(hwnd);
            day_xaml_relayout_chrome(g_app);
        }
        report_size_state(hwnd, kPrimaryNode, wp);
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
        if (is_color_scheme_change(lp)) {
            // Everything, not just this window's title bar: the roots' grounds are as stale as the
            // chrome after a system flip, and other windows get no broadcast of their own.
            apply_appearance_everywhere();
            // …and tell day, so APP-PAINTED color follows too. XAML's own controls re-theme
            // themselves from theme resources, which is why the window looked like it was
            // following all along — but every palette closure day evaluates reads day's dark
            // signal, and nothing was ever updating it here.
            if (g_appearance_cb) g_appearance_cb();
        }
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
    case WM_POWERBROADCAST:
        // The display's power state, which a capture needs (see pump_until_presented).
        if (wp == PBT_POWERSETTINGCHANGE) {
            auto s = reinterpret_cast<const POWERBROADCAST_SETTING*>(lp);
            if (s && IsEqualGUID(s->PowerSetting, kDisplayStateGuid) && s->DataLength >= sizeof(DWORD))
                g_display_state = *reinterpret_cast<const DWORD*>(s->Data);
        }
        return TRUE;
    case WM_DESTROY:
        // NOT the end of the app any more (docs/windows.md close policy): this window is an
        // ordinary primary window, and day-core ends the process — through `day_xaml_quit` —
        // when the last primary closes. Quitting here would take the other windows with it.
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

extern "C" void day_xaml_set_lifecycle_cb(void (*cb)(int)) { g_lifecycle_cb = cb; }

extern "C" void day_xaml_set_primary_closed_cb(void (*cb)()) { g_primary_closed = cb; }

// Appearance (docs/settings.md): the system light/dark setting, and a notification when the
// user changes it. `day_xaml_is_dark` answers the `dark_mode` duty — a DAY_THEME force wins,
// else the live system value — so day's palette resolves the same way the window chrome does.
extern "C" void day_xaml_set_appearance_cb(void (*cb)()) { g_appearance_cb = cb; }
extern "C" int day_xaml_is_dark() { return effective_dark() ? 1 : 0; }

// Reduce motion (docs/accessibility.md): Windows' "Show animations" switch, which UISettings
// publishes as `AnimationsEnabled` — reduce motion is that switch OFF. `day_xaml_reduce_motion`
// answers the `reduce_motion` duty from a fresh read, so it is current whenever day-core asks.
//
// `AnimationsEnabledChanged` (Windows 10 1803+; an older build throws out of the subscription,
// which leaves the setting readable but unwatched) is raised on a WinRT thread-pool thread,
// never the UI thread, and day-core's gate lives on Day's thread — so the handler only posts,
// and the callback into Rust runs from the posted message like any other cross-thread work.
// The UISettings instance holds the subscription, so it is kept alive for the process, never
// released, like the DispatcherQueueController: its reference is detached into a raw ABI pointer.
// (A projected type is itself a reference, and C++/WinRT deletes its `operator new`.)
static void* g_ui_settings = nullptr;
static void (*g_motion_cb)() = nullptr;
// Defined further down, inside the big C-linkage block; declared with the same linkage.
extern "C" void day_xaml_post(void (*cb)(void*), void* data);

extern "C" int day_xaml_reduce_motion() {
    int out = 0;
    guard([&] {
        winrt::Windows::UI::ViewManagement::UISettings ui;
        out = ui.AnimationsEnabled() ? 0 : 1;
    });
    return out;
}

extern "C" void day_xaml_watch_reduce_motion(void (*cb)()) try {
    g_motion_cb = cb;
    if (g_ui_settings) return; // subscribed once; a later call only changes the callback
    winrt::Windows::UI::ViewManagement::UISettings ui;
    ui.AnimationsEnabledChanged([](auto const&, auto const&) {
        // Thread-pool thread: cross to the UI thread before touching Rust.
        day_xaml_post([](void*) { if (g_motion_cb) g_motion_cb(); }, nullptr);
    });
    // Only once the subscription took: a throw above leaves the setting unwatched, not leaked.
    g_ui_settings = winrt::detach_abi(ui);
} catch (...) {
}

// Destroy the primary window's host, once day-core has torn its content down (the released
// root handle is the signal). The counterpart of day_xaml_window_destroy2 for window zero.
extern "C" void day_xaml_destroy_primary() {
    if (g_app && g_app->host) {
        HWND h = g_app->host;
        {
            // day_xaml_post reads it from other threads, under this lock.
            std::lock_guard<std::mutex> lock(g_post_mutex);
            g_app->host = nullptr;
        }
        g_show_state.erase(h);
        g_chrome.erase(h);
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

// Defined with the rest of the set_appearance machinery further down this same block, but needed
// by the two window-creation paths above it: a window's ground and its root ElementTheme must
// agree with what a later re-theme would compute, or it opens in one scheme and re-themes into
// another. Declared in here, beside the definitions, for the linkage reason noted at
// `cached_path_geometry` — a declaration outside the block contradicts the definition inside it.
static void ground_root(WUXC::Canvas const& root, bool dark);
static WUX::ElementTheme element_theme_now();

// Tab past the last control (or Shift+Tab past the first) asks the host to TAKE focus, since an
// island assumes it sits among other Win32 controls. A Day window is the island and nothing else,
// so an unanswered request left focus parked on the last control: Tab stuck on the toolbar's
// overflow button, Shift+Tab stuck at the nav pane's button. Answer it by wrapping around inside
// the island, which is what Tab does in any window. Arrow-key edges (Left/Right/Up/Down) are left
// alone: those stop at the edge rather than wrap.
static void wrap_tab_focus(WUXH::DesktopWindowXamlSource const& source) {
    source.TakeFocusRequested([](WUXH::DesktopWindowXamlSource const& s,
                                 WUXH::DesktopWindowXamlSourceTakeFocusRequestedEventArgs const& a) {
        static bool wrapping = false; // an island with no tab stop at all would ask again
        auto request = a.Request();
        auto why = request.Reason();
        if (wrapping || (why != WUXH::XamlSourceFocusNavigationReason::First &&
                         why != WUXH::XamlSourceFocusNavigationReason::Last))
            return;
        wrapping = true;
        try {
            // The request already names the far end: First after a forward Tab, Last after a
            // backward one, which is where focus re-enters.
            s.NavigateFocus(request);
        } catch (...) {
        }
        wrapping = false;
    });
}

void* day_xaml_window_new(const char* title, int w, int h, int min_w, int min_h,
                          const DayXamlChrome* chrome) try {
    g_min_w = min_w;
    g_min_h = min_h;
    winrt::init_apartment(winrt::apartment_type::single_threaded);
    // OLE, not just COM — this is what makes drops from OTHER PROCESSES arrive (docs/drag-and-drop.md).
    //
    // `init_apartment` above is `CoInitializeEx(COINIT_APARTMENTTHREADED)`, which brings up COM
    // and nothing else. Cross-process drag and drop is an OLE service: to receive a drop, a
    // window must be registered with `RegisterDragDrop`, and that call is documented to fail
    // unless `OleInitialize` ran on the same thread first. XAML registers the island's HWND for
    // us when an element sets `AllowDrop`, so the failure is invisible — no error surfaces
    // anywhere, `AllowDrop` still reports true, and drags WITHIN the app keep working because
    // XAML routes those itself without OLE. Only drops from Explorer (or any other process)
    // silently never arrive, which is exactly how this looked.
    //
    // OleInitialize enters the same STA `init_apartment` just established, so it adds the OLE
    // layer rather than changing the apartment. No matching OleUninitialize: it would have to run
    // after XAML has released every island, and the process is exiting at that point anyway.
    OleInitialize(nullptr);

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
#ifdef DAY_WINUI
    // WinUI 3 runs on its own DispatcherQueue (Microsoft.UI.Dispatching), which has to exist on
    // this thread before its XAML does, and the framework package has to be on the package
    // graph before either can activate. Kept alive for the process, like the system one above.
    if (!winui_bootstrap()) return nullptr;
    static auto winui_dqc =
        winrt::Microsoft::UI::Dispatching::DispatcherQueueController::CreateOnCurrentThread();
#endif

    // Application must exist before controls so default styles resolve; its ctor also inits
    // the WindowsXamlManager for this thread.
    auto app = winrt::make<DayApp>();
    // DAYDBG (temporary): surface XAML-level exceptions that would otherwise fail-fast silently.
    app.UnhandledException([](WF::IInspectable const&,
                              WUX::UnhandledExceptionEventArgs const& e) {
        std::fprintf(stderr, "DAYDBG xaml unhandled: hr=0x%08X msg=%ls\n",
                     (unsigned)e.Exception(), e.Message().c_str());
        std::fflush(stderr);
    });
    SetUnhandledExceptionFilter([](EXCEPTION_POINTERS* p) -> LONG {
        std::fprintf(stderr, "DAYDBG seh: code=0x%08X addr=%p\n",
                     (unsigned)p->ExceptionRecord->ExceptionCode,
                     p->ExceptionRecord->ExceptionAddress);
        std::fflush(stderr);
        return EXCEPTION_CONTINUE_SEARCH;
    });
    // Effective light/dark: a DAY_THEME force wins, else the app's system-resolved theme (the
    // RequestedTheme getter reports the system value when nothing was forced). Drives the Win32
    // chrome (transient client brush + dark title bar) and, via g_forced_theme, the code-behind
    // theme-brush fills. The theme itself is forced per-element on the root below, not app-wide.
    g_forced_theme = day_theme_env(); // 0 unset, 1 light, 2 dark
    bool app_dark = effective_dark();

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

    // Resizable (WM_SIZE reflows the island + day tree) unless WindowOptions says otherwise.
    DWORD style = chrome_style(WS_OVERLAPPEDWINDOW, chrome);
    int frame_kind = chrome ? chrome->chrome : 0;
    int outer_w = w, outer_h = h;
    // A scripted run with a stated capture size (DAY_CAPTURE_SCALE, Day.toml [screenshots])
    // names the pixels it captures, and this backend captures the whole window, frame included
    // (snapshot_hwnd_png). There the stated size is the OUTER size; everywhere else it is the
    // client area, and the frame is added around it.
    char capture_scale[16];
    bool capture_run =
        GetEnvironmentVariableA("DAY_CAPTURE_SCALE", capture_scale, sizeof(capture_scale)) > 0;
    if (!capture_run) chrome_outer_size(style, frame_kind, w, h, &outer_w, &outer_h);
    HWND host = CreateWindowExW(0, L"day_xaml_host", hs(title).c_str(), style,
                                CW_USEDEFAULT, CW_USEDEFAULT, outer_w, outer_h,
                                nullptr, nullptr, wc.hInstance, nullptr);
    // The chrome (docs/window-chrome.md), and where the window opens, before it is first shown.
    bool material = chrome_init(host, kPrimaryNode, chrome);
    chrome_place(host, chrome);

    WUXH::DesktopWindowXamlSource source;
#ifdef DAY_WINUI
    HWND island = winui_attach_island(source, host);
    wrap_tab_focus(source);
#else
    auto interop = source.as<::IDesktopWindowXamlSourceNative>();
    interop->AttachToWindow(host);
    wrap_tab_focus(source);
    HWND island = nullptr;
    interop->get_WindowHandle(&island);
    RECT rc; GetClientRect(host, &rc);
    SetWindowPos(island, nullptr, 0, 0, rc.right, rc.bottom, SWP_SHOWWINDOW);
#endif

    // The display's power state, for captures (g_display_state). Windows answers the
    // registration with the current state straight away.
    RegisterPowerSettingNotification(host, &kDisplayStateGuid, DEVICE_NOTIFY_WINDOW_HANDLE);

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
    //
    // A window that asked for a material (WindowOptions.background) already has its own backdrop
    // from chrome_init, frame extension included; it is not replaced here.
    bool mica = material;
    if (!material) {
        DWORD backdrop = DWMSBT_MAINWINDOW;
        mica = SUCCEEDED(DwmSetWindowAttribute(host, DWMWA_SYSTEMBACKDROP_TYPE, &backdrop,
                                               sizeof(backdrop)));
    }

    WUXC::Canvas root;
    // STRETCH, or the ground below paints nothing. A Canvas has no desired size of its own (0×0),
    // so as the island's content it arranges to nothing and its Background covers no pixels — the
    // island's default backdrop shows instead. That backdrop follows the SYSTEM scheme, which is
    // why the window looked right until an app-level Appearance override disagreed with it, and
    // then only the controls that paint themselves (menu bar, toolbar, nav pane) changed. The
    // nav's own content canvas already does this for the same reason.
    root.HorizontalAlignment(WUX::HorizontalAlignment::Stretch);
    root.VerticalAlignment(WUX::VerticalAlignment::Stretch);
    // Force DAY_THEME PER-ELEMENT (islands-safe, unlike Application::RequestedTheme): ElementTheme
    // on the root cascades to every descendant control + its {ThemeResource} lookups, so the whole
    // tree renders in the forced scheme. Unset => Default (follows the system).
    root.RequestedTheme(element_theme_now());
    source.Content(root);
    // Ground the island: a Canvas paints nothing itself, and the raw HWND behind it is white — under
    // a dark tree that white would ghost through. The named page-background brush resolves per the
    // SYSTEM theme, so it is only trustworthy when unforced; when DAY_THEME forces a scheme, ground
    // with a solid neutral matching it (the Fluent page-base color for that scheme).
    {
        // Mica accepted ⇒ leave the root transparent so the material shows through it — but ONLY
        // when the window is on the system's own scheme. DWM draws the material from the SYSTEM
        // theme and offers no way to ask it for the other one, so under a DAY_THEME force or an
        // Appearance override a transparent root shows the wrong scheme: every self-painting
        // control (nav pane, toolbar, menu bar, title bar) turns dark while the page area and the
        // docked chrome stay the system's white. `ground_root` refuses Mica for the same reason on
        // the re-theme path; this is the creation-time half, which alone runs for a window that
        // opens already forced (a DAY_THEME capture run never calls set_appearance at all).
        bool grounded = mica && !theme_overridden();
        if (!mica && !theme_overridden()) {
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
                color_argb(effective_dark() ? 0xFF'202020u : 0xFF'F3F3F3u)));
    }

    // Load the island now, before day builds the control tree. Controls added to a live,
    // loaded tree get their default styles/templates applied immediately, so day's first
    // (synchronous) Measure returns real sizes. Without this, templated controls measure to 0
    // and lay out invisible. Pump until the root's Loaded event fires (bounded).
    ShowWindow(host, SW_SHOWNORMAL);
    UpdateWindow(host);
    auto loaded = std::make_shared<bool>(false);
    auto token = root.Loaded([loaded](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
        *loaded = true;
    });
    // The filtered pump: day's own WM_APP+1 posts stay queued for the real loop, so a
    // closure that re-enters day-core cannot run from inside this call.
    pump_until([&] { return *loaded; }, 4000);
    root.Loaded(token);

    chrome_attach(host, island, root);
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
    {
        // Published under the post lock, with the posts that arrived early sent on first, so
        // nothing posted in between can overtake them.
        std::lock_guard<std::mutex> lock(g_post_mutex);
        g_app = aw;
        g_post_open = true;
        for (PostMsg* m : g_post_early)
            PostMessageW(aw->host, WM_DAY_POST, 0, reinterpret_cast<LPARAM>(m));
        g_post_early.clear();
    }
    // An overlay primary's safe-area inset, before day builds the content that reads it.
    report_overlay_inset(host, overlay_band_dip(host));
    return aw;
} catch (winrt::hresult_error const& e) {
    std::string msg = u8(e.message());
    std::fprintf(stderr, "day-toolkit-winui: XAML Islands init failed: hr=0x%08X %s\n",
                 static_cast<unsigned>(e.code().value), msg.c_str());
    std::fflush(stderr);
    return nullptr;
} catch (...) {
    std::fprintf(stderr, "day-toolkit-winui: XAML Islands init failed (unknown C++ exception)\n");
    std::fflush(stderr);
    return nullptr;
}

void* day_xaml_window_root(void* win) {
    auto app = reinterpret_cast<AppWindow*>(win);
    return boxh(app->content);
}

// The ROOT canvas, and its full extent — the whole client area, docked chrome included.
//
// Distinct from `day_xaml_window_root` above, which despite the name hands back the CONTENT
// canvas: the child that day's tree mounts into, offset below the menu bar and inset past the
// toolbar. Anything parented there is bounded by the page area. A fullscreen cover
// (docs/cover.md) is the one thing that must escape it — parented to the content canvas it
// leaves the menu bar, toolbar and nav pane visible AND clickable, which is not what "presented
// over the whole window" means, and lets the user navigate underneath a modal.
void* day_xaml_window_chrome_root(void* win) {
    auto app = reinterpret_cast<AppWindow*>(win);
    return app ? boxh(app->root) : nullptr;
}

// The root's laid-out size in DIPs. Read from the element rather than tracked alongside the
// resize callback, because that callback reports the CONTENT size (it is what day's layout
// needs) and the difference between the two is exactly the chrome a cover has to cover.
// A null `win` means the primary window, which is the same single-window convention
// `day_xaml_window_on_resize` already follows — the resize callback carries no handle to pass on.
void day_xaml_window_chrome_size(void* win, double* out_w, double* out_h) {
    auto app = win ? reinterpret_cast<AppWindow*>(win) : g_app;
    if (out_w) *out_w = (app && app->root) ? app->root.ActualWidth() : 0.0;
    if (out_h) *out_h = (app && app->root) ? app->root.ActualHeight() : 0.0;
}

void day_xaml_window_on_resize(void* win, void (*cb)(int, int)) {
    (void)win; // single window (v1)
    g_resize_cb = cb;
    // Report the size that is already current, rather than waiting for the next WM_SIZE. day
    // builds the app's root before it subscribes here, and building it installs the menu bar and
    // the toolbar — each of which reserves its strip and reports the client size that REMAINS
    // through this very callback, while it is still null. Those reports were dropped, nothing
    // recomputed until the user resized the window, and until then day laid out against the FULL
    // client height: its content box hung below the window bottom, so a vertically centered page
    // rendered low with its top cut off.
    if (g_app) day_xaml_relayout_chrome(g_app);
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
// Each window's toolbar (see `insert_toolbar_item`).
struct ToolbarEntry {
    std::string id;
    FrameworkElement elem{ nullptr }; // null for an item this bar does not draw
    int region = -1;                  // 0 content, 1 primary, 2 secondary, -1 not drawn
    int rank = 0; // folding rank on the right: 1 Automatic, 2 Primary, 0 neither (see fold_by_role)
};
struct ToolbarState {
    WUXC::CommandBar bar{ nullptr };
    WUXC::StackPanel lead{ nullptr };
    std::vector<ToolbarEntry> order; // every item, in Day's order
    // Each segmented item's toggle buttons, by id, so a patch can move the selection without
    // the echo. Per window, like everything here: two windows show the same ids.
    std::map<std::string, std::shared_ptr<std::vector<WUXCP::ToggleButton>>> segments;
    // A segmented item placed in the overflow is a menu instead (see insert_toolbar_item), kept
    // here for the same reason.
    std::map<std::string, std::shared_ptr<struct SegmentMenu>> segment_menus;
    // The search field's container, held at the right end of PrimaryCommands (see the `F` case).
    FrameworkElement search_end{ nullptr };
};
// A segmented item folded into the overflow: one command titled by the choice in force, whose
// flyout lists every choice as a checkable row. That is a pick-one in a Windows menu; a row of
// bare toggle buttons inside the overflow list is not.
struct SegmentMenu {
    WUXC::AppBarButton button{ nullptr };
    std::vector<WUXC::ToggleMenuFlyoutItem> rows;
    std::vector<std::string> titles;
    std::vector<std::pair<std::string, std::string>> icons; // glyph, image per choice
    // Show choice `n`: its row checked and the rest not, and its title and icon on the command.
    void show(size_t n);
};
static std::map<void*, ToolbarState> g_toolbar_state;

struct SecWindow {
    HWND host{};
    HWND island{};
    WUXH::DesktopWindowXamlSource source{ nullptr };
    WUXC::Canvas root{ nullptr };
    // Day's tree mounts into `content`, a child canvas offset below the docked chrome — the
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
/// Used for the menu commands that act on their window rather than the app.
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
    double s = dip_scale(sw->host); // points, as for the primary (day_xaml_relayout_chrome)
    if (g_win_resized)
        g_win_resized(sw->node, static_cast<int>(std::lround(w / s)),
                      static_cast<int>(std::lround(h / s)));
}

/// Decoded Day path segments, consumed by the Win2D path builder.
struct GeoSeg {
    char op; // 'L' (x y), 'Q' (cx cy x y) or 'C' (ax ay bx by x y)
    float v[6];
};
struct GeoFigure {
    float x = 0.0f, y = 0.0f;
    bool closed = false;
    std::vector<GeoSeg> segs;
};
struct GeoData {
    int rule = -1; // 1 even-odd, 0 nonzero, -1 left at XAML's default (a polygon names none)
    std::vector<GeoFigure> figs;
};

/// Parse "M x y L x y Q .. C .. Z" (day_spec::encode_path).
static GeoData parse_path_data(const std::string &spec, int rule) {
    GeoData out;
    out.rule = rule == 1 ? 1 : 0;
    std::vector<std::string> tok;
    size_t pos = 0;
    while (pos < spec.size()) {
        size_t sp = spec.find(' ', pos);
        std::string s = spec.substr(pos, sp == std::string::npos ? std::string::npos : sp - pos);
        if (!s.empty()) tok.push_back(s);
        if (sp == std::string::npos) break;
        pos = sp + 1;
    }
    size_t i = 0;
    auto num = [&]() -> float {
        if (i >= tok.size()) return 0.0f;
        float v = 0.0f;
        std::from_chars(tok[i].data(), tok[i].data() + tok[i].size(), v);
        ++i;
        return v;
    };
    GeoFigure *fig = nullptr;
    auto seg = [&](char op, int count) {
        GeoSeg s{ op, {} };
        for (int q = 0; q < count; ++q) s.v[q] = num();
        fig->segs.push_back(s);
    };
    while (i < tok.size()) {
        std::string op = tok[i++];
        if (op == "M") {
            out.figs.emplace_back();
            fig = &out.figs.back();
            fig->x = num();
            fig->y = num();
        } else if (op == "L" && fig) {
            seg('L', 2);
        } else if (op == "Q" && fig) {
            seg('Q', 4);
        } else if (op == "C" && fig) {
            seg('C', 6);
        } else if (op == "Z" && fig) {
            fig->closed = true;
        }
    }
    return out;
}

static LRESULT CALLBACK SecWndProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    auto it = g_sec_windows.find(hwnd);
    SecWindow* sw = it == g_sec_windows.end() ? nullptr : it->second;
    if (LRESULT r = 0; chrome_wndproc(hwnd, msg, wp, lp, &r)) return r;
    switch (msg) {
    case WM_SIZE:
        if (sw && sw->island) {
            RECT rc; GetClientRect(hwnd, &rc);
#ifdef DAY_WINUI
            winui_place_island(sw->source, hwnd);
#else
            SetWindowPos(sw->island, nullptr, 0, 0, rc.right, rc.bottom, SWP_SHOWWINDOW);
#endif
            chrome_on_size(hwnd);
            // Reports the size below the chrome, not the whole client, or day would lay its
            // tree out under the menu bar.
            relayout_sec_chrome(sw);
        }
        if (sw) report_size_state(hwnd, sw->node, wp);
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
                           unsigned long long node, int fixed, const DayXamlChrome* chrome) try {
    bool app_dark = effective_dark();
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
    // owner is passed as hWndParent without WS_CHILD, which is what makes it owned rather than
    // parented. A Normal window gets none of this — it is an independent main window.
    DWORD style = chrome_style(WS_OVERLAPPEDWINDOW, chrome);
    HWND owner = nullptr;
    if (fixed) {
        style &= ~(WS_THICKFRAME | WS_MAXIMIZEBOX | WS_MINIMIZEBOX);
        owner = g_app ? g_app->host : nullptr;
    }
    int outer_w = w, outer_h = h;
    chrome_outer_size(style, chrome ? chrome->chrome : 0, w, h, &outer_w, &outer_h);
    HWND host = CreateWindowExW(0, L"day_xaml_win2", hs(title).c_str(), style,
                                CW_USEDEFAULT, CW_USEDEFAULT, outer_w, outer_h, owner, nullptr,
                                GetModuleHandleW(nullptr), nullptr);
    if (!host) return nullptr;
    // The chrome (docs/window-chrome.md), and where the window opens, before it is first shown.
    // A panel never takes a frameless resizable edge: `fixed` already took its sizing border.
    DayXamlChrome opt = chrome ? *chrome : DayXamlChrome{ 0, 0, 0, 1, 1, 0, 0.0, 0.0 };
    if (fixed) opt.resizable = 0;
    bool material = chrome_init(host, node, &opt);
    chrome_place(host, &opt);

    WUXH::DesktopWindowXamlSource source;
#ifdef DAY_WINUI
    HWND island = winui_attach_island(source, host);
    wrap_tab_focus(source);
#else
    auto interop = source.as<::IDesktopWindowXamlSourceNative>();
    interop->AttachToWindow(host);
    wrap_tab_focus(source);
    HWND island = nullptr;
    interop->get_WindowHandle(&island);
    RECT rc; GetClientRect(host, &rc);
    SetWindowPos(island, nullptr, 0, 0, rc.right, rc.bottom, SWP_SHOWWINDOW);
#endif
    apply_dark_titlebar(host); // re-applied on WM_SETTINGCHANGE, as for the primary
    WUXC::Canvas root;
    // Stretch, so the ground below actually covers the window — see the primary window's root.
    root.HorizontalAlignment(WUX::HorizontalAlignment::Stretch);
    root.VerticalAlignment(WUX::VerticalAlignment::Stretch);
    // The scheme in force, which is the env force AND the app's own Appearance pick — a window
    // opened from File ▸ New Window while the app is set to Dark used to consult only the former
    // and open light, next to the dark window that spawned it.
    root.RequestedTheme(element_theme_now());
    source.Content(root);
    // Solid neutral ground matching the scheme (the primary's themed-brush path needs the
    // unforced system lookup; a solid is correct in both cases and keeps this path simple) —
    // except under an accepted material, which shows only through a transparent root, and only
    // on the system's own scheme (see ground_root).
    if (!material || theme_overridden())
        root.Background(WUXM::SolidColorBrush(
            color_argb(effective_dark() ? 0xFF'202020u : 0xFF'F3F3F3u)));

    // Load the island before day builds (see day_xaml_window_new: unloaded templated
    // controls measure to 0). Bounded pump.
    ShowWindow(host, SW_SHOWNORMAL);
    UpdateWindow(host);
    // ACTIVATE it, which ShowWindow alone does not reliably do for a window created by a
    // background thread's request. Without this the new window appears while the window that
    // opened it keeps focus — so the next menu command goes to the old window. `File ▸ New
    // Window` then `File ▸ Close` read as "close the window I just opened" but closed the
    // PRIMARY, and closing the primary ends the app (above), taking the new window with it.
    SetForegroundWindow(host);
    auto loaded = std::make_shared<bool>(false);
    auto token = root.Loaded([loaded](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
        *loaded = true;
    });
    // This runs inside day-core's `with_tree` borrow (`Toolkit::open_window`), and the
    // filtered pump matters here: a day post (WM_APP+1) dispatched from this nested loop
    // runs a reactive flush, a timer or an eval reply that re-enters `with_tree` on the
    // borrowed tree ("RefCell already borrowed"). Seen opening Browser Settings in the
    // web view demo while a second tab's poll kept posts in flight.
    pump_until([&] { return *loaded; }, 2000);
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
    chrome_attach(host, island, root);
    // An overlay window's first safe-area report, from inside `open_window`, before day builds
    // the content that reads it: nothing else lays its chrome out before the app installs a menu
    // bar or the user resizes it.
    report_overlay_inset(host, overlay_band_dip(host));
    return sw;
} catch (...) {
    std::fprintf(stderr, "day-toolkit-winui: secondary window init failed\n");
    std::fflush(stderr);
    return nullptr;
}

void* day_xaml_window_content2(void* win) {
    return boxh(static_cast<SecWindow*>(win)->content);
}
void day_xaml_window_close2(void* win) {
    PostMessageW(static_cast<SecWindow*>(win)->host, WM_CLOSE, 0, 0);
}
// Bring a window to the front: the primary's AppWindow or a secondary's SecWindow. A minimized
// window is restored; a maximized one stays maximized (SW_SHOWNORMAL would un-zoom it).
void day_xaml_window_raise2(void* win) {
    if (!win) return;
    HWND h = win == static_cast<void*>(g_app) ? g_app->host : static_cast<SecWindow*>(win)->host;
    if (!h) return;
    ShowWindow(h, IsIconic(h) ? SW_RESTORE : SW_SHOW);
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
    g_toolbar_state.erase(win);
    // Close the island before the host window goes. The DesktopWindowXamlSource owns a child
    // HWND of `host`, so DestroyWindow takes that window out from under XAML and leaves the
    // source to tear down an island whose HWND no longer exists — Close() first is the order
    // the API documents. This is the only teardown that runs while the process keeps living:
    // the primary reaches its own destroy on the way out, where nothing outlives the mistake.
    if (sw->source) {
        try {
            sw->source.Close();
        } catch (...) {
        }
    }
    g_show_state.erase(sw->host);
    g_chrome.erase(sw->host);
    DestroyWindow(sw->host);
    delete sw;
}

// --- window properties (docs/windows.md "Window properties") --------------------------------
// `win` is what Rust holds for a window: the primary's AppWindow or a secondary's SecWindow.

static HWND host_of_window(void* win) {
    if (!win) return nullptr;
    if (win == static_cast<void*>(g_app)) return g_app->host;
    return static_cast<SecWindow*>(win)->host;
}

static unsigned long long node_of_window(void* win) {
    if (win == static_cast<void*>(g_app)) return kPrimaryNode;
    return static_cast<SecWindow*>(win)->node;
}

// Borderless fullscreen, the standard Win32 way: keep the placement and styles, swap the frame
// for WS_POPUP (no caption, no sizing border, no maximize box for Win+Up to act on), and cover the
// whole monitor the window is on, taskbar included. WS_SYSMENU and WS_MINIMIZEBOX stay: they draw
// nothing without a caption, and they are what lets the taskbar button minimize the window and
// open its menu. A maximized or minimized window is restored first, with the placement read
// before that, so leaving fullscreen goes back to its normal rect.
static void enter_fullscreen(HWND hwnd, WinShowState& st) {
    if (st.fullscreen) return;
    MONITORINFO mi{ sizeof(MONITORINFO) };
    if (!GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mi)) return;
    st.placement.length = sizeof(WINDOWPLACEMENT);
    if (!GetWindowPlacement(hwnd, &st.placement)) return;
    // Twice for a window minimized from maximized: the first restore brings it back maximized.
    if (IsIconic(hwnd)) ShowWindow(hwnd, SW_RESTORE);
    if (IsZoomed(hwnd)) ShowWindow(hwnd, SW_RESTORE);
    st.style = GetWindowLongPtrW(hwnd, GWL_STYLE);
    st.ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    st.fullscreen = true; // before the resize below, whose WM_SIZE reads it
    SetWindowLongPtrW(hwnd, GWL_STYLE,
                      (st.style & ~(WS_CAPTION | WS_THICKFRAME | WS_MAXIMIZEBOX)) | WS_POPUP);
    SetWindowLongPtrW(hwnd, GWL_EXSTYLE,
                      st.ex_style & ~(WS_EX_DLGMODALFRAME | WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE |
                                      WS_EX_STATICEDGE));
    RECT r = mi.rcMonitor;
    SetWindowPos(hwnd, HWND_TOP, r.left, r.top, r.right - r.left, r.bottom - r.top,
                 SWP_NOOWNERZORDER | SWP_FRAMECHANGED | SWP_SHOWWINDOW);
}

// Back to the styles and the normal placement the window had before fullscreen.
static void leave_fullscreen(HWND hwnd, WinShowState& st) {
    if (!st.fullscreen) return;
    if (IsIconic(hwnd)) ShowWindow(hwnd, SW_RESTORE);
    st.fullscreen = false; // before the resize below, whose WM_SIZE reads it
    SetWindowLongPtrW(hwnd, GWL_STYLE, st.style);
    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, st.ex_style);
    WINDOWPLACEMENT wp = st.placement;
    wp.showCmd = SW_SHOWNORMAL;
    wp.flags &= ~static_cast<UINT>(WPF_RESTORETOMAXIMIZED);
    SetWindowPlacement(hwnd, &wp);
    SetWindowPos(hwnd, nullptr, 0, 0, 0, 0,
                 SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_FRAMECHANGED);
}

void day_xaml_set_window_state_cb(void (*cb)(unsigned long long, int)) { g_win_state_cb = cb; }

// Ask for `state` (a day_spec::WindowState code), leaving whatever the window is in first, then
// report what it reads as. ShowWindow and SetWindowPos act before they return, so the answer is
// already the platform's. The WM_SIZE messages on the way are held back (g_state_request) and one report
// goes out at the end: on a change, or always when the window did not reach `state`, so a request
// that could not land is answered with the state the window is still in.
void day_xaml_window_set_state(void* win, int state) try {
    HWND hwnd = host_of_window(win);
    if (!hwnd || state < 0 || state > 3) return;
    auto& st = g_show_state[hwnd];
    g_state_request = true;
    switch (state) {
    case 0: // normal
        if (IsIconic(hwnd)) ShowWindow(hwnd, SW_RESTORE);
        leave_fullscreen(hwnd, st);
        if (IsZoomed(hwnd)) ShowWindow(hwnd, SW_RESTORE);
        break;
    case 1: // minimized; a fullscreen window comes back fullscreen
        if (!IsIconic(hwnd)) ShowWindow(hwnd, SW_MINIMIZE);
        break;
    case 2: // maximized
        if (IsIconic(hwnd)) ShowWindow(hwnd, SW_RESTORE);
        leave_fullscreen(hwnd, st);
        if (!IsZoomed(hwnd)) ShowWindow(hwnd, SW_MAXIMIZE);
        break;
    case 3: // fullscreen; a minimized fullscreen window only needs bringing back
        if (st.fullscreen) {
            if (IsIconic(hwnd)) ShowWindow(hwnd, SW_RESTORE);
        } else {
            enter_fullscreen(hwnd, st);
        }
        break;
    }
    g_state_request = false;
    int now = window_state_now(hwnd);
    report_window_state(hwnd, node_of_window(win), now, now != state);
} catch (...) {
    g_state_request = false;
}

// Keep the window out of screenshots, recordings and screen sharing. WDA_EXCLUDEFROMCAPTURE
// (Windows 10 2004+) leaves the window out of a capture altogether; an older Windows refuses it,
// and WDA_MONITOR, which shows the window black in a capture, is the closest it has.
void day_xaml_window_set_protected(void* win, int on) try {
    HWND hwnd = host_of_window(win);
    if (!hwnd) return;
    if (!on) {
        SetWindowDisplayAffinity(hwnd, WDA_NONE);
    } else if (!SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)) {
        SetWindowDisplayAffinity(hwnd, WDA_MONITOR);
    }
} catch (...) {
}

// --- window chrome exports (docs/window-chrome.md) --------------------------------------------

void day_xaml_set_chrome_inset_cb(void (*cb)(unsigned long long, double)) { g_chrome_inset_cb = cb; }

// Whether this Windows draws the system backdrops (Mica, Mica Alt, Acrylic): Windows 11 22621+.
int day_xaml_materials_supported() { return windows_build() >= 22621 ? 1 : 0; }

// Mark (`on`) or unmark an element as a drag region (`.window_drag_region()`). Nothing is attached
// to the element: the window's hit test finds it (chrome_hit_test → in_drag_region), and the
// island's windows are hooked so that test gets to run.
void day_xaml_set_drag_region(void* h, int on) try {
    UIElement e = elem(h);
    g_drag_regions.erase(std::remove(g_drag_regions.begin(), g_drag_regions.end(), e),
                         g_drag_regions.end());
    if (!on) return;
    g_drag_regions.push_back(e);
    for (auto const& entry : g_chrome) chrome_hook_island(entry.first);
} catch (...) {
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
    if (small_) g_app_icon_small = small_; // a status item without a glyph shows it
}

// Top-level host HWND, for a piece that needs the window handle behind the XAML island — the WebView2
// web view passes it as the composition controller's parentWindow (DPI / IME / input association),
// while the page renders windowless into a composition visual spliced into the XAML tree. Single
// window (v1), via g_app.
void* day_xaml_host_hwnd() { return g_app ? reinterpret_cast<void*>(g_app->host) : nullptr; }

void day_xaml_run(void* win) {
    auto app = reinterpret_cast<AppWindow*>(win);
#ifndef DAY_WINUI
    auto interop2 = app->source.as<::IDesktopWindowXamlSourceNative2>();
#else
    (void)app;
#endif
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
#ifdef DAY_WINUI
        if (!handled) handled = winui_pretranslate(&msg);
#else
        if (!handled && interop2) interop2->PreTranslateMessage(&msg, &handled);
#endif
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
    std::lock_guard<std::mutex> lock(g_post_mutex);
    if (g_app && g_app->host) {
        PostMessageW(g_app->host, WM_DAY_POST, 0, reinterpret_cast<LPARAM>(new PostMsg{ cb, data }));
    } else if (!g_post_open) {
        g_post_early.push_back(new PostMsg{ cb, data }); // delivered once the window exists
    } else if (g_shell_hwnd) {
        // The primary window is gone but the app runs on (a status item, keep_running): the
        // shell window takes the post.
        PostMessageW(g_shell_hwnd, WM_DAY_POST, 0, reinterpret_cast<LPARAM>(new PostMsg{ cb, data }));
    }
    // Otherwise the app is ending; the post has nowhere to run.
}

// The timer door behind day::sleep (docs/async.md). A thread-less Win32 timer belongs to the
// queue of the thread that armed it, so the arming itself rides day_xaml_post to the UI thread;
// WM_TIMER then runs the callback from the message pump. One-shot: the timer dies as it fires.
struct DelayedPost {
    void (*cb)(void*);
    void* data;
    unsigned ms;
    std::chrono::steady_clock::time_point deadline;
};
static std::map<UINT_PTR, DelayedPost*> g_delayed; // UI thread only
static void CALLBACK day_xaml_delayed_fire(HWND, UINT, UINT_PTR id, DWORD) {
    auto it = g_delayed.find(id);
    if (it == g_delayed.end()) return;
    DelayedPost* p = it->second;
    auto now = std::chrono::steady_clock::now();
    if (now < p->deadline) {
        // WM_TIMER has system-clock granularity and can arrive before the requested delay.
        // Keep the callback pending and reset the timer for the remaining time, rounded up.
        // If resetting fails, the existing periodic timer still gives us another turn.
        auto remaining = std::chrono::ceil<std::chrono::milliseconds>(p->deadline - now);
        SetTimer(nullptr, id, static_cast<UINT>(remaining.count()), day_xaml_delayed_fire);
        return;
    }
    KillTimer(nullptr, id);
    g_delayed.erase(it);
    p->cb(p->data);
    delete p;
}
void day_xaml_post_delayed(unsigned ms, void (*cb)(void*), void* data) {
    auto* p = new DelayedPost{
        cb, data, ms, std::chrono::steady_clock::now() + std::chrono::milliseconds(ms)
    };
    day_xaml_post([](void* raw) {
        auto* p = static_cast<DelayedPost*>(raw);
        UINT_PTR id = SetTimer(nullptr, 0, p->ms, day_xaml_delayed_fire);
        if (!id) { // no timer to be had: now beats never
            p->cb(p->data);
            delete p;
            return;
        }
        g_delayed[id] = p;
    }, p);
}

// One-shot CompositionTarget callbacks on the XAML UI thread. Revoke before entering Rust,
// so a continuing client may subscribe to the next frame, and cancellation cannot double-revoke.
static std::map<unsigned long long, winrt::event_token> g_frame_requests;
void day_xaml_request_frame(unsigned long long id, void (*cb)(unsigned long long, double)) {
    auto token = WUXM::CompositionTarget::Rendering([id, cb](WF::IInspectable const&, WF::IInspectable const& args) {
        auto it = g_frame_requests.find(id);
        if (it == g_frame_requests.end()) return;
        auto registration = it->second;
        g_frame_requests.erase(it);
        WUXM::CompositionTarget::Rendering(registration);
        auto timing = args.as<WUXM::RenderingEventArgs>().RenderingTime();
        cb(id, timing.count() / 10000000.0);
    });
    g_frame_requests.emplace(id, token);
}
void day_xaml_cancel_frame(unsigned long long id) {
    auto it = g_frame_requests.find(id);
    if (it == g_frame_requests.end()) return;
    auto token = it->second;
    g_frame_requests.erase(it);
    WUXM::CompositionTarget::Rendering(token);
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
    sv.VerticalContentAlignment(WUX::VerticalAlignment::Top);
    WUXC::Canvas content;
    content.VerticalAlignment(WUX::VerticalAlignment::Top);
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
    // KNOWN GAP — a screenshot taken in the SAME dayscript request as this scroll, with no step
    // between, captures the PREVIOUS scroll position (the walkthrough's `controls-pickers` shot
    // is the standing example). A ScrollViewer moves its content with a compositor
    // (DirectManipulation) transform and folds it into layout only when the view change commits;
    // `PrintWindow` re-renders the tree without that transform, so the capture tears — scrollbar
    // at the new offset, content at the old one.
    //
    // Three in-call fixes were tried here and NONE work, so do not reach for them again: the
    // snapshot's own present-barrier (`pump_until_presented`), pumping until `VerticalOffset`
    // reaches the target, and pumping until `ViewChanged` reports `IsIntermediate() == false`.
    // What does work is any step that returns to the engine first (a bare `wait_idle`), or
    // capturing in a separate `day drive` call — i.e. the commit needs a turn of the app's REAL
    // message loop, which a pump nested inside this request cannot give it: these pumps run
    // inside a day-core `with_tree` borrow and must exclude day's own WM_APP+1 post to avoid
    // re-entering it. Closing this needs the capture to yield to the loop and resume, not
    // another wait in here.
}
// The day name for a key the route carries, or null for every other key (docs/menus.md).
// `mods` is the day KeyEvent mask already read for this press. A digit is the PHYSICAL key,
// main row or keypad (a virtual key does not say what the layout types), and never under Ctrl
// or Alt: those belong to accelerators, and a canvas claiming one would starve the menu.
static const char* day_xaml_key_name(winrt::Windows::System::VirtualKey k, int mods) {
    using VK = winrt::Windows::System::VirtualKey;
    static const char* const digits[] = {"0", "1", "2", "3", "4", "5", "6", "7", "8", "9"};
    switch (k) {
        case VK::Left:  return "ArrowLeft";
        case VK::Right: return "ArrowRight";
        case VK::Up:    return "ArrowUp";
        case VK::Down:  return "ArrowDown";
        default: break;
    }
    if (mods & (2 | 4)) return nullptr;
    if (k >= VK::Number0 && k <= VK::Number9)
        return digits[static_cast<int>(k) - static_cast<int>(VK::Number0)];
    if (k >= VK::NumberPad0 && k <= VK::NumberPad9)
        return digits[static_cast<int>(k) - static_cast<int>(VK::NumberPad0)];
    return nullptr;
}

// The WinUI canvas uses Win2D for both screen drawing and PNG export.
// The deprecated system-XAML target deliberately has no canvas renderer.
static std::map<uint64_t, std::vector<uint8_t>>& xaml_bitmaps();
#ifdef DAY_WINUI
#include "canvas-winui.inc"
#else
void* day_xaml_canvas_new(unsigned long long, int (*)(unsigned long long),
                          void (*)(unsigned long long, const char*, int)) {
    std::fprintf(stderr, "day-toolkit-winui: canvas is unsupported on deprecated windows-xaml\n");
    return day_xaml_container_new();
}
void day_xaml_canvas_set_ops(void*, const double*, int, const char*) {}
#endif

// Recycling-list host: a real ScrollViewer whose Content is a Canvas that holds the row cells
// (day positions each cell by absolute frame). `out_content` receives a handle to that Canvas so
// the Rust side can add/position cells; the list drives scrolling via the content's extent.
void* day_xaml_list_new(void** out_content) {
    WUXC::ScrollViewer sv;
    sv.HorizontalScrollBarVisibility(WUXC::ScrollBarVisibility::Disabled);
    sv.VerticalScrollBarVisibility(WUXC::ScrollBarVisibility::Auto);
    sv.VerticalContentAlignment(WUX::VerticalAlignment::Top);
    WUXC::Canvas content;
    content.VerticalAlignment(WUX::VerticalAlignment::Top);
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

// What the list is actually SHOWING: the scrolled offset and the visible height. The emulated
// list positions every row itself, so this is the only way it can know which handful of a
// ten-thousand-row source needs building — the content Canvas is the full extent either way.
void day_xaml_list_viewport(void* sv, double* out_offset, double* out_height) {
    *out_offset = 0;
    *out_height = 0;
    if (auto s = elem(sv).try_as<WUXC::ScrollViewer>()) {
        *out_offset = s.VerticalOffset();
        *out_height = s.ViewportHeight();
    }
}

// Report scrolling, so rows coming into view get built before they are looked at. Fires for
// intermediate (mid-inertia) states too, which is what keeps a flick from trailing empty rows;
// Rust coalesces the run to one per loop turn.
void day_xaml_list_on_scroll(void* sv, uint64_t node, void (*cb)(uint64_t)) {
    auto s = elem(sv).try_as<WUXC::ScrollViewer>();
    if (!s) return;
    s.ViewChanged([node, cb](WF::IInspectable const&,
                             WUXC::ScrollViewerViewChangedEventArgs const&) { cb(node); });
}

// Leave focus and activation to controls embedded in a row.
static bool list_embedded_control(WF::IInspectable const& source, WUX::DependencyObject const& host) {
    auto current = source.try_as<WUX::DependencyObject>();
    while (current && current != host) {
        if (current.try_as<WUXC::Control>()) return true;
        current = WUXM::VisualTreeHelper::GetParent(current);
    }
    return false;
}

void day_xaml_list_keynav(void* sv, uint64_t node, int (*cb)(uint64_t, int)) {
    auto scroll = elem(sv).try_as<WUXC::ScrollViewer>();
    if (!scroll) return;
    scroll.IsTabStop(true);
    scroll.PointerPressed([sv](WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& event) {
        auto host = elem(sv).as<WUXC::ScrollViewer>();
        if (!list_embedded_control(event.OriginalSource(), host)) host.Focus(WUX::FocusState::Pointer);
    });
    scroll.KeyDown([sv, node, cb](WF::IInspectable const&, WUXIn::KeyRoutedEventArgs const& event) {
        if (event.Key() != WS::VirtualKey::Enter) return;
        if (event.OriginalSource() != elem(sv)) return;
        if (cb(node, 4)) event.Handled(true);
    });
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
    canvas.DoubleTapped([cell, id, row, cb](WF::IInspectable const&, WUXIn::DoubleTappedRoutedEventArgs const& event) {
        if (!list_embedded_control(event.OriginalSource(), elem(cell))) cb(id, row, 4);
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
#ifdef DAY_WINUI
    // WinUI 3's ListView selection, which is what a Windows 11 list looks like: a subtle neutral
    // fill with rounded corners and a short accent pill at the leading edge, not an accent-tinted
    // band. The fill is the translucent SubtleFillColorSecondary value for the scheme in force
    // (alpha-over-ground, so it needs no resource lookup that could answer for the wrong scheme);
    // the pill is the system accent color, which does not depend on the scheme.
    WUXSh::Rectangle pill{nullptr};
    {
        auto kids = canvas.Children();
        if (kids.Size() > 1)
            if (auto r = kids.GetAt(1).try_as<WUXSh::Rectangle>())
                if (r.Name() == L"day_pill") pill = r;
    }
    if (!on) {
        rect.Fill(WUXM::SolidColorBrush(color_argb(0x00'000000u))); // stays hit-testable
        if (pill) pill.Visibility(WUX::Visibility::Collapsed);
        return;
    }
    rect.Fill(WUXM::SolidColorBrush(color_argb(effective_dark() ? 0x0F'FFFFFFu : 0x09'000000u)));
    rect.RadiusX(4);
    rect.RadiusY(4);
    if (!pill) {
        pill = WUXSh::Rectangle();
        pill.Name(L"day_pill");
        pill.Width(3);
        pill.Height(16);
        pill.RadiusX(1.5);
        pill.RadiusY(1.5);
        pill.IsHitTestVisible(false);
        canvas.Children().InsertAt(1, pill);
        canvas.SizeChanged([pill](WF::IInspectable const&, WUX::SizeChangedEventArgs const& e) {
            WUXC::Canvas::SetTop(pill, (std::max)(0.0, (static_cast<double>(e.NewSize().Height) - 16) / 2));
        });
    }
    {
        winrt::Windows::UI::ViewManagement::UISettings ui;
        auto a = ui.GetColorValue(winrt::Windows::UI::ViewManagement::UIColorType::Accent);
        pill.Fill(WUXM::SolidColorBrush(a));
    }
    WUXC::Canvas::SetLeft(pill, 0);
    WUXC::Canvas::SetTop(pill, (std::max)(0.0, (canvas.ActualHeight() - 16) / 2));
    pill.Visibility(WUX::Visibility::Visible);
    return;
#endif
    if (!on) {
        // Transparent, not null: the cell must stay hit-testable so the next press still lands.
        rect.Fill(WUXM::SolidColorBrush(color_argb(0x00'000000u)));
        return;
    }
    // An app Appearance override counts as much as a DAY_THEME force: both re-theme per-element,
    // so the app resources still answer for the SYSTEM scheme (see day_xaml_container_set_card).
    if (g_forced_theme == 0 && g_app_override == 0) {
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
//
// Section titles (docs/navigation.md) are disabled ListViewItems ahead of their group's first row,
// so the user cannot pick one. The rows themselves stay boxed
// strings, so a ListViewItem in Items() is always a title, and the two helpers below turn an
// Items() position into Day's row index and back by counting past them (-1 = none).
static int navlist_row_of(WUXC::ListView const& lv, int at) {
    auto items = lv.Items();
    if (at < 0 || static_cast<uint32_t>(at) >= items.Size()) return -1;
    if (items.GetAt(static_cast<uint32_t>(at)).try_as<WUXC::ListViewItem>()) return -1;
    int row = 0;
    for (uint32_t i = 0; i < static_cast<uint32_t>(at); ++i)
        if (!items.GetAt(i).try_as<WUXC::ListViewItem>()) ++row;
    return row;
}
static int navlist_index_of(WUXC::ListView const& lv, int row) {
    if (row < 0) return -1;
    auto items = lv.Items();
    int seen = 0;
    for (uint32_t i = 0; i < items.Size(); ++i) {
        if (items.GetAt(i).try_as<WUXC::ListViewItem>()) continue;
        if (seen == row) return static_cast<int>(i);
        ++seen;
    }
    return -1;
}
void* day_xaml_navlist_new(unsigned long long id, void (*cb)(unsigned long long, int)) {
    WUXC::ListView lv;
    lv.SelectionMode(WUXC::ListViewSelectionMode::Single);
    lv.SelectionChanged([id, cb](WF::IInspectable const& s, WUXC::SelectionChangedEventArgs const&) {
        auto list = s.as<WUXC::ListView>();
        cb(id, navlist_row_of(list, list.SelectedIndex()));
    });
    return boxh(lv);
}
static std::vector<std::string> split_lines(const char* joined);
void day_xaml_navlist_set_items(void* w, const char* items_joined, const char* sections_joined) {
    auto lv = elem(w).try_as<WUXC::ListView>();
    if (!lv) return;
    lv.Items().Clear();
    auto titles = split_lines(items_joined);
    auto sections = split_lines(sections_joined);
    for (size_t i = 0; i < titles.size(); ++i) {
        if (i < sections.size() && !sections[i].empty()) {
            WUXC::TextBlock label;
            label.Text(hs(sections[i].c_str()));
            label.FontSize(12.0);
            label.FontWeight(WUTextNames::FontWeights::SemiBold());
            WUXC::ListViewItem heading;
            heading.Content(label);
            heading.IsEnabled(false);
            heading.MinHeight(32.0);
            heading.VerticalContentAlignment(WUX::VerticalAlignment::Bottom);
            lv.Items().Append(heading);
        }
        lv.Items().Append(winrt::box_value(hs(titles[i].c_str())));
    }
}
void day_xaml_navlist_set_selected(void* w, int idx) {
    auto lv = elem(w).try_as<WUXC::ListView>();
    if (!lv) return;
    int at = navlist_index_of(lv, idx);
    if (lv.SelectedIndex() != at) lv.SelectedIndex(at);
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
// — so the primary window's toolbar button collapsed the second window's sidebar and left its
// own alone. Each window is its own XAML island with its own XamlRoot, which is what tells them
// apart at click time.
static std::vector<WUXC::NavigationView> g_navviews;

// Show/hide one nav's pane. In PaneDisplayMode::Left the pane is always expanded and IsPaneOpen
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

// The window-less entry point (day-toolkit-winui's `toggle_sidebar` duty, and dayscript's step): no click
// to locate a window from, so it drives the primary window's nav — the first one still alive,
// which is the one the app opened with.
/// The ElementTheme every live root should carry right now.
static WUX::ElementTheme element_theme_now() {
    int mode = g_forced_theme ? g_forced_theme : g_app_override;
    switch (mode) {
        case 2: return WUX::ElementTheme::Dark;
        case 1: return WUX::ElementTheme::Light;
        default: return WUX::ElementTheme::Default; // follow the system
    }
}

/// Re-theme every window in place — the `set_appearance` duty (docs/settings.md).
///
/// Per-element rather than `Application::RequestedTheme`, which is unsupported under XAML Islands:
/// ElementTheme on a root cascades to every descendant control and its {ThemeResource} lookups, so
/// the whole tree re-renders in the chosen scheme. Secondary windows are separate islands with
/// their own roots, so each is set individually — a preferences window that restyled only itself
/// (or only the main window) is exactly the half-applied result this avoids.
/// Re-ground one root for the scheme now in force.
///
/// The GROUND is the half `RequestedTheme` cannot reach. A Canvas paints nothing of its own and the
/// HWND behind the island is white, so each root is given a background at creation — either the
/// system's `ApplicationPageBackgroundThemeBrush` or a solid neutral — computed from the scheme AT
/// THAT MOMENT and never revisited. Re-theming without this left every control correctly dark on a
/// still-light ground: the menu bar drew white-on-white and the page area stayed white, while the
/// toolbar and nav (which paint their own backgrounds) looked right. That mismatch IS the bug.
///
/// Mica is the complication. When the backdrop is accepted the root is deliberately left
/// TRANSPARENT so the material shows through — and that material is drawn by DWM from the SYSTEM
/// scheme, with no way to ask it for the other one. So under an override the backdrop has to be
/// covered by an opaque ground; without that, every surface that does not paint itself kept the
/// system's color while the controls went dark, which is the bug this whole chain was chasing.
/// Following the system again puts the transparency back, so Mica returns.
static void ground_root(WUXC::Canvas const& root, bool dark) {
    if (!root) return;
    if (theme_overridden()) {
        root.Background(WUXM::SolidColorBrush(color_argb(dark ? 0xFF'202020u : 0xFF'F3F3F3u)));
    } else if (root.Background()) {
        // Already grounded (Mica refused at creation): keep it opaque, just re-color it.
        root.Background(WUXM::SolidColorBrush(color_argb(dark ? 0xFF'202020u : 0xFF'F3F3F3u)));
    } else {
        root.Background(nullptr); // Mica window back on the system scheme: let the material show
    }
}

/// One window's scheme from its own override (WindowChange::Appearance, docs/windows.md), or the
/// app's when it has none: the title bar, the root's RequestedTheme, and the ground, which goes
/// opaque under an override for the same reason ground_root gives (DWM's material follows the
/// system scheme only).
static void apply_window_appearance(HWND h) {
    auto it = g_chrome.find(h);
    if (it == g_chrome.end()) return;
    WinChrome& c = it->second;
    apply_dark_titlebar(h);
    if (!c.root) return;
    if (c.appearance < 0) {
        c.root.RequestedTheme(element_theme_now());
        ground_root(c.root, effective_dark());
        return;
    }
    bool dark = c.appearance == 1;
    c.root.RequestedTheme(dark ? WUX::ElementTheme::Dark : WUX::ElementTheme::Light);
    c.root.Background(WUXM::SolidColorBrush(color_argb(dark ? 0xFF'202020u : 0xFF'F3F3F3u)));
}

static void apply_appearance_everywhere() {
    auto theme = element_theme_now();
    bool dark = effective_dark();
    if (g_app) {
        if (g_app->root) {
            g_app->root.RequestedTheme(theme);
            ground_root(g_app->root, dark);
        }
        if (g_app->host) apply_dark_titlebar(g_app->host);
    }
    for (auto const& [hwnd, sw] : g_sec_windows) {
        if (!sw) continue;
        if (sw->root) {
            sw->root.RequestedTheme(theme);
            ground_root(sw->root, dark);
        }
        if (sw->host) apply_dark_titlebar(sw->host);
        (void)hwnd;
    }
    // Windows with a scheme of their own keep it through the app-wide change.
    for (auto const& entry : g_chrome) {
        if (entry.second.appearance >= 0) apply_window_appearance(entry.first);
    }
}

/// `mode`: 0 follow the system, 1 light, 2 dark. Deliberately does NOT fire `g_appearance_cb` —
/// day-core calls `note_appearance_changed()` itself once its tree borrow has ended, and firing
/// from in here would land inside that borrow and be skipped.
extern "C" void day_xaml_set_appearance(int mode) try {
    g_app_override = (mode == 1 || mode == 2) ? mode : 0;
    apply_appearance_everywhere();
} catch (...) {
}

extern "C" int day_xaml_toggle_sidebar() try {
    prune_navviews();
    if (g_navviews.empty()) return 0;
    return toggle_nav_pane(g_navviews.front()) ? 1 : 0;
} catch (...) {
    return 0;
}

// Section titles (docs/navigation.md) are NavigationViewItemHeaders interleaved with the
// destinations in MenuItems, so a MenuItems position is not Day's row index. These two walk the
// pane counting only NavigationViewItems; -1 means "not a destination".
static int nav_row_of(WUXC::NavigationView const& nv, WF::IInspectable const& item) {
    if (!item) return -1;
    int row = 0;
    for (auto const& it : nv.MenuItems()) {
        if (!it.try_as<WUXC::NavigationViewItem>()) continue;
        if (it == item) return row;
        ++row;
    }
    return -1;
}
static int nav_menu_index(WUXC::NavigationView const& nv, int row) {
    if (row < 0) return -1;
    auto items = nv.MenuItems();
    int seen = 0;
    for (uint32_t i = 0; i < items.Size(); ++i) {
        if (!items.GetAt(i).try_as<WUXC::NavigationViewItem>()) continue;
        if (seen == row) return static_cast<int>(i);
        ++seen;
    }
    return -1;
}

// A tab strip (Top) and an icon rail (LeftCompact) present the destinations flat, so their
// section titles collapse there and come back with the sidebar forms.
static void nav_sync_headers(WUXC::NavigationView const& nv) {
    auto mode = nv.PaneDisplayMode();
    bool flat = mode == WUXC::NavigationViewPaneDisplayMode::Top ||
                mode == WUXC::NavigationViewPaneDisplayMode::LeftCompact;
    auto shown = flat ? WUX::Visibility::Collapsed : WUX::Visibility::Visible;
    for (auto const& it : nv.MenuItems())
        if (auto header = it.try_as<WUXC::NavigationViewItemHeader>()) header.Visibility(shown);
}

// The pane's FORM: 0 Left (split), 1 LeftMinimal (stack), 2 Top (tabs), 3 LeftCompact (rail).
//
// One NavigationView wears all four — which is why a re-present here is a property write rather
// than a rebuild. `Top` is WinUI's own answer to a tab bar (the destinations run along the top of
// the window), and `LeftCompact` is a real icon rail, so `NavPresentation::Rail` lands on a rail
// here rather than rounding to a sidebar the way it must on macOS (docs/navigation.md).
void day_xaml_nav_set_pane_mode(void* navh, int mode) {
    guard([&] {
    auto nv = elem(navh).try_as<WUXC::NavigationView>();
    if (!nv) return;
    switch (mode) {
        case 1:
            nv.PaneDisplayMode(WUXC::NavigationViewPaneDisplayMode::LeftMinimal);
            nv.IsPaneToggleButtonVisible(false);
            nv.IsPaneOpen(false);
            break;
        case 2:
            nv.PaneDisplayMode(WUXC::NavigationViewPaneDisplayMode::Top);
            nv.IsPaneToggleButtonVisible(false);
            break;
        case 3:
            nv.PaneDisplayMode(WUXC::NavigationViewPaneDisplayMode::LeftCompact);
            nv.IsPaneToggleButtonVisible(false);
            break;
        default:
            nv.PaneDisplayMode(WUXC::NavigationViewPaneDisplayMode::Left);
            nv.OpenPaneLength(DAY_NAV_SIDEBAR_WIDTH);
            break;
    }
    nav_sync_headers(nv);
    });
}

// Set while day is populating a NavigationView or writing the selection the app already holds.
// Scoped rather than a flag anyone can leave set: every mutation is synchronous on the UI thread,
// and WinUI raises SelectionChanged inside the assignment, so the guard is up exactly when the
// event that must not be reported arrives.
static int g_nav_mutating = 0;
struct NavMutation {
    NavMutation() { ++g_nav_mutating; }
    ~NavMutation() { --g_nav_mutating; }
    NavMutation(const NavMutation&) = delete;
    NavMutation& operator=(const NavMutation&) = delete;
};

// Document pages live in a shared content canvas, independently of tab-strip metadata.
// This keeps native page handles and their focus/undo state alive during reorder.
#ifdef DAY_WINUI
static WUXC::TabView document_tab_view(void* h) {
    auto grid=elem(h).try_as<WUXC::Grid>();
    if(grid && grid.Children().Size()>1) return grid.Children().GetAt(0).try_as<WUXC::TabView>();
    return nullptr;
}
#endif
void day_xaml_document_chrome(void* h, int native) {
#ifdef DAY_WINUI
    guard([&] {
        auto grid=elem(h).try_as<WUXC::Grid>();
        auto tabs=document_tab_view(h); if(!tabs || !grid) return;
        tabs.Visibility(native ? WUX::Visibility::Visible : WUX::Visibility::Collapsed);
        grid.RowDefinitions().GetAt(0).Height(WUX::GridLength{native ? 48.0 : 0.0, WUX::GridUnitType::Pixel});
    });
#else
    (void)h; (void)native;
#endif
}
void day_xaml_document_items(void* h, const char* titles_joined, const char* keys_joined) {
#ifdef DAY_WINUI
    NavMutation mutating;
    guard([&] {
        auto tabs=document_tab_view(h); if(!tabs) return;
        auto split=[](const char* text) {
            std::vector<std::string> out; std::string all=text ? text : "";
            if(all.empty()) return out;
            size_t start=0;
            while(true) { auto end=all.find('\x1f',start);
                out.push_back(all.substr(start,end==std::string::npos ? end : end-start));
                if(end==std::string::npos) break; start=end+1;
            }
            return out;
        };
        auto titles=split(titles_joined), keys=split(keys_joined);
        auto count=static_cast<uint32_t>(titles.size()<keys.size() ? titles.size() : keys.size());
        auto items=tabs.TabItems();
        auto key_of=[](WF::IInspectable const& item) {
            return winrt::to_string(winrt::unbox_value_or<winrt::hstring>(item.as<WUXC::TabViewItem>().Tag(),L""));
        };
        // Reconciled in place rather than cleared and refilled: a removed tab plays TabView's
        // close animation and an added one its entrance, and every other tab keeps its visual,
        // hover and focus state, which a rebuild would reset on each title edit. Removals run
        // back to front so the indices still to visit stay valid.
        for(uint32_t i=items.Size();i-->0;) {
            auto key=key_of(items.GetAt(i));
            if(std::find(keys.begin(),keys.begin()+count,key)==keys.begin()+count) items.RemoveAt(i);
        }
        for(uint32_t i=0;i<count;++i) {
            WUXC::TabViewItem tab{nullptr};
            for(uint32_t j=i;j<items.Size();++j) {
                auto candidate=items.GetAt(j).as<WUXC::TabViewItem>();
                if(key_of(candidate)!=keys[i]) continue;
                if(j!=i) { items.RemoveAt(j); items.InsertAt(i,candidate); }
                tab=candidate; break;
            }
            if(!tab) {
                tab=WUXC::TabViewItem{};
                tab.Tag(winrt::box_value(hs(keys[i].c_str())));
                items.InsertAt(i,tab);
            }
            auto title=hs(titles[i].c_str());
            if(winrt::unbox_value_or<winrt::hstring>(tab.Header(),L"")!=title) {
                tab.Header(winrt::box_value(title));
                WUXA::AutomationProperties::SetName(tab,title);
            }
        }
    });
#else
    (void)h; (void)titles_joined; (void)keys_joined;
#endif
}

void* day_xaml_nav_new(unsigned long long id,
                        void (*sel_cb)(unsigned long long, int),
                        void (*size_cb)(unsigned long long, int, int, int),
                        void (*back_cb)(unsigned long long),
                        void** out_content,
                        int stack, int rtl, int documents, int can_add, const char* new_label,
                        void (*document_cb)(unsigned long long,int,int,const char*)) {
    if(documents) {
        WUXC::Grid grid;
        WUXC::Canvas content;
        content.FlowDirection(WUX::FlowDirection::LeftToRight);
        content.HorizontalAlignment(WUX::HorizontalAlignment::Stretch);
        content.VerticalAlignment(WUX::VerticalAlignment::Stretch);
        content.SizeChanged([id,size_cb](WF::IInspectable const& sender,WUX::SizeChangedEventArgs const&) {
            auto fe=sender.as<FrameworkElement>();
            size_cb(id,0,static_cast<int>(fe.ActualWidth()),static_cast<int>(fe.ActualHeight()));
        });
#ifdef DAY_WINUI
        grid.FlowDirection(rtl ? WUX::FlowDirection::RightToLeft : WUX::FlowDirection::LeftToRight);
        WUXC::RowDefinition strip_row, content_row;
        strip_row.Height(WUX::GridLength{1,WUX::GridUnitType::Auto});
        content_row.Height(WUX::GridLength{1,WUX::GridUnitType::Star});
        grid.RowDefinitions().Append(strip_row); grid.RowDefinitions().Append(content_row);
        WUXC::TabView tabs;
        tabs.Height(48);
        tabs.CanDragTabs(true); tabs.CanReorderTabs(true); tabs.AllowDropTabs(true);
        // The add button is TabView's own, which the strip keeps right after the last tab the
        // way Edge and Terminal do; it answers with the app's localized label rather than WinUI's
        // string, written onto the template's button once it is realized (the template is
        // applied on load, so the name is set from Loaded, and again should the strip reload).
        tabs.IsAddTabButtonVisible(can_add!=0);
        if(can_add) {
            tabs.AddTabButtonClick([id,document_cb](auto const&, auto const&) { document_cb(id,0,0,""); });
            winrt::hstring label=hs(new_label);
            tabs.Loaded([label](WF::IInspectable const& s, WUX::RoutedEventArgs const&) {
                std::vector<WUX::DependencyObject> pending{s.as<WUX::DependencyObject>()};
                while(!pending.empty()) {
                    auto current=pending.back(); pending.pop_back();
                    if(auto add=current.try_as<WUXC::Button>(); add && add.Name()==L"AddButton") {
                        WUXA::AutomationProperties::SetName(add,label);
                        WUXC::ToolTipService::SetToolTip(add,winrt::box_value(label));
                        return;
                    }
                    for(int i=0;i<WUXM::VisualTreeHelper::GetChildrenCount(current);++i)
                        pending.push_back(WUXM::VisualTreeHelper::GetChild(current,i));
                }
            });
        }
        // During native drag reorder, selection indices are transient. Report selection only
        // after the stable-key order callback has brought Day's resident pages into sync.
        auto dragging=std::make_shared<bool>(false);
        tabs.TabDragStarting([dragging](auto const&, auto const&) { *dragging=true; });
        tabs.SelectionChanged([id,sel_cb,dragging](WF::IInspectable const& s,WUXC::SelectionChangedEventArgs const&) {
            // SelectionChangedEventHandler is untyped: the sender arrives as IInspectable.
            auto sender=s.as<WUXC::TabView>();
            if(!g_nav_mutating && !*dragging && sender.SelectedIndex()>=0) sel_cb(id,sender.SelectedIndex());
        });
        tabs.TabCloseRequested([id,document_cb](auto const&,WUXC::TabViewTabCloseRequestedEventArgs const& args) {
            auto key=winrt::to_string(winrt::unbox_value<winrt::hstring>(args.Tab().Tag()));
            document_cb(id,1,0,key.c_str());
        });
        tabs.TabDragCompleted([id,document_cb,dragging](WUXC::TabView const& sender,WUXC::TabViewTabDragCompletedEventArgs const& args) {
            *dragging=false;
            uint32_t index=0;
            if(sender.TabItems().IndexOf(args.Tab(),index)) {
                auto key=winrt::to_string(winrt::unbox_value<winrt::hstring>(args.Tab().Tag()));
                document_cb(id,2,static_cast<int>(index),key.c_str());
            }
        });
        grid.Children().Append(tabs);
        WUXC::Grid::SetRow(content,1);
#else
        (void)can_add; (void)new_label; (void)document_cb;
#endif
        grid.Children().Append(content);
        if(out_content) *out_content=boxh(content);
        return boxh(grid);
    }
    WUXC::NavigationView nv;
    nv.FlowDirection(rtl ? WUX::FlowDirection::RightToLeft : WUX::FlowDirection::LeftToRight);
    nv.IsSettingsVisible(false);
    nv.IsBackButtonVisible(WUXC::NavigationViewBackButtonVisible::Collapsed); // toggled per depth
    nv.IsBackEnabled(false);
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
    // Day already mirrors absolute child frames. Mirror native navigation chrome only.
    content.FlowDirection(WUX::FlowDirection::LeftToRight);
    content.HorizontalAlignment(WUX::HorizontalAlignment::Stretch);
    content.VerticalAlignment(WUX::VerticalAlignment::Stretch);
    nv.Content(content);
    content.SizeChanged(
        [id, size_cb](WF::IInspectable const& s, WUX::SizeChangedEventArgs const&) {
            auto fe = s.as<FrameworkElement>();
            size_cb(id, 0, static_cast<int>(fe.ActualWidth()), static_cast<int>(fe.ActualHeight()));
        });

    // User picked a menu item → report its index; day maps it back to the route via NAV_MENU.
    // Only the USER's picks are events. WinUI raises SelectionChanged for day's own population
    // too — clearing and refilling MenuItems moves the selection, and each appended item can take
    // it — and reporting those is day telling itself what it just said: the app's selection then
    // follows whichever item the population happened to leave selected, which is the last one
    // added. A destination set therefore opened on its final destination.
    nv.SelectionChanged(
        [id, sel_cb](WUXC::NavigationView const& sender,
                     WUXC::NavigationViewSelectionChangedEventArgs const& args) {
            if (g_nav_mutating) return;
            if (args.IsSettingsSelected()) return;
            auto item = args.SelectedItem();
            if (!item) return;
            int row = nav_row_of(sender, item);
            if (row >= 0) sel_cb(id, row);
        });
    nv.BackRequested([id, back_cb](WUXC::NavigationView const&,
                                   WUXC::NavigationViewBackRequestedEventArgs const&) {
        back_cb(id);
    });

    if (out_content) *out_content = boxh(content);
    return boxh(nv);
}

// --- inspector (docs/inspector.md): a SplitView with its pane placed RIGHT — the XAML
// trailing-pane container. Inline display, so the open pane sits beside the content rather
// than over it. Two stretched Canvas hosts, one per side; day positions content into each,
// and each host's SizeChanged reports its region back (0 = content, 1 = panel), the same
// contract the NavigationView regions use above.
void* day_xaml_inspector_new(unsigned long long id, double panel_width, int open, int leading,
                             void (*size_cb)(unsigned long long, int, int, int),
                             void** out_content, void** out_panel) {
    WUXC::SplitView sv;
    // `leading` puts the pane first — a layer panel (docs/inspector.md `.edge`), the same
    // arrangement Qt's splitter takes; Right stays the classic trailing inspector.
    sv.PanePlacement(leading ? WUXC::SplitViewPanePlacement::Left
                             : WUXC::SplitViewPanePlacement::Right);
    sv.DisplayMode(WUXC::SplitViewDisplayMode::Inline);
    sv.OpenPaneLength(panel_width);
    sv.IsPaneOpen(open != 0);
    WUXC::Canvas content;
    content.HorizontalAlignment(WUX::HorizontalAlignment::Stretch);
    content.VerticalAlignment(WUX::VerticalAlignment::Stretch);
    sv.Content(content);
    content.SizeChanged(
        [id, size_cb](WF::IInspectable const& s, WUX::SizeChangedEventArgs const&) {
            auto fe = s.as<FrameworkElement>();
            size_cb(id, 0, static_cast<int>(fe.ActualWidth()), static_cast<int>(fe.ActualHeight()));
        });
    WUXC::Canvas panel;
    panel.HorizontalAlignment(WUX::HorizontalAlignment::Stretch);
    panel.VerticalAlignment(WUX::VerticalAlignment::Stretch);
    sv.Pane(panel);
    panel.SizeChanged(
        [id, size_cb](WF::IInspectable const& s, WUX::SizeChangedEventArgs const&) {
            auto fe = s.as<FrameworkElement>();
            size_cb(id, 1, static_cast<int>(fe.ActualWidth()), static_cast<int>(fe.ActualHeight()));
        });
    if (out_content) *out_content = boxh(content);
    if (out_panel) *out_panel = boxh(panel);
    return boxh(sv);
}
void day_xaml_inspector_set_open(void* h, int open) {
    guard([&] { elem(h).as<WUXC::SplitView>().IsPaneOpen(open != 0); });
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
//   `geoms_joined`  the row's `.xamlgeom` spec with newlines escaped as \x1f (see day-toolkit-winui),
//                   empty when the glyph did not convert
//   `tints_joined`  an ARGB value, 0 meaning "no tint" — that row keeps the theme foreground
// A row draws as PathIcon geometry wherever it can, so the sidebar is vector at any DPI and its
// tint is a brush; only unconvertible art falls back to the monochrome raster.
void day_xaml_nav_set_items(void* navh, const char* items_joined, const char* icons_joined,
                            const char* geoms_joined, const char* tints_joined,
                            const char* badge_icons_joined, const char* badge_geoms_joined,
                            const char* badge_tints_joined, const char* sections_joined) {
    NavMutation mutating;
    guard([&] {
        auto nv = elem(navh).try_as<WUXC::NavigationView>();
        if (!nv) return;
        nv.MenuItems().Clear();
        // One line per row: the section title that opens before it, empty for none.
        auto sections = split_lines(sections_joined);
        auto titles = split_lines(items_joined);
        auto icons = split_lines(icons_joined);
        auto geoms = split_lines(geoms_joined ? geoms_joined : "");
        auto tints = split_lines(tints_joined ? tints_joined : "");
        auto badge_icons = split_lines(badge_icons_joined ? badge_icons_joined : "");
        auto badge_geoms = split_lines(badge_geoms_joined ? badge_geoms_joined : "");
        auto badge_tints = split_lines(badge_tints_joined ? badge_tints_joined : "");
        for (size_t i = 0; i < titles.size(); ++i) {
            if (i < sections.size() && !sections[i].empty()) {
                WUXC::NavigationViewItemHeader header;
                header.Content(winrt::box_value(hs(sections[i].c_str())));
                nv.MenuItems().Append(header);
            }
            WUXC::NavigationViewItem nvi;
            // The trailing status glyph (docs/navigation.md). NavigationViewItem has one Icon
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
            // A fully transparent value is the "no tint" encoding, not a real color.
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
        nav_sync_headers(nv);
    });
}

// Programmatic highlight sync (idx < 0 = clear). SelectionChanged still fires, but day's route
// set is idempotent (show() no-ops on the same key), so no feedback loop.
void day_xaml_nav_set_selected(void* navh, int idx) {
    NavMutation mutating;
    guard([&] {
#ifdef DAY_WINUI
        if(auto tabs=document_tab_view(navh)) { tabs.SelectedIndex(idx); return; }
#endif
        auto nv = elem(navh).try_as<WUXC::NavigationView>();
        if (!nv) return;
        int at = nav_menu_index(nv, idx);
        if (at < 0) {
            nv.SelectedItem(nullptr);
            return;
        }
        auto want = nv.MenuItems().GetAt(static_cast<uint32_t>(at));
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
            // IsBackEnabled defaults to false, and a visible-but-disabled button is drawn greyed and
            // never raises BackRequested, so enablement follows visibility.
            nv.IsBackEnabled(visible != 0);
        }
    });
}

#include "container-style.inc"

#include "controls.inc"

#include "text-input.inc"

#include "images.inc"

// ---- tree / geometry / props ----

void day_xaml_add_child(void* parent, void* child) {
    guard([&] {
        if (auto p = elem(parent).try_as<WUXC::Panel>()) {
            p.Children().Append(elem(child));
            if (g_field_live.count(child)) g_field_parents[child] = winrt::make_weak(p);
        }
    });
}
// Move `child` under `parent`, detaching it from wherever it currently hangs.
//
// `day_xaml_add_child` cannot do this: a XAML element may have only ONE logical parent, so
// `Children().Append` on an already-parented element throws, and `guard` swallows it — the call
// silently does nothing. That is how a fullscreen cover stayed boxed inside the content canvas
// while the code read as though it had been re-homed onto the root (docs/cover.md).
void day_xaml_reparent(void* parent, void* child) {
    guard([&] {
        auto c = elem(child);
        if (auto fe = c.try_as<WUX::FrameworkElement>()) {
            if (auto oldp = fe.Parent().try_as<WUXC::Panel>()) {
                uint32_t idx = 0;
                if (oldp.Children().IndexOf(c, idx)) oldp.Children().RemoveAt(idx);
            }
        }
        // Appended last, so it is topmost among its new siblings — what a cover needs.
        if (auto p = elem(parent).try_as<WUXC::Panel>()) {
            p.Children().Append(c);
            if (g_field_live.count(child)) g_field_parents[child] = winrt::make_weak(p);
        }
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
void day_xaml_transfer_release(void* h);
extern "C++" { // inside the extern "C" block; defined with the picker section below
static void picker_forget(void* h);
} // extern "C++"
void day_xaml_delete(void* h) {
    day_xaml_transfer_release(h);
    picker_forget(h);
#ifdef DAY_WINUI
    canvas_forget(h);
#endif
    g_clip_geometry.erase(h);
    g_image_views.erase(h);
    g_field_live.erase(h);
    g_field_parents.erase(h);
    g_button_selected.erase(h);
    g_button_tint.erase(h);
    delete reinterpret_cast<Node*>(h);
}

// External-piece handle seam (docs/picker.md): box any WinRT UI element into a day handle, and
// borrow it back — so an external piece can carry its OWN native XAML shim (like the Qt shims)
// without duplicating the private `Node` wrapper. The element crosses as a WinRT ABI pointer
// (`get_abi`, a stable COM interface pointer), which day-toolkit-xaml-sys owns the boxing for.
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
        if (auto it = g_image_views.find(h); it != g_image_views.end()) {
            layout_image(e.as<WUXC::Canvas>(), it->second->mode, width, height);
        }
        // A rounded clip tracks the size day just stated (see `g_clip_geometry`).
        auto it = g_clip_geometry.find(h);
        if (it != g_clip_geometry.end()) {
            it->second.Size({ static_cast<float>(width), static_cast<float>(height) });
        }
    });
}

void day_xaml_on_control_loaded(void* h, unsigned long long id,
                                void (*cb)(unsigned long long)) {
    if (!h) return;
    guard([&] {
        if (auto control = elem(h).try_as<WUXC::Control>()) {
            // No captured handle/reference: the control owns the subscription. Day defers
            // the callback's work and ignores a node disposed before that work runs.
            control.Loaded([id, cb](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
                cb(id);
            });
        }
    });
}

void day_xaml_measure(void* h, double aw, double ah, double* ow, double* oh) {
    *ow = 0; // sane defaults if a degraded element throws mid-measure (guard swallows it)
    *oh = 0;
    guard([&] {
        // Canvas reports no intrinsic size of its children. Preserve the image's natural
        // dimensions independently of the viewport size assigned by the previous layout.
        if (g_image_views.count(h)) {
            auto host = elem(h).as<WUXC::Canvas>();
            auto content = host.Children().GetAt(0);
            if (auto vb = content.try_as<WUXC::Viewbox>()) {
                auto art = vb.Child().as<FrameworkElement>();
                *ow = art.Width();
                *oh = art.Height();
            } else if (auto img = content.try_as<WUXC::Image>()) {
                if (auto bmp = img.Source().try_as<WUXM::Imaging::BitmapSource>()) {
                    *ow = bmp.PixelWidth();
                    *oh = bmp.PixelHeight();
                }
            } else {
                content.Measure({std::numeric_limits<float>::infinity(),
                                 std::numeric_limits<float>::infinity()});
                *ow = content.DesiredSize().Width;
                *oh = content.DesiredSize().Height;
            }
            return;
        }
        float fw = aw < 0 ? std::numeric_limits<float>::infinity() : static_cast<float>(aw);
        float fh = ah < 0 ? std::numeric_limits<float>::infinity() : static_cast<float>(ah);
        auto& e = elem(h);
        // Measure with the explicit size back at Auto. Once day has framed an element,
        // `set_geometry` has stamped a Width/Height on it, and XAML derives `DesiredSize` from
        // Those rather than from the content — so a re-measure after the content changed reports
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
        // Each step caught on its own: under WinUI 3 a templated control that is not in the tree
        // yet can throw from its first Measure (ProgressRing does, E_FAIL), and one throw used to
        // abandon the whole measure, reporting 0x0 and skipping the frame restore below. Caught
        // here, the forced layout and re-measure that follow still size it.
        auto step = [](const char*, auto&& f) {
            try {
                f();
            } catch (...) {
            }
        };
        WF::Size d{ 0, 0 };
        // A nested/resident page can be measured before its first native layout. Prepare
        // its control templates explicitly; UpdateLayout alone need not visit a hidden
        // page. Loaded also invalidates Day's cache once the control joins the live tree.
        step("template", [&] {
            if (auto control = e.try_as<WUXC::Control>()) (void)control.ApplyTemplate();
        });
        step("measure", [&] {
            e.Measure(WF::Size{ fw, fh });
            d = e.DesiredSize();
        });
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
            // the outer `fe` — re-querying here only shadowed it (C4456)
            step("layout", [&] { if (fe) fe.UpdateLayout(); });
            s_forcing_layout = false;
            step("remeasure", [&] {
                e.InvalidateMeasure();
                e.Measure(WF::Size{ fw, fh });
                d = e.DesiredSize();
            });
        }
        // A desired size is never below the element's minimums, which is XAML's own rule. Applied
        // here as well because a measure that threw (above) reports nothing: when the forced layout
        // cannot run (another element's is already in progress), the minimums are the size.
        if (fe) {
            d.Width = (std::max)(d.Width, static_cast<float>(fe.MinWidth()));
            d.Height = (std::max)(d.Height, static_cast<float>(fe.MinHeight()));
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

// First text baseline from the element's top, in DIPs, for a box `box_h` tall
// (docs/baseline.md). A TextBlock publishes `BaselineOffset` directly; every other control keeps
// its text in a TextBlock somewhere inside its template, but that template is not built until the
// control is in the visual tree, so the general case derives the baseline the way XAML lays a
// single line out: center the line box in the control's height, with the baseline an ascent below
// its top. Returns -1 for an element with no text.
double day_xaml_baseline(void* h, double box_h) {
    double out = -1;
    guard([&] {
        auto el = elem(h);
        if (auto tb = el.try_as<WUX::Controls::TextBlock>()) {
            // The element's own answer, and it already accounts for the font in use.
            out = tb.BaselineOffset();
            return;
        }
        auto ctl = el.try_as<WUX::Controls::Control>();
        if (!ctl) return;
        const double size = ctl.FontSize();
        if (!(size > 0)) return;
        // The metrics ratios of the Windows UI font family; XAML exposes no per-font ascent
        // outside a laid-out TextBlock, and these hold across the Segoe faces it ships with.
        const double ascent = size * 0.86;
        const double line = size * 1.33;
        const auto pad = ctl.Padding();
        const auto border = ctl.BorderThickness();
        const double top = border.Top + pad.Top;
        const double inner = box_h - top - pad.Bottom - border.Bottom;
        out = top + (inner > line ? (inner - line) / 2.0 : 0.0) + ascent;
    });
    return out;
}

void day_xaml_set_name(void* h, const char* name) {
    guard([&] { WUX::Automation::AutomationProperties::SetAutomationId(elem(h), hs(name)); });
}

#include "accessibility.inc"

#include "gestures.inc"

#include "snapshot.inc"

} // extern "C"

#include "menus.inc"

#include "toolbar.inc"

#include "dialogs.inc"

#include "shell-integration.inc"

// ---- window properties (docs/windows.md "Window properties") ------------------------------

// Move and/or resize: the outer frame's top-left in points on the desktop, the content size day
// lays out in points (the docked menu bar and toolbar, and the frame, are added around it).
extern "C" void day_xaml_window_set_frame(void* win, int has_origin, double x, double y,
                                          int has_size, double w, double h) try {
    HWND hwnd = host_of_window(win);
    if (!hwnd) return;
    RECT wr, cr;
    GetWindowRect(hwnd, &wr);
    GetClientRect(hwnd, &cr);
    UINT flags = SWP_NOZORDER | SWP_NOACTIVATE;
    int nx = 0, ny = 0, nw = 0, nh = 0;
    if (has_origin) {
        double s = desktop_scale();
        nx = static_cast<int>(std::lround(x * s));
        ny = static_cast<int>(std::lround(y * s));
    } else {
        flags |= SWP_NOMOVE;
    }
    if (has_size) {
        auto it = g_chrome.find(hwnd);
        int docked = it != g_chrome.end() ? it->second.docked_px : 0;
        double s = dip_scale(hwnd);
        int cw = static_cast<int>(std::lround(w * s));
        int ch = static_cast<int>(std::lround(h * s)) + docked;
        nw = (wr.right - wr.left) + (cw - cr.right);
        nh = (wr.bottom - wr.top) + (ch - cr.bottom);
    } else {
        flags |= SWP_NOSIZE;
    }
    SetWindowPos(hwnd, nullptr, nx, ny, nw, nh, flags);
} catch (...) {
}

// The content size the user can resize to, in points; 0 lifts that bound. WM_GETMINMAXINFO
// applies them (window_minmax); the window is resized into them at once.
extern "C" void day_xaml_window_set_limits(void* win, double min_w, double min_h, double max_w,
                                           double max_h) try {
    HWND hwnd = host_of_window(win);
    auto it = g_chrome.find(hwnd);
    if (!hwnd || it == g_chrome.end()) return;
    it->second.min_w = min_w;
    it->second.min_h = min_h;
    it->second.max_w = max_w;
    it->second.max_h = max_h;
    RECT wr;
    GetWindowRect(hwnd, &wr);
    // The same size again: DefWindowProc clamps it to the new track sizes on the way.
    SetWindowPos(hwnd, nullptr, 0, 0, wr.right - wr.left, wr.bottom - wr.top,
                 SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
} catch (...) {
}

// One on/off window property: 0 floating (topmost), 1 left off the taskbar, 2 resizable,
// 3 minimizable, 4 maximizable, 5 closable, 6 visible.
extern "C" void day_xaml_window_set_flag(void* win, int which, int on) try {
    HWND hwnd = host_of_window(win);
    auto it = g_chrome.find(hwnd);
    if (!hwnd || it == g_chrome.end()) return;
    WinChrome& c = it->second;
    auto style_bit = [hwnd](LONG_PTR bit, bool set) {
        LONG_PTR style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(hwnd, GWL_STYLE, set ? (style | bit) : (style & ~bit));
        SetWindowPos(hwnd, nullptr, 0, 0, 0, 0,
                     SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
    };
    switch (which) {
    case 0:
        SetWindowPos(hwnd, on ? HWND_TOPMOST : HWND_NOTOPMOST, 0, 0, 0, 0,
                     SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        break;
    case 1:
        c.skip_taskbar = on != 0;
        if (ITaskbarList3* tb = taskbar_list()) {
            if (on) tb->DeleteTab(hwnd);
            else tb->AddTab(hwnd);
        }
        break;
    case 2:
        c.resizable = on != 0; // a frameless window's edges follow it (chrome_hit_test)
        style_bit(WS_THICKFRAME, on != 0);
        break;
    case 3: style_bit(WS_MINIMIZEBOX, on != 0); break;
    case 4: style_bit(WS_MAXIMIZEBOX, on != 0); break;
    case 5:
        c.closable = on != 0;
        EnableMenuItem(GetSystemMenu(hwnd, FALSE), SC_CLOSE,
                       MF_BYCOMMAND | (on ? MF_ENABLED : (MF_DISABLED | MF_GRAYED)));
        break;
    case 6: ShowWindow(hwnd, on ? SW_SHOW : SW_HIDE); break;
    }
} catch (...) {
}

// This window's own scheme: -1 the app's, 0 light, 1 dark.
extern "C" void day_xaml_window_set_appearance(void* win, int mode) try {
    HWND hwnd = host_of_window(win);
    auto it = g_chrome.find(hwnd);
    if (!hwnd || it == g_chrome.end()) return;
    it->second.appearance = (mode == 0 || mode == 1) ? mode : -1;
    apply_window_appearance(hwnd);
} catch (...) {
}

// Flash the taskbar button: a few times (informational), or the caption and the button until the
// window comes to the foreground (critical).
extern "C" void day_xaml_window_attention(void* win, int critical) try {
    HWND hwnd = host_of_window(win);
    if (!hwnd) return;
    FLASHWINFO fi{};
    fi.cbSize = sizeof(fi);
    fi.hwnd = hwnd;
    fi.dwFlags = critical ? (FLASHW_ALL | FLASHW_TIMERNOFG) : FLASHW_TRAY;
    fi.uCount = critical ? 0 : 3;
    FlashWindowEx(&fi);
} catch (...) {
}

// The outer frame in points on the desktop: x, y, width, height into `out`. 0 when unknown.
extern "C" int day_xaml_window_frame(void* win, double* out) try {
    HWND hwnd = host_of_window(win);
    RECT r;
    if (!hwnd || !out || !GetWindowRect(hwnd, &r)) return 0;
    double s = desktop_scale();
    out[0] = r.left / s;
    out[1] = r.top / s;
    out[2] = (r.right - r.left) / s;
    out[3] = (r.bottom - r.top) / s;
    return 1;
} catch (...) {
    return 0;
}

// ---- monitors (docs/windows.md "Monitors") --------------------------------------------------
// day-toolkit-xaml-sys's `DayXamlMonitor`, field for field: rectangles in points on the desktop.
struct DayXamlMonitor {
    double frame[4];
    double work[4];
    double scale;
    int primary;
    char id[160];
    char name[128];
};

// UTF-8 into a fixed buffer, cut at a character boundary.
static void copy_u8(char* dst, size_t cap, const wchar_t* src) {
    std::string s = u8(winrt::hstring(src ? src : L""));
    size_t n = (std::min)(s.size(), cap - 1);
    while (n > 0 && n < s.size() && (static_cast<unsigned char>(s[n]) & 0xC0) == 0x80) --n;
    std::memcpy(dst, s.data(), n);
    dst[n] = '\0';
}

static BOOL CALLBACK collect_monitor(HMONITOR m, HDC, LPRECT, LPARAM lp) {
    reinterpret_cast<std::vector<HMONITOR>*>(lp)->push_back(m);
    return TRUE;
}

// Every attached display into `out` (up to `cap`); returns how many there are.
extern "C" int day_xaml_monitors(DayXamlMonitor* out, int cap) try {
    std::vector<HMONITOR> mons;
    EnumDisplayMonitors(nullptr, nullptr, collect_monitor, reinterpret_cast<LPARAM>(&mons));
    // Shcore's GetDpiForMonitor, loaded rather than linked (like CoreMessaging's export above).
    using DpiFn = HRESULT(WINAPI*)(HMONITOR, int, UINT*, UINT*);
    static DpiFn dpi_for = [] {
        HMODULE m = LoadLibraryW(L"Shcore.dll");
        return m ? reinterpret_cast<DpiFn>(GetProcAddress(m, "GetDpiForMonitor")) : nullptr;
    }();
    double s = desktop_scale();
    int n = 0;
    for (HMONITOR m : mons) {
        if (out && n < cap) {
            MONITORINFOEXW mi{};
            mi.cbSize = sizeof(mi);
            if (!GetMonitorInfoW(m, &mi)) continue;
            DayXamlMonitor& d = out[n];
            d.frame[0] = mi.rcMonitor.left / s;
            d.frame[1] = mi.rcMonitor.top / s;
            d.frame[2] = (mi.rcMonitor.right - mi.rcMonitor.left) / s;
            d.frame[3] = (mi.rcMonitor.bottom - mi.rcMonitor.top) / s;
            d.work[0] = mi.rcWork.left / s;
            d.work[1] = mi.rcWork.top / s;
            d.work[2] = (mi.rcWork.right - mi.rcWork.left) / s;
            d.work[3] = (mi.rcWork.bottom - mi.rcWork.top) / s;
            UINT dx = 96, dy = 96; // MDT_EFFECTIVE_DPI = 0
            if (!dpi_for || FAILED(dpi_for(m, 0, &dx, &dy))) dx = GetDpiForSystem();
            d.scale = dx / 96.0;
            d.primary = (mi.dwFlags & MONITORINFOF_PRIMARY) ? 1 : 0;
            // The monitor behind the adapter output: its device interface path is stable for
            // the display across launches, its string the name Windows shows.
            DISPLAY_DEVICEW dd{};
            dd.cb = sizeof(dd);
            bool have = EnumDisplayDevicesW(mi.szDevice, 0, &dd, EDD_GET_DEVICE_INTERFACE_NAME) != 0;
            copy_u8(d.id, sizeof(d.id), have && dd.DeviceID[0] ? dd.DeviceID : mi.szDevice);
            copy_u8(d.name, sizeof(d.name), have && dd.DeviceString[0] ? dd.DeviceString : mi.szDevice);
        }
        ++n;
    }
    return n;
} catch (...) {
    return 0;
}

#include "picker.inc"

// ---- textarea (docs/text.md) ----------------------------------------------
// A multi-line TextBox (AcceptsReturn, wrapping, native PlaceholderText). Also folded in from its
// own file; see the note on the picker section above.
//
// TextChanged reports edits back to Rust as a UTF-8 C string (valid only during the callback; Rust
// copies it). Programmatic Text(...) re-fires TextChanged, but the front-end's bind only re-patches
// on a real change and guards the echo, so there is no runaway loop.

// `editable` and `spell-check` are plain TextBox properties (IsReadOnly / IsSpellCheckEnabled).
// `selectable` is NOT: IsTextSelectionEnabled lives on TextBlock and RichTextBlock, and TextBox
// carries no equivalent — so it is EMULATED here. While selection is off, every selection that
// forms is collapsed as it is reported, and the context menu (the other route to Copy / Select All)
// is suppressed. `Cap::TextSelectable` reports `Emulated` for precisely this reason.
//
// The flag is shared with the handlers registered at construction rather than looked up inside
// them, so toggling it costs nothing and the handlers never touch this map.
static std::map<void*, std::shared_ptr<bool>> g_selectable;

static WUXC::TextBox textarea_box_of(void* handle) {
    WUX::UIElement e{ nullptr };
    winrt::copy_from_abi(e, day_xaml_unbox(handle));
    return e.try_as<WUXC::TextBox>();
}

extern "C" {

void* day_textarea_xaml_new(const char* placeholder, const char* initial, uint64_t id,
                            void (*cb)(uint64_t, const char*)) {
    WUXC::TextBox box;
    box.AcceptsReturn(true);
    box.TextWrapping(WUX::TextWrapping::Wrap);
    box.PlaceholderText(hs(placeholder));
    if (initial && *initial) box.Text(hs(initial));
    box.TextChanged([id, cb](WF::IInspectable const& sender, WUXC::TextChangedEventArgs const&) {
        if (auto tb = sender.try_as<WUXC::TextBox>()) {
            std::string t = lf(u8(tb.Text()));
            cb(id, t.c_str());
        }
    });

    auto selectable = std::make_shared<bool>(true);
    box.SelectionChanged([selectable](WF::IInspectable const& sender, WUX::RoutedEventArgs const&) {
        if (*selectable) return;
        if (auto tb = sender.try_as<WUXC::TextBox>()) {
            // Collapsing re-enters this handler once with a zero length, which then falls through
            // the guard above — so it converges without needing a re-entrancy flag. The caret
            // (SelectionStart with length 0) is left where the user put it.
            if (tb.SelectionLength() > 0) tb.SelectionLength(0);
        }
    });
    box.ContextMenuOpening(
        [selectable](WF::IInspectable const&, WUXC::ContextMenuEventArgs const& args) {
            if (!*selectable) args.Handled(true);
        });

    void* handle = day_xaml_box(winrt::get_abi(box));
    g_selectable[handle] = selectable;
    return handle;
}

void day_textarea_xaml_set_editable(void* handle, int on) {
    if (auto box = textarea_box_of(handle)) box.IsReadOnly(on == 0);
}

void day_textarea_xaml_set_spellcheck(void* handle, int on) {
    if (auto box = textarea_box_of(handle)) box.IsSpellCheckEnabled(on != 0);
}

void day_textarea_xaml_set_selectable(void* handle, int on) {
    auto it = g_selectable.find(handle);
    if (it == g_selectable.end()) return;
    *it->second = on != 0;
    // Turning it off with a selection already on screen has to clear that one too — the handler
    // only ever sees selections made from here on.
    if (on == 0) {
        if (auto box = textarea_box_of(handle)) {
            if (box.SelectionLength() > 0) box.SelectionLength(0);
        }
    }
}

void day_textarea_xaml_set_text(void* handle, const char* text) {
    if (auto box = textarea_box_of(handle)) {
        // Compared in Day's spelling: the box holds CRs, so comparing its raw text against an LF
        // string never matched and every patch re-set the whole box — which is what takes the
        // caret back to the start mid-typing.
        if (lf(u8(box.Text())) != std::string(text ? text : "")) box.Text(hs(text));
    }
}

#include "tree.inc"

void day_xaml_nav_custom_sidebar(void *w, void *page) { guard([&] {
    auto nav=elem(w).as<WUXC::NavigationView>(); nav.PaneHeader(nullptr); nav.PaneCustomContent(elem(page));
}); }

} // extern "C"

#include "transfer.inc"
#ifdef DAY_WINUI
#include "list-winui.inc"
#endif
