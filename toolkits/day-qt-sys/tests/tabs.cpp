// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// Synthetic labels and node IDs are test fixtures.
#include <QApplication>
#include <QKeyEvent>
#include <QTabBar>
#include <QTabWidget>
#include <QToolButton>
#include <cassert>
#include <cstdint>
#include <vector>

extern "C" {
void day_qt_open_file(const char *) {}
void *day_qt_tabs_new(uint64_t, void (*)(uint64_t, int));
void day_qt_tabs_add_page(void *, void *, const char *, int);
void day_qt_tabs_park(void *, void *);
void day_qt_tabs_set_current(void *, int);
void day_qt_tabs_documents(void *, uint64_t, const char *, const char *, int, void (*)(uint64_t, int, int, int));
void *day_qt_enclosing_tabs(void *);
}
static std::vector<int> selections;
static void selected(uint64_t node, int index) {
    assert(node == 41);
    selections.push_back(index);
}
struct DocumentEvent {
    int action, index, to;
};
static std::vector<DocumentEvent> documents;
static void document(uint64_t node, int action, int index, int to) {
    assert(node == 41);
    documents.push_back({action, index, to});
}
int main(int argc, char **argv) {
    QApplication app(argc, argv);
    auto *tabs = static_cast<QTabWidget *>(day_qt_tabs_new(41, selected));
    // Day's nav host keeps its menu as child 0, inside the sidebar page. The page is parked
    // under the widget: on the menu's parent chain, never a tab, never on screen.
    auto *sidebar = new QWidget;
    auto *menu = new QWidget(sidebar);
    day_qt_tabs_park(tabs, sidebar);
    auto *first = new QWidget;
    auto *second = new QWidget;
    auto *third = new QWidget;
    day_qt_tabs_add_page(tabs, first, "Fixture first", 0);
    day_qt_tabs_add_page(tabs, second, "Fixture second", 1);
    day_qt_tabs_add_page(tabs, third, "Fixture third", 2);
    day_qt_tabs_set_current(tabs, 0);
    tabs->show();
    QApplication::processEvents();
    assert(selections.empty());
    assert(tabs->count() == 3);
    assert(tabs->indexOf(sidebar) == -1);
    assert(day_qt_enclosing_tabs(menu) == tabs);
    assert(tabs->currentWidget() == first);
    sidebar->show(); // whatever is later set on the page, the parked holder keeps it hidden
    assert(!sidebar->isVisible());

    // Real keyboard events exercise QTabBar -> QTabWidget -> Day's callback, with the tab
    // index reported as the destination index unchanged.
    for (int key : {Qt::Key_Right, Qt::Key_Right, Qt::Key_Left, Qt::Key_Left}) {
        QKeyEvent event(QEvent::KeyPress, key, Qt::NoModifier);
        QApplication::sendEvent(tabs->tabBar(), &event);
    }
    assert((selections == std::vector<int>{1, 2, 1, 0}));
    assert(tabs->currentWidget() == first);
    day_qt_tabs_set_current(tabs, 2);
    assert(tabs->currentWidget() == third);
    assert(selections.size() == 4); // programmatic selection never echoes

    // Document mode: a close request and a drag to any slot, the first included, reach Day
    // with the tab's own index; nothing is reserved at index 0.
    day_qt_tabs_documents(tabs, 41, "Fixture new", "Fixture close", 1, document);
    assert(tabs->cornerWidget() != nullptr);
    emit tabs->tabCloseRequested(0);
    tabs->tabBar()->moveTab(2, 0);
    assert(documents.size() == 2);
    assert(documents[0].action == 1 && documents[0].index == 0);
    assert(documents[1].action == 2 && documents[1].index == 2 && documents[1].to == 0);
    assert(tabs->widget(0) == third);
    static_cast<QToolButton *>(tabs->cornerWidget())->click();
    assert(documents.size() == 3 && documents[2].action == 0);

    // The current tab followed the move; whatever that reported, emptying the widget (-1)
    // reports nothing more.
    const size_t reported = selections.size();
    while (tabs->count()) tabs->removeTab(tabs->count() - 1);
    assert(selections.size() == reported);
    delete tabs;
}
