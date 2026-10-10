// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// Synthetic labels and node IDs are test fixtures.
#include <QApplication>
#include <QKeyEvent>
#include <QTabBar>
#include <QTabWidget>
#include <cassert>
#include <cstdint>
#include <vector>

extern "C" {
void day_qt_open_file(const char *) {}
void *day_qt_tabs_new(uint64_t, void (*)(uint64_t, int));
void day_qt_tabs_add_page(void *, void *, const char *, int);
void day_qt_tabs_set_page_visible(void *, void *, int);
void day_qt_tabs_set_current(void *, int);
}
static std::vector<int> selections;
static void selected(uint64_t node, int index) {
    assert(node == 41);
    selections.push_back(index);
}
int main(int argc, char **argv) {
    QApplication app(argc, argv);
    // Plain tabbed navigation: document mode is never enabled.
    auto *tabs = static_cast<QTabWidget *>(day_qt_tabs_new(41, selected));
    auto *sidebar = new QWidget;
    auto *first = new QWidget;
    auto *second = new QWidget;
    auto *third = new QWidget;
    day_qt_tabs_add_page(tabs, sidebar, "Fixture sidebar", 0);
    day_qt_tabs_set_page_visible(tabs, sidebar, 0);
    day_qt_tabs_add_page(tabs, first, "Fixture first", 1);
    day_qt_tabs_add_page(tabs, second, "Fixture second", 2);
    day_qt_tabs_add_page(tabs, third, "Fixture third", 3);
    day_qt_tabs_set_current(tabs, 1);
    tabs->show();
    QApplication::processEvents();
    assert(selections.empty());
    assert(!tabs->isTabVisible(0));
    assert(tabs->currentWidget() == first);

    // Real keyboard events exercise QTabBar -> QTabWidget -> Day's callback.
    for (int key : {Qt::Key_Right, Qt::Key_Right, Qt::Key_Left, Qt::Key_Left}) {
        QKeyEvent event(QEvent::KeyPress, key, Qt::NoModifier);
        QApplication::sendEvent(tabs->tabBar(), &event);
    }
    assert((selections == std::vector<int>{1, 2, 1, 0}));
    assert(tabs->currentWidget() == first);
    day_qt_tabs_set_current(tabs, 3);
    assert(tabs->currentWidget() == third);
    assert(selections.size() == 4); // programmatic selection never echoes
    tabs->setCurrentIndex(0);
    assert(selections.size() == 4); // parking page never becomes a destination
    while (tabs->count()) tabs->removeTab(tabs->count() - 1);
    assert(selections.size() == 4); // empty (-1) is also ignored
    delete tabs;
}
