// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//
// The shim's pointer recognizers and routing under real Qt input (docs/gestures.md): QTest
// drives the window the way the platform does (through QWindowSystemInterface, so Qt's own
// double-click detection, mouse grab and touch delivery run), and the callbacks must report what
// the contract says. Press, long press, double tap against a held single tap, the contact list
// with a finger forwarded as the mouse, the input kind, hit slop, pass-through, the drag axis,
// the keyboard focus indicator, subtree enablement and the drop target's leave. Synthetic node
// IDs are test fixtures.
#include <QApplication>
#include <QDragEnterEvent>
#include <QDragLeaveEvent>
#include <QDropEvent>
#include <QLineEdit>
#include <QMimeData>
#include <QPointingDevice>
#include <QPushButton>
#include <QStyleHints>
#include <QTest>
#include <QWidget>
#include <cassert>
#include <cstdint>
#include <cstdio>
#include <vector>

extern "C" {
void day_qt_open_file(const char *) {}
typedef void (*DayGestureCb)(uint64_t, int, double, double, double, double);
void day_qt_enable_gesture(void *, uint64_t, int, DayGestureCb);
void day_qt_set_touches_cb(void (*)(uint64_t, const double *, int));
void day_qt_install_input_router(void (*)(int));
void day_qt_set_passive(void *);
void day_qt_set_hit_slop(void *, double, double, double, double);
void day_qt_set_drag_axis(void *, int);
void day_qt_subtree_enabled(void *, int);
void day_qt_set_focusable(void *);
void day_qt_enable_keys(void *, uint64_t, int (*)(uint64_t, int, int, int));
void day_qt_drop_target(void *, bool (*)(void *, double, double, const char *, bool),
                        bool (*)(void *, double, double, const unsigned char *, size_t, bool),
                        void (*)(void *));
}

struct Gesture {
    uint64_t node;
    int phase;
    double x, y;
};
static std::vector<Gesture> gestures;
static void on_gesture(uint64_t node, int phase, double x, double y, double, double) {
    gestures.push_back({node, phase, x, y});
}
static std::vector<int> phases(uint64_t node) {
    std::vector<int> out;
    for (const Gesture &g : gestures)
        if (g.node == node) out.push_back(g.phase);
    return out;
}

struct Contact {
    int id, phase, kind;
    double x, y, pressure;
};
static std::vector<std::vector<Contact>> touch_lists;
static void on_touches(uint64_t, const double *t, int n) {
    std::vector<Contact> list;
    for (int i = 0; i < n; ++i)
        list.push_back({int(t[i * 6]), int(t[i * 6 + 1]), int(t[i * 6 + 5]), t[i * 6 + 2],
                        t[i * 6 + 3], t[i * 6 + 4]});
    touch_lists.push_back(list);
}

static std::vector<int> kinds;
static void on_kind(int kind) { kinds.push_back(kind); }

static int short_key(uint64_t, int, int, int) { return 0; }

static int leaves = 0, accepts = 0, receives = 0;
static bool accepting = true;
static bool drop_accept(void *, double, double, const char *, bool) {
    ++accepts;
    return accepting;
}
static bool drop_receive(void *, double, double, const unsigned char *, size_t, bool) {
    ++receives;
    return true;
}
static void drop_leave(void *) { ++leaves; }

#define CHECK(cond)                                                                              \
    do {                                                                                         \
        if (!(cond)) {                                                                           \
            std::fprintf(stderr, "%s:%d: CHECK failed: %s\n", __FILE__, __LINE__, #cond);        \
            std::abort();                                                                        \
        }                                                                                        \
    } while (0)

int main(int argc, char **argv) {
    QApplication app(argc, argv);
    day_qt_set_touches_cb(on_touches);
    day_qt_install_input_router(on_kind);
    const int dbl = QApplication::doubleClickInterval();
    const int hold = QGuiApplication::styleHints()->mousePressAndHoldInterval();
    const int dragDistance = QGuiApplication::styleHints()->startDragDistance();

    QWidget top;
    top.resize(400, 400);
    day_qt_set_passive(&top);
    QWidget pad(&top);
    pad.setGeometry(20, 20, 120, 80);
    day_qt_set_passive(&pad);
    top.show();
    {
        const bool exposed = QTest::qWaitForWindowExposed(&top);
        CHECK(exposed);
        top.activateWindow();
        const bool active = QTest::qWaitForWindowActive(&top);
        CHECK(active);
    }
    QWindow *win = top.windowHandle();
    CHECK(win);
    auto at = [](int x, int y) { return QPoint(20 + x, 20 + y); }; // pad → window

    // Press: true at the down itself, false once when the pointer leaves the hit area, no
    // second true on re-entry, nothing more at the release; a secondary button never presses.
    day_qt_enable_gesture(&pad, 1, 6, on_gesture);
    QTest::mousePress(win, Qt::LeftButton, {}, at(10, 10));
    CHECK((phases(1) == std::vector<int>{18}));
    QTest::mouseMove(win, at(60, 40));
    CHECK((phases(1) == std::vector<int>{18}));
    QTest::mouseMove(win, at(200, 40));
    CHECK((phases(1) == std::vector<int>{18, 19}));
    QTest::mouseMove(win, at(60, 40));
    QTest::mouseRelease(win, Qt::LeftButton, {}, at(60, 40));
    CHECK((phases(1) == std::vector<int>{18, 19}));
    gestures.clear();
    QTest::mouseClick(win, Qt::RightButton, {}, at(10, 10));
    CHECK(phases(1).empty());
    QTest::mouseClick(win, Qt::LeftButton, {}, at(10, 10));
    CHECK((phases(1) == std::vector<int>{18, 19}));
    gestures.clear();
    // Disabled while pressed: day-core ends that press itself, so the view reports no second
    // `false` at the release.
    QTest::mousePress(win, Qt::LeftButton, {}, at(10, 10));
    day_qt_subtree_enabled(&pad, 0);
    QTest::mouseRelease(win, Qt::LeftButton, {}, at(10, 10));
    day_qt_subtree_enabled(&pad, 1);
    CHECK((phases(1) == std::vector<int>{18}));
    gestures.clear();

    // Tap and double tap on one view: a single tap waits out Qt's double-click interval, a
    // double click is a double tap and no tap, and a triple click is a double tap, then a tap.
    QWidget taps(&top);
    taps.setGeometry(200, 20, 120, 80);
    day_qt_set_passive(&taps);
    taps.show();
    day_qt_enable_gesture(&taps, 2, 8, on_gesture);
    day_qt_enable_gesture(&taps, 2, 0, on_gesture);
    const QPoint tp(240, 50);
    QTest::mouseClick(win, Qt::LeftButton, {}, tp);
    CHECK(phases(2).empty());
    QTest::qWait(dbl + 150);
    CHECK((phases(2) == std::vector<int>{0}));
    gestures.clear();
    QTest::mouseDClick(win, Qt::LeftButton, {}, tp);
    CHECK((phases(2) == std::vector<int>{17}));
    QTest::qWait(dbl + 150);
    CHECK((phases(2) == std::vector<int>{17}));
    gestures.clear();
    // A tap alone is immediate, every click of a double click included.
    QWidget quick(&top);
    quick.setGeometry(20, 300, 60, 40);
    quick.show();
    day_qt_enable_gesture(&quick, 3, 0, on_gesture);
    QTest::mouseDClick(win, Qt::LeftButton, {}, QPoint(40, 320));
    CHECK((phases(3) == std::vector<int>{0, 0}));
    gestures.clear();

    // Long press: once, at the press point, after Qt's press-and-hold interval; the release is
    // no tap; moving past the drag distance first cancels it.
    QWidget held(&top);
    held.setGeometry(200, 200, 120, 80);
    held.show();
    day_qt_enable_gesture(&held, 4, 0, on_gesture);
    day_qt_enable_gesture(&held, 4, 7, on_gesture);
    QTest::mousePress(win, Qt::LeftButton, {}, QPoint(230, 230));
    QTest::qWait(hold + 200);
    CHECK((phases(4) == std::vector<int>{16}));
    CHECK(gestures.back().x == 30 && gestures.back().y == 30);
    QTest::mouseRelease(win, Qt::LeftButton, {}, QPoint(230, 230));
    CHECK((phases(4) == std::vector<int>{16}));
    gestures.clear();
    QTest::mousePress(win, Qt::LeftButton, {}, QPoint(230, 230));
    QTest::mouseMove(win, QPoint(230 + dragDistance + 5, 230));
    QTest::qWait(hold + 200);
    QTest::mouseRelease(win, Qt::LeftButton, {}, QPoint(230 + dragDistance + 5, 230));
    CHECK(phases(4).empty());
    QTest::mouseClick(win, Qt::LeftButton, {}, QPoint(230, 230));
    CHECK((phases(4) == std::vector<int>{0}));
    gestures.clear();

    // Touches: every contact on the view with its own id, the ended ones once with their final
    // phase; the first finger also reaches the view's tap and press as the mouse would.
    day_qt_enable_gesture(&pad, 1, 9, on_gesture);
    day_qt_enable_gesture(&pad, 1, 0, on_gesture);
    QPointingDevice *screen = QTest::createTouchDevice();
    kinds.clear();
    QTest::touchEvent(&top, screen).press(0, at(10, 10));
    CHECK((kinds == std::vector<int>{0}));
    QTest::touchEvent(&top, screen).stationary(0).press(1, at(50, 30));
    QTest::touchEvent(&top, screen).release(0, at(10, 10)).stationary(1);
    QTest::touchEvent(&top, screen).release(1, at(50, 30));
    CHECK(touch_lists.size() == 4);
    CHECK(touch_lists[0].size() == 1 && touch_lists[0][0].phase == 0 && touch_lists[0][0].kind == 0);
    CHECK(touch_lists[1].size() == 2 && touch_lists[1][1].id == 1 && touch_lists[1][1].phase == 0);
    CHECK(touch_lists[1][1].x == 50 && touch_lists[1][1].y == 30);
    CHECK(touch_lists[2].size() == 2 && touch_lists[2][0].phase == 3 && touch_lists[2][1].phase == 2);
    CHECK(touch_lists[3].size() == 1 && touch_lists[3][0].id == 1 && touch_lists[3][0].phase == 3);
    // The first finger was the mouse: pressed, released, a tap where it lifted.
    CHECK((phases(1) == std::vector<int>{18, 19, 0}));
    touch_lists.clear();
    gestures.clear();
    // The mouse is one contact, id 0, kind mouse; and the input kind follows it.
    QTest::mouseClick(win, Qt::LeftButton, {}, at(5, 5));
    CHECK((kinds == std::vector<int>{0, 1}));
    CHECK(touch_lists.size() == 2 && touch_lists[0][0].id == 0 && touch_lists[0][0].kind == 1);
    CHECK(touch_lists[0][0].phase == 0 && touch_lists[1][0].phase == 3);
    touch_lists.clear();
    gestures.clear();

    // Drag axis: a horizontal drag hears nothing of a vertical movement, and all of a
    // horizontal one from its press point.
    QWidget swipe(&top);
    swipe.setGeometry(20, 140, 160, 120);
    day_qt_set_passive(&swipe);
    swipe.show();
    day_qt_enable_gesture(&swipe, 5, 1, on_gesture);
    day_qt_set_drag_axis(&swipe, 1);
    QTest::mousePress(win, Qt::LeftButton, {}, QPoint(40, 160));
    QTest::mouseMove(win, QPoint(42, 160 + dragDistance + 10));
    QTest::mouseMove(win, QPoint(80, 200));
    QTest::mouseRelease(win, Qt::LeftButton, {}, QPoint(80, 200));
    CHECK(phases(5).empty());
    QTest::mousePress(win, Qt::LeftButton, {}, QPoint(40, 160));
    QTest::mouseMove(win, QPoint(40 + dragDistance + 10, 162));
    QTest::mouseRelease(win, Qt::LeftButton, {}, QPoint(40 + dragDistance + 10, 162));
    CHECK((phases(5) == std::vector<int>{1, 2, 3}));
    CHECK(gestures.front().x == 20 && gestures.front().y == 20);
    gestures.clear();

    // Hit slop: a 12-point dot with 30 points of slop takes a press 20 points outside it, at a
    // point in its own coordinates; 40 points outside it does not. A view in front of the
    // slop area keeps its own presses.
    QWidget slopHost(&top);
    slopHost.setGeometry(150, 280, 240, 110);
    day_qt_set_passive(&slopHost);
    slopHost.show();
    QWidget dot(&slopHost);
    dot.setGeometry(100, 50, 12, 12);
    day_qt_set_passive(&dot);
    dot.show();
    day_qt_enable_gesture(&dot, 6, 0, on_gesture);
    day_qt_set_hit_slop(&dot, 30, 30, 30, 30);
    const QPoint dotAt(250, 330); // the dot's origin in the window
    QTest::mouseClick(win, Qt::LeftButton, {}, dotAt + QPoint(12 + 20, 6));
    CHECK((phases(6) == std::vector<int>{0}));
    CHECK(gestures.back().x == 32 && gestures.back().y == 6);
    gestures.clear();
    QTest::mouseClick(win, Qt::LeftButton, {}, dotAt + QPoint(12 + 40, 6));
    CHECK(phases(6).empty());
    QPushButton front("Front", &slopHost);
    front.setGeometry(60, 40, 30, 30); // over the dot's slop on its leading side, in front
    front.show();
    front.raise();
    int frontClicks = 0;
    QObject::connect(&front, &QPushButton::clicked, [&] { ++frontClicks; });
    QTest::mouseClick(win, Qt::LeftButton, {}, QPoint(150 + 75, 280 + 55));
    CHECK(frontClicks == 1 && phases(6).empty());
    // Zero insets restore the bounds.
    day_qt_set_hit_slop(&dot, 0, 0, 0, 0);
    QTest::mouseClick(win, Qt::LeftButton, {}, dotAt + QPoint(12 + 20, 6));
    CHECK(phases(6).empty());
    gestures.clear();

    // Pass-through: a passive view over a button lets the press reach the button; once it
    // takes a tap it takes the press itself.
    QWidget stack(&top);
    stack.setGeometry(150, 150, 100, 100);
    day_qt_set_passive(&stack);
    stack.show();
    stack.raise();
    QPushButton under("Under", &stack);
    under.setGeometry(0, 0, 100, 100);
    int underClicks = 0;
    QObject::connect(&under, &QPushButton::clicked, [&] { ++underClicks; });
    QWidget cover(&stack);
    cover.setGeometry(0, 0, 100, 100);
    day_qt_set_passive(&cover);
    under.show();
    cover.show();
    cover.raise();
    QTest::mouseClick(win, Qt::LeftButton, {}, QPoint(200, 200));
    CHECK(underClicks == 1);
    day_qt_enable_gesture(&cover, 7, 0, on_gesture);
    QTest::mouseClick(win, Qt::LeftButton, {}, QPoint(200, 200));
    CHECK(underClicks == 1 && (phases(7) == std::vector<int>{0}));
    gestures.clear();
    stack.hide();

    // Focus indicator: shown for a focus the keyboard brought, not for one a click did.
    QWidget focusable(&top);
    focusable.setGeometry(330, 140, 60, 60);
    focusable.show();
    day_qt_enable_keys(&focusable, 8, short_key);
    day_qt_set_focusable(&focusable);
    const auto widgetsIn = [&] { return top.findChildren<QWidget *>().size(); };
    const auto before = widgetsIn();
    focusable.setFocus(Qt::MouseFocusReason);
    QApplication::processEvents();
    CHECK(focusable.hasFocus() && widgetsIn() == before);
    focusable.clearFocus();
    focusable.setFocus(Qt::TabFocusReason);
    QApplication::processEvents();
    CHECK(focusable.hasFocus() && widgetsIn() == before + 1);
    focusable.clearFocus();
    QTest::qWait(10); // the indicator goes with deleteLater
    CHECK(widgetsIn() == before);

    // Subtree enablement: counted, focus inside resigns, a control disabled on its own stays so.
    QWidget group(&top);
    group.setGeometry(330, 210, 60, 60);
    QLineEdit field(&group);
    QPushButton own(&group);
    own.setEnabled(false);
    group.show();
    field.setFocus(Qt::OtherFocusReason);
    QApplication::processEvents();
    CHECK(field.hasFocus());
    day_qt_subtree_enabled(&group, 0);
    day_qt_subtree_enabled(&group, 0);
    CHECK(!field.isEnabled() && !field.hasFocus());
    day_qt_subtree_enabled(&group, 1);
    CHECK(!field.isEnabled());
    day_qt_subtree_enabled(&group, 1);
    CHECK(field.isEnabled() && !own.isEnabled());
    day_qt_subtree_enabled(&group, 1); // an unmatched enable changes nothing
    CHECK(field.isEnabled());

    // Drop target: a drag that leaves, or drops where the view refuses it, ends the hover.
    QWidget zone(&top);
    zone.setGeometry(20, 270, 100, 20);
    zone.show();
    day_qt_drop_target(&zone, drop_accept, drop_receive, drop_leave);
    QMimeData mime;
    mime.setText("hello");
    // Qt 6.12 moved the drag events to QPointF positions.
#if QT_VERSION >= QT_VERSION_CHECK(6, 12, 0)
    using DragPoint = QPointF;
#else
    using DragPoint = QPoint;
#endif
    QDragEnterEvent enter(DragPoint(5, 5), Qt::CopyAction, &mime, Qt::LeftButton, {});
    QApplication::sendEvent(&zone, &enter);
    QDragMoveEvent moved(DragPoint(6, 5), Qt::CopyAction, &mime, Qt::LeftButton, {});
    QApplication::sendEvent(&zone, &moved);
    CHECK(accepts == 2 && leaves == 0);
    QDragLeaveEvent left;
    QApplication::sendEvent(&zone, &left);
    CHECK(leaves == 1);
    // A drop always follows an entry: Qt delivers it to the view the drag entered last.
    QDragEnterEvent again(DragPoint(5, 5), Qt::CopyAction, &mime, Qt::LeftButton, {});
    QApplication::sendEvent(&zone, &again);
    accepting = false;
    QDropEvent refused(QPointF(5, 5), Qt::CopyAction, &mime, Qt::LeftButton, {});
    QApplication::sendEvent(&zone, &refused);
    CHECK(accepts == 4 && leaves == 2 && receives == 0);
    accepting = true;
    QDragEnterEvent last(DragPoint(5, 5), Qt::CopyAction, &mime, Qt::LeftButton, {});
    QApplication::sendEvent(&zone, &last);
    QDropEvent dropped(QPointF(5, 5), Qt::CopyAction, &mime, Qt::LeftButton, {});
    QApplication::sendEvent(&zone, &dropped);
    CHECK(leaves == 2 && receives == 1);

    std::puts("input: ok");
    return 0;
}
