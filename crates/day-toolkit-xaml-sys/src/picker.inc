// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Included by shim.cpp at file scope; owns its C ABI block. Shared declarations and include order live there.

// ---- picker (docs/pickers.md) ---------------------------------------------
// Three stylings behind a flat C ABI: 0 = menu (ComboBox), 1 = segmented (a row of flush
// ToggleButtons), 2 = inline (a vertical RadioButton group). This lived in shim-picker.cpp until
// it was folded in here — the split predated this file and bought nothing: `cc` recompiles every
// source whenever build.rs re-runs, so a one-line edit to the satellite cost a FULL rebuild
// anyway, while the separate translation unit still needed its own copies of `hs`, the namespace
// aliases and the boxing externs.

/// Options arrive newline-joined; empties are dropped (an empty label is not a choice).
static std::vector<std::string> split_items(const char* joined) {
    std::vector<std::string> out;
    std::string all = joined ? joined : "";
    size_t start = 0;
    while (true) {
        size_t nl = all.find('\n', start);
        std::string item =
            all.substr(start, nl == std::string::npos ? std::string::npos : nl - start);
        if (!item.empty()) out.push_back(item);
        if (nl == std::string::npos) break;
        start = nl + 1;
    }
    return out;
}

// A picker's identity for later option changes (PickerPatch::Options): which style it is and
// what its buttons need to report. Kept per handle, like `g_selectable` below, because the
// segmented/inline children carry per-index handlers that a new option list must re-wire.
struct XamlPickerState {
    int style;
    uint64_t id;
    void (*cb)(uint64_t, int);
    // Re-applied to the rebuilt buttons when the option list changes.
    bool enabled = true;
};
static std::map<void*, XamlPickerState> g_pickers;

// Dropped with the handle (day_xaml_delete), so a later handle at the same address is not
// taken for this picker.
static void picker_forget(void* h) { g_pickers.erase(h); }

// Control.IsEnabled for each button of a segmented or inline picker: the StackPanel around them
// is a Panel, not a Control, and carries no IsEnabled of its own.
static void picker_children_enabled(WUXC::Panel const& panel, bool on) {
    for (auto const& kid : panel.Children()) {
        if (auto c = kid.try_as<WUXC::Control>()) c.IsEnabled(on);
    }
}

// The string a picker option was built with (fill_segmented, fill_inline, the combo's items).
static char* picker_content_text(WF::IInspectable const& content) {
    auto s = content.try_as<WF::IReference<winrt::hstring>>();
    return s ? native_dup(u8(s.Value())) : nullptr;
}

// day_xaml_read_native's picker part: `text` is the selected option as displayed, and for the
// button styles `enabled` is the buttons' (all enabled = enabled). A handle that is not a picker
// is left alone.
static void picker_read_native(void* h, UIElement const& el, DayXamlNative* out) {
    if (g_pickers.find(h) == g_pickers.end()) return;
    if (auto box = el.try_as<WUXC::ComboBox>()) {
        if (auto item = box.SelectedItem().try_as<WUXC::ComboBoxItem>()) {
            out->text = picker_content_text(item.Content());
        }
        return;
    }
    auto panel = el.try_as<WUXC::Panel>();
    if (!panel) return;
    bool any = false;
    bool all_on = true;
    for (auto const& kid : panel.Children()) {
        // RadioButton derives from ToggleButton, so this reads both styles.
        auto tb = kid.try_as<WUXCP::ToggleButton>();
        if (!tb) continue;
        any = true;
        if (!tb.IsEnabled()) all_on = false;
        auto on = tb.IsChecked();
        if (!out->text && on && on.Value()) out->text = picker_content_text(tb.Content());
    }
    if (any) out->enabled = all_on ? 1 : 0;
}

// The segmented row's children, built fresh. Mutual exclusion is manual (ToggleButton has no
// GroupName): `guard` keeps the programmatic un-checking of siblings from re-entering as a user
// selection, and re-checking a button the user tried to toggle OFF keeps the group
// always-one-selected, which is what a segmented control means.
static void fill_segmented(WUXC::StackPanel const& row, std::vector<std::string> const& items,
                           int selected, uint64_t id, void (*cb)(uint64_t, int)) {
    row.Children().Clear();
    auto buttons = std::make_shared<std::vector<WUXCP::ToggleButton>>();
    auto guard = std::make_shared<bool>(false);
    for (size_t i = 0; i < items.size(); i++) {
        WUXCP::ToggleButton tb;
        tb.Content(winrt::box_value(hs(items[i].c_str())));
        tb.Margin(WUX::ThicknessHelper::FromLengths(0, 0, 0, 0)); // flush = reads as one control
        tb.IsChecked(static_cast<int>(i) == selected);
        buttons->push_back(tb);
        row.Children().Append(tb);
    }
    for (size_t i = 0; i < buttons->size(); i++) {
        int idx = static_cast<int>(i);
        auto tb = (*buttons)[i];
        tb.Checked([id, cb, idx, buttons, guard](WF::IInspectable const&,
                                                 WUX::RoutedEventArgs const&) {
            if (*guard) return;
            *guard = true;
            for (size_t j = 0; j < buttons->size(); j++) {
                if (static_cast<int>(j) != idx) (*buttons)[j].IsChecked(false);
            }
            *guard = false;
            cb(id, idx);
        });
        tb.Unchecked([buttons, idx, guard](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
            if (*guard) return;
            // The user pressed the already selected segment: keep it on rather than leaving
            // the group with nothing chosen.
            *guard = true;
            (*buttons)[idx].IsChecked(true);
            *guard = false;
        });
    }
}

// The inline group's children: RadioButtons sharing a per-instance GroupName so they are
// mutually exclusive. `Checked` fires on user selection AND programmatic IsChecked, but the
// front-end's bind only re-patches on a real change, so no runaway loop.
static void fill_inline(WUXC::StackPanel const& panel, std::vector<std::string> const& items,
                        int selected, uint64_t id, void (*cb)(uint64_t, int)) {
    panel.Children().Clear();
    winrt::hstring group{ std::to_wstring(id) }; // per-instance group = mutually exclusive
    for (size_t i = 0; i < items.size(); i++) {
        WUXC::RadioButton rb;
        rb.Content(winrt::box_value(hs(items[i].c_str())));
        rb.GroupName(group);
        if (static_cast<int>(i) == selected) rb.IsChecked(true);
        int idx = static_cast<int>(i);
        rb.Checked([id, cb, idx](WF::IInspectable const&, WUX::RoutedEventArgs const&) {
            cb(id, idx);
        });
        panel.Children().Append(rb);
    }
}

extern "C" {

void* day_picker_xaml_new(int style, const char* items_joined, int selected, uint64_t id,
                          void (*cb)(uint64_t, int)) {
    auto items = split_items(items_joined);
    if (style == 0) {
        WUXC::ComboBox box;
        for (auto& it : items) {
            WUXC::ComboBoxItem cbi;
            cbi.Content(winrt::box_value(hs(it.c_str())));
            box.Items().Append(cbi);
        }
        if (selected >= 0 && selected < static_cast<int>(items.size())) box.SelectedIndex(selected);
        box.SelectionChanged(
            [id, cb](WF::IInspectable const& s, WUXC::SelectionChangedEventArgs const&) {
                cb(id, s.as<WUXC::ComboBox>().SelectedIndex());
            });
        void* h = day_xaml_box(winrt::get_abi(box));
        g_pickers[h] = XamlPickerState{ style, id, cb };
        return h;
    }
    if (style == 1) {
        // SEGMENTED: a row of flush ToggleButtons, which is what Windows uses for a small
        // either/or choice (the Settings app's own segmented pickers, and what WinUI later named
        // SelectorBar). This used to be horizontal RadioButtons — identical to the `Inline` style
        // bar its orientation, so the two rendered the same and neither read as segmented.
        //
        // Mutual exclusion is manual: ToggleButton has no GroupName. `guard` keeps the
        // programmatic un-checking of the siblings from re-entering as a user selection, and
        // re-checking a button the user tried to toggle OFF keeps the group always-one-selected,
        // which is what a segmented control means (and what the picker model promises).
        WUXC::StackPanel row;
        row.Orientation(WUXC::Orientation::Horizontal);
        fill_segmented(row, items, selected, id, cb);
        void* h = day_xaml_box(winrt::get_abi(row));
        g_pickers[h] = XamlPickerState{ style, id, cb };
        return h;
    }
    WUXC::StackPanel panel;
    panel.Orientation(WUXC::Orientation::Vertical);
    fill_inline(panel, items, selected, id, cb);
    void* h = day_xaml_box(winrt::get_abi(panel));
    g_pickers[h] = XamlPickerState{ style, id, cb };
    return h;
}

// New option labels (PickerPatch::Options). The combo refills its items; the button styles are
// rebuilt from scratch, because each child carries a handler that captured its own index — the
// same construction path, so the two can never drift. The selection survives where it still
// exists.
void day_picker_xaml_set_options(void* handle, const char* items_joined) {
    auto items = split_items(items_joined);
    auto st = g_pickers.find(handle);
    if (st == g_pickers.end()) return;
    WUX::UIElement e{ nullptr };
    winrt::copy_from_abi(e, day_xaml_unbox(handle));
    int last = static_cast<int>(items.size()) - 1;
    if (auto box = e.try_as<WUXC::ComboBox>()) {
        int keep = box.SelectedIndex();
        box.Items().Clear();
        for (auto& it : items) {
            WUXC::ComboBoxItem cbi;
            cbi.Content(winrt::box_value(hs(it.c_str())));
            box.Items().Append(cbi);
        }
        if (last >= 0) box.SelectedIndex(keep < 0 ? 0 : (keep > last ? last : keep));
        return;
    }
    auto panel = e.try_as<WUXC::StackPanel>();
    if (!panel) return;
    int keep = 0;
    auto kids = panel.Children();
    for (uint32_t i = 0; i < kids.Size(); i++) {
        if (auto tb = kids.GetAt(i).try_as<WUXCP::ToggleButton>()) {
            if (tb.IsChecked().GetBoolean()) keep = static_cast<int>(i);
        } else if (auto rb = kids.GetAt(i).try_as<WUXC::RadioButton>()) {
            if (rb.IsChecked().GetBoolean()) keep = static_cast<int>(i);
        }
    }
    if (keep > last) keep = last < 0 ? 0 : last;
    if (st->second.style == 1) fill_segmented(panel, items, keep, st->second.id, st->second.cb);
    else fill_inline(panel, items, keep, st->second.id, st->second.cb);
    if (!st->second.enabled) picker_children_enabled(panel, false);
}

// PickerProps::enabled / PickerPatch::Enabled. The combo is a Control; the button styles disable
// each button (picker_children_enabled), and remember it for a later option rebuild.
void day_picker_xaml_set_enabled(void* handle, int on) {
    auto st = g_pickers.find(handle);
    if (st != g_pickers.end()) st->second.enabled = on != 0;
    guard([&] {
        WUX::UIElement e{ nullptr };
        winrt::copy_from_abi(e, day_xaml_unbox(handle));
        if (auto box = e.try_as<WUXC::ComboBox>()) {
            box.IsEnabled(on != 0);
            return;
        }
        if (auto panel = e.try_as<WUXC::Panel>()) picker_children_enabled(panel, on != 0);
    });
}

void day_picker_xaml_set_selected(void* handle, int idx) try {
    WUX::UIElement e{ nullptr };
    winrt::copy_from_abi(e, day_xaml_unbox(handle));
    if (auto box = e.try_as<WUXC::ComboBox>()) {
        if (idx < 0 || static_cast<uint32_t>(idx) >= box.Items().Size()) return;
        if (box.SelectedIndex() != idx) box.SelectedIndex(idx);
        return;
    }
    if (auto panel = e.try_as<WUXC::Panel>()) {
        auto kids = panel.Children();
        if (idx < 0 || static_cast<uint32_t>(idx) >= kids.Size()) return;
        if (auto rb = kids.GetAt(idx).try_as<WUXC::RadioButton>()) {
            rb.IsChecked(true);
            return;
        }
        // Segmented: set only the TARGET and let its own `Checked` handler clear the siblings.
        // Clearing them here instead would trip each one's `Unchecked` handler, which re-checks to
        // keep the group non-empty — the two would fight. XAML raises `Checked` only on an actual
        // change, so re-selecting the current segment is a no-op.
        if (auto tb = kids.GetAt(idx).try_as<WUXCP::ToggleButton>()) tb.IsChecked(true);
    }
} catch (...) {
    // No WinRT exception may unwind across this Rust FFI boundary.
}

} // extern "C"

