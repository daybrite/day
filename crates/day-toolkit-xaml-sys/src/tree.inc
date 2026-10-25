// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Included by shim.cpp inside the C ABI block. Shared declarations and include order live there.

// TreeView owns disclosure and selection. Day owns the Canvas row content and its scopes.
struct DayNativeTree {
    WUXC::TreeView view{nullptr};
    std::map<uint64_t,WUXC::TreeViewNode> nodes;
    std::map<uint64_t,WUXC::Canvas> cells;
    std::set<uint64_t> headers;
    std::vector<uint64_t> selection;
    bool suppress = false;
    bool selection_pending = false;
    bool containers_hooked = false;
    double height = 36;
    void *handle = nullptr;
    uint64_t id = 0;
    void (*expanded)(uint64_t,uint64_t,int) = nullptr;
    void (*selected)(uint64_t,const uint64_t*,int) = nullptr;
    void (*viewport)(void*) = nullptr;
    uint64_t token(WUXC::TreeViewNode const& node) const {
        for (auto const& pair : nodes) if (pair.second == node) return pair.first;
        return 0;
    }
};
static std::map<void*,std::shared_ptr<DayNativeTree>> day_native_trees;
void *day_xaml_tree_new(uint64_t id, double height, double indent, int selectable, int multi,
    void (*expanded)(uint64_t,uint64_t,int), void (*selected)(uint64_t,const uint64_t*,int), void (*viewport)(void*)) {
    void *result = nullptr;
    guard([&] {
        auto s = std::make_shared<DayNativeTree>(); s->view = WUXC::TreeView();
        // A reusable host, not a binding to the cached Canvas: a recycled presenter can
        // retain that UIElement and leave its next container blank (one native parent only).
        // tree_frame explicitly moves the Canvas into its current realized host.
        s->view.ItemTemplate(WUXMk::XamlReader::Load(
            L"<DataTemplate xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\">"
            L"<Grid/>"
            L"</DataTemplate>").as<WUX::DataTemplate>());
        s->id=id; s->height=height; s->expanded=expanded; s->selected=selected; s->viewport=viewport;
        s->view.CanDragItems(false); s->view.CanReorderItems(false);
        s->view.SelectionMode(!selectable ? WUXC::TreeViewSelectionMode::None : multi ? WUXC::TreeViewSelectionMode::Multiple : WUXC::TreeViewSelectionMode::Single);
        result=boxh(s->view); s->handle=result; day_native_trees[result]=s;
        std::weak_ptr<DayNativeTree> weak=s;
        s->view.Expanding([weak](auto const&, auto const& args) {
            if(auto s=weak.lock(); s && !s->suppress) { s->expanded(s->id,s->token(args.Node()),1); s->viewport(s->handle); }
        });
        s->view.Collapsed([weak](auto const&, auto const& args) {
            if(auto s=weak.lock(); s && !s->suppress) { s->expanded(s->id,s->token(args.Node()),0); s->viewport(s->handle); }
        });
        auto reportSelection = [weak](auto const&, auto const&) {
            if (auto s = weak.lock(); s && !s->suppress && !s->selection_pending) {
                // SelectedNodes is still changing during SelectionChanged. Reading or
                // correcting it here observes intermediate empty selections and re-enters
                // WinUI's synchronization (a native stack overflow when rejecting a header).
                // Coalesce notifications and read the final selection on the next UI turn.
                s->selection_pending = true;
                day_xaml_post([](void* raw) {
                    std::unique_ptr<std::weak_ptr<DayNativeTree>> pending(
                        static_cast<std::weak_ptr<DayNativeTree>*>(raw));
                    if (auto s = pending->lock()) {
                        s->selection_pending = false;
                        guard([&] {
                            auto selected = s->view.SelectedNodes();
                            std::vector<uint64_t> tokens;
                            bool heading = false;
                            for (auto const& node : selected) {
                                auto token = s->token(node);
                                if (s->headers.count(token)) heading = true;
                                else tokens.push_back(token);
                            }
                            if (heading) {
                                s->suppress = true;
                                try {
                                    selected.Clear();
                                    for (auto token : s->selection) {
                                        auto node = s->nodes.find(token);
                                        if (node != s->nodes.end()) selected.Append(node->second);
                                    }
                                } catch (...) {
                                    s->suppress = false;
                                    throw;
                                }
                                s->suppress = false;
                            } else if (tokens != s->selection) {
                                s->selection = tokens;
                                s->selected(s->id, tokens.data(), int(tokens.size()));
                            }
                        });
                    }
                }, new std::weak_ptr<DayNativeTree>(weak));
            }
        };
#ifdef DAY_WINUI
        s->view.SelectionChanged(reportSelection);
#else
        // System XAML has no TreeView.SelectionChanged; observe the native template's
        // ListViewBase selection so keyboard, pointer and accessibility changes all report.
        s->view.Loaded([weak, reportSelection](auto const&, auto const&) {
            if(auto s=weak.lock()) {
                std::vector<WUX::DependencyObject> pending{s->view};
                while(!pending.empty()) {
                    auto current=pending.back(); pending.pop_back();
                    if(auto list=current.try_as<WUXC::ListViewBase>()) {
                        list.SelectionChanged(reportSelection); break;
                    }
                    for(int i=0;i<WUXM::VisualTreeHelper::GetChildrenCount(current);++i)
                        pending.push_back(WUXM::VisualTreeHelper::GetChild(current,i));
                }
            }
        });
#endif
        s->view.SizeChanged([weak](auto const&, auto const&) { if(auto s=weak.lock()) s->viewport(s->handle); });
        s->view.Loaded([weak](auto const&, auto const&) {
            if (auto s = weak.lock()) {
                if (!s->containers_hooked) {
                    std::vector<WUX::DependencyObject> pending{s->view};
                    while (!pending.empty()) {
                        auto current = pending.back(); pending.pop_back();
                        if (auto list = current.try_as<WUXC::ListViewBase>()) {
                            list.ContainerContentChanging([weak](auto const&, auto const& args) {
                                if (auto s = weak.lock()) {
                                    if (args.InRecycleQueue()) {
                                        if (auto host = args.ItemContainer().ContentTemplateRoot().template try_as<WUXC::Panel>())
                                            host.Children().Clear();
                                    }
                                    // Realization can follow Expanding, and scrolling realizes
                                    // rows without changing the TreeView's size.
                                    s->viewport(s->handle);
                                }
                            });
                            s->containers_hooked = true;
                            break;
                        }
                        for (int i = 0; i < WUXM::VisualTreeHelper::GetChildrenCount(current); ++i)
                            pending.push_back(WUXM::VisualTreeHelper::GetChild(current, i));
                    }
                }
                s->viewport(s->handle);
            }
        });
    });
    return result;
}
void day_xaml_tree_forget(void *w) { day_native_trees.erase(w); }
void day_xaml_tree_begin(void *w) { guard([&] {
    auto s=day_native_trees.at(w); s->suppress=true;
    // WinUI selection tracks subscriptions on the old TreeViewNodes. Release those
    // while the nodes are still attached; clearing selection after RootNodes can
    // free stale tracker handles (native heap corruption during a selected reload).
    // Day restores the saved token selection after the replacement nodes are added.
    s->view.SelectedNodes().Clear();
    for(auto const& p:s->nodes) p.second.Content(nullptr);
    s->view.RootNodes().Clear(); s->nodes.clear(); s->headers.clear();
}); }
void day_xaml_tree_add(void *w, uint64_t token, uint64_t parent, int hasParent, const char *text, int expandable, int header, int open) { guard([&] {
    auto s=day_native_trees.at(w); WUXC::TreeViewNode n;
    n.HasUnrealizedChildren(expandable != 0); n.IsExpanded(open != 0);
    auto found=s->cells.find(token);
    if(found==s->cells.end()) {
        WUXC::Canvas c; c.Height(s->height); c.HorizontalAlignment(WUX::HorizontalAlignment::Stretch);
        c.FlowDirection(WUX::FlowDirection::LeftToRight);
        found=s->cells.emplace(token,c).first;
    }
    WUXA::AutomationProperties::SetName(found->second,hs(text));
    n.Content(found->second); s->nodes.emplace(token,n);
    if(header) s->headers.insert(token);
    auto p=s->nodes.find(parent);
    if(hasParent && p!=s->nodes.end()) { p->second.Children().Append(n); p->second.HasUnrealizedChildren(false); }
    else s->view.RootNodes().Append(n);
}); }
void day_xaml_tree_end(void *w) { guard([&] { auto s=day_native_trees.at(w); s->suppress=false; s->viewport(w); }); }
void *day_xaml_tree_cell(void *w, uint64_t token) {
    void *result=nullptr; guard([&] { result=boxh(day_native_trees.at(w)->cells.at(token)); }); return result;
}
int day_xaml_tree_frame(void *w, uint64_t token, double *width) {
    int visible=0; guard([&] {
        auto s=day_native_trees.at(w); auto found=s->nodes.find(token); if(found==s->nodes.end()) return;
        auto n=found->second; int depth=1;
        for(auto p=n.Parent(); p; p=p.Parent()) { if(!p.IsExpanded()) return; ++depth; }
        // The peer otherwise names the TreeViewNode object even with a visual template.
        if (auto item = s->view.ContainerFromNode(n).try_as<WUXC::TreeViewItem>()) {
            WUXA::AutomationProperties::SetName(item,
                WUXA::AutomationProperties::GetName(s->cells.at(token)));
            if (auto host = item.ContentTemplateRoot().try_as<WUXC::Panel>()) {
                auto cell = s->cells.at(token);
                auto parent = WUXM::VisualTreeHelper::GetParent(cell);
                if (parent != host) {
                    if (auto old = parent.try_as<WUXC::Panel>()) {
                        uint32_t index = 0;
                        if (old.Children().IndexOf(cell, index)) old.Children().RemoveAt(index);
                    }
                    host.Children().Clear();
                    host.Children().Append(cell);
                }
            }
        }
        *width=(std::max)(1.0,s->view.ActualWidth()-24.0*depth-16.0);
        s->cells.at(token).Width(*width); visible=1;
    }); return visible;
}
void day_xaml_tree_expand(void *w, uint64_t token, int open) { guard([&] {
    auto s=day_native_trees.at(w); auto i=s->nodes.find(token); if(i==s->nodes.end()) return;
    s->suppress=true; i->second.IsExpanded(open!=0); s->suppress=false; s->viewport(w);
}); }
void day_xaml_tree_select(void *w, const uint64_t *tokens, int len) { guard([&] {
    auto s=day_native_trees.at(w); s->suppress=true; s->view.SelectedNodes().Clear(); s->selection.clear();
    for(int i=0;i<len;++i) {
        auto n=s->nodes.find(tokens[i]);
        if(n!=s->nodes.end() && !s->headers.count(tokens[i])) {
            s->view.SelectedNodes().Append(n->second); s->selection.push_back(tokens[i]);
        }
    }
    s->suppress=false;
}); }
void day_xaml_tree_reveal(void *w, uint64_t token) { guard([&] {
    auto s=day_native_trees.at(w); auto n=s->nodes.find(token); if(n==s->nodes.end()) return;
    if(auto item=s->view.ContainerFromNode(n->second).try_as<WUX::UIElement>()) item.StartBringIntoView();
}); }
