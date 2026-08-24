// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// A flat C ABI over Qt 6 Widgets for day-qt (hop's CQt / pane's shim, extended for day):
// node-id-carrying callbacks, height-for-width labels, scroll areas, snapshots, main-thread
// posting. Only connects to existing Qt signals via lambdas — no moc required.

#include <QApplication>
#include <QWindow>
#include <QStyleHints>
#include <QBuffer>
#include <QByteArray>
#include <QCheckBox>
#include <QFileDialog>
#include <QFont>
#include <QFontDatabase>
#include <QEasingCurve>
#include <QFrame>
#include <QGraphicsEffect>
#include <QButtonGroup>
#include <QHBoxLayout>
#include <QFileInfo>
#include <QPainter>
#include <QStyledItemDelegate>
#include <QPainterPath>
#include <QPropertyAnimation>
#include <QVariantAnimation>
#include <functional>
#include <QToolBar>
#include <QToolButton>
#include <QStyle>
#include <QStyleFactory>
#include <QTabBar>
#include <QWidgetAction>
#include <QVBoxLayout>
#include <QLabel>
#include <QLineEdit>
// Baseline derivation (day_qt_baseline): the text-bearing classes it asks about, plus the
// float-precision metrics it derives from.
#include <QAbstractButton>
#include <QAbstractSpinBox>
#include <QComboBox>
#include <QDateTimeEdit>
#include <QFontMetrics>
#include <QTextEdit>
#include <QMessageBox>
#include <QMouseEvent>
#include <QNativeGestureEvent>
#include <QWheelEvent>
#include <QDrag>
#include <QMimeData>
#include <QDragMoveEvent>
#include <QDropEvent>
#include <QInputDialog>
#include <QStringList>
#include <QProgressBar>
#include <QPushButton>
#include <QTabWidget>
#include <QMetaObject>
#include <cstdint>
#include <map>
#include <vector>
#include <QIcon>
#include <QPointer>
#include <QPainter>
#include <QPalette>
#include <QPixmap>
#include <QResource>
#include <QPushButton>
#include <QScrollArea>
#include <QScrollBar>
#include <QCompleter>
#include <QStringListModel> // the completer's model, reused rather than rebuilt (see set_suggestions)
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
#include <QDesktopServices>
#include <QUrl>

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

// Whether Qt is currently painting dark. Read from the live palette rather than
// QStyleHints::colorScheme so one answer covers all three sources of the scheme: the system,
// a DAY_THEME override, and the hand-built dark Fusion palette on pre-6.8 Qt (where
// colorScheme reports Unknown). Widgets are palette-driven, so the palette IS the truth.
int day_qt_dark_mode(void) {
    const QColor window = QApplication::palette().color(QPalette::Active, QPalette::Window);
    return window.lightness() < 128 ? 1 : 0;
}

void *day_qt_app_new(const char *app_name) {
    if (app_name && *app_name) {
        strncpy(s_arg0, app_name, sizeof(s_arg0) - 1);
        s_arg0[sizeof(s_arg0) - 1] = '\0';
    }
    auto *app = new QApplication(s_argc, s_argv);
    // Quit is DELIBERATE (the primary window's closeEvent / role Quit / ⌘Q): the default
    // quit-on-last-window-closed would misfire once secondary windows exist
    // (docs/windows.md — closing the last secondary must not exit, closing the primary
    // must exit even while secondaries are open).
    QApplication::setQuitOnLastWindowClosed(false);
    QCoreApplication::setApplicationName(QString::fromUtf8(s_arg0));
    // DAY_THEME=light|dark forces the color scheme (themed CI screenshot runs and local theme
    // checks); unset => follow the system. QStyleHints::setColorScheme needs Qt 6.8+; on older
    // Qt (e.g. Ubuntu 24.04's 6.4) fall back to a hand-built dark palette — Qt Widgets are
    // palette-driven, so every control follows it (light needs nothing: the default palette is
    // light). The literals below are unavoidable on that path: pre-6.8 Qt exposes NO symbolic
    // dark scheme (no setColorScheme, no named dark QPalette, and the platform theme is
    // whatever the desktop reports — nothing, under CI's offscreen/xvfb). They are the values
    // of the Qt-wiki canonical "dark Fusion" palette, and are dead code on Qt 6.8+.
    {
        const QByteArray theme = qgetenv("DAY_THEME");
#if QT_VERSION >= QT_VERSION_CHECK(6, 8, 0)
        if (theme == "dark") app->styleHints()->setColorScheme(Qt::ColorScheme::Dark);
        else if (theme == "light") app->styleHints()->setColorScheme(Qt::ColorScheme::Light);
#else
        if (theme == "dark") {
            QApplication::setStyle("Fusion"); // platform styles may ignore custom palettes
            QPalette p;
            const QColor window(0x2d, 0x2d, 0x2d);
            const QColor base(0x1e, 0x1e, 0x1e);
            const QColor text(0xf0, 0xf0, 0xf0);
            const QColor disabled(0x7f, 0x7f, 0x7f);
            const QColor accent(0x2a, 0x82, 0xda);
            p.setColor(QPalette::Window, window);
            p.setColor(QPalette::WindowText, text);
            p.setColor(QPalette::Base, base);
            p.setColor(QPalette::AlternateBase, window);
            p.setColor(QPalette::ToolTipBase, base);
            p.setColor(QPalette::ToolTipText, text);
            p.setColor(QPalette::Text, text);
            p.setColor(QPalette::Button, window);
            p.setColor(QPalette::ButtonText, text);
            p.setColor(QPalette::BrightText, Qt::red);
            p.setColor(QPalette::Link, accent);
            p.setColor(QPalette::Highlight, accent);
            p.setColor(QPalette::HighlightedText, Qt::white);
            p.setColor(QPalette::PlaceholderText, disabled);
            p.setColor(QPalette::Disabled, QPalette::Text, disabled);
            p.setColor(QPalette::Disabled, QPalette::WindowText, disabled);
            p.setColor(QPalette::Disabled, QPalette::ButtonText, disabled);
            p.setColor(QPalette::Disabled, QPalette::HighlightedText, disabled);
            app->setPalette(p);
        }
#endif
    }
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

// Per-window event callbacks for SECONDARY windows (docs/windows.md), keyed by the day
// node id each window carries. Registered once at run(); the primary keeps its dedicated
// `resize_cb` and quit-on-close semantics.
static void (*g_win_resized)(unsigned long long, int, int) = nullptr;
static void (*g_win_closed)(unsigned long long) = nullptr;
static void (*g_win_focused)(unsigned long long, int) = nullptr;
void day_qt_set_window_events_cb(void (*resized)(unsigned long long, int, int),
                                 void (*closed)(unsigned long long),
                                 void (*focused)(unsigned long long, int)) {
    g_win_resized = resized;
    g_win_closed = closed;
    g_win_focused = focused;
}

// Resizable top-level that reports size changes back to day (docs §7.7). Day's tree mounts
// into the inner `content` widget, NOT the window itself: on platforms where the QMenuBar is
// an in-window bar (Linux/Windows — macOS uses the global bar), the bar owns a strip at the
// top and `content` sits below it, so Day's absolute frames can never overlap the menus.
class DayWindow : public QWidget {
public:
    void (*resize_cb)(int, int) = nullptr;
    QWidget *content = nullptr;
    QMenuBar *menubar = nullptr;
    // Nonzero = a SECONDARY window's day root node id: events route to the g_win_*
    // callbacks and close hides (Rust owns destruction — docs/windows.md).
    unsigned long long node = 0;

    // The window toolbar (docs/toolbars.md), a strip under the menu bar. A plain QToolBar
    // parented to the window rather than a QMainWindow dock: the geometry here is already
    // hand-managed, and the toolbar is a genuine QToolBar either way — it gets the style's
    // icon size, button style and hover behavior. What it does not get is dragging between
    // dock areas, which needs QMainWindow.
    QToolBar *toolbar = nullptr;

    // The in-window menu bar height (0 when absent or when the platform uses a global bar).
    int menuHeight() const {
        if (!menubar || menubar->isNativeMenuBar()) return 0;
        return menubar->sizeHint().height();
    }
    int toolbarHeight() const {
        if (!toolbar || toolbar->isHidden() || toolbar->actions().isEmpty()) return 0;
        return toolbar->sizeHint().height();
    }
    void relayoutChrome() {
        const int mh = menuHeight();
        const int th = toolbarHeight();
        if (menubar && mh > 0) menubar->setGeometry(0, 0, width(), mh);
        if (toolbar && th > 0) toolbar->setGeometry(0, mh, width(), th);
        const int top = mh + th;
        if (content) content->setGeometry(0, top, width(), height() - top);
        if (resize_cb) resize_cb(width(), height() - top);
        if (node && g_win_resized) g_win_resized(node, width(), height() - top);
    }

protected:
    void resizeEvent(QResizeEvent *e) override {
        QWidget::resizeEvent(e);
        relayoutChrome();
    }
    void closeEvent(QCloseEvent *e) override {
        if (node) {
            // Confirm to day (teardown is deferred there); the window HIDES — no
            // WA_DeleteOnClose, Rust drives destruction when day releases the content, so
            // child-widget release order stays sound.
            if (g_win_closed) g_win_closed(node);
            e->accept();
        } else {
            // Primary close quits, taking secondary windows with it (docs/windows.md) —
            // explicit, because quitOnLastWindowClosed is off (a secondary outliving the
            // primary must not keep the app alive).
            QWidget::closeEvent(e);
            QCoreApplication::quit();
        }
    }
    void changeEvent(QEvent *e) override {
        QWidget::changeEvent(e);
        if (node && e->type() == QEvent::ActivationChange && g_win_focused)
            g_win_focused(node, isActiveWindow() ? 1 : 0);
    }
};

void *day_qt_window_new(const char *title, int w, int h) {
    auto *win = new DayWindow();
    win->setWindowTitle(QString::fromUtf8(title));
    win->resize(w, h);
    win->content = new QWidget(win);
    win->content->setGeometry(0, 0, w, h);
    win->content->show();
    return win;
}
void day_qt_window_on_resize(void *win, void (*cb)(int, int)) {
    static_cast<DayWindow *>(win)->resize_cb = cb;
}
void day_qt_window_show(void *win) { static_cast<QWidget *>(win)->show(); }

// A SECONDARY window (docs/windows.md): carries its day node id; `fixed` pins the size
// (the Preferences-window convention).
void *day_qt_window_new2(const char *title, int w, int h, unsigned long long node, int fixed) {
    auto *win = static_cast<DayWindow *>(day_qt_window_new(title, w, h));
    win->node = node;
    if (fixed) win->setFixedSize(w, h);
    return win;
}
void *day_qt_window_content(void *win) { return static_cast<DayWindow *>(win)->content; }
void day_qt_window_close(void *win) { static_cast<QWidget *>(win)->close(); }
void day_qt_window_raise(void *win) {
    auto *w = static_cast<QWidget *>(win);
    w->show();
    w->raise();
    w->activateWindow();
}
void day_qt_window_set_title(void *win, const char *title) {
    static_cast<QWidget *>(win)->setWindowTitle(QString::fromUtf8(title));
}
void day_qt_window_destroy(void *win) { static_cast<QWidget *>(win)->deleteLater(); }
int day_qt_window_is_active(void *win) {
    return static_cast<QWidget *>(win)->isActiveWindow() ? 1 : 0;
}

void *day_qt_container_new() { return new QWidget(); }

// RTL locales (docs/localization): Day's layout engine mirrors every frame itself, and
// Qt's app-wide setLayoutDirection would flip splitter/scroll coordinate systems UNDERNEATH
// those absolute frames (double mirroring, panes swapping out from under day). So RTL here
// only switches widget-INTERNAL text handling: label alignment + text direction.
static bool g_rtl = false;
void day_qt_app_set_rtl() { g_rtl = true; }

// Open a URL in the desktop's default handler (browser for http(s), mail client for mailto:, ...).
// QUrl::fromUserInput tolerates bare hosts ("daybrite.dev") as well as full URLs. Fire and forget.
void day_qt_open_url(const char *url) {
    QDesktopServices::openUrl(QUrl::fromUserInput(QString::fromUtf8(url)));
}

// SurfaceRole::SectionCard: the grouped-card background. A translucent neutral over the window
// color stays subtle in every palette (it lightens dark themes and darkens light ones) — Qt has
// no grouped-card palette role, and the concrete roles (alternate-base) vary wildly per style.
// The unique object-name selector scopes the rule to THIS widget — a bare `background-color` on
// a parent QWidget cascades into every descendant and replaces their native drawing (flat buttons).
// A QWidget has neither an opacity of its own nor a 2-D transform, so both ride a single custom
// QGraphicsEffect (§8.4). The effect grabs the widget (and its children) as a pixmap and re-draws
// it through a transformed painter — rotate/scale/translate about the center — with a padded
// bounding rect so the result can spill OUTSIDE the widget's own 112px frame (into the parent, like
// a drop shadow) instead of clipping to the widget rect. A rounded clip (matching the surface's
// corner radius) is re-applied here because the grabbed pixmap has square corners. No Q_OBJECT/moc:
// the overrides are plain virtuals and the tweens are QVariantAnimations driving a lambda.
class DayEffect : public QGraphicsEffect {
public:
    double opacity = 1.0;
    double tx = 0, ty = 0, sx = 1, sy = 1, rot = 0;
    double radius = 0.0;
    explicit DayEffect(QObject *parent) : QGraphicsEffect(parent) {}
    QRectF boundingRectFor(const QRectF &r) const override {
        return r.adjusted(-400, -400, 400, 400);
    }
    void draw(QPainter *painter) override {
        // Capture the widget (and children) in logical coords; `offset` is where to blit it. Qt sets
        // the pixmap's devicePixelRatio, so drawPixmap(offset, pm) blits at the right logical size.
        QPoint offset;
        QPixmap pm = sourcePixmap(Qt::LogicalCoordinates, &offset);
        if (pm.isNull()) {
            return;
        }
        // The pixmap is padded to boundingRectFor (so a transform can spill beyond the frame), so it
        // is NOT the widget's size. sourceBoundingRect() is the widget's real rect — use it for the
        // pivot and the rounded clip; the widget content sits centered within the padded pixmap.
        QRectF src = sourceBoundingRect();
        QPointF c = src.center();
        painter->save();
        painter->setRenderHint(QPainter::SmoothPixmapTransform, true);
        painter->setRenderHint(QPainter::Antialiasing, true);
        painter->setOpacity(painter->opacity() * opacity);
        painter->translate(c.x() + tx, c.y() + ty);
        painter->rotate(rot);
        painter->scale(sx, sy);
        painter->translate(-c.x(), -c.y());
        if (radius > 0.0) {
            // Re-round the corners: a QWidget doesn't clip children to its CSS border-radius, and
            // the grabbed pixmap is square, so clip to the surface radius over the real rect.
            QPainterPath rounded;
            rounded.addRoundedRect(src, radius, radius);
            painter->setClipPath(rounded);
        }
        painter->drawPixmap(offset, pm);
        painter->restore();
    }
};

static DayEffect *day_qt_effect(QWidget *w) {
    DayEffect *eff = dynamic_cast<DayEffect *>(w->graphicsEffect());
    if (!eff) {
        eff = new DayEffect(w);
        w->setGraphicsEffect(eff);
    }
    // Keep the rounded clip in sync with the widget's surface radius (set by set_surface).
    eff->radius = w->property("dayRadius").toDouble();
    return eff;
}

void day_qt_widget_set_section_card(void *w, double radius) {
    auto *widget = static_cast<QWidget *>(w);
    static unsigned long counter = 0;
    if (widget->objectName().isEmpty())
        widget->setObjectName(QString("daySectionCard%1").arg(counter++));
    widget->setAttribute(Qt::WA_StyledBackground, true);
    widget->setStyleSheet(QStringLiteral("#%1 { background-color: rgba(127,127,127,12%); border-radius: %2px; }")
                              .arg(widget->objectName())
                              .arg(radius));
}

void day_qt_widget_set_surface(void *w, double r, double g, double b, double a, double radius,
                               int clips) {
    QWidget *widget = static_cast<QWidget *>(w);
    // A unique object name scopes the stylesheet to THIS widget (`#name { ... }`) so the fill does
    // not bleed into child widgets the way a bare `background-color` on a parent QWidget would.
    static unsigned long counter = 0;
    if (widget->objectName().isEmpty())
        widget->setObjectName(QString("daySurface%1").arg(counter++));
    widget->setAttribute(Qt::WA_StyledBackground, true);
    // Remember the radius/clips so a later background-only update (day_qt_widget_set_bg — e.g. a
    // reactive `.background`) can rebuild the stylesheet without dropping the rounded corners.
    widget->setProperty("dayRadius", radius);
    widget->setProperty("dayClips", clips);
    QString body;
    if (a > 0.0) {
        body += QString("background-color: rgba(%1,%2,%3,%4);")
                    .arg(int(r * 255.0 + 0.5))
                    .arg(int(g * 255.0 + 0.5))
                    .arg(int(b * 255.0 + 0.5))
                    .arg(a, 0, 'f', 3);
    }
    if (radius > 0.0)
        body += QString("border-radius: %1px;").arg(radius);
    widget->setStyleSheet(QString("#%1 { %2 }").arg(widget->objectName()).arg(body));
    // A QWidget's CSS border-radius rounds only its OWN background, not its children — so a clipped
    // rounded surface (`clips`, e.g. `.corner_radius`) would still show square child fills at the
    // corners. Route such a surface's subtree through the rounded-clip DayEffect (the same effect
    // used for transform/opacity), giving antialiased corners that match the other toolkits. A
    // non-clipping rounded fill stays on the stylesheet alone (cheaper; keeps native child drawing).
    if (radius > 0.0 && clips) {
        day_qt_effect(widget); // creates if absent; reads dayRadius (set above) into eff->radius
        widget->graphicsEffect()->update();
    } else if (DayEffect *eff = dynamic_cast<DayEffect *>(widget->graphicsEffect())) {
        eff->radius = radius;
        eff->update();
    }
}

// Update only the background color, preserving the radius/clips captured by the last
// day_qt_widget_set_surface (so a reactive `.background` doesn't square off a rounded surface).
void day_qt_widget_set_bg(void *w, double r, double g, double b, double a) {
    QWidget *widget = static_cast<QWidget *>(w);
    double radius = widget->property("dayRadius").toDouble();
    int clips = widget->property("dayClips").toInt();
    day_qt_widget_set_surface(w, r, g, b, a, radius, clips);
}

// --- label ---
void *day_qt_label_new(const char *text) {
    QLabel *l = new QLabel(QString::fromUtf8(text));
    l->setWordWrap(true);
    if (g_rtl) {
        l->setAlignment(Qt::AlignRight | Qt::AlignTop);
        l->setLayoutDirection(Qt::RightToLeft);
    } else {
        l->setAlignment(Qt::AlignLeft | Qt::AlignTop);
    }
    return l;
}
/// Set a label's text as RICH text — day's markup serializer produced the HTML (docs/text-runs.md).
///
/// `Qt::RichText` explicitly, never `AutoText`: auto-detection guesses from the content, so a
/// PLAIN translated string that happens to contain `<` would silently start being parsed as
/// markup. Day decides which it is, not a heuristic.
// Link runs (docs/text-runs.md). QLabel hit-tests the `<a href>` in its rich text itself and
// emits linkActivated; external opening stays OFF so Day's own `.on_link()` decides. Rich text
// also needs the mouse-interaction flag, which a plain label does not carry.
void day_qt_label_on_link(void *w, uint64_t id, void (*cb)(uint64_t, const char *)) {
    QLabel *l = static_cast<QLabel *>(w);
    l->setOpenExternalLinks(false);
    l->setTextInteractionFlags(l->textInteractionFlags() | Qt::LinksAccessibleByMouse |
                               Qt::LinksAccessibleByKeyboard);
    QObject::connect(l, &QLabel::linkActivated, [id, cb](const QString &url) {
        QByteArray ba = url.toUtf8();
        cb(id, ba.constData());
    });
}
// A label's horizontal alignment (0 leading, 1 center, 2 trailing). Applies to plain and rich
// text alike — QLabel aligns its laid-out document either way — and keeps the RTL flip, where
// "leading" is the right edge.
void day_qt_label_set_align(void *w, int align) {
    QLabel *l = static_cast<QLabel *>(w);
    Qt::Alignment h;
    if (align == 1) {
        h = Qt::AlignHCenter;
    } else if (align == 2) {
        h = g_rtl ? Qt::AlignLeft : Qt::AlignRight;
    } else {
        h = g_rtl ? Qt::AlignRight : Qt::AlignLeft;
    }
    l->setAlignment(h | Qt::AlignTop);
}
void day_qt_label_set_rich_text(void *w, const char *html) {
    QLabel *l = static_cast<QLabel *>(w);
    l->setTextFormat(Qt::RichText);
    l->setText(QString::fromUtf8(html));
}
void day_qt_label_set_text(void *w, const char *text) {
    QLabel *l = static_cast<QLabel *>(w);
    // Back to plain when a label loses its runs, or the previous markup would keep parsing.
    l->setTextFormat(Qt::PlainText);
    l->setText(QString::fromUtf8(text));
}
void day_qt_label_set_font(void *w, double pt, int weight, int italic, int tabular) {
    QLabel *l = static_cast<QLabel *>(w);
    QFont f = l->font();
    f.setPointSizeF(pt);
    // `weight` is a QFont::Weight numeric value (Thin=100 … Black=900).
    if (weight > 0)
        f.setWeight(static_cast<QFont::Weight>(weight));
    f.setItalic(italic != 0);
    // Tabular figures through the OpenType feature, so the face is untouched and only the digits
    // change metrics. QFont::setFeature arrived in Qt 6.7; on older Qt the request is dropped
    // (proportional figures), which is the documented degradation for a backend that cannot
    // express it.
#if QT_VERSION >= QT_VERSION_CHECK(6, 7, 0)
    f.setFeature(QFont::Tag("tnum"), tabular != 0 ? 1 : 0);
#else
    (void)tabular;
#endif
    l->setFont(f);
}
// Text color via the label's palette (WindowText is the role QLabel draws with). Palette, not a
// stylesheet: it composes with the style engine and never fights day_qt_widget_set_surface.
// `on == 0` restores the application default palette (theme-adaptive).
void day_qt_label_set_color(void *w, double r, double g, double b, double a, int on) {
    QLabel *l = static_cast<QLabel *>(w);
    if (on) {
        QPalette pal = l->palette();
        pal.setColor(QPalette::WindowText, QColor::fromRgbF(r, g, b, a));
        l->setPalette(pal);
    } else {
        l->setPalette(QPalette());
    }
}
// Point the label at a bundled font family (registered via day_qt_register_font). Called after
// day_qt_label_set_font, so it only swaps the family; Qt falls back to the default family when
// the name doesn't resolve.
/// Switch a label to the system's FIXED-PITCH family, keeping its size and weight — inline code
/// (docs/text-runs.md). `QFontDatabase::systemFont(FixedFont)` is the platform's own choice
/// rather than a guessed family name.
void day_qt_label_set_monospace(void *w) {
    QLabel *l = static_cast<QLabel *>(w);
    QFont mono = QFontDatabase::systemFont(QFontDatabase::FixedFont);
    QFont f = l->font();
    mono.setPointSizeF(f.pointSizeF());
    mono.setWeight(f.weight());
    mono.setItalic(f.italic());
    l->setFont(mono);
}
void day_qt_label_set_font_family(void *w, const char *family) {
    QLabel *l = static_cast<QLabel *>(w);
    QFont f = l->font();
    f.setFamily(QString::fromUtf8(family));
    l->setFont(f);
}
// Make a label's text user-selectable by mouse + keyboard (the `.selectable()` modifier,
// docs/text.md). qobject_cast guards a non-label widget — a no-op rather than a bad cast.
void day_qt_label_set_selectable(void *w, int on) {
    QLabel *l = qobject_cast<QLabel *>(static_cast<QWidget *>(w));
    if (!l)
        return;
    l->setTextInteractionFlags(on ? (Qt::TextSelectableByMouse | Qt::TextSelectableByKeyboard)
                                  : Qt::NoTextInteraction);
}
// Register an application font file with the QFontDatabase (§18.4). Returns the font id
// (>= 0) or -1 on failure. Requires a constructed QApplication.
int day_qt_register_font(const char *path) {
    return QFontDatabase::addApplicationFont(QString::fromUtf8(path));
}
// The label's UNWRAPPED natural width. QLabel::sizeHint() with wordWrap on is NOT that: Qt
// applies a "readable column" heuristic and suggests a narrow wrapped block — day's measure
// contract wants the real single-line width, then asks heightForWidth at the width day grants.
// Toggle wordWrap off around the query so QLabel's own text engine answers (shaping, margins,
// indent — plain QFontMetrics::horizontalAdvance comes up a few px short of it and the last
// word wraps). No event loop runs between the toggles, so nothing paints in the off state.
int day_qt_label_natural_width(void *w) {
    QLabel *l = static_cast<QLabel *>(w);
    l->setWordWrap(false);
    const int width = l->sizeHint().width();
    l->setWordWrap(true);
    return width;
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

/// Style a button WITHOUT replacing it: `kind` 0 automatic, 1 bordered, 2 prominent, 3 tinted.
///
/// A tint is a stylesheet on the QPushButton itself, so it stays a real button — focus, keyboard
/// activation (Space/Enter) and the accessibility role are the widget's, not ours. The stylesheet
/// spells out :hover, :pressed and :disabled, because setting a background-color makes Qt stop
/// drawing the native bevel that would otherwise provide them.
void day_qt_button_set_style(void *w, int kind, unsigned argb, unsigned fg_argb) {
    QPushButton *b = static_cast<QPushButton *>(w);
    if (kind != 3) {
        b->setStyleSheet(QString());
        // Prominent: Qt Widgets has no accent button, so this asks the style for the DEFAULT
        // button treatment and otherwise leaves the stock look (graceful degradation).
        b->setDefault(kind == 2);
        return;
    }
    auto hex = [](unsigned c) {
        return QString("#%1").arg(c & 0xFFFFFF, 6, 16, QChar('0'));
    };
    const QColor fill(QColor::fromRgb(argb));
    const QString base = hex(argb), text = hex(fg_argb);
    b->setStyleSheet(QString("QPushButton { background-color: %1; color: %2; border: none;"
                             " border-radius: 6px; padding: 6px 14px; }"
                             "QPushButton:hover:!pressed { background-color: %3; }"
                             "QPushButton:pressed { background-color: %4; }"
                             "QPushButton:disabled { background-color: %5; color: %6; }")
                         .arg(base, text,
                              hex(fill.lighter(112).rgb()), hex(fill.darker(115).rgb()),
                              hex(fill.lighter(140).rgb()), hex(QColor(150, 150, 150).rgb())));
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
// `cb(id, value, committed)`: `committed != 0` marks the value the user settled on, as against
// the stream a drag produces (day-spec `Event::ValueCommitted`). Qt tells the two apart exactly:
// while the thumb is held, `isSliderDown()` is true and only `sliderReleased` ends it; a keyboard
// step, a wheel notch, or a click on the groove moves the value with the thumb NOT down, and is
// therefore already settled when `valueChanged` fires.
namespace {
// `valueChanged` fires for PROGRAMMATIC setValue too, and with `isSliderDown()` false it
// would be reported as a COMMITTED user change — so a day-driven write (a binding following
// a selection switch) must be suppressed, or it echoes back as a phantom undo unit. A user's
// keyboard arrow / track click still reports committed, as the Event contract wants.
class DaySlider : public QSlider {
public:
    using QSlider::QSlider;
    bool suppress = false;
};
} // namespace

void *day_qt_slider_new(int value, uint64_t id, void (*cb)(uint64_t, int, int)) {
    DaySlider *s = new DaySlider(Qt::Horizontal);
    s->setMinimum(0);
    s->setMaximum(1000);
    s->setValue(value);
    QObject::connect(s, &QSlider::valueChanged, [s, id, cb](int v) {
        if (s->suppress)
            return;
        cb(id, v, s->isSliderDown() ? 0 : 1);
    });
    QObject::connect(s, &QSlider::sliderReleased, [s, id, cb]() { cb(id, s->value(), 1); });
    return s;
}
void day_qt_slider_set(void *w, int value) {
    DaySlider *s = static_cast<DaySlider *>(w);
    if (s->value() != value) {
        s->suppress = true;
        s->setValue(value);
        s->suppress = false;
    }
}

// --- line edit ---
void *day_qt_lineedit_new(const char *text, const char *ph, uint64_t id,
                          void (*cb)(uint64_t, const char *)) {
    QLineEdit *e = new QLineEdit(QString::fromUtf8(text));
    if (g_rtl) e->setLayoutDirection(Qt::RightToLeft);
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
void *day_qt_scroll_new(int horizontal) {
    QScrollArea *sa = new QScrollArea();
    sa->setWidgetResizable(false);
    sa->setFrameShape(QFrame::NoFrame);
    if (horizontal)
        sa->setVerticalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    else
        sa->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    // Transparent like AppKit's `setDrawsBackground(false)` scroll (and GTK's day-scroll CSS):
    // content layered BEHIND the scroll (e.g. a gradient backdrop in a zstack) must show
    // through. Both the palette flags AND the stylesheet are needed — QScrollArea otherwise
    // erases its viewport with the palette Window brush on some styles.
    sa->viewport()->setAutoFillBackground(false);
    sa->setStyleSheet("QScrollArea { background: transparent; } QScrollArea > QWidget > QWidget { background: transparent; }");
    QWidget *content = new QWidget();
    content->setAutoFillBackground(false);
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
// Minimal scroll so [x,y,w,h] (content px) is visible — scrollRectToVisible semantics on both
// axes; the scroll bars clamp to their own range.
void day_qt_scroll_to_rect(void *w, int x, int y, int rw, int rh) {
    QScrollArea *sa = qobject_cast<QScrollArea *>(static_cast<QWidget *>(w));
    if (!sa) return;
    if (QScrollBar *sb = sa->verticalScrollBar()) {
        int v = sb->value();
        int page = sa->viewport()->height();
        if (y + rh > v + page) v = y + rh - page;
        if (y < v) v = y;
        sb->setValue(v);
    }
    if (QScrollBar *sb = sa->horizontalScrollBar()) {
        int v = sb->value();
        int page = sa->viewport()->width();
        if (x + rw > v + page) v = x + rw - page;
        if (x < v) v = x;
        sb->setValue(v);
    }
}
// Scroll the (emulated) list/scroll area to its very bottom so the last row is fully visible.
void day_qt_scroll_to_bottom(void *w) {
    QScrollArea *sa = qobject_cast<QScrollArea *>(static_cast<QWidget *>(w));
    if (!sa) return;
    if (QScrollBar *sb = sa->verticalScrollBar()) sb->setValue(sb->maximum());
}

// What the list is actually SHOWING: the scrolled offset and the visible height. The emulated
// list positions every row itself, so this is the only way it can know which handful of a
// ten-thousand-row source needs building — the content widget is the full extent either way.
void day_qt_list_viewport(void *w, double *out_offset, double *out_height) {
    *out_offset = 0;
    *out_height = 0;
    QScrollArea *sa = qobject_cast<QScrollArea *>(static_cast<QWidget *>(w));
    if (!sa) return;
    if (QScrollBar *sb = sa->verticalScrollBar()) *out_offset = sb->value();
    if (sa->viewport()) *out_height = sa->viewport()->height();
}

// Report scrolling, so rows coming INTO view get built before they are looked at.
void day_qt_list_on_scroll(void *w, uint64_t node, void (*cb)(uint64_t)) {
    QScrollArea *sa = qobject_cast<QScrollArea *>(static_cast<QWidget *>(w));
    if (!sa) return;
    QScrollBar *sb = sa->verticalScrollBar();
    if (!sb) return;
    QObject::connect(sb, &QScrollBar::valueChanged, sb, [node, cb](int) { cb(node); });
}

// Scroll a QScrollArea host to an absolute vertical offset (px, clamped by the bar).
void day_qt_scroll_to_y(void *w, int y) {
    QScrollArea *sa = qobject_cast<QScrollArea *>(static_cast<QWidget *>(w));
    if (!sa) return;
    if (QScrollBar *sb = sa->verticalScrollBar()) sb->setValue(y);
}

// --- tree / geometry ---
// Emulated fullscreen cover (docs/cover.md): bring the re-homed cover to the front, and give it
// an OPAQUE default surface (the palette Window color) so it occludes the page under it.
void day_qt_cover_top(void *w) {
    QWidget *c = static_cast<QWidget *>(w);
    c->setAutoFillBackground(true);
    c->raise();
}

void day_qt_add_child(void *parent, void *child) {
    // Day's tree mounts under the window's CONTENT area (below any in-window menu bar).
    if (auto *dw = dynamic_cast<DayWindow *>(static_cast<QWidget *>(parent)))
        parent = dw->content;
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
// First text baseline from the widget's top, in device-independent px, for a widget laid out
// `h` tall (docs/baseline.md). Qt has no baseline-alignment protocol — QFormLayout aligns
// boxes, not text — so this derives it the way Qt's own painters do: center one line of the
// widget's font in the box, and the baseline sits an ascent below the line's top. Widgets with
// no text return -1, which day reads as "no baseline, keep centering".
double day_qt_baseline(void *w, double h) {
    QWidget *widget = static_cast<QWidget *>(w);
    // A widget with no text has nothing to sit on a line. Qt has no "has text" predicate, so
    // ask the layout-relevant classes directly and let everything else opt out.
    const bool texty = qobject_cast<QLabel *>(widget) || qobject_cast<QLineEdit *>(widget) ||
                       qobject_cast<QAbstractButton *>(widget) ||
                       qobject_cast<QComboBox *>(widget) || qobject_cast<QTextEdit *>(widget) ||
                       qobject_cast<QDateTimeEdit *>(widget) ||
                       qobject_cast<QAbstractSpinBox *>(widget);
    if (!texty) return -1.0;
    QFontMetricsF fm(widget->font());
    const double line = fm.height();
    const double top = h > line ? (h - line) / 2.0 : 0.0;
    return top + fm.ascent();
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
// --- inspector (docs/inspector.md): the same QSplitter family, panel pane TRAILING. Not a
// QDockWidget: DayWindow is a plain QWidget with hand-managed chrome (see the toolbar note
// above), and dock areas need QMainWindow. ---
void *day_qt_inspector_new(double panel_width) {
    auto *s = new QSplitter(Qt::Horizontal);
    s->setChildrenCollapsible(false);
    s->addWidget(new QWidget());
    s->addWidget(new QWidget());
    s->setStretchFactor(0, 1);
    s->setStretchFactor(1, 0);
    s->setSizes({640, static_cast<int>(panel_width)});
    return s;
}
void day_qt_splitter_on_moved(void *w, void (*cb)(void *)) {
    auto *s = qobject_cast<QSplitter *>(static_cast<QWidget *>(w));
    if (s) {
        QObject::connect(s, &QSplitter::splitterMoved, [s, cb](int, int) { cb(s); });
    }
}
// Stack-nav back header (docs/navigation.md): desktop has no system back affordance, so a
// pushed page gets a slim bar — back arrow + bold centered title — above the pages. Installed
// into the splitter's detail side; returns the NEW pages host (below the header), which the
// caller uses in place of the raw pane. Hidden until a page is pushed.
void *day_qt_nav_header_install(void *splitter, uint64_t id, void (*cb)(uint64_t)) {
    auto *s = qobject_cast<QSplitter *>(static_cast<QWidget *>(splitter));
    if (!s)
        return nullptr;
    QWidget *detail = s->widget(1);
    auto *header = new QWidget();
    header->setObjectName("day-nav-header");
    header->setFixedHeight(36);
    auto *hl = new QHBoxLayout(header);
    hl->setContentsMargins(4, 2, 8, 2);
    auto *back = new QToolButton();
    back->setArrowType(Qt::LeftArrow);
    back->setAutoRaise(true);
    QObject::connect(back, &QToolButton::clicked, [id, cb]() { cb(id); });
    auto *title = new QLabel();
    title->setObjectName("day-nav-title");
    title->setAlignment(Qt::AlignCenter);
    QFont f = title->font();
    f.setBold(true);
    title->setFont(f);
    // A right-side spacer the width of the back button keeps the title optically centered.
    auto *balance = new QWidget();
    balance->setFixedWidth(back->sizeHint().width());
    hl->addWidget(back);
    hl->addWidget(title, 1);
    hl->addWidget(balance);
    auto *pages = new QWidget();
    auto *vl = new QVBoxLayout(detail);
    vl->setContentsMargins(0, 0, 0, 0);
    vl->setSpacing(0);
    vl->addWidget(header);
    vl->addWidget(pages, 1);
    header->hide();
    return pages;
}
// Show/hide the back header and set its title; activates the layout so the pages host has its
// final size before the caller re-reports page frames.
void day_qt_nav_header_update(void *splitter, int visible, const char *title) {
    auto *s = qobject_cast<QSplitter *>(static_cast<QWidget *>(splitter));
    if (!s)
        return;
    if (auto *t = s->findChild<QLabel *>("day-nav-title"))
        t->setText(QString::fromUtf8(title));
    if (auto *h = s->findChild<QWidget *>("day-nav-header"))
        h->setVisible(visible != 0);
    if (QWidget *detail = s->widget(1); detail && detail->layout())
        detail->layout()->activate();
}
void day_qt_widget_size(void *w, double *out_w, double *out_h) {
    QWidget *q = static_cast<QWidget *>(w);
    *out_w = q->width();
    *out_h = q->height();
}
void day_qt_set_visible(void *w, int visible) {
    static_cast<QWidget *>(w)->setVisible(visible != 0);
}

// Day curve code (§8.4) → QEasingCurve. Spring approximates as OutBack (overshoot).
static QEasingCurve day_qt_easing(int curve) {
    switch (curve) {
    case 0:
        return QEasingCurve(QEasingCurve::Linear);
    case 1:
        return QEasingCurve(QEasingCurve::InQuad);
    case 2:
        return QEasingCurve(QEasingCurve::OutQuad);
    case 4:
        return QEasingCurve(QEasingCurve::OutBack);
    default:
        return QEasingCurve(QEasingCurve::InOutQuad);
    }
}

// Drive one field-group of the effect (tagged, so an opacity retrigger and a transform retrigger
// don't stop each other). `set(t)` writes the interpolated fields for progress `t` in 0..1, then
// requests a repaint. The final value is applied immediately so it holds even when the animation
// never ticks (headless / unmapped); the tween then runs over it. (Not a template — this lives in
// `extern "C"`, which forbids templates.)
static void day_qt_animate(DayEffect *eff, const char *tag, int durMs, int curve,
                           std::function<void(double)> set) {
    for (QVariantAnimation *old : eff->findChildren<QVariantAnimation *>()) {
        if (old->objectName() == QLatin1String(tag)) {
            old->stop();
        }
    }
    set(1.0);
    eff->update();
    if (durMs <= 0) {
        return;
    }
    QVariantAnimation *anim = new QVariantAnimation(eff);
    anim->setObjectName(QLatin1String(tag));
    anim->setDuration(durMs);
    anim->setStartValue(0.0);
    anim->setEndValue(1.0);
    anim->setEasingCurve(day_qt_easing(curve));
    QObject::connect(anim, &QVariantAnimation::valueChanged, eff,
                     [eff, set](const QVariant &v) {
                         set(v.toDouble());
                         eff->update();
                     });
    anim->start(QAbstractAnimation::DeleteWhenStopped);
}

void day_qt_set_opacity(void *w, double opacity, int durMs, int curve) {
    DayEffect *eff = day_qt_effect(static_cast<QWidget *>(w));
    double from = eff->opacity;
    day_qt_animate(eff, "op", durMs, curve,
                   [eff, from, opacity](double t) { eff->opacity = from + (opacity - from) * t; });
}

void day_qt_set_transform(void *w, double tx, double ty, double sx, double sy, double rot,
                          int durMs, int curve) {
    DayEffect *eff = day_qt_effect(static_cast<QWidget *>(w));
    double fx = eff->tx, fy = eff->ty, fsx = eff->sx, fsy = eff->sy, fr = eff->rot;
    day_qt_animate(eff, "tf", durMs, curve, [=](double t) {
        eff->tx = fx + (tx - fx) * t;
        eff->ty = fy + (ty - fy) * t;
        eff->sx = fsx + (sx - fsx) * t;
        eff->sy = fsy + (sy - fsy) * t;
        eff->rot = fr + (rot - fr) * t;
    });
}

// --- navigation menu (docs/navigation.md): QListWidget with a sidebar treatment ---
void *day_qt_navlist_new(uint64_t id, void (*cb)(uint64_t, int)) {
    auto *w = new QListWidget();
    w->setFrameShape(QFrame::NoFrame);
    w->setIconSize(QSize(18, 18));
    // Long entries (feed titles, file names) truncate with an ellipsis instead of running
    // under the sidebar's edge, and never widen the pane to fit.
    w->setTextElideMode(Qt::ElideRight);
    w->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    w->setStyleSheet(
        "QListWidget{background:transparent;outline:0;}"
        "QListWidget::item{padding:6px 10px;border-radius:6px;margin:1px 4px;}"
        "QListWidget::item:selected{background:palette(highlight);"
        "color:palette(highlighted-text);}");
    QObject::connect(w, &QListWidget::currentRowChanged,
                     [id, cb](int row) { cb(id, row); });
    return w;
}
// Template glyphs are black-on-transparent; tint to the palette text color so they show
// in both light and dark mode (raw black is invisible on a dark sidebar).
// Load a glyph at a usable size. An SVG goes through Qt's SVG icon engine, which RENDERS at the
// size asked for instead of scaling a cached bitmap (docs/vectors.md).
static QPixmap day_qt_load_glyph(const QString &path, int px) {
    QPixmap pm;
    if (path.endsWith(QLatin1String(".svg"), Qt::CaseInsensitive)) {
        // Qt's SVG icon engine (the `libqsvg` imageformats plugin) RENDERS at the size asked for,
        // rather than scaling a cached bitmap — so a glyph stays sharp at any icon size and on
        // any display scale (docs/vectors.md).
        const qreal dpr = qApp ? qApp->devicePixelRatio() : 1.0;
        const int target = px > 0 ? px : 64;
        pm = QIcon(path).pixmap(QSize(target, target) * dpr);
        pm.setDevicePixelRatio(dpr);
    } else {
        pm = QPixmap(path);
    }
    return pm;
}

// Recolor a template glyph: keep the alpha, replace every color (docs/vectors.md "Tint").
static QPixmap day_qt_tint_glyph(const QPixmap &src, const QColor &color) {
    if (src.isNull()) return src;
    QPixmap tinted = src;
    QPainter p(&tinted);
    p.setCompositionMode(QPainter::CompositionMode_SourceIn);
    p.fillRect(tinted.rect(), color);
    p.end();
    return tinted;
}

static QIcon day_qt_tinted_icon(const QString &path, const QColor &color, int px = 0) {
    QPixmap pm = day_qt_load_glyph(path, px);
    if (pm.isNull()) return QIcon();
    return QIcon(day_qt_tint_glyph(pm, color));
}

// A nav row's trailing status glyph (docs/navigation.md). QListWidgetItem carries ONE icon, and
// that slot is the row's leading icon — so the badge rides a custom data role and a delegate
// paints it at the trailing edge after the stock item. Painting it rather than embedding a widget
// keeps the list's native row rendering, selection highlight and keyboard handling untouched.
static constexpr int DAY_NAV_BADGE_ROLE = Qt::UserRole + 17;

class DayNavBadgeDelegate : public QStyledItemDelegate {
public:
    using QStyledItemDelegate::QStyledItemDelegate;
    void paint(QPainter *painter, const QStyleOptionViewItem &option,
               const QModelIndex &index) const override {
        QStyledItemDelegate::paint(painter, option, index);
        const QVariant v = index.data(DAY_NAV_BADGE_ROLE);
        if (!v.canConvert<QIcon>()) return;
        const QIcon badge = v.value<QIcon>();
        if (badge.isNull()) return;
        const int sz = qMin(16, option.rect.height() - 6);
        if (sz <= 0) return;
        // Trailing edge, mirrored under RTL like every other trailing affordance.
        const bool rtl = option.direction == Qt::RightToLeft;
        const int x = rtl ? option.rect.left() + 6 : option.rect.right() - sz - 6;
        const int y = option.rect.top() + (option.rect.height() - sz) / 2;
        badge.paint(painter, QRect(x, y, sz, sz));
    }
};

void day_qt_navlist_set_items(void *w, const char *joined, const char *icons,
                              const char *tints, const char *badge_icons,
                              const char *badge_tints) {
    auto *l = qobject_cast<QListWidget *>(static_cast<QWidget *>(w));
    if (!l) return;
    // Split titles WITHOUT SkipEmptyParts so the icon list stays row-aligned; the icon
    // and tint lists are likewise split keep-empty (empty entry = no icon / no tint).
    const QStringList titles =
        QString::fromUtf8(joined).split(QChar(0x1f), Qt::KeepEmptyParts);
    const QStringList iconPaths =
        QString::fromUtf8(icons).split(QChar(0x1f), Qt::KeepEmptyParts);
    const QStringList tintStrs =
        QString::fromUtf8(tints).split(QChar(0x1f), Qt::KeepEmptyParts);
    const QStringList badgePaths =
        QString::fromUtf8(badge_icons).split(QChar(0x1f), Qt::KeepEmptyParts);
    const QStringList badgeTintStrs =
        QString::fromUtf8(badge_tints).split(QChar(0x1f), Qt::KeepEmptyParts);
    const QColor textColor = l->palette().color(QPalette::Text);
    // Installed once, tracked by a dynamic property: `qobject_cast` would need Q_OBJECT and
    // therefore moc, which this shim is deliberately built without (plain cc-rs, no code
    // generation step). The delegate is inert on rows carrying no badge data.
    if (!l->property("dayBadgeDelegate").toBool()) {
        l->setItemDelegate(new DayNavBadgeDelegate(l));
        l->setProperty("dayBadgeDelegate", true);
    }
    l->blockSignals(true);
    l->clear();
    for (int i = 0; i < titles.size(); ++i) {
        auto *item = new QListWidgetItem(titles.at(i), l);
        if (i < iconPaths.size() && !iconPaths.at(i).isEmpty()) {
            // A row's own "#rrggbb" tint (docs/vectors.md) wins over the palette default.
            QColor rowColor = textColor;
            if (i < tintStrs.size() && !tintStrs.at(i).isEmpty()) {
                QColor c(tintStrs.at(i));
                if (c.isValid()) rowColor = c;
            }
            QIcon icon = day_qt_tinted_icon(iconPaths.at(i), rowColor);
            if (!icon.isNull()) item->setIcon(icon);
        }
        if (i < badgePaths.size() && !badgePaths.at(i).isEmpty()) {
            QColor badgeColor = textColor;
            if (i < badgeTintStrs.size() && !badgeTintStrs.at(i).isEmpty()) {
                QColor c(badgeTintStrs.at(i));
                if (c.isValid()) badgeColor = c;
            }
            QIcon badge = day_qt_tinted_icon(badgePaths.at(i), badgeColor);
            if (!badge.isNull()) item->setData(DAY_NAV_BADGE_ROLE, QVariant::fromValue(badge));
        }
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

// The first QTabWidget above `w`, or null.
//
// The navigation suite (`NavPresentation::Tabs`) is a QTabWidget, and its bar's labels come from
// the host's nav menu — which arrives separately, nested however the page wrapped it. Walking Qt's
// own parent chain is what connects the two without Day tracking a parallel tree.
void *day_qt_enclosing_tabs(void *w) {
    for (QWidget *p = static_cast<QWidget *>(w); p != nullptr; p = p->parentWidget()) {
        if (auto *t = qobject_cast<QTabWidget *>(p)) {
            return t;
        }
    }
    return nullptr;
}
// A tab that is present but not shown: the suite's sidebar page, whose rows BECAME the bar.
void day_qt_tabs_set_page_visible(void *tabs, void *page, int visible) {
    auto *t = static_cast<QTabWidget *>(tabs);
    int i = t->indexOf(static_cast<QWidget *>(page));
    if (i >= 0) {
        t->setTabVisible(i, visible != 0);
    }
}

// --- the navigation suite (docs/navigation.md): a QTabWidget owns its page widgets ---
void *day_qt_tabs_new(uint64_t id, void (*cb)(uint64_t, int)) {
    auto *t = new QTabWidget();
#ifdef Q_OS_MACOS
    // QMacStyle draws tab controls by rendering a live NSView, which needs a CGContext. Day's
    // window snapshot paints into an offscreen device that has none, and the style dereferences
    // it — a crash on capture, not a blank bar. Fusion draws the same bar in pure Qt and survives
    // it. macOS is a development target for this backend; every platform Qt actually ships to
    // keeps the native style.
    if (QStyle *fusion = QStyleFactory::create(QStringLiteral("Fusion"))) {
        fusion->setParent(t);
        t->tabBar()->setStyle(fusion);
    }
#endif
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
// A tab's leading glyph (docs/navigation.md). QTabWidget draws an icon beside the label when one is
// set, so a bundled template vector shows here exactly as it does on the phones' tab bars —
// tinted to the palette text color like every other template glyph in this shim.
void day_qt_tabs_set_icon(void *tabs, int index, const char *path) {
    auto *t = static_cast<QTabWidget *>(tabs);
    if (index < 0 || index >= t->count()) {
        return;
    }
    if (path == nullptr || *path == '\0') {
        t->setTabIcon(index, QIcon());
        return;
    }
    t->setTabIcon(index, day_qt_tinted_icon(QString::fromUtf8(path),
                                            t->palette().color(QPalette::WindowText)));
}
// Data-driven tabs (docs/navigation.md): drop a page's tab; relabel a tab.
void day_qt_tabs_remove_page(void *tabs, void *page) {
    auto *t = static_cast<QTabWidget *>(tabs);
    int i = t->indexOf(static_cast<QWidget *>(page));
    if (i >= 0) {
        bool b = t->blockSignals(true);
        t->removeTab(i); // QTabWidget::removeTab does not delete the page widget (Day owns it)
        t->blockSignals(b);
    }
}
void day_qt_tabs_set_title(void *tabs, int index, const char *title) {
    auto *t = static_cast<QTabWidget *>(tabs);
    if (index >= 0 && index < t->count())
        t->setTabText(index, QString::fromUtf8(title));
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
#include <QLinearGradient>
#include <QRadialGradient>
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
        // A decoded kind-14 record (set-gradient): type (0 linear, 1 radial) + unit geometry +
        // stops, applied as the brush of the NEXT fill-shape record (resolved against that
        // shape's bounding rect).
        bool gradPending = false;
        int gradType = 0;
        double gsx = 0, gsy = 0, gex = 0, gey = 0;
        QGradientStops gstops;
        // A decoded kind-18 record (stroke style), applied to the NEXT stroke record only.
        bool stylePending = false;
        int sCap = 0, sJoin = 0; double sMiter = 10.0, sPhase = 0.0;
        QVector<qreal> sDash;
        // Parse "M x y L x y Q .. C .. Z" (day_spec::encode_path) into a QPainterPath.
        auto parsePath = [](const QString &spec, int rule) {
            QPainterPath path;
            path.setFillRule(rule == 1 ? Qt::OddEvenFill : Qt::WindingFill);
            const QStringList tok = spec.split(' ', Qt::SkipEmptyParts);
            for (int i = 0; i < tok.size();) {
                const QString &op = tok[i++];
                auto num = [&]() { return i < tok.size() ? tok[i++].toDouble() : 0.0; };
                if (op == "M") { double x = num(), y = num(); path.moveTo(x, y); }
                else if (op == "L") { double x = num(), y = num(); path.lineTo(x, y); }
                else if (op == "Q") { double cx = num(), cy = num(), x = num(), y = num(); path.quadTo(cx, cy, x, y); }
                else if (op == "C") { double ax = num(), ay = num(), bx = num(), by = num(), x = num(), y = num(); path.cubicTo(ax, ay, bx, by, x, y); }
                else if (op == "Z") path.closeSubpath();
            }
            return path;
        };
        auto gradBrush = [&](const QRectF &bounds) {
            gradPending = false;
            if (gradType == 1) {
                // Radial: ObjectMode maps the unit square onto the drawn shape's bounding
                // rect, so a unit-space circle renders elliptically in non-square bounds —
                // the same rule as every other backend. (a,b = center, c = radius: gsx,gsy,gex.)
                QRadialGradient rg(QPointF(gsx, gsy), gex, QPointF(gsx, gsy));
                rg.setCoordinateMode(QGradient::ObjectMode);
                rg.setStops(gstops);
                return QBrush(rg);
            }
            QLinearGradient lg(bounds.x() + gsx * bounds.width(), bounds.y() + gsy * bounds.height(),
                               bounds.x() + gex * bounds.width(), bounds.y() + gey * bounds.height());
            lg.setStops(gstops);
            return QBrush(lg);
        };
        for (int i = 0; i + 8 < nums.size(); i += 9) {
            int k = (int)nums[i];
            double a = nums[i+1], b = nums[i+2], c = nums[i+3], d = nums[i+4];
            double e = nums[i+5], f = nums[i+6], g = nums[i+7];
            unsigned col = (unsigned)nums[i+8];
            QColor color((col >> 16) & 0xff, (col >> 8) & 0xff, col & 0xff, (col >> 24) & 0xff);
            QPen pen(color); pen.setWidthF(g);
            // Day's default cap is BUTT (this backend used to force RoundCap); a kind-18 record
            // overrides cap/join/miter/dash for the one stroke that follows it.
            pen.setCapStyle(Qt::FlatCap);
            if (stylePending) {
                pen.setCapStyle(sCap == 1 ? Qt::RoundCap : sCap == 2 ? Qt::SquareCap : Qt::FlatCap);
                pen.setJoinStyle(sJoin == 1 ? Qt::RoundJoin : sJoin == 2 ? Qt::BevelJoin : Qt::MiterJoin);
                pen.setMiterLimit(sMiter);
                if (!sDash.isEmpty()) {
                    // Qt's dash pattern is in PEN WIDTHS, not pixels.
                    QVector<qreal> scaled;
                    const qreal wdt = g > 0.0 ? g : 1.0;
                    for (qreal v : sDash) scaled << v / wdt;
                    pen.setDashPattern(scaled);
                    pen.setDashOffset(sPhase / wdt);
                }
                // Consumed by whichever stroke record this is; cleared at the end of the case.
            }
            switch (k) {
                case 0:
                    if (gradPending) { p.setPen(Qt::NoPen); p.setBrush(gradBrush(QRectF(a, b, c, d))); p.drawRect(QRectF(a, b, c, d)); }
                    else p.fillRect(QRectF(a, b, c, d), color);
                    break;
                case 1: p.setPen(pen); p.setBrush(Qt::NoBrush); p.drawRect(QRectF(a, b, c, d)); break;
                case 2: p.setPen(Qt::NoPen); p.setBrush(gradPending ? gradBrush(QRectF(a, b, c, d)) : QBrush(color)); p.drawRoundedRect(QRectF(a, b, c, d), e, e); break;
                case 13: p.setPen(pen); p.setBrush(Qt::NoBrush); p.drawRoundedRect(QRectF(a, b, c, d), e, e); break;
                case 3: p.setPen(Qt::NoPen); p.setBrush(gradPending ? gradBrush(QRectF(a, b, c, d)) : QBrush(color)); p.drawEllipse(QRectF(a, b, c, d)); break;
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
                        if (k == 11) {
                            p.setPen(Qt::NoPen);
                            p.setBrush(gradPending ? gradBrush(poly.boundingRect()) : QBrush(color));
                            p.drawPolygon(poly);
                        }
                        else { p.setPen(pen); p.setBrush(Qt::NoBrush); p.drawPolygon(poly); }
                    }
                    break;
                }
                case 15: case 16: { // path (15 fill / 16 stroke); segments in texts, f = fill rule
                    QString spec = ti < texts.size() ? texts[ti++] : QString();
                    QPainterPath path = parsePath(spec, (int)f);
                    if (k == 15) {
                        p.setPen(Qt::NoPen);
                        p.setBrush(gradPending ? gradBrush(path.boundingRect()) : QBrush(color));
                        p.drawPath(path);
                    } else {
                        if (gradPending) pen.setBrush(gradBrush(path.boundingRect()));
                        p.setPen(pen); p.setBrush(Qt::NoBrush); p.drawPath(path);
                        gradPending = false;
                    }
                    break;
                }
                case 17: { // clip: f names the shape, a..d geometry, e radius or fill rule
                    QPainterPath clip;
                    switch ((int)f) {
                        case 1: clip.addRoundedRect(QRectF(a, b, c, d), e, e); break;
                        case 2: clip.addEllipse(QRectF(a, b, c, d)); break;
                        case 3: clip = parsePath(ti < texts.size() ? texts[ti++] : QString(), (int)e); break;
                        case 4: {
                            QString tp = ti < texts.size() ? texts[ti++] : QString();
                            QPolygonF poly;
                            for (const QString &pair : tp.split(' ', Qt::SkipEmptyParts)) {
                                int comma = pair.indexOf(',');
                                if (comma > 0)
                                    poly << QPointF(pair.left(comma).toDouble(), pair.mid(comma + 1).toDouble());
                            }
                            clip.addPolygon(poly);
                            break;
                        }
                        default: clip.addRect(QRectF(a, b, c, d)); break;
                    }
                    // IntersectClip, matching the spec: a clip only ever narrows until restore.
                    p.setClipPath(clip, Qt::IntersectClip);
                    break;
                }
                case 18: { // stroke style for the NEXT stroke: a cap, b join, c miter, d phase
                    QString t = ti < texts.size() ? texts[ti++] : QString();
                    sCap = (int)a; sJoin = (int)b; sMiter = c; sPhase = d;
                    sDash.clear();
                    for (const QString &v : t.split(' ', Qt::SkipEmptyParts)) sDash << v.toDouble();
                    stylePending = true;
                    break;
                }
                case 14: { // set-gradient (f = type): stops ride the texts channel as "offset,aarrggbb …"
                    QString t = ti < texts.size() ? texts[ti++] : QString();
                    gradType = (int)f;
                    gstops.clear();
                    for (const QString &pair : t.split(' ', Qt::SkipEmptyParts)) {
                        int comma = pair.indexOf(',');
                        if (comma <= 0) continue;
                        double off = pair.left(comma).toDouble();
                        unsigned bits = pair.mid(comma + 1).toUInt(nullptr, 16);
                        gstops << QGradientStop(off, QColor((bits >> 16) & 0xff, (bits >> 8) & 0xff,
                                                            bits & 0xff, (bits >> 24) & 0xff));
                    }
                    gsx = a; gsy = b; gex = c; gey = d;
                    gradPending = !gstops.isEmpty();
                    break;
                }
            }
            // A style record applies to ONE stroke; anything else that consumed the pen clears it
            // too, so it can never leak into a later record.
            if (k != 18) stylePending = false;
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

// Aspect-aware image widget (§18.3): paints the pixmap scaled per content mode
// (0 = fit / KeepAspectRatio, 1 = fill / KeepAspectRatioByExpanding + crop, 2 = stretch).
class DayImageLabel : public QLabel {
public:
    QPixmap orig;
    int mode;
    explicit DayImageLabel(int m) : mode(m) {}
    void setImage(const QPixmap &p) { orig = p; update(); }
protected:
    void paintEvent(QPaintEvent *) override {
        if (orig.isNull()) return;
        QPainter painter(this);
        if (mode == 2) { // stretch
            painter.drawPixmap(rect(), orig);
            return;
        }
        Qt::AspectRatioMode arm = (mode == 1) ? Qt::KeepAspectRatioByExpanding : Qt::KeepAspectRatio;
        QPixmap scaled = orig.scaled(size(), arm, Qt::SmoothTransformation);
        if (mode == 1) painter.setClipRect(rect()); // fill: crop the overflow
        int x = (width() - scaled.width()) / 2;
        int y = (height() - scaled.height()) / 2;
        painter.drawPixmap(x, y, scaled);
    }
};

void *day_qt_image_new(const char *path, int mode, const char *tint) {
    DayImageLabel *l = new DayImageLabel(mode);
    const QString file = QString::fromUtf8(path); // ":/day/images/<name>" or a file path
    // 512 px so an upscaled glyph still has pixels to spare; an SVG renders at that size rather
    // than being blown up from the 256 px cache.
    QPixmap pm = day_qt_load_glyph(file, 512);
    if (tint != nullptr && *tint != '\0') {
        const QColor c(QString::fromUtf8(tint));
        if (c.isValid()) pm = day_qt_tint_glyph(pm, c);
    }
    if (!pm.isNull()) l->setImage(pm);
    l->setProperty("dayGlyphPath", file);
    return l;
}

// `ImagePatch::Tint`: repaint the realized glyph from its source, so a tint that follows a signal
// never rebuilds the view. An empty tint restores the authored colors.
void day_qt_image_set_tint(void *w, const char *tint) {
    auto *l = dynamic_cast<DayImageLabel *>(static_cast<QWidget *>(w));
    if (!l) return;
    const QString file = l->property("dayGlyphPath").toString();
    if (file.isEmpty()) return;
    QPixmap pm = day_qt_load_glyph(file, 512);
    if (tint != nullptr && *tint != '\0') {
        const QColor c(QString::fromUtf8(tint));
        if (c.isValid()) pm = day_qt_tint_glyph(pm, c);
    }
    if (!pm.isNull()) l->setImage(pm);
}

// App icon (§18.2): the window icon doubles as the Dock icon on macOS and the taskbar icon on
// Linux/Windows for an unbundled binary.
void day_qt_set_app_icon(const char *path) {
    QApplication::setWindowIcon(QIcon(QString::fromUtf8(path)));
}

// --- native Qt Resource System packing (§18.3) ---
// Register the app's compiled .rcc blob; then data reads are zero-copy from QResource::data().
void day_qt_register_resource(const char *path) {
    QResource::registerResource(QString::fromUtf8(path));
}
const void *day_qt_resource_data(const char *respath, size_t *out_len) {
    QResource r{ QString::fromUtf8(respath) };
    if (!r.isValid()) return nullptr;
    *out_len = (size_t)r.size();
    return (const void *)r.data(); // points into the registered (uncompressed) blob — app lifetime
}
int day_qt_resource_exists(const char *respath) {
    QResource r{ QString::fromUtf8(respath) };
    return r.isValid() ? 1 : 0;
}

// --- gestures (tap / drag / pinch / pan) ---
// kind: 0 = tap, 1 = drag, 2 = pinch, 3 = pan.
// phase: 0 = tap; 1/2/3 = drag began/changed/ended; 4/5/6 = pinch began/changed/ended
// (tx = cumulative scale); 7/8/9 = pan began/changed/ended (tx/ty = incremental delta).
typedef void (*DayGestureCb)(uint64_t node, int phase, double x, double y, double tx, double ty);

class DayGestureFilter : public QObject {
public:
    uint64_t node; int kind; DayGestureCb cb;
    bool pressed = false; QPointF start;
    double pinch_scale = 1.0;
    DayGestureFilter(uint64_t n, int k, DayGestureCb c) : node(n), kind(k), cb(c) {}
protected:
    bool eventFilter(QObject *obj, QEvent *ev) override {
        bool is_drag = kind == 1;
        switch (ev->type()) {
            case QEvent::MouseButtonPress: {
                if (kind > 1) break;
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
                if (kind > 1) break;
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
            case QEvent::NativeGesture: {
                // Trackpad pinch (macOS always; Linux where the platform plugin synthesizes
                // native gestures). Zoom values are per-event percentage deltas; the callback
                // contract wants the cumulative scale since Begin.
                if (kind != 2) break;
                QNativeGestureEvent *ge = static_cast<QNativeGestureEvent *>(ev);
                QPointF p = ge->position();
                switch (ge->gestureType()) {
                    case Qt::BeginNativeGesture:
                        pinch_scale = 1.0;
                        cb(node, 4, p.x(), p.y(), 1.0, 0.0);
                        return true;
                    case Qt::ZoomNativeGesture:
                        pinch_scale *= 1.0 + ge->value();
                        cb(node, 5, p.x(), p.y(), pinch_scale, 0.0);
                        return true;
                    case Qt::EndNativeGesture:
                        cb(node, 6, p.x(), p.y(), pinch_scale, 0.0);
                        pinch_scale = 1.0;
                        return true;
                    default: break;
                }
                break;
            }
            case QEvent::Wheel: {
                // Two-finger trackpad scroll / mouse wheel as a pan. Consumed so an enclosing
                // scroll area doesn't also move.
                if (kind != 3) break;
                QWheelEvent *we = static_cast<QWheelEvent *>(ev);
                QPointF p = we->position();
                QPointF d = we->pixelDelta();
                if (d.isNull()) {
                    // Classic wheel: notches only. Scale to a usable content step.
                    d = QPointF(we->angleDelta().x() / 120.0 * 40.0,
                                we->angleDelta().y() / 120.0 * 40.0);
                }
                int phase;
                switch (we->phase()) {
                    case Qt::ScrollBegin: phase = 7; break;
                    case Qt::ScrollEnd: phase = 9; break;
                    default: phase = 8; break; // updates and phase-less classic wheels
                }
                cb(node, phase, p.x(), p.y(), d.x(), d.y());
                return true;
            }
            default: break;
        }
        return false; // never consume: let normal widget behavior proceed
    }
};

void day_qt_enable_gesture(void *w, uint64_t node, int kind, DayGestureCb cb) {
    QWidget *widget = static_cast<QWidget *>(w);
    DayGestureFilter *f = new DayGestureFilter(node, kind, cb);
    f->setParent(widget); // freed with the widget
    widget->installEventFilter(f);
}

// --- emulated list row selection (docs/list.md) ---
// A press on a list cell reports (list node, row, modifiers) so the Rust side owns the
// selection semantics. modifiers: bit 0 = ctrl/cmd (toggle), bit 1 = shift (range).
typedef void (*DayRowClickCb)(uint64_t node, int row, int modifiers);

class DayRowClickFilter : public QObject {
public:
    uint64_t node; int row; DayRowClickCb cb;
    DayRowClickFilter(uint64_t n, int r, DayRowClickCb c) : node(n), row(r), cb(c) {}
protected:
    bool eventFilter(QObject *, QEvent *ev) override {
        if (ev->type() == QEvent::MouseButtonPress) {
            auto *me = static_cast<QMouseEvent *>(ev);
            if (me->button() == Qt::LeftButton) {
                int mods = 0;
                if (me->modifiers() & (Qt::ControlModifier | Qt::MetaModifier)) mods |= 1;
                if (me->modifiers() & Qt::ShiftModifier) mods |= 2;
                cb(node, row, mods);
            }
        }
        return false; // observe only: row content stays interactive
    }
};

// The emulated list's cell i shows row i for the cell's whole life (cells are created per
// row and never re-indexed), so the row is fixed at install time.
void day_qt_list_cell_click(void *w, uint64_t node, int row, DayRowClickCb cb) {
    QWidget *widget = static_cast<QWidget *>(w);
    DayRowClickFilter *f = new DayRowClickFilter(node, row, cb);
    f->setParent(widget); // freed with the widget
    widget->installEventFilter(f);
}

// --- emulated list drag-to-reorder (docs/list.md) ---
// Qt's own QDrag carries the affordance — the grabbed cell as the drag pixmap, the forbidden
// cursor over a denied slot, a 2px palette-highlight insertion line — while the DECISIONS stay
// Rust's: every hovered slot is vetted synchronously through the can-move callback (the app's
// guard), and the drop commits through the move callback.
typedef int (*DayListCanMoveCb)(uint64_t node, int from, int to);
typedef void (*DayListMoveCb)(uint64_t node, int from, int to);

static const char *DAY_ROW_MIME = "application/x-day-row";

// Starts a QDrag when a press on cell `row` moves past the platform drag threshold. Cell index
// == row for the cell's whole life (the click filter above relies on the same invariant).
class DayCellDragFilter : public QObject {
public:
    uint64_t node; int row;
    DayCellDragFilter(uint64_t n, int r) : node(n), row(r) {}
protected:
    bool eventFilter(QObject *obj, QEvent *ev) override {
        QWidget *w = static_cast<QWidget *>(obj);
        if (ev->type() == QEvent::MouseButtonPress) {
            auto *me = static_cast<QMouseEvent *>(ev);
            if (me->button() == Qt::LeftButton) press = me->pos();
        } else if (ev->type() == QEvent::MouseMove) {
            auto *me = static_cast<QMouseEvent *>(ev);
            if (!press.isNull()
                && (me->pos() - press).manhattanLength() >= QApplication::startDragDistance()) {
                QDrag *drag = new QDrag(w);
                QMimeData *mime = new QMimeData();
                mime->setData(DAY_ROW_MIME, QByteArray::number(row));
                drag->setMimeData(mime);
                drag->setPixmap(w->grab());
                drag->setHotSpot(press);
                press = QPoint();
                drag->exec(Qt::MoveAction);
                return true;
            }
        } else if (ev->type() == QEvent::MouseButtonRelease) {
            press = QPoint();
        }
        return false; // observe until the drag actually starts
    }
private:
    QPoint press;
};

// Accepts day-row drops on the list's content widget, drawing the insertion line where the
// (possibly retargeted) drop would land and refusing denied slots so Qt shows the no-drop cursor.
class DayListDropFilter : public QObject {
public:
    uint64_t node; int rowH; DayListCanMoveCb can; DayListMoveCb commit;
    QWidget *line;
    DayListDropFilter(uint64_t n, int rh, DayListCanMoveCb c, DayListMoveCb m, QWidget *content)
        : node(n), rowH(rh > 0 ? rh : 1), can(c), commit(m) {
        line = new QWidget(content);
        line->setFixedHeight(2);
        line->setAutoFillBackground(true);
        QPalette p = line->palette();
        p.setColor(QPalette::Window, p.color(QPalette::Highlight));
        line->setPalette(p);
        line->hide();
    }
protected:
    bool eventFilter(QObject *obj, QEvent *ev) override {
        QWidget *content = static_cast<QWidget *>(obj);
        if (ev->type() == QEvent::DragEnter || ev->type() == QEvent::DragMove) {
            auto *e = static_cast<QDragMoveEvent *>(ev);
            int from = fromOf(e);
            if (from < 0) return false; // not a day row — none of our business
            int accepted = can(node, from, slotOf(e));
            if (accepted < 0) {
                line->hide();
                e->ignore();
            } else {
                int ins = accepted > from ? accepted + 1 : accepted;
                line->setGeometry(0, ins * rowH - 1, content->width(), 2);
                line->raise();
                line->show();
                e->acceptProposedAction();
            }
            return true;
        }
        if (ev->type() == QEvent::DragLeave) {
            line->hide();
            return false;
        }
        if (ev->type() == QEvent::Drop) {
            auto *e = static_cast<QDropEvent *>(ev);
            line->hide();
            int from = fromOf(e);
            if (from < 0) return false;
            int accepted = can(node, from, slotOf(e));
            if (accepted >= 0 && accepted != from) {
                commit(node, from, accepted);
                e->acceptProposedAction();
            } else {
                e->ignore();
            }
            return true;
        }
        return false;
    }
private:
    static int fromOf(QDropEvent *e) {
        QByteArray b = e->mimeData()->data(DAY_ROW_MIME);
        return b.isEmpty() ? -1 : b.toInt();
    }
    int slotOf(QDropEvent *e) const { return (int)(e->position().y()) / rowH; }
};

void day_qt_list_enable_reorder(void *content, uint64_t node, int row_h,
                                DayListCanMoveCb can, DayListMoveCb mv) {
    QWidget *w = static_cast<QWidget *>(content);
    w->setAcceptDrops(true);
    auto *f = new DayListDropFilter(node, row_h, can, mv, w);
    f->setParent(w); // freed with the widget
    w->installEventFilter(f);
}

void day_qt_cell_drag(void *cell, uint64_t node, int row) {
    QWidget *w = static_cast<QWidget *>(cell);
    auto *f = new DayCellDragFilter(node, row);
    f->setParent(w);
    w->installEventFilter(f);
}

// Paint (or clear) the selected-row treatment on an emulated list cell: the palette
// highlight fill with its matching text color, the plain QListView look.
void day_qt_cell_set_selected(void *w, int on) {
    QWidget *widget = static_cast<QWidget *>(w);
    widget->setAutoFillBackground(on != 0);
    if (on) {
        QPalette p = widget->palette();
        p.setColor(QPalette::Window, p.color(QPalette::Highlight));
        p.setColor(QPalette::WindowText, p.color(QPalette::HighlightedText));
        widget->setPalette(p);
    } else {
        widget->setPalette(QPalette());
    }
}

// --- focus (docs/focus.md) ---
// kind: 1 = gained, 0 = lost, 2 = submitted (line-edit return key).
typedef void (*DayFocusCb)(uint64_t node, int kind);

class DayFocusFilter : public QObject {
public:
    uint64_t node; DayFocusCb cb;
    DayFocusFilter(uint64_t n, DayFocusCb c) : node(n), cb(c) {}
protected:
    bool eventFilter(QObject *, QEvent *ev) override {
        if (ev->type() == QEvent::FocusIn) {
            cb(node, 1);
        } else if (ev->type() == QEvent::FocusOut) {
            // Popup grabs (menus, combo dropdowns) are transient — focus returns when they
            // close — so they are not reported as a loss.
            if (static_cast<QFocusEvent *>(ev)->reason() != Qt::PopupFocusReason) cb(node, 0);
        }
        return false; // observe only: never consume
    }
};

void day_qt_enable_focus(void *w, uint64_t node, DayFocusCb cb) {
    QWidget *widget = static_cast<QWidget *>(w);
    DayFocusFilter *f = new DayFocusFilter(node, cb);
    f->setParent(widget); // freed with the widget
    widget->installEventFilter(f);
    if (QLineEdit *e = qobject_cast<QLineEdit *>(widget))
        QObject::connect(e, &QLineEdit::returnPressed, [node, cb]() { cb(node, 2); });
}

// --- the arrow keys, for a widget that can hold focus (docs/menus.md) ---
// `code`: 0 left, 1 right, 2 up, 3 down; `modifiers` is a day KeyEvent mask (1 shift, 2
// primary, 4 alt). The callback answers whether the app CLAIMED the key — an unclaimed one
// falls through, so a scroll area around the widget still scrolls with the keyboard.
typedef int (*DayKeyCb)(uint64_t node, int code, int modifiers);

class DayKeyFilter : public QObject {
public:
    uint64_t node; DayKeyCb cb;
    DayKeyFilter(uint64_t n, DayKeyCb c) : node(n), cb(c) {}
protected:
    bool eventFilter(QObject *, QEvent *ev) override {
        if (ev->type() != QEvent::KeyPress) return false;
        QKeyEvent *ke = static_cast<QKeyEvent *>(ev);
        int code;
        switch (ke->key()) {
            case Qt::Key_Left:  code = 0; break;
            case Qt::Key_Right: code = 1; break;
            case Qt::Key_Up:    code = 2; break;
            case Qt::Key_Down:  code = 3; break;
            default: return false;
        }
        int mods = 0;
        if (ke->modifiers() & Qt::ShiftModifier) mods |= 1;
        if (ke->modifiers() & Qt::ControlModifier) mods |= 2;
        if (ke->modifiers() & Qt::AltModifier) mods |= 4;
        return cb(node, code, mods) != 0;
    }
};

// StrongFocus, because a canvas has no other way in: click or tab, the same as a line edit.
void day_qt_enable_keys(void *w, uint64_t node, DayKeyCb cb) {
    QWidget *widget = static_cast<QWidget *>(w);
    widget->setFocusPolicy(Qt::StrongFocus);
    DayKeyFilter *f = new DayKeyFilter(node, cb);
    f->setParent(widget); // freed with the widget
    widget->installEventFilter(f);
}

// Drive focus: request it, or clear it only while this widget still owns it (a stale
// release must not blur a sibling).
void day_qt_widget_focus(void *w, int focused) {
    QWidget *widget = static_cast<QWidget *>(w);
    if (focused) {
        // Qt only delivers focus events inside the ACTIVE window; an inactive one just
        // records the focus widget for later. Day's duty is window-local (AppKit's
        // makeFirstResponder, GTK's grab_focus) — so activate: politely via the OS, then
        // app-locally (the scripted/background-launch path, where the OS says no).
        QWidget *win = widget->window();
        if (win && !win->isActiveWindow()) {
            win->activateWindow();
            if (QApplication::activeWindow() != win) {
                QT_WARNING_PUSH
                QT_WARNING_DISABLE_DEPRECATED
                // Deprecated for user code (it bypasses the window manager) — that bypass is
                // the point here, and Qt keeps it for exactly this embedding/driving case.
                QApplication::setActiveWindow(win);
                QT_WARNING_POP
            }
        }
        widget->setFocus(Qt::OtherFocusReason);
    } else if (widget->hasFocus()) {
        widget->clearFocus();
    }
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
    auto *window = static_cast<DayWindow *>(win);
    QMenuBar *bar = window->menubar;
    if (!bar) {
        bar = new QMenuBar(window); // native global bar on macOS; top-of-window elsewhere
        bar->setNativeMenuBar(true);
        bar->show();
        window->menubar = bar;
    }
    bar->clear();
    return bar;
}

// Called by day-qt after the menus are populated: an in-window bar now has its real height,
// so reserve its strip and shrink day's content area under it.
void day_qt_window_menubar_done(void *win) {
    static_cast<DayWindow *>(win)->relayoutChrome();
}

void *day_qt_menubar_add_menu(void *bar, const char *label) {
    return static_cast<QMenuBar *>(bar)->addMenu(QString::fromUtf8(label));
}

// --- window toolbar (docs/toolbars.md) ---------------------------------------------------
// A real QToolBar under the menu bar, populated with QActions so the style decides icon size
// and whether labels show — the KDE convention, and what the user's Qt settings expect.
// Buttons ride the same g_menu_cb rail as menu items; values (toggle, search) go through
// g_toolbar_cb.

// kind 0 = toggle (`on`), kind 1 = search text (`text`).
static void (*g_toolbar_cb)(uint64_t, int, int, const char *) = nullptr;
void day_qt_set_toolbar_cb(void (*cb)(uint64_t, int, int, const char *)) { g_toolbar_cb = cb; }

// Item widgets by id, for the targeted patches (search text, toggle state, enabled).
// QPointer, not a raw pointer: a toolbar REBUILD destroys these widgets, and any patch that
// arrives between the destroy and the rebuild's re-add would otherwise `qobject_cast` a freed
// QObject — undefined behavior that showed up as a crash inside `deleteLater` when a search
// clear raced a re-install. A QPointer reads null once its object dies, so a stale patch is a
// no-op instead.
static std::map<std::string, QPointer<QWidget>> g_toolbar_widgets;
// A segmented item's exclusive button group, so a patch can move the selection without the
// echo — the group emits for the button going off as well as the one coming on.
static std::map<std::string, QPointer<QButtonGroup>> g_toolbar_groups;
static std::map<std::string, QAction *> g_toolbar_actions;

// The icon for a standard symbol: the freedesktop theme first (Linux, where KDE and GNOME
// both ship one), then the Qt style's own standard pixmap, which exists on every platform —
// so a macOS or Windows Qt build still gets real icons for the common commands.
static QIcon day_qt_toolbar_icon(const char *theme, int standard_pixmap) {
    QString spec = QString::fromUtf8(theme);
    // A symbol arrives as `theme-name|/path/to/outline.svg`: the platform's icon if the desktop
    // has one, and Day's own drawing of the symbol when it does not. The freedesktop names are
    // a GNOME/KDE fact, so off those desktops the theme lookup finds nothing and a toolbar item
    // would otherwise be label-only (docs/toolbars.md).
    QString outline;
    int bar = spec.indexOf(QLatin1Char('|'));
    if (bar >= 0) {
        outline = spec.mid(bar + 1);
        spec = spec.left(bar);
    }
    QString name = spec;
    // A BUNDLED image arrives here as a resolved file path (day-qt's `icon_args`), and it is a
    // template glyph: black on transparent. Loading it as-is drew a flat black star on the
    // toolbar, invisible in dark mode and wrong in light. Tint it to the palette text color,
    // exactly as the sidebar rows do — the file test is what tells a path from a theme NAME.
    if (!name.isEmpty() && QFileInfo(name).isFile()) {
        QColor fg = QApplication::palette().color(QPalette::WindowText);
        QIcon tinted = day_qt_tinted_icon(name, fg);
        if (!tinted.isNull()) return tinted;
    }
    if (!name.isEmpty()) {
        QIcon themed = QIcon::fromTheme(name);
        if (!themed.isNull()) return themed;
    }
    // Day's own drawing BEFORE QStyle's standard set. A themed icon above is the desktop's real
    // answer and wins outright; QStyle's is a small dialog-oriented set whose semantics often
    // miss — `SP_DialogApplyButton` stands in for a checkmark and draws nothing like one. The
    // outline at least has the right shape, and it is the shape every other backend falls back to.
    if (!outline.isEmpty() && QFileInfo(outline).isFile()) {
        QColor fg = QApplication::palette().color(QPalette::WindowText);
        QIcon own = day_qt_tinted_icon(outline, fg);
        if (!own.isNull()) return own;
    }
    if (standard_pixmap >= 0 && QApplication::style())
        return QApplication::style()->standardIcon(
            static_cast<QStyle::StandardPixmap>(standard_pixmap));
    return QIcon();
}

void *day_qt_window_toolbar(void *win) {
    auto *window = static_cast<DayWindow *>(win);
    QToolBar *bar = window->toolbar;
    if (!bar) {
        bar = new QToolBar(window);
        // No explicit tool-button style or icon size: inheriting them is what makes the bar
        // match the rest of the user's Qt desktop.
        bar->setMovable(false);
        bar->setFloatable(false);
        bar->show();
        window->toolbar = bar;
    }
    bar->clear();
    g_toolbar_widgets.clear();
    g_toolbar_groups.clear();
    g_toolbar_actions.clear();
    return bar;
}

void day_qt_window_toolbar_done(void *win) {
    auto *window = static_cast<DayWindow *>(win);
    if (window->toolbar) window->toolbar->setVisible(!window->toolbar->actions().isEmpty());
    window->relayoutChrome();
}

void day_qt_toolbar_add_action(void *bar, const char *id, const char *label, const char *theme,
                               int standard_pixmap, const char *tooltip, uint64_t action,
                               int enabled, int checkable, int checked) {
    auto *tb = static_cast<QToolBar *>(bar);
    QAction *a = tb->addAction(QString::fromUtf8(label));
    QIcon icon = day_qt_toolbar_icon(theme, standard_pixmap);
    if (!icon.isNull()) a->setIcon(icon);
    a->setToolTip(QString::fromUtf8(tooltip));
    a->setEnabled(enabled != 0);
    const uint64_t aid = action;
    if (checkable) {
        a->setCheckable(true);
        a->setChecked(checked != 0);
        QObject::connect(a, &QAction::toggled, [aid](bool on) {
            if (g_toolbar_cb) g_toolbar_cb(aid, 0, on ? 1 : 0, "");
        });
    } else if (aid) {
        QObject::connect(a, &QAction::triggered, [aid]() {
            if (g_menu_cb) g_menu_cb(aid);
        });
    }
    g_toolbar_actions[std::string(id)] = a;
}

// A segmented control: Qt has no such widget, so it is what Qt apps build — a row of checkable
// QToolButtons in an EXCLUSIVE QButtonGroup, hosted in one widget so the toolbar treats it as a
// single item. `titles` and `icons` are unit-separated lists, one entry per segment.
void day_qt_toolbar_add_segmented(void *bar, const char *id, const char *titles, const char *icons,
                                  int selected, uint64_t action, int enabled) {
    auto *tb = static_cast<QToolBar *>(bar);
    auto *host = new QWidget(tb);
    auto *lay = new QHBoxLayout(host);
    lay->setContentsMargins(0, 0, 0, 0);
    lay->setSpacing(0);
    auto *group = new QButtonGroup(host);
    group->setExclusive(true);
    const QStringList ts = QString::fromUtf8(titles).split(QChar(0x1f));
    const QStringList is = QString::fromUtf8(icons).split(QChar(0x1f));
    for (int i = 0; i < ts.size(); ++i) {
        auto *b = new QToolButton(host);
        b->setCheckable(true);
        QIcon ic = (i < is.size() && !is[i].isEmpty()) ? QIcon::fromTheme(is[i]) : QIcon();
        if (!ic.isNull()) {
            b->setIcon(ic);
        } else {
            b->setText(ts[i]);
        }
        b->setToolTip(ts[i]);
        b->setEnabled(enabled != 0);
        b->setChecked(i == selected);
        group->addButton(b, i);
        lay->addWidget(b);
    }
    const uint64_t aid = action;
    QObject::connect(group, &QButtonGroup::idToggled, [aid](int which, bool on) {
        // Only the segment coming ON is the choice; the one going off is its other half.
        if (on && g_toolbar_cb) g_toolbar_cb(aid, 2, which, "");
    });
    g_toolbar_widgets[std::string(id)] = host;
    g_toolbar_groups[std::string(id)] = group;
    tb->addWidget(host);
}

void day_qt_toolbar_set_selected(const char *id, int index) {
    auto it = g_toolbar_groups.find(std::string(id));
    if (it == g_toolbar_groups.end() || !it->second) return;
    QAbstractButton *b = it->second->button(index);
    if (!b || b->isChecked()) return;
    const bool blocked = it->second->blockSignals(true);
    b->setChecked(true);
    it->second->blockSignals(blocked);
}

// A pull-down: a QToolButton in InstantPopup mode, which is how Qt draws a menu button on a
// toolbar (with the little chevron). Returns the QMenu for day-qt to fill.
void *day_qt_toolbar_add_menu(void *bar, const char *id, const char *label, const char *theme,
                              int standard_pixmap, const char *tooltip, int enabled) {
    auto *tb = static_cast<QToolBar *>(bar);
    auto *button = new QToolButton(tb);
    auto *menu = new QMenu(button);
    button->setText(QString::fromUtf8(label));
    QIcon icon = day_qt_toolbar_icon(theme, standard_pixmap);
    if (!icon.isNull()) button->setIcon(icon);
    button->setToolTip(QString::fromUtf8(tooltip));
    button->setEnabled(enabled != 0);
    button->setMenu(menu);
    button->setPopupMode(QToolButton::InstantPopup);
    button->setToolButtonStyle(tb->toolButtonStyle());
    tb->addWidget(button);
    g_toolbar_widgets[std::string(id)] = button;
    return menu;
}

void day_qt_toolbar_add_search(void *bar, const char *id, const char *text,
                               const char *placeholder, uint64_t action, int enabled) {
    auto *tb = static_cast<QToolBar *>(bar);
    auto *edit = new QLineEdit(QString::fromUtf8(text), tb);
    edit->setPlaceholderText(QString::fromUtf8(placeholder));
    edit->setClearButtonEnabled(true);
    edit->addAction(QIcon::fromTheme(QStringLiteral("edit-find")), QLineEdit::LeadingPosition);
    edit->setEnabled(enabled != 0);
    edit->setMaximumWidth(240);
    const uint64_t aid = action;
    if (aid) {
        QObject::connect(edit, &QLineEdit::textChanged, [aid](const QString &s) {
            QByteArray ba = s.toUtf8();
            if (g_toolbar_cb) g_toolbar_cb(aid, 1, 0, ba.constData());
        });
    }
    tb->addWidget(edit);
    g_toolbar_widgets[std::string(id)] = edit;
}

// Completions for a toolbar search field (docs/search.md): Qt's own QCompleter, so the popup, the
// keyboard handling and the inline completion are the ones every Qt app has. An empty list drops
// the completer rather than leaving an empty popup armed.
void day_qt_toolbar_set_suggestions(const char *id, const char *joined) {
    auto it = g_toolbar_widgets.find(std::string(id));
    if (it == g_toolbar_widgets.end() || !it->second) return;
    auto *edit = qobject_cast<QLineEdit *>(it->second.data());
    if (!edit) return;
    QString all = QString::fromUtf8(joined);
    QStringList items = all.isEmpty() ? QStringList{} : all.split(QLatin1Char('\n'));

    // ONE completer and ONE model per field, for the life of the field: only the string list is
    // replaced. The previous version built a new QCompleter on every keystroke and `deleteLater`d
    // the one the QLineEdit was still wired to — and clearing the query took that branch while the
    // very same edit was being torn down, because emptying the search also un-filters the sidebar,
    // which re-lowers the toolbar and `bar->clear()`s the widget out from under the pending
    // deletion. The process died with no Qt diagnostic at all.
    //
    // Reusing the model removes the whole class of problem: nothing is destroyed on a patch, so
    // there is no lifetime to get wrong, and it stops allocating two objects per keystroke.
    auto *c = edit->completer();
    if (!c) {
        auto *model = new QStringListModel(edit);
        c = new QCompleter(model, edit);
        c->setCaseSensitivity(Qt::CaseInsensitive);
        c->setCompletionMode(QCompleter::PopupCompletion);
        edit->setCompleter(c);
    }
    if (auto *model = qobject_cast<QStringListModel *>(c->model())) {
        model->setStringList(items);
    }
}

void day_qt_toolbar_add_label(void *bar, const char *id, const char *text) {
    auto *tb = static_cast<QToolBar *>(bar);
    auto *label = new QLabel(QString::fromUtf8(text), tb);
    tb->addWidget(label);
    g_toolbar_widgets[std::string(id)] = label;
}

void day_qt_toolbar_add_separator(void *bar) { static_cast<QToolBar *>(bar)->addSeparator(); }

// `expand` != 0 makes the spacer absorb the leftover width, which is how the model's flexible
// space pushes everything after it to the trailing edge.
void day_qt_toolbar_add_space(void *bar, int expand) {
    auto *tb = static_cast<QToolBar *>(bar);
    auto *spacer = new QWidget(tb);
    if (expand)
        spacer->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
    else
        spacer->setFixedWidth(12);
    tb->addWidget(spacer);
}

void day_qt_toolbar_set_text(const char *id, const char *text) {
    auto it = g_toolbar_widgets.find(std::string(id));
    if (it == g_toolbar_widgets.end() || !it->second) return;
    if (auto *edit = qobject_cast<QLineEdit *>(it->second.data())) {
        QString next = QString::fromUtf8(text);
        if (edit->text() == next) return;
        // textChanged fires on a programmatic set too, which would echo into the signal.
        const bool blocked = edit->blockSignals(true);
        edit->setText(next);
        edit->blockSignals(blocked);
    }
}

void day_qt_toolbar_set_checked(const char *id, int on) {
    auto it = g_toolbar_actions.find(std::string(id));
    if (it == g_toolbar_actions.end() || !it->second->isCheckable()) return;
    if (it->second->isChecked() == (on != 0)) return;
    const bool blocked = it->second->blockSignals(true);
    it->second->setChecked(on != 0);
    it->second->blockSignals(blocked);
}

void day_qt_toolbar_set_enabled(const char *id, int on) {
    auto a = g_toolbar_actions.find(std::string(id));
    if (a != g_toolbar_actions.end()) a->second->setEnabled(on != 0);
    auto w = g_toolbar_widgets.find(std::string(id));
    if (w != g_toolbar_widgets.end() && w->second) w->second->setEnabled(on != 0);
}

void *day_qt_menu_new() { return new QMenu(); }

void *day_qt_menu_add_submenu(void *menu, const char *label) {
    return static_cast<QMenu *>(menu)->addMenu(QString::fromUtf8(label));
}

void day_qt_menu_add_separator(void *menu) {
    static_cast<QMenu *>(menu)->addSeparator();
}

void day_qt_menu_add_action(void *menu, const char *label, uint64_t id,
                            const char *shortcut, int enabled, const char *icon,
                            int icon_fallback) {
    QAction *a = static_cast<QMenu *>(menu)->addAction(QString::fromUtf8(label));
    if (shortcut && *shortcut) a->setShortcut(QKeySequence(QString::fromUtf8(shortcut)));
    a->setEnabled(enabled != 0);
    // The item's glyph, resolved exactly like a toolbar item's (docs/menus.md): theme name,
    // Day's own outline, then the QStyle standard set.
    if ((icon && *icon) || icon_fallback >= 0) {
        QIcon ic = day_qt_toolbar_icon(icon ? icon : "", icon_fallback);
        if (!ic.isNull()) a->setIcon(ic);
    }
    uint64_t aid = id;
    QObject::connect(a, &QAction::triggered, [aid]() {
        if (g_menu_cb) g_menu_cb(aid);
    });
}

// The top-level window a window-scoped menu role (Close/Minimize/Fullscreen) targets.
// Layered because each Qt-side answer goes stale in a state the macOS global menu bar can
// reach: focusWidget() is null whenever no child widget holds keyboard focus, and after a
// secondary window closes (hides) and the app is re-activated from outside, activeWindow()
// AND focusWidget() are BOTH null while AppKit considers the primary key. So: Qt's answer
// if it names a visible window, else the platform's key window mapped back to its widget,
// else — when exactly one candidate remains — that window. Never a hidden one: close() on
// an already-hidden window is a silent no-op, which is how "File ▸ Close does nothing"
// looked to the user.
static QWidget *day_qt_role_target() {
    if (QWidget *w = QApplication::activeWindow()) {
        if (w->isVisible()) return w;
    }
    if (QWidget *f = QApplication::focusWidget()) {
        QWidget *t = f->window();
        if (t->isVisible()) return t;
    }
    if (QWindow *fw = QGuiApplication::focusWindow()) {
        for (QWidget *tl : QApplication::topLevelWidgets())
            if (tl->windowHandle() == fw && tl->isVisible()) return tl;
    }
    QWidget *only = nullptr;
    for (QWidget *tl : QApplication::topLevelWidgets()) {
        if (!tl->isVisible() || tl->windowType() != Qt::Window) continue;
        if (only) return nullptr; // two candidates and no focus info — refuse to guess
        only = tl;
    }
    return only;
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
                if (QWidget *w = day_qt_role_target()) w->showMinimized();
            });
            break;
        case 11:
            QObject::connect(a, &QAction::triggered, []() {
                if (QWidget *w = day_qt_role_target()) w->close();
            });
            break;
        case 12:
            QObject::connect(a, &QAction::triggered, []() {
                if (QWidget *w = day_qt_role_target()) {
                    if (w->isFullScreen()) w->showNormal();
                    else w->showFullScreen();
                }
            });
            break;
        default: break;
    }
}

// Attach `menu` as `widget`'s context menu (secondary-click / long-press). A null menu clears it.
// Per-row context menus for the nav list (docs/menus.md): `menus` is a parallel array of
// QMenu* (null = no menu for that row). Menus are reparented to the list (freed with it /
// on the next set), and a custom-context request maps the click to its row's menu.
void day_qt_navlist_set_row_menus(void *w, void *const *menus, int32_t n) {
    auto *l = qobject_cast<QListWidget *>(static_cast<QWidget *>(w));
    if (!l) return;
    // Drop the previous set (tracked by object name, the day_ctx_menu pattern).
    for (QMenu *old : l->findChildren<QMenu *>(QStringLiteral("day_nav_row_menu"),
                                               Qt::FindDirectChildrenOnly)) {
        old->setObjectName(QString());
        old->deleteLater();
    }
    QObject::disconnect(l, &QWidget::customContextMenuRequested, nullptr, nullptr);
    auto *rows = new QVector<QMenu *>();
    for (int32_t i = 0; i < n; ++i) {
        QMenu *m = static_cast<QMenu *>(menus[i]);
        if (m) {
            m->setObjectName(QStringLiteral("day_nav_row_menu"));
            // Reparent KEEPING the window flags. A QMenu is a `Qt::Popup` top-level; the
            // one-argument `setParent` clears that, turning it into an ordinary child widget of
            // the list — which Qt then shows with its parent, drawing the menu's entries inline
            // over the top-left of the sidebar. The list is only its owner for lifetime, never
            // its layout parent.
            m->setParent(l, m->windowFlags());
            // Belt and braces: a child that has never been explicitly hidden is shown with its
            // parent, so say it once here.
            m->hide();
        }
        rows->append(m);
    }
    l->setContextMenuPolicy(Qt::CustomContextMenu);
    QObject::connect(l, &QWidget::customContextMenuRequested, l, [l, rows](const QPoint &pos) {
        QListWidgetItem *item = l->itemAt(pos);
        if (!item) return;
        int row = l->row(item);
        if (row >= 0 && row < rows->size() && (*rows)[row])
            (*rows)[row]->popup(l->mapToGlobal(pos));
    });
    // The vector dies with the connection's context object (the list) — leak-free enough for
    // a widget that lives as long as the sidebar; the menus themselves are children of `l`.
    QObject::connect(l, &QObject::destroyed, [rows]() { delete rows; });
}

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
