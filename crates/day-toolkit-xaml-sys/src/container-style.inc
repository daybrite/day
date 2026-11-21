// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Included by shim.cpp inside the C ABI block. Shared declarations and include order live there.

// A container is a Canvas (day positions children by absolute frame). It has no rounded-corner clip
// of its own — Windows.UI.Xaml's RectangleGeometry can't round, and UIElement.Clip is rectangular
// only — so a background fill / rounded corner is drawn by a `Rectangle` SHAPE (which does carry
// RadiusX/RadiusY) kept as child[0], behind day's children, tracking the Canvas size via SizeChanged.
static WUXSh::Rectangle ensure_bg_rect(WUXC::Canvas const& canvas) {
    auto kids = canvas.Children();
    if (kids.Size() > 0)
        if (auto r = kids.GetAt(0).try_as<WUXSh::Rectangle>())
            if (r.Name() == L"day_bg") return r;
    WUXSh::Rectangle rect;
    rect.Name(L"day_bg");
    // A fill takes no input (docs/gestures.md "Pass-through"): presses on a container with
    // nothing attached reach what is behind it. A container that does take input is given a
    // transparent background of its own by `day_hit_on`.
    rect.IsHitTestVisible(false);
    WUXC::Canvas::SetLeft(rect, 0);
    WUXC::Canvas::SetTop(rect, 0);
    rect.Width(canvas.ActualWidth());
    rect.Height(canvas.ActualHeight());
    // day sizes the container via set_geometry (Width/Height), which fires SizeChanged; grow with it.
    canvas.SizeChanged([rect](WF::IInspectable const&, WUX::SizeChangedEventArgs const& e) mutable {
        rect.Width(e.NewSize().Width);
        rect.Height(e.NewSize().Height);
    });
    kids.InsertAt(0, rect);
    return rect;
}

// ---- pass-through (docs/gestures.md "Pass-through") ----
//
// A piece Day realized takes pointer input only once something is attached that needs it: a
// recognizer, a context menu, a drag or a drop, focus, selectable text or a link. Until then a
// press, a hover or a wheel over it reaches what is behind it. A container (a Canvas) does this
// by itself: a panel with no background is hit only on its children, and the fills Day draws
// inside it take no input. A canvas's host keeps its surface background null for the same
// reason. Labels, images and dividers are made passive outright (`IsHitTestVisible`).
extern "C++" {
static std::set<void*> g_passive;

/// The node `h` starts taking pointer input over its whole box.
static void day_hit_on(void* h) {
    auto el = elem(h);
    if (g_passive.erase(h)) el.IsHitTestVisible(true);
    WUXC::Panel panel = el.try_as<WUXC::Panel>();
    if (!panel)
        if (auto host = el.try_as<WUXC::ContentControl>()) panel = host.Content().try_as<WUXC::Panel>();
    if (panel && !panel.Background()) panel.Background(WUXM::SolidColorBrush(color_argb(0)));
}
} // extern "C++"

/// A label, an image or a divider: no pointer input until `day_xaml_hit_on`.
void day_xaml_set_passive(void* h) {
    guard([&] {
        elem(h).IsHitTestVisible(false);
        g_passive.insert(h);
    });
}

/// The node takes pointer input from now on (a cover over the content, a list cell).
void day_xaml_hit_on(void* h) {
    guard([&] { day_hit_on(h); });
}

// ---- styled surfaces (docs/surfaces.md) ----
//
// A surface with a border, a gradient, a shadow or a corner selection is drawn by a `Border`
// element instead of the `Rectangle` above: `Border` is what carries per-corner radii
// (`CornerRadius`) and any brush as its background. Like the rectangle it is child[0] of the
// container's Canvas, a sibling of day's children and sized to the Canvas. The border line is a
// second, brushless `Border` over the children (`Canvas.ZIndex` above them, no hit testing), so
// a child that reaches the edge never covers it (docs/surfaces.md "Rules"), and its thickness
// never moves or shrinks what day has placed.
//
// The shadow is a Composition `DropShadow` (not `ThemeShadow`, which offers no color, blur or
// offset): a brushless `SpriteVisual` the size of the surface, masked to its rounded outline,
// hung on an empty host element placed beneath the `Border`. A child visual takes no layout
// room and no input.

extern "C++" { // helpers that return C++ types
/// The outline of a surface as a Composition geometry: the rounded-rectangle geometry when the
/// corners are uniform, a path when they are not.
struct SurfaceOutline {
    WUComp::CompositionRoundedRectangleGeometry round{nullptr};
    WUComp::CompositionPathGeometry path{nullptr};

    WUComp::CompositionGeometry geometry() const {
        if (path) return path;
        return round;
    }
};

/// `radius` on the corners in `corners` (1 top-left, 2 top-right, 4 bottom-right, 8
/// bottom-left) and 0 on the rest, never more than half the shorter side.
static std::array<float, 4> surface_radii(float w, float h, float radius, int corners) {
    const float r = (std::max)(0.0f, (std::min)(radius, (std::min)(w, h) / 2));
    return {corners & 1 ? r : 0, corners & 2 ? r : 0, corners & 4 ? r : 0, corners & 8 ? r : 0};
}

#ifdef DAY_WINUI
/// The rounded rectangle with per-corner radii as a Composition path. Composition has no such
/// geometry of its own; Win2D builds the path it takes.
static WUComp::CompositionPath surface_path(float w, float h, float radius, int corners) {
    const auto [tl, tr, br, bl] = surface_radii(w, h, radius, corners);
    WCG::CanvasPathBuilder b(canvas_device());
    const auto turn = [&b](float x, float y, float r) {
        if (r > 0) b.AddArc({x, y}, r, r, 0, WCG::CanvasSweepDirection::Clockwise, WCG::CanvasArcSize::Small);
    };
    b.BeginFigure({tl, 0});
    b.AddLine({w - tr, 0}); turn(w, tr, tr);
    b.AddLine({w, h - br}); turn(w - br, h, br);
    b.AddLine({bl, h});     turn(0, h - bl, bl);
    b.AddLine({0, tl});     turn(tl, 0, tl);
    b.EndFigure(WCG::CanvasFigureLoop::Closed);
    return WUComp::CompositionPath(WCG::CanvasGeometry::CreatePath(b));
}
#endif

static void outline_resize(SurfaceOutline& o, float w, float h, float radius, int corners) {
#ifdef DAY_WINUI
    if (o.path) {
        try { o.path.Path(surface_path(w, h, radius, corners)); } catch (...) {}
        return;
    }
#endif
    if (o.round) {
        const float r = surface_radii(w, h, radius, 15)[0];
        o.round.CornerRadius({r, r});
        o.round.Size({w, h});
    }
}

static SurfaceOutline outline_new(WUComp::Compositor const& compositor, float w, float h,
                                  float radius, int corners) {
    SurfaceOutline o;
#ifdef DAY_WINUI
    // A selection of corners needs the path. If Win2D cannot be reached the outline falls back
    // to the uniform geometry, which rounds all four.
    if ((corners & 15) != 15 && radius > 0) {
        try { o.path = compositor.CreatePathGeometry(surface_path(w, h, radius, corners)); } catch (...) {}
    }
#endif
    if (!o.path) o.round = compositor.CreateRoundedRectangleGeometry();
    outline_resize(o, w, h, radius, corners);
    return o;
}

// Every rounded CLIP this shim installed on a plain container, by the handle that owns it.
// `day_xaml_set_geometry` is where day states a container's real size, and the clip has to
// follow it from there: the `SizeChanged` hook in `day_xaml_container_set_corner` is not enough
// on its own, because a Canvas given an explicit Width and Height by day does not always raise
// it — and a clip left at its birth size of 0x0 hides the whole subtree, which is how a rounded
// `vector(…)` mark rendered as nothing at all.
static std::map<void*, WUComp::CompositionRoundedRectangleGeometry> g_clip_geometry;

struct SurfaceState {
    float radius = 0;
    int corners = 15;
    bool clips = false;
    // The size day last stated for the surface.
    float w = 0, h = 0;
    WUXC::Border box{nullptr};
    // The border line, over the children; null without a border.
    WUXC::Border edge{nullptr};
    double edge_width = 0;
    SurfaceOutline clip;
    // The shadow: its visual and effect, the visual surface its mask is read from, and the
    // mask's outline.
    WUComp::SpriteVisual shadow{nullptr};
    WUComp::DropShadow drop{nullptr};
    WUComp::ShapeVisual mask_visual{nullptr};
    WUComp::CompositionVisualSurface mask_surface{nullptr};
    SurfaceOutline mask;
#ifdef DAY_WINUI
    // A conic fill: no XAML brush draws one, so Win2D paints it (the canvas's own wedge fan)
    // into an image the box shows, again at each new size. Empty: the fill is something else.
    std::vector<WCB::CanvasGradientStop> conic;
    float conic_x = 0.5f, conic_y = 0.5f, conic_start = 0;
    float conic_w = -1, conic_h = -1; // the size last painted
#endif
};
static std::map<void*, std::shared_ptr<SurfaceState>> g_surfaces;

#ifdef DAY_WINUI
/// Paint a conic fill at `w` × `h` points into an image brush on the box, at the window's
/// pixel scale (twice the points before the box is in a window).
static void surface_paint_conic(SurfaceState& s, float w, float h) {
    if (s.conic.empty() || !s.box || !(w >= 1) || !(h >= 1)) return;
    if (w == s.conic_w && h == s.conic_h) return;
    try {
        float scale = 2;
        if (auto root = s.box.XamlRoot()) scale = static_cast<float>(root.RasterizationScale());
        auto device = canvas_device();
        WCX::CanvasImageSource image(device, w, h, 96.0f * scale);
        {
            auto session = image.CreateDrawingSession(color_argb(0));
            auto region = WCG::CanvasGeometry::CreateRectangle(device, 0, 0, w, h);
            canvas_conic(session, region, WF::Rect{0, 0, w, h}, s.conic_x, s.conic_y, s.conic_start,
                         s.conic);
            session.Close();
        }
        WUXM::ImageBrush brush;
        brush.ImageSource(image);
        brush.Stretch(WUXM::Stretch::Fill);
        s.box.Background(brush);
        s.conic_w = w;
        s.conic_h = h;
    } catch (...) {
    }
}
#endif

/// The surface's fill is no longer a conic gradient.
static void surface_clear_conic([[maybe_unused]] SurfaceState& s) {
#ifdef DAY_WINUI
    s.conic.clear();
    s.conic_w = s.conic_h = -1;
#endif
}

// A surface follows the size day states for it: from `day_xaml_set_geometry` directly, and from
// `SizeChanged` for the cases that never pass through there.
static void surface_resize(SurfaceState& s, double width, double height) {
    if (!std::isfinite(width) || !std::isfinite(height) || width < 0 || height < 0) return;
    const float w = static_cast<float>(width), h = static_cast<float>(height);
    s.w = w;
    s.h = h;
    if (s.box) {
        s.box.Width(width);
        s.box.Height(height);
    }
    if (s.edge) {
        s.edge.Width(width);
        s.edge.Height(height);
    }
    outline_resize(s.clip, w, h, s.radius, s.corners);
    outline_resize(s.mask, w, h, s.radius, s.corners);
    if (s.mask_visual) s.mask_visual.Size({w, h});
    if (s.mask_surface) s.mask_surface.SourceSize({w, h});
    if (s.shadow) s.shadow.Size({w, h});
#ifdef DAY_WINUI
    surface_paint_conic(s, w, h);
#endif
}

// The corners. The box has no border of its own, so its `CornerRadius` is the frame's.
// `Border.CornerRadius` rounds the MIDDLE of a border line: the outer edge comes out half the
// thickness larger and the inner edge half smaller. Day's radius is the frame's, the outer
// edge, so the border line is given the radius less half its thickness. (A radius smaller than
// half the border has no such value and is drawn square.)
static void surface_round_box(SurfaceState const& s) {
    const auto radii = [&](double r) {
        r = (std::max)(r, 0.0);
        const auto on = [&](int bit) { return (s.corners & bit) ? r : 0.0; };
        return WUX::CornerRadius{on(1), on(2), on(4), on(8)};
    };
    s.box.CornerRadius(radii(s.radius));
    if (s.edge) s.edge.CornerRadius(radii(double(s.radius) - s.edge_width / 2));
}

static std::shared_ptr<SurfaceState> surface_of(void* h) {
    auto it = g_surfaces.find(h);
    return it == g_surfaces.end() ? nullptr : it->second;
}

/// Shape the clip over the children and the shadow's mask to the surface's corners now: a
/// clip only when the surface clips and has a radius.
static void surface_reshape(void* h, SurfaceState& s) {
    auto c = elem(h).try_as<WUXC::Canvas>();
    if (!c) return;
    const bool clip = s.clips && s.radius > 0;
    // The container's Composition visual is asked for only to clip (or to stop clipping).
    try {
        if (clip || s.clip.geometry()) {
            // A COMPOSITION clip, as in `day_xaml_container_set_corner`: `UIElement.Clip` is a
            // plain rectangle.
            auto visual = WUXH::ElementCompositionPreview::GetElementVisual(c);
            auto compositor = visual.Compositor();
            if (clip) {
                s.clip = outline_new(compositor, s.w, s.h, s.radius, s.corners);
                visual.Clip(compositor.CreateGeometricClip(s.clip.geometry()));
            } else {
                visual.Clip(nullptr);
                s.clip = SurfaceOutline{};
            }
        }
        if (s.mask_visual) {
            auto compositor = s.mask_visual.Compositor();
            s.mask = outline_new(compositor, s.w, s.h, s.radius, s.corners);
            auto shape = compositor.CreateSpriteShape(s.mask.geometry());
            shape.FillBrush(compositor.CreateColorBrush(color_argb(0xFF'000000u)));
            s.mask_visual.Shapes().Clear();
            s.mask_visual.Shapes().Append(shape);
        }
    } catch (...) {
        s.clip = SurfaceOutline{};
    }
}

} // extern "C++"

/// Make the container a styled surface, or reshape the one it is: `radius` on the corners in
/// `corners` (the `Corners::bits` set), and a clip of that shape over the children when `clips`.
/// A plain container's fill rectangle and rounded clip give way to the surface's.
void day_xaml_container_set_surface(void* h, double radius, int corners, int clips) {
    guard([&] {
        auto c = elem(h).try_as<WUXC::Canvas>();
        if (!c) return;
        auto s = surface_of(h);
        if (!s) {
            s = std::make_shared<SurfaceState>();
            auto kids = c.Children();
            if (kids.Size() > 0)
                if (auto r = kids.GetAt(0).try_as<WUXSh::Rectangle>())
                    if (r.Name() == L"day_bg") kids.RemoveAt(0);
            if (g_clip_geometry.erase(h)) {
                try {
                    WUXH::ElementCompositionPreview::GetElementVisual(c).Clip(nullptr);
                } catch (...) {
                }
            }
            WUXC::Border box;
            box.Name(L"day_surface");
            box.IsHitTestVisible(false);
            WUXC::Canvas::SetLeft(box, 0);
            WUXC::Canvas::SetTop(box, 0);
            // The fill runs under the border line to the outer edge, so a translucent border
            // tints the fill instead of showing what is behind the surface.
            box.BackgroundSizing(WUXC::BackgroundSizing::OuterBorderEdge);
            s->box = box;
            kids.InsertAt(0, box);
            s->w = static_cast<float>(c.ActualWidth());
            s->h = static_cast<float>(c.ActualHeight());
            g_surfaces.insert_or_assign(h, s);
            std::weak_ptr<SurfaceState> weak = s;
            c.SizeChanged([weak](WF::IInspectable const&, WUX::SizeChangedEventArgs const& e) {
                if (auto s = weak.lock()) surface_resize(*s, e.NewSize().Width, e.NewSize().Height);
            });
        }
        s->radius = static_cast<float>((std::max)(radius, 0.0));
        s->corners = corners & 15;
        s->clips = clips != 0;
        surface_round_box(*s);
        surface_reshape(h, *s);
        surface_resize(*s, s->w, s->h);
    });
}

/// A line of `width` points around a styled surface, inside its frame and over its children,
/// following its corners. A width of zero removes it.
void day_xaml_container_set_border(void* h, unsigned int argb, double width) {
    guard([&] {
        auto c = elem(h).try_as<WUXC::Canvas>();
        auto s = surface_of(h);
        if (!c || !s) return;
        if (!(width > 0)) {
            if (s->edge) {
                uint32_t at = 0;
                if (c.Children().IndexOf(s->edge, at)) c.Children().RemoveAt(at);
                s->edge = nullptr;
            }
            s->edge_width = 0;
            return;
        }
        if (!s->edge) {
            WUXC::Border edge;
            edge.Name(L"day_edge");
            edge.IsHitTestVisible(false);
            WUXC::Canvas::SetLeft(edge, 0);
            WUXC::Canvas::SetTop(edge, 0);
            // Above every child day places, which keep the default z-index of 0. (XAML takes
            // z-indices up to 1,000,000 and throws past that.)
            WUXC::Canvas::SetZIndex(edge, 100000);
            c.Children().Append(edge);
            s->edge = edge;
        }
        s->edge.BorderBrush(WUXM::SolidColorBrush(color_argb(argb)));
        s->edge.BorderThickness(WUX::Thickness{width, width, width, width});
        s->edge_width = width;
        surface_round_box(*s);
        surface_resize(*s, s->w, s->h);
    });
}

/// Fill a styled surface with a linear gradient between two unit points of its frame. The
/// brush maps to the element's bounding box, so it follows every resize by itself.
void day_xaml_container_set_gradient(void* h, const double* line, const double* offsets,
                                     const unsigned int* colors, int n) {
    guard([&] {
        auto s = surface_of(h);
        if (!s || n <= 0 || !line || !offsets || !colors) return;
        surface_clear_conic(*s);
        WUXM::LinearGradientBrush brush;
        brush.MappingMode(WUXM::BrushMappingMode::RelativeToBoundingBox);
        brush.StartPoint({static_cast<float>(line[0]), static_cast<float>(line[1])});
        brush.EndPoint({static_cast<float>(line[2]), static_cast<float>(line[3])});
        for (int i = 0; i < n; ++i) {
            WUXM::GradientStop stop;
            stop.Offset(offsets[i]);
            stop.Color(color_argb(colors[i]));
            brush.GradientStops().Append(stop);
        }
        s->box.Background(brush);
    });
}

/// Fill a styled surface with a radial gradient around the unit point (`cx`, `cy`) of its frame,
/// `radius` frame-widths across and frame-heights down (an ellipse in a frame that is not square).
/// System XAML has no radial brush and shows the outermost stop's color.
void day_xaml_container_set_radial(void* h, double cx, double cy, double radius,
                                   const double* offsets, const unsigned int* colors, int n) {
    guard([&] {
        auto s = surface_of(h);
        if (!s || n <= 0 || !offsets || !colors) return;
        surface_clear_conic(*s);
#ifdef DAY_WINUI
        WUXM::RadialGradientBrush brush;
        brush.MappingMode(WUXM::BrushMappingMode::RelativeToBoundingBox);
        brush.Center({static_cast<float>(cx), static_cast<float>(cy)});
        brush.GradientOrigin({static_cast<float>(cx), static_cast<float>(cy)});
        brush.RadiusX(radius);
        brush.RadiusY(radius);
        for (int i = 0; i < n; ++i) {
            WUXM::GradientStop stop;
            stop.Offset(offsets[i]);
            stop.Color(color_argb(colors[i]));
            brush.GradientStops().Append(stop);
        }
        s->box.Background(brush);
#else
        (void)cx;
        (void)cy;
        (void)radius;
        s->box.Background(WUXM::SolidColorBrush(color_argb(colors[n - 1])));
#endif
    });
}

/// Fill a styled surface with a conic gradient around the unit point (`cx`, `cy`) of its frame,
/// its first stop at `start_deg` clockwise from +x. Win2D paints it, again at each new size.
/// System XAML has no Win2D and shows the first stop's color.
void day_xaml_container_set_conic(void* h, double cx, double cy, double start_deg,
                                  const double* offsets, const unsigned int* colors, int n) {
    guard([&] {
        auto s = surface_of(h);
        if (!s || n <= 0 || !offsets || !colors) return;
#ifdef DAY_WINUI
        s->conic.clear();
        for (int i = 0; i < n; ++i)
            s->conic.push_back(WCB::CanvasGradientStop{static_cast<float>(offsets[i]), color_argb(colors[i])});
        s->conic_x = static_cast<float>(cx);
        s->conic_y = static_cast<float>(cy);
        s->conic_start = static_cast<float>(start_deg);
        s->conic_w = s->conic_h = -1;
        surface_paint_conic(*s, s->w, s->h);
#else
        (void)cx;
        (void)cy;
        (void)start_deg;
        s->box.Background(WUXM::SolidColorBrush(color_argb(colors[0])));
#endif
    });
}

// Composition's `DropShadow.BlurRadius` is not the Gaussian's standard deviation: the blur it
// produces has a standard deviation of a third of it (its default of 9 is Direct2D's shadow
// effect at its default deviation of 3). Day's blur is twice the deviation, hence 1.5.
static constexpr float kShadowBlurPerDayBlur = 1.5f;

/// The shadow of a styled surface's outline: `argb`, feathered over `blur` points and moved by
/// (`dx`, `dy`). A surface that has one takes the new values; a transparent `argb` hides it.
void day_xaml_container_set_shadow(void* h, unsigned int argb, double blur, double dx, double dy) {
    guard([&] {
        auto c = elem(h).try_as<WUXC::Canvas>();
        auto s = surface_of(h);
        if (!c || !s) return;
        if (s->drop) {
            s->drop.Color(color_argb(argb));
            s->drop.BlurRadius(static_cast<float>((std::max)(blur, 0.0)) * kShadowBlurPerDayBlur);
            s->drop.Offset({static_cast<float>(dx), static_cast<float>(dy), 0.0f});
            return;
        }
        if ((argb >> 24) == 0) return;
        const float w = s->w, ht = s->h;
        // The shadow hangs on an empty host of its own beneath the box; its compositor comes
        // from that host, so the container's own visual is left to the clip.
        WUXC::Canvas host;
        auto compositor = WUXH::ElementCompositionPreview::GetElementVisual(host).Compositor();
        auto shadow = compositor.CreateDropShadow();
        shadow.Color(color_argb(argb));
        shadow.BlurRadius(static_cast<float>((std::max)(blur, 0.0)) * kShadowBlurPerDayBlur);
        shadow.Offset({static_cast<float>(dx), static_cast<float>(dy), 0.0f});
        try {
            // The mask: the outline, filled, drawn by a ShapeVisual that lives only as the
            // source of a visual surface. Without it the shadow is the visual's square box.
            s->mask = outline_new(compositor, w, ht, s->radius, s->corners);
            auto shape = compositor.CreateSpriteShape(s->mask.geometry());
            shape.FillBrush(compositor.CreateColorBrush(color_argb(0xFF'000000u)));
            s->mask_visual = compositor.CreateShapeVisual();
            s->mask_visual.Shapes().Append(shape);
            s->mask_surface = compositor.CreateVisualSurface();
            s->mask_surface.SourceVisual(s->mask_visual);
            shadow.Mask(compositor.CreateSurfaceBrush(s->mask_surface));
        } catch (...) {
            // Visual surfaces need Windows 10 1903. Before that the shadow keeps square corners.
            s->mask = SurfaceOutline{};
            s->mask_visual = nullptr;
            s->mask_surface = nullptr;
        }
        s->drop = shadow;
        s->shadow = compositor.CreateSpriteVisual();
        s->shadow.Shadow(shadow);
        host.Name(L"day_shadow");
        host.IsHitTestVisible(false);
        WUXC::Canvas::SetLeft(host, 0);
        WUXC::Canvas::SetTop(host, 0);
        c.Children().InsertAt(0, host);
        WUXH::ElementCompositionPreview::SetElementChildVisual(host, s->shadow);
        surface_resize(*s, s->w, s->h);
    });
}

void day_xaml_container_set_bg(void* h, unsigned int argb) {
    if (auto s = surface_of(h)) {
        guard([&] {
            surface_clear_conic(*s);
            s->box.Background(WUXM::SolidColorBrush(color_argb(argb)));
        });
        return;
    }
    if (auto c = elem(h).try_as<WUXC::Canvas>())
        ensure_bg_rect(c).Fill(WUXM::SolidColorBrush(color_argb(argb)));
}

// --- backend-executed animation (DESIGN.md §8.4, docs/tween.md "The animation contract") ---
// Day passes INTENT — a target value plus an AnimSpec — and XAML plays it: one Storyboard per
// (element, property), a key-frame animation whose frames are the spec's own samples
// (`AnimSpec::keyframes`, one forward cycle, the spring included), so every curve plays exactly as
// the contract defines it. The parts of the contract, in XAML's terms:
// - delay: the storyboard's BeginTime, with the property's base value held at the START until the
//   first cycle begins (XAML timelines have no backwards fill);
// - repeat: RepeatBehavior, counted in cycles; with autoreverse a forward-and-back pair is one XAML
//   iteration, so `(repeat + 1) / 2` (a fractional count ends on a forward half);
// - rest: when a finite run completes the base value becomes the target and the storyboard goes;
// - retarget: a new run starts from the value shown now (the running animation's), and a plain set
//   stops any run, one that repeats for ever included.
// A null `DayAnim` sets the value outright.
struct DayAnim {
    int delay_ms;
    int duration_ms;
    unsigned repeat;     // cycles beyond the first; UINT_MAX = for ever
    int autoreverse;
    int frames;          // samples in `progress`, uniformly spaced over one cycle
    const double* progress;
};

} // extern "C"
extern "C++" {
static std::map<std::pair<void*, std::wstring>, WUXMA::Storyboard> g_anims;
// Each run's number, so a completion that arrives after a newer run replaced it is ignored.
static std::map<std::pair<void*, std::wstring>, uint64_t> g_anim_runs;
static uint64_t g_anim_next = 0;

static void stop_anim(void* key, std::wstring const& path) {
    auto it = g_anims.find({key, path});
    if (it != g_anims.end()) {
        it->second.Stop();
        g_anims.erase(it);
    }
}

static WF::TimeSpan day_ms(double ms) { return WF::TimeSpan{static_cast<int64_t>(std::llround(ms * 10000.0))}; }

// The XAML timing of one spec: begin time, repeat count and auto-reverse, on `t`.
static void day_time(WUXMA::Timeline const& t, DayAnim const& a) {
    t.AutoReverse(a.autoreverse != 0);
    if (a.repeat == UINT_MAX) {
        t.RepeatBehavior(WUXMA::RepeatBehaviorHelper::Forever());
    } else {
        const double cycles = static_cast<double>(a.repeat) + 1.0;
        t.RepeatBehavior(WUXMA::RepeatBehaviorHelper::FromCount(a.autoreverse ? cycles / 2.0 : cycles));
    }
}

// Play `path` on `target` from `from` to `to` under `a`. `set` writes the property's base value:
// the start while the delay runs, the target once a finite run completes (and at once, for a run
// with no duration).
static void animate_double(void* key, WUX::DependencyObject const& target, std::wstring const& path,
                           double from, double to, DayAnim const& a, bool dependent,
                           std::function<void(double)> set) {
    stop_anim(key, path);
    if (a.duration_ms <= 0 || a.frames < 2 || !a.progress) {
        if (a.delay_ms <= 0) { set(to); return; }
        // Nothing to play: hold the start through the delay, then the target.
        set(from);
        day_after(static_cast<unsigned>(a.delay_ms), [set, to] { set(to); });
        return;
    }
    WUXMA::DoubleAnimationUsingKeyFrames anim;
    anim.EnableDependentAnimation(dependent);
    const double cycle = a.duration_ms;
    for (int i = 0; i < a.frames; ++i) {
        WUXMA::LinearDoubleKeyFrame k;
        k.KeyTime(WUXMA::KeyTimeHelper::FromTimeSpan(day_ms(cycle * i / (a.frames - 1))));
        k.Value(from + (to - from) * a.progress[i]);
        anim.KeyFrames().Append(k);
    }
    anim.Duration(WUX::DurationHelper::FromTimeSpan(day_ms(cycle)));
    day_time(anim, a);
    anim.FillBehavior(WUXMA::FillBehavior::HoldEnd);
    WUXMA::Storyboard sb;
    sb.BeginTime(day_ms(a.delay_ms));
    sb.Children().Append(anim);
    WUXMA::Storyboard::SetTarget(anim, target);
    WUXMA::Storyboard::SetTargetProperty(anim, path);
    set(from);
    const std::pair<void*, std::wstring> slot{key, path};
    const uint64_t run = ++g_anim_next;
    g_anim_runs[slot] = run;
    sb.Completed([slot, set, to, run](WF::IInspectable const&, WF::IInspectable const&) {
        auto it = g_anims.find(slot);
        auto current = g_anim_runs.find(slot);
        if (it == g_anims.end() || current == g_anim_runs.end() || current->second != run) return;
        g_anim_runs.erase(current);
        set(to);
        it->second.Stop();
        g_anims.erase(it);
    });
    g_anims[slot] = sb;
    sb.Begin();
}

// The same for a brush color: the in-between colors mixed per channel in sRGB, a spring's
// overshoot clamped to the two ends.
static void animate_color(void* key, WUXM::SolidColorBrush const& brush, WUI::Color from, WUI::Color to,
                          DayAnim const& a) {
    const std::wstring path = L"BgColor";
    stop_anim(key, path);
    auto set = [brush](WUI::Color c) { brush.Color(c); };
    if (a.duration_ms <= 0 || a.frames < 2 || !a.progress) {
        if (a.delay_ms <= 0) { set(to); return; }
        set(from);
        day_after(static_cast<unsigned>(a.delay_ms), [set, to] { set(to); });
        return;
    }
    auto mix = [](uint8_t x, uint8_t y, double p) {
        const double v = x + (static_cast<double>(y) - x) * (std::min)(1.0, (std::max)(0.0, p));
        return static_cast<uint8_t>(std::lround(v));
    };
    WUXMA::ColorAnimationUsingKeyFrames anim;
    anim.EnableDependentAnimation(true);
    const double cycle = a.duration_ms;
    for (int i = 0; i < a.frames; ++i) {
        const double p = a.progress[i];
        WUXMA::LinearColorKeyFrame k;
        k.KeyTime(WUXMA::KeyTimeHelper::FromTimeSpan(day_ms(cycle * i / (a.frames - 1))));
        k.Value(WUI::Color{mix(from.A, to.A, p), mix(from.R, to.R, p), mix(from.G, to.G, p), mix(from.B, to.B, p)});
        anim.KeyFrames().Append(k);
    }
    anim.Duration(WUX::DurationHelper::FromTimeSpan(day_ms(cycle)));
    day_time(anim, a);
    anim.FillBehavior(WUXMA::FillBehavior::HoldEnd);
    WUXMA::Storyboard sb;
    sb.BeginTime(day_ms(a.delay_ms));
    sb.Children().Append(anim);
    WUXMA::Storyboard::SetTarget(anim, brush);
    WUXMA::Storyboard::SetTargetProperty(anim, L"Color");
    set(from);
    const std::pair<void*, std::wstring> slot{key, path};
    const uint64_t run = ++g_anim_next;
    g_anim_runs[slot] = run;
    sb.Completed([slot, set, to, run](WF::IInspectable const&, WF::IInspectable const&) {
        auto it = g_anims.find(slot);
        auto current = g_anim_runs.find(slot);
        if (it == g_anims.end() || current == g_anim_runs.end() || current->second != run) return;
        g_anim_runs.erase(current);
        set(to);
        it->second.Stop();
        g_anims.erase(it);
    });
    g_anims[slot] = sb;
    sb.Begin();
}
} // extern "C++"
extern "C" {

// The CompositeTransform every transform channel animates through. Installed once per element;
// RenderTransformOrigin is the CENTER, matching AppKit's layer anchor and Qt's painter transform
// (Day's anchor_x/anchor_y are 0.5/0.5 in practice and the other backends center unconditionally).
static WUXM::CompositeTransform ensure_transform(WUX::UIElement const& el) {
    if (auto existing = el.RenderTransform().try_as<WUXM::CompositeTransform>()) return existing;
    WUXM::CompositeTransform t;
    el.RenderTransform(t);
    el.RenderTransformOrigin(WF::Point{0.5f, 0.5f});
    return t;
}

void day_xaml_set_opacity(void* h, double opacity, const DayAnim* anim) {
    auto el = elem(h).try_as<WUX::UIElement>();
    if (!el) return;
    if (!anim) {
        stop_anim(h, L"Opacity");
        el.Opacity(opacity);
        return;
    }
    // The value shown now (a running animation's), read before that animation stops.
    const double from = el.Opacity();
    animate_double(h, el, L"Opacity", from, opacity, *anim, false,
                   [el](double v) { el.Opacity(v); });
}

void day_xaml_set_transform(void* h, double tx, double ty, double sx, double sy, double rotate_deg,
                            const DayAnim* anim) {
    auto el = elem(h).try_as<WUX::UIElement>();
    if (!el) return;
    auto t = ensure_transform(el);
    struct Channel {
        const wchar_t* path;
        double to;
        double now;
        std::function<void(double)> set;
    };
    const Channel channels[] = {
        {L"TranslateX", tx, t.TranslateX(), [t](double v) { t.TranslateX(v); }},
        {L"TranslateY", ty, t.TranslateY(), [t](double v) { t.TranslateY(v); }},
        {L"ScaleX", sx, t.ScaleX(), [t](double v) { t.ScaleX(v); }},
        {L"ScaleY", sy, t.ScaleY(), [t](double v) { t.ScaleY(v); }},
        {L"Rotation", rotate_deg, t.Rotation(), [t](double v) { t.Rotation(v); }},
    };
    for (auto const& ch : channels) {
        if (!anim) {
            stop_anim(h, ch.path);
            ch.set(ch.to);
        } else {
            animate_double(h, t, ch.path, ch.now, ch.to, *anim, false, ch.set);
        }
    }
}

// Animated background fill. A brush color is not composited independently, so this one is a
// dependent animation — a single box, not a per-frame layout cost.
void day_xaml_container_animate_bg(void* h, unsigned int argb, const DayAnim* anim) {
    auto canvas = elem(h).try_as<WUXC::Canvas>();
    if (!canvas) return;
    WUXM::SolidColorBrush brush{nullptr};
    if (auto s = surface_of(h)) {
        // A styled surface's fill is its Border's background; the border and corners stay.
        brush = s->box.Background().try_as<WUXM::SolidColorBrush>();
        if (!anim || !brush) {
            stop_anim(h, L"BgColor");
            surface_clear_conic(*s);
            s->box.Background(WUXM::SolidColorBrush(color_argb(argb)));
            return;
        }
    } else {
        auto rect = ensure_bg_rect(canvas);
        brush = rect.Fill().try_as<WUXM::SolidColorBrush>();
        if (!anim || !brush) {
            stop_anim(h, L"BgColor");
            rect.Fill(WUXM::SolidColorBrush(color_argb(argb)));
            return;
        }
    }
    animate_color(h, brush, brush.Color(), color_argb(argb), *anim);
}

// Defined beside `day_xaml_set_geometry`.
void day_geometry_follows(void* h, int width, int height);

// An animated frame (`set_frame` with an AnimSpec): the four layout values Day writes in
// `day_xaml_set_geometry`, played as dependent animations from the frame shown now.
void day_xaml_set_geometry_animated(void* h, double x, double y, double width, double height,
                                    const DayAnim* anim) {
    auto fe = elem(h).try_as<FrameworkElement>();
    if (!fe || !anim) return;
    auto num = [](double v, double fallback) { return std::isnan(v) ? fallback : v; };
    struct Channel {
        const wchar_t* path;
        double from, to;
        std::function<void(double)> set;
    };
    const Channel channels[] = {
        {L"(Canvas.Left)", num(WUXC::Canvas::GetLeft(fe), x), x, [fe](double v) { WUXC::Canvas::SetLeft(fe, v); }},
        {L"(Canvas.Top)", num(WUXC::Canvas::GetTop(fe), y), y, [fe](double v) { WUXC::Canvas::SetTop(fe, v); }},
        {L"Width", num(fe.Width(), width), width, [fe](double v) { fe.Width(v); }},
        {L"Height", num(fe.Height(), height), height, [fe](double v) { fe.Height(v); }},
    };
    for (auto const& ch : channels) animate_double(h, fe, ch.path, ch.from, ch.to, *anim, true, ch.set);
    day_geometry_follows(h, static_cast<int>(std::lround(width)), static_cast<int>(std::lround(height)));
}

// Emulated fullscreen cover (docs/cover.md): give the cover an OPAQUE page-background surface
// (the same grounding as the window root) so it occludes the content beneath it.
void day_xaml_cover_ground(void* h) {
    auto c = elem(h).try_as<WUXC::Canvas>();
    if (!c) return;
    auto rect = ensure_bg_rect(c);
    // The named brush resolves per the SYSTEM theme, so it is only correct while nothing overrides
    // it — under a DAY_THEME force or the app's own Appearance pick it would ground the cover in
    // the wrong scheme. `effective_dark` already ranks those, so ask it first and only fall back to
    // the resource when the app is genuinely following the system.
    if (!g_forced_theme && !g_app_override) {
        auto res = WUX::Application::Current().Resources();
        auto key = winrt::box_value(winrt::hstring(L"ApplicationPageBackgroundThemeBrush"));
        if (res.HasKey(key)) {
            if (auto brush = res.Lookup(key).try_as<WUXM::Brush>()) {
                rect.Fill(brush);
                return;
            }
        }
    }
    rect.Fill(WUXM::SolidColorBrush(color_argb(effective_dark() ? 0xFF'202020u : 0xFF'F3F3F3u)));
}

// SurfaceRole::SectionCard: the grouped-card fill from the theme resources — resolved per the
// APP theme (a DAY_THEME force is applied app-wide, so this tracks it). Fallback where the
// resource set predates the card brush: a translucent neutral, which reads correctly over
// either scheme's page ground precisely because it is alpha-over-ground rather than a fixed
// opaque color.
static void fill_section_card(WUXSh::Rectangle const& r) {
    bool filled = false;
    // The app-resource card brush resolves per the SYSTEM theme; only trust it when nothing
    // overrides that — a DAY_THEME force or the app's own Appearance pick both re-theme
    // per-element (`Application::RequestedTheme` is unsupported under Islands), so the app
    // resources still answer for the system scheme and would mis-color the chosen one. That is
    // exactly what left cards light on a dark override. The translucent-neutral fallback is
    // alpha-over-ground, so it reads correctly over either scheme's page ground.
    if (g_forced_theme == 0 && g_app_override == 0) {
        auto res = WUX::Application::Current().Resources();
        auto key = winrt::box_value(winrt::hstring(L"CardBackgroundFillColorDefaultBrush"));
        if (res.HasKey(key)) {
            if (auto brush = res.Lookup(key).try_as<WUXM::Brush>()) {
                r.Fill(brush);
                filled = true;
            }
        }
    }
    if (!filled) r.Fill(WUXM::SolidColorBrush(color_argb(0x14'808080u)));
}

void day_xaml_container_set_card(void* h, double radius) {
    if (auto c = elem(h).try_as<WUXC::Canvas>()) {
        auto r = ensure_bg_rect(c);
        fill_section_card(r);
        // An application-resource lookup is a brush snapshot, not a ThemeResource binding.
        // Resolve it again when the island changes theme, including secondary windows.
        r.ActualThemeChanged([](FrameworkElement const& sender, WF::IInspectable const&) {
            fill_section_card(sender.as<WUXSh::Rectangle>());
        });
        r.RadiusX(radius);
        r.RadiusY(radius);
    }
}

// Rounded corners for a container's background (login card, chat bubbles, avatar/badge discs).
// RadiusX/RadiusY live on the Rectangle SHAPE, not on RectangleGeometry.
void day_xaml_container_set_corner(void* h, double radius) {
    auto c = elem(h).try_as<WUXC::Canvas>();
    if (!c) return;
    // The container's OWN fill, for the case where this container also carries the background.
    auto r = ensure_bg_rect(c);
    r.RadiusX(radius);
    r.RadiusY(radius);

    // …and a real rounded CLIP over the children, which is what `ContainerProps.clips` asks for
    // and what the common case actually needs. `corner_radius(r)` is its own container in the
    // piece tree, wrapping the `background(c)` container rather than sharing one with it:
    //
    //   corner_radius container   (transparent, radius r, clips)
    //     └── background container  (opaque fill, square)
    //
    // so rounding only this container's own — invisible — rect left the child's square fill
    // painting over the top, and every `.background(…).corner_radius(…)` surface came out sharp.
    //
    // It has to be a COMPOSITION clip: XAML's `UIElement.Clip` is typed as RectangleGeometry,
    // which has no corner radius, so the rounded shape is unreachable from the XAML side.
    try {
        auto visual = WUXH::ElementCompositionPreview::GetElementVisual(c);
        if (!visual) return;
        auto compositor = visual.Compositor();
        auto geo = compositor.CreateRoundedRectangleGeometry();
        auto rf = static_cast<float>(radius);
        geo.CornerRadius({ rf, rf });
        geo.Size({ static_cast<float>(c.ActualWidth()), static_cast<float>(c.ActualHeight()) });
        visual.Clip(compositor.CreateGeometricClip(geo));
        // `insert_or_assign`, not `operator[]`: a projected WinRT type has no default ctor.
        g_clip_geometry.insert_or_assign(h, geo);
        // day sizes the container after realize, so the clip has to track it the way the
        // background rect does — a clip left at 0×0 would hide the whole subtree.
        c.SizeChanged([geo](WF::IInspectable const&, WUX::SizeChangedEventArgs const& e) mutable {
            geo.Size({ e.NewSize().Width, e.NewSize().Height });
        });
    } catch (...) {
        // CreateGeometricClip needs 1809+; on older builds the corners stay square, which is the
        // behavior this backend already had.
    }
}

