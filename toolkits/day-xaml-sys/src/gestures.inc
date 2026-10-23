// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Included by shim.cpp inside the C ABI block. Shared declarations and include order live there.

// Attach a native pointer recognizer to `h`, reporting to cb(id, phase, x, y, tx, ty) with `x,y` in
// the element's local space (docs/shapes.md). kind 0 = Tap, 1 = LongPress, 2 = Drag. Phase codes:
// 0 Tap, 1 Drag-Began, 2 Drag-Changed, 3 Drag-Ended, 4 LongPress (matching day-winui's on_gesture).
// Routed events bubble from the shape's Path children up to the Canvas handle, so only the drawn
// shape is hit (a transparent Canvas isn't hit-testable) — the "hit-test the path" semantic.
void day_xaml_enable_gesture(void* h, unsigned long long id, int kind,
                              void (*cb)(unsigned long long, int, double, double, double,
                                         double)) try {
    auto el = elem(h);
    if (kind == 2) { // Drag → manipulation (translate only)
        el.ManipulationMode(WUXIn::ManipulationModes::TranslateX |
                            WUXIn::ManipulationModes::TranslateY);
        el.ManipulationStarted(
            [id, cb](WF::IInspectable const&, WUXIn::ManipulationStartedRoutedEventArgs const& a) {
                auto p = a.Position();
                cb(id, 1, p.X, p.Y, 0, 0);
            });
        el.ManipulationDelta(
            [id, cb](WF::IInspectable const&, WUXIn::ManipulationDeltaRoutedEventArgs const& a) {
                auto p = a.Position();
                auto t = a.Cumulative().Translation;
                cb(id, 2, p.X, p.Y, t.X, t.Y);
            });
        el.ManipulationCompleted(
            [id, cb](WF::IInspectable const&, WUXIn::ManipulationCompletedRoutedEventArgs const& a) {
                auto p = a.Position();
                auto t = a.Cumulative().Translation;
                cb(id, 3, p.X, p.Y, t.X, t.Y);
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

