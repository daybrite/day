// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The settings piece's Qt shim (src/lib-qt.rs) behind a flat C ABI: the KDE preferences-dialog
// structure (KConfigDialog / KPageDialog in its list face, the shape of Plasma's System
// Settings pages). A fixed-width QListWidget of icon+label page entries beside a QStackedWidget
// of pages under a title header; each page a column of QGroupBoxes (one per section, the title
// as the box title, the footer a dim label under the rows) holding a QFormLayout of
// right-aligned labels and fields. The field of a control row is a CELL, a plain QWidget Day's
// own control widget is moved into once Day has mounted it (`day_settings_qt_slot_mount`); the
// cell takes the size Day measured (`day_settings_qt_cell_set_size`), since a QLayout overrides
// `setGeometry` on what it manages, and the form lays the cell out like any field. A spanning
// cell (a custom row, a group's custom piece) reports the width the form grants it, so Day can
// re-lay its piece at that width.
//
// Callbacks reach Rust outside Qt's own dispatch where a Day reaction could rebuild widgets
// (a page selection: posted to the next loop turn); a button click calls back inline, as
// day-toolkit-qt's own buttons do. Qt libs are already linked by day-toolkit-qt-sys.

#include <QApplication>
#include <QEvent>
#include <QFormLayout>
#include <QFrame>
#include <QGroupBox>
#include <QHBoxLayout>
#include <QHash>
#include <QIcon>
#include <QLabel>
#include <QListWidget>
#include <QPalette>
#include <QPointer>
#include <QPushButton>
#include <QResizeEvent>
#include <QScrollArea>
#include <QStackedWidget>
#include <QStyle>
#include <QStyleOptionButton>
#include <QStylePainter>
#include <QTimer>
#include <QVBoxLayout>
#include <QVector>
#include <QWidget>

#include <cmath>
#include <cstdint>

// From day-toolkit-qt-sys (already linked into the binary): the toolbar's icon resolution.
extern "C" void *day_qt_icon_new(const char *spec, int standard_pixmap, int px);
extern "C" void day_qt_icon_delete(void *icon);

namespace {

// Row kinds, the `RowShape` of lib-qt.rs.
enum Kind { KIND_SLOT = 0, KIND_BUTTON = 1, KIND_LINK = 2, KIND_NOTE = 3, KIND_FILL = 4 };

// The sidebar's icon box; KDE's list face draws 22px icons.
constexpr int kListIconPx = 22;
// Standard dialog margins and spacing (the KDE HIG's base unit, matching QStyle's defaults).
constexpr int kMargin = 12;
constexpr int kSpacing = 12;

QString utf8(const char *s) { return s ? QString::fromUtf8(s) : QString(); }

// A dim, wrapping explanatory label: a caption under a control, a section footer, a note. The
// palette's placeholder color is the one secondary text role every style sets, and a
// foreground ROLE follows a theme switch where a fixed color would not.
QLabel *dim_label(const QString &text) {
    auto *l = new QLabel(text);
    l->setWordWrap(true);
    l->setForegroundRole(QPalette::PlaceholderText);
    QFont f = l->font();
    f.setPointSizeF(f.pointSizeF() * 0.92);
    l->setFont(f);
    l->setTextInteractionFlags(Qt::TextSelectableByMouse);
    return l;
}

// The negative text color of the current theme: Breeze's on a light window, a lighter red
// that stays readable on a dark one.
QColor negative_text(const QPalette &pal) {
    return pal.color(QPalette::Window).lightness() < 128 ? QColor(0xf0, 0x6e, 0x6e)
                                                          : QColor(0xda, 0x44, 0x53);
}

// A destructive command's button: the native button with its title in the negative text
// color. The style draws the frame with no title, and the title is painted over it, since the
// macOS style draws a light-mode button's title through AppKit and ignores the palette's
// `ButtonText` there.
class DayDangerButton : public QPushButton {
public:
    explicit DayDangerButton(const QString &title) : QPushButton(title) {}

protected:
    void paintEvent(QPaintEvent *) override {
        QStylePainter p(this);
        QStyleOptionButton opt;
        initStyleOption(&opt);
        const QString title = opt.text;
        opt.text.clear();
        p.drawControl(QStyle::CE_PushButton, opt);
        p.setPen(isEnabled() ? negative_text(palette()) : palette().color(QPalette::Disabled, QPalette::ButtonText));
        p.setFont(font());
        p.drawText(style()->subElementRect(QStyle::SE_PushButtonContents, &opt, this), Qt::AlignCenter, title);
    }
};

// The widget a Day control lives in once mounted: sized by Day (`setFixedSize`) for a control
// field; width-granted by the form for a spanning cell, which reports that width back.
class DayCell : public QWidget {
public:
    uint64_t key = 0;
    uint64_t node = 0;
    bool fill = false;
    void (*on_fill)(uint64_t, uint64_t, double) = nullptr;
    int reported = -1;
    bool pending = false;

    DayCell(uint64_t k, bool f) : key(k), fill(f) {
        if (fill) setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
        else setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Fixed);
    }

protected:
    void resizeEvent(QResizeEvent *e) override {
        QWidget::resizeEvent(e);
        // Outside the layout pass Qt is running (shim-resize.h's rule): a Day relayout sets
        // sizes, which must not land mid-pass. One report per turn, with the latest width.
        if (fill && on_fill && width() != reported && !pending) {
            pending = true;
            QPointer<DayCell> self(this);
            QTimer::singleShot(0, this, [self] {
                if (!self) return;
                self->pending = false;
                if (self->width() == self->reported) return;
                self->reported = self->width();
                self->on_fill(self->node, self->key, double(self->width()));
            });
        }
    }
};

struct DayRow {
    QLabel *label = nullptr;    // the label column (a control row's title)
    QWidget *field = nullptr;   // the field column: the cell, the button, the link label
    QPushButton *button = nullptr;
    QLabel *link = nullptr;
    QLabel *caption = nullptr;  // the dim line under the field
    QLabel *note = nullptr;     // a note row's spanning label
    DayCell *cell = nullptr;
};

struct DayGroup;

// The host: everything the shell realizes, keyed for patches.
class DayShell : public QWidget {
public:
    uint64_t node = 0;
    void (*on_select)(uint64_t, int) = nullptr;
    void (*on_row)(uint64_t, uint64_t) = nullptr;
    void (*on_fill)(uint64_t, uint64_t, double) = nullptr;

    QListWidget *list = nullptr;      // the page list (null with one page)
    QLabel *header = nullptr;         // the selected page's title (null with one page)
    QFrame *rule = nullptr;
    QStackedWidget *stack = nullptr;
    QVector<QWidget *> pages;         // each page's content widget (inside its scroll area)
    QVector<DayGroup *> groups;
    QHash<uint64_t, DayRow> rows;
    // Programmatic selection in flight, or the list still being built: no echo as a user
    // selection. The first `addItem` on an empty QListWidget makes row 0 current by itself,
    // which would otherwise report a selection of the first page over the one Day asked for.
    bool suppress = true;
    bool sidebar = false;

    explicit DayShell(bool with_sidebar) : sidebar(with_sidebar) {
        auto *row = new QHBoxLayout(this);
        row->setContentsMargins(kMargin, kMargin, kMargin, kMargin);
        row->setSpacing(kSpacing);
        if (sidebar) {
            list = new QListWidget(this);
            list->setIconSize(QSize(kListIconPx, kListIconPx));
            list->setSelectionMode(QAbstractItemView::SingleSelection);
            list->setUniformItemSizes(true);
            list->setSpacing(1);
            list->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
            list->setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Expanding);
            row->addWidget(list);
        }
        auto *column = new QWidget(this);
        auto *col = new QVBoxLayout(column);
        col->setContentsMargins(0, 0, 0, 0);
        col->setSpacing(kSpacing / 2);
        if (sidebar) {
            header = new QLabel(column);
            QFont f = header->font();
            f.setPointSizeF(f.pointSizeF() * 1.25);
            f.setBold(true);
            header->setFont(f);
            col->addWidget(header);
            rule = new QFrame(column);
            rule->setFrameShape(QFrame::HLine);
            rule->setFrameShadow(QFrame::Sunken);
            col->addWidget(rule);
        }
        stack = new QStackedWidget(column);
        col->addWidget(stack, 1);
        row->addWidget(column, 1);
    }

    int page_width(double for_w) const {
        int w = int(std::lround(for_w)) - 2 * kMargin;
        if (list) w -= list->minimumWidth() + kSpacing;
        return w;
    }

    // The right column's chrome above the page.
    int header_height() const {
        if (!header) return 0;
        return header->sizeHint().height() + rule->sizeHint().height() + kSpacing;
    }

    void select(int index) {
        if (index < 0 || index >= stack->count()) return;
        suppress = true;
        stack->setCurrentIndex(index);
        if (list && list->currentRow() != index) list->setCurrentRow(index);
        if (header) {
            if (auto *item = list->item(index)) header->setText(item->text());
        }
        suppress = false;
    }
};

struct DayGroup {
    QWidget *box = nullptr;       // the QGroupBox (a titled section) or a plain widget
    QGroupBox *titled = nullptr;
    QFormLayout *form = nullptr;
    QLabel *footer = nullptr;
};

// Drop every cached size under `layout`, through the group widgets' own forms. A hidden page
// never processes the layout requests its cells' size changes post, so nothing there ever runs
// `updateGeometry` on a group widget, and the page layout's item for it (a `QWidgetItemV2`)
// keeps the size hint it cached before Day sized the cells: `invalidate()` on the layouts
// alone leaves that cache in place.
void invalidate_tree(QLayout *layout) {
    for (int i = 0; QLayoutItem *item = layout->itemAt(i); ++i) {
        if (QLayout *nested = item->layout()) invalidate_tree(nested);
        if (QWidget *w = item->widget()) {
            if (w->layout()) invalidate_tree(w->layout());
            w->updateGeometry();
        }
    }
    layout->invalidate();
}

QFormLayout *make_form(QWidget *parent) {
    auto *form = new QFormLayout(parent);
    form->setLabelAlignment(Qt::AlignRight | Qt::AlignVCenter);
    form->setFormAlignment(Qt::AlignLeft | Qt::AlignTop);
    form->setFieldGrowthPolicy(QFormLayout::ExpandingFieldsGrow);
    form->setRowWrapPolicy(QFormLayout::DontWrapRows);
    return form;
}

} // namespace

extern "C" {

void *day_settings_qt_shell_new(uint64_t node, int sidebar, void (*on_select)(uint64_t, int),
                                void (*on_row)(uint64_t, uint64_t),
                                void (*on_fill)(uint64_t, uint64_t, double)) {
    auto *shell = new DayShell(sidebar != 0);
    shell->node = node;
    shell->on_select = on_select;
    shell->on_row = on_row;
    shell->on_fill = on_fill;
    if (shell->list) {
        QObject::connect(shell->list, &QListWidget::currentRowChanged, shell, [shell](int row) {
            if (shell->suppress || row < 0) return;
            shell->select(row);
            QPointer<DayShell> self(shell);
            QTimer::singleShot(0, shell, [self, row] {
                if (self && self->on_select) self->on_select(self->node, row);
            });
        });
    }
    return shell;
}

// A page: a list entry (icon + title) and a scrolling column of groups. Returns its index.
int day_settings_qt_shell_add_page(void *host, const char *title, const char *icon_spec,
                                   int icon_fallback) {
    auto *shell = static_cast<DayShell *>(host);
    auto *scroll = new QScrollArea(shell->stack);
    scroll->setFrameShape(QFrame::NoFrame);
    scroll->setWidgetResizable(true);
    scroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    // The page scrolls like its window: the viewport takes the window color, not the
    // text-entry base a QScrollArea paints by default.
    scroll->viewport()->setAutoFillBackground(false);
    auto *page = new QWidget();
    page->setAutoFillBackground(false);
    auto *col = new QVBoxLayout(page);
    col->setContentsMargins(0, 0, 0, 0);
    col->setSpacing(kSpacing);
    col->addStretch(1);
    scroll->setWidget(page);
    const int index = shell->stack->addWidget(scroll);
    shell->pages.push_back(page);
    if (shell->list) {
        auto *item = new QListWidgetItem(utf8(title));
        if (icon_spec && *icon_spec) {
            void *icon = day_qt_icon_new(icon_spec, icon_fallback, kListIconPx);
            item->setIcon(*static_cast<QIcon *>(icon));
            day_qt_icon_delete(icon);
        }
        shell->list->addItem(item);
    }
    return index;
}

// A section as a group on `page`: a titled QGroupBox, or a plain form for an untitled section
// (KDE pages put top-level rows straight on the page). The footer is a dim label under the rows.
void *day_settings_qt_page_add_group(void *host, int page, const char *title, const char *footer) {
    auto *shell = static_cast<DayShell *>(host);
    if (page < 0 || page >= shell->pages.size()) return nullptr;
    auto *g = new DayGroup();
    const QString t = utf8(title);
    if (!t.isEmpty()) {
        g->titled = new QGroupBox(t);
        g->box = g->titled;
    } else {
        g->box = new QWidget();
    }
    g->form = make_form(g->box);
    const QString f = utf8(footer);
    g->footer = dim_label(f);
    g->footer->setVisible(!f.isEmpty());
    auto *col = static_cast<QVBoxLayout *>(shell->pages[page]->layout());
    // Before the trailing stretch.
    col->insertWidget(col->count() - 1, g->box);
    shell->groups.push_back(g);
    return g;
}

// A row of `group`'s form. `kind`: 0 a control cell as the field under `label`; 1 a button
// titled `text` (with a trailing arrow for a navigate row, red for a destructive one); 2 a link
// titled `text`; 3 a note (a spanning dim label with `text`); 4 a spanning cell. A caption is a
// dim line under the field. Returns the cell for kinds 0 and 4, and for a button row with
// `with_cell` (a busy action's spinner beside the button); else null.
void *day_settings_qt_group_add_row(void *host, void *group, uint64_t key, int kind,
                                    const char *label, const char *text, const char *caption,
                                    int arrow, int destructive, int with_cell, int enabled) {
    auto *shell = static_cast<DayShell *>(host);
    auto *g = static_cast<DayGroup *>(group);
    if (!g) return nullptr;
    DayRow row;
    DayCell *cell = nullptr;
    const QString cap = utf8(caption);
    switch (kind) {
    case KIND_SLOT: {
        row.label = new QLabel(utf8(label));
        cell = new DayCell(key, false);
        row.field = cell;
        g->form->addRow(row.label, cell);
        break;
    }
    case KIND_BUTTON: {
        QPushButton *b = destructive ? new DayDangerButton(utf8(text)) : new QPushButton(utf8(text));
        if (arrow && QApplication::style()) {
            b->setIcon(QApplication::style()->standardIcon(QStyle::SP_ArrowForward));
            b->setLayoutDirection(Qt::RightToLeft); // the arrow after the title
        }
        b->setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Fixed);
        QObject::connect(b, &QPushButton::clicked, shell, [shell, key] {
            if (shell->on_row) shell->on_row(shell->node, key);
        });
        row.button = b;
        if (with_cell) {
            auto *field = new QWidget();
            auto *h = new QHBoxLayout(field);
            h->setContentsMargins(0, 0, 0, 0);
            h->setSpacing(kSpacing / 2);
            h->addWidget(b);
            cell = new DayCell(key, false);
            h->addWidget(cell);
            h->addStretch(1);
            row.field = field;
        } else {
            row.field = b;
        }
        g->form->addRow(static_cast<QWidget *>(nullptr), row.field);
        break;
    }
    case KIND_LINK: {
        auto *l = new QLabel();
        l->setText(QStringLiteral("<a href=\"day:row\">%1</a>").arg(utf8(text).toHtmlEscaped()));
        l->setTextFormat(Qt::RichText);
        l->setTextInteractionFlags(Qt::LinksAccessibleByMouse | Qt::LinksAccessibleByKeyboard);
        l->setOpenExternalLinks(false);
        QObject::connect(l, &QLabel::linkActivated, shell, [shell, key](const QString &) {
            if (shell->on_row) shell->on_row(shell->node, key);
        });
        row.link = l;
        row.field = l;
        g->form->addRow(static_cast<QWidget *>(nullptr), l);
        break;
    }
    case KIND_NOTE: {
        row.note = dim_label(utf8(text));
        g->form->addRow(row.note);
        break;
    }
    case KIND_FILL: {
        cell = new DayCell(key, true);
        cell->node = shell->node;
        cell->on_fill = shell->on_fill;
        row.field = cell;
        g->form->addRow(cell);
        break;
    }
    default:
        break;
    }
    row.cell = cell;
    if (!cap.isEmpty() && kind != KIND_NOTE) {
        row.caption = dim_label(cap);
        g->form->addRow(static_cast<QWidget *>(nullptr), row.caption);
    }
    if (!enabled) {
        if (row.label) row.label->setEnabled(false);
        if (row.field) row.field->setEnabled(false);
        if (row.caption) row.caption->setEnabled(false);
    }
    shell->rows.insert(key, row);
    return cell;
}

// After the last row: the footers go under their groups, the list takes the width its
// entries need, and the first page shows.
void day_settings_qt_shell_done(void *host, int selected) {
    auto *shell = static_cast<DayShell *>(host);
    for (DayGroup *g : shell->groups) {
        if (!g->footer->text().isEmpty()) g->form->addRow(g->footer);
        else g->footer->hide();
    }
    // A page that is not the stack's current one is hidden, and nothing under it is ever
    // shown; a layout counts a never-shown widget as empty, so such a page would measure as
    // its labels alone. Shown once here, each widget stays "not hidden" under its hidden page,
    // and every page measures its full content: what the window fits at open.
    for (QWidget *page : shell->pages) {
        page->show();
        const auto children = page->findChildren<QWidget *>();
        for (QWidget *w : children) w->show();
    }
    if (shell->list) {
        int w = shell->list->sizeHintForColumn(0) + 2 * shell->list->frameWidth() + 8;
        shell->list->setFixedWidth(std::max(140, std::min(220, w)));
    }
    shell->select(selected);
}

// Move the Day control (mounted under the host by Day) into its cell.
void day_settings_qt_slot_mount(void *cell_ptr, void *slot) {
    auto *cell = static_cast<DayCell *>(cell_ptr);
    auto *w = static_cast<QWidget *>(slot);
    w->setParent(cell);
    w->move(0, 0);
    w->show();
}

// The size Day measured for the control in `cell`: a control cell takes it exactly; a spanning
// cell takes the height and keeps the width the form grants.
void day_settings_qt_cell_set_size(void *cell_ptr, double w, double h) {
    auto *cell = static_cast<DayCell *>(cell_ptr);
    const int iw = int(std::lround(w));
    const int ih = int(std::lround(h));
    if (cell->fill) {
        if (cell->minimumHeight() != ih || cell->maximumHeight() != ih) cell->setFixedHeight(ih);
    } else if (cell->size() != QSize(iw, ih) || cell->minimumSize() != cell->maximumSize()) {
        cell->setFixedSize(iw, ih);
    }
}

// The shell's natural size at `for_w` of width (0 = unconstrained): the list beside the widest
// page, and the tallest page's content under the header, so a window that fits it shows every
// page without scrolling.
void day_settings_qt_shell_measure(void *host, double for_w, double *out_w, double *out_h) {
    auto *shell = static_cast<DayShell *>(host);
    if (shell->layout()) shell->layout()->activate();
    int list_w = 0, list_h = 0;
    if (shell->list) {
        list_w = shell->list->minimumWidth() + kSpacing;
        list_h = shell->list->sizeHintForRow(0) * shell->list->count() + 2 * shell->list->frameWidth() + 8;
    }
    int page_w = 0, page_h = 0;
    const int avail = for_w > 0 ? shell->page_width(for_w) : -1;
    for (QWidget *page : shell->pages) {
        if (page->layout()) invalidate_tree(page->layout());
        const QSize hint = page->sizeHint();
        page_w = std::max(page_w, hint.width());
        int h = hint.height();
        if (avail > 0 && page->layout() && page->layout()->hasHeightForWidth())
            h = page->layout()->totalHeightForWidth(avail);
        page_h = std::max(page_h, h);
    }
    const int column_h = shell->header_height() + page_h;
    *out_w = 2 * kMargin + list_w + page_w;
    *out_h = 2 * kMargin + std::max(column_h, list_h);
}

void day_settings_qt_shell_select(void *host, int page) {
    static_cast<DayShell *>(host)->select(page);
}

void day_settings_qt_shell_set_page_title(void *host, int page, const char *title) {
    auto *shell = static_cast<DayShell *>(host);
    if (shell->list) {
        if (auto *item = shell->list->item(page)) item->setText(utf8(title));
        if (shell->header && shell->stack->currentIndex() == page) shell->header->setText(utf8(title));
    }
}

void day_settings_qt_group_set_title(void *group, const char *title) {
    auto *g = static_cast<DayGroup *>(group);
    if (g && g->titled) g->titled->setTitle(utf8(title));
}

void day_settings_qt_group_set_footer(void *group, const char *footer) {
    auto *g = static_cast<DayGroup *>(group);
    if (g && g->footer) g->footer->setText(utf8(footer));
}

// `which`: 0 the label column, 1 the field's own text (a button, a link), 2 the caption, 3 a
// note.
void day_settings_qt_row_set_text(void *host, uint64_t key, int which, const char *text) {
    auto *shell = static_cast<DayShell *>(host);
    auto it = shell->rows.find(key);
    if (it == shell->rows.end()) return;
    const QString t = utf8(text);
    switch (which) {
    case 0:
        if (it->label) it->label->setText(t);
        break;
    case 1:
        if (it->button) it->button->setText(t);
        if (it->link) it->link->setText(QStringLiteral("<a href=\"day:row\">%1</a>").arg(t.toHtmlEscaped()));
        break;
    case 2:
        if (it->caption) it->caption->setText(t);
        break;
    case 3:
        if (it->note) it->note->setText(t);
        break;
    default:
        break;
    }
}

void day_settings_qt_row_set_enabled(void *host, uint64_t key, int on) {
    auto *shell = static_cast<DayShell *>(host);
    auto it = shell->rows.find(key);
    if (it == shell->rows.end()) return;
    if (it->label) it->label->setEnabled(on != 0);
    if (it->field) it->field->setEnabled(on != 0);
    if (it->caption) it->caption->setEnabled(on != 0);
}

} // extern "C"
