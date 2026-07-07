// The activity piece's own Qt shim behind a flat C ABI. Qt ships no native spinner widget, so the
// idiomatic indeterminate indicator is a QProgressBar in busy mode (min == max == 0) — the same
// approach day-qt-sys uses for `spinner()` (docs/progress.md). Only Qt6Widgets is needed (day-qt-sys
// already links it), so build.rs compiles this with the Qt6Widgets --cflags and emits no extra libs.
//
// Animating toggles the range: 0..0 runs the busy animation; 0..1 with value 0 freezes it as a
// static empty bar (so a stopped spinner stays on screen, matching the other backends). `.large`
// gives the widget a bigger minimum size.

#include <QProgressBar>

extern "C" {

void *day_activity_qt_new(int large) {
    QProgressBar *b = new QProgressBar();
    b->setTextVisible(false);
    b->setRange(0, 0); // busy / indeterminate animation
    b->setMinimumHeight(large ? 28 : 18);
    if (large)
        b->setMinimumWidth(96);
    return b;
}

void day_activity_qt_set_animating(void *w, int on) {
    QProgressBar *b = static_cast<QProgressBar *>(w);
    if (on) {
        b->setRange(0, 0); // resume the busy animation
    } else {
        b->setRange(0, 1); // freeze: a static, non-animating bar
        b->setValue(0);
    }
}

} // extern "C"
