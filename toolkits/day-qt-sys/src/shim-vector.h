// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Retain SVG geometry until paint time. In particular, screenshot scale and monitor scale
// may differ: use the painter's complete device transform, not qApp's primary screen DPR.
#pragma once

#include <QIconEngine>
#include <QPainter>
#include <QPointer>
#include <QSvgRenderer>
#include <QVariant>
#include <QWidget>
#include <cmath>
#include <memory>

static QRectF day_qt_vector_rect(QSizeF source, const QRectF &box, int mode = 0) {
    if (source.isEmpty() || box.isEmpty() || mode == 2) return box;
    qreal scale = mode == 1 ? qMax(box.width() / source.width(), box.height() / source.height())
                           : qMin(box.width() / source.width(), box.height() / source.height());
    QSizeF size = source * scale;
    return QRectF(box.center() - QPointF(size.width() / 2, size.height() / 2), size);
}

static void day_qt_paint_svg(QPainter *p, QSvgRenderer &svg, const QRectF &box,
                              const QColor &tint = QColor(), int mode = 0) {
    if (!svg.isValid() || box.isEmpty()) return;
    const QRectF dest = day_qt_vector_rect(svg.defaultSize(), box, mode);
    p->save();
    p->setClipRect(box, Qt::IntersectClip);
    if (!tint.isValid()) {
        svg.render(p, dest);
    } else {
        // Alpha compositing preserves authored holes, strokes and partial opacity. The
        // temporary layer is rendered from geometry at the actual device resolution.
        const QTransform transform = p->deviceTransform();
        const qreal sx = std::hypot(transform.m11(), transform.m12());
        const qreal sy = std::hypot(transform.m21(), transform.m22());
        const QSize pixels(qMax(1, int(std::ceil(box.width() * sx))),
                           qMax(1, int(std::ceil(box.height() * sy))));
        QImage layer(pixels, QImage::Format_ARGB32_Premultiplied);
        layer.fill(Qt::transparent);
        QPainter mask(&layer);
        mask.scale(pixels.width() / box.width(), pixels.height() / box.height());
        svg.render(&mask, dest.translated(-box.topLeft()));
        mask.setCompositionMode(QPainter::CompositionMode_SourceIn);
        mask.fillRect(QRectF(QPointF(), box.size()), tint);
        mask.end();
        p->drawImage(box, layer);
    }
    p->restore();
}

class DaySvgIconEngine : public QIconEngine {
    std::shared_ptr<QSvgRenderer> svg;
    QColor tint;
    QPointer<QWidget> owner;
    QRectF inkBounds;
    qreal inkFill = 0.0;
public:
    DaySvgIconEngine(const QString &file, const QColor &ink, QWidget *widget = nullptr, qreal fill = 0.0)
        : svg(std::make_shared<QSvgRenderer>(file)), tint(ink), owner(widget), inkFill(fill) {
        if (fill <= 0 || !svg->isValid()) return;
        // Measure ink once for toolbar normalization. Only bounds survive; every paint
        // still renders the SVG itself, never this measurement image.
        QImage measure(512, 512, QImage::Format_ARGB32_Premultiplied);
        measure.fill(Qt::transparent);
        QPainter p(&measure);
        day_qt_paint_svg(&p, *svg, QRectF(0, 0, 512, 512));
        p.end();
        int left = 512, top = 512, right = -1, bottom = -1;
        for (int y = 0; y < 512; ++y) {
            const QRgb *row = reinterpret_cast<const QRgb *>(measure.constScanLine(y));
            for (int x = 0; x < 512; ++x) if (qAlpha(row[x]) > 8) {
                left = qMin(left, x); top = qMin(top, y);
                right = qMax(right, x); bottom = qMax(bottom, y);
            }
        }
        if (right >= left) inkBounds = QRectF(left, top, right - left + 1, bottom - top + 1);
    }
    bool valid() const { return svg->isValid(); }
    QIconEngine *clone() const override { return new DaySvgIconEngine(*this); }
    void paint(QPainter *p, const QRect &rect, QIcon::Mode mode, QIcon::State) override {
        QColor ink = tint;
        if (owner) {
            const auto group = mode == QIcon::Disabled ? QPalette::Disabled : QPalette::Active;
            ink = owner->property("dayButtonInk").value<QColor>();
            if (!ink.isValid() || mode == QIcon::Disabled)
                ink = owner->palette().color(group, QPalette::ButtonText);
        }
        if (inkBounds.isEmpty()) {
            day_qt_paint_svg(p, *svg, rect, ink);
        } else {
            p->save();
            p->setClipRect(rect, Qt::IntersectClip);
            const qreal scale = qMin(rect.width(), rect.height()) * inkFill
                / qMax(inkBounds.width(), inkBounds.height());
            p->translate(QRectF(rect).center());
            p->scale(scale, scale);
            p->translate(-inkBounds.center());
            day_qt_paint_svg(p, *svg, QRectF(0, 0, 512, 512), ink);
            p->restore();
        }
    }
    QPixmap pixmap(const QSize &size, QIcon::Mode mode, QIcon::State state) override {
        return scaledPixmap(size, mode, state, 1.0);
    }
    QPixmap scaledPixmap(const QSize &size, QIcon::Mode mode, QIcon::State state, qreal scale) override {
        // QIcon passed device pixels before 6.8; newer Qt passes logical pixels.
        // Keep both the distro Qt and Homebrew/MSYS2 Qt sharp at fractional DPRs.
#if QT_VERSION >= QT_VERSION_CHECK(6, 8, 0)
        const QSize pixels = (QSizeF(size) * scale).toSize();
#else
        const QSize pixels = size;
#endif
        QPixmap result(pixels);
        result.fill(Qt::transparent);
        QPainter painter(&result);
        paint(&painter, QRect(QPoint(), pixels), mode, state);
        painter.end();
        result.setDevicePixelRatio(scale);
        return result;
    }
};

static QIcon day_qt_svg_icon(const QString &file, const QColor &ink, QWidget *owner = nullptr, qreal fill = 0.0) {
    auto *engine = new DaySvgIconEngine(file, ink, owner, fill);
    if (engine->valid()) return QIcon(engine);
    delete engine;
    return QIcon();
}
