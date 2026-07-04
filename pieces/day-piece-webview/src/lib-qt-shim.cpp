// The web-view piece's own Qt shim: a QWebEngineView wrapped in a QWidget, behind a flat C ABI.
// `urlChanged` is forwarded to a C callback so a bound text field follows navigation. The callback's
// `const char*` is only valid for the duration of the call (the Rust side copies it immediately).

#include <QUrl>
#include <QVBoxLayout>
#include <QWebEngineView>
#include <QWidget>

#include <cstdint>

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
