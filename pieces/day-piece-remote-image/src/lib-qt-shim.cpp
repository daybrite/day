// The remote-image piece's OWN Qt shim behind a flat C ABI: a QWidget that paints a QPixmap decoded
// from encoded bytes (PNG/JPEG/…), aspect fit/fill, under a centered-circle / rounded / rectangular
// clip, over the placeholder color. Painting in paintEvent (rather than a QLabel + setPixmap) makes
// the clip resize-correct for free and gives true aspect-fill. Bytes cross as a pointer + length
// (valid only during the call; QPixmap::loadFromData copies them). Qt libs are already linked by
// day-qt-sys.

#include <QColor>
#include <QPainter>
#include <QPainterPath>
#include <QPixmap>
#include <QWidget>

#include <cstddef>
#include <cstdint>

class DayRemoteImage : public QWidget {
public:
    int clipKind = 0; // 0 none, 1 circle, 2 rounded
    double radius = 0.0;
    int mode = 1; // 1 fill (cover), 0 fit (contain)
    QColor placeholder;
    QPixmap pixmap;

    DayRemoteImage() { setAttribute(Qt::WA_TranslucentBackground, true); }

    void setBytes(const uint8_t *data, size_t len) {
        if (!data || len == 0) {
            pixmap = QPixmap();
        } else {
            QPixmap pm;
            if (pm.loadFromData(data, static_cast<uint>(len)))
                pixmap = pm;
            else
                pixmap = QPixmap();
        }
        update();
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        p.setRenderHint(QPainter::SmoothPixmapTransform, true);
        QRectF r(0, 0, width(), height());

        QPainterPath path;
        if (clipKind == 1) {
            double d = qMin(r.width(), r.height());
            QRectF sq((r.width() - d) / 2.0, (r.height() - d) / 2.0, d, d);
            path.addEllipse(sq);
        } else if (clipKind == 2) {
            path.addRoundedRect(r, radius, radius);
        } else {
            path.addRect(r);
        }
        p.setClipPath(path);

        // Placeholder fill (shows in Fit's letterbox margins and while there's no pixmap).
        p.fillRect(r, placeholder);

        if (!pixmap.isNull()) {
            Qt::AspectRatioMode arm =
                (mode == 1) ? Qt::KeepAspectRatioByExpanding : Qt::KeepAspectRatio;
            QPixmap scaled = pixmap.scaled(r.size().toSize(), arm, Qt::SmoothTransformation);
            double x = (r.width() - scaled.width()) / 2.0;
            double y = (r.height() - scaled.height()) / 2.0;
            p.drawPixmap(QPointF(x, y), scaled);
        }
    }
};

extern "C" {

void *day_remote_image_new(int clip, double radius, int mode, double r, double g, double b,
                           double a) {
    DayRemoteImage *w = new DayRemoteImage();
    w->clipKind = clip;
    w->radius = radius;
    w->mode = mode;
    w->placeholder = QColor::fromRgbF(r, g, b, a);
    return w;
}

void day_remote_image_set_bytes(void *w, const uint8_t *data, uint64_t len) {
    static_cast<DayRemoteImage *>(w)->setBytes(data, static_cast<size_t>(len));
}

} // extern "C"
