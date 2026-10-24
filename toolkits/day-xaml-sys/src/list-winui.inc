// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// Included only by the WinUI build. ItemsRepeater/StackLayout own realization and
// scroll extents; Day owns the contents of each recycled Canvas. Factory callbacks
// only post work, since calling ListSource during XAML layout would re-enter Day.
namespace {
struct WinuiListCell {
    WUXC::ItemContainer root;
    WUXC::SwipeControl swipe;
    WUXC::Canvas content;
    WUXC::MenuFlyout menu;
    WUXC::SwipeItems leading;
    WUXC::SwipeItems trailing;
    void* handle = nullptr;
    int row = -1;
    uint64_t generation = 0;
    ~WinuiListCell() { if (handle) day_xaml_delete(handle); }
};
struct WinuiList {
    WUXC::ScrollViewer scroll;
    WUXC::ItemsRepeater repeater;
    WUXC::StackLayout layout;
    std::vector<std::shared_ptr<WinuiListCell>> cells;
    std::set<int> selection;
    void* host = nullptr;
    int count = 0, focus = 0, pending = -2;
    bool pending_nearest = false;
    winrt::event_token reveal_frame{};
    std::weak_ptr<WinuiListCell> drag;
    uint64_t drag_generation = 0;
    double pitch = 44;
    bool selectable = false, activatable = false, reorderable = false, syncing = false;
    void (*changed)(void*) = nullptr;
    void (*selected)(void*, int, int) = nullptr;
    void (*offer)(void*, void*, int) = nullptr;
    void (*action)(void*, int, uint64_t, int, int) = nullptr;
    int (*can_move)(void*, int, int) = nullptr;
    void (*moved)(void*, int, int) = nullptr;
    void notify() { if (host) changed(host); }
};
std::map<void*, std::shared_ptr<WinuiList>> winui_lists;
static std::shared_ptr<WinuiList> winui_list(void* host) {
    auto it = winui_lists.find(host);
    return it == winui_lists.end() ? nullptr : it->second;
}
static std::shared_ptr<WinuiListCell> winui_cell(WinuiList const& list, void* handle) {
    for (auto const& c : list.cells) if (c->handle == handle) return c;
    return nullptr;
}
static int winui_list_modifiers() {
    return ((GetKeyState(VK_CONTROL) & 0x8000) ? 1 : 0)
        | ((GetKeyState(VK_SHIFT) & 0x8000) ? 2 : 0);
}
static bool winui_list_embedded(WF::IInspectable const& source, WinuiListCell const& cell) {
    auto current = source.try_as<WUX::DependencyObject>();
    while (current && current != cell.root) {
        if (current.try_as<WUXC::Control>() && current != cell.swipe) return true;
        current = WUXM::VisualTreeHelper::GetParent(current);
    }
    return false;
}
static bool winui_list_reveal_landed(WinuiList const& list, int row, bool nearest) {
    auto element = list.repeater.TryGetElement(row).try_as<WUX::FrameworkElement>();
    if (!element || element.ActualWidth() <= 0 || element.ActualHeight() <= 0) return false;
    auto bounds = element.TransformToVisual(list.scroll).TransformBounds(
        {0, 0, float(element.ActualWidth()), float(element.ActualHeight())});
    double viewport = list.scroll.ViewportHeight();
    if (nearest) {
        return (bounds.Y >= -.5 && bounds.Y + bounds.Height <= viewport + .5) ||
            (bounds.Y <= 0 && bounds.Y + bounds.Height >= viewport);
    }
    double alignment = list.pending < 0 ? viewport - bounds.Height : 0;
    double offset = list.scroll.VerticalOffset();
    double destination = std::clamp(offset + bounds.Y - alignment, 0.0,
        list.scroll.ScrollableHeight());
    return std::abs(destination - offset) <= .5;
}
static void winui_list_reveal(WinuiList& list) {
    if (list.pending == -2 || list.reveal_frame.value || list.count == 0 ||
        list.scroll.ViewportHeight() <= 0) return;
    int row = list.pending < 0 ? list.count - 1 : (std::min)(list.pending, list.count - 1);
    auto element = list.repeater.GetOrCreateElement(row);
    list.repeater.UpdateLayout();
    WUX::BringIntoViewOptions options;
    options.AnimationDesired(false);
    if (!list.pending_nearest) options.VerticalAlignmentRatio(list.pending < 0 ? 1.0 : 0.0);
    int request = list.pending;
    bool nearest = list.pending_nearest;
    element.StartBringIntoView(options);
    // A queued keyboard jump may not have a container until this request runs.
    if (nearest) element.as<WUXC::Control>().Focus(WUX::FocusState::Keyboard);
    // ItemsRepeater retains its bring-into-view anchor until Rendering. Another request
    // before that frame can replace the anchor while the first scroll is still pending.
    // Keep the request until its target reaches the viewport. Rendering releases the
    // repeater's anchor, but does not guarantee ScrollViewer has applied the offset yet.
    // Retry on a later frame if it was lost; newer requests replace the queued target.
    // Only post work from Rendering, since it can run inside Day's layout borrow.
    std::weak_ptr<WinuiList> weak = winui_list(list.host);
    list.reveal_frame = WUXM::CompositionTarget::Rendering([weak, request, row, nearest](auto const&, auto const&) {
        if (auto l = weak.lock()) {
            WUXM::CompositionTarget::Rendering(l->reveal_frame);
            l->reveal_frame = {};
            int target = request < 0 ? l->count - 1 : (std::min)(request, l->count - 1);
            if (l->pending == request && l->pending_nearest == nearest && row == target &&
                winui_list_reveal_landed(*l, row, nearest)) l->pending = -2;
            if (l->host && l->pending != -2) l->notify();
        }
    });
    list.notify();
}
static std::shared_ptr<WinuiListCell> winui_list_make_cell(std::shared_ptr<WinuiList> const& list) {
    auto cell = std::make_shared<WinuiListCell>();
    cell->handle = boxh(cell->content);
    cell->content.Background(WUXM::SolidColorBrush(color_argb(0)));
    cell->swipe.Content(cell->content);
    cell->root.Child(cell->swipe);
    // ItemContainer's theme padding is for text templates. Day supplies the complete
    // row (including its padding), so subtracting that chrome clips short/automatic rows.
    cell->root.Padding({0, 0, 0, 0});
    cell->root.MinHeight(0);
    cell->root.HorizontalContentAlignment(WUX::HorizontalAlignment::Stretch);
    cell->root.VerticalContentAlignment(WUX::VerticalAlignment::Stretch);
    cell->swipe.Padding({0, 0, 0, 0});
    cell->swipe.MinHeight(0);
    cell->swipe.HorizontalContentAlignment(WUX::HorizontalAlignment::Stretch);
    cell->swipe.VerticalContentAlignment(WUX::VerticalAlignment::Stretch);
    cell->root.Height(list->pitch);
    cell->root.HorizontalAlignment(WUX::HorizontalAlignment::Stretch);
    cell->root.IsTabStop(list->selectable || list->activatable);
    cell->root.ContextFlyout(cell->menu);
    std::weak_ptr<WinuiList> weak = list;
    std::weak_ptr<WinuiListCell> wc = cell;
    // ItemContainer's automation peer writes IsSelected directly. Feed that back
    // through the same model as pointer/keyboard selection, without echoing our paint.
    cell->root.RegisterPropertyChangedCallback(WUXC::ItemContainer::IsSelectedProperty(),
        [weak, wc](auto const&, auto const&) {
            auto l = weak.lock(); auto c = wc.lock();
            if (l && l->host && !l->syncing && c && c->row >= 0 && l->selectable)
                l->selected(l->host, c->row, c->root.IsSelected() ? 32 : 64);
        });
    cell->root.Tapped([weak, wc](auto const&, WUXIn::TappedRoutedEventArgs const& e) {
        auto l = weak.lock(); auto c = wc.lock();
        if (!l || !l->host || !c || c->row < 0 || winui_list_embedded(e.OriginalSource(), *c)) return;
        l->focus = c->row;
        c->root.Focus(WUX::FocusState::Pointer);
        if (l->selectable) l->selected(l->host, c->row, winui_list_modifiers());
    });
    cell->root.DoubleTapped([weak, wc](auto const&, WUXIn::DoubleTappedRoutedEventArgs const& e) {
        auto l = weak.lock(); auto c = wc.lock();
        if (l && l->host && c && c->row >= 0 && l->activatable && !winui_list_embedded(e.OriginalSource(), *c))
            l->selected(l->host, c->row, 4);
    });
    cell->root.GotFocus([weak, wc](auto const&, auto const&) {
        if (auto l = weak.lock()) if (auto c = wc.lock()) if (c->row >= 0) l->focus = c->row;
    });
    cell->root.PointerPressed([weak, wc](auto const&, auto const&) {
        auto l = weak.lock(); auto c = wc.lock();
        if (l && l->host && c && c->row >= 0) l->offer(l->host, c->handle, c->row);
    });
    // Refresh menus opened with the keyboard as well as those opened by a pointer.
    cell->menu.Opening([weak, wc](auto const&, auto const&) {
        auto l = weak.lock(); auto c = wc.lock();
        if (l && l->host && c && c->row >= 0) l->offer(l->host, c->handle, c->row);
    });
    if (list->reorderable) {
        using Operation = winrt::Windows::ApplicationModel::DataTransfer::DataPackageOperation;
        cell->root.CanDrag(true);
        cell->root.AllowDrop(true);
        cell->root.DragStarting([weak, wc](auto const&, WUX::DragStartingEventArgs const& e) {
            auto l = weak.lock(); auto c = wc.lock();
            if (!l || !l->host || !c || c->row < 0 || l->can_move(l->host, c->row, c->row) < 0) {
                e.Cancel(true); return;
            }
            l->drag = c; l->drag_generation = c->generation;
            e.Data().SetText(L"Day list row");
            e.Data().RequestedOperation(Operation::Move);
        });
        cell->root.DropCompleted([weak](auto const&, auto const&) {
            if (auto l = weak.lock()) l->drag.reset();
        });
        cell->root.DragOver([weak, wc](auto const&, WUX::DragEventArgs const& e) {
            auto l = weak.lock(); auto c = wc.lock();
            if (!l || !l->host || !c || c->row < 0) return;
            auto from = l->drag.lock();
            if (!from || from->generation != l->drag_generation || from->row < 0) return;
            e.AcceptedOperation(l->can_move(l->host, from->row, c->row) >= 0 ? Operation::Move : Operation::None);
            e.Handled(true);
        });
        cell->root.Drop([weak, wc](auto const&, WUX::DragEventArgs const& e) {
            auto l = weak.lock(); auto c = wc.lock();
            if (!l || !l->host || !c || c->row < 0) return;
            auto from = l->drag.lock(); l->drag.reset();
            if (!from || from->generation != l->drag_generation || from->row < 0) return;
            int target = l->can_move(l->host, from->row, c->row);
            if (target >= 0 && target != from->row) l->moved(l->host, from->row, target);
            e.Handled(true);
        });
    }
    list->cells.push_back(cell);
    return cell;
}
struct WinuiListFactory : winrt::implements<WinuiListFactory, WUX::IElementFactory> {
    std::weak_ptr<WinuiList> list;
    explicit WinuiListFactory(std::shared_ptr<WinuiList> const& value) : list(value) {}
    UIElement GetElement(WUX::ElementFactoryGetArgs const& args) {
        auto l = list.lock();
        if (!l) return nullptr;
        std::shared_ptr<WinuiListCell> cell;
        for (auto const& c : l->cells) if (c->row < 0) { cell = c; break; }
        if (!cell) cell = winui_list_make_cell(l);
        cell->row = winrt::unbox_value<int>(args.Data());
        ++cell->generation;
        l->syncing = true;
        cell->root.IsSelected(l->selection.count(cell->row) != 0);
        l->syncing = false;
        cell->root.Height(l->pitch);
        l->notify();
        return cell->root;
    }
    void RecycleElement(WUX::ElementFactoryRecycleArgs const& args) {
        if (auto l = list.lock()) {
            for (auto const& c : l->cells) if (c->root == args.Element()) {
                c->row = -1;
                ++c->generation;
                c->swipe.Close();
                c->menu.Hide();
                // Old action closures must never follow this physical cell to a new row.
                c->menu.Items().Clear();
                c->swipe.LeftItems(nullptr); c->swipe.RightItems(nullptr);
                break;
            }
            l->notify();
        }
    }
};
} // namespace

extern "C" {
struct DayWinuiListCell {
    void* cell;
    int row;
    uint64_t generation;
    double width;
    int visible;
};
void* day_winui_list_new(double pitch, int selectable, int activatable, int reorderable,
    void (*changed)(void*), void (*selected)(void*, int, int),
    void (*offer)(void*, void*, int), void (*action)(void*, int, uint64_t, int, int),
    int (*can_move)(void*, int, int), void (*moved)(void*, int, int)) {
    auto l = std::make_shared<WinuiList>();
    l->pitch = pitch; l->selectable = selectable != 0; l->activatable = activatable != 0;
    l->reorderable = reorderable != 0;
    l->changed = changed; l->selected = selected; l->offer = offer; l->action = action;
    l->can_move = can_move; l->moved = moved;
    l->layout.Orientation(WUXC::Orientation::Vertical);
    l->repeater.Layout(l->layout);
    l->repeater.ItemTemplate(winrt::make<WinuiListFactory>(l));
    l->repeater.VerticalCacheLength(1.0);
    l->scroll.HorizontalScrollMode(WUXC::ScrollMode::Disabled);
    l->scroll.HorizontalScrollBarVisibility(WUXC::ScrollBarVisibility::Disabled);
    l->scroll.VerticalScrollMode(WUXC::ScrollMode::Enabled);
    l->scroll.VerticalScrollBarVisibility(WUXC::ScrollBarVisibility::Auto);
    l->scroll.Content(l->repeater);
    l->scroll.IsTabStop(true);
    l->host = boxh(l->scroll);
    winui_lists.emplace(l->host, l);
    std::weak_ptr<WinuiList> weak = l;
    l->scroll.ViewChanged([weak](auto const&, auto const&) { if (auto l = weak.lock()) l->notify(); });
    l->scroll.SizeChanged([weak](auto const&, auto const&) { if (auto l = weak.lock()) l->notify(); });
    l->scroll.Loaded([weak](auto const&, auto const&) { if (auto l = weak.lock()) l->notify(); });
    l->scroll.PreviewKeyDown([weak](auto const&, WUXIn::KeyRoutedEventArgs const& e) {
        auto l = weak.lock();
        if (!l || !l->host || l->count == 0) return;
        // Intercept list navigation before ScrollViewer/ItemContainer consume it
        // or move focus; leave embedded editors' keys to those controls.
        if (e.OriginalSource() != l->scroll) {
            for (auto const& c : l->cells) if (c->row == l->focus && winui_list_embedded(e.OriginalSource(), *c)) return;
        }
        auto key = e.Key(); int target = l->focus, mods = winui_list_modifiers();
        if (key == WS::VirtualKey::Enter) {
            if (!l->activatable) return;
            l->selected(l->host, target, 4);
        } else if (key == WS::VirtualKey::A && (mods & 1) && l->selectable) {
            l->selected(l->host, target, 16);
        } else {
            if (key == WS::VirtualKey::Up) --target;
            else if (key == WS::VirtualKey::Down) ++target;
            else if (key == WS::VirtualKey::Home) target = 0;
            else if (key == WS::VirtualKey::End) target = l->count - 1;
            else if (key == WS::VirtualKey::PageUp) target -= (std::max)(1, int(l->scroll.ViewportHeight() / l->pitch));
            else if (key == WS::VirtualKey::PageDown) target += (std::max)(1, int(l->scroll.ViewportHeight() / l->pitch));
            else if (key != WS::VirtualKey::Space || !l->selectable) return;
            target = std::clamp(target, 0, l->count - 1);
            l->focus = target; l->pending = target;
            l->pending_nearest = true;
            winui_list_reveal(*l);
            if (l->selectable && (!(mods & 1) || key == WS::VirtualKey::Space)) l->selected(l->host, target, mods);
        }
        e.Handled(true);
    });
    return l->host;
}
void day_winui_list_forget(void* host) {
    guard([&] {
        auto l = winui_list(host); if (!l) return;
        if (l->reveal_frame.value) {
            WUXM::CompositionTarget::Rendering(l->reveal_frame);
            l->reveal_frame = {};
        }
        l->host = nullptr;
        l->repeater.ItemsSource(nullptr);
        l->repeater.ItemTemplate(nullptr);
        l->scroll.Content(nullptr);
        winui_lists.erase(host);
    });
}
void day_winui_list_frame(void* host, double width, double height) {
    guard([&] { if (auto l = winui_list(host)) { l->scroll.Width(width); l->scroll.Height(height); } });
}
void day_winui_list_reload(void* host, int count) {
    guard([&] {
        auto l = winui_list(host); if (!l) return;
        l->count = count;
        l->drag.reset();
        std::vector<WF::IInspectable> rows;
        rows.reserve(count);
        for (int i = 0; i < count; ++i) rows.push_back(winrt::box_value(i));
        l->repeater.ItemsSource(winrt::single_threaded_observable_vector<WF::IInspectable>(std::move(rows)));
        l->focus = std::clamp(l->focus, 0, (std::max)(0, count - 1));
    });
}
void day_winui_list_layout(void* host) {
    guard([&] { if (auto l = winui_list(host)) { l->scroll.UpdateLayout(); winui_list_reveal(*l); } });
}
int day_winui_list_cells(void* host, DayWinuiListCell* out, int capacity) {
    auto l = winui_list(host); if (!l) return 0;
    int i = 0;
    guard([&] {
        for (auto const& c : l->cells) {
            if (c->row < 0) continue;
            if (out && i < capacity) {
                auto bounds = c->root.TransformToVisual(l->scroll).TransformBounds(
                    {0, 0, float(c->root.ActualWidth()), float(c->root.ActualHeight())});
                out[i] = {c->handle, c->row, c->generation, c->root.ActualWidth(),
                    bounds.Y + bounds.Height > 0 && bounds.Y < l->scroll.ViewportHeight() ? 1 : 0};
            }
            ++i;
        }
    });
    return i;
}
void day_winui_list_cell_size(void* host, void* cell, uint64_t generation, double width, double height) {
    guard([&] {
        auto l = winui_list(host); if (!l) return;
        auto c = winui_cell(*l, cell); if (!c || c->generation != generation) return;
        c->content.Width(width); c->content.Height(height);
        if (std::abs(c->root.Height() - height) > .25) { c->root.Height(height); l->notify(); }
    });
}
void day_winui_list_scroll(void* host, int row) {
    guard([&] {
        if (auto l = winui_list(host)) {
            l->pending = row;
            l->pending_nearest = false;
            winui_list_reveal(*l);
        }
    });
}
void day_winui_list_select(void* host, int const* rows, int len) {
    guard([&] {
        auto l = winui_list(host); if (!l) return;
        l->selection.clear();
        for (int i = 0; i < len; ++i) if (rows[i] >= 0 && rows[i] < l->count) l->selection.insert(rows[i]);
        l->syncing = true;
        for (auto const& c : l->cells) c->root.IsSelected(c->row >= 0 && l->selection.count(c->row));
        l->syncing = false;
    });
}
void day_winui_list_actions_begin(void* host, void* cell) {
    guard([&] {
        auto l = winui_list(host); if (!l) return;
        auto c = winui_cell(*l, cell); if (!c) return;
        c->leading = WUXC::SwipeItems(); c->trailing = WUXC::SwipeItems();
        c->menu.Items().Clear();
    });
}
void day_winui_list_action_add(void* host, void* cell, uint64_t token, int edge, int index,
    char const* label, char const* glyph, int destructive, double const* color) {
    guard([&] {
        auto l = winui_list(host); if (!l) return;
        auto c = winui_cell(*l, cell); if (!c || c->row < 0) return;
        std::weak_ptr<WinuiList> weak = l; std::weak_ptr<WinuiListCell> wc = c;
        auto invoke = [weak, wc, generation = c->generation, row = c->row, token, edge, index]() {
            auto l = weak.lock(); auto c = wc.lock();
            if (l && l->host && c && c->generation == generation && c->row == row)
                l->action(l->host, row, token, edge, index);
        };
        auto text = winrt::to_hstring(label ? label : "");
        WUXC::SwipeItem item; item.Text(text);
        if (color) {
            auto byte = [](double v) { return uint8_t(std::clamp(v, 0.0, 1.0) * 255); };
            item.Background(WUXM::SolidColorBrush({byte(color[3]), byte(color[0]), byte(color[1]), byte(color[2])}));
        } else if (destructive) item.Background(WUXM::SolidColorBrush(color_argb(0xffc42b1c)));
        item.Invoked([invoke](auto const&, auto const&) { invoke(); });
        (edge == 0 ? c->leading : c->trailing).Append(item);
        WUXC::MenuFlyoutItem menu; menu.Text(text);
        menu.Click([invoke](auto const&, auto const&) { invoke(); });
        if (glyph && *glyph) {
            menu.Icon(toolbar_icon(glyph, ""));
            WUXC::FontIconSource icon;
            icon.FontFamily(WUXM::FontFamily(toolbar_icon_font()));
            icon.Glyph(winrt::hstring{std::wstring(1, wchar_t(std::strtoul(glyph, nullptr, 16)))});
            item.IconSource(icon);
        }
        c->menu.Items().Append(menu);
    });
}
void day_winui_list_actions_end(void* host, void* cell) {
    guard([&] {
        auto l = winui_list(host); if (!l) return;
        auto c = winui_cell(*l, cell); if (!c) return;
        bool rtl = c->root.FlowDirection() == WUX::FlowDirection::RightToLeft;
        auto left = rtl ? c->trailing : c->leading;
        auto right = rtl ? c->leading : c->trailing;
        c->swipe.LeftItems(left.Size() ? left : nullptr);
        c->swipe.RightItems(right.Size() ? right : nullptr);
        c->root.ContextFlyout(c->menu.Items().Size() ? c->menu : nullptr);
        // Action-only lists still need a keyboard route to their context menu.
        c->root.IsTabStop(l->selectable || l->activatable || c->menu.Items().Size() != 0);
    });
}
} // extern "C"
