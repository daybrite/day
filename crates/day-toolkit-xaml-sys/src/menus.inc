// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Included by shim.cpp at file scope. Shared declarations and include order live there.

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
            // A 9th field, when present, is the item's checked state: -1 a plain command, 0/1 a
            // checkable one. A checkable item is a ToggleMenuFlyoutItem (which derives from
            // MenuFlyoutItem, so everything below still applies to it) — WinUI's own checked row,
            // with the mark's column reserved when it is off.
            int checked = f.size() > 8 && !f[8].empty() ? std::atoi(f[8].c_str()) : -1;
            WUXC::ToggleMenuFlyoutItem toggle{nullptr};
            WUXC::MenuFlyoutItem item{nullptr};
            if (checked >= 0) {
                toggle = WUXC::ToggleMenuFlyoutItem{};
                toggle.IsChecked(checked != 0);
                item = toggle;
            } else {
                item = WUXC::MenuFlyoutItem{};
            }
            item.Text(hs(label.c_str()));
            // An 8th field, when present, is the item's Segoe Fluent code point in hex
            // (docs/menus.md) — the same table the toolbar's icons come from.
            if (f.size() > 7 && !f[7].empty()) {
                wchar_t code = static_cast<wchar_t>(std::wcstol(
                    std::wstring(f[7].begin(), f[7].end()).c_str(), nullptr, 16));
                if (code) {
                    WUXC::FontIcon fi;
                    fi.Glyph(winrt::hstring(std::wstring(1, code)));
                    item.Icon(fi);
                }
            }
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
                // Does end the whole app — right where someone reaches to close one window.
                closes_window = role == 11;
            }
            if (fire || closes_window) {
                item.Click([fire, closes_window, toggle, checked](WF::IInspectable const& s,
                                                                  WUX::RoutedEventArgs const&) {
                    // ToggleMenuFlyoutItem flips itself on click. Put it back: whether the mark
                    // moves is the app's decision, and it arrives as the next menu install.
                    if (toggle) toggle.IsChecked(checked != 0);
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
/// matters more than the choice — two menus sharing a key leaves both unreachable, and localized
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

