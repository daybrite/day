// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Included by shim.cpp at file scope. Shared declarations and include order live there.

// ---- the app outside its windows (docs/status-item.md) ------------------------------------
// One hidden window carries everything that must not depend on a Day window being open: the
// notification-area icons' callbacks, the "TaskbarCreated" broadcast (Explorer restarted: every
// icon has to be added again), and Day's posts once the primary window is gone, which is what
// keeps an app with a status item (or `keep_running`) alive and responsive with no window open.
// A tool window rather than a message-only one: message-only windows get no broadcasts, and
// WS_EX_TOOLWINDOW keeps it off the taskbar should Explorer ever decide to show it.

static const UINT WM_DAY_TRAY = WM_APP + 2;

// One icon in the notification area.
struct TrayItem {
    std::string id;
    UINT uid = 0;
    HICON icon = nullptr;  // owned unless it is the app's icon (`own_icon`)
    bool own_icon = false;
    std::wstring tip;
    std::string menu;      // the menu spec (same lines as the app menu's)
    unsigned long long activate = 0;
    std::string glyph, image; // what `icon` was drawn from, to redraw it only on a change
    int templ = 0;
};
static std::vector<TrayItem> g_tray;
static UINT g_tray_next_uid = 1;
// Day's handler for a tray click or menu choice: (0, dispatch id) or (1, 0) for the Quit role.
static void (*g_status_cb)(int, unsigned long long) = nullptr;

static UINT taskbar_created_msg() {
    static UINT msg = RegisterWindowMessageW(L"TaskbarCreated");
    return msg;
}

// The taskbar's own scheme, which a template glyph is drawn for: white on a dark taskbar, black on
// a light one. That is the SYSTEM setting, not the apps' one the windows follow.
static COLORREF tray_template_color() {
    DWORD light = 0, size = sizeof(light);
    if (RegGetValueW(HKEY_CURRENT_USER,
                     L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
                     L"SystemUsesLightTheme", RRF_RT_REG_DWORD, nullptr, &light, &size)
        != ERROR_SUCCESS)
        light = 0;
    return light ? RGB(0, 0, 0) : RGB(255, 255, 255);
}

// A 32-bit icon from a top-down premultiplied BGRA DIB section (the color bitmap) and an empty
// mask, which a 32-bit icon with alpha does not consult.
static HICON icon_from_dib(HBITMAP dib, int size) {
    std::vector<BYTE> zeros(static_cast<size_t>(((size + 15) / 16) * 2 * size), 0);
    HBITMAP mask = CreateBitmap(size, size, 1, 1, zeros.data());
    ICONINFO ii{};
    ii.fIcon = TRUE;
    ii.hbmMask = mask;
    ii.hbmColor = dib;
    HICON icon = CreateIconIndirect(&ii);
    if (mask) DeleteObject(mask);
    return icon;
}

// An icon-font glyph (the toolbar's Segoe Fluent / MDL2 code points) drawn in `color`: rendered
// white on black with grayscale antialiasing, whose coverage becomes the alpha.
static HICON glyph_icon(wchar_t code, COLORREF color, int size) {
    BITMAPINFO bi{};
    bi.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    bi.bmiHeader.biWidth = size;
    bi.bmiHeader.biHeight = -size; // top-down
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    bi.bmiHeader.biCompression = BI_RGB;
    void* bits = nullptr;
    HDC dc = CreateCompatibleDC(nullptr);
    HBITMAP dib = dc ? CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &bits, nullptr, 0) : nullptr;
    if (!dib || !bits) {
        if (dib) DeleteObject(dib);
        if (dc) DeleteDC(dc);
        return nullptr;
    }
    HGDIOBJ old_bmp = SelectObject(dc, dib);
    std::memset(bits, 0, static_cast<size_t>(size) * size * 4);
    HFONT font = CreateFontW(-size, 0, 0, 0, FW_NORMAL, FALSE, FALSE, FALSE, DEFAULT_CHARSET,
                             OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY,
                             DEFAULT_PITCH, toolbar_icon_font().c_str());
    HGDIOBJ old_font = SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, RGB(255, 255, 255));
    RECT rc{ 0, 0, size, size };
    wchar_t text[2] = { code, 0 };
    DrawTextW(dc, text, 1, &rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    GdiFlush();
    auto* px = static_cast<uint32_t*>(bits);
    unsigned r = GetRValue(color), g = GetGValue(color), b = GetBValue(color);
    for (int i = 0; i < size * size; ++i) {
        unsigned a = px[i] & 0xFF; // white text: every channel is the coverage
        px[i] = (a << 24) | ((r * a / 255) << 16) | ((g * a / 255) << 8) | (b * a / 255);
    }
    SelectObject(dc, old_font);
    if (font) DeleteObject(font);
    SelectObject(dc, old_bmp);
    HICON icon = icon_from_dib(dib, size);
    DeleteObject(dib);
    DeleteDC(dc);
    return icon;
}

// A bundled image file (PNG, ICO, …) scaled to the icon size; a template keeps only its alpha and
// takes the taskbar's color.
static HICON image_icon(const std::wstring& path, bool templ, COLORREF color, int size) {
    HICON icon = nullptr;
    ULONG_PTR token = 0;
    Gdiplus::GdiplusStartupInput si;
    if (Gdiplus::GdiplusStartup(&token, &si, nullptr) != Gdiplus::Ok) return nullptr;
    {
        Gdiplus::Bitmap src(path.c_str());
        if (src.GetLastStatus() == Gdiplus::Ok) {
            Gdiplus::Bitmap bmp(size, size, PixelFormat32bppARGB);
            {
                Gdiplus::Graphics g(&bmp);
                g.SetInterpolationMode(Gdiplus::InterpolationModeHighQualityBicubic);
                g.SetPixelOffsetMode(Gdiplus::PixelOffsetModeHighQuality);
                g.Clear(Gdiplus::Color(0, 0, 0, 0));
                g.DrawImage(&src, 0, 0, size, size);
            }
            if (templ) {
                Gdiplus::BitmapData data{};
                Gdiplus::Rect all(0, 0, size, size);
                if (bmp.LockBits(&all, Gdiplus::ImageLockModeRead | Gdiplus::ImageLockModeWrite,
                                 PixelFormat32bppARGB, &data) == Gdiplus::Ok) {
                    for (int y = 0; y < size; ++y) {
                        auto* row = reinterpret_cast<uint32_t*>(static_cast<BYTE*>(data.Scan0) +
                                                                static_cast<ptrdiff_t>(y) * data.Stride);
                        for (int x = 0; x < size; ++x) {
                            uint32_t a = row[x] >> 24; // straight (not premultiplied) ARGB
                            row[x] = (a << 24) | (GetRValue(color) << 16) | (GetGValue(color) << 8) |
                                     GetBValue(color);
                        }
                    }
                    bmp.UnlockBits(&data);
                }
            }
            bmp.GetHICON(&icon);
        }
    } // every GDI+ object goes before GdiplusShutdown
    Gdiplus::GdiplusShutdown(token);
    return icon;
}

// (Re)draw an item's icon from its glyph or image, else show the app's icon.
static void tray_draw_icon(TrayItem& t) {
    if (t.own_icon && t.icon) DestroyIcon(t.icon);
    t.icon = nullptr;
    t.own_icon = false;
    int size = GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem());
    if (!t.glyph.empty()) {
        wchar_t code = static_cast<wchar_t>(std::wcstol(
            std::wstring(t.glyph.begin(), t.glyph.end()).c_str(), nullptr, 16));
        // Symbols are always templates (StatusItemSpec::template).
        if (code) t.icon = glyph_icon(code, tray_template_color(), size);
    } else if (!t.image.empty()) {
        t.icon = image_icon(std::wstring(hs(t.image.c_str())), t.templ != 0,
                            tray_template_color(), size);
    }
    t.own_icon = t.icon != nullptr;
    if (!t.icon) t.icon = g_app_icon_small ? g_app_icon_small : LoadIconW(nullptr, IDI_APPLICATION);
}

static HWND shell_window();

// Add (or with `modify` update) the item's icon. NOTIFYICON_VERSION_4 gives the callback the
// event in LOWORD(lParam), the icon in HIWORD and the anchor point in wParam.
static bool tray_notify(TrayItem const& t, bool modify) {
    NOTIFYICONDATAW nid{};
    nid.cbSize = sizeof(nid);
    nid.hWnd = shell_window();
    nid.uID = t.uid;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    nid.uCallbackMessage = WM_DAY_TRAY;
    nid.hIcon = t.icon;
    wcsncpy_s(nid.szTip, t.tip.c_str(), _TRUNCATE); // 127 characters at most
    if (!nid.hWnd || !Shell_NotifyIconW(modify ? NIM_MODIFY : NIM_ADD, &nid)) return false;
    if (!modify) {
        nid.uVersion = NOTIFYICON_VERSION_4;
        Shell_NotifyIconW(NIM_SETVERSION, &nid);
    }
    return true;
}

static void tray_delete(TrayItem const& t) {
    NOTIFYICONDATAW nid{};
    nid.cbSize = sizeof(nid);
    nid.hWnd = g_shell_hwnd;
    nid.uID = t.uid;
    if (nid.hWnd) Shell_NotifyIconW(NIM_DELETE, &nid);
}

// The tray menu as a Win32 popup menu, from the app menu's spec lines: `A` items dispatch their
// id, the Quit role ends the app, the other roles have nothing to act on outside a window and are
// left out. `cmds` maps each command id TrackPopupMenu can return to what it does.
static HMENU build_tray_menu(const std::string& spec,
                             std::map<UINT, std::pair<int, unsigned long long>>& cmds) {
    HMENU root = CreatePopupMenu();
    if (!root) return nullptr;
    std::vector<std::pair<HMENU, std::wstring>> stack; // open submenus and their labels
    UINT next = 1;
    auto menu_text = [](const std::string& s) {
        // `&` marks an access key in a Win32 menu; Day's labels mean a literal ampersand.
        std::wstring w(hs(s.c_str()));
        std::wstring out;
        for (wchar_t c : w) {
            out += c;
            if (c == L'&') out += L'&';
        }
        return out;
    };
    for (auto const& line : split_lines(spec)) {
        if (line.empty()) continue;
        auto f = split_tabs(line);
        std::string kind = f.size() > 0 ? f[0] : "";
        std::string label = f.size() > 6 ? f[6] : "";
        HMENU cur = stack.empty() ? root : stack.back().first;
        if (kind == "-") {
            AppendMenuW(cur, MF_SEPARATOR, 0, nullptr);
        } else if (kind == "S") {
            HMENU sub = CreatePopupMenu();
            if (sub) stack.emplace_back(sub, menu_text(label));
        } else if (kind == "E") {
            if (stack.empty()) continue;
            auto [sub, text] = stack.back();
            stack.pop_back();
            HMENU parent = stack.empty() ? root : stack.back().first;
            AppendMenuW(parent, MF_POPUP, reinterpret_cast<UINT_PTR>(sub), text.c_str());
        } else {
            int role = f.size() > 2 ? std::atoi(f[2].c_str()) : -1;
            std::pair<int, unsigned long long> what{ 0, 0 };
            if (kind == "A") {
                what.second = f.size() > 1 ? std::strtoull(f[1].c_str(), nullptr, 10) : 0;
            } else if (role == 8) {
                what.first = 1; // Quit
            } else {
                continue;
            }
            UINT flags = MF_STRING;
            if (f.size() > 5 && f[5] == "0") flags |= MF_GRAYED;
            if (f.size() > 8 && f[8] == "1") flags |= MF_CHECKED;
            UINT cmd = next++;
            cmds[cmd] = what;
            AppendMenuW(cur, flags, cmd, menu_text(label).c_str());
        }
    }
    // An unterminated submenu still belongs somewhere; DestroyMenu(root) then frees it.
    while (!stack.empty()) {
        auto [sub, text] = stack.back();
        stack.pop_back();
        HMENU parent = stack.empty() ? root : stack.back().first;
        AppendMenuW(parent, MF_POPUP, reinterpret_cast<UINT_PTR>(sub), text.c_str());
    }
    return root;
}

// Open an item's menu at (x, y), the documented way for a notification icon: the owner window
// to the foreground first (or the menu would not close on a click elsewhere), and a WM_NULL
// after, so a second click works.
static void tray_menu(std::string spec, int x, int y) {
    std::map<UINT, std::pair<int, unsigned long long>> cmds;
    HMENU menu = build_tray_menu(spec, cmds);
    if (!menu) return;
    HWND w = shell_window();
    SetForegroundWindow(w);
    UINT flags = TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON |
                 (GetSystemMetrics(SM_MENUDROPALIGNMENT) ? TPM_RIGHTALIGN : TPM_LEFTALIGN);
    UINT cmd = static_cast<UINT>(TrackPopupMenuEx(menu, flags, x, y, w, nullptr));
    PostMessageW(w, WM_NULL, 0, 0);
    DestroyMenu(menu);
    auto it = cmds.find(cmd);
    if (cmd && it != cmds.end() && g_status_cb) g_status_cb(it->second.first, it->second.second);
}

static LRESULT CALLBACK ShellWndProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    if (msg == WM_DAY_POST) {
        auto p = reinterpret_cast<PostMsg*>(lp);
        if (p) { p->cb(p->data); delete p; }
        return 0;
    }
    if (msg == WM_DAY_TRAY) {
        UINT event = LOWORD(lp);
        UINT uid = HIWORD(lp);
        int x = static_cast<short>(LOWORD(wp)), y = static_cast<short>(HIWORD(wp));
        auto it = std::find_if(g_tray.begin(), g_tray.end(),
                               [uid](TrayItem const& t) { return t.uid == uid; });
        if (it == g_tray.end()) return 0;
        // Copies: the handler may replace the item list.
        unsigned long long activate = it->activate;
        std::string spec = it->menu;
        if (event == NIN_SELECT || event == NIN_KEYSELECT) {
            // A primary click: the item's action, or its menu when it has none.
            if (activate) {
                if (g_status_cb) g_status_cb(0, activate);
            } else {
                tray_menu(spec, x, y);
            }
        } else if (event == WM_CONTEXTMENU) {
            tray_menu(spec, x, y);
        }
        return 0;
    }
    if (msg == taskbar_created_msg()) {
        // Explorer restarted and forgot every icon.
        for (auto const& t : g_tray) tray_notify(t, false);
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

static HWND shell_window() {
    if (g_shell_hwnd) return g_shell_hwnd;
    WNDCLASSW wc{};
    wc.lpfnWndProc = ShellWndProc;
    wc.hInstance = GetModuleHandleW(nullptr);
    wc.lpszClassName = L"day_xaml_shell";
    RegisterClassW(&wc);
    HWND h = CreateWindowExW(WS_EX_TOOLWINDOW, L"day_xaml_shell", L"", WS_POPUP, 0, 0, 0, 0,
                             nullptr, nullptr, wc.hInstance, nullptr);
    std::lock_guard<std::mutex> lock(g_post_mutex); // day_xaml_post reads it from other threads
    g_shell_hwnd = h;
    return h;
}

extern "C" void day_xaml_set_status_cb(void (*cb)(int, unsigned long long)) { g_status_cb = cb; }

// Add or update the status item `id`: `glyph` an icon-font code point in hex, else `image` a file
// path, else the app's icon; `templ` draws the image in the taskbar's color; `activate` the
// dispatch id a primary click runs (0: a click opens the menu); `menu` the app menu's spec lines.
extern "C" void day_xaml_status_set(const char* id, const char* tip, const char* glyph,
                                    const char* image, int templ, unsigned long long activate,
                                    const char* menu) try {
    std::string key = id ? id : "";
    auto it = std::find_if(g_tray.begin(), g_tray.end(),
                           [&](TrayItem const& t) { return t.id == key; });
    bool fresh = it == g_tray.end();
    if (fresh) {
        TrayItem t;
        t.id = key;
        t.uid = g_tray_next_uid++;
        g_tray.push_back(t);
        it = g_tray.end() - 1;
    }
    TrayItem& t = *it;
    std::string g = glyph ? glyph : "", img = image ? image : "";
    if (fresh || g != t.glyph || img != t.image || templ != t.templ) {
        t.glyph = g;
        t.image = img;
        t.templ = templ;
        tray_draw_icon(t);
    }
    t.tip = std::wstring(hs(tip));
    if (t.tip.size() > 127) t.tip.resize(127);
    t.activate = activate;
    t.menu = menu ? menu : "";
    tray_notify(t, !fresh);
} catch (...) {
}

// Remove every item whose id is not in `ids` (newline-joined).
extern "C" void day_xaml_status_retain(const char* ids) try {
    std::set<std::string> keep;
    for (auto const& line : split_lines(std::string(ids ? ids : ""))) {
        if (!line.empty()) keep.insert(line);
    }
    for (auto it = g_tray.begin(); it != g_tray.end();) {
        if (keep.count(it->id)) {
            ++it;
            continue;
        }
        tray_delete(*it);
        if (it->own_icon && it->icon) DestroyIcon(it->icon);
        it = g_tray.erase(it);
    }
} catch (...) {
}

// Keep Day's posts deliverable with no window open (docs/windows.md "Keeping the app running"):
// the shell window takes them once the primary is gone.
extern "C" void day_xaml_set_keep_running(int keep) try {
    if (keep) shell_window();
} catch (...) {
}

// The app's progress on the primary's taskbar button: 0 none, 1 indeterminate, 2 value,
// 3 paused, 4 error; `value` 0..1.
extern "C" void day_xaml_set_app_progress(int kind, double value) try {
    g_progress_kind = kind;
    g_progress_value = value;
    apply_progress();
} catch (...) {
}

// The app's badge as the taskbar button's overlay: 0 none, 1 a count (0 clears), 2 a dot.
extern "C" void day_xaml_set_app_badge(int kind, unsigned count) try {
    g_badge_kind = (kind == 1 && count == 0) ? 0 : kind;
    g_badge_count = count;
    apply_badge();
} catch (...) {
}

// ---- single instance (docs/deep-links.md "Single-instance forwarding") ----------------------
// One process per app id. The first takes a named mutex and a hidden message-only window titled
// with the app id; a second one finds that window, hands it its command line in a WM_COPYDATA, and
// exits before it creates a window of its own. The first then opens what the second was started
// with (`day_core::forward_launch_args`) and comes to the front.
//
// The id is the app's, not the build's: two builds of one app (a `day launch` dev run beside an
// installed copy) are one app here, and whichever started first receives the other's launches.

static const ULONG_PTR kForwardMagic = 0x44415931; // "DAY1": the payload is NUL-separated UTF-16
static HANDLE g_instance_mutex = nullptr;
static HWND g_instance_hwnd = nullptr;
// Day's handler for forwarded launch arguments: UTF-8, NUL-separated, `len` bytes in all.
static void (*g_forward_cb)(const char*, int) = nullptr;

static LRESULT CALLBACK InstanceWndProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    if (msg == WM_COPYDATA) {
        auto cds = reinterpret_cast<const COPYDATASTRUCT*>(lp);
        if (!cds || cds->dwData != kForwardMagic) return FALSE;
        // The data lives only for this message: turn it into UTF-8 now.
        const wchar_t* w = static_cast<const wchar_t*>(cds->lpData);
        size_t count = cds->lpData ? cds->cbData / sizeof(wchar_t) : 0;
        std::string out;
        size_t start = 0;
        for (size_t i = 0; i <= count; ++i) {
            if (i == count || w[i] == L'\0') {
                if (i > start) out += u8(winrt::hstring(std::wstring_view(w + start, i - start)));
                if (i < count) out.push_back('\0');
                start = i + 1;
            }
        }
        // The sender allowed it (AllowSetForegroundWindow): the first window comes forward now,
        // and day-core focuses the app's front window once the arguments are delivered.
        if (g_app && g_app->host) {
            if (IsIconic(g_app->host)) ShowWindow(g_app->host, SW_RESTORE);
            SetForegroundWindow(g_app->host);
        }
        if (g_forward_cb) g_forward_cb(out.data(), static_cast<int>(out.size()));
        return TRUE;
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

extern "C" void day_xaml_set_forward_cb(void (*cb)(const char*, int)) { g_forward_cb = cb; }

// Whether this process runs unpackaged: a packaged app already has its AppUserModelID.
static bool unpackaged() {
    UINT32 len = 0;
    return GetCurrentPackageFullName(&len, nullptr) == APPMODEL_ERROR_NO_PACKAGE;
}

// Claim the app id: 1 = this is the first instance (or the id is unusable), carry on; 0 = another
// instance has this process's arguments now, and the caller ends the process without opening a
// window. Also gives an unpackaged process the app id as its AppUserModelID, which the taskbar
// groups by and the jump list belongs to; that has to happen before the first window.
extern "C" int day_xaml_claim_instance(const char* app_id) try {
    std::wstring id(hs(app_id));
    if (id.empty()) return 1;
    std::wstring name = L"Local\\day.instance.";
    for (wchar_t c : id) name += (c == L'\\') ? L'_' : c; // a backslash would name a namespace
    HANDLE m = CreateMutexW(nullptr, TRUE, name.c_str());
    if (m && GetLastError() == ERROR_ALREADY_EXISTS) {
        CloseHandle(m);
        // The first instance may still be starting: give it a few seconds to put its window up.
        HWND target = nullptr;
        for (int i = 0; i < 50 && !target; ++i) {
            target = FindWindowExW(HWND_MESSAGE, nullptr, L"day_xaml_instance", id.c_str());
            if (!target) Sleep(100);
        }
        if (target) {
            int argc = 0;
            LPWSTR* argv = CommandLineToArgvW(GetCommandLineW(), &argc);
            std::wstring payload;
            for (int i = 1; argv && i < argc; ++i) {
                if (i > 1) payload.push_back(L'\0');
                payload += argv[i];
            }
            if (argv) LocalFree(argv);
            COPYDATASTRUCT cds{};
            cds.dwData = kForwardMagic;
            cds.cbData = static_cast<DWORD>(payload.size() * sizeof(wchar_t));
            cds.lpData = payload.empty() ? nullptr : const_cast<wchar_t*>(payload.data());
            AllowSetForegroundWindow(ASFW_ANY);
            DWORD_PTR result = 0;
            if (SendMessageTimeoutW(target, WM_COPYDATA, 0, reinterpret_cast<LPARAM>(&cds),
                                    SMTO_ABORTIFHUNG, 5000, &result) &&
                result)
                return 0;
        }
        // No answer: the first instance is hung or going away. Run rather than lose the launch.
        std::fprintf(stderr, "day-xaml: another instance holds the app id but did not answer\n");
        return 1;
    }
    g_instance_mutex = m; // held for the life of the process
    if (unpackaged()) SetCurrentProcessExplicitAppUserModelID(id.c_str());
    WNDCLASSW wc{};
    wc.lpfnWndProc = InstanceWndProc;
    wc.hInstance = GetModuleHandleW(nullptr);
    wc.lpszClassName = L"day_xaml_instance";
    RegisterClassW(&wc);
    g_instance_hwnd = CreateWindowExW(0, L"day_xaml_instance", id.c_str(), 0, 0, 0, 0, 0,
                                      HWND_MESSAGE, nullptr, wc.hInstance, nullptr);
    return 1;
} catch (...) {
    return 1;
}

// ---- jump list (docs/deep-links.md "Launcher shortcuts") ------------------------------------
// The taskbar button's and Start entry's Tasks: one shell link per shortcut, each starting this
// exe with `--day-open-url <link>`. A second process started that way forwards the link to the
// running one (above), so a task opens its route in the app already on screen.

// PKEY_Title, spelled out rather than taken from propkey.h, which defines it only for INITGUID.
static const PROPERTYKEY kTitleKey = {
    { 0xF29F85E0, 0x4FF9, 0x1068, { 0xAB, 0x91, 0x08, 0x00, 0x2B, 0x27, 0xB3, 0xD9 } }, 2 };

// One task: this exe with `args`, titled `title`.
static winrt::com_ptr<IShellLinkW> jump_task(const std::wstring& exe, const std::wstring& args,
                                             const std::wstring& title) {
    winrt::com_ptr<IShellLinkW> link;
    if (FAILED(CoCreateInstance(__uuidof(ShellLink), nullptr, CLSCTX_INPROC_SERVER,
                                __uuidof(IShellLinkW), link.put_void())))
        return nullptr;
    link->SetPath(exe.c_str());
    link->SetArguments(args.c_str());
    link->SetIconLocation(exe.c_str(), 0);
    auto store = link.try_as<IPropertyStore>();
    if (!store) return nullptr;
    PROPVARIANT pv;
    PropVariantInit(&pv);
    size_t bytes = (title.size() + 1) * sizeof(wchar_t);
    pv.vt = VT_LPWSTR;
    pv.pwszVal = static_cast<LPWSTR>(CoTaskMemAlloc(bytes));
    if (!pv.pwszVal) return nullptr;
    std::memcpy(pv.pwszVal, title.c_str(), bytes);
    HRESULT hr = store->SetValue(kTitleKey, pv);
    PropVariantClear(&pv);
    if (FAILED(hr) || FAILED(store->Commit())) return nullptr;
    return link;
}

// Replace the Tasks with `spec`: one "label \t link" line per shortcut. Empty clears the list.
extern "C" void day_xaml_set_jump_tasks(const char* app_id, const char* spec) try {
    winrt::com_ptr<ICustomDestinationList> list;
    if (FAILED(CoCreateInstance(__uuidof(DestinationList), nullptr, CLSCTX_INPROC_SERVER,
                                __uuidof(ICustomDestinationList), list.put_void())))
        return;
    std::wstring id(hs(app_id));
    // The list belongs to the AppUserModelID set in day_xaml_claim_instance; a packaged app's
    // own is the default.
    if (!id.empty() && unpackaged()) list->SetAppID(id.c_str());
    std::vector<std::pair<std::wstring, std::wstring>> tasks; // label, link
    for (auto const& line : split_lines(std::string(spec ? spec : ""))) {
        auto f = split_tabs(line);
        if (f.size() < 2 || f[0].empty() || f[1].empty()) continue;
        tasks.emplace_back(std::wstring(hs(f[0].c_str())), std::wstring(hs(f[1].c_str())));
    }
    if (tasks.empty()) {
        list->DeleteList(id.empty() || !unpackaged() ? nullptr : id.c_str());
        return;
    }
    wchar_t exe[MAX_PATH]{};
    DWORD n = GetModuleFileNameW(nullptr, exe, MAX_PATH);
    if (n == 0 || n >= MAX_PATH) return;
    UINT max_slots = 0;
    winrt::com_ptr<IObjectArray> removed;
    if (FAILED(list->BeginList(&max_slots, __uuidof(IObjectArray), removed.put_void()))) return;
    winrt::com_ptr<IObjectCollection> items;
    if (FAILED(CoCreateInstance(__uuidof(EnumerableObjectCollection), nullptr,
                                CLSCTX_INPROC_SERVER, __uuidof(IObjectCollection),
                                items.put_void()))) {
        list->AbortList();
        return;
    }
    for (auto const& [label, link] : tasks) {
        // The link quoted as one argument; a quote inside it cannot survive the command line.
        std::wstring safe;
        for (wchar_t c : link) safe += c == L'"' ? L'\'' : c;
        auto task = jump_task(exe, L"--day-open-url \"" + safe + L"\"", label);
        if (task) items->AddObject(task.get());
    }
    auto array = items.try_as<IObjectArray>();
    if (!array || FAILED(list->AddUserTasks(array.get()))) {
        list->AbortList();
        return;
    }
    list->CommitList();
} catch (...) {
}

