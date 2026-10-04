// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
#include <QApplication>
#include <QAction>
#include <QCoreApplication>
#include <QEvent>
#include <QMenu>
#include <QKeySequence>
#include <QWidget>
#include <cassert>

extern "C" {
// Rust owns file-open dispatch in the app; this isolated native fixture has no files.
void day_qt_open_file(const char *) {}
void *day_qt_window_new(const char *, int, int);
void *day_qt_window_new2(const char *, int, int, unsigned long long, int);
void day_qt_window_destroy(void *);
void *day_qt_menu_new();
void day_qt_menu_add_role(void *, const char *, int, const char *);
}

int main(int argc, char **argv) {
    QApplication app(argc, argv);
    auto *primary = static_cast<QWidget *>(day_qt_window_new("Fixture", 400, 300));
    assert(primary->actions().size() == 1);
    auto *quit = primary->actions().front();
    // The offscreen platform may not define StandardKey::Quit; seed a portable fixture key.
    quit->setShortcut(QKeySequence("Ctrl+Q"));
    assert(!quit->shortcuts().isEmpty());
    auto *secondary = static_cast<QWidget *>(day_qt_window_new2("Fixture", 200, 100, 1, 0));
    assert(secondary->actions().isEmpty()); // no conflicting application Quit shortcut
    day_qt_window_destroy(secondary);
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    auto *menu = static_cast<QMenu *>(day_qt_menu_new());
    day_qt_menu_add_role(menu, "Quit fixture", 8, "Ctrl+Q");
    assert(quit->shortcuts().isEmpty()); // the primary action is still targeted
    delete menu;
    day_qt_window_destroy(primary);
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    menu = static_cast<QMenu *>(day_qt_menu_new());
    day_qt_menu_add_role(menu, "Quit fixture", 8, "Ctrl+Q"); // disposed primary is safe too
    delete menu;
}
