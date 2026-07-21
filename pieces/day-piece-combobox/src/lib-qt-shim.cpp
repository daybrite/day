// The combo piece's OWN Qt shim behind a flat C ABI: an EDITABLE QComboBox — Qt's real combo
// box (free text + a dropdown of items). editTextChanged fires on typing AND when picking an
// item (the pick writes the edit text), so it is the single change path back to Rust (UTF-8,
// valid only during the callback; Rust copies it). Programmatic setters are wrapped in
// blockSignals so they never echo. Items cross joined by '\n'. Qt libs are already linked by
// day-qt-sys.

#include <QComboBox>
#include <QLineEdit>
#include <QString>
#include <QStringList>

#include <cstdint>

extern "C" {

void *day_combo_new(const char *items_joined, const char *text, const char *placeholder,
                    uint64_t id, void (*cb)(uint64_t, const char *)) {
    QComboBox *c = new QComboBox();
    c->setEditable(true);
    c->setInsertPolicy(QComboBox::NoInsert); // typing Enter must not grow the list
    c->addItems(QString::fromUtf8(items_joined).split(QChar('\n'), Qt::SkipEmptyParts));
    if (c->lineEdit())
        c->lineEdit()->setPlaceholderText(QString::fromUtf8(placeholder));
    // The TEXT is the value: nothing pre-selected (addItems auto-selects item 0 on an editable
    // combo, writing it into the edit — undo that), then seed the entry.
    c->setCurrentIndex(-1);
    c->setEditText(QString::fromUtf8(text));
    QObject::connect(c, &QComboBox::editTextChanged, [id, cb](const QString &t) {
        QByteArray b = t.toUtf8();
        cb(id, b.constData());
    });
    return c;
}

void day_combo_set_items(void *w, const char *items_joined) {
    QComboBox *c = static_cast<QComboBox *>(w);
    const QString keep = c->currentText(); // the text is the value; it survives the list swap
    c->blockSignals(true);
    c->clear(); // clears the edit text too — restored below
    c->addItems(QString::fromUtf8(items_joined).split(QChar('\n'), Qt::SkipEmptyParts));
    c->setCurrentIndex(-1);
    c->setEditText(keep);
    c->blockSignals(false);
}

void day_combo_set_text(void *w, const char *text) {
    QComboBox *c = static_cast<QComboBox *>(w);
    const QString t = QString::fromUtf8(text);
    if (c->currentText() != t) {
        c->blockSignals(true);
        c->setEditText(t);
        c->blockSignals(false);
    }
}

} // extern "C"
