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

// Read a local file's bytes into an in-memory stream. The system-XAML BitmapImage can't load a
// `file://` / bare-path Uri (a UWP restriction that carries into XAML Islands), so bundled images
// must be fed as a stream via SetSource. `path` is a UTF-8 native path; open it wide so Unicode
// paths work. StoreAsync().get() commits the in-memory buffer (completes on a pool thread — no UI
// deadlock). Returns null on any failure.
static WSS::IRandomAccessStream read_file_stream(const char* path) {
    try {
        std::ifstream f(hs(path).c_str(), std::ios::binary); // hstring::c_str() is wchar_t* (MSVC)
        if (!f) return nullptr;
        std::vector<uint8_t> bytes((std::istreambuf_iterator<char>(f)),
                                   std::istreambuf_iterator<char>());
        if (bytes.empty()) return nullptr;
        WSS::InMemoryRandomAccessStream stream;
        WSS::DataWriter writer(stream);
        writer.WriteBytes(winrt::array_view<uint8_t const>(bytes.data(), bytes.data() + bytes.size()));
        writer.StoreAsync().get();
        writer.DetachStream();
        stream.Seek(0);
        return stream;
    } catch (...) {
        return nullptr;
    }
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
        }
    }
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

/// A random-access stream over `bytes` — the in-memory twin of `read_file_stream`, which XAML's
/// image loader needs because it accepts neither `file://` nor a bare path.
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

/// An `Image` element showing raw encoded bytes (an `ImageSource::Bytes` realize).
void* day_xaml_image_bytes_new(const unsigned char* bytes, int len, int mode) {
    WUXC::Image img;
    img.Stretch(mode == 2 ? WUXM::Stretch::Fill
                : mode == 1 ? WUXM::Stretch::UniformToFill
                            : WUXM::Stretch::Uniform);
    if (auto bmp = bitmap_from_bytes(bytes, len > 0 ? (uint32_t)len : 0)) img.Source(bmp);
    return image_view(img, mode);
}

/// An `Image` element showing a bitmap already held under `id` (an `ImageSource::Decoded`).
void* day_xaml_image_bitmap_new(unsigned long long id, int mode) {
    auto it = xaml_bitmaps().find(id);
    if (it == xaml_bitmaps().end()) return day_xaml_image_bytes_new(nullptr, 0, mode);
    return day_xaml_image_bytes_new(it->second.data(), (int)it->second.size(), mode);
}

/// A `BitmapImage` over a resolved native path or an http(s) URI, or null: the loader behind
/// `day_xaml_image_new`, shared with the source swap so both load exactly the same way.
static WUXM::Imaging::BitmapImage bitmap_from_uri(const char* uri) {
    if (!uri || !*uri) return nullptr;
    try {
        WUXM::Imaging::BitmapImage bmp;
        std::string s = uri;
        if (s.rfind("http://", 0) == 0 || s.rfind("https://", 0) == 0) {
            bmp.UriSource(WF::Uri{ hs(uri) }); // remote — BitmapImage loads http(s) directly
        } else if (auto stream = read_file_stream(uri)) {
            bmp.SetSource(stream); // local bundled file — feed the bytes, not a file:// Uri
        } else {
            return nullptr;
        }
        return bmp;
    } catch (...) {
        return nullptr;
    }
}

/// Replace content inside the stable viewport, including a previous vector. A source that
/// cannot load leaves the current content in place.
static void image_set(void* w, WUXM::Imaging::BitmapImage const& bmp) {
    if (!bmp) return;
    try {
        auto it = g_image_views.find(w);
        if (it == g_image_views.end()) return;
        auto& s = *it->second;
        WUXC::Image img;
        img.Stretch(s.mode == 2 ? WUXM::Stretch::Fill
                    : s.mode == 1 ? WUXM::Stretch::UniformToFill : WUXM::Stretch::Uniform);
        img.Source(bmp);
        replace_image(w, img);
        s.spec.clear();
    } catch (...) {}
}
void day_xaml_image_set_uri(void* w, const char* uri) { image_set(w, bitmap_from_uri(uri)); }
void day_xaml_image_set_bytes(void* w, const unsigned char* bytes, int len) {
    image_set(w, bitmap_from_bytes(bytes, len > 0 ? (uint32_t)len : 0));
}
void day_xaml_image_set_bitmap(void* w, unsigned long long id) {
    auto it = xaml_bitmaps().find(id);
    if (it == xaml_bitmaps().end()) return;
    image_set(w, bitmap_from_bytes(it->second.data(), (uint32_t)it->second.size()));
}

void* day_xaml_image_new(const char* uri, int mode) {
    WUXC::Image img;
    // Scaling (§18.3): 0=fit (Uniform), 1=fill (UniformToFill, cropped), 2=stretch (Fill).
    img.Stretch(mode == 2 ? WUXM::Stretch::Fill
                : mode == 1 ? WUXM::Stretch::UniformToFill
                            : WUXM::Stretch::Uniform);
    if (auto bmp = bitmap_from_uri(uri)) img.Source(bmp);
    return image_view(img, mode);
}

