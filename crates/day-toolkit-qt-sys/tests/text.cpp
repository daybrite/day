// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//
// Day's label on Qt (docs/text.md): the line limit with each truncation mode, the measured
// height against what QLabel shows, line height and letter spacing in ems, the full text the
// label reports, Qt's parse of the decoration markup the toolkit writes, the label's own
// direction, the text scale, and the selection reports under real QTest input (drags, a
// double click, the keyboard, a text change). Synthetic node IDs are test fixtures.
#include <QApplication>
#include <QFontMetricsF>
#include <QLabel>
#include <QStyle>
#include <QTest>
#include <QTextBlock>
#include <QTextDocument>
#include <QWidget>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <utility>
#include <vector>

extern "C" {
void day_qt_open_file(const char *) {}
void *day_qt_label_new(const char *, int);
void day_qt_label_set_text(void *, const char *);
void day_qt_label_set_rich_text(void *, const char *);
void day_qt_label_set_paragraph(void *, int, int, double, double);
void day_qt_label_set_font(void *, double, int, int, int, int, int);
void day_qt_label_set_selectable(void *, int);
void day_qt_label_on_selection(void *, uint64_t, void (*)(uint64_t, int, int));
void day_qt_label_set_align(void *, int);
void day_qt_widget_set_direction(void *, int);
int day_qt_label_natural_width(void *);
int day_qt_label_height_for_width(void *, int);
double day_qt_baseline(void *, double);
double day_qt_text_scale(void);
struct DayQtNative {
    int found, role, level;
    char *label, *value, *identifier, *text;
    int number_kind, number, checked, enabled, visible, frame_ok;
    double x, y, w, h;
};
void day_qt_read_native(void *, DayQtNative *);
void day_qt_native_free(DayQtNative *);
}

#define CHECK(cond)                                                                              \
    do {                                                                                         \
        if (!(cond)) {                                                                           \
            std::fprintf(stderr, "%s:%d: CHECK failed: %s\n", __FILE__, __LINE__, #cond);        \
            std::abort();                                                                        \
        }                                                                                        \
    } while (0)

static std::vector<std::pair<int, int>> selections;
static void on_selection(uint64_t node, int start, int end) {
    CHECK(node == 7);
    selections.emplace_back(start, end);
}

static std::string native_text(void *w) {
    DayQtNative n{};
    day_qt_read_native(w, &n);
    std::string out = n.text ? n.text : "";
    day_qt_native_free(&n);
    return out;
}

static const char *LONG = "Day lays this sentence out over many lines when its label is narrow, so a "
                          "line limit has plenty of text to cut and the ellipsis has something to "
                          "stand for.";
static const char *PATH = "/usr/local/share/day/templates/app/src/pages.rs";

int main(int argc, char **argv) {
    QApplication app(argc, argv);
    QWidget top;
    top.resize(600, 600);
    top.show();
    {
        const bool exposed = QTest::qWaitForWindowExposed(&top);
        CHECK(exposed);
        top.activateWindow();
        const bool active = QTest::qWaitForWindowActive(&top);
        CHECK(active);
    }
    const QChar ellipsis(0x2026);
    auto label = [&](const char *text, int wraps, double pt) {
        auto *l = static_cast<QLabel *>(day_qt_label_new(text, wraps));
        day_qt_label_set_font(l, pt, 400, 0, 0, 0, 0);
        l->setParent(&top);
        l->show();
        return l;
    };
    auto place = [](QLabel *l, int x, int y, int w) {
        l->setGeometry(x, y, w, day_qt_label_height_for_width(l, w));
    };

    // The line limit: the measured height is that of as many lines, the label shows that many
    // lines ending in an ellipsis, and the text it reports stays whole.
    QLabel *one = label("Ag", 1, 13);
    QLabel *two = label("Ag\nAg", 1, 13);
    QLabel *longl = label(LONG, 1, 13);
    const int h1 = day_qt_label_height_for_width(one, 100);
    const int h2 = day_qt_label_height_for_width(two, 100);
    CHECK(h2 > h1);
    CHECK(day_qt_label_height_for_width(longl, 100) > h2 + h1);
    day_qt_label_set_paragraph(longl, 2, 0, 0, 0);
    CHECK(std::abs(day_qt_label_height_for_width(longl, 100) - h2) <= 1);
    place(longl, 0, 0, 100);
    CHECK(longl->text().count(QLatin1Char('\n')) == 1);
    CHECK(longl->text().endsWith(ellipsis));
    CHECK(longl->sizeHint().height() <= h2 + 1);
    CHECK(native_text(longl) == LONG);
    CHECK(longl->accessibleName() == QString::fromUtf8(LONG));
    day_qt_label_set_paragraph(longl, 1, 0, 0, 0);
    CHECK(std::abs(day_qt_label_height_for_width(longl, 100) - h1) <= 1);
    place(longl, 0, 0, 100);
    CHECK(!longl->text().contains(QLatin1Char('\n')));
    day_qt_label_set_paragraph(longl, 0, 0, 0, 0);
    place(longl, 0, 0, 100);
    CHECK(longl->text() == QString::fromUtf8(LONG));
    CHECK(longl->accessibleName().isEmpty());

    // Truncation on one line, each mode, and the text reported whole.
    QLabel *path = label(PATH, 0, 13);
    place(path, 0, 100, 120);
    CHECK(path->text().endsWith(ellipsis)); // tail, the default
    day_qt_label_set_paragraph(path, 0, 2, 0, 0);
    CHECK(path->text().contains(ellipsis) && path->text().startsWith(QLatin1String("/u")) &&
          path->text().endsWith(QLatin1String("rs")));
    day_qt_label_set_paragraph(path, 0, 1, 0, 0);
    CHECK(path->text().startsWith(ellipsis) && path->text().endsWith(QLatin1String("pages.rs")));
    day_qt_label_set_paragraph(path, 0, 0, 0, 0);
    CHECK(path->text().startsWith(QLatin1String("/usr")) && path->text().endsWith(ellipsis));
    CHECK(QFontMetricsF(path->font()).horizontalAdvance(path->text()) <= 120);
    CHECK(native_text(path) == PATH);
    CHECK(day_qt_label_height_for_width(path, 120) == day_qt_label_height_for_width(one, 120));
    CHECK(day_qt_label_natural_width(path) > 120); // the natural width is the whole text's

    // A rich-text label held to one line keeps its runs and ends in an ellipsis.
    QLabel *rich = label("", 1, 13);
    day_qt_label_set_rich_text(rich, "<b>Bold</b> and plain words that run on well past one line");
    day_qt_label_set_paragraph(rich, 1, 0, 0, 0);
    CHECK(std::abs(day_qt_label_height_for_width(rich, 100) - h1) <= 1);
    place(rich, 0, 140, 100);
    CHECK(rich->textFormat() == Qt::RichText);
    CHECK(rich->text().contains(QLatin1String("<b>Bold</b>")) && rich->text().contains(ellipsis));
    CHECK(native_text(rich) == "Bold and plain words that run on well past one line");

    // A label placed at its natural width shows its runs on one line (synthesized small
    // capitals and a wavy underline included): the measure is the layout QLabel paints.
    {
        auto *dec = static_cast<QLabel *>(day_qt_label_new("Check unknown and old in Small Caps", 1));
        dec->setParent(&top);
        dec->show();
        day_qt_label_set_align(dec, 0);
        day_qt_label_set_font(dec, 13, 400, 0, 0, 0, 0);
        day_qt_label_set_paragraph(dec, 0, 0, 0, 0);
        day_qt_label_set_rich_text(
            dec, "Check <span style=\"text-underline-style:wave;text-decoration-color:#d0021b;\">"
                 "unknown</span> and <s>old</s> in <span style=\"font-variant:small-caps;\">Small "
                 "Caps</span>");
        const int w = day_qt_label_natural_width(dec);
        dec->setGeometry(0, 180, 162, 48); // first placed narrower, as a layout pass may
        QTest::qWait(20);
        place(dec, 0, 180, w);
        QTest::qWait(20);
        CHECK(std::abs(day_qt_label_height_for_width(dec, w) - h1) <= 1);
        CHECK(std::abs(dec->heightForWidth(w) - h1) <= 1);
    }

    // Line height and letter spacing are multiples of the font's em, in pixels.
    QLabel *lh = label("Ag", 1, 20);
    const double em = 20.0 * lh->logicalDpiY() / 72.0;
    day_qt_label_set_paragraph(lh, 0, 0, 0, 2.0);
    CHECK(std::abs(day_qt_label_height_for_width(lh, 300) - 2 * em) <= 1.0);
    CHECK(std::abs(day_qt_baseline(lh, 2 * em) - 1.6 * em) < 0.01);
    QLabel *lh2 = label("Ag\nAg", 1, 20);
    day_qt_label_set_paragraph(lh2, 0, 0, 0, 2.0);
    CHECK(std::abs(day_qt_label_height_for_width(lh2, 300) - 4 * em) <= 1.5);
    day_qt_label_set_paragraph(lh, 0, 0, 0, 1.5);
    CHECK(std::abs(day_qt_label_height_for_width(lh, 300) - 1.5 * em) <= 1.0);
    QLabel *plain = label("HHHH", 1, 20);
    QLabel *spaced = label("HHHH", 1, 20);
    day_qt_label_set_paragraph(spaced, 0, 0, 0.5, 0);
    const int grown = day_qt_label_natural_width(spaced) - day_qt_label_natural_width(plain);
    CHECK(std::abs(grown - 4 * 0.5 * em) <= 1.5);
    day_qt_label_set_font(spaced, 10, 400, 0, 0, 0, 0); // the spacing follows a font change
    CHECK(std::abs(spaced->font().letterSpacing() - 0.5 * 10 * spaced->logicalDpiY() / 72.0) < 1.0 / 32); // QFont keeps 1/64 px

    // The decoration markup the toolkit writes (day-toolkit-qt `qt_label_markup`) parses into
    // the underline style and color Qt draws; a strikethrough has no color of its own.
    {
        QTextDocument doc;
        doc.setHtml("<span style=\"text-underline-style:wave;text-decoration-color:#d0021b;\">"
                    "unknown</span> <span style=\"text-underline-style:dotted;\">dots</span>");
        QTextBlock::iterator it = doc.begin().begin();
        CHECK(it.fragment().charFormat().underlineStyle() == QTextCharFormat::WaveUnderline);
        CHECK(it.fragment().charFormat().underlineColor() == QColor(0xd0, 0x02, 0x1b));
        ++it;
        ++it;
        CHECK(it.fragment().charFormat().underlineStyle() == QTextCharFormat::DotLine);
    }

    // The label's own direction: leading is the right edge, the base direction right to left.
    QLabel *rtl = label("Leading", 1, 13);
    CHECK(!(QStyle::visualAlignment(rtl->layoutDirection(), rtl->alignment()) & Qt::AlignRight));
    day_qt_widget_set_direction(rtl, 1);
    CHECK(rtl->layoutDirection() == Qt::RightToLeft);
    CHECK(QStyle::visualAlignment(rtl->layoutDirection(), rtl->alignment()) & Qt::AlignRight);
    CHECK(rtl->text().contains(QLatin1String("dir=\"rtl\"")));
    CHECK(native_text(rtl) == "Leading");
    day_qt_label_set_align(rtl, 2); // trailing: the left edge
    CHECK(QStyle::visualAlignment(rtl->layoutDirection(), rtl->alignment()) & Qt::AlignLeft);

    // The text scale: the platform's reference DPI is 1.0.
    const double scale = day_qt_text_scale();
    CHECK(std::isfinite(scale) && scale > 0);

    // Selection reports under real input, as byte offsets into the displayed text.
    QWindow *win = top.windowHandle();
    CHECK(win);
    const char *words = "Grüße aus Köln";
    QLabel *sel = label(words, 1, 13);
    day_qt_label_set_selectable(sel, 1);
    day_qt_label_on_selection(sel, 7, on_selection);
    place(sel, 0, 300, 400);
    QTest::qWait(10);
    const QFontMetricsF fm(sel->font());
    const int y = 300 + sel->height() / 2;
    const int gruesse = int(std::lround(fm.horizontalAdvance(QString::fromUtf8("Grüße"))));
    QTest::mousePress(win, Qt::LeftButton, {}, QPoint(1, y));
    CHECK(selections.empty());
    QTest::mouseMove(win, QPoint(gruesse / 2, y));
    QTest::mouseMove(win, QPoint(gruesse, y));
    QTest::mouseRelease(win, Qt::LeftButton, {}, QPoint(gruesse, y));
    CHECK(!selections.empty());
    CHECK((selections.back() == std::pair<int, int>{0, 7})); // "Grüße" is 7 bytes
    const size_t afterDrag = selections.size();
    // Each change reported once: a release where the drag ended adds nothing.
    for (size_t i = 1; i < afterDrag; i++) CHECK(selections[i] != selections[i - 1]);
    // A double click selects the word under it.
    const int aus = int(std::lround(fm.horizontalAdvance(QString::fromUtf8("Grüße a"))));
    QTest::mouseDClick(win, Qt::LeftButton, {}, QPoint(aus, y));
    CHECK((selections.back() == std::pair<int, int>{8, 11}));
    // The keyboard: select all.
    QTest::keyClick(sel, Qt::Key_A, Qt::ControlModifier);
    CHECK((selections.back() == std::pair<int, int>{0, int(std::strlen(words))}));
    // A text change discards the selection, which is reported cleared.
    day_qt_label_set_text(sel, "Neu");
    CHECK((selections.back() == std::pair<int, int>{0, 0}));
    const size_t cleared = selections.size();
    day_qt_label_set_text(sel, "Neu");
    CHECK(selections.size() == cleared);

    // An elided label maps a selection of what it shows onto the whole text.
    day_qt_label_set_text(sel, PATH);
    day_qt_label_set_paragraph(sel, 1, 1, 0, 0);
    sel->setGeometry(0, 300, 120, sel->height());
    CHECK(sel->text().startsWith(ellipsis));
    sel->setFocus();
    QTest::keyClick(sel, Qt::Key_A, Qt::ControlModifier);
    CHECK((selections.back() == std::pair<int, int>{0, int(std::strlen(PATH))}));
    {
        // Drag over the last three characters shown ("." "r" "s"): the end of the whole text.
        // Past the double-click interval first, or the press is the double click's third.
        QTest::qWait(QApplication::doubleClickInterval() + 100);
        const int w = int(std::lround(fm.horizontalAdvance(sel->text())));
        const int from = int(std::lround(fm.horizontalAdvance(sel->text().chopped(3))));
        // A press inside a selection would drag the selected text: click it away first.
        QTest::mouseClick(win, Qt::LeftButton, {}, QPoint(from, y));
        CHECK((selections.back() == std::pair<int, int>{0, 0}));
        QTest::qWait(QApplication::doubleClickInterval() + 100);
        QTest::mousePress(win, Qt::LeftButton, {}, QPoint(from, y));
        QTest::mouseMove(win, QPoint(w - 1, y));
        QTest::mouseMove(win, QPoint(w + 4, y));
        QTest::mouseRelease(win, Qt::LeftButton, {}, QPoint(w + 4, y));
        const int n = int(std::strlen(PATH));
        CHECK((selections.back() == std::pair<int, int>{n - 3, n}));
    }
    std::puts("text: ok");
    return 0;
}
