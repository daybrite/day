// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The picker piece's own Qt shim: three stylings behind a flat C ABI. Options cross joined by '\n'.
// style: 0 = menu (QComboBox), 1 = segmented (checkable QPushButtons), 2 = inline (QRadioButtons).
// Segmented/inline share an exclusive QButtonGroup; `idClicked` fires on USER clicks only, so
// programmatic `setSelected` never echoes back.

#include <QButtonGroup>
#include <QComboBox>
#include <QHBoxLayout>
#include <QPushButton>
#include <QRadioButton>
#include <QString>
#include <QStringList>
#include <QVBoxLayout>
#include <QWidget>

#include <cstdint>

class DayPicker : public QWidget {
public:
    QComboBox *combo = nullptr;
    QButtonGroup *group = nullptr;
    int style = 0;
    uint64_t node = 0;
    void (*cb)(uint64_t, int) = nullptr;
    // New option labels, in place. The combo swaps its items; the button styles relabel what
    // they have and add or drop the tail, because each button carries its OWN group id.
    void setOptions(const QStringList &items) {
        int keep = 0;
        if (combo) {
            keep = combo->currentIndex();
            combo->blockSignals(true);
            combo->clear();
            combo->addItems(items);
            if (!items.isEmpty())
                combo->setCurrentIndex(qBound(0, keep, items.size() - 1));
            combo->blockSignals(false);
            return;
        }
        if (!group)
            return;
        QAbstractButton *checked = group->checkedButton();
        keep = checked ? group->id(checked) : 0;
        QList<QAbstractButton *> have = group->buttons();
        for (int i = 0; i < items.size(); i++) {
            if (i < have.size()) {
                have[i]->setText(items[i]);
                continue;
            }
            QAbstractButton *b;
            if (style == 1) {
                QPushButton *pb = new QPushButton(items[i]);
                pb->setCheckable(true);
                b = pb;
            } else {
                b = new QRadioButton(items[i]);
            }
            group->addButton(b, i);
            layout()->addWidget(b);
        }
        for (int i = items.size(); i < have.size(); i++) {
            group->removeButton(have[i]);
            layout()->removeWidget(have[i]);
            have[i]->deleteLater();
        }
        if (!items.isEmpty())
            setSelected(qBound(0, keep, items.size() - 1));
    }
    void setSelected(int idx) {
        if (combo) {
            if (combo->currentIndex() != idx) {
                combo->blockSignals(true);
                combo->setCurrentIndex(idx);
                combo->blockSignals(false);
            }
        } else if (group) {
            QAbstractButton *b = group->button(idx);
            if (b && !b->isChecked())
                b->setChecked(true); // programmatic ⇒ toggled, not clicked: no echo
        }
    }
};

extern "C" {

void *day_picker_new(int style, const char *items_joined, int selected, uint64_t id,
                     void (*cb)(uint64_t, int)) {
    QStringList items = QString::fromUtf8(items_joined).split(QChar('\n'), Qt::SkipEmptyParts);
    DayPicker *w = new DayPicker();
    if (style == 0) {
        QVBoxLayout *lay = new QVBoxLayout(w);
        lay->setContentsMargins(0, 0, 0, 0);
        QComboBox *c = new QComboBox();
        c->addItems(items);
        if (selected >= 0)
            c->setCurrentIndex(selected);
        QObject::connect(c, QOverload<int>::of(&QComboBox::currentIndexChanged),
                         [id, cb](int idx) { cb(id, idx); });
        lay->addWidget(c);
        w->combo = c;
    } else {
        QBoxLayout *lay = (style == 1) ? static_cast<QBoxLayout *>(new QHBoxLayout(w))
                                       : static_cast<QBoxLayout *>(new QVBoxLayout(w));
        lay->setContentsMargins(0, 0, 0, 0);
        lay->setSpacing(style == 1 ? 0 : 2);
        QButtonGroup *g = new QButtonGroup(w);
        g->setExclusive(true);
        for (int i = 0; i < items.size(); i++) {
            QAbstractButton *b;
            if (style == 1) {
                QPushButton *pb = new QPushButton(items[i]);
                pb->setCheckable(true);
                b = pb;
            } else {
                b = new QRadioButton(items[i]);
            }
            if (i == selected)
                b->setChecked(true);
            g->addButton(b, i);
            lay->addWidget(b);
        }
        QObject::connect(g, &QButtonGroup::idClicked, [id, cb](int idx) { cb(id, idx); });
        w->group = g;
    }
    w->style = style;
    w->node = id;
    w->cb = cb;
    return w;
}

void day_picker_set_selected(void *w, int idx) { static_cast<DayPicker *>(w)->setSelected(idx); }

void day_picker_set_options(void *w, const char *items_joined) {
    DayPicker *p = static_cast<DayPicker *>(w);
    QStringList items = QString::fromUtf8(items_joined).split(QChar('\n'), Qt::SkipEmptyParts);
    p->setOptions(items);
}

} // extern "C"
