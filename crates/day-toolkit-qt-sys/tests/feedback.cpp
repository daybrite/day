// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
#include <QApplication>
#include <QLabel>
#include <cassert>

extern "C" {
void day_qt_open_file(const char *) {}
void day_qt_toast(void *, const char *);
}

int main(int argc, char **argv) {
    QApplication app(argc, argv);
    QWidget window;
    window.resize(480, 300);
    window.show();
    const auto current = [&] { return window.findChild<QLabel *>("day.feedback.toast"); };
    day_qt_toast(&window, "First fixture");
    auto *first = current();
    assert(first && first->isVisible());
    day_qt_toast(&window, "Replacement fixture long enough to wrap when the window narrows");
    auto *replacement = current();
    assert(replacement && replacement != first && !first->isVisible());
    assert(replacement->text().startsWith("Replacement fixture"));
    for (const QSize area : {QSize(190, 240), QSize(620, 400)}) {
        window.resize(area);
        QApplication::processEvents();
        assert(replacement->isVisible());
        assert(replacement->width() <= window.width() - 24);
        assert(replacement->x() == (window.width() - replacement->width()) / 2);
        assert(replacement->y() == window.height() - replacement->height() - 16);
    }
}
