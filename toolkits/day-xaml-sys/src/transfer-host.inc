// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//
// Cross-process drops: the HOST window is the OLE drop target (docs/drag-and-drop.md).
//
// transfer.inc routes drags that BEGIN inside the island. A drag from Explorer never reaches it:
// `DesktopWindowXamlSource` does not register its HWND with OLE, so `RegisterDragDrop` is never
// called for any window in the process — probe a running app for the `OleDropTargetInterface`
// window property and every window answers no, including the island. `AllowDrop(true)` still
// succeeds, which is why this failed silently: the drop zones simply never saw an external drag.
//
// So the host HWND registers itself and forwards. OLE finds a target by walking up from the window
// under the cursor, and the island is an unregistered child of the host, so a drop over the island
// arrives here. Each event is hit-tested against the XAML tree for the day drop target under the
// pointer and handed to the SAME `accept`/`receive` callbacks the in-app path uses, so a file from
// Explorer and a tile dragged within the page reach the app by one route.
//
// Included from transfer.inc, at file scope.

#include <shellapi.h> // DragQueryFileW — CF_HDROP is how Explorer offers files

namespace day_transfer {

constexpr auto bundle_utf8 = "application/vnd.day.transfer";

static UINT bundle_format() {
    static UINT cf = RegisterClipboardFormatW(bundle);
    return cf;
}

/// One clipboard format's bytes, or false when the source cannot render it as global memory.
static bool medium_bytes(IDataObject *obj, UINT cf, Bytes &out) {
    FORMATETC fe{static_cast<CLIPFORMAT>(cf), nullptr, DVASPECT_CONTENT, -1, TYMED_HGLOBAL};
    STGMEDIUM sm{};
    if (FAILED(obj->GetData(&fe, &sm))) return false;
    bool ok = false;
    if (sm.tymed == TYMED_HGLOBAL && sm.hGlobal) {
        if (auto p = static_cast<const uint8_t *>(GlobalLock(sm.hGlobal))) {
            auto n = GlobalSize(sm.hGlobal);
            if (n <= limit) {
                out.assign(p, p + n);
                ok = true;
            }
            GlobalUnlock(sm.hGlobal);
        }
    }
    ReleaseStgMedium(&sm);
    return ok;
}

/// Percent-escape a path into the `file:///C:/…` spelling day carries. day-spec's `file_paths`
/// drops the slash before a drive letter, so this matches what the in-app path already emits and a
/// page cannot tell an Explorer file from one dragged out of another Day window.
static std::string file_uri(std::string path) {
    std::replace(path.begin(), path.end(), '\\', '/');
    std::string escaped;
    constexpr char hex[] = "0123456789ABCDEF";
    for (unsigned char c : path) {
        if (isalnum(c) || c == '/' || c == ':' || c == '-' || c == '_' || c == '.' || c == '~')
            escaped += static_cast<char>(c);
        else {
            escaped += '%';
            escaped += hex[c >> 4];
            escaped += hex[c & 15];
        }
    }
    return "file:///" + escaped + "\r\n";
}

/// CF_HDROP as a `text/uri-list` representation.
static bool hdrop_uris(IDataObject *obj, Bytes &out) {
    FORMATETC fe{CF_HDROP, nullptr, DVASPECT_CONTENT, -1, TYMED_HGLOBAL};
    STGMEDIUM sm{};
    if (FAILED(obj->GetData(&fe, &sm))) return false;
    std::string uris;
    if (sm.hGlobal) {
        if (auto drop = static_cast<HDROP>(GlobalLock(sm.hGlobal))) {
            UINT n = DragQueryFileW(drop, 0xFFFFFFFFu, nullptr, 0);
            for (UINT i = 0; i < n && i < 32; i++) {
                UINT len = DragQueryFileW(drop, i, nullptr, 0);
                if (!len) continue;
                std::wstring w(static_cast<size_t>(len) + 1, L'\0');
                DragQueryFileW(drop, i, w.data(), len + 1);
                w.resize(len);
                uris += file_uri(winrt::to_string(winrt::hstring(w)));
            }
            GlobalUnlock(sm.hGlobal);
        }
    }
    ReleaseStgMedium(&sm);
    if (uris.empty()) return false;
    out.assign(uris.begin(), uris.end());
    return true;
}

/// Every MIME-shaped format the source offers, newline separated — what `accept` is given.
///
/// Win32 names its own formats without a slash (`FileDrop`, `Shell IDList Array`), so the two that
/// carry meaning here are translated by hand and everything else passes through as the MIME type it
/// already is. That is what lets the Showcase page's `has("text/uri-list")` match an Explorer file.
static std::string external_types(IDataObject *obj) {
    std::string out;
    FORMATETC fe{CF_HDROP, nullptr, DVASPECT_CONTENT, -1, TYMED_HGLOBAL};
    if (obj->QueryGetData(&fe) == S_OK) out += "text/uri-list\n";
    fe.cfFormat = CF_UNICODETEXT;
    if (obj->QueryGetData(&fe) == S_OK) out += "text/plain\n";
    IEnumFORMATETC *en = nullptr;
    if (SUCCEEDED(obj->EnumFormatEtc(DATADIR_GET, &en)) && en) {
        FORMATETC f{};
        ULONG got = 0;
        while (en->Next(1, &f, &got) == S_OK && got == 1) {
            wchar_t name[256]{};
            if (GetClipboardFormatNameW(f.cfFormat, name, 255) > 0) {
                auto mime = winrt::to_string(winrt::hstring(name));
                if (mime.find('/') != std::string::npos && mime.size() <= 255) out += mime + "\n";
            }
            if (f.ptd) CoTaskMemFree(f.ptd);
        }
        en->Release();
    }
    return out;
}

/// The drag's contents as a day packet. A Day source in another process already published the whole
/// packet under `bundle`, so that is taken verbatim; anything else is assembled from the formats
/// Windows does offer.
static Bytes external_payload(IDataObject *obj) {
    Bytes direct;
    if (medium_bytes(obj, bundle_format(), direct) && direct.size() >= 8) return direct;
    std::vector<Rep> reps;
    Bytes uris;
    if (hdrop_uris(obj, uris)) reps.push_back({"text/uri-list", std::move(uris)});
    FORMATETC fe{CF_UNICODETEXT, nullptr, DVASPECT_CONTENT, -1, TYMED_HGLOBAL};
    STGMEDIUM sm{};
    if (SUCCEEDED(obj->GetData(&fe, &sm))) {
        if (sm.hGlobal) {
            if (auto p = static_cast<const wchar_t *>(GlobalLock(sm.hGlobal))) {
                auto utf8 = winrt::to_string(winrt::hstring(p));
                reps.push_back({"text/plain", Bytes(utf8.begin(), utf8.end())});
                GlobalUnlock(sm.hGlobal);
            }
        }
        ReleaseStgMedium(&sm);
    }
    IEnumFORMATETC *en = nullptr;
    if (SUCCEEDED(obj->EnumFormatEtc(DATADIR_GET, &en)) && en) {
        FORMATETC f{};
        ULONG got = 0;
        size_t total = 0;
        while (reps.size() < 32 && en->Next(1, &f, &got) == S_OK && got == 1) {
            wchar_t name[256]{};
            if (GetClipboardFormatNameW(f.cfFormat, name, 255) > 0) {
                auto mime = winrt::to_string(winrt::hstring(name));
                bool dup = std::any_of(reps.begin(), reps.end(),
                                       [&](Rep const &r) { return r.first == mime; });
                if (!dup && mime != bundle_utf8 && mime.find('/') != std::string::npos &&
                    mime.size() <= 255) {
                    Bytes v;
                    if (medium_bytes(obj, f.cfFormat, v) && total + v.size() <= limit) {
                        total += v.size();
                        reps.push_back({mime, std::move(v)});
                    }
                }
            }
            if (f.ptd) CoTaskMemFree(f.ptd);
        }
        en->Release();
    }
    return packet(reps);
}

struct Hit {
    std::shared_ptr<State> state;
    WF::Point local{};
};

/// The day drop target under a SCREEN point, with the point in that element's own coordinates.
static bool hit_test(AppWindow *app, POINTL pt, Hit &out) {
    if (!app || !app->host || !app->root) return false;
    POINT p{pt.x, pt.y};
    if (!ScreenToClient(app->host, &p)) return false;
    // DIPs == px: this backend assumes a 100% scale throughout (window creation, resize reports).
    WF::Point at{static_cast<float>(p.x), static_cast<float>(p.y)};
    auto hits = WUXM::VisualTreeHelper::FindElementsInHostCoordinates(at, app->root);
    for (auto const &el : hits) {
        for (auto const &entry : states) {
            auto const &s = entry.second;
            if (!s || !s->alive || !s->accept) continue;
            if (elem(entry.first) == el) {
                out.state = s;
                out.local = app->root.TransformToVisual(el).TransformPoint(at);
                return true;
            }
        }
    }
    return false;
}

struct HostDrop : winrt::implements<HostDrop, IDropTarget> {
    AppWindow *app;
    winrt::com_ptr<IDataObject> data;
    explicit HostDrop(AppWindow *a) : app(a) {}

    HRESULT __stdcall DragEnter(IDataObject *obj, DWORD, POINTL pt, DWORD *effect) noexcept {
        data = nullptr;
        data.copy_from(obj);
        return over(pt, effect);
    }
    HRESULT __stdcall DragOver(DWORD, POINTL pt, DWORD *effect) noexcept { return over(pt, effect); }
    HRESULT __stdcall DragLeave() noexcept {
        data = nullptr;
        return S_OK;
    }
    HRESULT __stdcall Drop(IDataObject *obj, DWORD, POINTL pt, DWORD *effect) noexcept {
        if (effect) *effect = DROPEFFECT_NONE;
        try {
            Hit hit;
            if (obj && hit_test(app, pt, hit) && hit.state->receive) {
                auto payload = external_payload(obj);
                // `local` is false by construction: another process owns this drag, so the app
                // treats it as a copy rather than a move of an object it already holds.
                if (!payload.empty() && payload.size() <= limit &&
                    hit.state->receive(hit.state->handle, hit.local.X, hit.local.Y, payload.data(),
                                       payload.size(), false) &&
                    effect)
                    *effect = DROPEFFECT_COPY;
            }
        } catch (...) {
        }
        data = nullptr;
        return S_OK;
    }

  private:
    HRESULT over(POINTL pt, DWORD *effect) noexcept {
        if (effect) *effect = DROPEFFECT_NONE;
        try {
            Hit hit;
            if (!data || !hit_test(app, pt, hit)) return S_OK;
            auto types = external_types(data.get());
            if (hit.state->accept(hit.state->handle, hit.local.X, hit.local.Y, types.c_str(),
                                  false) &&
                effect)
                *effect = DROPEFFECT_COPY;
        } catch (...) {
        }
        return S_OK;
    }
};

/// Registered once, the first time the app declares any drop target: an app that never takes a drop
/// should not advertise itself to the desktop as somewhere to drop things.
static void ensure_host_drop() {
    static bool done = false;
    if (done || !g_app || !g_app->host) return;
    done = true;
    static winrt::com_ptr<IDropTarget> keep; // outlives the call; RegisterDragDrop holds its own
    keep = winrt::make<HostDrop>(g_app);
    RegisterDragDrop(g_app->host, keep.get());
}

} // namespace day_transfer
