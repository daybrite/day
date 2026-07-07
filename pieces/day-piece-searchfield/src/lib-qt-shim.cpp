// The search-field piece's OWN Qt shim behind a flat C ABI: a QLineEdit dressed as a search box —
// a built-in clear button (setClearButtonEnabled) and a leading magnifier action. textChanged
// reports edits back to Rust as a UTF-8 C string (valid only during the callback; Rust copies it);
// programmatic setText is wrapped in blockSignals so it never echoes back as a change (mirrors the
// picker shim's setSelected). Qt libs are already linked by day-qt-sys.

#include <QAction>
#include <QIcon>
#include <QLineEdit>
#include <QString>

#include <cstdint>

class DaySearch : public QLineEdit {
public:
    void setTextGuarded(const QString &t) {
        if (text() != t) {
            blockSignals(true); // programmatic ⇒ no textChanged echo
            setText(t);
            blockSignals(false);
        }
    }
};

extern "C" {

void *day_search_new(const char *placeholder, const char *initial, uint64_t id,
                     void (*cb)(uint64_t, const char *)) {
    DaySearch *w = new DaySearch();
    w->setPlaceholderText(QString::fromUtf8(placeholder));
    w->setClearButtonEnabled(true);
    // Leading magnifier icon (from the icon theme; harmless no-op where the theme lacks it).
    w->addAction(QIcon::fromTheme(QStringLiteral("edit-find")), QLineEdit::LeadingPosition);
    if (initial && *initial)
        w->setText(QString::fromUtf8(initial));
    QObject::connect(w, &QLineEdit::textChanged, [id, cb](const QString &t) {
        QByteArray b = t.toUtf8();
        cb(id, b.constData());
    });
    return w;
}

void day_search_set_text(void *w, const char *text) {
    static_cast<DaySearch *>(w)->setTextGuarded(QString::fromUtf8(text));
}

} // extern "C"
