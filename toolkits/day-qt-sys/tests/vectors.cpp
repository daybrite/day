// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#include <QApplication>
#include <QPushButton>
#include <cassert>
#include <cstdint>
#include <iostream>

extern "C" {
void day_qt_open_file(const char *) {}
void day_qt_button_set_content(void *, const char *, const char *, int, int);
void day_qt_button_set_style(void *, int, unsigned, unsigned);
}

int main(int argc, char **argv) {
    QApplication app(argc, argv);
    assert(argc == 2);
    QPushButton button;
    day_qt_button_set_content(&button, "Fixture", argv[1], -1, 1);
    assert(!button.icon().isNull());
    assert(button.text().isEmpty());
    // Set foreground after installing the icon: the retained engine reads the live button.
    day_qt_button_set_style(&button, 3, 0xff000000, 0xffff0000);
    for (qreal scale : {1.0, 1.5, 2.0, 3.0}) {
        const QPixmap pixmap = button.icon().pixmap(QSize(1024, 128), scale);
        assert(pixmap.size() == QSize(qRound(1024 * scale), qRound(128 * scale)));
        assert(pixmap.devicePixelRatio() == scale);
        const QImage image = pixmap.toImage();
        const QColor line = image.pixelColor(qRound(131 * scale), qRound(64 * scale));
        const QColor gap = image.pixelColor(qRound(139 * scale), qRound(64 * scale));
        assert(line == QColor(Qt::red));
        assert(gap.alpha() == 0);
        std::cout << "DPR " << scale << ": thin stroke and transparent gap preserved\n";
    }
    day_qt_button_set_style(&button, 3, 0xff000000, 0xff0000ff);
    assert(button.icon().pixmap(QSize(1024, 128)).toImage().pixelColor(131, 64) == QColor(Qt::blue));

    // A symbol is a theme/fallback pair, not a filename, even though it ends in .svg.
    // Force the portable fallback instead of depending on an installed icon theme.
    const QByteArray symbol = QByteArray("day-test-missing-theme-icon|") + argv[1];
    day_qt_button_set_content(&button, "Symbol fixture", symbol.constData(), -1, 1);
    assert(!button.icon().isNull());
    assert(button.text().isEmpty());
}
