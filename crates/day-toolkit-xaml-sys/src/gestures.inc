// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Included by shim.cpp inside the C ABI block. Shared declarations and include order live there.

// Attach a native pointer recognizer to `h`, reporting to cb(id, phase, x, y, tx, ty) with `x,y` in
// the element's local space (docs/shapes.md). kind 0 = Tap, 1 = LongPress, 2 = Drag, 3 = Hover,
// 4 = Pinch, 5 = Pan on a canvas (every wheel tick), 6 = Pan on any other container (sideways
// ticks only; vertical ones keep scrolling natively), 7 = Rotate. Phase codes: 0 Tap, 1
// Drag-Began, 2 Drag-Changed, 3 Drag-Ended, 4 LongPress,
// 5/6/7 Pinch Began/Changed/Ended (tx = the scale), 8 Pan (tx,ty = the delta), 10/11/12 Hover,
// 13/14/15 Rotate Began/Changed/Ended (tx = cumulative radians, clockwise positive)
// (matching day-toolkit-winui's on_gesture).
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

void day_xaml_enable_gesture(void* h, unsigned long long id, int kind,
                              void (*cb)(unsigned long long, int, double, double, double,
                                         double)) try {
    auto el = elem(h);
    if (kind == 2) { // Drag → manipulation (translate)
        day_add_manipulation_mode(el, WUXIn::ManipulationModes::TranslateX |
                                          WUXIn::ManipulationModes::TranslateY);
        // Once the same manipulation scales or turns (a second finger on a pinch or rotate
        // target), the drag ends where it was and the rest of the manipulation is theirs: the
        // two fingers never come out as drags (docs/canvas.md "Zoom and pan").
        auto ended = std::make_shared<bool>(false);
        el.ManipulationStarted(
            [id, cb, ended](WF::IInspectable const&, WUXIn::ManipulationStartedRoutedEventArgs const& a) {
                *ended = false;
                auto p = a.Position();
                cb(id, 1, p.X, p.Y, 0, 0);
            });
        el.ManipulationDelta(
            [id, cb, ended](WF::IInspectable const&, WUXIn::ManipulationDeltaRoutedEventArgs const& a) {
                if (*ended) return;
                auto p = a.Position();
                auto t = a.Cumulative().Translation;
                if (day_manipulation_scaled(a.Cumulative().Scale) ||
                    day_manipulation_rotated(a.Cumulative().Rotation)) {
                    *ended = true;
                    cb(id, 3, p.X, p.Y, t.X, t.Y);
                    return;
                }
                cb(id, 2, p.X, p.Y, t.X, t.Y);
            });
        el.ManipulationCompleted(
            [id, cb, ended](WF::IInspectable const&, WUXIn::ManipulationCompletedRoutedEventArgs const& a) {
                if (*ended) return;
                auto p = a.Position();
                auto t = a.Cumulative().Translation;
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
    } else if (kind == 1) { // LongPress → Holding (touch/pen; fire once on Started)
        el.IsHoldingEnabled(true);
        el.Holding([id, cb, el](WF::IInspectable const&, WUXIn::HoldingRoutedEventArgs const& a) {
            if (a.HoldingState() == WUIIn::HoldingState::Started) {
                auto p = a.GetPosition(el);
                cb(id, 4, p.X, p.Y, 0, 0);
            }
        });
    } else { // Tap
        el.Tapped([id, cb, el](WF::IInspectable const&, WUXIn::TappedRoutedEventArgs const& a) {
            auto p = a.GetPosition(el);
            cb(id, 0, p.X, p.Y, 0, 0);
        });
    }
} catch (...) {
}

