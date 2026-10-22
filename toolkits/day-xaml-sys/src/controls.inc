// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Included by shim.cpp inside the C ABI block. Shared declarations and include order live there.

// ---- label ----

void* day_xaml_label_new(const char* text, int wraps) {
    WUXC::TextBlock t;
    t.Text(hs(text));
    t.TextWrapping(wraps ? WUX::TextWrapping::Wrap : WUX::TextWrapping::NoWrap);
    if (!wraps) { t.MaxLines(1); t.TextTrimming(WUX::TextTrimming::CharacterEllipsis); }
    return boxh(t);
}
void day_xaml_label_set_text(void* h, const char* t) {
    auto tb = elem(h).try_as<WUXC::TextBlock>();
    if (!tb) return;
    // Clear any Inlines first: setting Text while Inlines are populated leaves the old runs in
    // place on some builds, so a label losing its runs would keep rendering the styled version.
    tb.Inlines().Clear();
    tb.Text(hs(t));
}

typedef void (*DayLinkCb)(unsigned long long id, const char* url);

/// The node a link run reports against, and the trampoline. Set by `day_xaml_label_runs_begin`
/// so every Hyperlink appended after it knows where to send its Click (docs/text-runs.md).
///
/// Declared here, above the first use: `runs_begin` below assigns `g_link_node`, and C++ resolves
/// plain identifiers by declaration order, so defining these after it does not compile.
static unsigned long long g_link_node = 0;
static DayLinkCb g_link_cb = nullptr;

/// Begin a styled label: clear the block so runs can be appended (docs/text-runs.md).
void day_xaml_label_runs_begin(void* h, unsigned long long node) {
    g_link_node = node;
    if (auto tb = elem(h).try_as<WUXC::TextBlock>()) {
        tb.Text(hs(""));
        tb.Inlines().Clear();
    }
}

void day_xaml_label_link_cb(DayLinkCb cb) { g_link_cb = cb; }

/// Append one run. `flags` packs bold/italic/mono/strike/hasColour; `link` is empty for a plain
/// run. Each run is an INLINE inside the one TextBlock, so the paragraph still wraps, selects
/// and is read as a single block — which is the whole reason for runs.

void day_xaml_label_runs_add(void* h, const char* text, int flags, unsigned argb, unsigned bg_argb,
                             int scale_permille, const char* link) {
    auto tb = elem(h).try_as<WUXC::TextBlock>();
    if (!tb) return;
    WUXD::Run run;
    run.Text(hs(text));
    if (flags & 1) run.FontWeight(WUTextNames::FontWeights::Bold());
    if (flags & 2) run.FontStyle(WUI::Text::FontStyle::Italic);
    // The family name is the XAML convention for "the fixed-pitch face"; Consolas is the
    // Windows default behind it.
    if (flags & 4) run.FontFamily(WUXM::FontFamily(L"Consolas, Courier New, monospace"));
    if (flags & 16) run.Foreground(brush_bits(argb));
    // TextDecorations is a FLAGS enum, so underline and strikethrough combine in one write —
    // setting the property twice would keep only the second (Windows 10 1809+).
    {
        auto deco = WUI::Text::TextDecorations::None;
        if (flags & 8) deco |= WUI::Text::TextDecorations::Strikethrough;
        if (flags & 64) deco |= WUI::Text::TextDecorations::Underline;
        if (deco != WUI::Text::TextDecorations::None) run.TextDecorations(deco);
    }
    if (scale_permille != 1000 && scale_permille > 0) {
        // `FontSpec::scale` is relative and FontSize is absolute, so the multiply happens against
        // whatever size the TextBlock resolved for itself — which is what a run inherits.
        run.FontSize(tb.FontSize() * scale_permille / 1000.0);
    }
    if (link && link[0]) {
        // A Hyperlink is an inline container: the run goes inside it, so the link sits in the
        // same wrapping paragraph as its neighbors. Activation is Phase 4.
        WUXD::Hyperlink hl;
        hl.Inlines().Append(run);
        // NavigateUri is deliberately NOT set: it would make the shell open the target itself,
        // behind Day's back. Click reports to Rust and the label's `.on_link()` decides — whose
        // default opens the URL through `Toolkit::open_url`, the path every backend shares.
        std::string target(link);
        unsigned long long node = g_link_node;
        hl.Click([node, target](WUXD::Hyperlink const&, WUXD::HyperlinkClickEventArgs const&) {
            if (g_link_cb) g_link_cb(node, target.c_str());
        });
        tb.Inlines().Append(hl);
        return;
    }
    tb.Inlines().Append(run);
    if (flags & 32) {
        // A `Run` has NO Background property — XAML paints text highlights through the
        // TextBlock's TextHighlighters, which take character ranges rather than inlines. The
        // range is where this run's text just landed, which is the running length of everything
        // appended before it.
        int start = 0;
        for (auto const& inl : tb.Inlines()) {
            if (auto r = inl.try_as<WUXD::Run>()) {
                if (r == run) break;
                start += (int)r.Text().size();
            }
        }
        WUXD::TextHighlighter hi;
        hi.Background(brush_bits(bg_argb));
        WUXD::TextRange range{start, (int)run.Text().size()};
        hi.Ranges().Append(range);
        tb.TextHighlighters().Append(hi);
    }
}
// Make a label's text user-selectable (the `.selectable()` modifier, docs/text.md). try_as
// guards a non-TextBlock handle — a no-op rather than a bad cast.
// How a WRAPPED label's lines sit within its width (`LabelProps::align`): 0 leading, 1 center,
// 2 trailing — the same encoding the Qt shim takes. A single-line label fills its own box, so this
// shows only once the text wraps, which is exactly where a centered paragraph differs from a
// leading one. Day mirrors the whole layout under an RTL locale, so leading maps to Left here just
// as it does there; the text's own bidi is the TextBlock's business.
void day_xaml_label_set_align(void* h, int mode) {
    guard([&] {
        if (auto tb = elem(h).try_as<WUXC::TextBlock>()) {
            tb.TextAlignment(mode == 1   ? WUX::TextAlignment::Center
                             : mode == 2 ? WUX::TextAlignment::Right
                                         : WUX::TextAlignment::Left);
        }
    });
}
#ifdef DAY_WINUI
// WinUI 3 owns the island's cursor: its input system sets it on every pointer move, so the Win32
// path below (SetCursor on enter, reasserted on the host's WM_SETCURSOR) is overwritten at once
// and the Cursors page showed the arrow over every shape. The WinUI way is an InputCursor on the
// element's ProtectedCursor, which WinUI shows while the pointer is over the element or anything
// inside it that sets none of its own. The cursor is built from the same HCURSOR the Win32 path
// uses, through the runtime's interop factory (Microsoft.UI.Input.InputCursor.Interop.h, declared
// here because that header needs ABI types the package does not ship), so every code, "hidden"
// included, maps exactly as it does on system XAML.
struct __declspec(uuid("ac6f5065-90c4-46ce-beb7-05e138e54117")) IInputCursorStaticsInterop
    : ::IInspectable {
    virtual HRESULT __stdcall CreateFromHCursor(HCURSOR cursor, void** result) = 0;
};

static winrt::Microsoft::UI::Input::InputCursor winui_cursor_for_code(int code) {
    static std::map<int, winrt::Microsoft::UI::Input::InputCursor> cache;
    auto it = cache.find(code);
    if (it != cache.end()) return it->second;
    HCURSOR hc = nullptr;
    if (code == 17) {
        // Hidden: a fully transparent cursor (AND mask all ones, XOR mask all zeros).
        static BYTE and_mask[32 * 4];
        static BYTE xor_mask[32 * 4];
        std::memset(and_mask, 0xFF, sizeof(and_mask));
        hc = CreateCursor(GetModuleHandleW(nullptr), 0, 0, 32, 32, and_mask, xor_mask);
    } else {
        hc = cursor_for_code(code);
    }
    winrt::Microsoft::UI::Input::InputCursor cursor{ nullptr };
    auto factory = winrt::get_activation_factory<winrt::Microsoft::UI::Input::InputCursor,
                                                 IInputCursorStaticsInterop>();
    void* raw = nullptr;
    if (hc && SUCCEEDED(factory->CreateFromHCursor(hc, &raw)) && raw)
        cursor = { raw, winrt::take_ownership_from_abi };
    cache.emplace(code, cursor);
    return cursor;
}
#endif

void day_xaml_set_cursor(void* h, int code) {
    UIElement e = elem(h);
#ifdef DAY_WINUI
    guard([&] {
        auto prot = e.try_as<WUX::IUIElementProtected>();
        if (!prot) return;
        prot.ProtectedCursor(code == 0 ? winrt::Microsoft::UI::Input::InputCursor{ nullptr }
                                       : winui_cursor_for_code(code));
    });
    return;
#endif
    void* key = winrt::get_abi(e);
    if (code == 0) {
        g_cursor_codes.erase(key);
    } else {
        g_cursor_codes[key] = code;
    }
    if (g_cursor_hooked.insert(key).second) {
        e.PointerEntered([](WF::IInspectable const& s, WUXIn::PointerRoutedEventArgs const&) {
            if (auto el = s.try_as<UIElement>()) apply_cursor_now(cursor_code_over(el));
        });
        e.PointerExited([](WF::IInspectable const& s, WUXIn::PointerRoutedEventArgs const&) {
            auto el = s.try_as<UIElement>();
            if (!el) { apply_cursor_now(0); return; }
            // Back to whatever the nearest decorated ancestor wants, or the arrow.
            WUX::DependencyObject parent = WUXM::VisualTreeHelper::GetParent(el);
            UIElement pe{nullptr};
            if (parent) pe = parent.try_as<UIElement>();
            apply_cursor_now(pe ? cursor_code_over(pe) : 0);
        });
    }
    // A reactive change while the pointer is already inside takes effect at once.
    if (g_cursor_current != 0) apply_cursor_now(cursor_code_over(e));
}
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
// `.secondary()` (TextRole::Secondary): the platform's de-emphasized text color. Applied as a
// style whose setter is a {ThemeResource}, not a brush looked up here, because only a theme
// reference re-resolves when the element's scheme changes (the Appearance override re-themes per
// element, and a brush fetched once would keep the old scheme's gray). A later explicit color is
// a local value, which beats the style; clearing it (alpha 0) falls back to the style again.
void day_xaml_label_set_secondary(void* h) try {
    auto tb = elem(h).try_as<WUXC::TextBlock>();
    if (!tb) return;
    static WUX::Style style{nullptr};
    if (!style) {
        style = WUXMk::XamlReader::Load(
                    L"<Style xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\" "
                    L"TargetType=\"TextBlock\"><Setter Property=\"Foreground\" Value=\""
#ifdef DAY_WINUI
                    L"{ThemeResource TextFillColorSecondaryBrush}"
#else
                    L"{ThemeResource SystemControlForegroundBaseMediumBrush}"
#endif
                    L"\"/></Style>")
                    .as<WUX::Style>();
    }
    tb.Style(style);
} catch (...) {
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

// `ButtonProps::selected`, per button: the state day_xaml_button_set_selected applied, restored
// after a style change and read back as `checked`; and a tinted button's colors, which a
// deselection puts back. Both are dropped with the node (day_xaml_free).
static std::map<void*, bool> g_button_selected;
static std::map<void*, std::pair<unsigned, unsigned>> g_button_tint;

// A plain Button has no checked state, so the selected look is the theme's accent fill and
// the foreground it pairs with it, the treatment a chosen item gets; off restores the stock
// look, or the button's tint. A ToggleButton (none are made here, but a tweak may) takes its own.
static void apply_button_selected(void* h, bool on) {
    auto b = elem(h).try_as<WUXC::Button>();
    if (!b) return;
    if (auto tb = b.try_as<WUXCP::ToggleButton>()) {
        tb.IsChecked(on);
        return;
    }
    if (on) {
        auto res = WUX::Application::Current().Resources();
        auto bg = winrt::box_value(winrt::hstring(L"SystemControlHighlightAccentBrush"));
        auto fg = winrt::box_value(winrt::hstring(L"SystemControlHighlightAltChromeWhiteBrush"));
        if (res.HasKey(bg)) {
            if (auto brush = res.Lookup(bg).try_as<WUXM::Brush>()) b.Background(brush);
        }
        if (res.HasKey(fg)) {
            if (auto brush = res.Lookup(fg).try_as<WUXM::Brush>()) b.Foreground(brush);
        }
        return;
    }
    b.ClearValue(WUXC::Control::BackgroundProperty());
    b.ClearValue(WUXC::Control::ForegroundProperty());
    auto tint = g_button_tint.find(h);
    if (tint != g_button_tint.end()) {
        b.Background(brush_bits(tint->second.first));
        b.Foreground(brush_bits(tint->second.second));
    }
}
void day_xaml_button_set_selected(void* h, int on) {
    guard([&] {
        g_button_selected[h] = on != 0;
        apply_button_selected(h, on != 0);
    });
}

/// Style a button in place: kind 0 automatic, 1 bordered, 2 prominent, 3 tinted.
///
/// Prominent takes the resource set's AccentButtonStyle where it exists. A tint sets Background
/// and Foreground on the Button, which leaves it a Button — its pointer-over and pressed visual
/// states, focus rectangle, keyboard activation and automation peer are all still XAML's.
///
/// The pressed state is the caveat worth knowing: the default template's PointerOver/Pressed
/// brushes come from the theme, and a local Background wins over them, so a tinted button dims
/// less than a stock one. Setting per-state brushes needs a full template, which would replace
/// the control rather than style it.
void day_xaml_button_set_style(void* h, int kind, unsigned argb, unsigned fg_argb) {
    auto b = elem(h).try_as<WUXC::Button>();
    if (!b) return;
    b.ClearValue(WUXC::Control::BackgroundProperty());
    b.ClearValue(WUXC::Control::ForegroundProperty());
    b.ClearValue(WUX::FrameworkElement::StyleProperty());
    g_button_tint.erase(h);
    if (kind == 2) {
        auto res = WUX::Application::Current().Resources();
        auto key = winrt::box_value(winrt::hstring(L"AccentButtonStyle"));
        if (res.HasKey(key)) {
            if (auto style = res.Lookup(key).try_as<WUX::Style>()) b.Style(style);
        }
    } else if (kind == 3) {
        // `brush_bits` is this file's own 0xAARRGGBB → SolidColorBrush helper.
        g_button_tint[h] = { argb, fg_argb };
        b.Background(brush_bits(argb));
        b.Foreground(brush_bits(fg_argb));
    }
    // A selected button keeps its selected look across a restyle.
    auto selected = g_button_selected.find(h);
    if (selected != g_button_selected.end() && selected->second) apply_button_selected(h, true);
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
    // The default style reserves 154px (MinWidth) for the On/Off text beside the knob. Day's
    // toggle has no such text, so the control is the knob alone (40px), like every other
    // toolkit's switch: with the floor left in place a `labeled` pair in a narrow wrapped row
    // overran its container by the empty text column (conformance
    // `wrapped-controls-stay-inside-row`).
    t.MinWidth(0.0);
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
    // WinUI's unloaded template has no intrinsic size. Keep the built-in spinner
    // at the same 20-DIP size as the activity piece, including its first measure.
    r.MinWidth(20.0);
    r.MinHeight(20.0);
    return boxh(r);
}
int day_xaml_progress_is_ring(void* h) {
    return elem(h).try_as<WUXC::ProgressRing>() ? 1 : 0;
}
void day_xaml_progress_set(void* h, int value) {
    if (auto b = elem(h).try_as<WUXC::ProgressBar>())
        if (static_cast<int>(b.Value()) != value) b.Value(value);
}

// ---- the navigation suite (docs/navigation.md): a Pivot owns its page content ----

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
// Data-driven tabs (docs/navigation.md): drop a page's PivotItem. The item must release its
// Content first — the page widget survives the removal (Day owns and disposes it).
void day_xaml_tabs_remove_page(void* tabs, void* page) {
    auto p = elem(tabs).as<WUXC::Pivot>();
    auto items = p.Items();
    WF::IInspectable target = elem(page);
    for (uint32_t i = 0; i < items.Size(); ++i) {
        auto item = items.GetAt(i).try_as<WUXC::PivotItem>();
        if (item && item.Content() == target) {
            item.Content(nullptr);
            items.RemoveAt(i);
            return;
        }
    }
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

