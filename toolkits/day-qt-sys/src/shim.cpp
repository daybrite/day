// A flat C ABI over Qt 6 Widgets for day-qt (hop's CQt / pane's shim, extended for day):
// node-id-carrying callbacks, height-for-width labels, scroll areas, snapshots, main-thread
// posting. Only connects to existing Qt signals via lambdas — no moc required.

#include <QApplication>
#include <QBuffer>
#include <QByteArray>
#include <QCheckBox>
#include <QFileDialog>
#include <QFont>
#include <QFrame>
#include <QLabel>
#include <QLineEdit>
#include <QMessageBox>
#include <QMouseEvent>
#include <QInputDialog>
#include <QStringList>
#include <QProgressBar>
#include <QPushButton>
#include <QTabWidget>
#include <QMetaObject>
#include <cstdint>
#include <map>
#include <vector>
#include <QPixmap>
#include <QPushButton>
#include <QScrollArea>
#include <QListWidget>
#include <QResizeEvent>
#include <QSplitter>
#include <QSlider>
#include <QString>
#include <QWidget>
#include <QMenu>
#include <QMenuBar>
#include <QAction>
#include <QKeySequence>

#include <cstdint>

extern "C" {

static int s_argc = 1;
// argv[0] doubles as the macOS app-menu name (Qt captures it at QApplication construction, so it
// must be set BEFORE `day_qt_app_new`). day-qt fills it with the app's display name.
static char s_arg0[256] = "day";
static char *s_argv[] = {s_arg0, nullptr};

// Lifecycle (docs/lifecycle.md): codes match day_spec::Lifecycle order (2=DidBecomeActive,
// 3=WillResignActive, 7=WillTerminate). Set from Rust before exec.
static void (*g_lifecycle_cb)(int) = nullptr;

void *day_qt_app_new(const char *app_name) {
    if (app_name && *app_name) {
        strncpy(s_arg0, app_name, sizeof(s_arg0) - 1);
        s_arg0[sizeof(s_arg0) - 1] = '\0';
    }
    auto *app = new QApplication(s_argc, s_argv);
    QCoreApplication::setApplicationName(QString::fromUtf8(s_arg0));
    QObject::connect(app, &QApplication::applicationStateChanged, [](Qt::ApplicationState s) {
        if (!g_lifecycle_cb) return;
        if (s == Qt::ApplicationActive) g_lifecycle_cb(2);        // DidBecomeActive
        else if (s == Qt::ApplicationInactive) g_lifecycle_cb(3); // WillResignActive
    });
    QObject::connect(app, &QCoreApplication::aboutToQuit, []() {
        if (g_lifecycle_cb) g_lifecycle_cb(7);                    // WillTerminate
    });
    return app;
}
void day_qt_app_run(void *app) { static_cast<QApplication *>(app)->exec(); }

// Resizable top-level that reports size changes back to day (docs §7.7).
class DayWindow : public QWidget {
public:
    void (*resize_cb)(int, int) = nullptr;

protected:
    void resizeEvent(QResizeEvent *e) override {
        QWidget::resizeEvent(e);
        if (resize_cb) resize_cb(width(), height());
    }
};

void *day_qt_window_new(const char *title, int w, int h) {
    auto *win = new DayWindow();
    win->setWindowTitle(QString::fromUtf8(title));
    win->resize(w, h);
    return win;
}
void day_qt_window_on_resize(void *win, void (*cb)(int, int)) {
    static_cast<DayWindow *>(win)->resize_cb = cb;
}
void day_qt_window_show(void *win) { static_cast<QWidget *>(win)->show(); }

void *day_qt_container_new() { return new QWidget(); }

// --- label ---
void *day_qt_label_new(const char *text) {
    QLabel *l = new QLabel(QString::fromUtf8(text));
    l->setWordWrap(true);
    l->setAlignment(Qt::AlignLeft | Qt::AlignTop);
    return l;
}
void day_qt_label_set_text(void *w, const char *text) {
    static_cast<QLabel *>(w)->setText(QString::fromUtf8(text));
}
void day_qt_label_set_font(void *w, double pt, int weight, int italic) {
    QLabel *l = static_cast<QLabel *>(w);
    QFont f = l->font();
    f.setPointSizeF(pt);
    // `weight` is a QFont::Weight numeric value (Thin=100 … Black=900).
    if (weight > 0)
        f.setWeight(static_cast<QFont::Weight>(weight));
    f.setItalic(italic != 0);
    l->setFont(f);
}
int day_qt_label_height_for_width(void *w, int width) {
    return static_cast<QLabel *>(w)->heightForWidth(width);
}

// --- button ---
void *day_qt_button_new(const char *title, uint64_t id, void (*cb)(uint64_t)) {
    QPushButton *b = new QPushButton(QString::fromUtf8(title));
    QObject::connect(b, &QPushButton::clicked, [id, cb]() { cb(id); });
    return b;
}
void day_qt_button_set_title(void *w, const char *title) {
    static_cast<QPushButton *>(w)->setText(QString::fromUtf8(title));
}

// --- toggle (checkbox: Qt Widgets has no native switch) ---
void *day_qt_checkbox_new(int on, uint64_t id, void (*cb)(uint64_t, int)) {
    QCheckBox *c = new QCheckBox();
    c->setChecked(on != 0);
    QObject::connect(c, &QCheckBox::toggled, [id, cb](bool v) { cb(id, v ? 1 : 0); });
    return c;
}
void day_qt_checkbox_set(void *w, int on) {
    QCheckBox *c = static_cast<QCheckBox *>(w);
    if (c->isChecked() != (on != 0)) c->setChecked(on != 0);
}

// --- slider (int 0..=1000; day-qt maps to f64 range) ---
void *day_qt_slider_new(int value, uint64_t id, void (*cb)(uint64_t, int)) {
    QSlider *s = new QSlider(Qt::Horizontal);
    s->setMinimum(0);
    s->setMaximum(1000);
    s->setValue(value);
    QObject::connect(s, &QSlider::valueChanged, [id, cb](int v) { cb(id, v); });
    return s;
}
void day_qt_slider_set(void *w, int value) {
    QSlider *s = static_cast<QSlider *>(w);
    if (s->value() != value) s->setValue(value);
}

// --- line edit ---
void *day_qt_lineedit_new(const char *text, const char *ph, uint64_t id,
                          void (*cb)(uint64_t, const char *)) {
    QLineEdit *e = new QLineEdit(QString::fromUtf8(text));
    e->setPlaceholderText(QString::fromUtf8(ph));
    QObject::connect(e, &QLineEdit::textChanged, [id, cb](const QString &s) {
        QByteArray ba = s.toUtf8();
        cb(id, ba.constData());
    });
    return e;
}
void day_qt_lineedit_set_text(void *w, const char *text) {
    QLineEdit *e = static_cast<QLineEdit *>(w);
    QString s = QString::fromUtf8(text);
    if (e->text() != s) e->setText(s);
}
void day_qt_lineedit_set_placeholder(void *w, const char *text) {
    static_cast<QLineEdit *>(w)->setPlaceholderText(QString::fromUtf8(text));
}

// --- divider ---
void *day_qt_separator_new() {
    QFrame *f = new QFrame();
    f->setFrameShape(QFrame::HLine);
    f->setFrameShadow(QFrame::Sunken);
    return f;
}

// --- progress (QProgressBar: determinate 0..1000, or busy/indeterminate range 0..0) ---
// Qt has no native spinner widget; the idiomatic indeterminate indicator is a busy
// progress bar (min==max==0), so both variants use QProgressBar (docs/progress.md).
void *day_qt_progress_new(int determinate, int value) {
    QProgressBar *b = new QProgressBar();
    b->setTextVisible(false);
    if (determinate) {
        b->setRange(0, 1000);
        b->setValue(value);
    } else {
        b->setRange(0, 0); // busy animation
    }
    return b;
}
void day_qt_progress_set(void *w, int value) {
    QProgressBar *b = static_cast<QProgressBar *>(w);
    if (b->value() != value) b->setValue(value);
}

// --- scroll ---
void *day_qt_scroll_new() {
    QScrollArea *sa = new QScrollArea();
    sa->setWidgetResizable(false);
    sa->setFrameShape(QFrame::NoFrame);
    sa->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    QWidget *content = new QWidget();
    sa->setWidget(content);
    return sa;
}
void *day_qt_scroll_content(void *w) {
    QScrollArea *sa = qobject_cast<QScrollArea *>(static_cast<QWidget *>(w));
    return sa ? sa->widget() : nullptr;
}
void day_qt_scroll_set_content_size(void *w, int cw, int ch) {
    QScrollArea *sa = qobject_cast<QScrollArea *>(static_cast<QWidget *>(w));
    if (sa && sa->widget()) sa->widget()->resize(cw, ch);
}

// --- tree / geometry ---
void day_qt_add_child(void *parent, void *child) {
    QWidget *c = static_cast<QWidget *>(child);
    c->setParent(static_cast<QWidget *>(parent));
    c->show();
}
void day_qt_remove_child(void *child) {
    QWidget *c = static_cast<QWidget *>(child);
    c->setParent(nullptr);
    c->hide();
}
void day_qt_delete(void *w) { static_cast<QWidget *>(w)->deleteLater(); }
void day_qt_set_geometry(void *w, int x, int y, int width, int height) {
    static_cast<QWidget *>(w)->setGeometry(x, y, width, height);
}
void day_qt_size_hint(void *w, double *out_w, double *out_h) {
    QSize s = static_cast<QWidget *>(w)->sizeHint();
    *out_w = s.width();
    *out_h = s.height();
}
void day_qt_set_enabled(void *w, int enabled) {
    static_cast<QWidget *>(w)->setEnabled(enabled != 0);
}
void day_qt_set_object_name(void *w, const char *name) {
    static_cast<QWidget *>(w)->setObjectName(QString::fromUtf8(name));
}
void day_qt_set_tooltip(void *w, const char *text) {
    static_cast<QWidget *>(w)->setToolTip(QString::fromUtf8(text));
}
// Accessibility (§13): QWidget accessibleName/Description surface via QAccessible (UIA on Windows,
// AT-SPI on Linux, NSAccessibility on macOS). Role/value derive from the widget type.
void day_qt_set_accessible_name(void *w, const char *name) {
    static_cast<QWidget *>(w)->setAccessibleName(QString::fromUtf8(name));
}
void day_qt_set_accessible_description(void *w, const char *text) {
    static_cast<QWidget *>(w)->setAccessibleDescription(QString::fromUtf8(text));
}

// --- misc ---
// --- navigation (docs/navigation.md): QSplitter host with two plain-widget panes ---
void *day_qt_splitter_new() {
    auto *s = new QSplitter(Qt::Horizontal);
    s->setChildrenCollapsible(false);
    s->addWidget(new QWidget());
    s->addWidget(new QWidget());
    s->setStretchFactor(0, 0);
    s->setStretchFactor(1, 1);
    s->setSizes({240, 480});
    return s;
}
void *day_qt_splitter_pane(void *w, int index) {
    auto *s = qobject_cast<QSplitter *>(static_cast<QWidget *>(w));
    return s ? static_cast<void *>(s->widget(index)) : nullptr;
}
void day_qt_splitter_on_moved(void *w, void (*cb)(void *)) {
    auto *s = qobject_cast<QSplitter *>(static_cast<QWidget *>(w));
    if (s) {
        QObject::connect(s, &QSplitter::splitterMoved, [s, cb](int, int) { cb(s); });
    }
}
void day_qt_widget_size(void *w, double *out_w, double *out_h) {
    QWidget *q = static_cast<QWidget *>(w);
    *out_w = q->width();
    *out_h = q->height();
}
void day_qt_set_visible(void *w, int visible) {
    static_cast<QWidget *>(w)->setVisible(visible != 0);
}

// --- navigation menu (docs/navigation.md): QListWidget with a sidebar treatment ---
void *day_qt_navlist_new(uint64_t id, void (*cb)(uint64_t, int)) {
    auto *w = new QListWidget();
    w->setFrameShape(QFrame::NoFrame);
    w->setStyleSheet(
        "QListWidget{background:transparent;outline:0;}"
        "QListWidget::item{padding:6px 10px;border-radius:6px;margin:1px 4px;}"
        "QListWidget::item:selected{background:palette(highlight);"
        "color:palette(highlighted-text);}");
    QObject::connect(w, &QListWidget::currentRowChanged,
                     [id, cb](int row) { cb(id, row); });
    return w;
}
void day_qt_navlist_set_items(void *w, const char *joined) {
    auto *l = qobject_cast<QListWidget *>(static_cast<QWidget *>(w));
    if (!l) return;
    l->blockSignals(true);
    l->clear();
    for (const QString &item :
         QString::fromUtf8(joined).split(QChar(0x1f), Qt::SkipEmptyParts)) {
        l->addItem(item);
    }
    l->blockSignals(false);
}
void day_qt_navlist_set_selected(void *w, int idx) {
    auto *l = qobject_cast<QListWidget *>(static_cast<QWidget *>(w));
    if (!l) return;
    l->blockSignals(true);
    l->setCurrentRow(idx);
    l->blockSignals(false);
}

// --- tabs (docs/tabs.md): QTabWidget owns its page widgets ---
void *day_qt_tabs_new(uint64_t id, void (*cb)(uint64_t, int)) {
    auto *t = new QTabWidget();
    QObject::connect(t, &QTabWidget::currentChanged,
                     [id, cb](int index) { cb(id, index); });
    return t;
}
void day_qt_tabs_add_page(void *tabs, void *page, const char *title, int index) {
    auto *t = static_cast<QTabWidget *>(tabs);
    // Block signals during setup so insertion / initial selection do not echo back.
    bool b = t->blockSignals(true);
    t->insertTab(index, static_cast<QWidget *>(page), QString::fromUtf8(title));
    t->blockSignals(b);
}
void day_qt_tabs_set_current(void *tabs, int index) {
    auto *t = static_cast<QTabWidget *>(tabs);
    bool b = t->blockSignals(true);
    t->setCurrentIndex(index);
    t->blockSignals(b);
}
void day_qt_tabs_content_size(void *tabs, double *w, double *h) {
    auto *t = static_cast<QTabWidget *>(tabs);
    if (QWidget *cur = t->currentWidget()) {
        *w = cur->width();
        *h = cur->height();
    } else {
        *w = 0;
        *h = 0;
    }
}

void day_qt_post(void (*cb)(void *), void *data) {
    QMetaObject::invokeMethod(
        qApp, [cb, data]() { cb(data); }, Qt::QueuedConnection);
}
int day_qt_snapshot_png(void *widget, const char *path) {
    QPixmap pm = static_cast<QWidget *>(widget)->grab();
    return pm.save(QString::fromUtf8(path), "PNG") ? 0 : 1;
}

// --- imperative presentation (docs/dialogs.md) ---
struct DayPresent { QDialog *dialog; std::vector<QAbstractButton *> buttons; };
static std::map<uint64_t, DayPresent> g_presents;
static void (*g_present_cb)(uint64_t, int, long long, const char *) = nullptr;

void day_qt_set_present_cb(void (*cb)(uint64_t, int, long long, const char *)) {
    g_present_cb = cb;
}

void day_qt_present_dialog(uint64_t req, const char *title, const char *message,
                           const char *buttons_joined, const char *roles_joined, void *parent) {
    auto *box = new QMessageBox(static_cast<QWidget *>(parent));
    box->setWindowTitle(QString::fromUtf8(title));
    box->setText(QString::fromUtf8(title));
    if (message && *message) box->setInformativeText(QString::fromUtf8(message));
    QStringList labels =
        QString::fromUtf8(buttons_joined).split(QChar(0x1f), Qt::SkipEmptyParts);
    QStringList roles = QString::fromUtf8(roles_joined).split(QChar(','), Qt::SkipEmptyParts);
    std::vector<QAbstractButton *> btns;
    for (int i = 0; i < labels.size(); i++) {
        int role = (i < roles.size()) ? roles[i].toInt() : 0;
        QMessageBox::ButtonRole r = QMessageBox::AcceptRole;
        if (role == 1) r = QMessageBox::RejectRole;
        else if (role == 2) r = QMessageBox::DestructiveRole;
        btns.push_back(box->addButton(labels[i], r));
    }
    g_presents[req] = {box, btns};
    QObject::connect(box, &QMessageBox::finished, [req, box](int) {
        auto it = g_presents.find(req);
        if (it == g_presents.end()) return;
        QAbstractButton *clicked = box->clickedButton();
        int idx = -1;
        for (size_t i = 0; i < it->second.buttons.size(); i++)
            if (it->second.buttons[i] == clicked) idx = (int)i;
        g_presents.erase(it);
        if (g_present_cb) {
            if (idx >= 0) g_present_cb(req, 1, idx, "");
            else g_present_cb(req, 0, 0, "");
        }
        box->deleteLater();
    });
    box->open();
}

void day_qt_present_prompt(uint64_t req, const char *title, const char *message,
                           const char *placeholder, const char *initial, const char *ok,
                           const char *cancel, void *parent) {
    auto *dlg = new QInputDialog(static_cast<QWidget *>(parent));
    dlg->setWindowTitle(QString::fromUtf8(title));
    dlg->setLabelText(QString::fromUtf8((message && *message) ? message : title));
    dlg->setTextValue(QString::fromUtf8(initial));
    dlg->setOkButtonText(QString::fromUtf8(ok));
    dlg->setCancelButtonText(QString::fromUtf8(cancel));
    dlg->setInputMode(QInputDialog::TextInput);
    (void)placeholder; // QInputDialog does not expose the line edit's placeholder portably
    g_presents[req] = {dlg, {}};
    QObject::connect(dlg, &QInputDialog::finished, [req, dlg](int result) {
        g_presents.erase(req);
        if (g_present_cb) {
            if (result == QDialog::Accepted) {
                QByteArray utf8 = dlg->textValue().toUtf8();
                g_present_cb(req, 2, 0, utf8.constData());
            } else {
                g_present_cb(req, 0, 0, "");
            }
        }
        dlg->deleteLater();
    });
    dlg->open();
}

// Convert Day's flattened filter string ("Name|ext1,ext2" joined by 0x1f) into a Qt name filter
// ("Name (*.ext1 *.ext2);;…"). Empty input → no filter.
static QString day_qt_name_filters(const char *filters_joined) {
    QString all = QString::fromUtf8(filters_joined);
    if (all.isEmpty()) return QString();
    QStringList out;
    for (const QString &f : all.split(QChar(0x1f), Qt::SkipEmptyParts)) {
        int bar = f.indexOf('|');
        QString name = bar >= 0 ? f.left(bar) : f;
        QString exts = bar >= 0 ? f.mid(bar + 1) : QString();
        QStringList globs;
        for (const QString &e : exts.split(',', Qt::SkipEmptyParts)) globs << ("*." + e);
        if (globs.isEmpty()) globs << "*";
        out << (name + " (" + globs.join(' ') + ")");
    }
    return out.join(";;");
}

// Report a file dialog result: tag 3 (files) with the chosen path, or tag 0 (dismissed).
static void day_qt_finish_file(uint64_t req, QFileDialog *dlg, int result) {
    g_presents.erase(req);
    if (g_present_cb) {
        QStringList sel = dlg->selectedFiles();
        if (result == QDialog::Accepted && !sel.isEmpty()) {
            QByteArray path = sel.first().toUtf8();
            g_present_cb(req, 3, 0, path.constData());
        } else {
            g_present_cb(req, 0, 0, "");
        }
    }
    dlg->deleteLater();
}

void day_qt_present_file_open(uint64_t req, const char *title, const char *filters_joined,
                              void *parent) {
    auto *dlg = new QFileDialog(static_cast<QWidget *>(parent), QString::fromUtf8(title));
    dlg->setFileMode(QFileDialog::ExistingFile);
    dlg->setAcceptMode(QFileDialog::AcceptOpen);
    QString nf = day_qt_name_filters(filters_joined);
    if (!nf.isEmpty()) dlg->setNameFilter(nf);
    g_presents[req] = {dlg, {}};
    QObject::connect(dlg, &QFileDialog::finished,
                     [req, dlg](int result) { day_qt_finish_file(req, dlg, result); });
    dlg->open();
}

void day_qt_present_file_save(uint64_t req, const char *title, const char *suggested,
                              const char *filters_joined, void *parent) {
    auto *dlg = new QFileDialog(static_cast<QWidget *>(parent), QString::fromUtf8(title));
    dlg->setFileMode(QFileDialog::AnyFile);
    dlg->setAcceptMode(QFileDialog::AcceptSave);
    if (suggested && *suggested) dlg->selectFile(QString::fromUtf8(suggested));
    QString nf = day_qt_name_filters(filters_joined);
    if (!nf.isEmpty()) dlg->setNameFilter(nf);
    g_presents[req] = {dlg, {}};
    QObject::connect(dlg, &QFileDialog::finished,
                     [req, dlg](int result) { day_qt_finish_file(req, dlg, result); });
    dlg->open();
}

void day_qt_dismiss_present(uint64_t req) {
    auto it = g_presents.find(req);
    if (it != g_presents.end()) it->second.dialog->reject();
}

} // extern "C"

// --- canvas + image (day M8) ---
#include <QPaintEvent>
#include <QPainter>
#include <QPolygonF>
#include <QVector>

extern "C" {

class DayCanvasWidget : public QWidget {
public:
    QVector<double> nums;
    QStringList texts;
    using QWidget::QWidget;

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        int ti = 0;
        for (int i = 0; i + 8 < nums.size(); i += 9) {
            int k = (int)nums[i];
            double a = nums[i+1], b = nums[i+2], c = nums[i+3], d = nums[i+4];
            double e = nums[i+5], f = nums[i+6], g = nums[i+7];
            unsigned col = (unsigned)nums[i+8];
            QColor color((col >> 16) & 0xff, (col >> 8) & 0xff, col & 0xff, (col >> 24) & 0xff);
            QPen pen(color); pen.setWidthF(g); pen.setCapStyle(Qt::RoundCap);
            switch (k) {
                case 0: p.fillRect(QRectF(a, b, c, d), color); break;
                case 1: p.setPen(pen); p.setBrush(Qt::NoBrush); p.drawRect(QRectF(a, b, c, d)); break;
                case 2: p.setPen(Qt::NoPen); p.setBrush(color); p.drawRoundedRect(QRectF(a, b, c, d), e, e); break;
                case 13: p.setPen(pen); p.setBrush(Qt::NoBrush); p.drawRoundedRect(QRectF(a, b, c, d), e, e); break;
                case 3: p.setPen(Qt::NoPen); p.setBrush(color); p.drawEllipse(QRectF(a, b, c, d)); break;
                case 4: p.setPen(pen); p.setBrush(Qt::NoBrush); p.drawEllipse(QRectF(a, b, c, d)); break;
                case 5: // arc: spec is clockwise-degrees; Qt is CCW 1/16°
                    p.setPen(pen); p.setBrush(Qt::NoBrush);
                    p.drawArc(QRectF(a, b, c, d), (int)(-e * 16.0), (int)(-f * 16.0));
                    break;
                case 6: p.setPen(pen); p.drawLine(QPointF(a, b), QPointF(c, d)); break;
                case 7: {
                    QString t = ti < texts.size() ? texts[ti++] : QString();
                    QFont font = p.font(); font.setPointSizeF(e); p.setFont(font);
                    p.setPen(QPen(color));
                    QPointF pos(a, b);
                    if (f > 0.5) {
                        QFontMetricsF fm(font);
                        pos.setX(a - fm.horizontalAdvance(t) / 2.0);
                        pos.setY(b + fm.ascent() / 2.0 - fm.descent() / 2.0);
                    }
                    p.drawText(pos, t);
                    break;
                }
                case 8: p.save(); break;
                case 9: p.restore(); break;
                case 10:
                    // Packed affine (a,b,c,d,tx,ty); QTransform(m11,m12,m21,m22,dx,dy) has the same
                    // row-vector meaning. combine=true concatenates onto the current world transform.
                    p.setWorldTransform(QTransform(a, b, c, d, e, f), true);
                    break;
                case 11: case 12: { // polygon (11 fill / 12 stroke); points in texts as "x,y x,y …"
                    QString t = ti < texts.size() ? texts[ti++] : QString();
                    QPolygonF poly;
                    for (const QString &pair : t.split(' ', Qt::SkipEmptyParts)) {
                        int comma = pair.indexOf(',');
                        if (comma > 0)
                            poly << QPointF(pair.left(comma).toDouble(), pair.mid(comma + 1).toDouble());
                    }
                    if (poly.size() >= 2) {
                        if (k == 11) { p.setPen(Qt::NoPen); p.setBrush(color); p.drawPolygon(poly); }
                        else { p.setPen(pen); p.setBrush(Qt::NoBrush); p.drawPolygon(poly); }
                    }
                    break;
                }
            }
        }
    }
};

void *day_qt_canvas_new() { return new DayCanvasWidget(); }
void day_qt_canvas_set_ops(void *w, const double *nums, int n, const char *texts_joined) {
    DayCanvasWidget *c = static_cast<DayCanvasWidget *>(w);
    c->nums.clear();
    for (int i = 0; i < n; i++) c->nums.append(nums[i]);
    // 0x1f unit separator; keep empties — one entry per kind-7/11/12 record.
    c->texts = QString::fromUtf8(texts_joined).split(QChar(0x1f));
    c->update();
}

void *day_qt_image_new(const char *path) {
    QLabel *l = new QLabel();
    QPixmap pm(QString::fromUtf8(path));
    if (!pm.isNull()) { l->setPixmap(pm); l->setScaledContents(true); }
    return l;
}

// --- gestures (tap / drag) ---
// phase: 0 = tap, 1 = drag began, 2 = drag changed, 3 = drag ended.
typedef void (*DayGestureCb)(uint64_t node, int phase, double x, double y, double tx, double ty);

class DayGestureFilter : public QObject {
public:
    uint64_t node; bool is_drag; DayGestureCb cb;
    bool pressed = false; QPointF start;
    DayGestureFilter(uint64_t n, bool d, DayGestureCb c) : node(n), is_drag(d), cb(c) {}
protected:
    bool eventFilter(QObject *obj, QEvent *ev) override {
        switch (ev->type()) {
            case QEvent::MouseButtonPress: {
                QMouseEvent *me = static_cast<QMouseEvent *>(ev);
                start = me->position();
                pressed = true;
                if (is_drag) cb(node, 1, start.x(), start.y(), 0.0, 0.0);
                break;
            }
            case QEvent::MouseMove: {
                if (is_drag && pressed) {
                    QPointF p = static_cast<QMouseEvent *>(ev)->position();
                    cb(node, 2, p.x(), p.y(), p.x() - start.x(), p.y() - start.y());
                }
                break;
            }
            case QEvent::MouseButtonRelease: {
                QMouseEvent *me = static_cast<QMouseEvent *>(ev);
                QPointF p = me->position();
                if (is_drag && pressed) {
                    cb(node, 3, p.x(), p.y(), p.x() - start.x(), p.y() - start.y());
                } else if (!is_drag && pressed) {
                    QWidget *w = qobject_cast<QWidget *>(obj);
                    if (!w || w->rect().contains(p.toPoint())) cb(node, 0, p.x(), p.y(), 0.0, 0.0);
                }
                pressed = false;
                break;
            }
            default: break;
        }
        return false; // never consume: let normal widget behavior proceed
    }
};

void day_qt_enable_gesture(void *w, uint64_t node, int is_drag, DayGestureCb cb) {
    QWidget *widget = static_cast<QWidget *>(w);
    DayGestureFilter *f = new DayGestureFilter(node, is_drag != 0, cb);
    f->setParent(widget); // freed with the widget
    widget->installEventFilter(f);
}

// ---- Menus (docs/menus.md) -------------------------------------------------
// A flat builder mirrored from the day-neutral MenuItem tree: Rust walks the tree and issues
// add_submenu / add_action / add_role / add_separator calls. Custom actions fire g_menu_cb(id);
// standard roles map to Qt's native affordances (QAction::menuRole on macOS moves About/Preferences/
// Quit into the app menu; clipboard/undo roles dispatch to the focused editing widget).

static void (*g_menu_cb)(uint64_t) = nullptr;

void day_qt_set_menu_cb(void (*cb)(uint64_t)) { g_menu_cb = cb; }

void day_qt_set_lifecycle_cb(void (*cb)(int)) { g_lifecycle_cb = cb; }

// Invoke a QLineEdit/QTextEdit public slot on whatever widget currently has focus.
static void day_qt_edit_dispatch(const char *slot) {
    if (QWidget *w = QApplication::focusWidget())
        QMetaObject::invokeMethod(w, slot, Qt::DirectConnection);
}

void *day_qt_window_menubar(void *win) {
    QWidget *window = static_cast<QWidget *>(win);
    QMenuBar *bar = window->findChild<QMenuBar *>(QString(), Qt::FindDirectChildrenOnly);
    if (!bar) {
        bar = new QMenuBar(window); // native global bar on macOS; top-of-window elsewhere
        bar->setNativeMenuBar(true);
    }
    bar->clear();
    return bar;
}

void *day_qt_menubar_add_menu(void *bar, const char *label) {
    return static_cast<QMenuBar *>(bar)->addMenu(QString::fromUtf8(label));
}

void *day_qt_menu_new() { return new QMenu(); }

void *day_qt_menu_add_submenu(void *menu, const char *label) {
    return static_cast<QMenu *>(menu)->addMenu(QString::fromUtf8(label));
}

void day_qt_menu_add_separator(void *menu) {
    static_cast<QMenu *>(menu)->addSeparator();
}

void day_qt_menu_add_action(void *menu, const char *label, uint64_t id,
                            const char *shortcut, int enabled) {
    QAction *a = static_cast<QMenu *>(menu)->addAction(QString::fromUtf8(label));
    if (shortcut && *shortcut) a->setShortcut(QKeySequence(QString::fromUtf8(shortcut)));
    a->setEnabled(enabled != 0);
    uint64_t aid = id;
    QObject::connect(a, &QAction::triggered, [aid]() {
        if (g_menu_cb) g_menu_cb(aid);
    });
}

// role codes match day_spec::MenuRole order.
void day_qt_menu_add_role(void *menu, const char *label, int role, const char *shortcut) {
    QAction *a = static_cast<QMenu *>(menu)->addAction(QString::fromUtf8(label));
    if (shortcut && *shortcut) a->setShortcut(QKeySequence(QString::fromUtf8(shortcut)));
    switch (role) {
        case 0: QObject::connect(a, &QAction::triggered, []() { day_qt_edit_dispatch("cut"); }); break;
        case 1: QObject::connect(a, &QAction::triggered, []() { day_qt_edit_dispatch("copy"); }); break;
        case 2: QObject::connect(a, &QAction::triggered, []() { day_qt_edit_dispatch("paste"); }); break;
        case 3: QObject::connect(a, &QAction::triggered, []() { day_qt_edit_dispatch("selectAll"); }); break;
        case 4: QObject::connect(a, &QAction::triggered, []() { day_qt_edit_dispatch("undo"); }); break;
        case 5: QObject::connect(a, &QAction::triggered, []() { day_qt_edit_dispatch("redo"); }); break;
        case 6: QObject::connect(a, &QAction::triggered, []() { day_qt_edit_dispatch("del"); }); break;
        case 7: a->setMenuRole(QAction::AboutRole); break;
        case 8:
            a->setMenuRole(QAction::QuitRole);
            QObject::connect(a, &QAction::triggered, []() { qApp->quit(); });
            break;
        case 9: a->setMenuRole(QAction::PreferencesRole); break;
        case 10:
            QObject::connect(a, &QAction::triggered, []() {
                if (QWidget *w = QApplication::focusWidget()) w->window()->showMinimized();
            });
            break;
        case 11:
            QObject::connect(a, &QAction::triggered, []() {
                if (QWidget *w = QApplication::focusWidget()) w->window()->close();
            });
            break;
        case 12:
            QObject::connect(a, &QAction::triggered, []() {
                if (QWidget *w = QApplication::focusWidget()) {
                    QWidget *top = w->window();
                    if (top->isFullScreen()) top->showNormal();
                    else top->showFullScreen();
                }
            });
            break;
        default: break;
    }
}

// Attach `menu` as `widget`'s context menu (secondary-click / long-press). A null menu clears it.
void day_qt_set_context_menu(void *w, void *menu) {
    QWidget *widget = static_cast<QWidget *>(w);
    // Drop any previously attached context menu + its connection (tracked by object name).
    if (QMenu *old = widget->findChild<QMenu *>(QStringLiteral("day_ctx_menu"),
                                                Qt::FindDirectChildrenOnly)) {
        old->setObjectName(QString()); // so it isn't re-found before deleteLater runs
        old->deleteLater();
    }
    QObject::disconnect(widget, &QWidget::customContextMenuRequested, nullptr, nullptr);
    if (!menu) {
        widget->setContextMenuPolicy(Qt::DefaultContextMenu);
        return;
    }
    QMenu *m = static_cast<QMenu *>(menu);
    m->setObjectName(QStringLiteral("day_ctx_menu"));
    m->setParent(widget); // freed with the widget
    widget->setContextMenuPolicy(Qt::CustomContextMenu);
    QObject::connect(widget, &QWidget::customContextMenuRequested,
                     [widget, m](const QPoint &pos) { m->popup(widget->mapToGlobal(pos)); });
}

} // extern "C"
