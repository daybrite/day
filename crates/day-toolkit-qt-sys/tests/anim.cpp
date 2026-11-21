// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//
// The animation runs (docs/tween.md "The animation contract") on Qt's animation framework,
// each put at a known moment by the test through QAbstractAnimation::setCurrentTime: the delay holds the start, repeats restart, autoreverse plays back, a run that
// repeats for ever stops on a plain set, a retarget starts from the value shown, a spring's
// samples overshoot, the frame and the fill pass between their ends, and each lands exactly on
// its target. The values are read through `day_qt_read_displayed`, as `Toolkit::read_displayed`
// reads them.
#include <QAbstractAnimation>
#include <QApplication>
#include <QColor>
#include <QGraphicsEffect>
#include <QImage>
#include <QPointer>
#include <QScrollArea>
#include <QWidget>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <vector>

extern "C" {
void day_qt_open_file(const char *) {}
struct DayQtAnim {
    int delay_ms;
    int duration_ms;
    int cycles;
    int autoreverse;
    const double *keys;
    int n_keys;
};
struct DayQtDisplayed {
    double opacity, tx, ty;
    int frame_ok;
    double x, y, w, h;
    int fill_ok;
    double r, g, b, a;
};
void day_qt_set_opacity(void *, double, const DayQtAnim *);
void day_qt_set_transform(void *, double, double, double, double, double, const DayQtAnim *);
void day_qt_set_frame(void *, int, int, int, int, const DayQtAnim *);
void day_qt_widget_set_surface(void *, double, double, double, double, double, int);
void day_qt_widget_set_bg(void *, double, double, double, double, const DayQtAnim *);
void day_qt_read_displayed(void *, DayQtDisplayed *);
void day_qt_widget_set_styled_surface(void *, const double *, const double *, int);
}

#define CHECK(cond)                                                                              \
    do {                                                                                         \
        if (!(cond)) {                                                                           \
            std::fprintf(stderr, "%s:%d: CHECK failed: %s\n", __FILE__, __LINE__, #cond);        \
            std::abort();                                                                        \
        }                                                                                        \
    } while (0)

#define NEAR(a, b, tol)                                                                          \
    do {                                                                                         \
        const double va = (a), vb = (b);                                                         \
        if (!(std::fabs(va - vb) <= (tol))) {                                                    \
            std::fprintf(stderr, "%s:%d: %s = %f, expected %f\n", __FILE__, __LINE__, #a, va,   \
                         vb);                                                                    \
            std::abort();                                                                        \
        }                                                                                        \
    } while (0)

// The run a call just started on `channel` of `holder`, paused so that the test, not the
// wall clock, sets its time: `at(ms)` puts the run (its pause, its looped cycles, its end) at
// `ms` after the call, through Qt's own `QAbstractAnimation::setCurrentTime`.
static QPointer<QAbstractAnimation> run_;

static void begin(QObject *holder, const char *channel) {
    const auto runs = holder->findChildren<QAbstractAnimation *>(QString::fromLatin1(channel),
                                                                 Qt::FindDirectChildrenOnly);
    CHECK(runs.size() == 1);
    run_ = runs.first();
    CHECK(run_->state() == QAbstractAnimation::Running);
    run_->pause();
}

static void at(int ms) {
    CHECK(run_);
    run_->setCurrentTime(ms);
}

// A plain set stopped the run in flight for good, and left none on the channel.
static void stopped(QObject *holder, const char *channel) {
    CHECK(run_ && run_->state() == QAbstractAnimation::Stopped);
    CHECK(holder->findChildren<QAbstractAnimation *>(QString::fromLatin1(channel),
                                                    Qt::FindDirectChildrenOnly)
              .isEmpty());
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    CHECK(!run_);
}

static void begin_opacity(QWidget *w) { begin(w->graphicsEffect(), "dayOpacity"); }
static void begin_transform(QWidget *w) { begin(w->graphicsEffect(), "dayTransform"); }

static DayQtDisplayed shown(QWidget *w) {
    DayQtDisplayed d;
    day_qt_read_displayed(w, &d);
    return d;
}

static const double LINEAR[] = {0.0, 1.0};

static DayQtAnim linear(int duration, int delay = 0, int cycles = 1, bool reverse = false) {
    return DayQtAnim{delay, duration, cycles, reverse ? 1 : 0, LINEAR, 2};
}

// Rule 1: the start holds through the delay, once.
static void delay_holds_the_start(QWidget *w) {
    day_qt_set_opacity(w, 1.0, nullptr);
    const DayQtAnim a = linear(600, 900);
    day_qt_set_opacity(w, 0.0, &a);
    begin_opacity(w);
    at(450);
    NEAR(shown(w).opacity, 1.0, 1e-9);
    at(1200);
    NEAR(shown(w).opacity, 0.5, 0.03);
    at(1600);
    NEAR(shown(w).opacity, 0.0, 0.0);
}

// Rule 3: each cycle restarts from the start; rule 5: the run rests on the target.
static void repeat_restarts(QWidget *w) {
    day_qt_set_opacity(w, 1.0, nullptr);
    const DayQtAnim a = linear(800, 0, 3);
    day_qt_set_opacity(w, 0.0, &a);
    begin_opacity(w);
    at(400);
    NEAR(shown(w).opacity, 0.5, 0.03);
    at(1200);
    NEAR(shown(w).opacity, 0.5, 0.03);
    at(2000);
    NEAR(shown(w).opacity, 0.5, 0.03);
    at(2500);
    NEAR(shown(w).opacity, 0.0, 0.0);
}

// Rule 4: the second cycle plays back; with an even count the run ends on the start and then
// rests on the target (rule 5).
static void autoreverse_plays_back(QWidget *w) {
    day_qt_set_opacity(w, 1.0, nullptr);
    const DayQtAnim three = linear(800, 0, 3, true);
    day_qt_set_opacity(w, 0.0, &three);
    begin_opacity(w);
    at(400);
    NEAR(shown(w).opacity, 0.5, 0.03);
    at(1100); // 300 ms into the backward cycle: progress 0.625
    NEAR(shown(w).opacity, 0.375, 0.03);
    at(2000); // 400 ms into the third, forward again
    NEAR(shown(w).opacity, 0.5, 0.03);
    at(2500);
    NEAR(shown(w).opacity, 0.0, 0.0);

    day_qt_set_opacity(w, 1.0, nullptr);
    const DayQtAnim two = linear(800, 0, 2, true);
    day_qt_set_opacity(w, 0.0, &two);
    begin_opacity(w);
    at(1500); // almost back at the start
    NEAR(shown(w).opacity, 0.875, 0.03);
    at(1700);
    NEAR(shown(w).opacity, 0.0, 0.0);
}

// Rules 3, 4, 7: a run for ever keeps going until a plain set, which lands at once and stays.
static void forever_until_set(QWidget *w) {
    day_qt_set_opacity(w, 1.0, nullptr);
    const DayQtAnim a = linear(600, 100, -1, true);
    day_qt_set_opacity(w, 0.0, &a);
    begin_opacity(w);
    at(50);
    NEAR(shown(w).opacity, 1.0, 1e-9);
    at(400); // 300 ms into the first cycle
    NEAR(shown(w).opacity, 0.5, 0.03);
    at(2800); // 2700 ms after the delay: the fifth cycle (forward), 300 ms in
    NEAR(shown(w).opacity, 0.5, 0.03);
    at(3050); // 550 ms in
    NEAR(shown(w).opacity, 1.0 - 550.0 / 600.0, 0.03);
    day_qt_set_opacity(w, 0.5, nullptr);
    NEAR(shown(w).opacity, 0.5, 0.0);
    stopped(w->graphicsEffect(), "dayOpacity");

    // Without autoreverse too, and on the transform channel.
    day_qt_set_transform(w, 0, 0, 1, 1, 0, nullptr);
    const DayQtAnim b = linear(500, 0, -1);
    day_qt_set_transform(w, 100, 0, 1, 1, 0, &b);
    begin_transform(w);
    at(1250);
    NEAR(shown(w).tx, 50.0, 3.0);
    day_qt_set_transform(w, 20, 10, 1, 1, 0, nullptr);
    stopped(w->graphicsEffect(), "dayTransform");
    NEAR(shown(w).tx, 20.0, 0.0);
    NEAR(shown(w).ty, 10.0, 0.0);
}

// Rule 7: a change in flight starts from the value shown.
static void retarget_from_shown(QWidget *w) {
    day_qt_set_opacity(w, 1.0, nullptr);
    const DayQtAnim a = linear(1000);
    day_qt_set_opacity(w, 0.0, &a);
    begin_opacity(w);
    at(500);
    const double turned = shown(w).opacity;
    NEAR(turned, 0.5, 0.03);
    day_qt_set_opacity(w, 1.0, &a);
    begin_opacity(w);
    NEAR(shown(w).opacity, turned, 0.02);
    at(250);
    NEAR(shown(w).opacity, turned + (1.0 - turned) * 0.25, 0.03);
    at(1100);
    NEAR(shown(w).opacity, 1.0, 0.0);
}

// Rule 6: a spring's samples are played as key values, overshoot included, and the run lands
// exactly on the target. Five samples a quarter cycle apart stand for AnimSpec::keyframes().
static void spring_samples_overshoot(QWidget *w) {
    day_qt_set_transform(w, 0, 0, 1, 1, 0, nullptr);
    static const double keys[] = {0.0, 1.44, 0.8, 1.05, 1.0};
    const DayQtAnim a{0, 1000, 1, 0, keys, 5};
    day_qt_set_transform(w, 100, 0, 1, 1, 0, &a);
    begin_transform(w);
    at(250);
    NEAR(shown(w).tx, 144.0, 3.0);
    at(375); // half way between the first peak and the trough
    NEAR(shown(w).tx, 112.0, 3.0);
    at(500);
    NEAR(shown(w).tx, 80.0, 3.0);
    at(1100);
    NEAR(shown(w).tx, 100.0, 0.0);
    // A backward spring cycle reads the samples in reverse.
    day_qt_set_transform(w, 0, 0, 1, 1, 0, nullptr);
    const DayQtAnim pair{0, 1000, 2, 1, keys, 5};
    day_qt_set_transform(w, 100, 0, 1, 1, 0, &pair);
    begin_transform(w);
    at(1750); // 750 ms into the backward cycle = the forward cycle at 250 ms
    NEAR(shown(w).tx, 144.0, 3.0);
    at(1999); // back at the start, then resting on the target (rule 5)
    NEAR(shown(w).tx, 0.0, 1.0);
    at(2000);
    NEAR(shown(w).tx, 100.0, 0.0);
}

// A zero-length cycle after a delay: the start, then the target.
static void delay_then_jump(QWidget *w) {
    day_qt_set_opacity(w, 1.0, nullptr);
    const DayQtAnim a = linear(0, 300);
    day_qt_set_opacity(w, 0.25, &a);
    begin_opacity(w);
    at(200);
    NEAR(shown(w).opacity, 1.0, 0.0);
    at(400);
    NEAR(shown(w).opacity, 0.25, 0.0);
}

// The frame passes between its ends and lands on the target.
static void frame_passes_between(QWidget *w) {
    day_qt_set_frame(w, 10, 20, 80, 40, nullptr);
    NEAR(shown(w).w, 80.0, 0.0);
    const DayQtAnim a = linear(1200);
    day_qt_set_frame(w, 10, 20, 240, 40, &a);
    NEAR(shown(w).w, 80.0, 0.0);
    begin(w, "dayFrame");
    at(600);
    NEAR(shown(w).w, 160.0, 4.0);
    at(1300);
    const DayQtDisplayed d = shown(w);
    CHECK(d.frame_ok == 1);
    NEAR(d.x, 10.0, 0.0);
    NEAR(d.y, 20.0, 0.0);
    NEAR(d.w, 240.0, 0.0);
    NEAR(d.h, 40.0, 0.0);
    // A plain set during a run lands at once and the run does not move it again.
    day_qt_set_frame(w, 10, 20, 80, 40, &a);
    begin(w, "dayFrame");
    at(300);
    day_qt_set_frame(w, 0, 0, 50, 50, nullptr);
    stopped(w, "dayFrame");
    NEAR(shown(w).w, 50.0, 0.0);
}

// Rule 8: a stylesheet fill passes through the grays between black and white, and lands on
// white; a fill appearing applies at once.
static void fill_passes_between(QWidget *w) {
    CHECK(shown(w).fill_ok == 0);
    const DayQtAnim a = linear(2500);
    day_qt_widget_set_bg(w, 0, 0, 0, 1, &a); // appears: at once
    DayQtDisplayed d = shown(w);
    CHECK(d.fill_ok == 1);
    NEAR(d.r, 0.0, 0.0);
    day_qt_widget_set_bg(w, 1, 1, 1, 1, &a);
    begin(w, "dayFill");
    at(1250);
    d = shown(w);
    CHECK(d.fill_ok == 1);
    NEAR(d.r, 0.5, 0.03);
    NEAR(d.g, d.r, 1e-9);
    NEAR(d.b, d.r, 1e-9);
    at(2600);
    d = shown(w);
    NEAR(d.r, 1.0, 0.0);
    NEAR(d.a, 1.0, 0.0);
    // Going away applies at once.
    day_qt_widget_set_bg(w, 0, 0, 0, 0, &a);
    CHECK(shown(w).fill_ok == 0);
}

// A painted surface (a border here) keeps everything but its fill through an animated change;
// a scroll area's viewport color passes between too.
static void surface_and_scroll_fills(QWidget *top) {
    auto *card = new QWidget(top);
    card->setGeometry(100, 100, 80, 40);
    card->show();
    double spec[24] = {0};
    spec[3] = 1.0;              // black fill
    spec[4] = 6.0;              // radius
    spec[5] = 15.0;             // all corners
    spec[7] = 1.0, spec[10] = 1.0, spec[11] = 2.0; // red 2 pt border
    day_qt_widget_set_styled_surface(card, spec, nullptr, 0);
    const DayQtAnim a = linear(1000);
    day_qt_widget_set_bg(card, 1, 1, 1, 1, &a);
    begin(card, "dayFill");
    at(500);
    DayQtDisplayed d = shown(card);
    CHECK(d.fill_ok == 1);
    NEAR(d.r, 0.5, 0.03);
    at(1000);
    NEAR(shown(card).r, 1.0, 0.0);
    // The border and the corners are still the surface's: a pixel on the border is red.
    const QImage img = card->grab().toImage();
    const QColor edge = img.pixelColor(int(img.width() / 2), int(img.devicePixelRatio()));
    CHECK(edge.red() > 200 && edge.green() < 80);

    auto *scroll = new QScrollArea(top);
    scroll->setGeometry(200, 100, 80, 40);
    scroll->show();
    day_qt_widget_set_surface(scroll, 0, 0, 0, 1, 0, 0);
    day_qt_widget_set_bg(scroll, 1, 1, 1, 1, &a);
    begin(scroll, "dayFill");
    at(250);
    NEAR(shown(scroll).g, 0.25, 0.03);
    at(1000);
    NEAR(shown(scroll).g, 1.0, 0.0);
}

int main(int argc, char **argv) {
    QApplication app(argc, argv);
    QWidget top;
    top.resize(400, 300);
    auto *w = new QWidget(&top);
    w->setGeometry(10, 20, 80, 40);
    top.show();

    // At rest: the model values, untransformed, no fill.
    DayQtDisplayed d = shown(w);
    NEAR(d.opacity, 1.0, 0.0);
    NEAR(d.tx, 0.0, 0.0);
    CHECK(d.frame_ok == 1 && d.w == 80.0 && d.fill_ok == 0);

    delay_holds_the_start(w);
    repeat_restarts(w);
    autoreverse_plays_back(w);
    forever_until_set(w);
    retarget_from_shown(w);
    spring_samples_overshoot(w);
    delay_then_jump(w);
    frame_passes_between(w);
    auto *box = new QWidget(&top);
    box->setGeometry(0, 100, 80, 40);
    box->show();
    fill_passes_between(box);
    surface_and_scroll_fills(&top);
    // No run is left behind on any channel.
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    CHECK(w->findChildren<QAbstractAnimation *>().isEmpty());
    CHECK(w->graphicsEffect() &&
          w->graphicsEffect()->findChildren<QAbstractAnimation *>().isEmpty());
    CHECK(box->findChildren<QAbstractAnimation *>().isEmpty());
    std::puts("anim: ok");
    return 0;
}
