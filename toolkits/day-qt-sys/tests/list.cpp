// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//
// The list host's contract with the Rust side (docs/list.md): a count-only model, cells placed
// over rows as viewport children that survive every count change, selection sync without an
// echo, sequential splices that carry the selection along, coalesced row heights that keep the
// top row anchored, and a window report that may arrive while the view is being resized.
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
    return 0;
}
