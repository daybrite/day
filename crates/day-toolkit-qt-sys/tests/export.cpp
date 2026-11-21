// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Canvas export (docs/canvas.md "Exporting"): the canvas painter replayed into an offscreen
// bitmap that joins the decoded-bitmap registry, and onto QPdfWriter pages that stay vector.

#include <QApplication>
#include <QFontDatabase>
#include <QImage>
#include <cassert>
#include <cmath>
#include <cstdio>
#include <cstdint>
#include <cstdlib>
#include <iostream>
#include <vector>

extern "C" {
void day_qt_open_file(const char *) {}
int day_qt_canvas_render_image(unsigned long long id, const double *nums, int n,
                               const char *texts_joined, int pw, int ph, double scale);
unsigned char *day_qt_image_encode(unsigned long long id, const char *format, int quality,
                                   double fitW, double fitH, int *out_len);
void day_qt_bytes_free(unsigned char *p);
void day_qt_image_release(unsigned long long id);
void day_qt_string_free(char *p);
int day_qt_blur_boxes(double sigma, int out[6]);
void day_qt_blur_alpha(float *plane, int w, int h, double sigma);
struct DayQtPdfPage {
    double w, h;
    const double *nums;
    int n;
    const char *texts;
};
unsigned char *day_qt_canvas_render_pdf(const DayQtPdfPage *pages, int count, const char *title,
                                        const char *author, int *out_len, char **err);
}

// Where `effects` writes its PDF when a path is given on the command line.
static const char *argc_out = nullptr;

// One encoded record (day_spec::encode_ops): kind, seven geometry slots, ARGB color.
static void op(std::vector<double> &v, int kind, double a, double b, double c, double d,
               double e, double f, double g, uint32_t argb) {
    for (double x : {double(kind), a, b, c, d, e, f, g, double(argb)}) v.push_back(x);
}

// The conformance swatch at 120 × 80: red left, blue right, a white disc, a dark bottom bar.
static std::vector<double> swatch() {
    std::vector<double> v;
    op(v, 0, 0, 0, 60, 80, 0, 0, 0, 0xffff0000);
    op(v, 0, 60, 0, 60, 80, 0, 0, 0, 0xff0000ff);
    op(v, 3, 45, 25, 30, 30, 0, 0, 0, 0xffffffff);
    op(v, 0, 0, 64, 120, 16, 0, 0, 0, 0xff202020);
    return v;
}

// Render `nums` (with its texts channel) into a `w` × `h` point bitmap at `scale` and read it back.
static QImage render(const std::vector<double> &nums, const char *texts, int w, int h, double scale) {
    assert(day_qt_canvas_render_image(90, nums.data(), int(nums.size()), texts, int(w * scale),
                                      int(h * scale), scale) == 1);
    int len = 0;
    unsigned char *png = day_qt_image_encode(90, "png", -1, 0, 0, &len);
    QImage img;
    assert(png && img.loadFromData(png, len, "PNG"));
    day_qt_bytes_free(png);
    day_qt_image_release(90);
    return img;
}

static bool near(const QColor &got, const QColor &want, int slack = 6) {
    return std::abs(got.red() - want.red()) <= slack && std::abs(got.green() - want.green()) <= slack
        && std::abs(got.blue() - want.blue()) <= slack && std::abs(got.alpha() - want.alpha()) <= slack;
}

// The four hard-edged quarters of the conformance case: red, green, blue, yellow from 0°.
static const char *QUARTERS =
    "0,ffff0000 0.25,ffff0000 0.25,ff00ff00 0.5,ff00ff00 0.5,ff0000ff 0.75,ff0000ff "
    "0.75,ffffff00 1,ffffff00";

// Conic gradients, shadows and blend modes (docs/canvas.md) through the shared painter.
static void effects() {
    // The box filters: an odd width is centered three times; an even one leans left, leans
    // right, then widens by one. Every pass keeps the total, so a blur conserves coverage.
    int boxes[6];
    assert(day_qt_blur_boxes(0.0, boxes) == 0);
    assert(day_qt_blur_boxes(0.4, boxes) == 0);
    assert(day_qt_blur_boxes(1.6, boxes) == 1); // d = 3
    for (int v : boxes) assert(v == 1);
    assert(day_qt_blur_boxes(2.0, boxes) == 1); // d = 4
    assert(boxes[0] == 2 && boxes[1] == 1 && boxes[2] == 1 && boxes[3] == 2 && boxes[4] == 2 && boxes[5] == 2);
    const int n = 41;
    std::vector<float> plane(n * n, 0.0f);
    plane[20 * n + 20] = 255.0f;
    day_qt_blur_alpha(plane.data(), n, n, 2.0);
    double total = 0;
    for (float v : plane) total += v;
    assert(std::abs(total - 255.0) < 0.01);
    // Symmetric about the source, and falling away from it.
    assert(std::abs(plane[20 * n + 17] - plane[20 * n + 23]) < 1e-3);
    assert(std::abs(plane[17 * n + 20] - plane[23 * n + 20]) < 1e-3);
    assert(plane[20 * n + 20] > plane[20 * n + 22] && plane[20 * n + 22] > plane[20 * n + 25]);
    assert(plane[20 * n + 30] == 0.0f);
    std::cout << "blur: boxes, conservation, symmetry\n";

    // Conic: 0° is +x and the turn is clockwise on screen, so the first quarter is bottom-right.
    std::vector<double> v;
    op(v, 14, 0.5, 0.5, 0, 0, 8, 2, 0, 0);
    op(v, 0, 0, 0, 100, 100, 0, 0, 0, 0xffffffff);
    QImage img = render(v, QUARTERS, 100, 100, 1.0);
    assert(near(img.pixelColor(75, 75), QColor(255, 0, 0)));
    assert(near(img.pixelColor(25, 75), QColor(0, 255, 0)));
    assert(near(img.pixelColor(25, 25), QColor(0, 0, 255)));
    assert(near(img.pixelColor(75, 25), QColor(255, 255, 0)));
    // A start angle turns the whole wheel clockwise: at 90° the first quarter is bottom-left.
    v.clear();
    op(v, 14, 0.5, 0.5, 90, 0, 8, 2, 0, 0);
    op(v, 0, 0, 0, 100, 100, 0, 0, 0, 0xffffffff);
    img = render(v, QUARTERS, 100, 100, 1.0);
    assert(near(img.pixelColor(25, 75), QColor(255, 0, 0)));
    assert(near(img.pixelColor(25, 25), QColor(0, 255, 0)));
    // A stroke takes it too, and the gradient ends with its record.
    v.clear();
    op(v, 14, 0.5, 0.5, 0, 0, 8, 2, 0, 0);
    op(v, 4, 10, 10, 80, 80, 0, 0, 10, 0xffffffff);
    op(v, 0, 45, 45, 10, 10, 0, 0, 0, 0xff000000);
    img = render(v, QUARTERS, 100, 100, 1.0);
    assert(near(img.pixelColor(78, 78), QColor(255, 0, 0)));
    assert(near(img.pixelColor(22, 22), QColor(0, 0, 255)));
    assert(near(img.pixelColor(50, 50), QColor(0, 0, 0)));
    // A hard edge in a LINEAR gradient stays hard (Qt would merge two stops at one offset).
    v.clear();
    op(v, 14, 0, 0, 1, 0, 4, 0, 0, 0);
    op(v, 0, 0, 0, 100, 10, 0, 0, 0, 0xffffffff);
    img = render(v, "0,ffff0000 0.5,ffff0000 0.5,ff0000ff 1,ff0000ff", 100, 10, 1.0);
    assert(near(img.pixelColor(40, 5), QColor(255, 0, 0)));
    assert(near(img.pixelColor(60, 5), QColor(0, 0, 255)));
    std::cout << "conic: clockwise from +x, start angle, strokes, hard stops\n";

    // Shadow: set under scale(2,2) inside a save; the offset stays 20 points, the blur soft,
    // and the restore ends it. The conformance drawing at 2x.
    v.clear();
    op(v, 0, 0, 0, 200, 100, 0, 0, 0, 0xffffffff);
    op(v, 8, 0, 0, 0, 0, 0, 0, 0, 0);
    op(v, 10, 2, 0, 0, 2, 0, 0, 0, 0);
    op(v, 24, 0, 20, 2, 0, 0, 0, 0, 0xff000000);
    op(v, 0, 15, 10, 20, 15, 0, 0, 0, 0xff0000ff);
    op(v, 9, 0, 0, 0, 0, 0, 0, 0, 0);
    op(v, 0, 130, 20, 40, 30, 0, 0, 0, 0xff0000ff);
    img = render(v, "", 200, 100, 2.0);
    assert(near(img.pixelColor(100, 70), QColor(0, 0, 255)));  // the box, over its shadow
    assert(near(img.pixelColor(100, 120), QColor(0, 0, 0)));   // 20 points below, not 40
    assert(near(img.pixelColor(100, 170), QColor(255, 255, 255)));
    assert(near(img.pixelColor(300, 120), QColor(255, 255, 255))); // after the restore: none
    // The edge is a ramp, not a step: half way at the silhouette's edge (y = 70 points).
    const int edge = img.pixelColor(100, 140).red();
    assert(edge > 60 && edge < 200);
    assert(img.pixelColor(100, 136).red() < edge && edge < img.pixelColor(100, 144).red());
    // A transparent color clears it.
    v.clear();
    op(v, 24, 0, 20, 2, 0, 0, 0, 0, 0xff000000);
    op(v, 24, 0, 0, 0, 0, 0, 0, 0, 0x00000000);
    op(v, 0, 10, 10, 20, 20, 0, 0, 0, 0xff0000ff);
    img = render(v, "", 60, 60, 1.0);
    assert(img.pixelColor(20, 40).alpha() == 0);
    // Text and a stamped shape cast one as well.
    v.clear();
    op(v, 24, 30, 0, 0, 0, 0, 0, 0, 0xffff0000);
    op(v, 20, 2, 0, 0, 0, 0, 0, 0, 0);
    op(v, 21, 0, 0, 0, 40, 0, 0, 0, 0);
    op(v, 0, 0, 0, 10, 10, 0, 0, 0, 0xff0000ff);
    img = render(v, "", 60, 60, 1.0);
    assert(near(img.pixelColor(5, 5), QColor(0, 0, 255)) && near(img.pixelColor(35, 5), QColor(255, 0, 0)));
    assert(near(img.pixelColor(5, 45), QColor(0, 0, 255)) && near(img.pixelColor(35, 45), QColor(255, 0, 0)));
    std::cout << "shadow: offset in points under a transform, scoped, cleared, stamps\n";

    // Blend: multiply, add, the scope, an unknown mode, and erase leaving the bitmap clear.
    v.clear();
    op(v, 0, 0, 0, 200, 40, 0, 0, 0, 0xffffffff);
    op(v, 0, 0, 0, 50, 40, 0, 0, 0, 0xffffff00);
    op(v, 8, 0, 0, 0, 0, 0, 0, 0, 0);
    op(v, 25, 1, 0, 0, 0, 0, 0, 0, 0);
    op(v, 0, 0, 0, 50, 40, 0, 0, 0, 0xff00ffff);
    op(v, 9, 0, 0, 0, 0, 0, 0, 0, 0);
    op(v, 0, 50, 0, 50, 40, 0, 0, 0, 0xffff0000);
    op(v, 8, 0, 0, 0, 0, 0, 0, 0, 0);
    op(v, 25, 6, 0, 0, 0, 0, 0, 0, 0);
    op(v, 0, 50, 0, 50, 40, 0, 0, 0, 0xff0000ff);
    op(v, 9, 0, 0, 0, 0, 0, 0, 0, 0);
    op(v, 0, 100, 0, 50, 40, 0, 0, 0, 0xffffff00);
    op(v, 0, 100, 0, 50, 40, 0, 0, 0, 0xff0000ff);
    op(v, 25, 99, 0, 0, 0, 0, 0, 0, 0);
    op(v, 0, 150, 0, 50, 40, 0, 0, 0, 0xff00ff00);
    op(v, 25, 7, 0, 0, 0, 0, 0, 0, 0);
    op(v, 3, 10, 10, 20, 20, 0, 0, 0, 0xff00ff00);
    img = render(v, "", 200, 40, 1.0);
    assert(near(img.pixelColor(45, 5), QColor(0, 255, 0)));
    assert(near(img.pixelColor(75, 20), QColor(255, 0, 255)));
    assert(near(img.pixelColor(125, 20), QColor(0, 0, 255)));
    assert(near(img.pixelColor(175, 20), QColor(0, 255, 0)));
    assert(img.pixelColor(20, 20).alpha() == 0);
    std::cout << "blend: multiply, add, scope, unknown, erase\n";

    // A PDF page takes all three without failing: the shadow as an image, the conic as a
    // texture where the writer has no conical fill.
    v.clear();
    op(v, 24, 0, 6, 8, 0, 0, 0, 0, 0x80000000);
    op(v, 14, 0.5, 0.5, 0, 0, 8, 2, 0, 0);
    op(v, 3, 20, 20, 100, 100, 0, 0, 0, 0xffffffff);
    op(v, 25, 1, 0, 0, 0, 0, 0, 0, 0);
    op(v, 0, 40, 40, 60, 60, 0, 0, 0, 0xff00ffff);
    op(v, 25, 7, 0, 0, 0, 0, 0, 0, 0);
    op(v, 0, 60, 60, 20, 20, 0, 0, 0, 0xff000000);
    const DayQtPdfPage page[1] = {{200, 200, v.data(), int(v.size()), QUARTERS}};
    int len = 0;
    char *err = nullptr;
    unsigned char *pdf = day_qt_canvas_render_pdf(page, 1, nullptr, nullptr, &len, &err);
    assert(pdf && len > 0 && !err);
    const QByteArray doc(reinterpret_cast<const char *>(pdf), len);
    if (argc_out) {
        FILE *f = fopen(argc_out, "wb");
        if (f) { fwrite(pdf, 1, len, f); fclose(f); }
    }
    day_qt_bytes_free(pdf);
    assert(doc.contains("/Subtype /Image")); // the shadow
    // QPdfWriter offers no blend modes: multiply draws as normal, and an erase's records are
    // left out rather than drawn in the eraser's color.
    std::cout << "pdf: shadow, conic and blends, " << len << " bytes\n";
}

int main(int argc, char **argv) {
    QApplication app(argc, argv);
    // An optional path: where to leave the effects PDF, for a look at it.
    argc_out = argc > 1 ? argv[1] : nullptr;
    effects();

    // Bitmap: 120 × 80 points at 2× is 240 × 160 pixels, reachable through the registry.
    const std::vector<double> nums = swatch();
    assert(day_qt_canvas_render_image(7, nums.data(), int(nums.size()), "", 240, 160, 2.0) == 1);
    int len = 0;
    unsigned char *png = day_qt_image_encode(7, "png", -1, 0, 0, &len);
    assert(png && len > 0);
    QImage img;
    assert(img.loadFromData(png, len, "PNG"));
    day_qt_bytes_free(png);
    assert(img.size() == QSize(240, 160));
    assert(img.pixelColor(36, 48) == QColor(Qt::red));
    assert(img.pixelColor(204, 48) == QColor(Qt::blue));
    assert(img.pixelColor(120, 80) == QColor(Qt::white));
    // The bar sits at the BOTTOM: the export keeps the screen's top-left origin.
    assert(img.pixelColor(120, 150) == QColor(0x20, 0x20, 0x20));
    day_qt_image_release(7);
    assert(day_qt_image_encode(7, "png", -1, 0, 0, &len) == nullptr);
    std::cout << "bitmap: 240x160 at 2x, origin top-left, joined the registry\n";

    // Transparent background: an empty drawing stays alpha 0.
    assert(day_qt_canvas_render_image(8, nullptr, 0, "", 4, 4, 1.0) == 1);
    png = day_qt_image_encode(8, "png", -1, 0, 0, &len);
    assert(png && img.loadFromData(png, len, "PNG"));
    day_qt_bytes_free(png);
    assert(img.pixelColor(2, 2).alpha() == 0);
    day_qt_image_release(8);

    // PDF: two pages of their own sizes, with text drawn as text.
    std::vector<double> text = swatch();
    op(text, 7, 10, 10, 0, 0, 14, 0, 0, 0xff000000);
    const DayQtPdfPage pages[2] = {
        {612, 792, text.data(), int(text.size()), "Hello PDF"},
        {300, 200, nums.data(), int(nums.size()), ""},
    };
    char *err = nullptr;
    unsigned char *pdf = day_qt_canvas_render_pdf(pages, 2, "Export", "Day", &len, &err);
    assert(pdf && len > 0 && !err);
    const QByteArray doc(reinterpret_cast<const char *>(pdf), len);
    day_qt_bytes_free(pdf);
    assert(doc.startsWith("%PDF-"));
    assert(doc.contains("/MediaBox [0 0 612"));
    assert(doc.contains("/MediaBox [0 0 300"));
    assert(doc.contains("/Count 2"));
    assert(!doc.contains("/Subtype /Image"));
    // Where the host has any font at all, the text is embedded as a font, not rasterized.
    if (!QFontDatabase::families().isEmpty()) assert(doc.contains("/Type /Font"));
    std::cout << "pdf: " << len << " bytes, two pages, vector\n";

    assert(day_qt_canvas_render_pdf(pages, 0, nullptr, nullptr, &len, &err) == nullptr);
    assert(err);
    day_qt_string_free(err);
}
