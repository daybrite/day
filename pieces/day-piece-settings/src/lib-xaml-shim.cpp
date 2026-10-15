// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The settings piece's C++/WinRT shim: one settings-card row (the Windows 11 Settings look, what
// the CommunityToolkit's SettingsCard draws) composed from stock XAML, boxed into a Day handle via
// the day_xaml_box/day_xaml_unbox functions that day-xaml-sys exports, so this piece carries its
// own native XAML code with no edits to day's toolkit crates (docs/extending.md §5, the
// day-piece-searchfield recipe).
//
// A row takes one of two shapes, told apart by the handle's type:
//
// A row that hosts a Day control (toggle, picker, slider, field, button, …) is a Canvas, because
// day positions every child it mounts by absolute frame (Canvas.Left/Top + Width/Height,
// `day_xaml_set_geometry`) and `day_xaml_add_child` appends into a Panel; the control is mounted
// as such a child and placed at the trailing edge by the piece's own Layout. The Canvas carries
// two children of its own, kept in front of the handle at fixed indices so day's appended
// children never disturb them:
//
//   [0] Border  — the card chrome: CardBackgroundFillColorDefaultBrush fill,
//                 CardStrokeColorDefaultBrush 1px stroke, 4px radius; tracks the Canvas size.
//   [1] Grid    — the text column: header (body text) over description (caption, secondary).
//                 Its width is whatever the Layout last measured it at
//                 (`day_settings_xaml_card_measure`), its height the row's.
//
// A clickable row (navigate, link) hosts no Day control and is a Button: a ButtonBase is a tab
// stop, activates on Enter/Space, draws the system focus visual, exposes a ButtonAutomationPeer
// with the Invoke pattern, and carries the template's pointer-over/pressed states, none of which
// a Canvas with a Tapped handler gives a keyboard or assistive-technology user. The Button is
// styled as the card (the same fill/stroke brushes, 1px border, 4px radius) and its Content is
// the same text Grid with the action glyph (chevron / open-in-new) in a trailing Auto column;
// AutomationProperties.Name is the row title (the Grid content would otherwise leave the peer
// nameless) and HelpText the description. Click reports through the same `cb(id)` the Canvas
// row's Tapped did, so the Rust side sees one Tap path for both shapes.
//
// Theme brushes are applied as Styles whose setters are {ThemeResource} references, loaded through
// XamlReader: only a theme reference re-resolves when the element's scheme changes (the app's
// Appearance override re-themes per element, and a brush looked up once would keep the old
// scheme's color — the same reasoning day-xaml-sys gives for `.secondary()`). Where the resource
// set predates the card brushes (system XAML before Windows 11) the loader throws, and the row
// falls back to fixed translucent neutrals keyed on `day_xaml_is_dark()`.
//
// Windows-only; compiled by build.rs and linked alongside day-xaml-sys. Not verified locally.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.UI.h>
// One shim, two XAML stacks (docs/winui.md): windows-xaml builds it against system XAML,
// windows-winui (DAY_WINUI) against WinUI 3. The namespaces differ; the controls do not.
#ifdef DAY_WINUI
#define DAY_XAML_NS winrt::Microsoft::UI::Xaml
#include <winrt/Microsoft.UI.Xaml.h>
#include <winrt/Microsoft.UI.Xaml.Automation.h>
#include <winrt/Microsoft.UI.Xaml.Controls.h>
#include <winrt/Microsoft.UI.Xaml.Controls.Primitives.h>
#include <winrt/Microsoft.UI.Xaml.Markup.h>
#include <winrt/Microsoft.UI.Xaml.Media.h>
#else
#define DAY_XAML_NS winrt::Windows::UI::Xaml
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Automation.h>
#include <winrt/Windows.UI.Xaml.Controls.h>
#include <winrt/Windows.UI.Xaml.Controls.Primitives.h>
#include <winrt/Windows.UI.Xaml.Markup.h>
#include <winrt/Windows.UI.Xaml.Media.h>
#endif

#include <windows.h>

#include <cmath>
#include <cstdint>
#include <limits>
#include <string>
#include <vector>

using namespace winrt;
namespace WF = winrt::Windows::Foundation;
namespace WUX = DAY_XAML_NS;
namespace WUXC = DAY_XAML_NS::Controls;
namespace WUXM = DAY_XAML_NS::Media;
namespace WUXMk = DAY_XAML_NS::Markup;
namespace WUXA = DAY_XAML_NS::Automation;

// Exported by day-xaml-sys (already linked into the app): the handle seam, and the effective
// scheme (system, DAY_THEME force, or the app's Appearance pick) for the fallback colors.
extern "C" void *day_xaml_box(void *iinspectable_abi);
extern "C" void *day_xaml_unbox(void *handle);
extern "C" int day_xaml_is_dark();

// SettingsCard metrics (the CommunityToolkit's SettingsCard resources): 16px padding all round,
// a 68px minimum height, a 4px radius. The Rust Layout holds the same numbers.
static constexpr double kPad = 16.0;
static constexpr double kRadius = 4.0;
static constexpr wchar_t kXamlNs[] = L"http://schemas.microsoft.com/winfx/2006/xaml/presentation";

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

static winrt::Windows::UI::Color color_argb(unsigned int argb) {
    winrt::Windows::UI::Color c;
    c.A = static_cast<uint8_t>((argb >> 24) & 0xFF);
    c.R = static_cast<uint8_t>((argb >> 16) & 0xFF);
    c.G = static_cast<uint8_t>((argb >> 8) & 0xFF);
    c.B = static_cast<uint8_t>(argb & 0xFF);
    return c;
}

static WUXM::SolidColorBrush solid(unsigned int argb) {
    return WUXM::SolidColorBrush(color_argb(argb));
}

// A Style of {ThemeResource} setters for `target` ("Border", "TextBlock", "FontIcon"), or null
// when the loader rejects it (an unknown resource key on an older resource set).
static WUX::Style theme_style(wchar_t const *target, std::wstring const &setters) {
    try {
        std::wstring xaml = L"<Style xmlns=\"";
        xaml += kXamlNs;
        xaml += L"\" TargetType=\"";
        xaml += target;
        xaml += L"\">";
        xaml += setters;
        xaml += L"</Style>";
        return WUXMk::XamlReader::Load(winrt::hstring{xaml}).as<WUX::Style>();
    } catch (...) {
        return nullptr;
    }
}

static std::wstring setter(wchar_t const *property, wchar_t const *key) {
    std::wstring s = L"<Setter Property=\"";
    s += property;
    s += L"\" Value=\"{ThemeResource ";
    s += key;
    s += L"}\"/>";
    return s;
}

// The card chrome's brushes: the theme's card fill and stroke, else translucent neutrals that
// read over either scheme's page ground (alpha-over-ground, like day-xaml-sys's section card
// fallback). The fallback is re-applied when the element's theme flips.
static void apply_chrome(WUXC::Border const &border) {
    static WUX::Style style = theme_style(
        L"Border", setter(L"Background", L"CardBackgroundFillColorDefaultBrush") +
                       setter(L"BorderBrush", L"CardStrokeColorDefaultBrush"));
    if (style) {
        border.Style(style);
        return;
    }
    auto paint = [](WUXC::Border const &b) {
        bool dark = day_xaml_is_dark() != 0;
        b.Background(solid(dark ? 0x0F'FFFFFFu : 0xB3'FFFFFFu));
        b.BorderBrush(solid(dark ? 0x19'000000u : 0x0F'000000u));
    };
    paint(border);
    border.ActualThemeChanged([paint](WUX::FrameworkElement const &sender, WF::IInspectable const &) mutable {
        if (auto b = sender.try_as<WUXC::Border>())
            paint(b);
    });
}

// The description's de-emphasized color and the header's destructive red, as theme references.
static void apply_secondary(WUXC::TextBlock const &tb) {
    static WUX::Style style = theme_style(L"TextBlock",
#ifdef DAY_WINUI
                                          setter(L"Foreground", L"TextFillColorSecondaryBrush")
#else
                                          setter(L"Foreground", L"SystemControlForegroundBaseMediumBrush")
#endif
    );
    if (style)
        tb.Style(style);
    else
        tb.Foreground(solid(day_xaml_is_dark() != 0 ? 0xC5'FFFFFFu : 0x9E'000000u));
}

static void apply_destructive(WUXC::TextBlock const &tb) {
    static WUX::Style style = theme_style(L"TextBlock", setter(L"Foreground", L"SystemFillColorCriticalBrush"));
    if (style)
        tb.Style(style);
    else
        tb.Foreground(solid(day_xaml_is_dark() != 0 ? 0xFF'FF99A4u : 0xFF'C42B1Cu));
}

static void apply_glyph_color(WUXC::FontIcon const &icon) {
    static WUX::Style style = theme_style(L"FontIcon",
#ifdef DAY_WINUI
                                          setter(L"Foreground", L"TextFillColorSecondaryBrush")
#else
                                          setter(L"Foreground", L"SystemControlForegroundBaseMediumBrush")
#endif
    );
    if (style)
        icon.Style(style);
    else
        icon.Foreground(solid(day_xaml_is_dark() != 0 ? 0xC5'FFFFFFu : 0x9E'000000u));
}

// The clickable card's Button, styled as the card: the same themed fill and stroke the Canvas
// row's Border takes, as {ThemeResource} setters on a Button Style (the default template still
// applies — a Style without a Template keeps the control's own — so the pointer-over and pressed
// states, the focus visual and the automation peer are all still the Button's), else the same
// translucent fallback re-applied on a theme flip. No padding of its own: the content Grid
// carries the card's 16px side inset so its trailing glyph ends where the Canvas row's does.
static void apply_card_button(WUXC::Button const &button) {
    button.HorizontalContentAlignment(WUX::HorizontalAlignment::Stretch);
    button.VerticalContentAlignment(WUX::VerticalAlignment::Stretch);
    button.Padding(WUX::Thickness{0, 0, 0, 0});
    button.BorderThickness(WUX::Thickness{1, 1, 1, 1});
    try {
        // Control.CornerRadius (system XAML 1809+; always in WinUI 3).
        button.CornerRadius(WUX::CornerRadius{kRadius, kRadius, kRadius, kRadius});
    } catch (...) {
    }
    static WUX::Style style = theme_style(
        L"Button", setter(L"Background", L"CardBackgroundFillColorDefaultBrush") +
                       setter(L"BorderBrush", L"CardStrokeColorDefaultBrush"));
    if (style) {
        button.Style(style);
        return;
    }
    auto paint = [](WUXC::Button const &b) {
        bool dark = day_xaml_is_dark() != 0;
        b.Background(solid(dark ? 0x0F'FFFFFFu : 0xB3'FFFFFFu));
        b.BorderBrush(solid(dark ? 0x19'000000u : 0x0F'000000u));
    };
    paint(button);
    button.ActualThemeChanged([paint](WUX::FrameworkElement const &sender, WF::IInspectable const &) mutable {
        if (auto b = sender.try_as<WUXC::Button>())
            paint(b);
    });
}

// What UI Automation reads for a clickable card: the title names it (the Grid content gives the
// ButtonAutomationPeer no name of its own), the description is its help text. A row without a
// title (none is clickable today) is named by its description instead.
static void name_card_button(WUXC::Button const &button, winrt::hstring const &header,
                             winrt::hstring const &description) {
    WUXA::AutomationProperties::SetName(button, header.empty() ? description : header);
    WUXA::AutomationProperties::SetHelpText(button, header.empty() ? winrt::hstring{} : description);
}

// The parts of a card, found back from the handle: a hosting row's Canvas by its fixed child
// indices, a clickable row's Button through its Content. Exactly one of `root` and `button` is
// set; `text` is set for both shapes.
struct Card {
    WUXC::Canvas root{nullptr};
    WUXC::Button button{nullptr};
    WUXC::Border chrome{nullptr};
    WUXC::Grid text{nullptr};
};

static Card card_of(void *handle) {
    Card c;
    try {
        WUX::UIElement e{nullptr};
        winrt::copy_from_abi(e, day_xaml_unbox(handle));
        if (auto b = e.try_as<WUXC::Button>()) {
            c.button = b;
            c.text = b.Content().try_as<WUXC::Grid>();
            return c;
        }
        c.root = e.try_as<WUXC::Canvas>();
        if (!c.root)
            return c;
        auto kids = c.root.Children();
        if (kids.Size() >= 2) {
            c.chrome = kids.GetAt(0).try_as<WUXC::Border>();
            c.text = kids.GetAt(1).try_as<WUXC::Grid>();
        }
    } catch (...) {
    }
    return c;
}

// Header and description live in the text grid's StackPanel (column 0): [0] header, [1]
// description.
static WUXC::TextBlock text_block(WUXC::Grid const &text, uint32_t index) {
    if (!text || text.Children().Size() == 0)
        return nullptr;
    auto stack = text.Children().GetAt(0).try_as<WUXC::StackPanel>();
    if (!stack || stack.Children().Size() <= index)
        return nullptr;
    return stack.Children().GetAt(index).try_as<WUXC::TextBlock>();
}

// The text column both shapes share: a Grid with header (body text) over description (caption,
// secondary) in a Star column and, when `glyph` is set, the action icon in a trailing Auto
// column. Not hit-testable: the painted surface underneath it (the Canvas row's chrome, the
// Button's own background) takes the press, so a click anywhere in the card lands.
static WUXC::Grid build_text(const char *header, const char *description, unsigned glyph,
                             int destructive) {
    WUXC::Grid text;
    WUXC::ColumnDefinition col_text;
    col_text.Width(WUX::GridLength{1, WUX::GridUnitType::Star});
    WUXC::ColumnDefinition col_glyph;
    col_glyph.Width(WUX::GridLength{0, WUX::GridUnitType::Auto});
    text.ColumnDefinitions().Append(col_text);
    text.ColumnDefinitions().Append(col_glyph);
    text.IsHitTestVisible(false);

    WUXC::StackPanel stack;
    stack.Orientation(WUXC::Orientation::Vertical);
    stack.VerticalAlignment(WUX::VerticalAlignment::Center);
    WUXC::Grid::SetColumn(stack, 0);

    // An empty header collapses: the row then shows its description alone (a row whose trailing
    // button or link already carries the title).
    WUXC::TextBlock head;
    head.Text(hs(header));
    head.TextWrapping(WUX::TextWrapping::Wrap);
    if (destructive)
        apply_destructive(head);
    head.Visibility((header && *header) ? WUX::Visibility::Visible : WUX::Visibility::Collapsed);
    stack.Children().Append(head);

    WUXC::TextBlock desc;
    desc.Text(hs(description));
    desc.FontSize(12.0); // CaptionTextBlockStyle
    desc.TextWrapping(WUX::TextWrapping::Wrap);
    apply_secondary(desc);
    desc.Visibility((description && *description) ? WUX::Visibility::Visible : WUX::Visibility::Collapsed);
    stack.Children().Append(desc);
    text.Children().Append(stack);

    if (glyph != 0) {
        WUXC::FontIcon icon;
        wchar_t g[2] = {static_cast<wchar_t>(glyph), L'\0'};
        icon.Glyph(winrt::hstring{g});
        icon.FontSize(glyph == 0xE974u ? 12.0 : 16.0);
        icon.VerticalAlignment(WUX::VerticalAlignment::Center);
        icon.Margin(WUX::Thickness{kPad, 0, 0, 0});
        apply_glyph_color(icon);
        WUXC::Grid::SetColumn(icon, 1);
        text.Children().Append(icon);
    }
    return text;
}

extern "C" {

// Standard GroupIcons have no bundled-image name. Keep their native icon slot populated
// alongside image-backed groups, including after localization rebuilds the menu items.
void day_settings_xaml_sidebar_symbols(void *abi, const char *paths) {
    try {
        WUX::UIElement element{nullptr};
        winrt::copy_from_abi(element, abi);
        auto nav = element.try_as<WUXC::NavigationView>();
        if (!nav) return;
        std::vector<std::string> outlines;
        std::string joined = paths ? paths : "";
        size_t start = 0;
        while (true) {
            auto end = joined.find('\x1f', start);
            outlines.push_back(joined.substr(start, end == std::string::npos ? end : end - start));
            if (end == std::string::npos) break;
            start = end + 1;
        }
        auto paint = [outlines](auto const& items) {
            for (uint32_t i = 0; i < items.Size() && i < outlines.size(); ++i) {
                if (outlines[i].empty()) continue;
                auto item = items.GetAt(i).template try_as<WUXC::NavigationViewItem>();
                if (!item || item.Icon()) continue;
                // Day's symbol SVGs use a 24px viewport and even-odd filled geometry. Match
                // both here: PathIcon does not scale its Data to NavigationView's 16px slot.
                // Leave Foreground inherited for native theme and selection-state colors.
                auto xaml = std::string("<PathIcon xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\" Data=\"F0 ")
                    + outlines[i] + "\"/>";
                auto icon = WUXMk::XamlReader::Load(hs(xaml.c_str())).as<WUXC::PathIcon>();
                WUXM::ScaleTransform scale;
                scale.ScaleX(16.0 / 24.0);
                scale.ScaleY(16.0 / 24.0);
                icon.Data().Transform(scale);
                item.Icon(icon);
            }
        };
        paint(nav.MenuItems());
        auto observable = nav.MenuItems().as<WF::Collections::IObservableVector<WF::IInspectable>>();
        observable.VectorChanged([paint](auto const& items, auto const&) { paint(items); });
    } catch (...) {
    }
}

// A card row. `glyph` is a Segoe symbol code point for the trailing action icon (0 = none;
// U+E974 ChevronRightMed on a navigate row, U+E8A7 OpenInNewWindow on a link row).
// `clickable` rows are Buttons that report a Click through `cb(id)`; the others are Canvases
// that host the row's Day control. `destructive` paints the header red.
void *day_settings_xaml_card_new(const char *header, const char *description, unsigned glyph,
                                 int clickable, int destructive, uint64_t id,
                                 void (*cb)(uint64_t)) {
    WUXC::Grid text = build_text(header, description, glyph, destructive);

    if (clickable) {
        WUXC::Button button;
        apply_card_button(button);
        // The card's side inset, on the content: the Button has no Padding of its own, so day's
        // stated Width/Height (set_geometry) is the card's outer size, border included, and the
        // Grid stretches to the inner width less the inset — what the Canvas row's Layout
        // measures the text column at (`text_w` = row width − 2·kPad).
        text.Margin(WUX::Thickness{kPad - 1, 0, kPad - 1, 0});
        text.HorizontalAlignment(WUX::HorizontalAlignment::Stretch);
        text.VerticalAlignment(WUX::VerticalAlignment::Stretch);
        button.Content(text);
        name_card_button(button, hs(header), hs(description));
        if (cb)
            button.Click([id, cb](WF::IInspectable const &, WUX::RoutedEventArgs const &) { cb(id); });
        return day_xaml_box(winrt::get_abi(button));
    }

    WUXC::Canvas root;

    // [0] The chrome, tracking the Canvas size day states through set_geometry.
    WUXC::Border chrome;
    chrome.BorderThickness(WUX::Thickness{1, 1, 1, 1});
    chrome.CornerRadius(WUX::CornerRadius{kRadius, kRadius, kRadius, kRadius});
    WUXC::Canvas::SetLeft(chrome, 0);
    WUXC::Canvas::SetTop(chrome, 0);
    chrome.Width(0);
    chrome.Height(0);
    apply_chrome(chrome);
    root.Children().Append(chrome);

    // [1] The text column, at the card's side inset; its width is the Layout's (set in
    // `day_settings_xaml_card_measure`), its height the row's.
    WUXC::Canvas::SetLeft(text, kPad);
    WUXC::Canvas::SetTop(text, 0);
    root.Children().Append(text);

    // day sizes the Canvas via set_geometry (Width/Height), which fires SizeChanged; the chrome
    // grows with it and the text column takes the row height.
    root.SizeChanged([chrome, text](WF::IInspectable const &, WUX::SizeChangedEventArgs const &e) mutable {
        chrome.Width(e.NewSize().Width);
        chrome.Height(e.NewSize().Height);
        text.Height(e.NewSize().Height);
    });

    return day_xaml_box(winrt::get_abi(root));
}

// Live text (a locale change re-resolves the row's sources); a clickable card's automation name
// and help text follow it.
void day_settings_xaml_card_set_text(void *handle, const char *header, const char *description) {
    try {
        auto card = card_of(handle);
        auto h = hs(header);
        auto d = hs(description);
        if (auto head = text_block(card.text, 0)) {
            if (head.Text() != h)
                head.Text(h);
            head.Visibility(h.empty() ? WUX::Visibility::Collapsed : WUX::Visibility::Visible);
        }
        if (auto desc = text_block(card.text, 1)) {
            if (desc.Text() != d)
                desc.Text(d);
            desc.Visibility(d.empty() ? WUX::Visibility::Collapsed : WUX::Visibility::Visible);
        }
        if (card.button)
            name_card_button(card.button, h, d);
    } catch (...) {
    }
}

// The text column's natural size at `text_w` (< 0 = unconstrained): what the Layout adds its
// padding to for the row height. On a hosting (Canvas) row the width also becomes the column's
// layout width, so the text wraps clear of the control the Layout places at the trailing edge;
// the Layout calls this again from `place` with the final width, so the last call always states
// the real column. A clickable (Button) row has no control and stretches its content to the
// card's inner width itself, so only the measurement is taken there.
void day_settings_xaml_card_measure(void *handle, double text_w, double *out_w, double *out_h) {
    *out_w = 0;
    *out_h = 0;
    try {
        auto card = card_of(handle);
        if (!card.text)
            return;
        const double kAuto = std::numeric_limits<double>::quiet_NaN();
        double saved_h = card.text.Height();
        card.text.Width(kAuto);
        card.text.Height(kAuto);
        float fw = text_w < 0 ? std::numeric_limits<float>::infinity() : static_cast<float>(text_w);
        card.text.Measure(WF::Size{fw, std::numeric_limits<float>::infinity()});
        auto d = card.text.DesiredSize();
        *out_w = d.Width;
        *out_h = d.Height;
        if (text_w >= 0 && card.root)
            card.text.Width(text_w);
        if (!std::isnan(saved_h))
            card.text.Height(saved_h);
    } catch (...) {
    }
}

} // extern "C"
