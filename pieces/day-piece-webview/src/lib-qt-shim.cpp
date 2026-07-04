// The web-view piece's own Qt shim behind a flat C ABI. When Qt6WebEngineWidgets is available
// (build.rs probes pkg-config and defines DAY_WEBVIEW_QT_ENGINE) this wraps a real QWebEngineView
// and forwards `urlChanged` to a C callback so a bound text field follows navigation. When it is
// NOT — e.g. MSYS2/MINGW64, which does not package Qt6 WebEngine (Chromium won't build with MinGW
// GCC) — it degrades to a QLabel showing the URL, so windows-qt still builds/launches/screenshots
// (mirrors day-piece-webview's winui EdgeHTML degrade). The C ABI is identical either way, so
// lib-qt.rs is unchanged. The callback's `const char*` is only valid for the call (Rust copies it).

#include <QUrl>
#include <QVBoxLayout>
#include <QWidget>

#include <cstdint>

#ifdef DAY_WEBVIEW_QT_ENGINE

#include <QWebEngineView>

class DayWebView : public QWidget {
public:
    QWebEngineView *view = nullptr;
    void load(const QString &url) {
        if (view && !url.isEmpty())
            view->load(QUrl::fromUserInput(url));
    }
};

extern "C" {

void *day_webview_new(const char *url, uint64_t id, void (*cb)(uint64_t, const char *)) {
    DayWebView *w = new DayWebView();
    QVBoxLayout *lay = new QVBoxLayout(w);
    lay->setContentsMargins(0, 0, 0, 0);
    QWebEngineView *v = new QWebEngineView();
    QObject::connect(v, &QWebEngineView::urlChanged, [id, cb](const QUrl &u) {
        QByteArray bytes = u.toString().toUtf8();
        cb(id, bytes.constData());
    });
    lay->addWidget(v);
    w->view = v;
    w->load(QString::fromUtf8(url));
    return w;
}

void day_webview_load(void *w, const char *url) {
    static_cast<DayWebView *>(w)->load(QString::fromUtf8(url));
}
void day_webview_back(void *w) {
    if (QWebEngineView *v = static_cast<DayWebView *>(w)->view)
        v->back();
}
void day_webview_forward(void *w) {
    if (QWebEngineView *v = static_cast<DayWebView *>(w)->view)
        v->forward();
}
void day_webview_stop(void *w) {
    if (QWebEngineView *v = static_cast<DayWebView *>(w)->view)
        v->stop();
}
void day_webview_reload(void *w) {
    if (QWebEngineView *v = static_cast<DayWebView *>(w)->view)
        v->reload();
}

} // extern "C"

#else // no Qt6WebEngineWidgets — degrade to a URL label (QtWidgets only, already linked by day-qt-sys)

#include <QLabel>

class DayWebView : public QWidget {
public:
    QLabel *label = nullptr;
    void load(const QString &url) {
        if (label)
            label->setText(url);
    }
};

extern "C" {

void *day_webview_new(const char *url, uint64_t id, void (*cb)(uint64_t, const char *)) {
    (void)id;
    (void)cb; // no navigation to report without a real engine
    DayWebView *w = new DayWebView();
    QVBoxLayout *lay = new QVBoxLayout(w);
    lay->setContentsMargins(0, 0, 0, 0);
    QLabel *l = new QLabel();
    l->setText(QString::fromUtf8(url));
    l->setAlignment(Qt::AlignTop | Qt::AlignLeft);
    l->setTextInteractionFlags(Qt::TextSelectableByMouse);
    lay->addWidget(l);
    w->label = l;
    return w;
}

void day_webview_load(void *w, const char *url) {
    static_cast<DayWebView *>(w)->load(QString::fromUtf8(url));
}
void day_webview_back(void *) {}
void day_webview_forward(void *) {}
void day_webview_stop(void *) {}
void day_webview_reload(void *) {}

} // extern "C"

#endif
