// The combobox piece's own Qt shim: QComboBox behind a flat C ABI. Items cross joined by '\n'.

#include <QComboBox>
#include <QString>
#include <QStringList>

#include <cstdint>

extern "C" {

void *day_combo_new(const char *items_joined, int selected, uint64_t id,
                    void (*cb)(uint64_t, int)) {
    QComboBox *c = new QComboBox();
    c->addItems(QString::fromUtf8(items_joined).split(QChar('\n'), Qt::SkipEmptyParts));
    if (selected >= 0) c->setCurrentIndex(selected);
    QObject::connect(c, QOverload<int>::of(&QComboBox::currentIndexChanged),
                     [id, cb](int idx) { cb(id, idx); });
    return c;
}

void day_combo_set_items(void *w, const char *items_joined) {
    QComboBox *c = static_cast<QComboBox *>(w);
    c->blockSignals(true);
    c->clear();
    c->addItems(QString::fromUtf8(items_joined).split(QChar('\n'), Qt::SkipEmptyParts));
    c->blockSignals(false);
}

void day_combo_set_selected(void *w, int idx) {
    QComboBox *c = static_cast<QComboBox *>(w);
    if (c->currentIndex() != idx) c->setCurrentIndex(idx);
}

} // extern "C"
