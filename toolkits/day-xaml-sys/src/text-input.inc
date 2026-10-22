// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Included by shim.cpp inside the C ABI block. Shared declarations and include order live there.

// Every text field this shim built (day_xaml_textbox_new, day_xaml_field_set_secure), by handle,
// with the flag its event handlers read before they report. A field that turns secure is rebuilt
// as a PasswordBox (and back as a TextBox), and the control it replaces stays alive for as long
// as XAML is still unwinding through it: clearing the flag is what keeps that control's late
// TextChanged and LostFocus from reaching the node its replacement now reports for. The flag is
// shared with the handlers at construction, as the text area's `g_selectable` is, so they never
// touch this map. Entries drop in day_xaml_delete.
static std::map<void*, std::shared_ptr<bool>> g_field_live;
// XAML may not expose Parent until layout loads the control. Remember the collection
// it was inserted into so an initial secure replacement cannot leave a plain field behind.
static std::map<void*, winrt::weak_ref<WUXC::Panel>> g_field_parents;

// ---- focus (docs/focus.md) ----
// Observe: kind 1 = gained, 0 = lost, 2 = submitted (Enter in a TextBox or a PasswordBox). System
// XAML has no global focus event, so each control reports its own GotFocus/LostFocus.
void day_xaml_enable_focus(void* h, unsigned long long id,
                            void (*cb)(unsigned long long, int)) try {
    // GotFocus/LostFocus are UIElement events, so this reads focus off whatever day hands it —
    // a plain control, or the canvas's ContentControl host (day_xaml_canvas_new) — through the
    // same pair, with no Control cast to fail on.
    auto e = elem(h);
    if (!e) return;
    // A text field's flag (see g_field_live); null for every other control, which always reports.
    std::shared_ptr<bool> live;
    auto field = g_field_live.find(h);
    if (field != g_field_live.end()) live = field->second;
    e.GotFocus([id, cb, live](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
        if (live && !*live) return;
        cb(id, 1);
    });
    e.LostFocus([id, cb, live](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
        if (live && !*live) return;
        cb(id, 0);
    });
    // KeyDown is UIElement's, so the one subscription serves both field classes.
    if (e.try_as<WUXC::TextBox>() || e.try_as<WUXC::PasswordBox>()) {
        e.KeyDown([id, cb, live](WF::IInspectable const&, WUXIn::KeyRoutedEventArgs const& a) {
            if (live && !*live) return;
            if (a.Key() == winrt::Windows::System::VirtualKey::Enter) cb(id, 2);
        });
    }
} catch (...) {}

// Drive: request programmatic focus, or resign it to the window's focus sink — only while
// this control still owns focus, so a stale release can't blur a sibling. (Programmatic
// focus draws no focus visual; that is system-XAML behavior, not a bug.)
void day_xaml_control_focus(void* h, int focused) try {
    auto e = elem(h);
    if (!e) return;
    if (focused) {
        // `Control::Focus` — the only Focus system XAML has (`UIElement` grew one in WinUI 3,
        // not here). The canvas is not the exception it looks like: it is handed out as a
        // focusable ContentControl host (day_xaml_canvas_new), so this one call serves both.
        if (auto c = e.try_as<WUXC::Control>()) c.Focus(WUX::FocusState::Programmatic);
        return;
    }
    if (!g_app || !g_app->focus_sink) return;
    // Resign only while this element still owns focus, so a stale release can't blur a
    // sibling. A Control answers directly; anything else has to ask the focus manager, which
    // is the only reader of focus a plain UIElement has.
    bool owns = false;
    if (auto c = e.try_as<WUXC::Control>()) {
        owns = c.FocusState() != WUX::FocusState::Unfocused;
    } else if (auto focusedNow = WUXIn::FocusManager::GetFocusedElement()) {
        auto other = focusedNow.try_as<UIElement>();
        owns = other && winrt::get_abi(other) == winrt::get_abi(e);
    }
    if (!owns) return;
    auto sink = g_app->focus_sink;
    sink.IsTabStop(true);
    sink.Focus(WUX::FocusState::Programmatic);
    sink.IsTabStop(false);
} catch (...) {}

// ---- textbox ----
// A text field is a TextBox, or a PasswordBox while it is secure (docs/textfield.md): XAML hides
// characters with a class of its own, not a property. Both are built here so the two report text
// through the same callback and the same node id, and every entry point below accepts either.

static void* field_new(bool secure, winrt::hstring const& text, winrt::hstring const& placeholder,
                       unsigned long long id, void (*cb)(unsigned long long, const char*)) {
    auto live = std::make_shared<bool>(true);
    UIElement e{ nullptr };
    if (secure) {
        WUXC::PasswordBox pb;
        pb.Password(text);
        pb.PlaceholderText(placeholder);
        pb.PasswordChanged([id, cb, live](WF::IInspectable const& s, WUX::RoutedEventArgs const&) {
            if (!*live) return;
            std::string str = u8(s.as<WUXC::PasswordBox>().Password());
            cb(id, str.c_str());
        });
        e = pb;
    } else {
        WUXC::TextBox tb;
        tb.Text(text);
        tb.PlaceholderText(placeholder);
        tb.TextChanged([id, cb, live](WF::IInspectable const& s, WUXC::TextChangedEventArgs const&) {
            if (!*live) return;
            std::string str = u8(s.as<WUXC::TextBox>().Text());
            cb(id, str.c_str());
        });
        e = tb;
    }
    void* h = boxh(e);
    g_field_live[h] = live;
    return h;
}

void* day_xaml_textbox_new(const char* text, const char* placeholder, unsigned long long id,
                            void (*cb)(unsigned long long, const char*)) {
    return field_new(false, hs(text), hs(placeholder), id, cb);
}
void day_xaml_textbox_set_text(void* h, const char* t) {
    // Write only a real change, so a value day already holds raises no change event for it.
    if (auto tb = elem(h).try_as<WUXC::TextBox>()) {
        auto nt = hs(t);
        if (tb.Text() != nt) tb.Text(nt);
    } else if (auto pb = elem(h).try_as<WUXC::PasswordBox>()) {
        auto nt = hs(t);
        if (pb.Password() != nt) pb.Password(nt);
    }
}
void day_xaml_textbox_set_placeholder(void* h, const char* t) {
    if (auto tb = elem(h).try_as<WUXC::TextBox>()) tb.PlaceholderText(hs(t));
    else if (auto pb = elem(h).try_as<WUXC::PasswordBox>()) pb.PlaceholderText(hs(t));
}

// 1 while the field is the secure class. Asked by the tweak seam, which names the class it hands
// out.
int day_xaml_field_is_secure(void* h) {
    int out = 0;
    guard([&] { out = elem(h).try_as<WUXC::PasswordBox>() ? 1 : 0; });
    return out;
}

// The entry traits that are plain properties (docs/textfield.md); the secure one is a class and
// is day_xaml_field_set_secure's. Every member is written on every call, so a member going back to
// its default is restored, and each is written only when it differs so a focused field is not
// disturbed by a call that changes nothing.
//
// `scope` picks the InputScope, which is what the touch keyboard lays itself out from: 0 Default,
// 1 a person's name, 2 an email address, 3 a URL, 4 a telephone number, 5 a number. `plain` turns
// spell checking and text prediction off, for text that is an identifier and not prose.
//
// A PasswordBox has only the first of those, in two values: its own Password scope and the
// NumericPin one, which a numeric `scope` asks for. It has no IsReadOnly either. A read-only
// secure field therefore leaves hit testing and the tab order instead: its characters are hidden
// and a PasswordBox never copies them out, so what read-only still has to promise is that the
// field keeps its ordinary look and takes no edits, and an unreachable field promises both.
void day_xaml_field_set_traits(void* h, int read_only, int scope, int plain) {
    guard([&] {
        auto e = elem(h);
        if (!e) return;
        if (auto tb = e.try_as<WUXC::TextBox>()) {
            if (tb.IsReadOnly() != (read_only != 0)) tb.IsReadOnly(read_only != 0);
            if (tb.IsSpellCheckEnabled() != (plain == 0)) tb.IsSpellCheckEnabled(plain == 0);
            if (tb.IsTextPredictionEnabled() != (plain == 0)) tb.IsTextPredictionEnabled(plain == 0);
            auto want = WUXIn::InputScopeNameValue::Default;
            switch (scope) {
            case 1: want = WUXIn::InputScopeNameValue::PersonalFullName; break;
            case 2: want = WUXIn::InputScopeNameValue::EmailSmtpAddress; break;
            case 3: want = WUXIn::InputScopeNameValue::Url; break;
            case 4: want = WUXIn::InputScopeNameValue::TelephoneNumber; break;
            case 5: want = WUXIn::InputScopeNameValue::Number; break;
            default: break;
            }
            // A field that never had a scope reads back null, which is the Default one.
            auto have = WUXIn::InputScopeNameValue::Default;
            if (auto current = tb.InputScope()) {
                auto names = current.Names();
                if (names.Size() > 0) have = names.GetAt(0).NameValue();
            }
            if (have != want) {
                WUXIn::InputScopeName name;
                name.NameValue(want);
                WUXIn::InputScope next;
                next.Names().Append(name);
                tb.InputScope(next);
            }
        } else if (auto pb = e.try_as<WUXC::PasswordBox>()) {
            bool reachable = read_only == 0;
            if (!reachable && pb.FocusState() != WUX::FocusState::Unfocused) {
                // Typing reaches a focused field whatever its hit testing says.
                day_xaml_control_focus(h, 0);
            }
            if (pb.IsHitTestVisible() != reachable) pb.IsHitTestVisible(reachable);
            if (pb.IsTabStop() != reachable) pb.IsTabStop(reachable);
            auto want = scope == 5 ? WUXIn::InputScopeNameValue::NumericPin
                                   : WUXIn::InputScopeNameValue::Password;
            auto have = WUXIn::InputScopeNameValue::Password;
            if (auto current = pb.InputScope()) {
                auto names = current.Names();
                if (names.Size() > 0) have = names.GetAt(0).NameValue();
            }
            if (have != want) {
                WUXIn::InputScopeName name;
                name.NameValue(want);
                WUXIn::InputScope next;
                next.Names().Append(name);
                pb.InputScope(next);
            }
        }
    });
}

void day_xaml_delete(void* h); // defined with the tree functions below

// Make the field the secure class (a PasswordBox) or the plain one (a TextBox). Returns the
// replacement's handle when the class changed, and null when the field already is the class asked
// for, is not a text field, or could not be replaced; the caller then owns the replacement and
// releases `h`, whose control has already left the tree.
//
// The replacement is built by `field_new` and wired by `day_xaml_enable_focus`, the same two calls
// that make a field in the first place, so it reports text, Enter and focus for the same node id.
// It takes over the text, the placeholder, the enabled state, the automation id and name, the
// frame day gave the old control (Canvas.Left/Top and Width/Height), visibility, opacity, any
// transform, context menu and pointer shape, the old control's position among its siblings, and
// keyboard focus if the old control had it.
void* day_xaml_field_set_secure(void* h, int secure, unsigned long long id,
                                void (*text_cb)(unsigned long long, const char*),
                                void (*focus_cb)(unsigned long long, int)) {
    void* nh = nullptr;
    try {
        auto old_live = g_field_live.find(h);
        if (old_live == g_field_live.end()) return nullptr;
        UIElement old = elem(h);
        if (!old) return nullptr;
        auto otb = old.try_as<WUXC::TextBox>();
        auto opb = old.try_as<WUXC::PasswordBox>();
        if (!otb && !opb) return nullptr;
        bool was_secure = opb ? true : false;
        if (was_secure == (secure != 0)) return nullptr;
        auto oc = old.as<WUXC::Control>();

        // Where the old control hangs. Day parents every field on a Canvas; anything else has no
        // child list to take the replacement's place in, so the field stays the class it is.
        WUXC::Panel parent{ nullptr };
        if (auto found = g_field_parents.find(h); found != g_field_parents.end()) {
            parent = found->second.get();
        }
        auto above = oc.Parent();
        if (!above) above = WUXM::VisualTreeHelper::GetParent(oc).try_as<FrameworkElement>();
        if (!parent && above) {
            parent = above.try_as<WUXC::Panel>();
            if (!parent) return nullptr;
        }

        winrt::hstring text = opb ? opb.Password() : otb.Text();
        winrt::hstring placeholder = opb ? opb.PlaceholderText() : otb.PlaceholderText();
        nh = field_new(secure != 0, text, placeholder, id, text_cb);
        day_xaml_enable_focus(nh, id, focus_cb);
        UIElement fresh = elem(nh);
        auto nc = fresh.as<WUXC::Control>();

        // Each group on its own, so one property a degraded control refuses costs only itself.
        guard([&] { nc.IsEnabled(oc.IsEnabled()); });
        guard([&] {
            // NaN is Auto and carries over as itself: a field day has not framed yet stays unframed.
            nc.Width(oc.Width());
            nc.Height(oc.Height());
            WUXC::Canvas::SetLeft(nc, WUXC::Canvas::GetLeft(oc));
            WUXC::Canvas::SetTop(nc, WUXC::Canvas::GetTop(oc));
        });
        guard([&] {
            nc.Visibility(oc.Visibility());
            nc.Opacity(oc.Opacity());
        });
        guard([&] {
            if (auto t = oc.RenderTransform()) {
                nc.RenderTransform(t);
                nc.RenderTransformOrigin(oc.RenderTransformOrigin());
            }
        });
        guard([&] {
            WUX::Automation::AutomationProperties::SetAutomationId(
                nc, WUX::Automation::AutomationProperties::GetAutomationId(oc));
            WUX::Automation::AutomationProperties::SetName(
                nc, WUX::Automation::AutomationProperties::GetName(oc));
        });
        bool focused = oc.FocusState() != WUX::FocusState::Unfocused;
        if (parent) {
            auto kids = parent.Children();
            uint32_t idx = 0;
            if (kids.IndexOf(old, idx)) {
                // In beside the old control first, which keeps focus where it is until the
                // replacement can take it. A throw here leaves the old field as it was.
                kids.InsertAt(idx, fresh);
                g_field_parents[nh] = winrt::make_weak(parent);
            } else {
                day_xaml_delete(nh);
                return nullptr;
            }
        }
        // From here the replacement is the field. The old control goes quiet before focus moves
        // off it, so day is not told that the node lost focus on its way to the same node.
        *old_live->second = false;
        // What only one control can hold moves now that nothing above can still fail.
        guard([&] {
            // A flyout has one owner at a time, so it leaves the old control first.
            if (auto fly = oc.ContextFlyout()) {
                oc.ContextFlyout(nullptr);
                nc.ContextFlyout(fly);
            }
        });
        guard([&] {
#ifdef DAY_WINUI
            auto from = old.try_as<WUX::IUIElementProtected>();
            auto to = fresh.try_as<WUX::IUIElementProtected>();
            if (from && to) {
                if (auto cursor = from.ProtectedCursor()) to.ProtectedCursor(cursor);
            }
#else
            // The shape is keyed by the element, and the old element's entries go with it.
            void* key = winrt::get_abi(old);
            int code = 0;
            auto it = g_cursor_codes.find(key);
            if (it != g_cursor_codes.end()) code = it->second;
            g_cursor_codes.erase(key);
            g_cursor_hooked.erase(key);
            if (code != 0) day_xaml_set_cursor(nh, code);
#endif
        });
        if (parent && focused) {
            guard([&] {
                // A control takes focus only once its template is applied, which the forced
                // layout does. One that still refuses takes it when it loads: once, because
                // Loaded is raised again each time the page it sits on comes back.
                parent.UpdateLayout();
                if (!nc.Focus(WUX::FocusState::Programmatic)) {
                    auto pending = std::make_shared<bool>(true);
                    nc.Loaded([pending](WF::IInspectable const& s, WUX::RoutedEventArgs const&) {
                        if (!*pending) return;
                        *pending = false;
                        try {
                            if (auto c = s.try_as<WUXC::Control>()) {
                                c.Focus(WUX::FocusState::Programmatic);
                            }
                        } catch (...) {
                        }
                    });
                }
            });
            guard([&] {
                // The caret goes where typing continues: after the text. A PasswordBox has no
                // caret or selection-range API, only SelectAll, which at least shows the user
                // what the next key replaces.
                if (auto ntb = fresh.try_as<WUXC::TextBox>()) {
                    ntb.Select(static_cast<int32_t>(ntb.Text().size()), 0);
                } else if (auto npb = fresh.try_as<WUXC::PasswordBox>()) {
                    npb.SelectAll();
                }
            });
        }
        if (parent) {
            bool gone = false;
            guard([&] {
                auto kids = parent.Children();
                uint32_t idx = 0;
                if (kids.IndexOf(old, idx)) kids.RemoveAt(idx);
                gone = true;
            });
            // A control that would not leave must at least not show: it may be the plain-text
            // one a secure field has just replaced.
            if (!gone) guard([&] { old.Visibility(WUX::Visibility::Collapsed); });
        }
        return nh;
    } catch (...) {
        if (nh) day_xaml_delete(nh);
        return nullptr;
    }
}

