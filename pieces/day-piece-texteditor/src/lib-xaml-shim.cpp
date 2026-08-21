// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-piece-texteditor's OWN C++/WinRT shim — parallel to src/lib-qt-shim.cpp. A `RichEditBox`,
// XAML's rich text editor, driven through its Text Object Model document (`ITextDocument`, the
// same model RichEdit has carried since Win32).
//
// The TOM shape this arm relies on:
//
// - `Document.GetRange(start, end)` takes character positions, which for RichEdit are UTF-16 code
//   units — the same unit the Apple, Android and Qt arms use, so Rust converts once and every one
//   of them takes it.
// - `range.CharacterFormat` is the run's attributes; assigning a whole `ITextCharacterFormat` to a
//   range applies it in one call, which is what keeps a per-keystroke re-highlight to O(runs)
//   rather than O(characters).
// - `Document.Selection` with a COLLAPSED range is the typing format — native, like Qt's, and
//   unlike GTK's and the web's.
// - `Document.SetText(TextSetOptions::None, …)` replaces the characters. `BatchDisplayUpdates` /
//   `ApplyDisplayUpdates` bracket a sweep so the control lays out once.
//
// What this shim does NOT do is read attributes back. Day owns them (see the crate docs), so
// `IsSpellCheckEnabled` and the characters are the only things the control decides. `Ctrl+B` and
// friends are RichEditBox's own and would change formatting behind Day's back, so they are
// swallowed — the same call the Qt shim makes for the same reason.
//
// Windows-only; compiled by build.rs, built in CI, NOT verified locally. docs/texteditor.md lists
// what a check on Windows has to confirm.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h> // IVector methods — else C3779
#include <winrt/Windows.System.h>  // VirtualKey
#include <winrt/Windows.UI.h>
#include <winrt/Windows.UI.Core.h> // CoreWindow, CoreVirtualKeyStates
#include <winrt/Windows.UI.Text.h>
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>
#include <winrt/Windows.UI.Xaml.Input.h>
#include <winrt/Windows.UI.Xaml.Media.h>

#include <windows.h>

#include <cmath>
#include <cstdint>
#include <map>
#include <memory>
#include <string>

using namespace winrt;
namespace WF = winrt::Windows::Foundation;
namespace WS = winrt::Windows::System;
namespace WU = winrt::Windows::UI;
namespace WUC = winrt::Windows::UI::Core;
namespace WUT = winrt::Windows::UI::Text;
namespace WUX = winrt::Windows::UI::Xaml;
namespace WUXC = winrt::Windows::UI::Xaml::Controls;
namespace WUXI = winrt::Windows::UI::Xaml::Input;
namespace WUXM = winrt::Windows::UI::Xaml::Media;

// The boxing seam, exported by day-xaml-sys (already linked into the app).
extern "C" void *day_xaml_box(void *iinspectable_abi);
extern "C" void *day_xaml_unbox(void *handle);

namespace {

/// UTF-8 → UTF-16, the same helper every other xaml shim in the tree carries.
std::wstring wide(const char *s) {
    if (!s || !*s) return std::wstring();
    const int n = MultiByteToWideChar(CP_UTF8, 0, s, -1, nullptr, 0);
    if (n <= 1) return std::wstring();
    std::wstring out(static_cast<size_t>(n - 1), L'\0');
    MultiByteToWideChar(CP_UTF8, 0, s, -1, out.data(), n);
    return out;
}

std::string narrow(winrt::hstring const &h) {
    if (h.empty()) return std::string();
    const int n = WideCharToMultiByte(CP_UTF8, 0, h.c_str(), static_cast<int>(h.size()), nullptr, 0,
                                      nullptr, nullptr);
    std::string out(static_cast<size_t>(n), '\0');
    WideCharToMultiByte(CP_UTF8, 0, h.c_str(), static_cast<int>(h.size()), out.data(), n, nullptr,
                        nullptr);
    return out;
}

/// day's sizes cross as XAML DIPs (1/96 in) — `FontSize` takes them verbatim everywhere else in
/// this backend, which is what fixes one number to one rendered size across a window. RichEdit's
/// TOM is typographic POINTS (1/72 in), so handing it the same number drew this editor's text a
/// third larger than every label beside it. Convert on the way in; ranges and the typing style go
/// through here too, so a run's own size lands on the same scale as the base.
float tom_points(double dip) {
    return static_cast<float>(dip * 72.0 / 96.0);
}

WU::Color unpack(uint32_t argb) {
    return WU::ColorHelper::FromArgb(static_cast<uint8_t>((argb >> 24) & 0xFF),
                                     static_cast<uint8_t>((argb >> 16) & 0xFF),
                                     static_cast<uint8_t>((argb >> 8) & 0xFF),
                                     static_cast<uint8_t>(argb & 0xFF));
}

/// Per-control state. `suppress` guards the reports while Day itself writes; `base` is the point
/// size a run's relative scale multiplies, which the control cannot be asked for per range.
struct EditorState {
    bool suppress = false;
    double base = 15.0;
};

std::map<void *, std::shared_ptr<EditorState>> g_state;

WUXC::RichEditBox box_of(void *handle) {
    WUX::UIElement e{nullptr};
    winrt::copy_from_abi(e, day_xaml_unbox(handle));
    return e.try_as<WUXC::RichEditBox>();
}

std::shared_ptr<EditorState> state_of(void *handle) {
    auto it = g_state.find(handle);
    return it == g_state.end() ? nullptr : it->second;
}

/// The document's plain text — what Rust diffs a keystroke out of.
std::string document_text(WUXC::RichEditBox const &box) {
    winrt::hstring text;
    box.Document().GetText(WUT::TextGetOptions::None, text);
    // RichEdit terminates its document with a paragraph mark Day's model does not have.
    std::string s = narrow(text);
    while (!s.empty() && (s.back() == '\r' || s.back() == '\n')) s.pop_back();
    // RichEdit's paragraph mark is a CR, and a Shift+Enter line break a VT. Every other toolkit
    // reports a line ending as LF, and Day's model is the ONE text an app splits, searches and
    // diffs, so it has to read the same on all eight — a `\n` the app looks for is simply not
    // there otherwise, on Windows alone.
    //
    // Rewritten in place, one code unit for one: these offsets ARE the ones the selection and
    // every attribute range are expressed in, so collapsing anything here (a pair to a single
    // LF) would shift every position after it by one per line — the class of bug the
    // walkthrough's select-a-word assertion exists to catch.
    for (char &c : s) {
        if (c == '\r' || c == '\v') c = '\n';
    }
    return s;
}

} // namespace

extern "C" {

void *day_texteditor_xaml_new(uint64_t id, int editable, int spellcheck, double base_pt,
                              const char *placeholder, const char *initial,
                              void (*text_cb)(uint64_t, const char *),
                              void (*sel_cb)(uint64_t, uint64_t, uint64_t)) {
    WUXC::RichEditBox box;
    box.AcceptsReturn(true);
    box.TextWrapping(WUX::TextWrapping::Wrap);
    box.IsReadOnly(editable == 0);
    box.IsSpellCheckEnabled(spellcheck != 0);
    box.PlaceholderText(winrt::hstring(wide(placeholder)));
    // A paste keeps its characters and takes the surrounding style — the same call every arm
    // makes, so what a paste means is one behavior across the eight.
    box.Document().DefaultTabStop(36.0f);
    box.Document().UndoLimit(100);
    if (initial && *initial) {
        box.Document().SetText(WUT::TextSetOptions::None, winrt::hstring(wide(initial)));
    }

    auto st = std::make_shared<EditorState>();
    st->base = base_pt;

    box.TextChanged([id, text_cb, st](WF::IInspectable const &sender, WUX::RoutedEventArgs const &) {
        if (st->suppress) return;
        if (auto b = sender.try_as<WUXC::RichEditBox>()) {
            const std::string t = document_text(b);
            text_cb(id, t.c_str());
        }
    });
    box.SelectionChanged([id, sel_cb, st](WF::IInspectable const &sender, WUX::RoutedEventArgs const &) {
        if (st->suppress) return;
        if (auto b = sender.try_as<WUXC::RichEditBox>()) {
            auto sel = b.Document().Selection();
            sel_cb(id, static_cast<uint64_t>(sel.StartPosition()),
                   static_cast<uint64_t>(sel.EndPosition()));
        }
    });
    // RichEditBox's own Ctrl+B / Ctrl+I / Ctrl+U change the character format directly. Swallow
    // them: attributes are Day's, and a toolbar button goes through the bound signal.
    box.KeyDown([](WF::IInspectable const &, WUXI::KeyRoutedEventArgs const &args) {
        const auto ctrl = WUX::Window::Current().CoreWindow().GetKeyState(WS::VirtualKey::Control);
        // Both `GetKeyState` and the flag operators over `CoreVirtualKeyStates` are defined in
        // <winrt/Windows.UI.Core.h>. Without it only the Xaml headers' forward declarations were
        // here, which left the call's `auto` return type undefined and collapsed the state test
        // into the operator== soup CI reported. The bit test is what the projection's own
        // `operator&` performs, and unlike `(ctrl & Down) == Down` it does not depend on whether a
        // given Windows Kit's operator hands back the enum or its underlying type.
        const bool down = (static_cast<uint32_t>(ctrl) &
                           static_cast<uint32_t>(WUC::CoreVirtualKeyStates::Down)) != 0;
        if (!down) return;
        const auto k = args.Key();
        if (k == WS::VirtualKey::B || k == WS::VirtualKey::I || k == WS::VirtualKey::U) {
            args.Handled(true);
        }
    });

    void *handle = day_xaml_box(winrt::get_abi(box));
    g_state[handle] = st;
    return handle;
}

void day_texteditor_xaml_set_text(void *handle, const char *utf8) {
    auto box = box_of(handle);
    auto st = state_of(handle);
    if (!box || !st) return;
    const std::wstring w = wide(utf8);
    if (document_text(box) == narrow(winrt::hstring(w))) return;
    const auto caret = box.Document().Selection().StartPosition();
    st->suppress = true;
    box.Document().SetText(WUT::TextSetOptions::None, winrt::hstring(w));
    auto sel = box.Document().Selection();
    sel.SetRange(caret, caret);
    st->suppress = false;
}

// One attribute sweep: begin (which resets the whole document to the base format), apply each run
// and paragraph, end. `BatchDisplayUpdates` keeps it to a single layout pass.
void day_texteditor_xaml_begin_attrs(void *handle) {
    auto box = box_of(handle);
    auto st = state_of(handle);
    if (!box || !st) return;
    st->suppress = true;
    auto doc = box.Document();
    doc.BatchDisplayUpdates();
    auto all = doc.GetRange(0, INT32_MAX);
    auto fmt = all.CharacterFormat();
    fmt.Size(tom_points(st->base));
    fmt.Bold(WUT::FormatEffect::Off);
    fmt.Italic(WUT::FormatEffect::Off);
    fmt.Underline(WUT::UnderlineType::None);
    fmt.Strikethrough(WUT::FormatEffect::Off);
    all.CharacterFormat(fmt);
    auto para = all.ParagraphFormat();
    para.Alignment(WUT::ParagraphAlignment::Left);
    // `LeftIndent` and `FirstLineIndent` are READ-ONLY on ITextParagraphFormat — the TOM writes the
    // three indents together, as SetIndents(start, left, right), where `start` is the first line's
    // offset relative to `left`.
    para.SetIndents(0.0f, 0.0f, 0.0f);
    all.ParagraphFormat(para);
}

// `underline`: 0 none, 1 single, 2 double, 3 dotted, 4 wavy — Day's `Underline`, and the one arm
// of the eight whose toolkit has a distinct spelling for every variant.
void day_texteditor_xaml_apply_run(void *handle, int start, int end, double pt, int bold, int italic,
                                   int mono, int underline, int strike, int has_fg, uint32_t fg,
                                   int has_bg, uint32_t bg) {
    auto box = box_of(handle);
    if (!box) return;
    auto range = box.Document().GetRange(start, end);
    auto fmt = range.CharacterFormat();
    fmt.Size(tom_points(pt));
    fmt.Bold(bold ? WUT::FormatEffect::On : WUT::FormatEffect::Off);
    fmt.Italic(italic ? WUT::FormatEffect::On : WUT::FormatEffect::Off);
    fmt.Strikethrough(strike ? WUT::FormatEffect::On : WUT::FormatEffect::Off);
    if (mono) fmt.Name(L"Cascadia Mono");
    switch (underline) {
    case 1: fmt.Underline(WUT::UnderlineType::Single); break;
    case 2: fmt.Underline(WUT::UnderlineType::Double); break;
    case 3: fmt.Underline(WUT::UnderlineType::Dotted); break;
    case 4: fmt.Underline(WUT::UnderlineType::Wave); break;
    default: fmt.Underline(WUT::UnderlineType::None); break;
    }
    if (has_fg) fmt.ForegroundColor(unpack(fg));
    if (has_bg) fmt.BackgroundColor(unpack(bg));
    range.CharacterFormat(fmt);
}

// `align`: 0 natural, 1 center, 2 trailing, 3 justified. The marker hangs in the gap the negative
// first-line indent opens, as on every other arm.
void day_texteditor_xaml_apply_paragraph(void *handle, int start, int end, int align, double indent,
                                         double space_before, double space_after, int marker) {
    auto box = box_of(handle);
    if (!box) return;
    auto range = box.Document().GetRange(start, end);
    auto para = range.ParagraphFormat();
    switch (align) {
    case 1: para.Alignment(WUT::ParagraphAlignment::Center); break;
    case 2: para.Alignment(WUT::ParagraphAlignment::Right); break;
    case 3: para.Alignment(WUT::ParagraphAlignment::Justify); break;
    // Natural: RichEdit's own default follows the paragraph's reading direction.
    default: para.Alignment(WUT::ParagraphAlignment::Undefined); break;
    }
    const float gap = marker != 0 ? 18.0f : 0.0f;
    // The hanging indent in that one call: the wrapped lines sit at `indent + gap` and the first
    // line starts `gap` back from them, which is the pair the Qt arm sets as
    // setLeftMargin(indent + gap) / setTextIndent(-gap). The right indent stays as it was.
    para.SetIndents(-gap, static_cast<float>(indent) + gap, para.RightIndent());
    para.SpaceBefore(static_cast<float>(space_before));
    para.SpaceAfter(static_cast<float>(space_after));
    range.ParagraphFormat(para);
}

void day_texteditor_xaml_end_attrs(void *handle) {
    auto box = box_of(handle);
    auto st = state_of(handle);
    if (!box || !st) return;
    box.Document().ApplyDisplayUpdates();
    st->suppress = false;
}

void day_texteditor_xaml_set_selection(void *handle, int start, int end) {
    auto box = box_of(handle);
    auto st = state_of(handle);
    if (!box || !st) return;
    st->suppress = true;
    box.Document().Selection().SetRange(start, end);
    st->suppress = false;
}

// The typing style: a collapsed selection's character format IS what the next character takes —
// native here, as it is on Qt and HarmonyOS.
void day_texteditor_xaml_set_typing(void *handle, double pt, int bold, int italic, int mono,
                                    int underline, int strike, int has_fg, uint32_t fg, int has_bg,
                                    uint32_t bg) {
    auto box = box_of(handle);
    if (!box) return;
    auto sel = box.Document().Selection();
    auto fmt = sel.CharacterFormat();
    fmt.Size(tom_points(pt));
    fmt.Bold(bold ? WUT::FormatEffect::On : WUT::FormatEffect::Off);
    fmt.Italic(italic ? WUT::FormatEffect::On : WUT::FormatEffect::Off);
    fmt.Strikethrough(strike ? WUT::FormatEffect::On : WUT::FormatEffect::Off);
    if (mono) fmt.Name(L"Cascadia Mono");
    switch (underline) {
    case 1: fmt.Underline(WUT::UnderlineType::Single); break;
    case 2: fmt.Underline(WUT::UnderlineType::Double); break;
    case 3: fmt.Underline(WUT::UnderlineType::Dotted); break;
    case 4: fmt.Underline(WUT::UnderlineType::Wave); break;
    default: fmt.Underline(WUT::UnderlineType::None); break;
    }
    if (has_fg) fmt.ForegroundColor(unpack(fg));
    if (has_bg) fmt.BackgroundColor(unpack(bg));
    sel.CharacterFormat(fmt);
}

void day_texteditor_xaml_set_editable(void *handle, int on) {
    if (auto box = box_of(handle)) box.IsReadOnly(on == 0);
}

void day_texteditor_xaml_release(void *handle) { g_state.erase(handle); }

} // extern "C"
