// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// Included inside shim.cpp's C ABI block. Every drawing goes through paint_canvas,
// whether its destination is a live control or an offscreen PNG.
extern "C++" {
namespace WC = winrt::Microsoft::Graphics::Canvas;
namespace WCG = WC::Geometry;
namespace WCB = WC::Brushes;
namespace WCE = WC::Effects;
namespace WCT = WC::Text;
namespace WCX = WC::UI::Xaml;
namespace WFN = winrt::Windows::Foundation::Numerics;

// Cargo does not run MSBuild's WinRT registration task. Preload Win2D from the
// app-local DLL, with the pinned SDK cache as the development fallback. C++/WinRT's
// normal activation then finds it by namespace. Do not replace winrt_activation_handler:
// delegating only to RoGetActivationFactory bypasses C++/WinRT's DLL fallback and
// breaks other unpackaged components such as WebView2 after the first canvas draws.
static void canvas_activate() {
    static HMODULE module = [] {
        wchar_t path[32768]{};
        GetModuleFileNameW(nullptr, path, 32768);
        std::wstring local(path);
        local.resize(local.find_last_of(L"\\/") + 1);
        local += L"Microsoft.Graphics.Canvas.dll";
        auto dll = LoadLibraryExW(local.c_str(), nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
        if (!dll) dll = LoadLibraryExW(DAY_WIN2D_DLL, nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
        return dll;
    }();
    if (!module) winrt::throw_hresult(HRESULT_FROM_WIN32(ERROR_MOD_NOT_FOUND));
}

static WC::CanvasDevice canvas_device() {
    canvas_activate();
    return WC::CanvasDevice::GetSharedDevice();
}

static winrt::com_ptr<IWICImagingFactory> canvas_wic() {
    return winrt::create_instance<IWICImagingFactory>(CLSID_WICImagingFactory);
}

struct CanvasPixels { uint32_t width, height; std::vector<uint8_t> bytes; };
static CanvasPixels canvas_decode(std::vector<uint8_t> const& bytes, uint32_t fit_w = 0, uint32_t fit_h = 0) {
    auto factory = canvas_wic();
    winrt::com_ptr<IWICStream> stream;
    winrt::check_hresult(factory->CreateStream(stream.put()));
    winrt::check_hresult(stream->InitializeFromMemory(const_cast<BYTE*>(bytes.data()), static_cast<DWORD>(bytes.size())));
    winrt::com_ptr<IWICBitmapDecoder> decoder;
    winrt::check_hresult(factory->CreateDecoderFromStream(stream.get(), nullptr, WICDecodeMetadataCacheOnLoad, decoder.put()));
    winrt::com_ptr<IWICBitmapFrameDecode> frame;
    winrt::check_hresult(decoder->GetFrame(0, frame.put()));
    uint32_t w, h;
    winrt::check_hresult(frame->GetSize(&w, &h));
    // The same allocation limit as Day's export API, also for imported image buffers.
    if (!w || !h || w > 16384 || h > 16384 || uint64_t(w) * h > 64000000) winrt::throw_hresult(E_INVALIDARG);
    winrt::com_ptr<IWICBitmapSource> source = frame.as<IWICBitmapSource>();
    if (fit_w && fit_h) {
        double scale = (std::min)({1.0, double(fit_w) / w, double(fit_h) / h});
        uint32_t nw = (std::max)(1u, uint32_t(std::round(w * scale)));
        uint32_t nh = (std::max)(1u, uint32_t(std::round(h * scale)));
        if (nw != w || nh != h) {
            winrt::com_ptr<IWICBitmapScaler> scaler;
            winrt::check_hresult(factory->CreateBitmapScaler(scaler.put()));
            winrt::check_hresult(scaler->Initialize(source.get(), nw, nh, WICBitmapInterpolationModeFant));
            source = scaler.as<IWICBitmapSource>(); w = nw; h = nh;
        }
    }
    winrt::com_ptr<IWICFormatConverter> converter;
    winrt::check_hresult(factory->CreateFormatConverter(converter.put()));
    winrt::check_hresult(converter->Initialize(source.get(), GUID_WICPixelFormat32bppPBGRA,
        WICBitmapDitherTypeNone, nullptr, 0, WICBitmapPaletteTypeCustom));
    CanvasPixels pixels{w, h, std::vector<uint8_t>(size_t(w) * h * 4)};
    winrt::check_hresult(converter->CopyPixels(nullptr, w * 4, static_cast<UINT>(pixels.bytes.size()), pixels.bytes.data()));
    return pixels;
}

static std::vector<uint8_t> canvas_png(CanvasPixels const& pixels) {
    auto factory = canvas_wic();
    winrt::com_ptr<IStream> stream;
    winrt::check_hresult(CreateStreamOnHGlobal(nullptr, TRUE, stream.put()));
    winrt::com_ptr<IWICBitmapEncoder> encoder;
    winrt::check_hresult(factory->CreateEncoder(GUID_ContainerFormatPng, nullptr, encoder.put()));
    winrt::check_hresult(encoder->Initialize(stream.get(), WICBitmapEncoderNoCache));
    winrt::com_ptr<IWICBitmapFrameEncode> frame;
    winrt::check_hresult(encoder->CreateNewFrame(frame.put(), nullptr));
    winrt::check_hresult(frame->Initialize(nullptr));
    winrt::check_hresult(frame->SetSize(pixels.width, pixels.height));
    WICPixelFormatGUID format = GUID_WICPixelFormat32bppBGRA;
    winrt::check_hresult(frame->SetPixelFormat(&format));
    // PNG stores straight alpha. Let WIC perform the premultiplied conversion.
    winrt::com_ptr<IWICBitmap> bitmap;
    winrt::check_hresult(factory->CreateBitmapFromMemory(pixels.width, pixels.height,
        GUID_WICPixelFormat32bppPBGRA, pixels.width * 4, static_cast<UINT>(pixels.bytes.size()),
        const_cast<BYTE*>(pixels.bytes.data()), bitmap.put()));
    winrt::com_ptr<IWICFormatConverter> converter;
    winrt::check_hresult(factory->CreateFormatConverter(converter.put()));
    winrt::check_hresult(converter->Initialize(bitmap.get(), format, WICBitmapDitherTypeNone,
        nullptr, 0, WICBitmapPaletteTypeCustom));
    winrt::check_hresult(frame->WriteSource(converter.get(), nullptr));
    winrt::check_hresult(frame->Commit());
    winrt::check_hresult(encoder->Commit());
    STATSTG stat{};
    winrt::check_hresult(stream->Stat(&stat, STATFLAG_NONAME));
    std::vector<uint8_t> bytes(static_cast<size_t>(stat.cbSize.QuadPart));
    winrt::check_hresult(stream->Seek({}, STREAM_SEEK_SET, nullptr));
    ULONG read = 0;
    winrt::check_hresult(stream->Read(bytes.data(), static_cast<ULONG>(bytes.size()), &read));
    if (read != bytes.size()) winrt::throw_hresult(E_FAIL);
    return bytes;
}

static std::map<uint64_t, WC::CanvasBitmap> canvas_images;
static WC::CanvasBitmap canvas_image(WC::CanvasDevice const& device, uint64_t id) {
    auto encoded = xaml_bitmaps().find(id);
    if (encoded == xaml_bitmaps().end()) return nullptr;
    auto found = canvas_images.find(id);
    if (found != canvas_images.end() && found->second.Device() == device) return found->second;
    auto pixels = canvas_decode(encoded->second);
    auto bitmap = WC::CanvasBitmap::CreateFromBytes(device, pixels.bytes, pixels.width, pixels.height,
        winrt::Windows::Graphics::DirectX::DirectXPixelFormat::B8G8R8A8UIntNormalized, 96,
        WC::CanvasAlphaMode::Premultiplied);
    canvas_images.insert_or_assign(id, bitmap);
    return bitmap;
}

static WCT::CanvasTextLayout canvas_text(WC::CanvasDevice const& device, std::string const& text,
                                        float size, int weight, bool italic, std::string const& family) {
    WCT::CanvasTextFormat format;
    format.FontSize(size);
    format.FontFamily(hs(family.empty() ? "Segoe UI" : family.c_str()));
    format.FontWeight(winrt::Windows::UI::Text::FontWeight{static_cast<uint16_t>(weight ? weight : 400)});
    format.FontStyle(italic ? winrt::Windows::UI::Text::FontStyle::Italic : winrt::Windows::UI::Text::FontStyle::Normal);
    format.WordWrapping(WCT::CanvasWordWrapping::NoWrap);
    return WCT::CanvasTextLayout(device, hs(text.c_str()), format, 0, 0);
}

static WCG::CanvasGeometry canvas_path(WC::CanvasDevice const& device, GeoData const& data) {
    WCG::CanvasPathBuilder builder(device);
    builder.SetFilledRegionDetermination(data.rule == 1 ? WCG::CanvasFilledRegionDetermination::Alternate
                                                       : WCG::CanvasFilledRegionDetermination::Winding);
    for (auto const& figure : data.figs) {
        builder.BeginFigure(WFN::float2{figure.x, figure.y});
        for (auto const& seg : figure.segs) {
            if (seg.op == 'L') builder.AddLine(WFN::float2{seg.v[0], seg.v[1]});
            else if (seg.op == 'Q') builder.AddQuadraticBezier({seg.v[0], seg.v[1]}, {seg.v[2], seg.v[3]});
            else builder.AddCubicBezier({seg.v[0], seg.v[1]}, {seg.v[2], seg.v[3]}, {seg.v[4], seg.v[5]});
        }
        builder.EndFigure(figure.closed ? WCG::CanvasFigureLoop::Closed : WCG::CanvasFigureLoop::Open);
    }
    return WCG::CanvasGeometry::CreatePath(builder);
}

static GeoData canvas_polygon(std::string const& text) {
    GeoData data; data.rule = 0; data.figs.emplace_back();
    auto& f = data.figs.back(); f.closed = true;
    const char* pos = text.data(); const char* end = pos + text.size();
    bool first = true;
    while (pos < end) {
        while (pos < end && (*pos == ' ' || *pos == ',')) ++pos;
        float x = 0, y = 0;
        auto a = std::from_chars(pos, end, x); if (a.ec != std::errc()) break; pos = a.ptr;
        while (pos < end && (*pos == ' ' || *pos == ',')) ++pos;
        auto b = std::from_chars(pos, end, y); if (b.ec != std::errc()) break; pos = b.ptr;
        if (first) { f.x = x; f.y = y; first = false; }
        else f.segs.push_back({'L', {x, y}});
    }
    return data;
}

struct CanvasDisplayList {
    std::vector<double> nums;
    std::vector<std::string> texts;
    void assign(const double* values, int count, const char* joined) {
        nums.assign(values, values + count); texts.clear();
        std::string all = joined ? joined : "";
        size_t start = 0;
        do {
            size_t next = all.find('\x1f', start);
            texts.push_back(all.substr(start, next == std::string::npos ? next : next - start));
            if (next == std::string::npos) break;
            start = next + 1;
        } while (true);
    }
};

// ---- conic gradients (docs/canvas.md "Conic gradients") ----
//
// Direct2D has no conic brush, so one is drawn as a fan of thin wedges around the center, each
// a flat color, clipped to the shape (or to the stroke's outline). The wedge edges fall on a
// uniform grid plus every stop offset, so a hard stop lands exactly where it was put.

/// The color `turn` (0..=1 of a full turn from the start angle) names: linear between stops,
/// channel by channel, flat before the first and after the last. `ConicGradient::color_at`.
static WUI::Color conic_color(std::vector<WCB::CanvasGradientStop> const& stops, double turn) {
    if (stops.empty()) return WUI::Color{};
    if (turn <= stops.front().Position) return stops.front().Color;
    for (size_t i = 1; i < stops.size(); ++i) {
        auto const& a = stops[i - 1];
        auto const& b = stops[i];
        if (turn <= b.Position) {
            double k = b.Position > a.Position ? (turn - a.Position) / (b.Position - a.Position) : 1.0;
            auto mix = [k](uint8_t x, uint8_t y) {
                return static_cast<uint8_t>(std::lround(x + (double(y) - double(x)) * k));
            };
            return WUI::Color{mix(a.Color.A, b.Color.A), mix(a.Color.R, b.Color.R),
                              mix(a.Color.G, b.Color.G), mix(a.Color.B, b.Color.B)};
        }
    }
    return stops.back().Color;
}

/// The wedge edges, as ascending turns from 0 to 1: `steps` equal parts and every stop offset.
static std::vector<double> conic_turns(std::vector<WCB::CanvasGradientStop> const& stops, int steps) {
    std::vector<double> at;
    at.reserve(size_t(steps) + stops.size() + 1);
    for (int i = 0; i <= steps; ++i) at.push_back(double(i) / steps);
    for (auto const& stop : stops)
        if (stop.Position > 0 && stop.Position < 1) at.push_back(double(stop.Position));
    std::sort(at.begin(), at.end());
    at.erase(std::unique(at.begin(), at.end()), at.end());
    return at;
}

/// How many equal wedges a fan reaching `reach` units from its center is cut into: one per two
/// units of circumference, never coarser than one degree nor finer than a quarter of one.
static int conic_steps(double reach) {
    double wanted = std::ceil(reach * 3.14159265358979323846);
    return int(std::clamp(wanted, 360.0, 1440.0));
}

/// Fill `region` with the conic gradient whose center is the unit point (`ux`, `uy`) of
/// `box` and whose first stop sits at `start_deg` (clockwise from +x, y down).
///
/// The wedges are drawn aliased with the Copy blend inside one layer clipped to `region`:
/// each overlaps the next by a sliver the next one then overwrites, so neighbors neither
/// leave a gap nor double a translucent color, and the layer's antialiased mask is the only
/// soft edge. The layer starts filled with the color the turn ends on, which is what shows in
/// the one place two wedges only touch (where the turn closes).
static void canvas_conic(WC::CanvasDrawingSession const& s, WCG::CanvasGeometry const& region,
                         WF::Rect const& box, float ux, float uy, float start_deg,
                         std::vector<WCB::CanvasGradientStop> const& stops) {
    auto cover = region.ComputeBounds();
    if (!(cover.Width > 0) || !(cover.Height > 0)) return;
    const double cx = box.X + ux * box.Width, cy = box.Y + uy * box.Height;
    double reach = 0;
    for (double x : {double(cover.X), double(cover.X) + cover.Width})
        for (double y : {double(cover.Y), double(cover.Y) + cover.Height})
            reach = (std::max)(reach, std::hypot(x - cx, y - cy));
    const int steps = conic_steps(reach);
    const auto turns = conic_turns(stops, steps);
    constexpr double tau = 6.28318530717958647692;
    const double start = double(start_deg) / 360.0, pad = 0.25 / steps;
    // A triangle's far side is a chord: push its corners out so the chord clears `reach`.
    const double radius = (reach + 1) * 1.01 / std::cos(tau * (1.0 / steps + pad) / 2);
    auto device = s.Device();
    auto layer = s.CreateLayer(1.0f, region);
    const auto antialiasing = s.Antialiasing();
    const auto blend = s.Blend();
    s.Antialiasing(WC::CanvasAntialiasing::Aliased);
    s.Blend(WC::CanvasBlend::Copy);
    try {
        s.FillRectangle(cover, conic_color(stops, 1.0));
        for (size_t i = 0; i + 1 < turns.size(); ++i) {
            const double from = turns[i], to = turns[i + 1];
            const double a0 = tau * (start + from);
            const double a1 = tau * (start + (i + 2 < turns.size() ? to + pad : to));
            const WFN::float2 corners[3] = {
                {float(cx), float(cy)},
                {float(cx + radius * std::cos(a0)), float(cy + radius * std::sin(a0))},
                {float(cx + radius * std::cos(a1)), float(cy + radius * std::sin(a1))},
            };
            s.FillGeometry(WCG::CanvasGeometry::CreatePolygon(device, corners),
                           conic_color(stops, (from + to) / 2));
        }
    } catch (...) {
        s.Antialiasing(antialiasing); s.Blend(blend); layer.Close();
        throw;
    }
    s.Antialiasing(antialiasing);
    s.Blend(blend);
    layer.Close();
}

/// The two offscreen targets a canvas with a separable blend mode paints through, kept between
/// frames so an animated one does not allocate them again each time.
struct CanvasScratch { WC::CanvasRenderTarget front{nullptr}, back{nullptr}; };

/// Paint a display list. `size` is the destination's extent in the session's units, `scale`
/// the units per point (1 on screen, the pixel scale for an export).
///
/// Shadows and blend modes are graphics state Direct2D does not have, so both live here:
///
/// - A shadow (kind 24) is a `ShadowEffect` over a command list holding the one record, drawn
///   under it with the identity transform, so its offset and blur are points whatever the
///   current transform is. `BlurAmount` is the Gaussian's standard deviation: half Day's blur.
/// - Add is Direct2D's own primitive blend and Erase its DestinationOut composite, both applied
///   to the record as an image. The clip is then cut into that image, because a clip layer on
///   the target would confine either one to the layer's own, empty, content.
/// - Multiply, Screen, Overlay, Darken and Lighten need the backdrop as an input
///   (`BlendEffect`), and a target cannot be read while it is drawn to. A list that uses one of
///   them is painted into an offscreen target and its twin, swapped at each such record, and
///   copied to the destination at the end. The backdrop is therefore only ever this canvas.
static void paint_canvas(WC::CanvasDrawingSession const& ds, CanvasDisplayList const& ops,
                         WF::Size size, float scale = 1, CanvasScratch* scratch = nullptr) {
    auto device = ds.Device();
    const WFN::float3x2 identity = WFN::float3x2::identity();
    const WF::Rect all{0, 0, size.Width, size.Height};
    bool separable = false;
    for (size_t i = 0; i + 8 < ops.nums.size(); i += 9) {
        const int mode = int(ops.nums[i + 1]);
        if (int(ops.nums[i]) == 25 && mode >= 1 && mode <= 5) separable = true;
    }
    // The session being drawn to: the destination, or the offscreen front target.
    WC::CanvasDrawingSession t = ds;
    WC::CanvasRenderTarget front{nullptr}, back{nullptr};
    if (separable && size.Width > 0 && size.Height > 0) {
        try {
            CanvasScratch local;
            auto& kept = scratch ? *scratch : local;
            auto fits = [&](WC::CanvasRenderTarget const& target) {
                return target && target.Device() == device && target.Size() == size && target.Dpi() == ds.Dpi();
            };
            if (!fits(kept.front)) kept.front = WC::CanvasRenderTarget(ds, size);
            if (!fits(kept.back)) kept.back = WC::CanvasRenderTarget(ds, size);
            front = kept.front; back = kept.back;
            t = front.CreateDrawingSession();
            t.Clear(color_argb(0));
        } catch (...) {
            // No offscreen target (a canvas larger than a texture): those modes draw as Normal.
            front = nullptr; back = nullptr; t = ds;
        }
    }
    const bool offscreen = static_cast<bool>(front);

    WFN::float3x2 cur = WFN::make_float3x2_scale(scale);
    struct Shade { WUI::Color color{}; float dx = 0, dy = 0, blur = 0; bool on = false; };
    Shade shade; int blend = 0;
    struct Saved { WFN::float3x2 transform; size_t clips; Shade shade; int blend; };
    std::vector<Saved> saved;
    // The clip stack, and the layers that realize it on `t`. The layers are opened lazily and
    // dropped whenever a record must reach the target unclipped (see above).
    struct Clip { WCG::CanvasGeometry geometry; WFN::float3x2 transform; };
    std::vector<Clip> clips;
    std::vector<WC::CanvasActiveLayer> layers;
    auto drop_layers = [&] { while (!layers.empty()) { layers.back().Close(); layers.pop_back(); } };
    auto ensure_layers = [&] {
        while (layers.size() < clips.size()) {
            auto const& clip = clips[layers.size()];
            t.Transform(clip.transform);
            layers.push_back(t.CreateLayer(1.0f, clip.geometry));
        }
    };
    // Close layers in reverse order, and the offscreen session, even if a drawing operation fails.
    struct Finish {
        std::vector<WC::CanvasActiveLayer>& layers; WC::CanvasDrawingSession& t; bool offscreen;
        ~Finish() {
            while (!layers.empty()) { try { layers.back().Close(); } catch (...) {} layers.pop_back(); }
            if (offscreen) { try { t.Close(); } catch (...) {} }
        }
    } finish{layers, t, offscreen};

    // Put `image`, moved by `at`, on the target under the current blend mode.
    auto compose = [&](WC::ICanvasImage const& image, WFN::float2 at) {
        if (blend == 0 || (blend >= 1 && blend <= 5 && !offscreen)) {
            ensure_layers(); t.Transform(identity); t.DrawImage(image, at);
            return;
        }
        WC::CanvasCommandList clipped(device);
        {
            auto s = clipped.CreateDrawingSession();
            std::vector<WC::CanvasActiveLayer> open;
            for (auto const& clip : clips) { s.Transform(clip.transform); open.push_back(s.CreateLayer(1.0f, clip.geometry)); }
            s.Transform(identity); s.DrawImage(image, at);
            while (!open.empty()) { open.back().Close(); open.pop_back(); }
            s.Close();
        }
        // Cropped to the canvas: an effect, so Direct2D composes it as one image, and bounded.
        WCE::CropEffect source; source.Source(clipped); source.SourceRectangle(all);
        drop_layers(); t.Transform(identity);
        if (blend == 6) {
            t.Blend(WC::CanvasBlend::Add); t.DrawImage(source); t.Blend(WC::CanvasBlend::SourceOver);
        } else if (blend == 7) {
            t.DrawImage(source, 0.0f, 0.0f, all, 1.0f, WC::CanvasImageInterpolation::NearestNeighbor,
                        WC::CanvasComposite::DestinationOut);
        } else {
            t.Close();
            std::swap(front, back);
            t = front.CreateDrawingSession();
            WCE::BlendEffect mixed; mixed.Background(back); mixed.Foreground(source);
            mixed.Mode(blend == 1 ? WCE::BlendEffectMode::Multiply : blend == 2 ? WCE::BlendEffectMode::Screen
                     : blend == 3 ? WCE::BlendEffectMode::Overlay : blend == 4 ? WCE::BlendEffectMode::Darken
                     : WCE::BlendEffectMode::Lighten);
            t.DrawImage(mixed, 0.0f, 0.0f, all, 1.0f, WC::CanvasImageInterpolation::NearestNeighbor,
                        WC::CanvasComposite::Copy);
        }
    };
    // Draw one record, `draw`, under the transform `m`, the shadow and the blend mode.
    auto emit = [&](std::function<void(WC::CanvasDrawingSession const&)> const& draw, WFN::float3x2 const& m) {
        if (!shade.on && blend == 0) { ensure_layers(); t.Transform(m); draw(t); return; }
        WC::CanvasCommandList shape(device);
        { auto s = shape.CreateDrawingSession(); s.Transform(m); draw(s); s.Close(); }
        if (shade.on) {
            WCE::ShadowEffect cast; cast.Source(shape);
            cast.BlurAmount((std::min)(shade.blur / 2 * scale, 250.0f));
            cast.ShadowColor(shade.color);
            compose(cast, {shade.dx * scale, shade.dy * scale});
        }
        if (blend == 0) { ensure_layers(); t.Transform(m); draw(t); }
        else compose(shape, {0, 0});
    };

    std::vector<WFN::float2> stamps;
    size_t stamp_count = 0, ti = 0;
    auto text = [&]() { return ti < ops.texts.size() ? ops.texts[ti++] : std::string(); };
    std::vector<WCB::CanvasGradientStop> stops;
    float gx=0, gy=0, gz=0, gw=0; int gradient_kind = 0;
    WCG::CanvasStrokeStyle style{nullptr};
    std::vector<float> style_dash; float style_phase = 0;
    int weight = 0; bool italic = false; std::string family;
    // A kind-23 record: the region, in the bitmap's pixels, the next image record draws.
    bool has_source = false; WF::Rect source{};
    for (size_t i = 0; i + 8 < ops.nums.size(); i += 9) {
        auto p = ops.nums.data() + i;
        int k = int(p[0]); float a=float(p[1]), b=float(p[2]), c=float(p[3]), d=float(p[4]);
        float e=float(p[5]), f=float(p[6]), g=float(p[7]); auto color = color_argb(static_cast<unsigned>(p[8]));
        if (k == 20) { stamps.clear(); stamp_count = size_t(a); continue; }
        if (k == 21) { for (int j=0; j<4 && stamps.size()<stamp_count; ++j) stamps.push_back({float(p[1+j*2]),float(p[2+j*2])}); continue; }
        if (k == 8) { saved.push_back({cur, clips.size(), shade, blend}); continue; }
        if (k == 9) {
            if (!saved.empty()) {
                auto const& was = saved.back();
                while (layers.size() > was.clips) { layers.back().Close(); layers.pop_back(); }
                if (clips.size() > was.clips) clips.erase(clips.begin() + was.clips, clips.end());
                cur = was.transform; shade = was.shade; blend = was.blend; saved.pop_back();
            }
            continue;
        }
        if (k == 10) { cur = WFN::float3x2{a,b,c,d,e,f} * cur; continue; }
        // Graphics state, like the clip: a transparent color is "no shadow", and a mode this
        // decoder does not know draws as Normal.
        if (k == 24) { shade = Shade{color, a, b, (std::max)(c, 0.0f), color.A != 0}; continue; }
        if (k == 25) { blend = int(a); if (blend < 0 || blend > 7) blend = 0; continue; }
        if (k == 19) { weight=int(a); italic=b!=0; family=text(); continue; }
        if (k == 23) { source=WF::Rect{a,b,c,d}; has_source=true; continue; }
        if (k == 18) {
            style = WCG::CanvasStrokeStyle();
            auto cap = int(a)==1 ? WCG::CanvasCapStyle::Round : int(a)==2 ? WCG::CanvasCapStyle::Square : WCG::CanvasCapStyle::Flat;
            style.StartCap(cap); style.EndCap(cap); style.DashCap(cap);
            style.LineJoin(int(b)==1 ? WCG::CanvasLineJoin::Round : int(b)==2 ? WCG::CanvasLineJoin::Bevel : WCG::CanvasLineJoin::Miter);
            style.MiterLimit(c); style_phase = d;
            auto payload = text(); std::vector<float> dash;
            const char* at=payload.data(); const char* end=at+payload.size();
            while (at<end) { if (*at==' ') { ++at; continue; } float v=0; auto parsed=std::from_chars(at,end,v); if(parsed.ec!=std::errc())break; dash.push_back(v); at=parsed.ptr; }
            style_dash = std::move(dash);
            continue;
        }
        if (k == 14) {
            gx=a; gy=b; gz=c; gw=d; gradient_kind=int(f); stops.clear();
            auto payload=text(); const char* at=payload.data(); const char* end=at+payload.size();
            while(at<end) {
                if (*at==' ') { ++at; continue; }
                float offset=0; unsigned bits=0;
                auto x=std::from_chars(at,end,offset); if(x.ec!=std::errc() || x.ptr==end)break;
                auto y=std::from_chars(x.ptr+1,end,bits,16); if(y.ec!=std::errc())break;
                stops.push_back({offset,color_argb(bits)}); at=y.ptr;
            }
            std::stable_sort(stops.begin(),stops.end(),[](auto const& x,auto const& y){return x.Position<y.Position;});
            continue;
        }
        const size_t payload_start=ti;
        size_t repetitions=stamps.empty()?1:stamps.size();
        for(size_t rep=0;rep<repetitions;++rep) {
            ti=payload_start;
            const WFN::float3x2 m=stamps.empty()?cur:WFN::make_float3x2_translation(stamps[rep])*cur;
            WCG::CanvasGeometry geometry{nullptr};
            bool stroke = k==1 || k==4 || k==5 || k==6 || k==12 || k==13 || k==16;
            int shape = k;
            if(k==17) shape = int(f)==0?0:int(f)==1?2:int(f)==2?3:int(f)==3?15:11;
            if(shape==0 || shape==1) geometry=WCG::CanvasGeometry::CreateRectangle(device, a,b,c,d);
            else if(shape==2 || shape==13) geometry=WCG::CanvasGeometry::CreateRoundedRectangle(device, a,b,c,d,e,e);
            else if(shape==3 || shape==4) geometry=WCG::CanvasGeometry::CreateEllipse(device,a+c/2,b+d/2,c/2,d/2);
            else if(shape==11 || shape==12) geometry=canvas_path(device,canvas_polygon(text()));
            else if(shape==15 || shape==16) geometry=canvas_path(device,parse_path_data(text(),int(k==17?e:f)));
            else if(shape==5 || shape==6) {
                WCG::CanvasPathBuilder path(device);
                if(shape==6) { path.BeginFigure({a,b}); path.AddLine({c,d}); }
                else {
                    constexpr float radians=3.14159265358979323846f/180;
                    // Center/radii arc API handles full revolutions and either sweep direction.
                    path.BeginFigure({a+c/2+c/2*std::cos(e*radians),b+d/2+d/2*std::sin(e*radians)});
                    path.AddArc({a+c/2,b+d/2},c/2,d/2,e*radians,f*radians);
                }
                path.EndFigure(WCG::CanvasFigureLoop::Open); geometry=WCG::CanvasGeometry::CreatePath(path);
            }
            if(k==17) { if(geometry) clips.push_back({geometry,m}); continue; }
            if(geometry) {
                if(stroke && !(g>0)) continue;
                WCB::ICanvasBrush brush=WCB::CanvasSolidColorBrush(device,color);
                const auto bounds=geometry.ComputeBounds();
                const bool conic=stops.size()>=2 && gradient_kind==2;
                if(stops.size()>=2 && !conic) {
                    if(gradient_kind==1) {
                        WCB::CanvasRadialGradientBrush gradient(device,stops);
                        gradient.Center({bounds.X+gx*bounds.Width,bounds.Y+gy*bounds.Height});
                        gradient.RadiusX(gz*bounds.Width); gradient.RadiusY(gz*bounds.Height); brush=gradient;
                    } else {
                        WCB::CanvasLinearGradientBrush gradient(device,stops);
                        gradient.StartPoint({bounds.X+gx*bounds.Width,bounds.Y+gy*bounds.Height});
                        gradient.EndPoint({bounds.X+gz*bounds.Width,bounds.Y+gw*bounds.Height}); brush=gradient;
                    }
                }
                WCG::CanvasStrokeStyle actual{nullptr};
                if(stroke) {
                    actual=style;
                    if(!actual) { actual=WCG::CanvasStrokeStyle(); if(k==5 || k==6) { actual.StartCap(WCG::CanvasCapStyle::Round); actual.EndCap(WCG::CanvasCapStyle::Round); } }
                    // Day dashes are absolute lengths; Direct2D's are multiples of stroke width.
                    if(style) { auto dash=style_dash; for(auto& value:dash)value/=g; actual.CustomDashStyle(dash); actual.DashOffset(style_phase/g); }
                }
                emit([&](WC::CanvasDrawingSession const& s) {
                    // A conic stroke fills the stroke's own outline, dashes and caps included.
                    if(conic) canvas_conic(s,stroke?geometry.Stroke(g,actual):geometry,bounds,gx,gy,gz,stops);
                    else if(stroke) s.DrawGeometry(geometry,brush,g,actual);
                    else s.FillGeometry(geometry,brush);
                },m);
            } else if(k==7) {
                auto value=text(); if(!(e>0) || !std::isfinite(e))continue;
                auto layout=canvas_text(device,value,e,weight,italic,family);
                auto bounds=layout.LayoutBoundsIncludingTrailingWhitespace(); auto lines=layout.LineMetrics();
                int horizontal=int(f)/4, vertical=int(f)%4;
                float x=a-(horizontal==1?bounds.Width/2:horizontal==2?bounds.Width:0);
                float y=b-(vertical==1?bounds.Height/2:vertical==2?(lines.empty()?0:lines[0].Baseline):vertical==3?bounds.Height:0);
                emit([&](WC::CanvasDrawingSession const& s) { s.DrawTextLayout(layout,{x,y},color); },m);
            } else if(k==22 && c>0 && d>0) {
                if(auto bitmap=canvas_image(device,static_cast<uint64_t>(p[5]))) {
                    // The bitmap is created at 96 DPI, so its bounds are its pixels: the space
                    // a region names. The region is clamped to the bitmap with its mapping into
                    // the rect kept (day_spec::clamp_image_source); an empty intersection
                    // draws nothing.
                    auto bounds=bitmap.Bounds(); WF::Rect dest{a,b,c,d}; WF::Rect src=bounds; bool draw=true;
                    if(has_source) {
                        float x0=(std::max)(0.0f,source.X), y0=(std::max)(0.0f,source.Y);
                        float x1=(std::min)(bounds.Width,source.X+source.Width), y1=(std::min)(bounds.Height,source.Y+source.Height);
                        draw=source.Width>0 && source.Height>0 && x1>x0 && y1>y0;
                        if(draw) {
                            float kx=c/source.Width, ky=d/source.Height;
                            dest=WF::Rect{a+(x0-source.X)*kx,b+(y0-source.Y)*ky,(x1-x0)*kx,(y1-y0)*ky};
                            src=WF::Rect{x0,y0,x1-x0,y1-y0};
                        }
                    }
                    if(draw) emit([&](WC::CanvasDrawingSession const& s) { s.DrawImage(bitmap,dest,src,std::clamp(f,0.0f,1.0f)); },m);
                }
            }
        }
        stamps.clear(); stops.clear(); style=nullptr; weight=0; italic=false; family.clear(); has_source=false;
    }
    drop_layers();
    if (offscreen) {
        t.Close();
        ds.Transform(identity);
        ds.DrawImage(front);
    }
}

struct CanvasState {
    WCX::CanvasControl control{nullptr}; CanvasDisplayList ops; winrt::event_token draw;
    CanvasScratch scratch;
};
static std::map<void*,std::shared_ptr<CanvasState>> canvas_states;
static void canvas_forget(void* h) {
    auto it=canvas_states.find(h); if(it==canvas_states.end())return;
    it->second->control.Draw(it->second->draw);
    it->second->control.RemoveFromVisualTree();
    canvas_states.erase(it);
}
} // extern C++

/// The conic wedge arithmetic with no device behind it, for the crate's tests: the color at
/// `turn`, the number of wedge edges through `edges`, and the uniform step count returned.
int day_xaml_conic_probe(const double* offsets, const unsigned int* colors, int n, double reach,
                         double turn, unsigned int* color, int* edges) {
    std::vector<WCB::CanvasGradientStop> stops;
    for (int i = 0; i < n; ++i) stops.push_back({float(offsets[i]), color_argb(colors[i])});
    const auto c = conic_color(stops, turn);
    *color = (unsigned(c.A) << 24) | (unsigned(c.R) << 16) | (unsigned(c.G) << 8) | unsigned(c.B);
    const int steps = conic_steps(reach);
    *edges = int(conic_turns(stops, steps).size());
    return steps;
}

// The full route (docs/menus.md "The full route"). `KeyDown` names the keys that type nothing
// and the chords; `CharacterReceived` delivers the text, once, after the layout and the input
// method have composed it, so nothing is heard twice: a character that reached `KeyDown` under
// Ctrl or Alt never arrives there (Windows swallows it), and one that arrives there was never
// named here.
static std::string day_xaml_named_key(winrt::Windows::System::VirtualKey k) {
    using VK = winrt::Windows::System::VirtualKey;
    switch (k) {
        case VK::Left:  return "ArrowLeft";
        case VK::Right: return "ArrowRight";
        case VK::Up:    return "ArrowUp";
        case VK::Down:  return "ArrowDown";
        case VK::Enter: return "Enter";
        case VK::Tab:   return "Tab";
        case VK::Escape: return "Escape";
        case VK::Back:  return "Backspace";
        case VK::Delete: return "Delete";
        case VK::Home:  return "Home";
        case VK::End:   return "End";
        case VK::PageUp: return "PageUp";
        case VK::PageDown: return "PageDown";
        case VK::Insert: return "Insert";
        default: break;
    }
    if (k >= VK::F1 && k <= VK::F12) return "F" + std::to_string(static_cast<int>(k) - static_cast<int>(VK::F1) + 1);
    return std::string();
}

// AltGr is Ctrl+Alt to Windows, with the right Alt down: it types (the @ on a German layout),
// so it is not a chord, and `CharacterReceived` names what it typed.
static bool day_xaml_altgr_held() {
    return (GetKeyState(VK_CONTROL)&0x8000) && (GetKeyState(VK_RMENU)&0x8000);
}

// The character a virtual key names under a Ctrl or Alt chord: the letters cased by Shift, and
// every other key by what it types unshifted on the current layout (`MapVirtualKeyW`, so Ctrl+[
// names "[" and Ctrl+\ names "\" on a layout that has them where the US one does, and whatever
// the user's layout puts there otherwise). Empty for a key with no character of its own.
static std::string day_xaml_chord_name(winrt::Windows::System::VirtualKey k, int mods) {
    const UINT code = static_cast<UINT>(k);
    if (code >= 'A' && code <= 'Z') return std::string(1, static_cast<char>((mods & 1) ? code : code + 32));
    const UINT ch = MapVirtualKeyW(code, MAPVK_VK_TO_CHAR) & 0xffff;
    if (ch < 0x20 || ch == 0x7f) return std::string();
    return winrt::to_string(std::wstring(1, static_cast<wchar_t>(ch)));
}

} // extern "C"
extern "C++" {
static int day_key_mods() {
    return (GetKeyState(VK_SHIFT) & 0x8000 ? 1 : 0) | (GetKeyState(VK_CONTROL) & 0x8000 ? 2 : 0) |
           (GetKeyState(VK_MENU) & 0x8000 ? 4 : 0);
}

// Route keys that reach `host` to `cb(id, key, text, mods)`, `mods` carrying the phase bits
// (docs/menus.md). The short route: the arrows and digits while the node has a key handler; the
// full route (`handles_text`) names the keys that type nothing and the chords, and
// `CharacterReceived` delivers the text, once, after the layout and the input method have composed
// it, so nothing is heard twice: a character that reached `KeyDown` under Ctrl or Alt never
// arrives there (Windows swallows it), and one that arrives there was never named here. A held
// key repeats (`KeyStatus.WasKeyDown`); the release of a claimed key reports an up with the same
// name, the modifiers held at the release and no text.
static void day_attach_keys(UIElement const& host, void* h, unsigned long long id,
                            int (*handles)(unsigned long long), int (*handles_text)(unsigned long long),
                            void (*cb)(unsigned long long, const char*, const char*, int)) {
    auto ks = std::make_shared<DayKeyState>();
    g_key_states[h] = ks;
    host.KeyDown([id, handles, handles_text, cb, ks](auto const&, WUXIn::KeyRoutedEventArgs const& args) {
        int mods = day_key_mods();
        const int vk = static_cast<int>(args.Key());
        ks->last_vk = vk;
        if (!handles(id)) return;
        const int phase = args.KeyStatus().WasKeyDown ? kDayKeyRepeat : 0;
        if (handles_text(id)) {
            // Ctrl is both the primary key and the Control key here (docs/menus.md). A Win
            // key chord is the system's, and goes on its way.
            if ((GetKeyState(VK_LWIN) & 0x8000) || (GetKeyState(VK_RWIN) & 0x8000)) return;
            if (mods & 2) mods |= 8;
            const std::string named = day_xaml_named_key(args.Key());
            if (!named.empty()) {
                cb(id, named.c_str(), nullptr, mods | phase);
                ks->down[vk] = named;
                args.Handled(true);
                return;
            }
            // A chord names its key and types nothing; everything else types, and
            // `CharacterReceived` delivers it. AltGr types.
            if ((mods & (2 | 4)) && !day_xaml_altgr_held()) {
                const std::string key = day_xaml_chord_name(args.Key(), mods);
                if (!key.empty()) {
                    cb(id, key.c_str(), nullptr, mods | phase);
                    ks->down[vk] = key;
                }
                args.Handled(true);
            }
            return;
        }
        const char* key = day_xaml_key_name(args.Key(), mods);
        if (!key) return;
        cb(id, key, nullptr, mods | phase);
        ks->down[vk] = key;
        args.Handled(true);
    });
    host.KeyUp([id, handles_text, cb, ks](auto const&, WUXIn::KeyRoutedEventArgs const& args) {
        auto it = ks->down.find(static_cast<int>(args.Key()));
        if (it == ks->down.end()) return;
        const std::string key = it->second;
        ks->down.erase(it);
        int mods = day_key_mods();
        if ((mods & 2) && handles_text(id)) mods |= 8;
        cb(id, key.c_str(), nullptr, mods | kDayKeyUp);
        args.Handled(true);
    });
    // The text a key types, as text (the full route), the two halves of a surrogate pair joined
    // into the one character they spell.
    host.CharacterReceived([id, handles_text, cb, ks](auto const&, WUXIn::CharacterReceivedRoutedEventArgs const& args) {
        if (!handles_text(id)) return;
        const wchar_t c = args.Character();
        std::wstring text;
        if (c >= 0xd800 && c <= 0xdbff) { ks->pending_high = c; args.Handled(true); return; }
        if (c >= 0xdc00 && c <= 0xdfff) {
            if (!ks->pending_high) { args.Handled(true); return; }
            text.push_back(ks->pending_high); text.push_back(c); ks->pending_high = 0;
        } else {
            ks->pending_high = 0;
            if (c < 0x20 || c == 0x7f) return;
            text.push_back(c);
        }
        // Shift is the only modifier a typed character carries: an AltGr character typed
        // plainly as far as the piece is concerned, and a Ctrl or Alt chord never gets here.
        const int mods = (GetKeyState(VK_SHIFT) & 0x8000 ? 1 : 0);
        const int phase = args.KeyStatus().WasKeyDown ? kDayKeyRepeat : 0;
        const std::string utf8 = winrt::to_string(text);
        // One character names itself; a pair is one character too. Its release reports it.
        cb(id, utf8.c_str(), utf8.c_str(), mods | phase);
        if (ks->last_vk >= 0) ks->down[ks->last_vk] = utf8;
        args.Handled(true);
    });
    // A chord the piece claims wins over an application accelerator (docs/menus.md "The full
    // route"): XAML asks the focused element first, and a canvas that took the whole keyboard
    // says it handled the accelerator, so a terminal's Ctrl+C is the terminal's.
    host.ProcessKeyboardAccelerators([id, handles_text](auto const&, WUXIn::ProcessKeyboardAcceleratorEventArgs const& args) {
        if (!handles_text(id)) return;
        if ((GetKeyState(VK_LWIN) & 0x8000) || (GetKeyState(VK_RWIN) & 0x8000)) return;
        args.Handled(true);
    });
    // No up for a key held across a focus loss: the app stops held-key movement on the loss.
    host.LostFocus([host, ks](auto const&, WUX::RoutedEventArgs const& args) {
        if (day_same(args.OriginalSource(), host)) ks->down.clear();
    });
}
} // extern "C++"
extern "C" {

void* day_xaml_canvas_new(unsigned long long id, int (*handles)(unsigned long long),
                          int (*handles_text)(unsigned long long),
                          void (*cb)(unsigned long long,const char*,const char*,int)) {
    canvas_activate();
    auto state=std::make_shared<CanvasState>();
    state->control=WCX::CanvasControl();
    state->control.ClearColor(color_argb(0));
    state->control.IsHitTestVisible(false);
    state->control.UseSharedDevice(true);
    std::weak_ptr<CanvasState> weak=state;
    state->draw=state->control.Draw([weak](WCX::CanvasControl const& sender,WCX::CanvasDrawEventArgs const& args){
        if(auto state=weak.lock()) paint_canvas(args.DrawingSession(),state->ops,sender.Size(),1,&state->scratch);
    });
    // No background: the canvas takes no pointer input until a recognizer or anything else that
    // needs it is attached (`day_hit_on`), so a decorative canvas over a control lets it click.
    WUXC::Grid surface;
    surface.Children().Append(state->control);
    WUXC::ContentControl host; host.IsTabStop(true);
    host.HorizontalContentAlignment(WUX::HorizontalAlignment::Stretch);
    host.VerticalContentAlignment(WUX::VerticalAlignment::Stretch); host.Content(surface);
    host.PointerPressed([](auto const& sender,auto const&){sender.template as<WUXC::Control>().Focus(WUX::FocusState::Pointer);});
    auto h=boxh(host);
    day_attach_keys(host,h,id,handles,handles_text,cb);
    canvas_states.emplace(h,state); return h;
}

void day_xaml_canvas_set_ops(void* h,const double* nums,int n,const char* texts) try {
    auto it=canvas_states.find(h); if(it==canvas_states.end())return;
    it->second->ops.assign(nums,n,texts); it->second->control.Invalidate();
} catch(winrt::hresult_error const& e) { std::fprintf(stderr,"day-toolkit-winui: canvas update failed: %ls\n",e.message().c_str()); }

int day_xaml_canvas_bitmap(unsigned long long id,const double* nums,int n,const char* texts,
                           int width,int height,double scale) try {
    auto device=canvas_device();
    WC::CanvasRenderTarget target(device,float(width),float(height),96);
    CanvasDisplayList ops; ops.assign(nums,n,texts);
    auto ds=target.CreateDrawingSession(); ds.Clear(color_argb(0));
    paint_canvas(ds,ops,WF::Size{float(width),float(height)},float(scale)); ds.Close();
    auto pixels=target.GetPixelBytes();
    auto size=target.SizeInPixels();
    if(size.Width!=unsigned(width)||size.Height!=unsigned(height))return 2;
    xaml_bitmaps()[id]=canvas_png({unsigned(width),unsigned(height),{pixels.begin(),pixels.end()}});
    canvas_images.erase(id);
    return 0;
} catch(winrt::hresult_error const& e) { std::fprintf(stderr,"day-toolkit-winui: bitmap render failed: %ls\n",e.message().c_str()); return 1; }
  catch(...) { return 1; }

unsigned char* day_xaml_encode_png(unsigned long long id,int fit_w,int fit_h,int* len) try {
    *len=0; auto it=xaml_bitmaps().find(id); if(it==xaml_bitmaps().end())return nullptr;
    auto bytes=canvas_png(canvas_decode(it->second,fit_w,fit_h));
    auto result=static_cast<unsigned char*>(malloc(bytes.size())); if(!result)return nullptr;
    memcpy(result,bytes.data(),bytes.size()); *len=static_cast<int>(bytes.size()); return result;
} catch(...) { return nullptr; }
void day_xaml_bytes_free(unsigned char* p) { free(p); }

int day_xaml_measure_text(const char* text,double size,int weight,int italic,const char* family,double* out) try {
    if(!(size>0)||!std::isfinite(size))return 1;
    auto layout=canvas_text(canvas_device(),text?text:"",float(size),weight,italic!=0,family?family:"");
    auto box=layout.LayoutBoundsIncludingTrailingWhitespace(); auto ink=layout.DrawBounds(); auto lines=layout.LineMetrics();
    out[0]=box.Width; out[1]=box.Height; out[2]=lines.empty()?0:lines[0].Baseline;
    // Measure H with the same format for cap ink height, including a bundled face.
    auto cap=canvas_text(canvas_device(),"H",float(size),weight,italic!=0,family?family:"").DrawBounds();
    out[3]=cap.Height; out[4]=ink.X; out[5]=ink.Y; out[6]=ink.Width; out[7]=ink.Height; return 0;
} catch(...) { return 1; }
