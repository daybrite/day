// The Qt half of day-tweak-slider-tickmarks — the canonical bring-your-own-C++ tweak recipe
// (docs/tweaks.md): a tweak crate compiles its own few lines of Qt against the raw `QWidget*`
// that `day_qt::with_native_raw` hands out. Qt itself is already linked by day-qt-sys.
//
// `cls` is the native class name Day realized for the node (here "QSlider"). Rust can't
// introspect the opaque pointer, so it tells us what it is — we guard on it and only then cast,
// instead of a blind `static_cast` that would be undefined behaviour on a mis-applied tweak.
//
// Contract (same as every tweak): the widget is owned by Day/Qt — mutate, never delete or
// reparent; main thread only.

#include <QtWidgets/QSlider>
#include <cstring>

extern "C" void day_tweak_slider_ticks_qt(void* w, const char* cls, int interval, int position) {
    if (!w || !cls || std::strcmp(cls, "QSlider") != 0) return;
    auto* s = static_cast<QSlider*>(w);
    switch (position) {
        case 1: s->setTickPosition(QSlider::TicksAbove); break;
        case 2: s->setTickPosition(QSlider::TicksBothSides); break;
        default: s->setTickPosition(QSlider::TicksBelow); break;
    }
    s->setTickInterval(interval > 0 ? interval : 100);
}
