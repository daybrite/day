// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Included by shim.cpp inside the C ABI block. Shared declarations and include order live there.

// A node's accessibility annotations (docs/accessibility.md), each written only when set: a null
// or empty string, role 0 and hidden 0 leave the element's own answer alone, so a repeat call
// with the same values changes nothing. `role` is day_spec::Role's declaration order (0 None,
// 1 Button, 2 Toggle, 3 Slider, 4 TextInput, 5 Heading, 6 Image, 7 Meter, 8 Group, 9 Tree,
// 10 TreeItem; the same table is in day-xaml's `set_a11y`); `level` is the 1-based heading level
// and is read only with role 5. XAML cannot retype an element, so a role lands as what UIA does
// carry: a heading level (what a screen reader's heading navigation walks) or a localized
// control-type string, the type name spoken in place of the element's own. Those strings are
// English: the shim has no catalog, and a UIA control type is a display string, not a key. The
// value rides ItemStatus, UIA's "current status of this item" property, so a native control's
// own Value pattern is never touched. Hidden switches the element to the Raw view, which takes
// it and its subtree out of the control and content views a screen reader walks while leaving it
// on screen. Only the label and automation id reach a bare Canvas: panels create no automation
// peer of their own, so the annotations an app puts on a plain container are held but never
// read; Day's controls, text and images all carry a peer.
void day_xaml_set_a11y(void* h, const char* label, const char* hint, const char* value, int role,
                       int level, int hidden) {
    guard([&] {
        if (!h) return;
        auto el = elem(h);
        if (!el) return;
        if (label && *label) WUXA::AutomationProperties::SetName(el, hs(label));
        reinterpret_cast<Node*>(h)->app_named = label && *label;
        if (hint && *hint) WUXA::AutomationProperties::SetHelpText(el, hs(hint));
        if (value && *value) WUXA::AutomationProperties::SetItemStatus(el, hs(value));
        if (role == 5) {
            // Level1..Level9 in order; a level outside 1..9 takes the nearest end.
            WUXAP::AutomationHeadingLevel hl = WUXAP::AutomationHeadingLevel::Level1;
            switch (level) {
                case 2: hl = WUXAP::AutomationHeadingLevel::Level2; break;
                case 3: hl = WUXAP::AutomationHeadingLevel::Level3; break;
                case 4: hl = WUXAP::AutomationHeadingLevel::Level4; break;
                case 5: hl = WUXAP::AutomationHeadingLevel::Level5; break;
                case 6: hl = WUXAP::AutomationHeadingLevel::Level6; break;
                case 7: hl = WUXAP::AutomationHeadingLevel::Level7; break;
                case 8: hl = WUXAP::AutomationHeadingLevel::Level8; break;
                default:
                    if (level >= 9) hl = WUXAP::AutomationHeadingLevel::Level9;
                    break;
            }
            WUXA::AutomationProperties::SetHeadingLevel(el, hl);
        } else if (role != 0) {
            const wchar_t* type = nullptr;
            switch (role) {
                case 1: type = L"button"; break;
                case 2: type = L"toggle switch"; break;
                case 3: type = L"slider"; break;
                case 4: type = L"text box"; break;
                case 6: type = L"image"; break;
                case 7: type = L"progress bar"; break;
                case 8: type = L"group"; break;
                case 9: type = L"tree"; break;
                case 10: type = L"tree item"; break;
                case 11: type = L"tab"; break;
                case 12: type = L"tab list"; break;
                default: break;
            }
            if (type) {
                WUXA::AutomationProperties::SetLocalizedControlType(el, winrt::hstring{ type });
            }
        }
        if (hidden) {
            WUXA::AutomationProperties::SetAccessibilityView(el, WUXAP::AccessibilityView::Raw);
        }
    });
}

// What a node's native control reports right now, read back for `Toolkit::read_native` (the
// dayscript a11y audit and the native assertions). Every member is the control's own answer; a
// member this element does not carry, or that could not be read, keeps its "not read" value: 0
// in a has_* flag, -1 in a tri-state, a null string. Strings are heap copies the caller releases
// with day_xaml_string_free. The layout is mirrored by day-xaml-sys's `DayXamlNative`.
struct DayXamlNative {
    int found;         // label, value, identifier and role below were all read
    int role;          // day_xaml_set_a11y's role table; 0 when the element carries none of it
    int level;         // the heading level, with role 5
    char* label;       // the automation peer's name (AutomationProperties.Name when set)
    char* value;       // ItemStatus, where day_xaml_set_a11y puts a value
    char* identifier;  // AutomationId, where day_xaml_set_name puts `.id()`
    char* text;        // the text on screen; null for a PasswordBox and non-text elements
    int has_number;
    double number;     // a Slider's Value, or a determinate progress bar's fraction
    int checked;       // a ToggleSwitch's IsOn / a ToggleButton's IsChecked
    int enabled;       // Control.IsEnabled
    int visible;       // Visibility along the visual parent chain
    int has_frame;
    double x, y, w, h; // bounds in the window's content canvas, DIPs
};

static char* native_dup(std::string const& s) { return _strdup(s.c_str()); }
extern "C++" { // inside the extern "C" block; defined with the picker section below
static void picker_read_native(void* h, UIElement const& el, DayXamlNative* out);
} // extern "C++"
static char* native_dup_nonempty(winrt::hstring const& s) {
    return s.empty() ? nullptr : native_dup(u8(s));
}

// A TextBlock's inlines as the plain text they draw: Runs, Spans (a link run is a Hyperlink, a
// Span) and LineBreaks. False on any other inline, whose text this cannot vouch for.
static bool inline_text(WUXD::InlineCollection const& inlines, std::wstring& out) {
    for (auto const& inl : inlines) {
        if (auto run = inl.try_as<WUXD::Run>()) {
            winrt::hstring t = run.Text();
            out.append(t.c_str(), t.size());
        } else if (auto span = inl.try_as<WUXD::Span>()) {
            if (!inline_text(span.Inlines(), out)) return false;
        } else if (inl.try_as<WUXD::LineBreak>()) {
            out.push_back(L'\n');
        } else {
            return false;
        }
    }
    return true;
}

// The window content canvas `d` is, if it is one: what day's frames are relative to.
static bool is_content_canvas(WUX::DependencyObject const& d) {
    if (g_app && g_app->content && g_app->content == d) return true;
    for (auto const& [hwnd, sw] : g_sec_windows) {
        if (sw && sw->content && sw->content == d) return true;
    }
    return false;
}

void day_xaml_read_native(void* h, DayXamlNative* out) {
    if (!out) return;
    *out = DayXamlNative{};
    out->checked = -1;
    out->enabled = -1;
    out->visible = -1;
    if (!h) return;
    UIElement el{ nullptr };
    guard([&] { el = elem(h); });
    if (!el) return;

    // The accessibility group, all or nothing.
    guard([&] {
        auto peer = WUXAP::FrameworkElementAutomationPeer::CreatePeerForElement(el);
        winrt::hstring name = peer ? peer.GetName() : WUXA::AutomationProperties::GetName(el);
        winrt::hstring status =
            peer ? peer.GetItemStatus() : WUXA::AutomationProperties::GetItemStatus(el);
        winrt::hstring id = WUXA::AutomationProperties::GetAutomationId(el);
        int role = 0, level = 0;
        auto heading = WUXA::AutomationProperties::GetHeadingLevel(el);
        if (heading != WUXAP::AutomationHeadingLevel::None) {
            role = 5;
            // Level1..Level9 are 1..9.
            level = static_cast<int>(heading);
        } else {
            // The localized control type is where day_xaml_set_a11y writes every other role,
            // so the same table read backwards; a string it did not write names no Day role.
            winrt::hstring type = WUXA::AutomationProperties::GetLocalizedControlType(el);
            if (type == L"button") role = 1;
            else if (type == L"toggle switch") role = 2;
            else if (type == L"slider") role = 3;
            else if (type == L"text box") role = 4;
            else if (type == L"image") role = 6;
            else if (type == L"progress bar") role = 7;
            else if (type == L"group") role = 8;
            else if (type == L"tree") role = 9;
            else if (type == L"tree item") role = 10;
        }
        out->label = native_dup_nonempty(name);
        out->value = native_dup_nonempty(status);
        out->identifier = native_dup_nonempty(id);
        out->role = role;
        out->level = level;
        out->found = 1;
    });

    // The control's own state, each read on its own so one refusal costs only itself.
    guard([&] {
        if (auto tb = el.try_as<WUXC::TextBlock>()) {
            std::wstring s;
            auto inlines = tb.Inlines();
            if (inlines.Size() == 0) {
                out->text = native_dup(u8(tb.Text()));
            } else if (inline_text(inlines, s)) {
                out->text = native_dup(u8(winrt::hstring(s)));
            }
        } else if (auto box = el.try_as<WUXC::TextBox>()) {
            // A multi-line TextBox separates lines with CR; Day's text uses LF (see `lf`).
            out->text = native_dup(lf(u8(box.Text())));
        } else if (auto button = el.try_as<WUXC::Button>()) {
            auto content = button.Content();
            if (auto s = content.try_as<WF::IReference<winrt::hstring>>()) {
                out->text = native_dup(u8(s.Value()));
            } else if (auto panel = content.try_as<WUXC::StackPanel>()) {
                // An icon button (day_xaml_button_set_content): the glyph and one TextBlock,
                // or the glyph alone when the title is spoken but not shown.
                WUXC::TextBlock only{ nullptr };
                int count = 0;
                for (auto const& kid : panel.Children()) {
                    if (auto t = kid.try_as<WUXC::TextBlock>()) {
                        only = t;
                        ++count;
                    }
                }
                if (count == 1) out->text = native_dup(u8(only.Text()));
            }
        }
    });
    guard([&] {
        if (auto slider = el.try_as<WUXC::Slider>()) {
            // Minimum/Maximum are Day's own range (day_xaml_slider_new), so Value is in it.
            out->number = slider.Value();
            out->has_number = 1;
        } else if (auto bar = el.try_as<WUXC::ProgressBar>()) {
            // day_xaml_progress_new's ticks: 0..1000 for Day's 0..1. A bar on any other range
            // is not one Day made, and its scale is not known here.
            if (!bar.IsIndeterminate() && bar.Minimum() == 0.0 && bar.Maximum() == 1000.0) {
                out->number = bar.Value() / 1000.0;
                out->has_number = 1;
            }
        }
    });
    guard([&] {
        if (auto sw = el.try_as<WUXC::ToggleSwitch>()) {
            out->checked = sw.IsOn() ? 1 : 0;
        } else if (auto tb = el.try_as<WUXCP::ToggleButton>()) {
            if (auto on = tb.IsChecked()) out->checked = on.Value() ? 1 : 0;
        } else if (el.try_as<WUXC::Button>()) {
            // A Day button's selected state (day_xaml_button_set_selected): a plain Button has
            // no checked property, so the state applied is what is read.
            auto it = g_button_selected.find(h);
            if (it != g_button_selected.end()) out->checked = it->second ? 1 : 0;
        }
    });
    guard([&] {
        if (auto c = el.try_as<WUXC::Control>()) out->enabled = c.IsEnabled() ? 1 : 0;
    });
    // A picker's choice as displayed, and a button-style picker's enabled state (its panel is
    // not a Control, so the read above left it unset).
    guard([&] { picker_read_native(h, el, out); });

    // Visibility up the visual parents to the window's content canvas. A collapsed element or
    // ancestor is hidden wherever the chain ends; an all-visible chain that never reaches a
    // content canvas (parked, not yet in the tree) says nothing.
    guard([&] {
        bool shown = true;
        WUXC::Canvas host{ nullptr };
        WUX::DependencyObject d = el;
        while (d) {
            if (auto u = d.try_as<UIElement>()) {
                if (u.Visibility() != WUX::Visibility::Visible) shown = false;
            }
            if (is_content_canvas(d)) {
                host = d.as<WUXC::Canvas>();
                break;
            }
            d = WUXM::VisualTreeHelper::GetParent(d);
        }
        if (!shown) {
            out->visible = 0;
            return;
        }
        if (!host) return;
        out->visible = 1;
        auto fe = el.try_as<FrameworkElement>();
        if (!fe) return;
        // The bounds, not the transformed origin: an element under a mirrored (RTL)
        // NavigationView maps its own top left to the top RIGHT of its box.
        auto r = el.TransformToVisual(host).TransformBounds(
            WF::Rect{ 0.0f, 0.0f, static_cast<float>(fe.ActualWidth()),
                      static_cast<float>(fe.ActualHeight()) });
        out->x = r.X;
        out->y = r.Y;
        out->w = r.Width;
        out->h = r.Height;
        out->has_frame = 1;
    });
}

// The first element at or under `e` that has an automation peer, at most `depth` levels down.
// A notification is raised through SOME peer in the window's tree — which one does not matter
// to the screen reader — but the window's root is a Canvas, and panels create no peer of their
// own, so the search walks down to the first text, image or control. Day's tree is Canvases
// most of the way, so the walk is short and runs only when the app announces something.
static WUXAP::AutomationPeer peer_under(UIElement const& e, int depth) {
    if (!e) return nullptr;
    if (auto p = WUXAP::FrameworkElementAutomationPeer::CreatePeerForElement(e)) return p;
    if (depth <= 0) return nullptr;
    if (auto panel = e.try_as<WUXC::Panel>()) {
        auto kids = panel.Children();
        for (uint32_t i = 0; i < kids.Size(); ++i) {
            if (auto p = peer_under(kids.GetAt(i), depth - 1)) return p;
        }
    } else if (auto host = e.try_as<WUXC::ContentControl>()) {
        if (auto content = host.Content()) {
            if (auto inner = content.try_as<UIElement>()) return peer_under(inner, depth - 1);
        }
    }
    return nullptr;
}

// Speak `text` through the running screen reader without moving its focus (docs/accessibility.md):
// a UIA notification raised from the primary window's tree. `urgent` interrupts what is being
// read (ImportantMostRecent); otherwise the sentence queues behind it (All). With no screen
// reader listening the notification goes nowhere, and a stack too old to raise one throws into
// `guard`. Before the window exists there is nothing to raise it from, so the text is dropped.
void day_xaml_announce(const char* text, int urgent) {
    guard([&] {
        if (!text || !*text || !g_app || !g_app->root) return;
        auto peer = peer_under(g_app->root, 16);
        if (!peer) return;
        peer.RaiseNotificationEvent(
            WUXAP::AutomationNotificationKind::ActionCompleted,
            urgent ? WUXAP::AutomationNotificationProcessing::ImportantMostRecent
                   : WUXAP::AutomationNotificationProcessing::All,
            hs(text), winrt::hstring{ L"day-announce" });
    });
}

