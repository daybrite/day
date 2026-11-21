// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//
// The list host's contract with the Rust side (docs/list.md): a count-only model, cells placed
// over rows as viewport children that survive every count change, selection sync without an
// echo, sequential splices that carry the selection along, coalesced row heights that keep the
// top row anchored, and a window report that may arrive while the view is being resized. Then
// the round-2 contract (docs/list.md, docs/scroll.md): splices validated against the rows the
// view holds (a reset that keeps the scroll position otherwise), supplementary rows, the
// viewport report, aligned and animated row scrolls, and the scroll piece's glide, offset
// reports and emulated snap, with the pure math under them.
#include <QApplication>
#include <QCoreApplication>
#include <QEvent>
#include <QListView>
#include <QScrollBar>
#include <QItemSelectionModel>
#include <QAbstractItemModel>
#include <QWidget>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QWheelEvent>
#include <QScrollArea>
#include <QElapsedTimer>
#include <cassert>
#include <cstdint>
#include <vector>
extern "C" {
void day_qt_open_file(const char *) {}
typedef void (*DayListSelectionCb)(uint64_t, const int *, int);
typedef int (*DayListCanMoveCb)(uint64_t, int, int);
typedef void (*DayListMoveCb)(uint64_t, int, int);
typedef void (*DayListWindowCb)(uint64_t);
void *day_qt_list_new(uint64_t, int, int, int, int, int, DayListSelectionCb, void (*)(uint64_t, int),
                      DayListCanMoveCb, DayListMoveCb, DayListWindowCb);
void day_qt_list_set_count(void *, int);
int day_qt_list_splice(void *, int, int, int);
void day_qt_list_set_automatic(void *);
void day_qt_list_set_row_height(void *, int, int);
void day_qt_list_set_estimate(void *, int);
void day_qt_list_clear_heights(void *);
void day_qt_list_place_cell(void *, int, void *);
void day_qt_list_park_cell(void *, void *);
int day_qt_list_placed_count(void *);
void day_qt_list_cell_frame(void *, int, int *, int *, int *, int *);
void day_qt_list_visible_rows(void *, int *, int *);
void day_qt_list_set_selected(void *, const int *, int);
void day_qt_list_scroll_to_row(void *, int);
void day_qt_list_set_headers(void *, const int *, int);
int day_qt_list_header_for(void *, int);
void *day_qt_list_pin_cell(void *);
void day_qt_list_pin_bound(void *, int, int);
int day_qt_splice_fits(int, const int *, int, int);
int day_qt_aligned_offset(int, int, int, int, int, int, int);
int day_qt_snap_target(int, int, int, int, double);
int day_qt_list_splice_all(void *, const int *, int, int);
void day_qt_list_set_supplements(void *, const int *, int);
int day_qt_list_viewport(void *, int *, int *, double *);
void day_qt_list_scroll(void *, int, int, int);
void day_qt_list_on_reader_scroll(void *, void (*)(uint64_t));
void *day_qt_scroll_new(int);
void day_qt_scroll_configure(void *, int, int, double);
void day_qt_scroll_track_offset(void *, uint64_t, void (*)(uint64_t, double, double));
void day_qt_scroll_offset(void *, double *, double *);
void day_qt_scroll_set_content_size(void *, int, int);
void day_qt_scroll_to_rect(void *, int, int, int, int, int);
}

static int selects = 0;
static std::vector<int> lastSelection;
static int windows = 0;
static QListView *resizeInWindow = nullptr;
static void selection(uint64_t node, const int *rows, int n) {
    assert(node == 7);
    ++selects;
    lastSelection.assign(rows, rows + n);
}
static void activate(uint64_t, int) {}
static int canMove(uint64_t, int, int to) { return to; }
static void move(uint64_t, int, int) {}
static void window(uint64_t node) {
    assert(node == 7);
    ++windows;
    // A resize delivered from inside the report, as Qt does when a scroll bar appears: the
    // view must take it without recursing into another report storm.
    if (QListView *v = resizeInWindow) {
        resizeInWindow = nullptr;
        v->resize(v->width() + 10, v->height());
    }
}

static int frameY(void *l, int row) {
    int x = 0, y = 0, w = 0, h = 0;
    day_qt_list_cell_frame(l, row, &x, &y, &w, &h);
    return y;
}
static int frameH(void *l, int row) {
    int x = 0, y = 0, w = 0, h = 0;
    day_qt_list_cell_frame(l, row, &x, &y, &w, &h);
    return h;
}
// Run the event loop for `ms` milliseconds: timers, posted events and animations all advance.
static void spin(int ms) {
    QElapsedTimer t;
    t.start();
    while (t.elapsed() < ms) QCoreApplication::processEvents(QEventLoop::AllEvents, 5);
}
// One wheel notch down (or up) at the middle of `w`, as a mouse wheel sends it.
static void wheel(QWidget *w, int dy) {
    const QPointF at(w->width() / 2.0, w->height() / 2.0);
    QWheelEvent ev(at, w->mapToGlobal(at), QPoint(), QPoint(0, dy), Qt::NoButton, Qt::NoModifier,
                   Qt::NoScrollPhase, false);
    QApplication::sendEvent(w, &ev);
}
static int readers = 0;
static void reader(uint64_t node) {
    assert(node == 7);
    ++readers;
}
// A key press on `w`, as the keyboard sends it.
static void keyPress(QWidget *w, int k) {
    QKeyEvent ev(QEvent::KeyPress, k, Qt::NoModifier);
    QApplication::sendEvent(w, &ev);
}
static std::vector<std::pair<double, double>> offsets;
static void offsetReport(uint64_t node, double x, double y) {
    assert(node == 9);
    offsets.emplace_back(x, y);
}
static std::vector<int> selectedRows(QListView *v) {
    std::vector<int> rows;
    for (const QModelIndex &ix : v->selectionModel()->selectedRows()) rows.push_back(ix.row());
    std::sort(rows.begin(), rows.end());
    return rows;
}

int main(int argc, char **argv) {
    QApplication app(argc, argv);

    // --- a uniform, multi-select, reorderable list --------------------------------------------
    void *l = day_qt_list_new(7, 32, 1, 1, 1, -1, selection, activate, canMove, move, window);
    auto *v = static_cast<QListView *>(l);
    v->resize(300, 200);
    v->show();
    QCoreApplication::processEvents();

    day_qt_list_set_count(l, 1000);
    QCoreApplication::processEvents();
    assert(v->model()->rowCount() == 1000);
    int first = -1, last = -1;
    day_qt_list_visible_rows(l, &first, &last);
    assert(first == 0 && last >= 5 && last <= 7);
    assert(frameY(l, 3) == 96 && frameH(l, 3) == 32);

    // A cell placed over row 3 is a viewport child at the row's frame, and follows a scroll.
    auto *cell = new QWidget();
    day_qt_list_place_cell(l, 3, cell);
    assert(cell->parentWidget() == v->viewport());
    assert(cell->isVisible() && cell->geometry().top() == 96);
    assert(day_qt_list_placed_count(l) == 1);
    const int before = windows;
    v->verticalScrollBar()->setValue(320);
    QCoreApplication::processEvents();
    assert(windows > before);
    assert(cell->geometry().top() == 96 - 320);
    day_qt_list_visible_rows(l, &first, &last);
    assert(first == 10);

    // Shrinking the model under the cell never deletes it; the Rust side parks it.
    day_qt_list_set_count(l, 2);
    QCoreApplication::processEvents();
    assert(v->model()->rowCount() == 2);
    assert(cell->parentWidget() == v->viewport()); // still alive, still ours
    assert(frameH(l, 3) == 0);
    day_qt_list_park_cell(l, cell);
    assert(!cell->isVisible() && day_qt_list_placed_count(l) == 0);
    day_qt_list_set_count(l, 0);
    day_qt_list_set_count(l, 20);
    QCoreApplication::processEvents();
    assert(v->model()->rowCount() == 20);
    day_qt_list_place_cell(l, 25, cell); // out of range: a no-op
    assert(day_qt_list_placed_count(l) == 0);

    // Programmatic selection: no echo; a native change reports the sorted set.
    selects = 0;
    const int two[] = {4, 1};
    day_qt_list_set_selected(l, two, 2);
    assert(selects == 0);
    assert((selectedRows(v) == std::vector<int>{1, 4}));
    v->selectionModel()->select(v->model()->index(6, 0), QItemSelectionModel::ClearAndSelect);
    assert(selects == 1 && (lastSelection == std::vector<int>{6}));
    // Dropping the selected row with the count reports nothing either.
    day_qt_list_set_count(l, 5);
    QCoreApplication::processEvents();
    assert(selects == 1 && selectedRows(v).empty());

    // Sequential splices carry the selection with its row.
    day_qt_list_set_count(l, 10);
    const int five[] = {5};
    day_qt_list_set_selected(l, five, 1);
    assert(day_qt_list_splice(l, 0, 2, 0) == 11); // insert at 2
    assert((selectedRows(v) == std::vector<int>{6}));
    assert(day_qt_list_splice(l, 1, 0, 0) == 10); // remove 0
    assert((selectedRows(v) == std::vector<int>{5}));
    assert(day_qt_list_splice(l, 2, 5, 0) == 10); // move 5 → 0
    assert((selectedRows(v) == std::vector<int>{0}));
    assert(day_qt_list_splice(l, 2, 0, 9) == 10); // move 0 → 9
    assert((selectedRows(v) == std::vector<int>{9}));
    assert(selects == 1);

    // A window report that resizes the view synchronously is survivable.
    resizeInWindow = v;
    v->verticalScrollBar()->setValue(0);
    v->verticalScrollBar()->setValue(16);
    QCoreApplication::processEvents();
    assert(resizeInWindow == nullptr && v->width() == 310);
    delete v;

    // --- a self-sizing list ------------------------------------------------------------------
    void *a = day_qt_list_new(7, 44, 1, 0, 0, 1, selection, activate, canMove, move, window);
    auto *av = static_cast<QListView *>(a);
    day_qt_list_set_automatic(a);
    av->resize(300, 200);
    av->show();
    day_qt_list_set_count(a, 100);
    QCoreApplication::processEvents();
    assert(frameY(a, 1) == 44);
    // Heights land together on the next turn, and the rows below move.
    day_qt_list_set_row_height(a, 0, 80);
    day_qt_list_set_row_height(a, 1, 20);
    assert(frameY(a, 1) == 44); // not yet: coalesced
    QCoreApplication::processEvents();
    assert(frameY(a, 1) == 80 && frameH(a, 1) == 20 && frameY(a, 2) == 100);
    // Rows not measured take the estimate.
    day_qt_list_set_estimate(a, 50);
    QCoreApplication::processEvents();
    assert(frameY(a, 3) == 150 && frameH(a, 3) == 50);
    // A row above the viewport growing keeps the top row where it was.
    av->verticalScrollBar()->setValue(frameY(a, 10) + av->verticalScrollBar()->value());
    QCoreApplication::processEvents();
    assert(av->indexAt(QPoint(1, 1)).row() == 10);
    const int topBefore = frameY(a, 10);
    day_qt_list_set_row_height(a, 2, 200);
    QCoreApplication::processEvents();
    assert(frameH(a, 2) == 200);
    assert(av->indexAt(QPoint(1, 1)).row() == 10 && frameY(a, 10) == topBefore);
    // Forgetting the heights returns every row to the estimate.
    day_qt_list_clear_heights(a);
    QCoreApplication::processEvents();
    assert(frameH(a, 2) == 50 && frameH(a, 0) == 50);
    day_qt_list_scroll_to_row(a, 0);
    QCoreApplication::processEvents();
    assert(av->verticalScrollBar()->value() == 0);
    delete av;

    // --- a sectioned Uniform list (docs/list.md "Sections") ----------------------------------
    // Display rows: headers at 0, 10, 20; items 32 tall; headers as tall as Day lays them.
    void *s = day_qt_list_new(7, 32, 1, 1, 0, 1, selection, activate, canMove, move, window);
    auto *sv = static_cast<QListView *>(s);
    day_qt_list_set_automatic(s); // what the Rust side asks of every sectioned list
    sv->resize(300, 200);
    sv->show();
    day_qt_list_set_count(s, 30);
    const int heads[] = {20, 0, 10, 10, 99}; // any order; duplicates and strays are dropped
    day_qt_list_set_headers(s, heads, 5);
    QCoreApplication::processEvents();
    const auto flags = [&](int row) { return sv->model()->flags(sv->model()->index(row, 0)); };
    assert(!(flags(0) & Qt::ItemIsSelectable) && !(flags(0) & Qt::ItemIsEnabled));
    assert(!(flags(10) & Qt::ItemIsSelectable) && (flags(11) & Qt::ItemIsSelectable));
    assert(day_qt_list_header_for(s, 0) == 0 && day_qt_list_header_for(s, 9) == 0);
    assert(day_qt_list_header_for(s, 10) == 10 && day_qt_list_header_for(s, 29) == 20);
    // A header laid 20 tall sizes alone, and is the guess for the headers not laid yet; the
    // items keep the pitch although the list is Uniform.
    day_qt_list_set_row_height(s, 0, 20);
    QCoreApplication::processEvents();
    assert(frameH(s, 0) == 20 && frameY(s, 1) == 20 && frameH(s, 1) == 32);
    assert(frameY(s, 10) == 20 + 9 * 32 && frameH(s, 10) == 20 && frameY(s, 11) == 328);

    // Selection: a click-range over a header reports only items; clicking a header selects
    // nothing and keeps the selection it had.
    selects = 0;
    sv->selectionModel()->select(QItemSelection(sv->model()->index(8, 0), sv->model()->index(12, 0)),
                                 QItemSelectionModel::ClearAndSelect);
    assert(selects == 1 && (lastSelection == std::vector<int>{8, 9, 11, 12}));
    const int one[] = {11};
    day_qt_list_set_selected(s, one, 1);
    const QPoint onHeader = sv->visualRect(sv->model()->index(10, 0)).center();
    QMouseEvent press(QEvent::MouseButtonPress, onHeader, sv->viewport()->mapToGlobal(onHeader), Qt::LeftButton,
                      Qt::LeftButton, Qt::NoModifier);
    QMouseEvent release(QEvent::MouseButtonRelease, onHeader, sv->viewport()->mapToGlobal(onHeader), Qt::LeftButton,
                        Qt::NoButton, Qt::NoModifier);
    QApplication::sendEvent(sv->viewport(), &press);
    QApplication::sendEvent(sv->viewport(), &release);
    assert((selectedRows(sv) == std::vector<int>{11}));

    // The keyboard walker steps over headers, and Up from a section's first row under the
    // first header stays put.
    const auto key = [&](int k) {
        QKeyEvent ev(QEvent::KeyPress, k, Qt::NoModifier);
        QApplication::sendEvent(sv, &ev);
        return sv->currentIndex().row();
    };
    sv->setFocus();
    sv->selectionModel()->setCurrentIndex(sv->model()->index(9, 0), QItemSelectionModel::ClearAndSelect);
    assert(key(Qt::Key_Down) == 11);
    assert(key(Qt::Key_Up) == 9);
    sv->selectionModel()->setCurrentIndex(sv->model()->index(1, 0), QItemSelectionModel::ClearAndSelect);
    assert(key(Qt::Key_Up) == 1);
    assert(key(Qt::Key_End) == 29);
    assert(key(Qt::Key_Home) == 1);

    // The pinned header: hidden while its own row shows at the top, pinned at y = 0 once the
    // row scrolls under the top, pushed up by the next header, gone when that one arrives.
    auto *pin = static_cast<QWidget *>(day_qt_list_pin_cell(s));
    assert(pin == day_qt_list_pin_cell(s) && pin->parentWidget() == sv->viewport());
    sv->verticalScrollBar()->setValue(0); // Home scrolled row 1 to the top, under the header
    QCoreApplication::processEvents();
    day_qt_list_pin_bound(s, 0, 20);
    assert(!pin->isVisible());
    sv->verticalScrollBar()->setValue(100);
    QCoreApplication::processEvents();
    assert(pin->isVisible() && pin->geometry() == QRect(0, 0, sv->viewport()->width(), 20));
    // A click on it lands on nothing under it.
    const QPoint onPin(40, 10);
    QMouseEvent pinPress(QEvent::MouseButtonPress, onPin, pin->mapToGlobal(onPin), Qt::LeftButton, Qt::LeftButton,
                         Qt::NoModifier);
    QApplication::sendEvent(pin, &pinPress);
    assert((selectedRows(sv) == std::vector<int>{1}));
    // Header 10 sits at 308: 10 px below the top, it pushes the pin 10 px up.
    sv->verticalScrollBar()->setValue(298);
    QCoreApplication::processEvents();
    assert(pin->isVisible() && pin->geometry().top() == -10);
    // Past it, a pin still bound to header 0 is out of sight until it is rebound.
    sv->verticalScrollBar()->setValue(340);
    QCoreApplication::processEvents();
    assert(!pin->isVisible());
    day_qt_list_pin_bound(s, day_qt_list_header_for(s, sv->indexAt(QPoint(1, 1)).row()), 20);
    assert(pin->isVisible() && pin->geometry().top() == 0);
    // Revealing a row the pin would cover scrolls it clear of the pin: row 12 at the top of
    // the viewport sits under header 10's 20 px copy.
    sv->verticalScrollBar()->setValue(frameY(s, 12) + sv->verticalScrollBar()->value() + 5);
    QCoreApplication::processEvents();
    day_qt_list_scroll_to_row(s, 12);
    QCoreApplication::processEvents();
    assert(frameY(s, 12) == 20);
    // A row already clear of it stays where it is.
    day_qt_list_scroll_to_row(s, 13);
    QCoreApplication::processEvents();
    assert(frameY(s, 12) == 20);
    // Headers gone (an unsectioned reload): the pin hides.
    day_qt_list_set_headers(s, nullptr, 0);
    assert(!pin->isVisible() && (flags(10) & Qt::ItemIsSelectable));
    delete sv;

    // --- pure math ----------------------------------------------------------------------------
    // Splice validation: every index against the rows the earlier deltas left, and the count.
    {
        const int ok[] = {0, 3, 0, /* insert at the end of 3 */ 1, 0, 0, /* remove 0 */ 2, 2, 0};
        assert(day_qt_splice_fits(3, ok, 3, 3) == 1);
        assert(day_qt_splice_fits(3, ok, 3, 4) == 0); // the source ends elsewhere
        const int far[] = {0, 5, 0};                  // past the end of 3 rows
        assert(day_qt_splice_fits(3, far, 1, 4) == 0);
        const int gone[] = {1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0}; // a fourth remove of 3 rows
        assert(day_qt_splice_fits(3, gone, 4, -1) == 0 && day_qt_splice_fits(3, gone, 3, 0) == 1);
        const int moveOut[] = {2, 0, 3};
        assert(day_qt_splice_fits(3, moveOut, 1, 3) == 0);
        const int bogus[] = {7, 0, 0};
        assert(day_qt_splice_fits(3, bogus, 1, 3) == 0);
        assert(day_qt_splice_fits(3, nullptr, 0, 3) == 1 && day_qt_splice_fits(3, nullptr, 1, 3) == 0);
        // 200 appends one above the other, the way day-core sends them, fit.
        std::vector<int> append;
        for (int i = 0; i < 200; ++i) append.insert(append.end(), {0, 10, 0});
        assert(day_qt_splice_fits(10, append.data(), 200, 210) == 1);
    }
    // Alignment: a 32 tall row at 3200 in a 160 viewport, range 0..10000.
    {
        enum { Nearest, Start, Center, End };
        assert(day_qt_aligned_offset(3200, 32, 160, 0, 10000, Start, 0) == 3200);
        assert(day_qt_aligned_offset(3200, 32, 160, 0, 10000, End, 0) == 3200 + 32 - 160);
        assert(day_qt_aligned_offset(3200, 32, 160, 0, 10000, Center, 0) == 3200 - 64);
        // Nearest: below → bottom edge; above → top edge; fully visible → stays.
        assert(day_qt_aligned_offset(3200, 32, 160, 0, 10000, Nearest, 0) == 3072);
        assert(day_qt_aligned_offset(3200, 32, 160, 5000, 10000, Nearest, 0) == 3200);
        assert(day_qt_aligned_offset(3200, 32, 160, 3100, 10000, Nearest, 0) == 3100);
        // A row taller than the viewport shows its top.
        assert(day_qt_aligned_offset(3200, 400, 160, 0, 10000, Nearest, 0) == 3200);
        // Clamped: row 0 with End stays at the top; the last row with Start stops at the end.
        assert(day_qt_aligned_offset(0, 32, 160, 0, 10000, End, 0) == 0);
        assert(day_qt_aligned_offset(9990, 32, 160, 0, 9862, Start, 0) == 9862);
        // A pinned header 20 tall: Start lands below it, Nearest counts it as covering.
        assert(day_qt_aligned_offset(3200, 32, 160, 0, 10000, Start, 20) == 3180);
        assert(day_qt_aligned_offset(3200, 32, 160, 3190, 10000, Nearest, 20) == 3180);
        assert(day_qt_aligned_offset(3200, 32, 160, 3150, 10000, Nearest, 20) == 3150);
    }
    // Snap: pages of 100 in a range of 0..450 (the end is a resting point too).
    {
        assert(day_qt_snap_target(140, 100, 450, 1, 100.0) == 100);
        assert(day_qt_snap_target(160, 100, 450, 1, 100.0) == 200);
        // A long fling from page 1 moves one page at most.
        assert(day_qt_snap_target(420, 100, 450, 1, 100.0) == 200);
        assert(day_qt_snap_target(0, 300, 450, 1, 100.0) == 200);
        // The last stretch: between 400 and the end at 450, the end is the nearer stop.
        assert(day_qt_snap_target(430, 400, 450, 1, 100.0) == 450);
        // Intervals of 40, no page limit.
        assert(day_qt_snap_target(61, 0, 450, 2, 40.0) == 80);
        assert(day_qt_snap_target(59, 400, 450, 2, 40.0) == 40);
        assert(day_qt_snap_target(77, 0, 450, 0, 40.0) == 77);
    }

    // --- splices validated against the view's rows ---------------------------------------------
    {
        void *k = day_qt_list_new(7, 32, 1, 1, 0, -1, selection, activate, canMove, move, window);
        auto *kv = static_cast<QListView *>(k);
        kv->resize(240, 160);
        kv->show();
        day_qt_list_set_count(k, 100);
        QCoreApplication::processEvents();
        const int sel[] = {50};
        day_qt_list_set_selected(k, sel, 1);
        selects = 0;
        // Fits: insert at the top, move the last to the top, remove from the middle.
        const int keyed[] = {0, 0, 0, 2, 100, 0, 1, 60, 0};
        assert(day_qt_list_splice_all(k, keyed, 3, 100) == 1);
        assert(kv->model()->rowCount() == 100 && (selectedRows(kv) == std::vector<int>{52}));
        // Does not fit (built for other rows): a reset to the source's count that keeps the
        // scroll position and, by index, the selection, and reports nothing.
        kv->verticalScrollBar()->setValue(1000);
        const int stale[] = {1, 500, 0};
        assert(day_qt_list_splice_all(k, stale, 1, 90) == 0);
        QCoreApplication::processEvents();
        assert(kv->model()->rowCount() == 90);
        assert(kv->verticalScrollBar()->value() == 1000);
        assert((selectedRows(kv) == std::vector<int>{52}));
        const int short_[] = {0, 0, 0};
        assert(day_qt_list_splice_all(k, short_, 1, 95) == 0 && kv->model()->rowCount() == 95);
        assert(selects == 0);

        // The viewport report: 1000..1160 shows rows 31 (from 992) to 36 (to 1184), and the
        // distance to the end is the bar's remaining range.
        int first = -1, last = -1;
        double end = -1;
        assert(day_qt_list_viewport(k, &first, &last, &end) == 1);
        assert(first == 31 && last == 36);
        assert(end == 95 * 32 - 160 - 1000);
        kv->verticalScrollBar()->setValue(kv->verticalScrollBar()->maximum());
        assert(day_qt_list_viewport(k, &first, &last, &end) == 1 && end == 0 && last == 94);

        // Aligned row scrolls, instant: row 50 at the top, row 80 at the bottom, row 10 centered,
        // and Nearest leaving a fully visible row where it is.
        day_qt_list_scroll(k, 50, 1, 0);
        assert(kv->verticalScrollBar()->value() == 50 * 32);
        day_qt_list_viewport(k, &first, &last, &end);
        assert(first == 50);
        day_qt_list_scroll(k, 80, 3, 0);
        assert(kv->verticalScrollBar()->value() == 81 * 32 - 160);
        day_qt_list_viewport(k, &first, &last, &end);
        assert(last == 80);
        day_qt_list_scroll(k, 10, 2, 0);
        assert(kv->verticalScrollBar()->value() == 10 * 32 + 16 - 80);
        day_qt_list_scroll(k, 11, 0, 0);
        assert(kv->verticalScrollBar()->value() == 10 * 32 + 16 - 80);
        day_qt_list_scroll(k, 0, 3, 0);
        assert(kv->verticalScrollBar()->value() == 0);
        // The end of the content.
        day_qt_list_scroll(k, -1, 0, 0);
        assert(kv->verticalScrollBar()->value() == kv->verticalScrollBar()->maximum());
        // Animated: still at the start the same turn, at the row 250 ms later, with frames
        // between; a window report at the end.
        day_qt_list_scroll(k, 20, 1, 0);
        windows = 0;
        day_qt_list_scroll(k, 40, 1, 1);
        assert(kv->verticalScrollBar()->value() == 20 * 32);
        spin(60);
        const int mid = kv->verticalScrollBar()->value();
        assert(mid > 20 * 32 && mid < 40 * 32);
        spin(400);
        assert(kv->verticalScrollBar()->value() == 40 * 32 && windows >= 3);
        // A wheel step stops a glide where it is.
        day_qt_list_scroll(k, 0, 1, 1);
        spin(40);
        wheel(kv->viewport(), -120);
        const int stopped = kv->verticalScrollBar()->value();
        assert(stopped > 0 && stopped < 40 * 32);
        spin(400);
        assert(kv->verticalScrollBar()->value() == stopped);
        // A newer request replaces a running glide.
        day_qt_list_scroll(k, 60, 1, 1);
        spin(40);
        day_qt_list_scroll(k, 30, 1, 0);
        spin(400);
        assert(kv->verticalScrollBar()->value() == 30 * 32);

        // The scrolls the reader starts (docs/list.md "Following the end"): never one the app
        // asked for, animated or not, nor a value the view sets itself.
        day_qt_list_on_reader_scroll(k, reader);
        readers = 0;
        day_qt_list_scroll(k, -1, 0, 1);
        spin(400);
        day_qt_list_scroll(k, 10, 1, 0);
        kv->verticalScrollBar()->setValue(500);
        day_qt_list_set_count(k, 96);
        QCoreApplication::processEvents();
        assert(readers == 0);
        // A wheel step that interrupts a glide is the reader's.
        day_qt_list_scroll(k, -1, 0, 1);
        spin(40);
        wheel(kv->viewport(), 120);
        assert(readers == 1);
        // A wheel step down at the end moves nothing: not a scroll.
        day_qt_list_scroll(k, -1, 0, 0);
        readers = 0;
        wheel(kv->viewport(), -120);
        assert(readers == 0 && kv->verticalScrollBar()->value() == kv->verticalScrollBar()->maximum());
        // The scroll bar: a page click (an action, as the bar's mouse handling triggers it).
        kv->verticalScrollBar()->triggerAction(QAbstractSlider::SliderPageStepSub);
        assert(readers == 1);
        // A key whose cursor move scrolls the list, and one that moves nothing.
        kv->setFocus();
        keyPress(kv, Qt::Key_Home);
        assert(readers == 2 && kv->verticalScrollBar()->value() == 0);
        keyPress(kv, Qt::Key_Home);
        assert(readers == 2);
        keyPress(kv, Qt::Key_End);
        assert(readers == 3 && kv->verticalScrollBar()->value() == kv->verticalScrollBar()->maximum());
        delete kv;
    }

    // --- supplementary rows --------------------------------------------------------------------
    // [top padding 16] [header 40] items 2..21 [footer 30], a Uniform list of 32.
    {
        void *u = day_qt_list_new(7, 32, 1, 1, 1, 1, selection, activate, canMove, move, window);
        auto *uv = static_cast<QListView *>(u);
        uv->resize(240, 160);
        uv->show();
        day_qt_list_set_count(u, 23);
        const int extra[] = {22, 0, 1, 1, 99};
        day_qt_list_set_supplements(u, extra, 5);
        day_qt_list_set_row_height(u, 0, 16);
        day_qt_list_set_row_height(u, 1, 40);
        day_qt_list_set_row_height(u, 22, 30);
        QCoreApplication::processEvents();
        const auto flags = [&](int row) { return uv->model()->flags(uv->model()->index(row, 0)); };
        assert(flags(0) == Qt::NoItemFlags && flags(1) == Qt::NoItemFlags && flags(22) == Qt::NoItemFlags);
        assert(flags(2) & Qt::ItemIsSelectable);
        // Sized like headers although the list is Uniform; the items keep the pitch.
        assert(frameH(u, 0) == 16 && frameY(u, 1) == 16 && frameH(u, 1) == 40);
        assert(frameY(u, 2) == 56 && frameH(u, 2) == 32 && frameH(u, 22) == 30);
        // A click selects nothing and keeps the selection; a range never reports one.
        const int two[] = {2};
        day_qt_list_set_selected(u, two, 1);
        selects = 0;
        const QPoint onHeader = uv->visualRect(uv->model()->index(1, 0)).center();
        QMouseEvent press(QEvent::MouseButtonPress, onHeader, uv->viewport()->mapToGlobal(onHeader),
                          Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
        QMouseEvent release(QEvent::MouseButtonRelease, onHeader, uv->viewport()->mapToGlobal(onHeader),
                            Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
        QApplication::sendEvent(uv->viewport(), &press);
        QApplication::sendEvent(uv->viewport(), &release);
        assert((selectedRows(uv) == std::vector<int>{2}));
        uv->selectionModel()->select(QItemSelection(uv->model()->index(0, 0), uv->model()->index(3, 0)),
                                     QItemSelectionModel::ClearAndSelect);
        assert((lastSelection == std::vector<int>{2, 3}));
        // The keyboard walker never lands on one: Home is the first item, End the last.
        const auto key = [&](int k) {
            QKeyEvent ev(QEvent::KeyPress, k, Qt::NoModifier);
            QApplication::sendEvent(uv, &ev);
            return uv->currentIndex().row();
        };
        uv->setFocus();
        uv->selectionModel()->setCurrentIndex(uv->model()->index(5, 0), QItemSelectionModel::ClearAndSelect);
        assert(key(Qt::Key_Home) == 2);
        assert(key(Qt::Key_Up) == 2);
        assert(key(Qt::Key_End) == 21);
        assert(key(Qt::Key_Down) == 21);
        // The end of the content is past the footer.
        day_qt_list_scroll(u, -1, 0, 0);
        assert(uv->verticalScrollBar()->value() == 16 + 40 + 20 * 32 + 30 - 160);
        int first = -1, last = -1;
        double end = -1;
        day_qt_list_viewport(u, &first, &last, &end);
        assert(last == 22 && end == 0);
        // A scroll queued behind a height relayout of this turn lands on the rows as laid.
        day_qt_list_set_row_height(u, 0, 116); // the top padding grows by 100
        day_qt_list_scroll(u, 10, 1, 0);
        QCoreApplication::processEvents();
        spin(20);
        assert(uv->verticalScrollBar()->value() == 116 + 40 + 8 * 32);
        delete uv;
    }

    // --- the scroll piece: glide, offset reports, emulated snap --------------------------------
    {
        void *sc = day_qt_scroll_new(0);
        auto *sa = static_cast<QScrollArea *>(sc);
        sa->resize(120, 120);
        sa->show();
        day_qt_scroll_set_content_size(sc, 120, 1200);
        day_qt_scroll_track_offset(sc, 9, offsetReport);
        QCoreApplication::processEvents();
        offsets.clear();
        // Instant: one jump, one report (coalesced to the frame).
        day_qt_scroll_to_rect(sc, 0, 1160, 120, 40, 0);
        assert(sa->verticalScrollBar()->value() == 1080);
        spin(50);
        assert(offsets.size() == 1 && offsets.back().second == 1080);
        double x = -1, y = -1;
        day_qt_scroll_offset(sc, &x, &y);
        assert(x == 0 && y == 1080);
        // Animated: frames in between, at most one report per 16 ms, the last at rest on the
        // value the jump sets.
        offsets.clear();
        QElapsedTimer t;
        t.start();
        day_qt_scroll_to_rect(sc, 0, 0, 120, 40, 1);
        assert(sa->verticalScrollBar()->value() == 1080);
        spin(450);
        assert(sa->verticalScrollBar()->value() == 0);
        assert(offsets.size() >= 3 && offsets.back().second == 0);
        assert(static_cast<qint64>(offsets.size()) <= t.elapsed() / 16 + 2);
        for (size_t i = 1; i < offsets.size(); ++i) assert(offsets[i].second < offsets[i - 1].second);
        // A wheel step stops the glide; a newer scroll wins over a running one.
        day_qt_scroll_to_rect(sc, 0, 1160, 120, 40, 1);
        spin(40);
        wheel(sa->viewport(), 120);
        const int held = sa->verticalScrollBar()->value();
        assert(held > 0 && held < 1080);
        spin(400);
        assert(sa->verticalScrollBar()->value() == held);
        day_qt_scroll_to_rect(sc, 0, 1160, 120, 40, 1);
        spin(40);
        day_qt_scroll_to_rect(sc, 0, 400, 120, 40, 1);
        spin(450);
        // From wherever the first glide was, the least scroll that shows 400..440.
        assert(sa->verticalScrollBar()->value() == 400);
        delete sa;

        // Hidden indicators: no bars, wheel scrolling still works. Snap every 100 points.
        void *sn = day_qt_scroll_new(0);
        auto *ss = static_cast<QScrollArea *>(sn);
        day_qt_scroll_configure(sn, 1, 2, 100.0);
        ss->resize(120, 120);
        ss->show();
        day_qt_scroll_set_content_size(sn, 120, 1200);
        QCoreApplication::processEvents();
        assert(ss->verticalScrollBarPolicy() == Qt::ScrollBarAlwaysOff);
        assert(ss->horizontalScrollBarPolicy() == Qt::ScrollBarAlwaysOff);
        wheel(ss->viewport(), -120);
        const int stepped = ss->verticalScrollBar()->value();
        assert(stepped > 0);
        spin(100);
        assert(ss->verticalScrollBar()->value() == stepped); // still within the gesture
        spin(500);
        const int rest = ss->verticalScrollBar()->value();
        assert(rest % 100 == 0 && std::abs(rest - stepped) <= 50);
        // A programmatic scroll does not snap.
        day_qt_scroll_to_rect(sn, 0, 333, 120, 1, 0);
        spin(400);
        assert(ss->verticalScrollBar()->value() == 214);
        delete ss;
    }
    return 0;
}
