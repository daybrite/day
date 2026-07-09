// The Qt half of day-tweak-slider-tickmarks — the canonical bring-your-own-C++ tweak recipe
// (docs/tweaks.md): a tweak crate compiles its own few lines of Qt against the raw `QWidget*`
// that `day_qt::with_native_raw` hands out. Qt itself is already linked by day-qt-sys.
//
// Contract (same as every tweak): the widget is owned by Day/Qt — mutate, never delete or
// reparent; main thread only.

#include <QtWidgets/QSlider>

extern "C" void day_tweak_slider_ticks_qt(void* w, int interval, int position) {
    auto* s = static_cast<QSlider*>(w);
    if (!s) return;
    switch (position) {
        case 1: s->setTickPosition(QSlider::TicksAbove); break;
        case 2: s->setTickPosition(QSlider::TicksBothSides); break;
        default: s->setTickPosition(QSlider::TicksBelow); break;
    }
    s->setTickInterval(interval > 0 ? interval : 100);
}
