// The textarea piece's OWN Qt shim behind a flat C ABI: a QPlainTextEdit (Qt's lightweight multi-line
// plain-text editor) with a native placeholder, word wrapping, and an internal vertical scrollbar.
// textChanged reports edits back to Rust as a UTF-8 C string (valid only during the callback; Rust
// copies it); programmatic setPlainText is wrapped in blockSignals so it never echoes back as a change
// (mirrors the searchfield shim's setTextGuarded). `day_textarea_measure` computes the content-driven
// height, clamped to the [min_lines, max_lines] band (max_lines == 0 = unbounded). Qt libs are already
// linked by day-qt-sys.

#include <QFontMetricsF>
#include <QPlainTextEdit>
#include <QString>
#include <QTextDocument>

#include <cstdint>

class DayTextArea : public QPlainTextEdit {
public:
    void setTextGuarded(const QString &t) {
        if (toPlainText() != t) {
            blockSignals(true); // programmatic ⇒ no textChanged echo
            setPlainText(t);
            blockSignals(false);
        }
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
