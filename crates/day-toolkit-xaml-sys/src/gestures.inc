// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Included by shim.cpp inside the C ABI block. Shared declarations and include order live there.

// Attach a native pointer recognizer to `h`, reporting to cb(id, phase, x, y, tx, ty) with `x,y` in
// the element's local space (docs/shapes.md). kind 0 = Tap, 1 = LongPress, 2 = Drag, 3 = Hover,
// 4 = Pinch, 5 = Pan on a canvas (every wheel tick), 6 = Pan on any other container (sideways
// ticks only; vertical ones keep scrolling natively), 7 = Rotate, 8 = DoubleTap, 9 = Press,
// 10 = Touches (reported through `day_xaml_set_touches_cb`). Phase codes: 0 Tap, 1
// Drag-Began, 2 Drag-Changed, 3 Drag-Ended, 4 LongPress,
// 5/6/7 Pinch Began/Changed/Ended (tx = the scale), 8 Pan (tx,ty = the delta), 10/11/12 Hover,
// 13/14/15 Rotate Began/Changed/Ended (tx = cumulative radians, clockwise positive),
// 16 DoubleTap, 17 press in, 18 press out, 19 the drag's release velocity (tx,ty in DIPs per
// second, sent just before its Ended) (matching day-toolkit-winui's on_gesture).
// Routed events bubble from the shape's Path children up to the Canvas handle, so only the drawn
// shape is hit (a transparent Canvas isn't hit-testable) — the "hit-test the path" semantic.

// Add `mode` to the element's manipulation modes: a drag and a pinch on the same element share
// one manipulation (translate and scale come out of the same events), so neither may replace
// the other's bit. `System` is the untouched default, not a mode to keep.
static void day_add_manipulation_mode(WUX::UIElement const& el, WUXIn::ManipulationModes mode) {
    auto current = el.ManipulationMode();
    if (current == WUXIn::ManipulationModes::System) current = WUXIn::ManipulationModes::None;
    el.ManipulationMode(current | mode);
}

static bool day_manipulation_scaled(float scale) {
    return std::fabs(scale - 1.0f) > 1e-4f;
}

// A manipulation's rotation (degrees) has left zero: two fingers are turning.
static bool day_manipulation_rotated(float degrees) {
    return std::fabs(degrees) > 1e-3f;
}


} // extern "C"
extern "C++" {
// What the recognizers on one element share (docs/gestures.md "Recognizers"): the kinds coexist on
// one element, and a tap has to know whether a double tap is listening, a long press whether its
// release must not count as a tap, the press and the slop the grown hit area. Dropped with the
// node (`gestures_forget`).
struct DayGestures {
    bool tap = false, dbl = false, hold = false, press = false, touches = false;
    // A single tap held back while a double tap may still follow; bumping it cancels.
    unsigned tap_generation = 0;
    // The mouse long press: its timer (bumped to cancel), where it went down, and whether it fired
    // (then the release that follows is not a tap).
    unsigned hold_generation = 0;
    bool mouse_down = false, suppress_tap = false;
    WF::Point mouse_at{};
    // The press: the pointer that pressed and its last point.
    bool pressed = false;
    uint32_t press_pointer = 0;
    WF::Point press_at{};
    // Touches: every contact that went down on the element and has not ended.
    struct Contact { double x, y, pressure; int kind, phase; };
    std::map<uint32_t, Contact> contacts;
    // The drag's axis (0 any, 1 horizontal, 2 vertical) and the hit slop in DIPs.
    int axis = 0;
    double slop_left = 0, slop_top = 0, slop_right = 0, slop_bottom = 0;
    WUXC::Canvas slop{nullptr};
};
static std::map<void*, std::shared_ptr<DayGestures>> g_gestures;
static std::shared_ptr<DayGestures> gestures_of(void* h) {
    auto& g = g_gestures[h];
    if (!g) g = std::make_shared<DayGestures>();
    return g;
}
static void gestures_forget(void* h) { g_gestures.erase(h); }

// A pointer handler added with handledEventsToo: the press and the touches recognizers observe,
// so a control under them that marks its pointer events handled does not hide them.
template <typename F>
static void day_observe(UIElement const& el, WUX::RoutedEvent const& event, F&& f) {
    el.AddHandler(event, winrt::box_value(WUXIn::PointerEventHandler(std::forward<F>(f))), true);
}

// The primary contact of a pointer down: the left mouse button, or a touch or pen in contact.
static bool day_primary_down(WUXIn::PointerRoutedEventArgs const& a, UIElement const& el) {
    auto pt = a.GetCurrentPoint(el);
    if (a.Pointer().PointerDeviceType() == DayPointerDeviceType::Mouse)
        return pt.Properties().IsLeftButtonPressed();
    return pt.IsInContact();
}

// Whether `p` (element space) is inside the element's bounds grown by its hit slop.
static bool day_inside(UIElement const& el, DayGestures const& g, WF::Point p) {
    auto fe = el.try_as<FrameworkElement>();
    if (!fe) return true;
    return p.X >= -g.slop_left && p.Y >= -g.slop_top &&
           p.X <= fe.ActualWidth() + g.slop_right && p.Y <= fe.ActualHeight() + g.slop_bottom;
}

// The device kind as the touches wire spells it: 0 touch, 1 mouse, 2 stylus.
static int day_pointer_kind(WUXIn::PointerRoutedEventArgs const& a) {
    switch (a.Pointer().PointerDeviceType()) {
        case DayPointerDeviceType::Mouse: return 1;
        case DayPointerDeviceType::Pen: return 2;
        default: return 0;
    }
}

static void (*g_touches_cb)(unsigned long long, const char*) = nullptr;

// One touches report: every live contact, then the ended ones are dropped and the rest go
// stationary until they move again (docs/gestures.md "Touches").
static void day_report_touches(unsigned long long id, DayGestures& g) {
    std::string out;
    char buf[128];
    for (auto const& [pid, c] : g.contacts) {
        std::snprintf(buf, sizeof buf, "%u,%d,%.3f,%.3f,%.4f,%d;", pid, c.phase, c.x, c.y,
                      c.pressure, c.kind);
        out += buf;
    }
    if (!out.empty()) out.pop_back();
    for (auto it = g.contacts.begin(); it != g.contacts.end();) {
        if (it->second.phase >= 3) {
            it = g.contacts.erase(it);
        } else {
            it->second.phase = 2;
            ++it;
        }
    }
    if (g_touches_cb && !out.empty()) g_touches_cb(id, out.c_str());
}

// The grown hit area (docs/gestures.md "Hit slop"): four transparent bands just outside the
// element's bounds, in one zero-size Canvas child. They lie outside the box, so they cover none
// of the element's own children, and a pointer on them is a pointer on the element: its routed
// events bubble from the band to the element's recognizers with the point in the element's own
// space (negative, or past its size). A Canvas does not clip, so the bands reach past the parent
// too; an ancestor that clips (a scroll view, a rounded corner) clips them with everything else.
static void day_place_slop(UIElement const& el, DayGestures& g) {
    auto fe = el.try_as<FrameworkElement>();
    if (!fe || !g.slop) return;
    const double w = fe.ActualWidth() > 0 ? fe.ActualWidth() : (std::isnan(fe.Width()) ? 0 : fe.Width());
    const double h = fe.ActualHeight() > 0 ? fe.ActualHeight() : (std::isnan(fe.Height()) ? 0 : fe.Height());
    const double l = g.slop_left, t = g.slop_top, r = g.slop_right, b = g.slop_bottom;
    struct Band { double x, y, w, h; };
    const Band bands[4] = {
        {-l, -t, w + l + r, t},   // above
        {-l, h, w + l + r, b},    // below
        {-l, 0, l, h},            // leading side
        {w, 0, r, h},             // trailing side
    };
    auto kids = g.slop.Children();
    for (uint32_t i = 0; i < 4 && i < kids.Size(); ++i) {
        auto rect = kids.GetAt(i).as<WUXSh::Rectangle>();
        WUXC::Canvas::SetLeft(rect, bands[i].x);
        WUXC::Canvas::SetTop(rect, bands[i].y);
        rect.Width((std::max)(0.0, bands[i].w));
        rect.Height((std::max)(0.0, bands[i].h));
    }
}
} // extern "C++"
extern "C" {

void day_xaml_set_touches_cb(void (*cb)(unsigned long long, const char*)) { g_touches_cb = cb; }

void day_xaml_set_hit_slop(void* h, double left, double top, double right, double bottom) try {
    auto el = elem(h);
    auto panel = el.try_as<WUXC::Panel>();
    if (!panel) return;
    auto g = gestures_of(h);
    g->slop_left = (std::max)(0.0, left);
    g->slop_top = (std::max)(0.0, top);
    g->slop_right = (std::max)(0.0, right);
    g->slop_bottom = (std::max)(0.0, bottom);
    const bool any = g->slop_left > 0 || g->slop_top > 0 || g->slop_right > 0 || g->slop_bottom > 0;
    if (!any) {
        if (g->slop) {
            uint32_t idx = 0;
            if (panel.Children().IndexOf(g->slop, idx)) panel.Children().RemoveAt(idx);
            g->slop = nullptr;
        }
        return;
    }
    if (!g->slop) {
        WUXC::Canvas host;
        host.Name(L"day_slop");
        WUXC::Canvas::SetLeft(host, 0);
        WUXC::Canvas::SetTop(host, 0);
        for (int i = 0; i < 4; ++i) {
            WUXSh::Rectangle band;
            band.Fill(WUXM::SolidColorBrush(color_argb(0)));
            host.Children().Append(band);
        }
        panel.Children().Append(host);
        g->slop = host;
        if (auto fe = el.try_as<FrameworkElement>()) {
            std::weak_ptr<DayGestures> weak = g;
            fe.SizeChanged([weak, el](WF::IInspectable const&, WUX::SizeChangedEventArgs const&) {
                if (auto g = weak.lock()) day_place_slop(el, *g);
            });
        }
    }
    day_place_slop(el, *g);
} catch (...) {
}

// The drag's axis (docs/gestures.md "Gestures inside a scroll view"): 0 claims any direction, 1
// horizontal, 2 vertical. With an axis, DirectManipulation keeps the other one (`System`), so an
// enclosing ScrollViewer scrolls it on touch, and a sequence that starts across the axis never
// becomes a drag (the Drag recognizer's own check below, for the mouse and the pen).
void day_xaml_set_drag_axis(void* h, int axis) try {
    auto el = elem(h);
    auto g = gestures_of(h);
    g->axis = axis;
    using MM = WUXIn::ManipulationModes;
    auto mode = el.ManipulationMode();
    if (mode == MM::System) mode = MM::None;
    mode = mode & ~(MM::TranslateX | MM::TranslateY | MM::System);
    if (axis == 1) mode = mode | MM::TranslateX | MM::System;
    else if (axis == 2) mode = mode | MM::TranslateY | MM::System;
    else mode = mode | MM::TranslateX | MM::TranslateY;
    el.ManipulationMode(mode);
} catch (...) {
}

void day_xaml_enable_gesture(void* h, unsigned long long id, int kind,
                              void (*cb)(unsigned long long, int, double, double, double,
                                         double)) try {
    auto el = elem(h);
    // Any recognizer makes the node take input over its whole box (docs/gestures.md
    // "Pass-through").
    day_hit_on(h);
    if (kind == 2) { // Drag → manipulation (translate)
        day_add_manipulation_mode(el, WUXIn::ManipulationModes::TranslateX |
                                          WUXIn::ManipulationModes::TranslateY);
        auto g = gestures_of(h);
        // Once the same manipulation scales or turns (a second finger on a pinch or rotate
        // target), the drag ends where it was and the rest of the manipulation is theirs: the
        // two fingers never come out as drags (docs/canvas.md "Zoom and pan").
        auto ended = std::make_shared<bool>(false);
        // With an axis (`day_xaml_set_drag_axis`) the drag begins at its first movement, and only
        // along that axis: a sequence that starts across it is completed at once and reported to
        // nobody, so it never becomes this node's drag (docs/gestures.md).
        auto began = std::make_shared<bool>(false);
        auto start = std::make_shared<WF::Point>();
        el.ManipulationStarted(
            [id, cb, ended, began, start, g](WF::IInspectable const&, WUXIn::ManipulationStartedRoutedEventArgs const& a) {
                *ended = false;
                *began = false;
                *start = a.Position();
                if (g->axis != 0) return;
                *began = true;
                cb(id, 1, start->X, start->Y, 0, 0);
            });
        el.ManipulationDelta(
            [id, cb, ended, began, start, g](WF::IInspectable const&, WUXIn::ManipulationDeltaRoutedEventArgs const& a) {
                if (*ended) return;
                auto p = a.Position();
                auto t = a.Cumulative().Translation;
                if (!*began) {
                    const double dx = std::fabs(t.X), dy = std::fabs(t.Y);
                    if (dx == 0 && dy == 0) return;
                    const bool along = g->axis == 1 ? dx >= dy : dy >= dx;
                    if (!along) {
                        *ended = true;
                        a.Complete();
                        return;
                    }
                    *began = true;
                    cb(id, 1, start->X, start->Y, 0, 0);
                }
                if (day_manipulation_scaled(a.Cumulative().Scale) ||
                    day_manipulation_rotated(a.Cumulative().Rotation)) {
                    *ended = true;
                    cb(id, 3, p.X, p.Y, t.X, t.Y);
                    return;
                }
                cb(id, 2, p.X, p.Y, t.X, t.Y);
            });
        el.ManipulationCompleted(
            [id, cb, ended, began](WF::IInspectable const&, WUXIn::ManipulationCompletedRoutedEventArgs const& a) {
                if (*ended || !*began) return;
                auto p = a.Position();
                auto t = a.Cumulative().Translation;
                // The recognizer's own release velocity, in DIPs per millisecond (docs/gestures.md
                // "Velocity"), sent just before the Ended it belongs to.
                auto v = a.Velocities().Linear;
                cb(id, 19, p.X, p.Y, v.X * 1000.0, v.Y * 1000.0);
                cb(id, 3, p.X, p.Y, t.X, t.Y);
            });
    } else if (kind == 4) {
        // Pinch → the manipulation's scale (two fingers, or a precision touchpad), reported
        // cumulatively from a Began that waits for the scale to leave 1: a one-finger drag runs
        // through the same manipulation and must not come out as a pinch. A mouse's ctrl+wheel
        // and a touchpad pinch the system folds into one arrive as wheel ticks: each is a lone
        // Changed carrying its own factor, and handled so nothing above zooms with it
        // (docs/canvas.md "Zoom and pan").
        day_add_manipulation_mode(el, WUXIn::ManipulationModes::Scale);
        auto active = std::make_shared<bool>(false);
        el.ManipulationStarted(
            [active](WF::IInspectable const&, WUXIn::ManipulationStartedRoutedEventArgs const&) {
                *active = false;
            });
        el.ManipulationDelta(
            [id, cb, active](WF::IInspectable const&, WUXIn::ManipulationDeltaRoutedEventArgs const& a) {
                auto p = a.Position();
                auto s = a.Cumulative().Scale;
                if (!*active) {
                    if (!day_manipulation_scaled(s)) return;
                    *active = true;
                    cb(id, 5, p.X, p.Y, 1.0, 0);
                }
                cb(id, 6, p.X, p.Y, s, 0);
            });
        el.ManipulationCompleted(
            [id, cb, active](WF::IInspectable const&, WUXIn::ManipulationCompletedRoutedEventArgs const& a) {
                if (!*active) return;
                *active = false;
                auto p = a.Position();
                cb(id, 7, p.X, p.Y, a.Cumulative().Scale, 0);
            });
        el.PointerWheelChanged(
            [id, cb, el](WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& a) {
                if ((a.KeyModifiers() & WS::VirtualKeyModifiers::Control) ==
                    WS::VirtualKeyModifiers::None)
                    return;
                auto pt = a.GetCurrentPoint(el);
                auto p = pt.Position();
                // A notch is 120: about a tenth of a step per notch, the touchpad's smaller
                // deltas proportionally less.
                double factor = std::exp(pt.Properties().MouseWheelDelta() / 1200.0);
                cb(id, 6, p.X, p.Y, factor, 0);
                a.Handled(true);
            });
    } else if (kind == 7) {
        // Rotate → the same manipulation's rotation (two fingers, or a precision touchpad),
        // in degrees clockwise and cumulative since the manipulation started, reported in
        // radians from a Began that waits for the angle to leave zero, as the pinch waits for
        // its scale: a one-finger drag or a pure pinch through the same manipulation must not
        // come out as a rotation (docs/canvas.md "Zoom and pan").
        day_add_manipulation_mode(el, WUXIn::ManipulationModes::Rotate);
        constexpr double kRadians = 3.14159265358979323846 / 180.0;
        auto active = std::make_shared<bool>(false);
        el.ManipulationStarted(
            [active](WF::IInspectable const&, WUXIn::ManipulationStartedRoutedEventArgs const&) {
                *active = false;
            });
        el.ManipulationDelta(
            [id, cb, active](WF::IInspectable const&, WUXIn::ManipulationDeltaRoutedEventArgs const& a) {
                auto p = a.Position();
                auto r = a.Cumulative().Rotation;
                if (!*active) {
                    if (!day_manipulation_rotated(r)) return;
                    *active = true;
                    cb(id, 13, p.X, p.Y, 0.0, 0);
                }
                cb(id, 14, p.X, p.Y, r * kRadians, 0);
            });
        el.ManipulationCompleted(
            [id, cb, active](WF::IInspectable const&, WUXIn::ManipulationCompletedRoutedEventArgs const& a) {
                if (!*active) return;
                *active = false;
                auto p = a.Position();
                cb(id, 15, p.X, p.Y, a.Cumulative().Rotation * kRadians, 0);
            });
    } else if (kind == 5 || kind == 6) {
        // Pan → wheel ticks (a mouse's notches, a precision touchpad's two-finger scroll), each
        // a lone Changed with the tick's delta as a CONTENT displacement: a wheel rolled toward
        // the user (a negative delta) moves content up, as on every other desktop. A tilt
        // wheel, or shift with a vertical one, pans sideways. ctrl+wheel is the pinch's. Over
        // a container (kind 6) only the sideways ticks are taken; a vertical one goes on to
        // scroll whatever scrolls, as on the other desktops.
        const bool sideways_only = kind == 6;
        el.PointerWheelChanged(
            [id, cb, el, sideways_only](WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& a) {
                auto mods = a.KeyModifiers();
                if ((mods & WS::VirtualKeyModifiers::Control) != WS::VirtualKeyModifiers::None)
                    return;
                auto pt = a.GetCurrentPoint(el);
                auto props = pt.Properties();
                auto p = pt.Position();
                double step = props.MouseWheelDelta() / 120.0 * 40.0; // GTK's 40 px per notch
                if (props.IsHorizontalMouseWheel()) {
                    // A tilt to the right scrolls right, which moves content left.
                    cb(id, 8, p.X, p.Y, -step, 0);
                } else if ((mods & WS::VirtualKeyModifiers::Shift) != WS::VirtualKeyModifiers::None) {
                    cb(id, 8, p.X, p.Y, step, 0);
                } else if (sideways_only) {
                    return;
                } else {
                    cb(id, 8, p.X, p.Y, 0, step);
                }
                a.Handled(true);
            });
    } else if (kind == 3) {
        // Hover → the pointer events, gated to a real pointer: WinUI reports a touch contact
        // through the same PointerEntered/Moved/Exited, and reporting those as hovers would make
        // every tap look like one (docs/canvas.md "Interaction"). Phases 10/11/12.
        auto lastX = std::make_shared<double>(0.0);
        auto lastY = std::make_shared<double>(0.0);
        auto is_pointer = [](WUXIn::PointerRoutedEventArgs const& a) {
            auto t = a.Pointer().PointerDeviceType();
            return t == DayPointerDeviceType::Mouse ||
                   t == DayPointerDeviceType::Pen;
        };
        el.PointerEntered([id, cb, el, lastX, lastY, is_pointer](
                              WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& a) {
            if (!is_pointer(a)) return;
            auto p = a.GetCurrentPoint(el).Position();
            *lastX = p.X; *lastY = p.Y;
            cb(id, 10, p.X, p.Y, 0, 0);
        });
        el.PointerMoved([id, cb, el, lastX, lastY, is_pointer](
                            WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& a) {
            if (!is_pointer(a)) return;
            auto p = a.GetCurrentPoint(el).Position();
            *lastX = p.X; *lastY = p.Y;
            cb(id, 11, p.X, p.Y, 0, 0);
        });
        el.PointerExited([id, cb, lastX, lastY, is_pointer](
                             WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& a) {
            if (!is_pointer(a)) return;
            // The exit's own point can already be outside; the contract is the last one inside.
            cb(id, 12, *lastX, *lastY, 0, 0);
        });
    } else if (kind == 1) {
        // LongPress → Holding for touch and pen (the system's delay and slop, fired once on
        // Started), plus a held left mouse button, which XAML does not report as holding: 500 ms
        // from the press, cancelled by the release or by a move past the drag threshold. The
        // release after a mouse long press is not a tap; a touch hold raises no Tapped at all.
        // An element with a context menu keeps its long press for the menu (docs/gestures.md).
        auto g = gestures_of(h);
        if (g->hold) return;
        g->hold = true;
        el.IsHoldingEnabled(true);
        el.Holding([id, cb, el](WF::IInspectable const&, WUXIn::HoldingRoutedEventArgs const& a) {
            if (a.HoldingState() != WUIIn::HoldingState::Started) return;
            if (el.ContextFlyout()) return;
            auto p = a.GetPosition(el);
            cb(id, 4, p.X, p.Y, 0, 0);
        });
        day_observe(el, UIElement::PointerPressedEvent(),
                    [id, cb, el, g](WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& a) {
                        g->suppress_tap = false;
                        if (a.Pointer().PointerDeviceType() != DayPointerDeviceType::Mouse) return;
                        auto pt = a.GetCurrentPoint(el);
                        if (!pt.Properties().IsLeftButtonPressed()) return;
                        g->mouse_down = true;
                        g->mouse_at = pt.Position();
                        const unsigned gen = ++g->hold_generation;
                        day_after(500, [id, cb, el, g, gen] {
                            if (gen != g->hold_generation || !g->mouse_down) return;
                            if (el.ContextFlyout()) return;
                            g->suppress_tap = true;
                            cb(id, 4, g->mouse_at.X, g->mouse_at.Y, 0, 0);
                        });
                    });
        day_observe(el, UIElement::PointerMovedEvent(),
                    [el, g](WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& a) {
                        if (!g->mouse_down) return;
                        auto p = a.GetCurrentPoint(el).Position();
                        const double sx = GetSystemMetrics(SM_CXDRAG), sy = GetSystemMetrics(SM_CYDRAG);
                        if (std::fabs(p.X - g->mouse_at.X) > sx || std::fabs(p.Y - g->mouse_at.Y) > sy) {
                            g->mouse_down = false;
                            ++g->hold_generation;
                        }
                    });
        auto up = [g](WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const&) {
            g->mouse_down = false;
            ++g->hold_generation;
        };
        day_observe(el, UIElement::PointerReleasedEvent(), up);
        day_observe(el, UIElement::PointerCanceledEvent(), up);
        day_observe(el, UIElement::PointerCaptureLostEvent(), up);
    } else if (kind == 9) {
        // Press → the pointer itself, observed (handledEventsToo, never marked handled): in at the
        // primary down, out exactly once, at the release, a cancel, a lost capture, or the
        // pointer leaving the bounds grown by the hit slop (docs/gestures.md "Press").
        auto g = gestures_of(h);
        if (g->press) return;
        g->press = true;
        auto out = [id, cb, g]() {
            if (!g->pressed) return;
            g->pressed = false;
            cb(id, 18, g->press_at.X, g->press_at.Y, 0, 0);
        };
        day_observe(el, UIElement::PointerPressedEvent(),
                    [id, cb, el, g](WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& a) {
                        if (g->pressed || !day_primary_down(a, el)) return;
                        g->pressed = true;
                        g->press_pointer = a.Pointer().PointerId();
                        g->press_at = a.GetCurrentPoint(el).Position();
                        cb(id, 17, g->press_at.X, g->press_at.Y, 0, 0);
                    });
        day_observe(el, UIElement::PointerMovedEvent(),
                    [el, g, out](WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& a) {
                        if (!g->pressed || a.Pointer().PointerId() != g->press_pointer) return;
                        auto p = a.GetCurrentPoint(el).Position();
                        if (!day_inside(el, *g, p)) { out(); return; }
                        g->press_at = p;
                    });
        auto end = [g, out](WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& a) {
            if (g->pressed && a.Pointer().PointerId() == g->press_pointer) out();
        };
        day_observe(el, UIElement::PointerReleasedEvent(), end);
        day_observe(el, UIElement::PointerCanceledEvent(), end);
        day_observe(el, UIElement::PointerCaptureLostEvent(), end);
        day_observe(el, UIElement::PointerExitedEvent(), end);
        if (auto fe = el.try_as<FrameworkElement>())
            fe.Unloaded([out](WF::IInspectable const&, WUX::RoutedEventArgs const&) { out(); });
    } else if (kind == 10) {
        // Touches → every contact that went down on the element, by pointer id (the mouse is
        // contact 0), captured so a finger that slides off keeps reporting until it lifts
        // (docs/gestures.md "Touches"). Observed like the press.
        auto g = gestures_of(h);
        if (g->touches) return;
        g->touches = true;
        auto contact_id = [](WUXIn::PointerRoutedEventArgs const& a) -> uint32_t {
            return a.Pointer().PointerDeviceType() == DayPointerDeviceType::Mouse ? 0u
                                                                                  : a.Pointer().PointerId();
        };
        auto read = [el](WUXIn::PointerRoutedEventArgs const& a, DayGestures::Contact& c) {
            auto pt = a.GetCurrentPoint(el);
            c.x = pt.Position().X;
            c.y = pt.Position().Y;
            c.kind = day_pointer_kind(a);
            // A mouse has no pressure: 1.0 while its button is down. Touch and pen report
            // their own (a digitizer without pressure says 0.5).
            c.pressure = c.kind == 1 ? 1.0 : pt.Properties().Pressure();
        };
        day_observe(el, UIElement::PointerPressedEvent(),
                    [id, el, g, contact_id, read](WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& a) {
                        if (!day_primary_down(a, el)) return;
                        DayGestures::Contact c{};
                        read(a, c);
                        c.phase = 0;
                        g->contacts[contact_id(a)] = c;
                        el.CapturePointer(a.Pointer());
                        day_report_touches(id, *g);
                    });
        day_observe(el, UIElement::PointerMovedEvent(),
                    [id, g, contact_id, read](WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& a) {
                        auto it = g->contacts.find(contact_id(a));
                        if (it == g->contacts.end()) return;
                        read(a, it->second);
                        it->second.phase = 1;
                        day_report_touches(id, *g);
                    });
        auto finish = [id, g, contact_id, read](WUXIn::PointerRoutedEventArgs const& a, int phase) {
            auto it = g->contacts.find(contact_id(a));
            if (it == g->contacts.end()) return;
            read(a, it->second);
            it->second.phase = phase;
            day_report_touches(id, *g);
        };
        day_observe(el, UIElement::PointerReleasedEvent(),
                    [finish](WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& a) { finish(a, 3); });
        day_observe(el, UIElement::PointerCanceledEvent(),
                    [finish](WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& a) { finish(a, 4); });
        day_observe(el, UIElement::PointerCaptureLostEvent(),
                    [finish](WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& a) { finish(a, 4); });
    } else if (kind == 0 || kind == 8) {
        // Tap and DoubleTap share one Tapped/DoubleTapped pair, whichever is enabled first
        // (docs/gestures.md "Double tap"). XAML raises Tapped for the first click of a double and
        // DoubleTapped for the second (no second Tapped): with both enabled the single waits the
        // system's double-click time and the double cancels it; with Tap alone the second click
        // of a double is still a tap; with DoubleTap alone nothing waits.
        auto g = gestures_of(h);
        const bool wired = g->tap || g->dbl;
        if (kind == 0) g->tap = true; else g->dbl = true;
        el.IsDoubleTapEnabled(true);
        if (wired) return;
        el.Tapped([id, cb, el, g](WF::IInspectable const&, WUXIn::TappedRoutedEventArgs const& a) {
            if (g->suppress_tap) { g->suppress_tap = false; return; }
            if (!g->tap) return;
            auto p = a.GetPosition(el);
            if (!g->dbl) { cb(id, 0, p.X, p.Y, 0, 0); return; }
            const unsigned gen = ++g->tap_generation;
            day_after(GetDoubleClickTime(), [id, cb, g, gen, p] {
                if (gen == g->tap_generation) cb(id, 0, p.X, p.Y, 0, 0);
            });
        });
        el.DoubleTapped([id, cb, el, g](WF::IInspectable const&, WUXIn::DoubleTappedRoutedEventArgs const& a) {
            ++g->tap_generation;
            auto p = a.GetPosition(el);
            if (g->dbl) cb(id, 16, p.X, p.Y, 0, 0);
            else if (g->tap) cb(id, 0, p.X, p.Y, 0, 0);
        });
    }
} catch (...) {
}


// `.focusable()` (docs/focus.md "Focusable pieces"). WinUI 3 gave every UIElement a tab stop, so a
// container panel joins the focus order where it sits in the tree, takes keyboard focus with the
// system focus visual when the keyboard brought it, is focused by a primary press that did not land
// on a focusable control inside it (before the press reaches its recognizers, and without
// consuming it), reports its own focus changes, and routes keys exactly as a canvas does. A
// control (the canvas's host included) is focusable already. System XAML has no such tab stop:
// the call is a no-op there.
void day_xaml_set_focusable(void* h, unsigned long long id, int (*handles)(unsigned long long),
                            int (*handles_text)(unsigned long long),
                            void (*key_cb)(unsigned long long, const char*, const char*, int),
                            void (*focus_cb)(unsigned long long, int)) try {
#ifdef DAY_WINUI
    auto el = elem(h);
    // A press focuses it, anywhere in its box.
    day_hit_on(h);
    if (auto c = el.try_as<WUXC::Control>()) {
        c.IsTabStop(true);
        return;
    }
    if (g_key_states.count(h)) return;
    el.IsTabStop(true);
    el.UseSystemFocusVisuals(true);
    day_attach_keys(el, h, id, handles, handles_text, key_cb);
    day_observe(el, UIElement::PointerPressedEvent(),
                [el](WF::IInspectable const&, WUXIn::PointerRoutedEventArgs const& a) {
                    if (!day_primary_down(a, el)) return;
                    auto d = a.OriginalSource().try_as<WUX::DependencyObject>();
                    for (; d && !day_same(d, el); d = WUXM::VisualTreeHelper::GetParent(d)) {
                        if (auto c = d.try_as<WUXC::Control>())
                            if (c.IsTabStop() && c.IsEnabled()) return;
                    }
                    el.Focus(WUX::FocusState::Pointer);
                });
    el.GotFocus([id, focus_cb, el](WF::IInspectable const&, WUX::RoutedEventArgs const& a) {
        if (day_same(a.OriginalSource(), el)) focus_cb(id, 1);
    });
    el.LostFocus([id, focus_cb, el](WF::IInspectable const&, WUX::RoutedEventArgs const& a) {
        if (day_same(a.OriginalSource(), el)) focus_cb(id, 0);
    });
#else
    (void)h; (void)id; (void)handles; (void)handles_text; (void)key_cb; (void)focus_cb;
#endif
} catch (...) {
}

} // extern "C"
extern "C++" {
// Subtree enablement (docs/forms.md "Disabling a group"). Each disabled root a node sits under
// counts once, so a node under two disabled groups stays disabled until both are enabled again;
// a node is put back only by the call that changed it, and only if that call changed it (a
// control already disabled on its own stays disabled).
static std::map<void*, int> g_disabled_depth;       // element (ABI) → disabled roots above it
static std::map<void*, int> g_disabled_flags;       // what the first disabling call turned off
constexpr int kDisabledControl = 1, kDisabledTabStop = 2, kDisabledHit = 4;

static void day_subtree_enable(WUX::DependencyObject const& d, bool enabled, bool root) {
    auto el = d.try_as<UIElement>();
    if (el) {
        void* key = winrt::get_abi(el.as<WF::IUnknown>());
        auto c = el.try_as<WUXC::Control>();
        if (!enabled) {
            if (g_disabled_depth[key]++ == 0) {
                int flags = 0;
                if (c && c.IsEnabled()) { c.IsEnabled(false); flags |= kDisabledControl; }
                // A focusable panel leaves the focus order (a control does by being disabled).
                // System XAML has no tab stop on a plain element (nor focusable panels).
#ifdef DAY_WINUI
                if (!c && el.IsTabStop()) { el.IsTabStop(false); flags |= kDisabledTabStop; }
#endif
                // The root stops pointer input to everything under it.
                if (root && el.IsHitTestVisible()) { el.IsHitTestVisible(false); flags |= kDisabledHit; }
                g_disabled_flags[key] = flags;
            }
        } else {
            auto it = g_disabled_depth.find(key);
            if (it != g_disabled_depth.end() && --it->second <= 0) {
                g_disabled_depth.erase(it);
                const int flags = g_disabled_flags[key];
                g_disabled_flags.erase(key);
                if ((flags & kDisabledControl) && c) c.IsEnabled(true);
#ifdef DAY_WINUI
                if (flags & kDisabledTabStop) el.IsTabStop(true);
#endif
                if (flags & kDisabledHit) el.IsHitTestVisible(true);
            }
        }
        // A control's own IsEnabled already cascades to everything inside it.
        if (c) return;
    }
    const int n = WUXM::VisualTreeHelper::GetChildrenCount(d);
    for (int i = 0; i < n; ++i) day_subtree_enable(WUXM::VisualTreeHelper::GetChild(d, i), enabled, false);
}
} // extern "C++"
extern "C" {
void day_xaml_set_subtree_enabled(void* h, int enabled) try {
    auto el = elem(h);
    if (!enabled) {
        // Focus inside the group is resigned first, while the focused element can still say so.
#ifdef DAY_WINUI
        if (auto root = el.XamlRoot()) {
            for (auto f = WUXIn::FocusManager::GetFocusedElement(root).try_as<WUX::DependencyObject>(); f;
                 f = WUXM::VisualTreeHelper::GetParent(f)) {
                if (day_same(f, el)) {
                    if (g_app && g_app->focus_sink) {
                        g_app->focus_sink.IsTabStop(true);
                        g_app->focus_sink.Focus(WUX::FocusState::Programmatic);
                        g_app->focus_sink.IsTabStop(false);
                    }
                    break;
                }
            }
        }
#endif
    }
    day_subtree_enable(el, enabled != 0, true);
} catch (...) {
}

// The kind of device behind the latest pointer input (docs/gestures.md "Input kind"): 0 touch,
// 1 mouse, 2 stylus. At launch a touch screen in tablet posture is touch, everything else mouse;
// after that every press, and every move of a hovering device, on any window reports its kind
// when it changed.
int day_xaml_primary_pointer() {
    if (g_pointer_kind < 0) {
        const bool touch = GetSystemMetrics(SM_MAXIMUMTOUCHES) > 0 &&
                           GetSystemMetrics(SM_CONVERTIBLESLATEMODE) == 0;
        g_pointer_kind = touch ? 0 : 1;
    }
    return g_pointer_kind;
}
void day_xaml_set_pointer_kind_cb(void (*cb)(int)) { g_pointer_kind_cb = cb; }
