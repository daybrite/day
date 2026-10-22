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

void day_xaml_container_set_bg(void* h, unsigned int argb) {
    if (auto c = elem(h).try_as<WUXC::Canvas>())
        ensure_bg_rect(c).Fill(WUXM::SolidColorBrush(color_argb(argb)));
}

// --- backend-executed animation (DESIGN.md §8.4) ---
// Day passes INTENT — a target value plus an AnimSpec — and the platform animates it; Day never
// ticks frames for native widgets. Here that is a Storyboard per channel, which XAML runs on its
// own compositor. `curve`: 0 linear, 1 ease-in, 2 ease-out, 3 ease-in-out, 4 spring (the shared
// encoding day-qt uses). `dur_ms <= 0` means "no animation" — set the value outright.

static WUXMA::EasingFunctionBase easing_for(int curve) {
    switch (curve) {
        case 1: {
            WUXMA::QuadraticEase e;
            e.EasingMode(WUXMA::EasingMode::EaseIn);
            return e;
        }
        case 2: {
            WUXMA::QuadraticEase e;
            e.EasingMode(WUXMA::EasingMode::EaseOut);
            return e;
        }
        case 3: {
            WUXMA::QuadraticEase e;
            e.EasingMode(WUXMA::EasingMode::EaseInOut);
            return e;
        }
        case 4: {
            // §8.4: a spring maps to a fixed-duration OVERSHOOT curve over exactly duration_ms, so
            // the timing matches the other toolkits rather than a physics settle time.
            WUXMA::BackEase e;
            e.EasingMode(WUXMA::EasingMode::EaseOut);
            e.Amplitude(0.35);
            return e;
        }
        default:
            return nullptr; // linear
    }
}

// One Storyboard per (element, property path). Re-animating a channel has to REPLACE the running
// storyboard: two live storyboards on one property fight, and a stopped one snaps its property
// back to the pre-animation value. Keyed by element+path so the channels stay independent.
static std::map<std::pair<void*, std::wstring>, WUXMA::Storyboard> g_anims;

static void stop_anim(void* key, std::wstring const& path) {
    auto it = g_anims.find({key, path});
    if (it != g_anims.end()) {
        it->second.Stop();
        g_anims.erase(it);
    }
}

// Animate one double property to `to`. `target` is the object the path is rooted at.
static void animate_double(void* key, WUX::DependencyObject const& target, std::wstring const& path,
                           double to, int dur_ms, int curve, bool dependent) {
    stop_anim(key, path);
    WUXMA::DoubleAnimation a;
    a.To(to);
    a.Duration(WUX::Duration(WF::TimeSpan{static_cast<int64_t>(dur_ms) * 10000}));
    a.EnableDependentAnimation(dependent);
    if (auto e = easing_for(curve)) a.EasingFunction(e);
    // FillBehavior::HoldEnd keeps the animated value after the run; without it the property snaps
    // back to its pre-animation value the moment the storyboard completes.
    a.FillBehavior(WUXMA::FillBehavior::HoldEnd);
    WUXMA::Storyboard sb;
    sb.Children().Append(a);
    WUXMA::Storyboard::SetTarget(a, target);
    WUXMA::Storyboard::SetTargetProperty(a, path);
    g_anims[{key, path}] = sb;
    sb.Begin();
}

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

void day_xaml_set_opacity(void* h, double opacity, int dur_ms, int curve) {
    auto el = elem(h).try_as<WUX::UIElement>();
    if (!el) return;
    if (dur_ms <= 0) {
        stop_anim(h, L"Opacity");
        el.Opacity(opacity);
        return;
    }
    // Opacity is GPU-composited, so this runs independent of the UI thread.
    animate_double(h, el, L"Opacity", opacity, dur_ms, curve, false);
}

void day_xaml_set_transform(void* h, double tx, double ty, double sx, double sy, double rotate_deg,
                            int dur_ms, int curve) {
    auto el = elem(h).try_as<WUX::UIElement>();
    if (!el) return;
    auto t = ensure_transform(el);
    struct Channel { const wchar_t* path; double to; };
    const Channel channels[] = {
        {L"TranslateX", tx}, {L"TranslateY", ty},
        {L"ScaleX", sx},     {L"ScaleY", sy},
        {L"Rotation", rotate_deg},
    };
    for (auto const& ch : channels) {
        if (dur_ms <= 0) {
            stop_anim(h, ch.path);
            if (std::wstring(ch.path) == L"TranslateX") t.TranslateX(ch.to);
            else if (std::wstring(ch.path) == L"TranslateY") t.TranslateY(ch.to);
            else if (std::wstring(ch.path) == L"ScaleX") t.ScaleX(ch.to);
            else if (std::wstring(ch.path) == L"ScaleY") t.ScaleY(ch.to);
            else t.Rotation(ch.to);
        } else {
            // CompositeTransform channels are GPU-composited too — independent animations.
            animate_double(h, t, ch.path, ch.to, dur_ms, curve, false);
        }
    }
}

// Animated background fill. Unlike the transform channels a brush color is NOT composited
// independently, so this one needs EnableDependentAnimation — it is a single box, not a per-frame
// layout cost. XAML is the only desktop backend that can tween this (DESIGN.md §8.4).
void day_xaml_container_animate_bg(void* h, unsigned int argb, int dur_ms, int curve) {
    auto canvas = elem(h).try_as<WUXC::Canvas>();
    if (!canvas) return;
    auto rect = ensure_bg_rect(canvas);
    auto brush = rect.Fill().try_as<WUXM::SolidColorBrush>();
    if (dur_ms <= 0 || !brush) {
        rect.Fill(WUXM::SolidColorBrush(color_argb(argb)));
        return;
    }
    stop_anim(h, L"BgColor");
    WUXMA::ColorAnimation a;
    a.To(color_argb(argb));
    a.Duration(WUX::Duration(WF::TimeSpan{static_cast<int64_t>(dur_ms) * 10000}));
    a.EnableDependentAnimation(true);
    if (auto e = easing_for(curve)) a.EasingFunction(e);
    a.FillBehavior(WUXMA::FillBehavior::HoldEnd);
    WUXMA::Storyboard sb;
    sb.Children().Append(a);
    WUXMA::Storyboard::SetTarget(a, brush);
    WUXMA::Storyboard::SetTargetProperty(a, L"Color");
    g_anims[{h, L"BgColor"}] = sb;
    sb.Begin();
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

// Every rounded CLIP this shim installed, by the handle that owns it. `day_xaml_set_geometry` is
// where day states a container's real size, and the clip has to follow it from there: the
// `SizeChanged` hook below is not enough on its own, because a Canvas given an explicit Width and
// Height by day does not always raise it — and a clip left at its birth size of 0x0 hides the
// whole subtree, which is how a rounded `vector(…)` mark rendered as nothing at all.
static std::map<void*, WUComp::CompositionRoundedRectangleGeometry> g_clip_geometry;

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

