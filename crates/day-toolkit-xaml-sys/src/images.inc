// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Included by shim.cpp inside the C ABI block. Shared declarations and include order live there.

// ---- divider / image ----

void* day_xaml_divider_new(int vertical) {
    WUXC::Border b;
    // The thin axis is fixed; layout's frame supplies the long one.
    if (vertical) b.Width(1); else b.Height(1);
    // The app-resource hairline brush resolves per the SYSTEM theme; only trust it when no
    // DAY_THEME force is active. Translucent-neutral fallback — alpha over the page ground reads
    // correctly in either scheme.
    bool filled = false;
    // …and an app Appearance override, for the same reason (see day_xaml_container_set_card).
    if (g_forced_theme == 0 && g_app_override == 0) {
        auto res = WUX::Application::Current().Resources();
        auto key = winrt::box_value(winrt::hstring(L"DividerStrokeColorDefaultBrush"));
        if (res.HasKey(key)) {
            if (auto brush = res.Lookup(key).try_as<WUXM::Brush>()) {
                b.Background(brush);
                filled = true;
            }
        }
    }
    if (!filled) b.Background(WUXM::SolidColorBrush(color_argb(0x33'808080u)));
    return boxh(b);
}

// ---- vector glyphs as real XAML geometry (docs/vectors.md) -------------------
//
// A `vector(…)` glyph draws as `Path` geometry, not as an image: the CLI converts the staged SVG
// into the shape list this parses (`build/day/vectors/xaml/<name>.xamlgeom`), so the glyph is
// resolution-INDEPENDENT — XAML rasterizes the geometry at whatever size the layout gives it,
// every frame, instead of scaling a 256 px cache. It is also what makes a tint a runtime
// composition: the color is a Brush set on the shape when it is realized, so one staged glyph
// serves every tint at every size. Art the CLI could not convert (gradients, clips, embedded
// rasters) stages no geometry, and the callers below fall back to the raster.

/// One shape parsed out of a `.xamlgeom` spec.
struct GeomShape {
    std::string data;
    unsigned int fill = 0;   // 0 = no fill
    bool even_odd = false;
    unsigned int stroke = 0; // 0 = no stroke
    double stroke_width = 0;
    std::string cap, join;
};
struct Geom {
    double w = 0, h = 0;
    std::vector<GeomShape> shapes;
};

// "#AARRGGBB" → packed ARGB; "-" (or anything unparsable) → 0, the "absent" encoding.
static unsigned int geom_color(const std::string& s) {
    if (s.size() != 9 || s[0] != '#') return 0;
    return static_cast<unsigned int>(std::strtoul(s.c_str() + 1, nullptr, 16));
}

static Geom parse_geom(const char* spec) {
    Geom g;
    if (!spec) return g;
    std::string text(spec);
    size_t pos = 0;
    while (pos <= text.size()) {
        size_t nl = text.find('\n', pos);
        std::string line = text.substr(pos, nl == std::string::npos ? std::string::npos : nl - pos);
        if (nl == std::string::npos) pos = text.size() + 1; else pos = nl + 1;
        if (line.empty()) continue;
        if (line[0] == 'V') {
            std::sscanf(line.c_str(), "V %lf %lf", &g.w, &g.h);
        } else if (line[0] == 'P') {
            // The data is everything after the tab, so it can hold spaces and commas untouched.
            size_t tab = line.find('\t');
            if (tab == std::string::npos) continue;
            GeomShape sh;
            sh.data = line.substr(tab + 1);
            char fill[32] = { 0 }, stroke[32] = { 0 }, cap[16] = { 0 }, join[16] = { 0 };
            int eo = 0;
            double w = 0;
            if (std::sscanf(line.substr(0, tab).c_str(), "P %31s %d %31s %lf %15s %15s",
                            fill, &eo, stroke, &w, cap, join) >= 4) {
                sh.fill = geom_color(fill);
                sh.even_odd = eo != 0;
                sh.stroke = geom_color(stroke);
                sh.stroke_width = w;
                sh.cap = cap;
                sh.join = join;
                g.shapes.push_back(std::move(sh));
            }
        }
    }
    return g;
}

// Build a Geometry from the emitted path data.
//
// Assembled object by object rather than handed to XamlReader as `<Path Data="…"/>` markup: the
// island's Application answers XAML type resolution with its own IXamlMetadataProvider, and
// under it XamlReader::Load fails to produce a Path at all — which is what silently dropped
// every glyph to its raster. Nothing is lost by not going through the parser, since day-vector
// emits a grammar this side already knows: absolute `M`/`L`/`C`/`Z` only, so an unrecognized
// command means the two ends disagree and the glyph is refused rather than drawn wrong.
static WUXM::Geometry geometry_from_data(const std::string& data, bool even_odd) {
    try {
        WUXM::PathGeometry geo;
        // SVG's default fill rule is nonzero; XAML's is even-odd. Always stated, never implied.
        geo.FillRule(even_odd ? WUXM::FillRule::EvenOdd : WUXM::FillRule::Nonzero);
        auto figures = geo.Figures();
        WUXM::PathFigure fig{ nullptr };
        const char* p = data.c_str();
        auto num = [&p]() -> float {
            while (*p == ' ' || *p == ',') ++p;
            char* end = nullptr;
            float v = std::strtof(p, &end);
            if (end) p = end;
            return v;
        };
        while (*p) {
            char cmd = *p;
            if (cmd == ' ' || cmd == ',') { ++p; continue; }
            ++p;
            if (cmd == 'M') {
                fig = WUXM::PathFigure();
                float x = num(), y = num();
                fig.StartPoint(WF::Point{ x, y });
                fig.IsFilled(true);
                figures.Append(fig);
            } else if (cmd == 'L') {
                if (!fig) return nullptr;
                WUXM::LineSegment s;
                float x = num(), y = num();
                s.Point(WF::Point{ x, y });
                fig.Segments().Append(s);
            } else if (cmd == 'C') {
                if (!fig) return nullptr;
                WUXM::BezierSegment s;
                float x1 = num(), y1 = num(), x2 = num(), y2 = num(), x3 = num(), y3 = num();
                s.Point1(WF::Point{ x1, y1 });
                s.Point2(WF::Point{ x2, y2 });
                s.Point3(WF::Point{ x3, y3 });
                fig.Segments().Append(s);
            } else if (cmd == 'Z' || cmd == 'z') {
                if (fig) fig.IsClosed(true);
            } else {
                return nullptr;
            }
        }
        if (figures.Size() == 0) return nullptr;
        return geo;
    } catch (...) {
        return nullptr;
    }
}

// Resolve template paint in each Path's theme, including live window overrides and
// high contrast. A style retains ThemeResource expressions rather than brush snapshots.
static void style_template_path(WUXSh::Path const& path, bool fill, bool stroke) {
    static WUX::Style styles[4] = {nullptr, nullptr, nullptr, nullptr};
    int mask = (fill ? 1 : 0) | (stroke ? 2 : 0);
    if (!styles[mask]) {
        std::wstring xaml = L"<Style xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\" TargetType=\"Path\">";
#ifdef DAY_WINUI
        auto brush = L"{ThemeResource TextFillColorPrimaryBrush}";
#else
        auto brush = L"{ThemeResource ApplicationForegroundThemeBrush}";
#endif
        for (auto property : {L"Fill", L"Stroke"}) {
            if ((property == std::wstring(L"Fill") && fill) ||
                (property == std::wstring(L"Stroke") && stroke)) {
                xaml += std::wstring(L"<Setter Property=\"") + property + L"\" Value=\"" + brush + L"\"/>";
            }
        }
        xaml += L"</Style>";
        styles[mask] = WUXMk::XamlReader::Load(xaml).as<WUX::Style>();
    }
    path.Style(styles[mask]);
}

// A glyph sized by the layout: the shapes go in a Canvas the size of the source viewport, and a
// Viewbox scales that to whatever frame day assigns — so one geometry serves every size.
// `tinted` 1 replaces every authored paint with `argb`; 2 paints them with the theme foreground
// (a template glyph with live theme resources); 3 is an explicitly tinted template;
// 0 keeps the art's own colors.
static UIElement vector_content(const char* spec, int mode, unsigned int argb, int tinted) {
    Geom g = parse_geom(spec);
    if (g.shapes.empty() || g.w <= 0 || g.h <= 0) return nullptr;
    try {
        WUXC::Canvas canvas;
        canvas.Width(g.w);
        canvas.Height(g.h);
        bool any = false;
        for (auto const& sh : g.shapes) {
            auto geo = geometry_from_data(sh.data, sh.even_odd);
            if (!geo) continue;
            WUXSh::Path p;
            p.Data(geo);
            if (tinted == 2) {
                style_template_path(p, sh.fill != 0, sh.stroke != 0);
            } else if (tinted) {
                // The tint composes over the geometry: fill where the art filled, stroke where
                // it stroked, so a glyph drawn as an outline stays an outline.
                auto paint = [argb](unsigned int authored) {
                    auto alpha = ((argb >> 24) * (authored >> 24) + 127) / 255;
                    return WUXM::SolidColorBrush(color_argb((argb & 0xffffff) | (alpha << 24)));
                };
                if (sh.fill) p.Fill(paint(sh.fill));
                if (sh.stroke) p.Stroke(paint(sh.stroke));
            } else {
                if (sh.fill) p.Fill(WUXM::SolidColorBrush(color_argb(sh.fill)));
                if (sh.stroke) p.Stroke(WUXM::SolidColorBrush(color_argb(sh.stroke)));
            }
            if (sh.stroke) {
                p.StrokeThickness(sh.stroke_width);
                if (sh.cap == "Round") p.StrokeStartLineCap(WUXM::PenLineCap::Round);
                else if (sh.cap == "Square") p.StrokeStartLineCap(WUXM::PenLineCap::Square);
                p.StrokeEndLineCap(p.StrokeStartLineCap());
                if (sh.join == "Round") p.StrokeLineJoin(WUXM::PenLineJoin::Round);
                else if (sh.join == "Bevel") p.StrokeLineJoin(WUXM::PenLineJoin::Bevel);
            }
            canvas.Children().Append(p);
            any = true;
        }
        if (!any) return nullptr;
        WUXC::Viewbox vb;
        vb.Child(canvas);
        // Matches the image content modes: 0=fit, 1=fill (crop), 2=stretch.
        vb.Stretch(mode == 2 ? WUXM::Stretch::Fill
                   : mode == 1 ? WUXM::Stretch::UniformToFill
                               : WUXM::Stretch::Uniform);
        return vb;
    } catch (...) {
        return nullptr;
    }
}

// A stable Day viewport supports source patches between vector and bitmap content. Center
// scaled vectors explicitly: Viewbox otherwise anchors its content at the origin. Constrain
// raster Images too, so UniformToFill cannot measure beyond the viewport and escape its crop.
struct ImageView {
    int mode = 0, tinted = 0;
    unsigned int argb = 0;
    bool templ = false;
    std::string spec;
    // The raster the view shows, so a tint can be composed over it at any time and outlive a
    // source swap (docs/images.md "Tinting"): the encoded bytes of a file or a byte source, or
    // the id of a held bitmap, whose bytes stay in the registry.
    std::shared_ptr<std::vector<uint8_t>> raster;
    uint64_t bitmap = 0;
};
static std::map<void*, std::shared_ptr<ImageView>> g_image_views;

static void layout_image(WUXC::Canvas const& host, int mode, double w, double h) {
    if (!std::isfinite(w) || !std::isfinite(h) || w < 0 || h < 0) return;
    WUXM::RectangleGeometry clip;
    clip.Rect({0, 0, static_cast<float>(w), static_cast<float>(h)});
    host.Clip(clip);
    if (!host.Children().Size()) return;
    auto child = host.Children().GetAt(0).as<FrameworkElement>();
    double cw = w, ch = h;
    if (auto vb = child.try_as<WUXC::Viewbox>()) {
        auto art = vb.Child().as<FrameworkElement>();
        if (mode != 2 && art.Width() > 0 && art.Height() > 0) {
            double sx = w / art.Width(), sy = h / art.Height();
            double scale = mode == 1 ? (std::max)(sx, sy) : (std::min)(sx, sy);
            cw = art.Width() * scale;
            ch = art.Height() * scale;
        }
        vb.Stretch(WUXM::Stretch::Fill);
    }
    child.Width(cw);
    child.Height(ch);
    WUXC::Canvas::SetLeft(child, (w - cw) / 2);
    WUXC::Canvas::SetTop(child, (h - ch) / 2);
}

static void replace_image(void* h, UIElement const& content) {
    if (!content) return;
    auto it = g_image_views.find(h);
    if (it == g_image_views.end()) return;
    auto host = elem(h).as<WUXC::Canvas>();
    host.Children().Clear();
    host.Children().Append(content);
    layout_image(host, it->second->mode, host.Width(), host.Height());
}

static void* image_view(UIElement const& content, int mode, const char* spec = "",
                        unsigned int argb = 0, int tinted = 0) {
    WUXC::Canvas host;
    host.Children().Append(content);
    auto state = std::make_shared<ImageView>();
    state->mode = mode;
    state->spec = spec;
    state->argb = argb;
    state->tinted = tinted;
    state->templ = tinted == 2 || tinted == 3;
    host.SizeChanged([mode](WF::IInspectable const& sender, WUX::SizeChangedEventArgs const& e) {
        layout_image(sender.as<WUXC::Canvas>(), mode, e.NewSize().Width, e.NewSize().Height);
    });
    void* h = boxh(host);
    g_image_views.emplace(h, state);
    return h;
}

void* day_xaml_vector_new(const char* spec, int mode, unsigned int argb, int tinted) {
    auto content = vector_content(spec, mode, argb, tinted);
    return content ? image_view(content, mode, spec, argb, tinted) : nullptr;
}

void day_xaml_image_set_vector(void* h, const char* spec) {
    auto it = g_image_views.find(h);
    if (it == g_image_views.end()) return;
    auto& s = *it->second;
    if (auto content = vector_content(spec, s.mode, s.argb, s.tinted)) {
        replace_image(h, content);
        s.spec = spec;
    }
}

extern "C++" { // defined with the raster section below
static UIElement raster_content(ImageView const& s);
} // extern "C++"

void day_xaml_image_set_tint(void* h, unsigned int argb, int tinted) {
    auto it = g_image_views.find(h);
    if (it == g_image_views.end()) return;
    auto& s = *it->second;
    s.argb = argb;
    s.tinted = tinted ? 1 : (s.templ ? 2 : 0);
    if (!s.spec.empty()) {
        replace_image(h, vector_content(s.spec.c_str(), s.mode, argb, s.tinted));
    } else {
        auto host = elem(h).as<WUXC::Canvas>();
        if (auto icon = host.Children().GetAt(0).try_as<WUXC::BitmapIcon>()) {
            icon.ShowAsMonochrome(s.tinted != 0);
            if (tinted) icon.Foreground(WUXM::SolidColorBrush(color_argb(argb)));
            else icon.ClearValue(WUXC::IconElement::ForegroundProperty());
        } else {
            // A raster: show it again, recolored or as authored.
            replace_image(h, raster_content(s));
        }
    }
}

/// A tinted image's tint color again, its tint mode kept (the appearance changed under a tint
/// that follows it, docs/color.md). An untinted image is left as it is.
void day_xaml_image_retint(void* h, unsigned int argb) {
    auto it = g_image_views.find(h);
    if (it == g_image_views.end()) return;
    auto& s = *it->second;
    if (s.tinted != 1 && s.tinted != 3) return;
    guard([&] {
        s.argb = argb;
        if (!s.spec.empty()) {
            replace_image(h, vector_content(s.spec.c_str(), s.mode, argb, s.tinted));
        } else {
            auto host = elem(h).as<WUXC::Canvas>();
            if (auto icon = host.Children().GetAt(0).try_as<WUXC::BitmapIcon>()) {
                icon.Foreground(WUXM::SolidColorBrush(color_argb(argb)));
            } else {
                replace_image(h, raster_content(s));
            }
        }
    });
}

#include "vector-icon.inc"

// The same geometry as a `PathIcon`, for the slots that demand an IconElement (nav rows).
// PathIcon draws its geometry in its OWN coordinates with no scaling of its own, so the source
// viewport is mapped onto `box` here; an untinted icon leaves Foreground alone and inherits the
// pane's theme color, which is what keeps unstyled rows theme-adaptive.
void* day_xaml_vector_icon_new(const char* spec, unsigned int argb, int tinted, double box) {
    Geom g = parse_geom(spec);
    if (g.shapes.empty() || g.w <= 0 || g.h <= 0) return nullptr;
    try {
        // PathIcon fills its Data; a stroked SVG path is a centerline, not a filled outline.
        // Expand strokes and union each painted area, preserving holes and per-shape fill rules.
        auto geo = icon_painted_geometry(g);
        if (!geo) return nullptr;
        double s = box / (g.w > g.h ? g.w : g.h);
        WUXM::ScaleTransform st;
        st.ScaleX(s);
        st.ScaleY(s);
        geo.Transform(st);
        WUXC::PathIcon icon;
        icon.Data(geo);
        if (tinted) icon.Foreground(WUXM::SolidColorBrush(color_argb(argb)));
        return boxh(icon);
    } catch (...) {
        return nullptr;
    }
}

// The raster fallback for art the CLI could not convert to geometry (gradients, clips, embedded
// rasters): ShowAsMonochrome turns the staged PNG into an alpha mask that Foreground fills.
// Lossier than the geometry path above — it tints a 256 px cache, not the glyph — which is why
// it is reached only when there is no geometry to draw. `icon_file` is the staged file NAME
// (BitmapIcon takes only a Uri, and unpackaged islands resolve `ms-appx:///images/<file>`
// against the exe directory); an empty name or `tinted` 0 falls through to a plain Image. A
// template (`tinted` 2) leaves Foreground alone: the icon inherits the theme foreground, as the
// nav pane's untinted rows do.
void* day_xaml_image_tinted_new(const char* icon_file, int mode, unsigned int argb, int tinted) {
    if (icon_file && *icon_file && tinted != 0) {
        WUXC::BitmapIcon bicon;
        bicon.UriSource(WF::Uri{ hs((std::string("ms-appx:///images/") + icon_file).c_str()) });
        bicon.ShowAsMonochrome(true);
        if (tinted != 2) bicon.Foreground(WUXM::SolidColorBrush(color_argb(argb)));
        return image_view(bicon, mode, "", argb, tinted);
    }
    return nullptr;
}

// ---- Raster images from bytes (docs/images.md) ---------------------------------------------
//
// Decoded bitmaps, keyed by the id day-core minted. XAML has no cheap decoded-bitmap type to
// hold across threads — a `BitmapImage` belongs to the UI thread and decodes lazily — so what is
// kept is the ENCODED buffer, and each consumer builds its own `BitmapImage` from it. The
// registry still owns the lifetime: entries leave only through `day_xaml_image_release`.
static std::map<uint64_t, std::vector<uint8_t>>& xaml_bitmaps() {
    static std::map<uint64_t, std::vector<uint8_t>> m;
    return m;
}

/// A random-access stream over `bytes`, which XAML's image loader needs because it accepts
/// neither `file://` nor a bare path. StoreAsync().get() commits the in-memory buffer (it
/// completes on a pool thread, so there is no UI deadlock).
static WSS::IRandomAccessStream bytes_stream(const uint8_t* bytes, uint32_t len) {
    try {
        if (!bytes || len == 0) return nullptr;
        WSS::InMemoryRandomAccessStream stream;
        WSS::DataWriter writer(stream);
        writer.WriteBytes(winrt::array_view<uint8_t const>(bytes, bytes + len));
        writer.StoreAsync().get();
        writer.DetachStream();
        stream.Seek(0);
        return stream;
    } catch (...) {
        return nullptr;
    }
}

/// A `BitmapImage` over encoded bytes, or null.
static WUXM::Imaging::BitmapImage bitmap_from_bytes(const uint8_t* bytes, uint32_t len) {
    try {
        auto stream = bytes_stream(bytes, len);
        if (!stream) return nullptr;
        WUXM::Imaging::BitmapImage bmp;
        bmp.SetSource(stream);
        return bmp;
    } catch (...) {
        return nullptr;
    }
}

/// Hold `bytes` under `id`. Returns 1 always — XAML decodes lazily when the image is first
/// shown, so there is nothing to validate here (docs/images.md records the consequence: this
/// backend cannot report pixel dimensions at decode time).
int day_xaml_image_decode(unsigned long long id, const unsigned char* bytes, int len) {
    if (!bytes || len <= 0) return 0;
    xaml_bitmaps()[id] = std::vector<uint8_t>(bytes, bytes + len);
    return 1;
}

/// The encoded bytes `id` holds, copied into `out` (at most `cap`); returns the full length so a
/// caller can tell whether it fitted. 0 when the id names nothing.
int day_xaml_image_bytes(unsigned long long id, unsigned char* out, int cap) {
    auto it = xaml_bitmaps().find(id);
    if (it == xaml_bitmaps().end()) return 0;
    const int len = (int)it->second.size();
    if (out && cap > 0) {
        const int n = cap < len ? cap : len;
        memcpy(out, it->second.data(), (size_t)n);
    }
    return len;
}

/// Drop a decoded bitmap; day-core calls this when the app's last handle goes.
void day_xaml_image_release(unsigned long long id) {
    xaml_bitmaps().erase(id);
#ifdef DAY_WINUI
    canvas_images.erase(id);
#endif
}

// ---- Raster tint (docs/images.md "Tinting") -------------------------------------------------
//
// A tinted raster is a template image: every pixel takes the tint's color and keeps its own
// alpha (source-in). Win2D's `TintEffect` multiplies, which leaves a black pixel black, so the
// recolor is a `ColorMatrixEffect` that drops the source color, adds the tint's, and scales
// alpha by the tint's. It is rendered once into the `WriteableBitmap` the XAML `Image` shows.

#ifdef DAY_WINUI
extern "C++" {
/// `bytes` decoded and recolored to `argb`, or null when they do not decode.
static WUXM::ImageSource tinted_bitmap(std::vector<uint8_t> const& bytes, unsigned int argb) {
    try {
        auto pixels = canvas_decode(bytes);
        const float a = ((argb >> 24) & 0xff) / 255.0f, r = ((argb >> 16) & 0xff) / 255.0f;
        const float g = ((argb >> 8) & 0xff) / 255.0f, b = (argb & 0xff) / 255.0f;
        try {
            auto device = canvas_device();
            auto source = WC::CanvasBitmap::CreateFromBytes(device, pixels.bytes, pixels.width, pixels.height,
                winrt::Windows::Graphics::DirectX::DirectXPixelFormat::B8G8R8A8UIntNormalized, 96,
                WC::CanvasAlphaMode::Premultiplied);
            WCE::ColorMatrixEffect recolor;
            recolor.Source(source);
            WCE::Matrix5x4 m{};
            m.M44 = a; m.M51 = r; m.M52 = g; m.M53 = b;
            recolor.ColorMatrix(m);
            WC::CanvasRenderTarget target(device, float(pixels.width), float(pixels.height), 96);
            auto ds = target.CreateDrawingSession();
            ds.Clear(color_argb(0));
            ds.DrawImage(recolor);
            ds.Close();
            auto out = target.GetPixelBytes();
            if (out.size() != pixels.bytes.size()) winrt::throw_hresult(E_FAIL);
            pixels.bytes.assign(out.begin(), out.end());
        } catch (...) {
            // No Direct3D device to run the effect on: the same arithmetic on the decoded
            // (premultiplied BGRA) pixels.
            for (size_t i = 0; i + 3 < pixels.bytes.size(); i += 4) {
                const float alpha = pixels.bytes[i + 3] * a;
                pixels.bytes[i] = static_cast<uint8_t>(std::lround(b * alpha));
                pixels.bytes[i + 1] = static_cast<uint8_t>(std::lround(g * alpha));
                pixels.bytes[i + 2] = static_cast<uint8_t>(std::lround(r * alpha));
                pixels.bytes[i + 3] = static_cast<uint8_t>(std::lround(alpha));
            }
        }
        WUXM::Imaging::WriteableBitmap bitmap(static_cast<int32_t>(pixels.width), static_cast<int32_t>(pixels.height));
        auto buffer = bitmap.PixelBuffer();
        if (buffer.Length() < pixels.bytes.size()) return nullptr;
        memcpy(buffer.data(), pixels.bytes.data(), pixels.bytes.size());
        bitmap.Invalidate();
        return bitmap;
    } catch (...) {
        return nullptr;
    }
}
} // extern "C++"
#endif

extern "C++" {
/// The bytes of the raster a view shows, or null when it shows none it can read again (a
/// vector, a remote image, a released bitmap).
static std::vector<uint8_t> const* raster_bytes(ImageView const& s) {
    if (s.raster) return s.raster.get();
    auto it = xaml_bitmaps().find(s.bitmap);
    return it == xaml_bitmaps().end() ? nullptr : &it->second;
}

/// An `Image` over the view's raster, recolored when the view is tinted. Null when there are
/// no bytes or they do not load, which leaves the current content in place.
static UIElement raster_content(ImageView const& s) {
    auto bytes = raster_bytes(s);
    if (!bytes || bytes->empty()) return nullptr;
    WUXM::ImageSource source{nullptr};
#ifdef DAY_WINUI
    if (s.tinted == 1) source = tinted_bitmap(*bytes, s.argb);
#endif
    if (!source) source = bitmap_from_bytes(bytes->data(), static_cast<uint32_t>(bytes->size()));
    if (!source) return nullptr;
    WUXC::Image img;
    img.Stretch(s.mode == 2 ? WUXM::Stretch::Fill
                : s.mode == 1 ? WUXM::Stretch::UniformToFill : WUXM::Stretch::Uniform);
    img.Source(source);
    return img;
}

/// The bytes of a local file, or null. The system-XAML BitmapImage can't load a `file://` or
/// bare-path Uri (a UWP restriction that carries into XAML Islands), so a bundled image is
/// read here and fed as a stream via SetSource. `path` is a UTF-8 native path, opened wide so
/// Unicode paths work.
static std::shared_ptr<std::vector<uint8_t>> read_file_bytes(const char* path) {
    try {
        if (!path || !*path) return nullptr;
        std::ifstream f(hs(path).c_str(), std::ios::binary);
        if (!f) return nullptr;
        auto bytes = std::make_shared<std::vector<uint8_t>>((std::istreambuf_iterator<char>(f)),
                                                            std::istreambuf_iterator<char>());
        return bytes->empty() ? nullptr : bytes;
    } catch (...) {
        return nullptr;
    }
}

static bool is_remote_uri(const char* uri) {
    std::string s = uri ? uri : "";
    return s.rfind("http://", 0) == 0 || s.rfind("https://", 0) == 0;
}

/// A new viewport over a raster: `raster` or the held bitmap `id`. An empty `Image` when
/// neither loads, which is what an unresolved source has always shown.
static void* raster_view(std::shared_ptr<std::vector<uint8_t>> raster, uint64_t id, int mode) {
    ImageView s;
    s.mode = mode;
    s.raster = std::move(raster);
    s.bitmap = id;
    auto content = raster_content(s);
    if (!content) {
        WUXC::Image img;
        img.Stretch(mode == 2 ? WUXM::Stretch::Fill
                    : mode == 1 ? WUXM::Stretch::UniformToFill : WUXM::Stretch::Uniform);
        content = img;
    }
    void* h = image_view(content, mode);
    auto& state = *g_image_views[h];
    state.raster = s.raster;
    state.bitmap = s.bitmap;
    return h;
}

/// Replace content inside the stable viewport, including a previous vector, keeping the
/// view's tint. A source that cannot load leaves the current content in place.
static void raster_set(void* w, std::shared_ptr<std::vector<uint8_t>> raster, uint64_t id) {
    try {
        auto it = g_image_views.find(w);
        if (it == g_image_views.end()) return;
        auto& s = *it->second;
        ImageView next = s;
        next.raster = std::move(raster);
        next.bitmap = id;
        auto content = raster_content(next);
        if (!content) return;
        replace_image(w, content);
        s.raster = next.raster;
        s.bitmap = next.bitmap;
        s.spec.clear();
    } catch (...) {}
}
} // extern "C++"

/// An `Image` element showing raw encoded bytes (an `ImageSource::Bytes` realize).
void* day_xaml_image_bytes_new(const unsigned char* bytes, int len, int mode) {
    std::shared_ptr<std::vector<uint8_t>> raster;
    if (bytes && len > 0) raster = std::make_shared<std::vector<uint8_t>>(bytes, bytes + len);
    return raster_view(raster, 0, mode);
}

/// An `Image` element showing a bitmap already held under `id` (an `ImageSource::Decoded`).
void* day_xaml_image_bitmap_new(unsigned long long id, int mode) {
    return raster_view(nullptr, id, mode);
}

/// A `BitmapImage` over an http(s) URI, or null. A remote image loads asynchronously inside
/// XAML, so there are no bytes here to recolor: it is shown untinted.
static WUXM::Imaging::BitmapImage bitmap_from_remote(const char* uri) {
    try {
        WUXM::Imaging::BitmapImage bmp;
        bmp.UriSource(WF::Uri{ hs(uri) });
        return bmp;
    } catch (...) {
        return nullptr;
    }
}

void day_xaml_image_set_uri(void* w, const char* uri) {
    if (!is_remote_uri(uri)) {
        // A local bundled file: feed the bytes, not a file:// Uri (see `read_file_bytes`).
        if (auto raster = read_file_bytes(uri)) raster_set(w, raster, 0);
        return;
    }
    try {
        auto it = g_image_views.find(w);
        auto bmp = bitmap_from_remote(uri);
        if (it == g_image_views.end() || !bmp) return;
        auto& s = *it->second;
        WUXC::Image img;
        img.Stretch(s.mode == 2 ? WUXM::Stretch::Fill
                    : s.mode == 1 ? WUXM::Stretch::UniformToFill : WUXM::Stretch::Uniform);
        img.Source(bmp);
        replace_image(w, img);
        s.spec.clear();
        s.raster = nullptr;
        s.bitmap = 0;
    } catch (...) {}
}
void day_xaml_image_set_bytes(void* w, const unsigned char* bytes, int len) {
    if (!bytes || len <= 0) return;
    raster_set(w, std::make_shared<std::vector<uint8_t>>(bytes, bytes + len), 0);
}
void day_xaml_image_set_bitmap(void* w, unsigned long long id) {
    if (xaml_bitmaps().find(id) == xaml_bitmaps().end()) return;
    raster_set(w, nullptr, id);
}

void* day_xaml_image_new(const char* uri, int mode) {
    // Scaling (§18.3): 0=fit (Uniform), 1=fill (UniformToFill, cropped), 2=stretch (Fill).
    if (is_remote_uri(uri)) {
        WUXC::Image img;
        img.Stretch(mode == 2 ? WUXM::Stretch::Fill
                    : mode == 1 ? WUXM::Stretch::UniformToFill
                                : WUXM::Stretch::Uniform);
        if (auto bmp = bitmap_from_remote(uri)) img.Source(bmp);
        return image_view(img, mode);
    }
    return raster_view(read_file_bytes(uri), 0, mode);
}
