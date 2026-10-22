// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Included by shim.cpp at file scope. Shared declarations and include order live there.

// ---- window toolbar (docs/toolbars.md) ------------------------------------
// A Fluent CommandBar docked under the menu bar. `PrimaryCommands` — which the CommandBar
// template right-aligns — carries the AppBarButton / AppBarToggleButton / AppBarSeparator
// commands, and `Content` — which it left-aligns — carries the leading items in a horizontal
// StackPanel. Each item's placement picks its region (see insert_toolbar_item). The search field
// rides an AppBarElementContainer (Windows.UI.Xaml, 1903+) at the right end of PrimaryCommands;
// a label lands in Content whichever side it asked for.
//
// Each item arrives as its own spec (`day_xaml_toolbar_insert`), in the format of the menu spec
// above, one line per item:
//   kind \t id \t action \t enabled \t on \t glyph \t image \t label \t tooltip \t text
//        \t placeholder \t suggestions \t geom \t place \t style
// kinds: B button, T toggle, G segmented, M menu, F search field, L label, `-` separator. `on`
// seeds a toggle (or a segmented item's index) and `text` a search field; `glyph` is a Segoe
// Fluent Icons code point in hex, `image` a bundled image FILE NAME. `place` is the region and
// folding rank, `style` the label style (docs/toolbars.md).
// An `M` line is followed by that item's MENU spec — the same lines build_menu_items already
// parses — closed by an `X` line, so the sub-spec is sliced out here and handed straight to it.
// Buttons ride the same g_menu_cb rail as menu items; a toggle's state and a search field's text
// go through g_toolbar_cb.

// kind 0 = toggle (`on`), kind 1 = search text (`text`), kind 2 = segment index (`on`),
// kind 3 = search submitted (`text`).
static void (*g_toolbar_cb)(unsigned long long, int, int, const char*) = nullptr;

/// One window's live toolbar item, or null. `win` is the window token the install used — the
/// primary AppWindow* or a SecWindow* — so a patch reaches the item in the window that owns it.
static FrameworkElement find_toolbar_elem(void* win, const char* id) {
    auto w = g_toolbar_elems.find(win);
    if (w == g_toolbar_elems.end()) return nullptr;
    auto it = w->second.find(std::string(id ? id : ""));
    return it == w->second.end() ? nullptr : it->second;
}

// Completions for a search field (docs/search.md): AutoSuggestBox's own ItemsSource, so the popup,
// the keyboard handling and the Fluent styling are the platform's. The list is unit-separated
// (\x1f) because tabs and newlines are the spec's record separators. Defined above the toolbar
// install that seeds it — C++ resolves plain calls by declaration order, so a definition parked
// down with the patch entry point below would not be visible there.
static void day_xaml_fill_suggestions(WUXC::AutoSuggestBox const& box, std::string const& joined) {
    auto items = winrt::single_threaded_observable_vector<WF::IInspectable>();
    size_t start = 0;
    while (start <= joined.size() && !joined.empty()) {
        size_t sep = joined.find('\x1f', start);
        std::string one = joined.substr(start, sep == std::string::npos ? std::string::npos
                                                                        : sep - start);
        if (!one.empty()) items.Append(winrt::box_value(hs(one.c_str())));
        if (sep == std::string::npos) break;
        start = sep + 1;
    }
    box.ItemsSource(items);
}

// A programmatic IsChecked write is in flight. ToggleButton raises Checked/Unchecked
// synchronously from the setter, so a flag around the write keeps day's own value from echoing
// back through g_toolbar_cb. (The search field can't use one: AutoSuggestBox raises TextChanged
// asynchronously, which is why that handler filters on the change REASON instead.)
static bool g_toolbar_setting_checked = false;

extern "C" void day_xaml_set_toolbar_cb(void (*cb)(unsigned long long, int, int, const char*)) {
    g_toolbar_cb = cb;
}

// Segoe Fluent Icons is the Windows 11 icon font; Windows 10 ships its predecessor, Segoe MDL2
// Assets, which carries the same code points for every glyph day maps. A family the system does
// not have draws notdef boxes rather than falling back to a real glyph, so pick by the file the
// Win11 font installs. Resolved once — it cannot change while the process runs.
static winrt::hstring const& toolbar_icon_font() {
    static winrt::hstring family = [] {
        wchar_t dir[MAX_PATH]{};
        UINT n = GetWindowsDirectoryW(dir, MAX_PATH);
        std::wstring path = (n > 0 && n < MAX_PATH) ? std::wstring(dir) : std::wstring(L"C:\\Windows");
        path += L"\\Fonts\\SegoeIcons.ttf";
        return GetFileAttributesW(path.c_str()) != INVALID_FILE_ATTRIBUTES
                   ? winrt::hstring{ L"Segoe Fluent Icons" }
                   : winrt::hstring{ L"Segoe MDL2 Assets" };
    }();
    return family;
}

// An item's icon: the standard symbol's glyph, else a bundled image staged next to the exe (an
// ms-appx BitmapIcon, as the nav icons load), else none — an AppBarButton without an Icon shows
// its label alone, which is exactly what an unmapped symbol should look like.
static WUXC::IconElement toolbar_icon(const std::string& glyph, const std::string& image,
                                      const std::string& geom = std::string()) {
    if (!glyph.empty()) {
        // Every code point day maps is in the BMP private-use area: one UTF-16 unit.
        wchar_t ch = static_cast<wchar_t>(std::strtoul(glyph.c_str(), nullptr, 16));
        if (ch) {
            WUXC::FontIcon fi;
            fi.FontFamily(WUXM::FontFamily(toolbar_icon_font()));
            fi.Glyph(winrt::hstring{ std::wstring(1, ch) });
            return fi;
        }
    }
    // Vector before raster, as the nav pane does: a PathIcon is resolution-independent and takes
    // the bar's Foreground, so it themes itself. No tint channel here — a toolbar command is
    // monochrome chrome by definition, and the CommandBar's own foreground is the right color in
    // both schemes and in the disabled/pressed visuals.
    if (!geom.empty()) {
        std::string spec = geom;
        // Undo both escapes day-xaml applied: this line format owns \t and \n, and a geometry
        // spec uses both (\n between shapes, \t before a shape's path data).
        for (auto& c : spec) {
            if (c == '\x1f') c = '\n';
            else if (c == '\x1e') c = '\t';
        }
        // 16 px is the AppBarButton icon box the default template lays out.
        if (void* h = day_xaml_vector_icon_new(spec.c_str(), 0u, 0, 16.0)) {
            auto ie = elem(h).try_as<WUXC::IconElement>();
            day_xaml_delete(h); // the command owns it now
            if (ie) return ie;
        }
    }
    if (!image.empty()) {
        WUXC::BitmapIcon bicon;
        bicon.UriSource(WF::Uri{ hs(("ms-appx:///images/" + image).c_str()) });
        bicon.ShowAsMonochrome(true); // tint to the bar foreground (theme-adaptive)
        return bicon;
    }
    return nullptr;
}

extern "C" void day_xaml_button_set_content(void* h, const char* title, const char* glyph, const char* image, const char* geometry, int icon_only) try {
    auto button = elem(h).try_as<WUXC::Button>();
    if (!button) return;
    auto icon = toolbar_icon(glyph, image, geometry);
    WUXC::StackPanel content;
    content.Orientation(WUXC::Orientation::Horizontal);
    content.Spacing(6);
    if (icon) content.Children().Append(icon);
    if (!icon_only || !icon) {
        WUXC::TextBlock label;
        label.Text(hs(title));
        content.Children().Append(label);
    }
    button.Content(content);
    // The title names the button for UI Automation (an icon-only one shows none), unless the app
    // named it itself; overwriting that here is what replaced app labels on a title change.
    if (!reinterpret_cast<Node*>(h)->app_named)
        WUX::Automation::AutomationProperties::SetName(button, hs(title));
    WUXC::ToolTipService::SetToolTip(button, icon_only && icon ? winrt::box_value(hs(title)) : nullptr);
} catch (...) {
}


// --- the window toolbar (docs/toolbars.md), edited one item at a time ---------------------
// A docked CommandBar with three regions: a leading StackPanel as its `Content`, the
// `PrimaryCommands` on the right, and the `SecondaryCommands` in the overflow. Each item goes
// into its region right after the item before it there, so an edit never touches an element it
// does not name: an AutoSuggestBox being typed into keeps its focus while the page commands
// around it change.


static bool region_index(ToolbarState& st, FrameworkElement const& e, int region, uint32_t& at) {
    if (region == 0) return st.lead.Children().IndexOf(e.as<WUX::UIElement>(), at);
    auto cmd = e.as<WUXC::ICommandBarElement>();
    if (region == 1) return st.bar.PrimaryCommands().IndexOf(cmd, at);
    return st.bar.SecondaryCommands().IndexOf(cmd, at);
}

static void region_insert(ToolbarState& st, FrameworkElement const& e, int region, uint32_t at) {
    if (region == 0) st.lead.Children().InsertAt(at, e);
    else if (region == 1) st.bar.PrimaryCommands().InsertAt(at, e.as<WUXC::ICommandBarElement>());
    else st.bar.SecondaryCommands().InsertAt(at, e.as<WUXC::ICommandBarElement>());
}

static void region_remove(ToolbarState& st, FrameworkElement const& e, int region) {
    uint32_t at = 0;
    if (!region_index(st, e, region, at)) return;
    if (region == 0) st.lead.Children().RemoveAt(at);
    else if (region == 1) st.bar.PrimaryCommands().RemoveAt(at);
    else st.bar.SecondaryCommands().RemoveAt(at);
}

// Put the bar on the window. Appended, so it paints over nothing it could hide, but FIRST in tab
// order: a Windows command bar comes before the content it acts on (File Explorer, Mail), and
// appended last it was the end of the Tab cycle, after every control in the page.
static void dock_toolbar(WUXC::Canvas const& root, WUXC::CommandBar const& bar) {
    bar.TabIndex(0);
    root.Children().Append(bar);
}

// The window's bar, docked into `root` on first use.
static ToolbarState& toolbar_state(void* win, WUXC::Canvas const& root) {
    ToolbarState& st = g_toolbar_state[win];
    if (!st.bar) {
        WUXC::CommandBar bar;
        bar.Name(L"day_toolbar");
        // Labels beside the icons: the desktop CommandBar look, and it keeps the strip one row
        // tall.
        bar.DefaultLabelPosition(WUXC::CommandBarDefaultLabelPosition::Right);
        bar.HorizontalContentAlignment(WUX::HorizontalAlignment::Left);
        bar.VerticalContentAlignment(WUX::VerticalAlignment::Center);
        WUXC::StackPanel lead;
        lead.Orientation(WUXC::Orientation::Horizontal);
        lead.VerticalAlignment(WUX::VerticalAlignment::Center);
        bar.Content(lead);
        WUXC::Canvas::SetLeft(bar, 0);
        // The top offset (below the menu bar, if there is one) and the width are relayout's
        // job — the two bars are installed in either order.
        dock_toolbar(root, bar);
        st.bar = bar;
        st.lead = lead;
    }
    return st;
}

void SegmentMenu::show(size_t n) {
    if (n >= rows.size()) return;
    for (size_t k = 0; k < rows.size(); ++k) rows[k].IsChecked(k == n);
    // A segmented control has no label of its own, so the command is titled by the choice in
    // force, the way the phones title the same fold (docs/toolbars.md).
    button.Label(hs(titles[n].c_str()));
    button.Icon(toolbar_icon(icons[n].first, icons[n].second));
    WUX::Automation::AutomationProperties::SetName(button, hs(titles[n].c_str()));
}

// Build the item `spec` describes (one record, and its menu or segment lines) and put it at
// position `index` of Day's order.
static void insert_toolbar_item(void* win, ToolbarState& st, size_t index, const char* spec) {
    ToolbarElems& elems = g_toolbar_elems[win];
    bool trailing = false;  // this item's placement says the right-hand group
    bool secondary = false; // …and the overflow within it
    // Into `region`, right after the nearest earlier item drawn in the same region.
    auto put = [&](FrameworkElement const& e, int region, const std::string& id, int rank = 0) {
        const size_t at = (std::min)(index, st.order.size());
        uint32_t pos = 0;
        for (size_t k = at; k-- > 0;) {
            auto& o = st.order[k];
            uint32_t found = 0;
            if (o.elem && o.region == region && region_index(st, o.elem, region, found)) {
                pos = found + 1;
                break;
            }
        }
        // The search field keeps the right end: it goes there itself, and a command inserted
        // after it in Day's order still lands to its left.
        if (region == 1 && st.search_end) {
            uint32_t end = 0;
            if (e == st.search_end) pos = st.bar.PrimaryCommands().Size();
            else if (region_index(st, st.search_end, 1, end)) pos = (std::min)(pos, end);
        }
        if (e) region_insert(st, e, region, pos);
        st.order.insert(st.order.begin() + at, ToolbarEntry{ id, e, e ? region : -1, rank });
        if (e && !id.empty()) elems.insert_or_assign(id, e);
    };
    // The overflow order of the item being built (see put_command).
    int fold_order = 0;
    // A command: `SecondaryCommands` for one the app marked foldable, `PrimaryCommands` for the
    // right-hand group, the leading panel otherwise.
    //
    // Each right-hand command records its placement's folding rank; `fold_by_role` turns the
    // ranks into DynamicOverflowOrder once the edit settles.
    auto put_command = [&](FrameworkElement const& e, const std::string& id) {
        put(e, secondary ? 2 : (trailing ? 1 : 0), id, fold_order);
    };
    // Whether the item asked to show its icon alone (LabelStyle::IconOnly, docs/toolbars.md).
    bool icon_only = false;
    // A leading AppBarButton is outside the bar's own collections, so DefaultLabelPosition does
    // not reach it and it would draw its label under the icon — two rows tall. Collapse the label
    // where there is an icon to show instead; without one the label is the only click target. A
    // trailing one collapses it only when the app asked for the icon alone: the label stays the
    // accessible name and the tooltip, and the overflow still shows it.
    auto compact = [&](WUXC::AppBarButton const& b, bool has_icon) {
        if (has_icon && (!trailing || icon_only))
            b.LabelPosition(WUXC::CommandBarLabelPosition::Collapsed);
    };

    auto lines = split_lines(spec);
    for (size_t i = 0; i < lines.size(); ++i) {
        if (lines[i].empty()) continue;
        auto f = split_tabs(lines[i]);
        auto fld = [&f](size_t n) { return f.size() > n ? f[n] : std::string(); };
        std::string kind = fld(0), id = fld(1);
        unsigned long long action = std::strtoull(fld(2).c_str(), nullptr, 10);
        bool enabled = fld(3) != "0";
        // A toggle's state, or a segmented item's selected index.
        const int value = std::atoi(fld(4).c_str());
        bool on = value == 1;
        std::string glyph = fld(5), image = fld(6), label = fld(7), tip = fld(8), text = fld(9),
                    placeholder = fld(10), suggestions = fld(11), geom = fld(12),
                    place = fld(13), style = fld(14);
        // LabelStyle (docs/toolbars.md): "title" draws the words alone, so the icon is dropped
        // here; "icon" collapses the label where the item is drawn (see `compact`).
        if (style == "title") glyph.clear(), image.clear(), geom.clear();
        icon_only = style == "icon";
        bool has_icon = !glyph.empty() || !image.empty() || !geom.empty();
        // WHERE this item sits (docs/toolbars.md). A CommandBar's three slots are exactly the
        // three roles Day's placements reduce to: `Content` on the left, `PrimaryCommands` on the
        // right, `SecondaryCommands` in the overflow. Set per item rather than by a running
        // flag, which is what the model's flexible space used to mean before placement existed.
        trailing = place != "content";
        secondary = place == "secondary";
        // Folding order among the right-hand commands: an `Automatic` one before a `Primary` one.
        fold_order = place == "automatic" ? 1 : place == "primary" ? 2 : 0;

        if (kind == "-") {
            if (trailing) {
                put_command(WUXC::AppBarSeparator{}, id);
            } else {
                // AppBarSeparator sizes itself against the bar's own row, not against a
                // StackPanel, so a leading divider is a hairline of our own — the same
                // translucent neutral the divider piece falls back to, which reads in both
                // schemes.
                WUXC::Border rule;
                rule.Width(1);
                rule.Height(20);
                rule.Margin(WUX::Thickness{ 6, 0, 6, 0 });
                rule.Background(WUXM::SolidColorBrush(color_argb(0x33'808080u)));
                put(rule, 0, id);
            }
        } else if (kind == "F") {
            WUXC::AutoSuggestBox box;
            day_xaml_fill_suggestions(box, suggestions);
            box.Text(hs(text.c_str()));
            box.PlaceholderText(hs(placeholder.c_str()));
            box.QueryIcon(toolbar_icon("E721", "")); // the search glyph
            box.IsEnabled(enabled);
            // Its accessible name: the item's label, else its prompt. Without one a screen reader
            // announced a bare "edit" in the middle of the bar.
            WUX::Automation::AutomationProperties::SetName(
                box, hs((!label.empty() ? label : !placeholder.empty() ? placeholder
                                                                         : std::string("Search"))
                            .c_str()));
            box.Width(240); // a bar search field is sized, not stretched
            box.Margin(WUX::Thickness{ 4, 0, 4, 0 });
            box.VerticalAlignment(WUX::VerticalAlignment::Center);
            if (action) {
                // The text last reported, so a submission reports `Text` first only when the
                // field holds something day has not been told.
                auto sent = std::make_shared<std::string>(text);
                box.TextChanged([action, sent](WUXC::AutoSuggestBox const& s,
                                               WUXC::AutoSuggestBoxTextChangedEventArgs const& a) {
                    // day writing the bound signal back raises this too; reporting that would
                    // echo the app's own value straight back at it.
                    if (a.Reason() != WUXC::AutoSuggestionBoxTextChangeReason::UserInput) return;
                    *sent = u8(s.Text());
                    if (g_toolbar_cb) g_toolbar_cb(action, 1, 0, sent->c_str());
                });
                // Return, the query glyph, or a suggestion picked from the list
                // (docs/search.md). A picked suggestion fills the box with reason
                // SuggestionChosen, which the handler above does not report, and TextChanged
                // is raised asynchronously, so the last keystroke may not have been reported
                // yet either: report the query's text first whenever it is new.
                box.QuerySubmitted([action, sent](WUXC::AutoSuggestBox const&,
                                                  WUXC::AutoSuggestBoxQuerySubmittedEventArgs const& a) {
                    if (!g_toolbar_cb) return;
                    std::string str = u8(a.QueryText());
                    if (str != *sent) {
                        *sent = str;
                        g_toolbar_cb(action, 1, 0, str.c_str());
                    }
                    g_toolbar_cb(action, 3, 0, str.c_str());
                });
            }
            if (!trailing) {
                put(box, 0, id); // asked for the leading side (Navigation / Principal)
                continue;
            }
            // Trailing, which is where Day places search: the right end of the bar, just before
            // the overflow button, where Windows apps keep their search box. An
            // AppBarElementContainer carries it into PrimaryCommands. It is the LAST thing the bar
            // folds (rank 3, after every command), and never goes to SecondaryCommands whatever
            // its placement says: a search field is not a menu row.
            WUXC::AppBarElementContainer host;
            host.Content(box);
            host.IsTabStop(false); // the field is the stop
            host.VerticalContentAlignment(WUX::VerticalAlignment::Center);
            st.search_end = host;
            put(host, 1, id, 3);
            // Patches address the field itself (text, suggestions, enablement), not its host.
            elems.insert_or_assign(id, box);
        } else if (id == "day::sidebar_toggle") {
            // The sidebar toggle is REALIZED BY THE NAVIGATIONVIEW, not by a bar command: its
            // built-in PaneToggleButton is the hamburger Windows puts at the head of the pane,
            // and drawing our own beside it left the window with two stacked hamburgers doing the
            // same thing. A split nav in this window already shows one, so drop the bar copy.
            //
            // Only the ELEMENT goes: the item stays in day's toolbar model, so `toolbar:` in
            // dayscript still resolves it and still dispatches through the toggle-sidebar duty
            // (which drives `g_navviews` directly, never this button).
            //
            // Unconditional rather than "only when this window has a split nav": the toolbar can
            // be installed before the nav is realized, so the conditional would depend on order.
            // Nothing is lost by always dropping it — in a window with a split nav the built-in
            // button does the job, and in a window without one this button drove nothing anyway
            // (`day_xaml_toggle_sidebar` no-ops on an empty `g_navviews`).
            //
            // Recognized by its RESERVED ID now that a sidebar host contributes an ordinary
            // button for it rather than a kind of its own (docs/toolbars.md). It still takes its
            // place in the bar's order, with no element, so the positions of the items after it
            // line up with Day's.
            put(nullptr, -1, id);
            continue;
        } else if (kind == "L") {
            WUXC::TextBlock caption;
            caption.Text(hs(label.c_str()));
            caption.VerticalAlignment(WUX::VerticalAlignment::Center);
            caption.Margin(WUX::Thickness{ 8, 0, 8, 0 });
            put(caption, 0, id);
        } else if (kind == "T") {
            WUXC::AppBarToggleButton toggle;
            toggle.Label(hs(label.c_str()));
            toggle.Icon(toolbar_icon(glyph, image, geom));
            toggle.IsEnabled(enabled);
            toggle.IsChecked(on);
            if (has_icon && (!trailing || icon_only))
                toggle.LabelPosition(WUXC::CommandBarLabelPosition::Collapsed);
            WUXC::ToolTipService::SetToolTip(toggle, winrt::box_value(hs(tip.c_str())));
            if (action) {
                toggle.Checked([action](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
                    if (g_toolbar_cb && !g_toolbar_setting_checked) g_toolbar_cb(action, 0, 1, "");
                });
                toggle.Unchecked([action](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
                    if (g_toolbar_cb && !g_toolbar_setting_checked) g_toolbar_cb(action, 0, 0, "");
                });
            }
            put_command(toggle, id);
        } else if (kind == "G") {
            // WinUI has no segmented control in the SDK day targets, so this is what a Fluent app
            // builds: a tight row of toggle buttons kept exclusive here. One AppBarElementContainer
            // holds them, so the CommandBar still treats the group as one item.
            std::vector<std::tuple<std::string, std::string, std::string>> segs; // glyph, image, title
            size_t j = i + 1;
            for (; j < lines.size(); ++j) {
                auto ff = split_tabs(lines[j]);
                if (!ff.empty() && ff[0] == "X") break;
                if (ff.size() >= 4 && ff[0] == "g") segs.emplace_back(ff[1], ff[2], ff[3]);
            }
            i = j;
            if (secondary) {
                // In the overflow: a command that opens the choices as checkable rows (see
                // SegmentMenu). The overflow is a menu, and a row of bare glyph buttons at the
                // bottom of one had neither the words nor the check a menu shows.
                auto menu = std::make_shared<SegmentMenu>();
                menu->button = WUXC::AppBarButton{};
                menu->button.IsEnabled(enabled);
                WUXC::MenuFlyout fly;
                std::weak_ptr<SegmentMenu> weak = menu;
                for (size_t n = 0; n < segs.size(); ++n) {
                    const auto& [g, img, title] = segs[n];
                    WUXC::ToggleMenuFlyoutItem row;
                    row.Text(hs(title.c_str()));
                    if (auto ic = toolbar_icon(g, img, "")) row.Icon(ic);
                    row.Click([weak, action, n](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
                        auto m = weak.lock();
                        if (!m) return;
                        // The row toggled itself; a pick-one keeps exactly the clicked row on.
                        m->show(n);
                        if (g_toolbar_cb) g_toolbar_cb(action, 2, static_cast<int>(n), "");
                    });
                    fly.Items().Append(row);
                    menu->rows.push_back(row);
                    menu->titles.push_back(title);
                    menu->icons.emplace_back(g, img);
                }
                menu->button.Flyout(fly);
                menu->show(value >= 0 ? static_cast<size_t>(value) : 0);
                st.segment_menus[id] = menu;
                put_command(menu->button, id);
                continue;
            }
            WUXC::StackPanel row;
            row.Orientation(WUXC::Orientation::Horizontal);
            auto buttons = std::make_shared<std::vector<WUXCP::ToggleButton>>();
            for (size_t n = 0; n < segs.size(); ++n) {
                const auto& [g, img, title] = segs[n];
                auto ic = toolbar_icon(g, img, "");
                WUXCP::ToggleButton b{ nullptr };
                if (ic) {
                    // A glyph segment is a compact AppBarToggleButton: flat at rest and accent
                    // when on, exactly like the bar's other toggles, where a plain ToggleButton
                    // painted each segment as a gray filled block among flat commands.
                    WUXC::AppBarToggleButton ab;
                    ab.Icon(ic);
                    ab.Label(hs(title.c_str()));
                    ab.LabelPosition(WUXC::CommandBarLabelPosition::Collapsed);
                    b = ab;
                } else {
                    // Words only: the AppBar button draws its label under an icon slot, so a
                    // text segment stays a plain toggle button, which draws its content.
                    b = WUXCP::ToggleButton{};
                    b.Content(winrt::box_value(hs(title.c_str())));
                }
                // The segment's name, for a screen reader: an icon segment has no text of its own,
                // and announced as a bare "button".
                WUX::Automation::AutomationProperties::SetName(b, hs(title.c_str()));
                WUXC::ToolTipService::SetToolTip(b, winrt::box_value(hs(title.c_str())));
                b.IsEnabled(enabled);
                b.IsChecked(static_cast<int>(n) == value);
                buttons->push_back(b);
                row.Children().Append(b);
            }
            // Exclusivity, and the report: a click checks one and unchecks the rest, and only the
            // one coming ON is the choice. `g_toolbar_setting_checked` covers the programmatic
            // path, exactly as the plain toggle above does. The handlers hold the list weakly:
            // the list holds the buttons, and a strong reference back kept every removed control
            // alive.
            std::weak_ptr<std::vector<WUXCP::ToggleButton>> weak = buttons;
            for (size_t n = 0; n < buttons->size(); ++n) {
                auto b = (*buttons)[n];
                b.Checked([action, weak, n](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
                    auto buttons = weak.lock();
                    if (!buttons || g_toolbar_setting_checked) return;
                    g_toolbar_setting_checked = true;
                    for (size_t k = 0; k < buttons->size(); ++k)
                        if (k != n) (*buttons)[k].IsChecked(false);
                    g_toolbar_setting_checked = false;
                    if (g_toolbar_cb) g_toolbar_cb(action, 2, static_cast<int>(n), "");
                });
                // Un-checking the chosen segment is not a state a radio row has: put it back.
                b.Unchecked([weak, n](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
                    auto buttons = weak.lock();
                    if (!buttons || g_toolbar_setting_checked) return;
                    bool any = false;
                    for (auto const& x : *buttons) if (x.IsChecked().GetBoolean()) any = true;
                    if (!any) {
                        g_toolbar_setting_checked = true;
                        (*buttons)[n].IsChecked(true);
                        g_toolbar_setting_checked = false;
                    }
                });
            }
            WUXC::AppBarElementContainer host;
            host.Content(row);
            // The segments are the tab stops; the container around them is not one, or Tab
            // paused on an invisible, nameless stop before reaching them.
            host.IsTabStop(false);
            st.segments[id] = buttons;
            put_command(host, id);
        } else if (kind == "M") {
            // The item's own menu spec follows, closed by an `X` line: slice it out and let the
            // menu builder fill a MenuFlyout with it, so a toolbar menu and its menu-bar twin are
            // built by the same code.
            std::string inner;
            size_t j = i + 1;
            for (; j < lines.size(); ++j) {
                auto ff = split_tabs(lines[j]);
                if (!ff.empty() && ff[0] == "X") break;
                inner += lines[j];
                inner += "\n";
            }
            i = j; // resume after the terminator (or at the end of the spec)
            WUXC::AppBarButton button;
            button.Label(hs(label.c_str()));
            button.Icon(toolbar_icon(glyph, image, geom));
            button.IsEnabled(enabled);
            compact(button, has_icon);
            WUXC::ToolTipService::SetToolTip(button, winrt::box_value(hs(tip.c_str())));
            WUXC::MenuFlyout fly;
            build_menu_items(fly.Items(), inner);
            button.Flyout(fly);
            put_command(button, id);
        } else { // "B", and anything a later model adds: a plain command
            WUXC::AppBarButton button;
            button.Label(hs(label.c_str()));
            button.Icon(toolbar_icon(glyph, image, geom));
            button.IsEnabled(enabled);
            compact(button, has_icon);
            WUXC::ToolTipService::SetToolTip(button, winrt::box_value(hs(tip.c_str())));
            if (action) {
                button.Click([action](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
                    if (g_menu_cb) g_menu_cb(action);
                });
            }
            put_command(button, id);
        }
    }

}

// Say in which order a crowded bar folds its right-hand commands into the overflow.
//
// Left to itself the CommandBar folds right to left, which sent a `Primary` command (the last
// thing a crowded chrome gives up, docs/toolbars.md) into the overflow before the `Automatic` ones
// to its left. DynamicOverflowOrder fixes the order, lowest first, but moves every command that
// shares a value TOGETHER, so a value per placement folded a whole role at once and emptied a
// wide bar. Each command gets its own: the `Automatic` ones first, right to left, then the
// `Primary` ones, right to left, and the search field after all of them. A separator takes its
// left neighbor's value, so it leaves with
// the command it follows instead of stranding a divider at the end of the bar. Recomputed after
// every edit, because an insertion shifts the positions that the order is made from.
static void fold_by_role(ToolbarState& st) {
    std::vector<ToolbarEntry*> cmds;
    for (auto& e : st.order)
        if (e.elem && e.region == 1) cmds.push_back(&e);
    int next = 1;
    for (int rank : { 1, 2, 3 }) { // Automatic, Primary, then the search field
        for (size_t k = cmds.size(); k-- > 0;) {
            if (cmds[k]->rank != rank) continue;
            if (auto el = cmds[k]->elem.try_as<DayCommandBarElement>())
                el.DynamicOverflowOrder(next++);
        }
    }
    for (size_t k = 1; k < cmds.size(); ++k) {
        if (!cmds[k]->elem.try_as<WUXC::AppBarSeparator>()) continue;
        auto left = cmds[k - 1]->elem.try_as<DayCommandBarElement>();
        auto sep = cmds[k]->elem.try_as<DayCommandBarElement>();
        if (left && sep) sep.DynamicOverflowOrder(left.DynamicOverflowOrder());
    }
}

// Dock or undock the bar to match what it DRAWS, and hand the window's chrome layout the bar.
//
// What it draws, not what it holds: an item this backend realizes elsewhere (the sidebar toggle,
// which the NavigationView draws) keeps its place in `order` with no element. A window whose only
// item is that one, which is every sidebar app that declares no commands, would otherwise dock
// an empty CommandBar: a blank strip under the title bar, taking a row from the content. The
// state stays while it has any items, so the order still lines up when a drawn one arrives.
static WUXC::CommandBar settle_toolbar(void* win, WUXC::Canvas const& root) {
    auto it = g_toolbar_state.find(win);
    if (it == g_toolbar_state.end()) return nullptr;
    ToolbarState& st = it->second;
    const bool draws = std::any_of(st.order.begin(), st.order.end(),
                                   [](ToolbarEntry const& e) { return static_cast<bool>(e.elem); });
    uint32_t at = 0;
    const bool docked = st.bar && root.Children().IndexOf(st.bar, at);
    if (draws) {
        if (!docked) dock_toolbar(root, st.bar);
        fold_by_role(st);
        return st.bar;
    }
    if (docked) root.Children().RemoveAt(at);
    if (st.order.empty()) {
        g_toolbar_state.erase(it);
        g_toolbar_elems.erase(win);
    }
    return nullptr;
}

// Put item `spec` at position `index` of window `win`'s bar (`secondary` != 0: a SecWindow).
extern "C" void day_xaml_toolbar_insert(void* win, int secondary, int index, const char* spec) try {
    WUXC::Canvas root{ nullptr };
    if (secondary) {
        auto sw = static_cast<SecWindow*>(win);
        if (sw) root = sw->root;
    } else {
        auto app = reinterpret_cast<AppWindow*>(win);
        if (app) root = app->root;
    }
    if (!root || !spec) return;
    ToolbarState& st = toolbar_state(win, root);
    insert_toolbar_item(win, st, static_cast<size_t>((std::max)(index, 0)), spec);
} catch (...) {
}

// Take item `id` off window `win`'s bar. Nothing else on it moves.
extern "C" void day_xaml_toolbar_remove(void* win, const char* id) try {
    auto it = g_toolbar_state.find(win);
    if (it == g_toolbar_state.end() || !id) return;
    ToolbarState& st = it->second;
    const std::string key(id);
    for (size_t k = 0; k < st.order.size(); ++k) {
        if (st.order[k].id != key) continue;
        if (st.order[k].elem && st.order[k].elem == st.search_end) st.search_end = nullptr;
        if (st.order[k].elem) region_remove(st, st.order[k].elem, st.order[k].region);
        st.order.erase(st.order.begin() + k);
        break;
    }
    g_toolbar_elems[win].erase(key);
    st.segments.erase(key);
    st.segment_menus.erase(key);
} catch (...) {
}

// The edit is complete: undock an empty bar and lay the window's chrome out.
extern "C" void day_xaml_toolbar_done(void* win, int secondary) try {
    if (secondary) {
        auto sw = static_cast<SecWindow*>(win);
        if (!sw || !sw->root) return;
        sw->toolbar = settle_toolbar(win, sw->root);
        relayout_sec_chrome(sw);
    } else {
        auto app = reinterpret_cast<AppWindow*>(win);
        if (!app || !app->root) return;
        app->toolbar = settle_toolbar(win, app->root);
        day_xaml_relayout_chrome(app);
    }
} catch (...) {
}

extern "C" void day_xaml_toolbar_set_suggestions(void* win, const char* id, const char* joined) try {
    auto e = find_toolbar_elem(win, id);
    if (auto box = e.try_as<WUXC::AutoSuggestBox>())
        day_xaml_fill_suggestions(box, std::string(joined ? joined : ""));
} catch (...) {
}



extern "C" void day_xaml_toolbar_focus(void* win, const char* id) try {
    auto e = find_toolbar_elem(win, id);
    if (auto c = e.try_as<WUXC::Control>()) c.Focus(WUX::FocusState::Programmatic);
} catch (...) {
}

extern "C" void day_xaml_toolbar_set_text(void* win, const char* id, const char* text) try {
    auto e = find_toolbar_elem(win, id);
    if (!e) return;
    auto box = e.try_as<WUXC::AutoSuggestBox>();
    if (!box) return;
    // Write only a real change, exactly as day_xaml_textbox_set_text does: an identical Text
    // raises no TextChanged at all, so the app's caret and selection survive the sync.
    auto next = hs(text);
    if (box.Text() != next) box.Text(next);
} catch (...) {
}

extern "C" void day_xaml_toolbar_set_selected(void* win, const char* id, int index) try {
    auto state = g_toolbar_state.find(win);
    if (state == g_toolbar_state.end()) return;
    auto menu = state->second.segment_menus.find(std::string(id));
    if (menu != state->second.segment_menus.end()) {
        // Programmatic IsChecked on a menu row raises no Click, so there is no echo to guard.
        if (menu->second && index >= 0) menu->second->show(static_cast<size_t>(index));
        return;
    }
    auto it = state->second.segments.find(std::string(id));
    if (it == state->second.segments.end() || !it->second) return;
    auto& buttons = *it->second;
    if (index < 0 || static_cast<size_t>(index) >= buttons.size()) return;
    g_toolbar_setting_checked = true;
    for (size_t k = 0; k < buttons.size(); ++k)
        buttons[k].IsChecked(static_cast<int>(k) == index);
    g_toolbar_setting_checked = false;
} catch (...) {
    g_toolbar_setting_checked = false;
}

extern "C" void day_xaml_toolbar_set_checked(void* win, const char* id, int on) try {
    auto e = find_toolbar_elem(win, id);
    if (!e) return;
    auto toggle = e.try_as<WUXC::AppBarToggleButton>();
    if (!toggle) return;
    auto current = toggle.IsChecked(); // IReference<bool>: null = indeterminate
    if (current && current.Value() == (on != 0)) return;
    g_toolbar_setting_checked = true;
    toggle.IsChecked(on != 0);
    g_toolbar_setting_checked = false;
} catch (...) {
    g_toolbar_setting_checked = false;
}

extern "C" void day_xaml_toolbar_set_enabled(void* win, const char* id, int on) try {
    auto e = find_toolbar_elem(win, id);
    if (!e) return;
    // A label item is a TextBlock, which has no enabled state — the cast fails and nothing
    // happens, which is the honest outcome for "disable a caption".
    if (auto control = e.try_as<WUXC::Control>()) control.IsEnabled(on != 0);
} catch (...) {
}

