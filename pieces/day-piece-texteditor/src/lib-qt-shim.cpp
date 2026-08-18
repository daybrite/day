// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-piece-texteditor's OWN Qt shim (the day-piece-colorpicker recipe): a `QTextEdit` — Qt's rich
// text editor, as opposed to the `QPlainTextEdit` the built-in text area uses — behind a flat C ABI.
//
// Attributes are applied through a QTextCursor rather than through `setHtml`, and that is the whole
// design of this file. `setHtml` would be one call for a whole document, but it REPLACES the
// document: the caret jumps, and the undo stack is cleared. A syntax highlighter re-styling on
// every keystroke would be unusable. Selecting a range and calling `setCharFormat` keeps both, and
// `beginEditBlock`/`endEditBlock` around the sweep collapses it into a single undo step and a
// single relayout.
//
// Positions are QChar counts — UTF-16 code units, the same unit the Apple arms use — so Rust sends
// UTF-16 offsets and nothing here converts.
//
// Colors cross as packed 0xAARRGGBB. Unlike the color picker's shim, which passes doubles because
// it IS the color source, a text color is a rendering input and Qt stores it 8-bit per channel.
//
// The editor's own formatting shortcuts (Ctrl+B/I/U) are Qt's, and they are turned OFF: attributes
// belong to Day (see the crate docs), and a shortcut that changed them behind Day's back would be
// repainted away by the next patch.

#include <QFontDatabase>
#include <QFontMetricsF>
#include <QKeyEvent>
#include <QScrollBar>
#include <QString>
#include <QTextBlockFormat>
#include <QTextCharFormat>
#include <QTextCursor>
#include <QTextDocument>
#include <QTextEdit>

#include <cstdint>

namespace {

class DayTextEditor : public QTextEdit {
public:
    uint64_t node = 0;
    void (*textCb)(uint64_t, const char *) = nullptr;
    void (*selCb)(uint64_t, uint64_t, uint64_t) = nullptr;
    // Set while Day itself writes the document, so a programmatic change never echoes back as if
    // the user had typed it.
    bool suppress = false;

    void reportText() {
        if (suppress || !textCb)
            return;
        QByteArray b = toPlainText().toUtf8();
        textCb(node, b.constData());
    }

    void reportSelection() {
        if (suppress || !selCb)
            return;
        const QTextCursor c = textCursor();
        selCb(node, static_cast<uint64_t>(c.selectionStart()),
              static_cast<uint64_t>(c.selectionEnd()));
    }

protected:
    // Qt's built-in Ctrl+B / Ctrl+I / Ctrl+U toggle the char format directly on the document.
    // Swallow them: Day owns attributes, and a toolbar button goes through the bound signal.
    void keyPressEvent(QKeyEvent *e) override {
        if (e->modifiers() & Qt::ControlModifier) {
            const int k = e->key();
            if (k == Qt::Key_B || k == Qt::Key_I || k == Qt::Key_U) {
                e->accept();
                return;
            }
        }
        QTextEdit::keyPressEvent(e);
    }
};

QColor unpack(uint32_t argb) {
    return QColor::fromRgb(static_cast<int>((argb >> 16) & 0xFF), static_cast<int>((argb >> 8) & 0xFF),
                           static_cast<int>(argb & 0xFF), static_cast<int>((argb >> 24) & 0xFF));
}

// A run's attributes as one QTextCharFormat. `underline` follows Day's `Underline`: 0 none,
// 1 single, 2 double, 3 dotted, 4 wavy.
QTextCharFormat runFormat(double pt, int weight, int italic, int mono, int underline, int strike,
                          int has_fg, uint32_t fg, int has_bg, uint32_t bg) {
    QTextCharFormat f;
    f.setFontPointSize(pt);
    f.setFontWeight(weight);
    f.setFontItalic(italic != 0);
    if (mono != 0) {
        // NOT the generic "monospace": Qt's rich text does not resolve a generic family from a
        // char format (day-qt's label path hit the same wall and worked around it with <code>).
        // `QFontDatabase::systemFont(FixedFont)` is the real fixed face the desktop ships.
        static const QString fixed = QFontDatabase::systemFont(QFontDatabase::FixedFont).family();
        f.setFontFamilies({fixed});
    }
    f.setFontStrikeOut(strike != 0);
    if (underline != 0) {
        f.setFontUnderline(true);
        switch (underline) {
        case 3:
            f.setUnderlineStyle(QTextCharFormat::DotLine);
            break;
        case 4:
            f.setUnderlineStyle(QTextCharFormat::WaveUnderline);
            break;
        // Qt draws no double rule, so a double underline degrades to a single one — stated in
        // docs/texteditor.md next to GTK's dotted, which degrades the same way.
        default:
            f.setUnderlineStyle(QTextCharFormat::SingleUnderline);
            break;
        }
    }
    if (has_fg != 0)
        f.setForeground(unpack(fg));
    if (has_bg != 0)
        f.setBackground(unpack(bg));
    return f;
}

QTextCursor cursorOver(DayTextEditor *w, int start, int len) {
    QTextCursor c(w->document());
    c.setPosition(start);
    c.setPosition(start + len, QTextCursor::KeepAnchor);
    return c;
}

} // namespace

extern "C" {

void *day_texteditor_new(uint64_t id, int editable, double base_pt, const char *placeholder,
                         void (*text_cb)(uint64_t, const char *),
                         void (*sel_cb)(uint64_t, uint64_t, uint64_t)) {
    DayTextEditor *w = new DayTextEditor();
    w->node = id;
    w->textCb = text_cb;
    w->selCb = sel_cb;
    w->setAcceptRichText(false); // a paste keeps its characters and takes the surrounding style
    w->setReadOnly(editable == 0);
    w->setLineWrapMode(QTextEdit::WidgetWidth);
    w->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    w->setVerticalScrollBarPolicy(Qt::ScrollBarAsNeeded);
    w->setPlaceholderText(QString::fromUtf8(placeholder ? placeholder : ""));
    QFont f = w->font();
    f.setPointSizeF(base_pt);
    w->setFont(f);
    QObject::connect(w, &QTextEdit::textChanged, [w]() { w->reportText(); });
    QObject::connect(w, &QTextEdit::selectionChanged, [w]() { w->reportSelection(); });
    QObject::connect(w, &QTextEdit::cursorPositionChanged, [w]() { w->reportSelection(); });
    return w;
}

// Replace the text, keeping the caret where the user left it. Signals are blocked throughout: this
// IS Day's own write, and the piece already knows the resulting text.
void day_texteditor_set_text(void *ptr, const char *utf8) {
    DayTextEditor *w = static_cast<DayTextEditor *>(ptr);
    const QString t = QString::fromUtf8(utf8 ? utf8 : "");
    if (w->toPlainText() == t)
        return;
    const int caret = w->textCursor().position();
    w->suppress = true;
    w->setPlainText(t);
    QTextCursor c = w->textCursor();
    c.setPosition(qMin(caret, w->document()->characterCount() - 1));
    w->setTextCursor(c);
    w->suppress = false;
}

// One attribute sweep: begin, reset to the base format, apply each run and paragraph, end. The
// edit block makes it one undo step and one relayout no matter how many runs it carries.
void day_texteditor_begin_attrs(void *ptr) {
    DayTextEditor *w = static_cast<DayTextEditor *>(ptr);
    w->suppress = true;
    QTextCursor c(w->document());
    c.beginEditBlock();
    c.select(QTextCursor::Document);
    QTextCharFormat base;
    base.setFontPointSize(w->font().pointSizeF());
    base.setFontWeight(QFont::Normal);
    base.setFontItalic(false);
    base.setFontUnderline(false);
    base.setFontStrikeOut(false);
    base.setFontFamilies({w->font().family()});
    c.setCharFormat(base);
    QTextBlockFormat bf;
    c.setBlockFormat(bf);
    c.endEditBlock();
}

void day_texteditor_apply_run(void *ptr, int start, int len, double pt, int weight, int italic,
                              int mono, int underline, int strike, int has_fg, uint32_t fg,
                              int has_bg, uint32_t bg) {
    DayTextEditor *w = static_cast<DayTextEditor *>(ptr);
    QTextCursor c = cursorOver(w, start, len);
    c.mergeCharFormat(runFormat(pt, weight, italic, mono, underline, strike, has_fg, fg, has_bg, bg));
}

// `align`: 0 natural, 1 center, 2 trailing, 3 justified. `marker` is non-zero for a list item,
// whose marker hangs in the gap the negative first-line indent opens.
void day_texteditor_apply_paragraph(void *ptr, int start, int len, int align, double indent,
                                    double space_before, double space_after, int marker) {
    DayTextEditor *w = static_cast<DayTextEditor *>(ptr);
    QTextCursor c = cursorOver(w, start, len);
    QTextBlockFormat f;
    switch (align) {
    case 1:
        f.setAlignment(Qt::AlignHCenter);
        break;
    case 2:
        f.setAlignment(Qt::AlignRight);
        break;
    case 3:
        f.setAlignment(Qt::AlignJustify);
        break;
    // Natural: leave the alignment unset, so the block follows the document's layout direction
    // and an Arabic or Hebrew paragraph starts on the right.
    default:
        break;
    }
    const double gap = marker != 0 ? 18.0 : 0.0;
    f.setLeftMargin(indent + gap);
    f.setTextIndent(-gap);
    f.setTopMargin(space_before);
    f.setBottomMargin(space_after);
    c.mergeBlockFormat(f);
}

void day_texteditor_end_attrs(void *ptr) {
    static_cast<DayTextEditor *>(ptr)->suppress = false;
}

void day_texteditor_set_selection(void *ptr, int start, int len) {
    DayTextEditor *w = static_cast<DayTextEditor *>(ptr);
    w->suppress = true;
    w->setTextCursor(cursorOver(w, start, len));
    w->suppress = false;
}

// Qt's typing style is the nicest of the eight: with a collapsed cursor, the current char format IS
// what the next character takes.
void day_texteditor_set_typing(void *ptr, double pt, int weight, int italic, int mono, int underline,
                               int strike, int has_fg, uint32_t fg, int has_bg, uint32_t bg) {
    DayTextEditor *w = static_cast<DayTextEditor *>(ptr);
    w->setCurrentCharFormat(
        runFormat(pt, weight, italic, mono, underline, strike, has_fg, fg, has_bg, bg));
}

void day_texteditor_set_editable(void *ptr, int editable) {
    static_cast<DayTextEditor *>(ptr)->setReadOnly(editable == 0);
}

// Content-driven height for the proposed width, clamped to the line band (`max_lines == 0` =
// unbounded) — the same shape as the built-in text area's measure.
void day_texteditor_measure(void *ptr, double avail_w, uint32_t min_lines, uint32_t max_lines,
                            double *out_w, double *out_h) {
    DayTextEditor *w = static_cast<DayTextEditor *>(ptr);
    QFontMetricsF fm(w->font());
    const double line_h = fm.lineSpacing();
    QTextDocument *doc = w->document();
    const double frame = w->frameWidth();
    const double pad = 2.0 * frame + 2.0 * doc->documentMargin();
    double inner_w = avail_w - pad;
    if (inner_w < 1.0)
        inner_w = 1.0;
    doc->setTextWidth(inner_w);
    double h = doc->size().height() + 2.0 * frame;
    const double min_h = static_cast<double>(min_lines) * line_h + pad;
    const double max_h = (max_lines > 0) ? static_cast<double>(max_lines) * line_h + pad : 1.0e12;
    if (h < min_h)
        h = min_h;
    if (h > max_h)
        h = max_h;
    *out_w = avail_w;
    *out_h = h;
}

} // extern "C"
