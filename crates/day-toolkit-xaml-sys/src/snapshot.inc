// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Included by shim.cpp inside the C ABI block. Shared declarations and include order live there.

// ---- snapshot (PrintWindow → Gdiplus PNG) ----

static int png_encoder_clsid(CLSID* clsid) {
    UINT num = 0, size = 0;
    Gdiplus::GetImageEncodersSize(&num, &size);
    if (size == 0) return -1;
    auto info = reinterpret_cast<Gdiplus::ImageCodecInfo*>(malloc(size));
    if (!info) return -1;
    Gdiplus::GetImageEncoders(num, size, info);
    int result = -1;
    for (UINT i = 0; i < num; ++i) {
        if (wcscmp(info[i].MimeType, L"image/png") == 0) {
            *clsid = info[i].Clsid;
            result = static_cast<int>(i);
            break;
        }
    }
    free(info);
    return result;
}

/// Block until XAML has laid the tree out AND the compositor has PRESENTED it (bounded).
///
/// Without this a capture is routinely one frame stale: it shows the page you just navigated
/// away from, or the scroll offset you just left. `wait_idle` settles day's reactive graph
/// (`day_reactive::flush_sync`) and returns, but the XAML mutations that flush produced are
/// still queued as layout work, and DirectComposition presents asynchronously after that.
/// `PrintWindow` reads the window's last COMPOSED surface, so it hands back the previous frame.
/// Nothing catches it: every assertion reads the logical tree, which is already correct, so only
/// the pixels are wrong — and those pixels are what the published gallery ships.
///
/// `CompositionTarget::Rendering` fires once per frame the XAML compositor drives, so a tick
/// proves layout ran; `DwmFlush` then waits for DWM to finish the present that tick scheduled.
/// TWO ticks, because the first one to arrive can be a frame already in flight when we get here,
/// which would prove nothing about our own mutations.
///
/// Bounded and best-effort: a window that is never composed (minimized, or a headless session
/// where DWM does not tick) must not hang the capture, so this falls through on a timeout and
/// lets the snapshot proceed with whatever the surface holds.
static long long qpc_time_100ns() {
    LARGE_INTEGER now{}, frequency{};
    QueryPerformanceCounter(&now);
    QueryPerformanceFrequency(&frequency);
    return frequency.QuadPart > 0
        ? static_cast<long long>((static_cast<long double>(now.QuadPart) * 10000000.0L) /
                                 static_cast<long double>(frequency.QuadPart))
        : 0;
}

static long long pump_until_presented() {
    // Direct3D11CaptureFrame::SystemRelativeTime uses this same QPC clock. A WGC session can
    // initially hand out a frame it cached before this barrier; returning the barrier's start
    // lets the capture loop reject that frame without guessing how many milliseconds are enough.
    const long long fresh_after = qpc_time_100ns();
    // Keep the display on, for the rest of the run. With the monitor off (the power plan's idle
    // timeout; nothing a script does counts as user input), DWM stops composing and WinUI stops
    // presenting, so every capture read a blank window, or a page from before the monitor went
    // off, while every step still passed. ES_DISPLAY_REQUIRED switches a sleeping display back on;
    // ES_CONTINUOUS keeps it on while this (UI) thread lives, which only a process that takes
    // screenshots (a dayscript run, `day drive`) ever asks for.
    static bool display_held = false;
    const bool first = !display_held;
    SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED);
    display_held = true;
    auto seen = std::make_shared<int>(0);
    auto token = WUXM::CompositionTarget::Rendering(
        [seen](WF::IInspectable const&, WF::IInspectable const&) { ++*seen; });
    // A display that is already off (or dimmed) stays that way: the execution state only holds an
    // awake one. What wakes it is input, so send a zero-distance relative mouse move, which
    // Windows counts as user activity without moving the pointer, wait for the display to report
    // itself on, then give composition time to resume and the window to present again. The
    // state comes from the display's power notification (WM_POWERBROADCAST), not from counting
    // frames: XAML keeps raising Rendering while nothing reaches the screen.
    if (g_display_state != 1) {
        INPUT nudge{};
        nudge.type = INPUT_MOUSE;
        nudge.mi.dwFlags = MOUSEEVENTF_MOVE;
        SendInput(1, &nudge, sizeof(nudge));
        pump_until([] { return g_display_state == 1; }, 5000);
        std::fprintf(stderr, "day-toolkit-winui: the display was %s; woke it for the capture\n",
                     g_display_state == 1 ? "off" : "off and did not come back on");
        std::fflush(stderr);
        *seen = 0;
        pump_until([&] { return *seen >= 10; }, 4000);
    } else {
        pump_until([&] { return *seen >= (first ? 10 : 2); }, first ? 4000 : 2000);
    }
    WUXM::CompositionTarget::Rendering(token);
    DwmFlush();
    return fresh_after;
}

// Snapshot via RenderTargetBitmap: renders the XAML visual tree straight to a bitmap,
// independent of whether the host window is visible/foreground/composed (so it works for a
// background-launched app and on headless CI, unlike PrintWindow). Pixels are BGRA8 — which is
// exactly Gdiplus PixelFormat32bppARGB's in-memory byte order. Returns 0 on success.
// One window's REAL pixels, frame included, encoded as a PNG.
//
// Deliberately not `RenderTargetBitmap` (which this used to use). RTB re-renders the XAML tree
// into an offscreen surface, and what comes back is not what the user sees: templated control
// chrome — TextBox and ComboBox borders, the date/time pickers — arrived as flat gray blocks,
// so the CI gallery misreported styling on every page that has an input. It also cannot capture
// anything the compositor draws outside the tree. Reading the window back is the only way a
// screenshot can be evidence of what shipped.
//
// It also makes a per-window capture trivial, which is what `window:` screenshots need: an
// island's HWND is the whole story, so the primary and a secondary differ only in which handle
// they pass.
static int snapshot_hwnd_png(HWND hwnd, const char* path) {
    if (!hwnd || !IsWindow(hwnd)) return 1;
    // Settle here rather than in the dayscript `wait_idle` step: the barrier belongs to the one
    // operation whose correctness depends on it, so an ad-hoc `day drive` screenshot with no
    // preceding `wait_idle` is as trustworthy as a walkthrough capture.
    pump_until_presented();
    RECT r{};
    if (!GetWindowRect(hwnd, &r)) return 1;
    int w = r.right - r.left, h = r.bottom - r.top;
    if (w <= 0 || h <= 0) return 2;

    HDC screen = GetDC(nullptr);
    if (!screen) return 3;
    HDC mem = CreateCompatibleDC(screen);
    HBITMAP bmp = CreateCompatibleBitmap(screen, w, h);
    int rc_out = 4;
    if (mem && bmp) {
        HGDIOBJ old = SelectObject(mem, bmp);
        // PW_RENDERFULLCONTENT: without it a DirectComposition-hosted surface (which every XAML
        // island is) comes back blank. Screen-reading is the fallback for the cases PrintWindow
        // still refuses; it needs the window unobscured, which the walkthrough's own raise gives.
        BOOL ok = PrintWindow(hwnd, mem, 0x00000002 /* PW_RENDERFULLCONTENT */);
        if (!ok) ok = BitBlt(mem, 0, 0, w, h, screen, r.left, r.top, SRCCOPY | CAPTUREBLT);
        if (ok) {
            ULONG_PTR token = 0;
            Gdiplus::GdiplusStartupInput si;
            if (Gdiplus::GdiplusStartup(&token, &si, nullptr) == Gdiplus::Ok) {
                {
                    Gdiplus::Bitmap bitmap(bmp, nullptr);
                    CLSID clsid;
                    if (png_encoder_clsid(&clsid) >= 0) {
                        std::wstring wpath = hs(path).c_str();
                        if (bitmap.Save(wpath.c_str(), &clsid, nullptr) == Gdiplus::Ok) rc_out = 0;
                    }
                } // bitmap destroyed before GdiplusShutdown
                Gdiplus::GdiplusShutdown(token);
            }
        }
        SelectObject(mem, old);
    }
    if (bmp) DeleteObject(bmp);
    if (mem) DeleteDC(mem);
    ReleaseDC(nullptr, screen);
    return rc_out;
}

#ifdef DAY_WINUI
// WinUI 3's capture: the window as DWM composes it, read back through Windows.Graphics.Capture.
//
// GDI cannot see a WinUI window. WinUI 3 presents through its own compositor into a visual
// DWM owns, so PrintWindow (even with PW_RENDERFULLCONTENT) and a screen BitBlt return the
// host's empty client area: solid white. Windows.Graphics.Capture (WGC) is the platform API for
// reading a window's composed output (Windows 10 1903+ for HWND items): it captures the frame,
// the XAML content and whatever the compositor draws outside the XAML tree (WebView2, video),
// and it keeps working when the window is covered by others. It is what OBS, the Snipping Tool
// and Teams window-sharing use.
//
// One frame is enough: start a session on a free-threaded frame pool (it delivers on a worker
// thread, so no dispatcher is needed), pump this thread's messages until the first frame lands,
// copy it to a CPU-readable texture and encode it. The pool's texture can be larger than the
// window; `ContentSize` is the part that is the window.
extern "C++" { // inside the extern "C" block, and these pass C++ types
namespace WGC = winrt::Windows::Graphics::Capture;
namespace WGD = winrt::Windows::Graphics::DirectX;

// The D3D device every capture reuses. Hardware when there is a GPU, WARP otherwise (a VM, a CI
// runner): WGC accepts either, and one frame's copy is cheap on the CPU.
static winrt::com_ptr<ID3D11Device> wgc_d3d_device() {
    static winrt::com_ptr<ID3D11Device> device;
    if (device) return device;
    for (D3D_DRIVER_TYPE type : {D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP}) {
        if (SUCCEEDED(D3D11CreateDevice(nullptr, type, nullptr, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                                        nullptr, 0, D3D11_SDK_VERSION, device.put(), nullptr,
                                        nullptr)))
            break;
    }
    return device;
}

static int snapshot_wgc_png(HWND hwnd, const char* path) {
    if (!hwnd || !IsWindow(hwnd) || IsIconic(hwnd)) return 1;
    if (!WGC::GraphicsCaptureSession::IsSupported()) return 10;
    auto d3d = wgc_d3d_device();
    if (!d3d) return 11;
    winrt::com_ptr<IInspectable> inspectable;
    winrt::check_hresult(CreateDirect3D11DeviceFromDXGIDevice(d3d.as<IDXGIDevice>().get(), inspectable.put()));
    auto device = inspectable.as<WGD::Direct3D11::IDirect3DDevice>();

    auto interop = winrt::get_activation_factory<WGC::GraphicsCaptureItem, IGraphicsCaptureItemInterop>();
    WGC::GraphicsCaptureItem item{nullptr};
    winrt::check_hresult(interop->CreateForWindow(hwnd, winrt::guid_of<WGC::GraphicsCaptureItem>(),
                                                  winrt::put_abi(item)));

    const long long fresh_after = pump_until_presented();
    auto pool = WGC::Direct3D11CaptureFramePool::CreateFreeThreaded(
        device, WGD::DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, item.Size());
    auto session = pool.CreateCaptureSession(item);
    // Both are later additions (1 = 2004, 2 = 20348), so they are set only where the running
    // system has them. The border setting is a request: without the user's consent (a packaged
    // app's graphicsCaptureWithoutBorder prompt) Windows still draws its yellow outline, but that
    // outline is drawn around the window on screen, never into the captured frame.
    try { session.IsCursorCaptureEnabled(false); } catch (...) {}
    try { session.IsBorderRequired(false); } catch (...) {}
    session.StartCapture();

    WGC::Direct3D11CaptureFrame frame{nullptr};
    int stale_frames = 0;
    pump_until([&] {
        frame = pool.TryGetNextFrame();
        if (!frame) return false;
        // Allow 1 ms for conversion rounding between QueryPerformanceCounter ticks and WinRT's
        // 100 ns TimeSpan. Anything older was composed before the layout/present barrier and is
        // exactly the previous-page frame that made walkthrough captures intermittently lie.
        if (frame.SystemRelativeTime().count() + 10000 >= fresh_after) return true;
        ++stale_frames;
        frame.Close();
        frame = nullptr;
        return false;
    }, 10000);
    if (stale_frames > 0) {
        std::fprintf(stderr, "day-toolkit-winui: discarded %d stale capture frame%s\n", stale_frames,
                     stale_frames == 1 ? "" : "s");
        std::fflush(stderr);
    }
    int rc_out = 12;
    if (frame) {
        auto access = frame.Surface().as<::Windows::Graphics::DirectX::Direct3D11::IDirect3DDxgiInterfaceAccess>();
        winrt::com_ptr<ID3D11Texture2D> texture;
        winrt::check_hresult(access->GetInterface(IID_PPV_ARGS(texture.put())));
        D3D11_TEXTURE2D_DESC desc{};
        texture->GetDesc(&desc);
        auto content = frame.ContentSize();
        const UINT w = (std::min)(desc.Width, static_cast<UINT>(content.Width));
        const UINT h = (std::min)(desc.Height, static_cast<UINT>(content.Height));
        rc_out = 2;
        if (w > 0 && h > 0) {
            D3D11_TEXTURE2D_DESC staging_desc = desc;
            staging_desc.Width = w;
            staging_desc.Height = h;
            staging_desc.MipLevels = 1;
            staging_desc.ArraySize = 1;
            staging_desc.Usage = D3D11_USAGE_STAGING;
            staging_desc.BindFlags = 0;
            staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
            staging_desc.MiscFlags = 0;
            winrt::com_ptr<ID3D11Texture2D> staging;
            winrt::check_hresult(d3d->CreateTexture2D(&staging_desc, nullptr, staging.put()));
            winrt::com_ptr<ID3D11DeviceContext> ctx;
            d3d->GetImmediateContext(ctx.put());
            D3D11_BOX box{0, 0, 0, w, h, 1};
            ctx->CopySubresourceRegion(staging.get(), 0, 0, 0, 0, texture.get(), 0, &box);
            D3D11_MAPPED_SUBRESOURCE mapped{};
            winrt::check_hresult(ctx->Map(staging.get(), 0, D3D11_MAP_READ, 0, &mapped));
            rc_out = 4;
            ULONG_PTR token = 0;
            Gdiplus::GdiplusStartupInput si;
            if (Gdiplus::GdiplusStartup(&token, &si, nullptr) == Gdiplus::Ok) {
                {
                    // Premultiplied BGRA8: DWM's surfaces are premultiplied, and only the
                    // rounded corners outside the frame are anything but opaque.
                    Gdiplus::Bitmap bitmap(static_cast<INT>(w), static_cast<INT>(h),
                                           static_cast<INT>(mapped.RowPitch), PixelFormat32bppPARGB,
                                           static_cast<BYTE*>(mapped.pData));
                    CLSID clsid;
                    if (png_encoder_clsid(&clsid) >= 0) {
                        std::wstring wpath = hs(path).c_str();
                        if (bitmap.Save(wpath.c_str(), &clsid, nullptr) == Gdiplus::Ok) rc_out = 0;
                    }
                }
                Gdiplus::GdiplusShutdown(token);
            }
            ctx->Unmap(staging.get(), 0);
        }
        frame.Close();
    }
    session.Close();
    pool.Close();
    if (rc_out == 12) {
        std::fprintf(stderr, "day-toolkit-winui: window capture delivered no frame within 10 s\n");
        std::fflush(stderr);
    }
    return rc_out;
}

// The capture a scripted screenshot takes: the window itself, falling back to the XAML tree
// (snapshot_tree_png) only where Windows.Graphics.Capture is unavailable or fails, which loses
// the frame and anything drawn outside the tree but still shows the content.
static int snapshot_tree_png(WUX::UIElement const& root, HWND host, const char* path);
static int snapshot_winui_png(WUX::UIElement const& root, HWND host, const char* path,
                              double* origin = nullptr) {
    if (origin) origin[0] = origin[1] = 0.0;
    int rc = 13;
    try {
        rc = snapshot_wgc_png(host, path);
    } catch (winrt::hresult_error const& e) {
        std::fprintf(stderr, "day-toolkit-winui: window capture threw hr=0x%08X %ls\n",
                     static_cast<unsigned>(e.code()), e.message().c_str());
    }
    if (rc == 0) {
        // WGC excludes the invisible resize margins but retains the visible border.
        // Ignoring that border shifted narrow pixel assertions one pixel to the left.
        RECT frame{};
        POINT client{0, 0};
        if (origin && SUCCEEDED(DwmGetWindowAttribute(host, DWMWA_EXTENDED_FRAME_BOUNDS,
                                                      &frame, sizeof(frame))) &&
            ClientToScreen(host, &client)) {
            double scale = GetDpiForWindow(host) / 96.0;
            if (scale <= 0.0) scale = 1.0;
            origin[0] = (client.x - frame.left) / scale;
            origin[1] = (client.y - frame.top) / scale;
        }
        return 0;
    }
    std::fprintf(stderr, "day-toolkit-winui: window capture failed (%d); capturing the XAML tree instead\n", rc);
    std::fflush(stderr);
    return snapshot_tree_png(root, host, path);
}

// The fallback capture: the XAML tree rendered by RenderTargetBitmap.
// It sees the content without the compositor, at the price the note on snapshot_hwnd_png gives
// (no window frame, and anything drawn outside the XAML tree is missing). Pixels are
// premultiplied BGRA8, GDI+'s PixelFormat32bppPARGB.
static int snapshot_tree_png(WUX::UIElement const& root, HWND host, const char* path) {
    if (!root) return 1;
    // Said, not just returned: the scripted runner reports any failure as "nothing to capture",
    // which cannot tell a slow render from a refused one.
    auto failed = [](int code, const char* stage, WF::IAsyncInfo const& op) {
        std::fprintf(stderr, "day-toolkit-winui: capture failed at %s (status %d, hr=0x%08X)\n", stage,
                     static_cast<int>(op.Status()),
                     op.Status() == WF::AsyncStatus::Error ? static_cast<unsigned>(op.ErrorCode().value) : 0u);
        std::fflush(stderr);
        return code;
    };
    pump_until_presented();
    WUXM::Imaging::RenderTargetBitmap rtb;
    auto render = rtb.RenderAsync(root);
    // Generous: a scripted run captures at 2x (2560x1600 px), and the first render of a dense
    // page takes seconds on a machine without a GPU.
    pump_until([&] { return render.Status() != WF::AsyncStatus::Started; }, 20000);
    if (render.Status() != WF::AsyncStatus::Completed) return failed(5, "render", render);
    auto pixels = rtb.GetPixelsAsync();
    pump_until([&] { return pixels.Status() != WF::AsyncStatus::Started; }, 20000);
    if (pixels.Status() != WF::AsyncStatus::Completed) return failed(6, "pixels", pixels);
    auto buffer = pixels.GetResults();
    const int stride_w = rtb.PixelWidth();
    int w = stride_w, h = rtb.PixelHeight();
    // Cropped to what the window shows: the root canvas can extend past the island (a child
    // laid out below the client area), and RenderTargetBitmap renders all of it.
    RECT client{};
    if (host && GetClientRect(host, &client)) {
        w = (std::min)(w, static_cast<int>(client.right));
        h = (std::min)(h, static_cast<int>(client.bottom));
    }
    if (w <= 0 || h <= 0) return 2;
    uint8_t* data = nullptr;
    buffer.as<::Windows::Storage::Streams::IBufferByteAccess>()->Buffer(&data);
    int rc_out = 4;
    ULONG_PTR token = 0;
    Gdiplus::GdiplusStartupInput si;
    if (Gdiplus::GdiplusStartup(&token, &si, nullptr) == Gdiplus::Ok) {
        {
            // PREMULTIPLIED: RenderTargetBitmap hands back premultiplied BGRA8, and reading it as
            // straight ARGB darkened every translucent pixel (WinUI's layered page fills went gray).
            Gdiplus::Bitmap bitmap(w, h, stride_w * 4, PixelFormat32bppPARGB, data);
            CLSID clsid;
            if (png_encoder_clsid(&clsid) >= 0) {
                std::wstring wpath = hs(path).c_str();
                if (bitmap.Save(wpath.c_str(), &clsid, nullptr) == Gdiplus::Ok) rc_out = 0;
            }
        }
        Gdiplus::GdiplusShutdown(token);
    }
    return rc_out;
}
} // extern "C++"
#endif

int day_xaml_snapshot_png(void* win, const char* path) try {
    auto app = reinterpret_cast<AppWindow*>(win);
#ifdef DAY_WINUI
    return app ? snapshot_winui_png(app->root, app->host, path, app->capture_origin) : 1;
#else
    return app ? snapshot_hwnd_png(app->host, path) : 1;
#endif
} catch (winrt::hresult_error const& e) {
    std::fprintf(stderr, "day-toolkit-winui: capture threw hr=0x%08X %ls\n",
                 static_cast<unsigned>(e.code()), e.message().c_str());
    std::fflush(stderr);
    return 9;
} catch (...) {
    return 9;
}

// Where the content sits in the primary capture, in DIPs (`Toolkit::snapshot_origin`). The
// system-XAML capture is the whole window rect: the title bar above and the invisible resize
// borders on the left, right and bottom, so the client area's offset in that rect is what a
// reader of the capture needs. WinUI records the visible-frame offset with each WGC capture,
// or (0, 0) when falling back to a capture of just the XAML tree.
int day_xaml_snapshot_origin(void* win, double* out) try {
    auto app = reinterpret_cast<AppWindow*>(win);
    if (!app || !out) return 1;
#ifdef DAY_WINUI
    out[0] = app->capture_origin[0];
    out[1] = app->capture_origin[1];
    return 0;
#else
    if (!app->host) return 1;
    RECT r{};
    POINT p{0, 0};
    if (!GetWindowRect(app->host, &r) || !ClientToScreen(app->host, &p)) return 2;
    double scale = GetDpiForWindow(app->host) / 96.0;
    if (scale <= 0.0) scale = 1.0;
    out[0] = (p.x - r.left) / scale;
    out[1] = (p.y - r.top) / scale;
    return 0;
#endif
} catch (...) {
    return 9;
}

// The same capture for a SECONDARY window (docs/windows.md) — what a dayscript
// `screenshot: { window: … }` step targets. Without this the backend fell back to the primary
// and the walkthrough's `preferences` capture silently showed the main window instead.
int day_xaml_snapshot_png2(void* win, const char* path) try {
    auto sw = reinterpret_cast<SecWindow*>(win);
#ifdef DAY_WINUI
    return sw ? snapshot_winui_png(sw->root, sw->host, path) : 1;
#else
    return sw ? snapshot_hwnd_png(sw->host, path) : 1;
#endif
} catch (...) {
    return 9;
}

