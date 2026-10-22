// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Canvas export (docs/canvas.md "Exporting"): the canvas painter replayed into an offscreen
// bitmap that joins the decoded-bitmap registry, and onto QPdfWriter pages that stay vector.

#include <QApplication>
#include <QFontDatabase>
#include <QImage>
#include <cassert>
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
struct DayQtPdfPage {
    double w, h;
    const double *nums;
    int n;
    const char *texts;
};
unsigned char *day_qt_canvas_render_pdf(const DayQtPdfPage *pages, int count, const char *title,
                                        const char *author, int *out_len, char **err);
}

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

int main(int argc, char **argv) {
    QApplication app(argc, argv);

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
