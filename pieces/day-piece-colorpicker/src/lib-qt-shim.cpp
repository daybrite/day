// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-piece-colorpicker's OWN Qt shim (the day-piece-datetime recipe): a swatch button that opens
// `QColorDialog`, behind a flat C ABI. Qt has no color-well widget, so the swatch is a
// `QPushButton` painted with the current color; the dialog behind it is the real Qt chooser
// (basic + custom palettes, an HSV picker, the screen eyedropper, and an alpha channel on
// request).
//
// Components cross as doubles rather than as a packed 32-bit ARGB: `QColor` is float-precision
// internally (`getRgbF`) and Day's `Color` is f64, so quantizing to 8 bits in the middle would
// lose precision that both ends have.
//
// The swatch is painted through a stylesheet rather than a `QPalette` role, because a themed
// QPushButton draws its own background over the palette on most styles — the stylesheet is what
// actually shows through. The text color flips with the fill's luminance so the hex label stays
// readable on both a near-black and a near-white pick.

#include <QColor>
#include <QColorDialog>
#include <QPushButton>
#include <QString>
#include <QVBoxLayout>
#include <QWidget>

#include <cstdint>

namespace {

class DayColorWidget : public QWidget {
public:
    QPushButton *button = nullptr;
    QColor color;
    bool alpha = false;
    QString title;

    void apply(const QColor &c) {
        color = c;
        if (!button)
            return;
        // Relative luminance (Rec. 709) decides the label color, the same rule Day's own
        // `on_tint` uses for a tinted control.
        const double lum = 0.2126 * c.redF() + 0.7152 * c.greenF() + 0.0722 * c.blueF();
        const QString fg = lum > 0.55 ? QStringLiteral("#000000") : QStringLiteral("#ffffff");
        button->setStyleSheet(QStringLiteral("QPushButton { background-color: %1; color: %2; "
                                             "border: 1px solid palette(mid); padding: 4px 10px; }")
                                  .arg(c.name(QColor::HexRgb), fg));
        button->setText(alpha && c.alphaF() < 1.0 ? c.name(QColor::HexArgb) : c.name(QColor::HexRgb));
    }
};

} // namespace

extern "C" {

void *day_colorpicker_new(double r, double g, double b, double a, int with_alpha,
                          const char *title, uint64_t id,
                          void (*cb)(uint64_t, double, double, double, double)) {
    DayColorWidget *w = new DayColorWidget();
    QVBoxLayout *lay = new QVBoxLayout(w);
    lay->setContentsMargins(0, 0, 0, 0);
    w->alpha = with_alpha != 0;
    w->title = QString::fromUtf8(title ? title : "");
    QPushButton *btn = new QPushButton();
    w->button = btn;
    lay->addWidget(btn);
    w->apply(QColor::fromRgbF(r, g, b, a));
    QObject::connect(btn, &QPushButton::clicked, [w, id, cb]() {
        QColorDialog::ColorDialogOptions opts;
        if (w->alpha)
            opts |= QColorDialog::ShowAlphaChannel;
        const QString title = w->title.isEmpty() ? QString() : w->title;
        const QColor picked = QColorDialog::getColor(w->color, w, title, opts);
        // An invalid color is the Cancel button: keep the last real pick rather than reporting
        // black, which is what `QColor()` would decode to.
        if (!picked.isValid())
            return;
        w->apply(picked);
        cb(id, picked.redF(), picked.greenF(), picked.blueF(), picked.alphaF());
    });
    return w;
}

void day_colorpicker_set(void *handle, double r, double g, double b, double a) {
    DayColorWidget *w = static_cast<DayColorWidget *>(handle);
    const QColor c = QColor::fromRgbF(r, g, b, a);
    if (w->color != c)
        w->apply(c);
}

} // extern "C"
