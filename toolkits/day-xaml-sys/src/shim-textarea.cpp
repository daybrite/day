// The textarea piece's OWN C++/WinRT shim — parallel to src/lib-qt-shim.cpp. A multi-line TextBox
// (AcceptsReturn = true, TextWrapping = Wrap, a native PlaceholderText), boxed into a Day handle via the
// day_xaml_box/day_xaml_unbox seam that day-xaml-sys exports, so this piece carries its own XAML
// native code with ZERO edits to day's toolkit crates.
//
// TextChanged reports edits back to Rust as a UTF-8 C string (valid only during the callback; Rust
// copies it). Programmatic Text(...) re-fires TextChanged, but the front-end's bind only re-patches on a
// real change and guards the echo, so there is no runaway loop.
//
// Windows-only; compiled by build.rs (like the Qt shim) and linked alongside day-xaml-sys.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>

#include <windows.h>

#include <cstdint>
#include <map>
#include <memory>
#include <string>

using namespace winrt;
namespace WUX = winrt::Windows::UI::Xaml;
namespace WUXC = winrt::Windows::UI::Xaml::Controls;

// The boxing seam, exported by day-xaml-sys (already linked into the app).
extern "C" void *day_xaml_box(void *iinspectable_abi);
extern "C" void *day_xaml_unbox(void *handle);

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

// `editable` and `spell-check` are plain TextBox properties (IsReadOnly / IsSpellCheckEnabled).
// `selectable` is NOT: IsTextSelectionEnabled lives on TextBlock and RichTextBlock, and TextBox
// carries no equivalent — so it is EMULATED here. While selection is off, every selection that
// forms is collapsed as it is reported, and the context menu (the other route to Copy / Select
// All) is suppressed. `Cap::TextSelectable` reports `Emulated` for precisely this reason.
//
// The flag is shared with the handlers registered at construction rather than looked up inside
// them, so toggling it costs nothing and the handlers never touch this map.
static std::map<void *, std::shared_ptr<bool>> g_selectable;

static WUXC::TextBox box_of(void *handle) {
    WUX::UIElement e{nullptr};
    winrt::copy_from_abi(e, day_xaml_unbox(handle));
    return e.try_as<WUXC::TextBox>();
}

extern "C" {

void *day_textarea_xaml_new(const char *placeholder, const char *initial, uint64_t id,
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

    auto selectable = std::make_shared<bool>(true);
    box.SelectionChanged([selectable](winrt::Windows::Foundation::IInspectable const &sender,
                                      WUX::RoutedEventArgs const &) {
        if (*selectable)
            return;
        if (auto tb = sender.try_as<WUXC::TextBox>()) {
            // Collapsing re-enters this handler once with a zero length, which then falls through
            // the guard below — so it converges without needing a re-entrancy flag. The caret
            // (SelectionStart with length 0) is left where the user put it.
            if (tb.SelectionLength() > 0)
                tb.SelectionLength(0);
        }
    });
    box.ContextMenuOpening([selectable](winrt::Windows::Foundation::IInspectable const &,
                                        WUXC::ContextMenuEventArgs const &args) {
        if (!*selectable)
            args.Handled(true);
    });

    void *handle = day_xaml_box(winrt::get_abi(box));
    g_selectable[handle] = selectable;
    return handle;
}

void day_textarea_xaml_set_editable(void *handle, int on) {
    if (auto box = box_of(handle))
        box.IsReadOnly(on == 0);
}

void day_textarea_xaml_set_spellcheck(void *handle, int on) {
    if (auto box = box_of(handle))
        box.IsSpellCheckEnabled(on != 0);
}

void day_textarea_xaml_set_selectable(void *handle, int on) {
    auto it = g_selectable.find(handle);
    if (it == g_selectable.end())
        return;
    *it->second = on != 0;
    // Turning it off with a selection already on screen has to clear that one too — the handler
    // only ever sees selections made from here on.
    if (on == 0) {
        if (auto box = box_of(handle)) {
            if (box.SelectionLength() > 0)
                box.SelectionLength(0);
        }
    }
}

void day_textarea_xaml_set_text(void *handle, const char *text) {
    WUX::UIElement e{nullptr};
    winrt::copy_from_abi(e, day_xaml_unbox(handle));
    if (auto box = e.try_as<WUXC::TextBox>()) {
        auto nt = hs(text);
        if (box.Text() != nt)
            box.Text(nt);
    }
}

} // extern "C"
