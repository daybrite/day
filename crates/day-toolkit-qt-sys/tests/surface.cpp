// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//
// A styled surface (docs/surfaces.md) painted by the shim: the border is drawn over the
// surface's content, also after a child mounts later and after the surface resizes, and it goes
// with the border; a radial fill's first stop is at its center and its last at the edge; a conic
// fill's quarters run clockwise from +x; a solid fill replaces a gradient; and a container that
// had a stylesheet fill drops it when it starts painting. Each is read from a grab of the
// widget, as `day test` reads pixels. Then the appearance (docs/color.md): an override reaches
// day-toolkit-qt's callback once, on the next turn, however many signals it raised; a dark
// override is dark (through the platform theme, or the hand-built palette where the
// theme ignores it) and the roles follow; a light one gives the light palette back.
#include <QApplication>
#include <QColor>
#include <QImage>
#include <QPixmap>
#include <QWidget>
#include <cmath>
#include <cstdio>
#include <cstdlib>

extern "C" {
void day_qt_open_file(const char *) {}
void day_qt_widget_set_surface(void *, double, double, double, double, double, int);
void day_qt_widget_set_bg(void *, double, double, double, double, const void *);
void day_qt_widget_set_styled_surface(void *, const double *, const double *, int);
int day_qt_widget_has_styled_surface(void *);
void day_qt_on_appearance(void (*)(void));
int day_qt_set_appearance(int);
int day_qt_dark_mode(void);
int day_qt_system_color(int, double *);
}

#define CHECK(cond)                                                                              \
    do {                                                                                         \
        if (!(cond)) {                                                                           \
            std::fprintf(stderr, "%s:%d: CHECK failed: %s\n", __FILE__, __LINE__, #cond);        \
            std::abort();                                                                        \
        }                                                                                        \
    } while (0)

// The color at unit point (x, y) of `w`, from a grab of it and its children.
static QColor at(QWidget *w, double x, double y) {
    QCoreApplication::processEvents();
    const QImage img = w->grab().toImage();
    const int px = qBound(0, int(x * img.width()), img.width() - 1);
    const int py = qBound(0, int(y * img.height()), img.height() - 1);
    return img.pixelColor(px, py);
}

// Within 64 per channel of `want` (`sample_pixel`'s tolerance).
static bool near(const QColor &c, const QColor &want) {
    return std::abs(c.red() - want.red()) <= 64 && std::abs(c.green() - want.green()) <= 64 &&
           std::abs(c.blue() - want.blue()) <= 64;
}

static void spec_base(double *spec) {
    for (int i = 0; i < 24; i++) spec[i] = 0.0;
    spec[5] = 15.0; // every corner
}

static void border_over_children(QWidget *top) {
    auto *card = new QWidget(top);
    card->setGeometry(0, 0, 80, 80);
    card->show();
    double spec[24];
    spec_base(spec);
    spec[4] = 12.0;                                       // radius
    spec[7] = 1.0, spec[10] = 1.0, spec[11] = 8.0;        // red border, 8 wide
    day_qt_widget_set_styled_surface(card, spec, nullptr, 0);
    CHECK(day_qt_widget_has_styled_surface(card));
    // A child mounted after the border, filling the surface edge to edge.
    auto *child = new QWidget(card);
    child->setAutoFillBackground(true);
    QPalette p = child->palette();
    p.setColor(QPalette::Window, Qt::blue);
    child->setPalette(p);
    child->setGeometry(card->rect());
    child->show();
    QCoreApplication::processEvents();
    CHECK(near(at(card, 0.5, 0.5), Qt::blue));
    CHECK(near(at(card, 0.5, 0.04), Qt::red));
    CHECK(near(at(card, 0.04, 0.5), Qt::red));
    // The border follows the surface as it grows.
    card->resize(120, 80);
    child->setGeometry(card->rect());
    CHECK(near(at(card, 0.98, 0.5), Qt::red));
    // No border: the child shows to the edge.
    spec[11] = 0.0;
    day_qt_widget_set_styled_surface(card, spec, nullptr, 0);
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    CHECK(near(at(card, 0.5, 0.04), Qt::blue));
    delete card;
}

static void radial_and_conic(QWidget *top) {
    auto *radial = new QWidget(top);
    radial->setGeometry(100, 0, 80, 80);
    radial->show();
    double spec[24];
    spec_base(spec);
    spec[19] = 2.0, spec[20] = 0.5, spec[21] = 0.5, spec[22] = 0.5;
    const double rstops[] = {0.0, 1, 0, 0, 1, 0.2, 1, 0, 0, 1, 0.8, 0, 0, 1, 1, 1.0, 0, 0, 1, 1};
    day_qt_widget_set_styled_surface(radial, spec, rstops, 4);
    CHECK(near(at(radial, 0.5, 0.5), Qt::red));
    CHECK(near(at(radial, 0.5, 0.97), Qt::blue));
    // A solid fill replaces the gradient.
    day_qt_widget_set_bg(radial, 0, 1, 0, 1, nullptr);
    CHECK(near(at(radial, 0.5, 0.5), Qt::green));

    auto *conic = new QWidget(top);
    conic->setGeometry(200, 0, 80, 80);
    conic->show();
    spec_base(spec);
    spec[19] = 3.0, spec[20] = 0.5, spec[21] = 0.5, spec[22] = 0.0;
    const double cstops[] = {0.0,  1, 0, 0, 1, 0.25, 1, 0, 0, 1, 0.25, 0, 1, 0, 1, 0.5, 0, 1, 0, 1,
                             0.5,  0, 0, 1, 1, 0.75, 0, 0, 1, 1, 0.75, 1, 1, 0, 1, 1.0, 1, 1, 0, 1};
    day_qt_widget_set_styled_surface(conic, spec, cstops, 8);
    // Clockwise from +x on screen: right-below red, left-below green, left-above blue,
    // right-above yellow.
    CHECK(near(at(conic, 0.8, 0.6), Qt::red));
    CHECK(near(at(conic, 0.2, 0.6), Qt::green));
    CHECK(near(at(conic, 0.2, 0.4), Qt::blue));
    CHECK(near(at(conic, 0.8, 0.4), Qt::yellow));
}

static void stylesheet_fill_gives_way(QWidget *top) {
    auto *box = new QWidget(top);
    box->setGeometry(300, 0, 80, 80);
    box->show();
    day_qt_widget_set_surface(box, 1, 0, 0, 1, 0, 0);
    CHECK(near(at(box, 0.5, 0.5), Qt::red));
    CHECK(!day_qt_widget_has_styled_surface(box));
    double spec[24];
    spec_base(spec);
    spec[19] = 1.0, spec[21] = 0.0, spec[23] = 1.0; // vertical linear
    const double stops[] = {0.0, 0, 0, 1, 1, 1.0, 0, 0, 1, 1};
    day_qt_widget_set_styled_surface(box, spec, stops, 2);
    CHECK(box->styleSheet().isEmpty());
    CHECK(near(at(box, 0.5, 0.5), Qt::blue));
}

static int g_calls = 0;
static void counted() { g_calls++; }

static void appearance_follows() {
    day_qt_on_appearance(counted);
#if QT_VERSION >= QT_VERSION_CHECK(6, 8, 0)
    CHECK(day_qt_set_appearance(1));
    CHECK(g_calls == 0); // not inside Qt's change
    QCoreApplication::processEvents();
    CHECK(day_qt_dark_mode());
    CHECK(g_calls == 1);
    double bg[4];
    CHECK(day_qt_system_color(3, bg)); // Background: the palette's window color
    CHECK(bg[0] < 0.5 && bg[3] == 1.0);
    double sep[4];
    CHECK(day_qt_system_color(8, sep)); // Separator: the text color at 0.15
    CHECK(sep[0] > 0.5 && std::abs(sep[3] - 0.15) < 1e-9);
    CHECK(!day_qt_system_color(13, sep)); // Destructive: no palette role
    CHECK(day_qt_set_appearance(0));
    QCoreApplication::processEvents();
    CHECK(!day_qt_dark_mode());
#endif
    day_qt_on_appearance(nullptr);
}

int main(int argc, char **argv) {
    QApplication app(argc, argv);
    QWidget top;
    top.resize(400, 200);
    top.show();
    border_over_children(&top);
    radial_and_conic(&top);
    stylesheet_fill_gives_way(&top);
    appearance_follows();
    std::printf("surface: ok\n");
    return 0;
}
