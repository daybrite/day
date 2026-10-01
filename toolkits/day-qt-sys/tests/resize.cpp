// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
#include "../src/shim-resize.h"
#include <QApplication>
#include <QResizeEvent>
#include <cassert>

static int calls = 0;
static QWidget *reported = nullptr;
static void report(void *host) {
    ++calls;
    reported = static_cast<QWidget *>(host);
}
static void resize(QWidget &pane) {
    QResizeEvent event(QSize(400, 300), QSize(200, 100));
    QApplication::sendEvent(&pane, &event);
}
static void install(QWidget &pane, QWidget &host, void (*cb)(void *) = report) {
    auto *filter = new DayPaneResizeFilter(&host, cb);
    filter->setParent(&pane);
    pane.installEventFilter(filter);
}
int main(int argc, char **argv) {
    QApplication app(argc, argv);
    QWidget host, pane;
    install(pane, host);
    resize(pane);
    resize(pane);
    assert(calls == 0); // no event pump/reparenting inside native resize/paint
    QApplication::processEvents();
    assert(calls == 1 && reported == &host); // coalesced, latest geometry
    resize(pane);
    QApplication::processEvents();
    assert(calls == 2); // delivery re-arms the filter

    auto *removed = new QWidget;
    install(*removed, host);
    resize(*removed);
    delete removed;
    QApplication::processEvents();
    assert(calls == 2); // disposal cancels a pending notification

    QWidget orphan;
    auto *gone = new QWidget;
    install(orphan, *gone);
    resize(orphan);
    delete gone;
    QApplication::processEvents();
    assert(calls == 2); // host lifetime can differ from the filter's

    auto *selfRemoving = new QWidget;
    install(*selfRemoving, *selfRemoving, [](void *p) {
        ++calls;
        delete static_cast<QWidget *>(p);
    });
    resize(*selfRemoving);
    QApplication::processEvents();
    assert(calls == 3); // a Day callback may dispose its own widget/filter
}
