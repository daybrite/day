// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The textarea piece's OWN Qt shim behind a flat C ABI: a QPlainTextEdit (Qt's lightweight multi-line
// plain-text editor) with a native placeholder, word wrapping, and an internal vertical scrollbar.
// textChanged reports edits back to Rust as a UTF-8 C string (valid only during the callback; Rust
// copies it); programmatic setPlainText is wrapped in blockSignals so it never echoes back as a change
// (mirrors the searchfield shim's setTextGuarded). `day_textarea_measure` computes the content-driven
// height, clamped to the [min_lines, max_lines] band (max_lines == 0 = unbounded). Qt libs are already
// linked by day-qt-sys.

#include <QFontMetricsF>
#include <QKeyEvent>
#include <QPlainTextEdit>
#include <QString>
#include <QTextDocument>

#include <cstdint>

class DayTextArea : public QPlainTextEdit {
public:
    // Submit-on-enter (day's TextAreaProps::submit_on_enter): armed via day_textarea_set_submit.
    uint64_t submitId = 0;
    void (*submitCb)(uint64_t) = nullptr;

    void setTextGuarded(const QString &t) {
        if (toPlainText() != t) {
            blockSignals(true); // programmatic ⇒ no textChanged echo
            setPlainText(t);
            blockSignals(false);
        }
    }

protected:
    // A plain Return/Enter submits instead of inserting a newline; Shift+Return still breaks.
    void keyPressEvent(QKeyEvent *e) override {
        if (submitCb && (e->key() == Qt::Key_Return || e->key() == Qt::Key_Enter) &&
            !(e->modifiers() & Qt::ShiftModifier)) {
            submitCb(submitId);
            return;
        }
        QPlainTextEdit::keyPressEvent(e);
    }
};

extern "C" {

void *day_textarea_new(const char *placeholder, const char *initial, uint64_t id,
                       void (*cb)(uint64_t, const char *)) {
    DayTextArea *w = new DayTextArea();
    w->setPlaceholderText(QString::fromUtf8(placeholder));
    w->setLineWrapMode(QPlainTextEdit::WidgetWidth);
    w->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    w->setVerticalScrollBarPolicy(Qt::ScrollBarAsNeeded);
    if (initial && *initial)
        w->setPlainText(QString::fromUtf8(initial));
    QObject::connect(w, &QPlainTextEdit::textChanged, [id, cb, w]() {
        QByteArray b = w->toPlainText().toUtf8();
        cb(id, b.constData());
    });
    return w;
}

void day_textarea_set_text(void *w, const char *text) {
    static_cast<DayTextArea *>(w)->setTextGuarded(QString::fromUtf8(text));
}

// Arm submit-on-enter: `cb(id)` fires on a plain Return/Enter press (the newline is claimed).
void day_textarea_set_submit(void *ptr, uint64_t id, void (*cb)(uint64_t)) {
    DayTextArea *w = static_cast<DayTextArea *>(ptr);
    w->submitId = id;
    w->submitCb = cb;
}

// editable / selectable. Qt has no built-in spell-check (Cap::TextSpellCheck = Unsupported), so
// there's no setter for it. Editing implies selection; a read-only editor is selectable when asked,
// otherwise inert. The two single-attribute setters read the current state of the other attribute
// off the widget so they stay consistent.
static void applyTextAreaAttrs(QPlainTextEdit *w, bool editable, bool selectable) {
    w->setReadOnly(!editable);
    Qt::TextInteractionFlags flags;
    if (editable)
        flags = Qt::TextEditorInteraction;
    else if (selectable)
        flags = Qt::TextSelectableByMouse | Qt::TextSelectableByKeyboard;
    else
        flags = Qt::NoTextInteraction;
    w->setTextInteractionFlags(flags);
}

void day_textarea_set_attrs(void *ptr, int editable, int selectable) {
    applyTextAreaAttrs(static_cast<QPlainTextEdit *>(ptr), editable != 0, selectable != 0);
}

void day_textarea_set_read_only(void *ptr, int read_only) {
    QPlainTextEdit *w = static_cast<QPlainTextEdit *>(ptr);
    bool selectable = (w->textInteractionFlags() &
                       (Qt::TextSelectableByMouse | Qt::TextSelectableByKeyboard)) != 0;
    applyTextAreaAttrs(w, read_only == 0, selectable);
}

void day_textarea_set_selectable(void *ptr, int selectable) {
    QPlainTextEdit *w = static_cast<QPlainTextEdit *>(ptr);
    applyTextAreaAttrs(w, !w->isReadOnly(), selectable != 0);
}

// Content-driven height for the proposed width, clamped to the line band. `max_lines == 0` = unbounded.
void day_textarea_measure(void *ptr, double avail_w, uint32_t min_lines, uint32_t max_lines,
                          double *out_w, double *out_h) {
    QPlainTextEdit *w = static_cast<QPlainTextEdit *>(ptr);
    QFontMetricsF fm(w->font());
    double line_h = fm.lineSpacing();
    QTextDocument *doc = w->document();
    double frame = w->frameWidth();
    double doc_margin = doc->documentMargin();
    double pad = 2.0 * frame + 2.0 * doc_margin;

    double inner_w = avail_w - pad;
    if (inner_w < 1.0)
        inner_w = 1.0;
    doc->setTextWidth(inner_w);
    double content_h = doc->size().height() + 2.0 * frame;

    double min_h = static_cast<double>(min_lines) * line_h + pad;
    double max_h = (max_lines > 0) ? static_cast<double>(max_lines) * line_h + pad : 1.0e12;
    double h = content_h;
    if (h < min_h)
        h = min_h;
    if (h > max_h)
        h = max_h;

    *out_w = avail_w;
    *out_h = h;
}

} // extern "C"
