// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Included by shim.cpp at file scope. Shared declarations and include order live there.

// ---------------------------------------------------------------------------
// present / dismiss (docs/dialogs.md)
// ---------------------------------------------------------------------------
// Native modals mirroring day-qt-sys's flat C ABI: a ContentDialog for alert/prompt, WinRT file
// pickers for open/save. Everything is ASYNC (ShowAsync / Pick*Async) so present() returns at once
// — the message loop keeps pumping and dayscript can `respond`/`dismiss` — with a per-`req` registry
// so dismiss() can close/cancel a still-open modal. Results flow back through
// g_present_cb(req, tag, index, text): tag 0 dismissed, 1 button@index, 2 text, 3 files(path) —
// matching day_spec::PresentResult::decode. Each entry is completed by the modal's OWN handler
// (which erases it and fires the cb once), like the Qt shim; dismiss only starts the close.
// present() bodies are try/catch: a WinRT throw must not cross into Rust's run_posted (would abort),
// and on failure we leave the request pending so a scripted `respond` can still resolve it.

static void (*g_present_cb)(uint64_t, int, long long, const char*) = nullptr;

struct DayPresent {
    WUXC::ContentDialog dialog{ nullptr };
    WF::IAsyncInfo op{ nullptr }; // file-picker async op — cancelable by dismiss
};
static std::map<uint64_t, DayPresent> g_presents;

// Split a unit-separator-joined (0x1f) list, dropping empty parts (buttons_joined / filters_joined).
static std::vector<std::string> split_units(const char* joined) {
    std::vector<std::string> out;
    std::string s = joined ? joined : "";
    size_t p = 0;
    while (true) {
        size_t u = s.find('\x1f', p);
        std::string part = s.substr(p, u == std::string::npos ? std::string::npos : u - p);
        if (!part.empty()) out.push_back(part);
        if (u == std::string::npos) break;
        p = u + 1;
    }
    return out;
}

extern "C" void day_xaml_set_present_cb(void (*cb)(uint64_t, int, long long, const char*)) {
    g_present_cb = cb;
}

// A settings action can originate in a secondary island. Attaching its ContentDialog to
// the primary XamlRoot leaves the modal behind the owned preferences window.
static WUX::XamlRoot presentation_root(AppWindow* app) {
    auto active = GetAncestor(GetActiveWindow(), GA_ROOT);
    auto secondary = g_sec_windows.find(active);
    if (secondary != g_sec_windows.end() && secondary->second && secondary->second->root)
        return secondary->second->root.XamlRoot();
    return app->root.XamlRoot();
}

extern "C" void day_xaml_present_dialog(uint64_t req, const char* title, const char* message,
                                         const char* buttons_joined, const char* roles_joined,
                                         void* win) try {
    (void)roles_joined; // ContentDialog styles by slot, not per-button role
    auto app = reinterpret_cast<AppWindow*>(win);
    if (!app || !app->root) { if (g_present_cb) g_present_cb(req, 0, 0, ""); return; }
    WUXC::ContentDialog dlg;
    dlg.XamlRoot(presentation_root(app));
    if (title && *title) dlg.Title(winrt::box_value(hs(title)));
    if (message && *message) dlg.Content(winrt::box_value(hs(message)));
    // Map buttons in spec order to ContentDialog's Primary / Secondary / Close slots; the async
    // result maps back to the original index. Windows dialogs are conventionally <=3 buttons; any
    // beyond the third are dropped (the showcase's alert/confirm/delete/sheet all fit).
    auto labels = split_units(buttons_joined);
    int nbtns = static_cast<int>(labels.size());
    if (nbtns > 0) dlg.PrimaryButtonText(hs(labels[0].c_str()));
    if (nbtns > 1) dlg.SecondaryButtonText(hs(labels[1].c_str()));
    if (nbtns > 2) dlg.CloseButtonText(hs(labels[2].c_str()));

    g_presents[req].dialog = dlg;
    dlg.ShowAsync().Completed(
        [req, nbtns](WF::IAsyncOperation<WUXC::ContentDialogResult> const& a, WF::AsyncStatus st) {
            if (!g_presents.erase(req)) return; // already resolved (scripted respond) → drop
            if (!g_present_cb) return;
            auto res = (st == WF::AsyncStatus::Completed) ? a.GetResults()
                                                          : WUXC::ContentDialogResult::None;
            if (res == WUXC::ContentDialogResult::Primary && nbtns > 0) g_present_cb(req, 1, 0, "");
            else if (res == WUXC::ContentDialogResult::Secondary && nbtns > 1) g_present_cb(req, 1, 1, "");
            else if (res == WUXC::ContentDialogResult::None && nbtns > 2) g_present_cb(req, 1, 2, "");
            else g_present_cb(req, 0, 0, "");
        });
} catch (...) {
}

extern "C" void day_xaml_present_prompt(uint64_t req, const char* title, const char* message,
                                         const char* placeholder, const char* initial,
                                         const char* ok, const char* cancel, void* win) try {
    auto app = reinterpret_cast<AppWindow*>(win);
    if (!app || !app->root) { if (g_present_cb) g_present_cb(req, 0, 0, ""); return; }
    WUXC::ContentDialog dlg;
    dlg.XamlRoot(presentation_root(app));
    if (title && *title) dlg.Title(winrt::box_value(hs(title)));
    WUXC::StackPanel panel;
    if (message && *message) {
        WUXC::TextBlock tb;
        tb.Text(hs(message));
        tb.Margin(WUX::Thickness{ 0, 0, 0, 8 });
        panel.Children().Append(tb);
    }
    WUXC::TextBox box;
    box.PlaceholderText(hs(placeholder));
    box.Text(hs(initial));
    panel.Children().Append(box);
    dlg.Content(panel);
    dlg.PrimaryButtonText(hs((ok && *ok) ? ok : "OK"));
    dlg.CloseButtonText(hs((cancel && *cancel) ? cancel : "Cancel"));

    g_presents[req].dialog = dlg;
    dlg.ShowAsync().Completed(
        [req, box](WF::IAsyncOperation<WUXC::ContentDialogResult> const& a, WF::AsyncStatus st) {
            if (!g_presents.erase(req)) return;
            if (!g_present_cb) return;
            if (st == WF::AsyncStatus::Completed &&
                a.GetResults() == WUXC::ContentDialogResult::Primary) {
                std::string txt = u8(box.Text());
                g_present_cb(req, 2, 0, txt.c_str());
            } else {
                g_present_cb(req, 0, 0, "");
            }
        });
} catch (...) {
}

// Resolve only registered requests: cancellation/scripted dismissal and late completion must
// never answer twice. Resolve the path BEFORE removing the request, since Path() can throw.
static void finish_file(uint64_t req, std::string const& path = {}) {
    if (!g_presents.erase(req)) return;
    if (g_present_cb) g_present_cb(req, path.empty() ? 0 : 3, 0, path.c_str());
}

static void file_completed(uint64_t req,
                           WF::IAsyncOperation<WSt::StorageFile> const& op,
                           WF::AsyncStatus status) noexcept {
    std::string path;
    try {
        if (status == WF::AsyncStatus::Completed) {
            if (auto file = op.GetResults()) path = u8(file.Path());
        }
    } catch (...) {
        // Provider/GetResults/Path errors have the same public result as cancellation.
        // In particular, never let a WinRT exception escape the completion delegate.
    }
    finish_file(req, path);
}

// A multiple-selection pick: every chosen path, joined by the unit separator (the tag-3 wire
// form `PresentResult::decode` splits). An empty selection is a cancel.
static void files_completed(uint64_t req,
                            WF::IAsyncOperation<WFC::IVectorView<WSt::StorageFile>> const& op,
                            WF::AsyncStatus status) noexcept {
    std::string joined;
    try {
        if (status == WF::AsyncStatus::Completed) {
            if (auto files = op.GetResults()) {
                for (auto const& file : files) {
                    if (!file) continue;
                    std::string path = u8(file.Path());
                    if (path.empty()) continue;
                    if (!joined.empty()) joined += '\x1f';
                    joined += path;
                }
            }
        }
    } catch (...) {
        // As above: a provider error reads as a cancel, and never escapes the delegate.
    }
    finish_file(req, joined);
}

static void folder_completed(uint64_t req,
                             WF::IAsyncOperation<WSt::StorageFolder> const& op,
                             WF::AsyncStatus status) noexcept {
    std::string path;
    try {
        if (status == WF::AsyncStatus::Completed) {
            if (auto folder = op.GetResults()) path = u8(folder.Path());
        }
    } catch (...) {
    }
    finish_file(req, path);
}

extern "C" void day_xaml_present_file_open(uint64_t req, const char* title,
                                            const char* filters_joined, int multiple,
                                            void* win) try {
    (void)title; // FileOpenPicker has no title property in the WinRT API
    // Register before any WinRT activation/filter/owner setup that can fail.
    g_presents.try_emplace(req);
    auto app = reinterpret_cast<AppWindow*>(win);
    if (!app || !app->host) { finish_file(req); return; }
    WStP::FileOpenPicker picker;
    picker.SuggestedStartLocation(WStP::PickerLocationId::DocumentsLibrary);
    // FileOpenPicker requires >=1 FileTypeFilter or PickSingleFileAsync throws. Flatten day's named
    // filters ("Name|ext1,ext2") to a bare ".ext" list; no filters → "*" (all files).
    bool any = false;
    for (auto const& f : split_units(filters_joined)) {
        size_t bar = f.find('|');
        std::string exts = (bar == std::string::npos) ? "" : f.substr(bar + 1);
        size_t p = 0;
        while (p <= exts.size()) {
            size_t c = exts.find(',', p);
            std::string e = exts.substr(p, c == std::string::npos ? std::string::npos : c - p);
            if (!e.empty()) { picker.FileTypeFilter().Append(hs(("." + e).c_str())); any = true; }
            if (c == std::string::npos) break;
            p = c + 1;
        }
    }
    if (!any) picker.FileTypeFilter().Append(L"*");
    winrt::check_hresult(picker.as<::IInitializeWithWindow>()->Initialize(app->host));

    if (multiple) {
        auto op = picker.PickMultipleFilesAsync();
        g_presents[req].op = op;
        op.Completed([req](WF::IAsyncOperation<WFC::IVectorView<WSt::StorageFile>> const& a,
                           WF::AsyncStatus st) { files_completed(req, a, st); });
        return;
    }
    auto op = picker.PickSingleFileAsync();
    g_presents[req].op = op;
    op.Completed([req](WF::IAsyncOperation<WSt::StorageFile> const& a, WF::AsyncStatus st) {
        file_completed(req, a, st);
    });
} catch (...) {
    finish_file(req);
}

// The folder chooser: a WinRT FolderPicker parented like the file pickers. It too needs at
// least one FileTypeFilter entry on the desktop or PickSingleFolderAsync throws; "*" is the
// documented placeholder (docs/files.md).
extern "C" void day_xaml_present_folder_open(uint64_t req, const char* title, void* win) try {
    (void)title; // FolderPicker has no title property in the WinRT API
    g_presents.try_emplace(req);
    auto app = reinterpret_cast<AppWindow*>(win);
    if (!app || !app->host) { finish_file(req); return; }
    WStP::FolderPicker picker;
    picker.SuggestedStartLocation(WStP::PickerLocationId::DocumentsLibrary);
    picker.FileTypeFilter().Append(L"*");
    winrt::check_hresult(picker.as<::IInitializeWithWindow>()->Initialize(app->host));

    auto op = picker.PickSingleFolderAsync();
    g_presents[req].op = op;
    op.Completed([req](WF::IAsyncOperation<WSt::StorageFolder> const& a, WF::AsyncStatus st) {
        folder_completed(req, a, st);
    });
} catch (...) {
    finish_file(req);
}

extern "C" void day_xaml_present_file_save(uint64_t req, const char* title, const char* suggested,
                                            const char* filters_joined, void* win) try {
    (void)title; // FileSavePicker has no title property in the WinRT API
    // Register before any WinRT activation/filter/owner setup that can fail.
    g_presents.try_emplace(req);
    auto app = reinterpret_cast<AppWindow*>(win);
    if (!app || !app->host) { finish_file(req); return; }
    WStP::FileSavePicker picker;
    picker.SuggestedStartLocation(WStP::PickerLocationId::DocumentsLibrary);
    // FileSavePicker requires >=1 FileTypeChoice (name → [".ext", ...]); no filters → "Any" → ".".
    bool any = false;
    for (auto const& f : split_units(filters_joined)) {
        size_t bar = f.find('|');
        std::string name = (bar == std::string::npos) ? f : f.substr(0, bar);
        std::string exts = (bar == std::string::npos) ? "" : f.substr(bar + 1);
        auto vec = winrt::single_threaded_vector<winrt::hstring>();
        size_t p = 0;
        while (p <= exts.size()) {
            size_t c = exts.find(',', p);
            std::string e = exts.substr(p, c == std::string::npos ? std::string::npos : c - p);
            if (!e.empty()) vec.Append(hs(("." + e).c_str()));
            if (c == std::string::npos) break;
            p = c + 1;
        }
        if (vec.Size() > 0) { picker.FileTypeChoices().Insert(hs(name.c_str()), vec); any = true; }
    }
    if (!any) {
        picker.FileTypeChoices().Insert(
            L"Any", winrt::single_threaded_vector<winrt::hstring>({ L"." }));
    }
    if (suggested && *suggested) picker.SuggestedFileName(hs(suggested));
    winrt::check_hresult(picker.as<::IInitializeWithWindow>()->Initialize(app->host));

    auto op = picker.PickSaveFileAsync();
    g_presents[req].op = op;
    op.Completed([req](WF::IAsyncOperation<WSt::StorageFile> const& a, WF::AsyncStatus st) {
        file_completed(req, a, st);
    });
} catch (...) {
    finish_file(req);
}

extern "C" void day_xaml_dismiss_present(uint64_t req) try {
    auto it = g_presents.find(req);
    if (it == g_presents.end()) return;
    // Start the close; the modal's own Completed handler erases the entry and fires the cb (which
    // day-core ignores, having already resolved). Matches the Qt shim's dismiss.
    if (it->second.dialog) it->second.dialog.Hide();
    else if (it->second.op) it->second.op.Cancel();
} catch (...) {
}

