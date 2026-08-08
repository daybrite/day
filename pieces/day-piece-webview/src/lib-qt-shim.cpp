// The web-view piece's own Qt shim behind a flat C ABI. When Qt6WebEngineWidgets is available
// (build.rs probes pkg-config and defines DAY_WEBVIEW_QT_ENGINE) this wraps a real QWebEngineView
// and forwards `urlChanged` to a C callback so a bound text field follows navigation. When it is
// NOT — e.g. MSYS2/MINGW64, which does not package Qt6 WebEngine (Chromium won't build with MinGW
// GCC) — it degrades to a QLabel showing the URL, so windows-qt still builds/launches/screenshots
// (mirrors day-piece-webview's xaml EdgeHTML degrade). The C ABI is identical either way, so
// lib-qt.rs is unchanged. The callback's `const char*` is only valid for the call (Rust copies it).

#include <QUrl>
#include <QVBoxLayout>
#include <QWidget>

#include <cstdint>
#include <map>
#include <string>

// A JavaScript-evaluation reply, in the 0x1F-separated form the Rust front-end decodes
// (docs/webview-eval.md). Only used for ENGINE failures — a script that merely throws is caught by
// the front-end's JS wrapper and arrives as an ordinary result string.
static std::string day_webview_eval_error(const char *msg) {
    std::string s = "0";
    s.push_back('\x1f');
    s += "QtWebEngine";
    s.push_back('\x1f');
    s += msg;
    return s;
}

// Set once from Rust. Qt has NO error channel on runJavaScript — a throw, a syntax error and a
// genuine null all arrive as the same invalid QVariant — which is why the front-end wraps every
// script in try/catch before it gets here.
static void (*g_eval_cb)(uint64_t, uint64_t, const char *) = nullptr;

#ifdef DAY_WEBVIEW_QT_ENGINE

#include <QWebEnginePage>
#include <QWebEngineView>

// Session id -> the retained engine view. Qt is the one backend whose `release` DELETES the handle
// (day_qt_delete -> deleteLater), so a second pointer to the container would dangle. What is
// retained instead is the QWebEngineView INSIDE it: ~DayWebView re-parents it out before ~QWidget
// deletes its children, and the next container adopts it. The page, history and JS context live in
// the QWebEnginePage the view owns, so re-parenting is lossless.
static std::map<uint64_t, QWebEngineView *> g_sessions;

class DayWebView : public QWidget {
public:
    QWebEngineView *view = nullptr;
    uint64_t id = 0;
    uint64_t session = 0;
    ~DayWebView() override {
        // Runs BEFORE ~QWidget deletes children — the only window in which the engine view can be
        // rescued from the container's destruction.
        if (session != 0 && view)
            view->setParent(nullptr);
    }
    void load(const QString &url) {
        if (view && !url.isEmpty())
            view->load(QUrl::fromUserInput(url));
    }
};

// (Re)point a view's navigation reports at the node currently showing it. A retained view outlives
// the node that first realized it, so the old connection would report to a torn-down node.
static void day_webview_connect_url(QWebEngineView *v, uint64_t id,
                                    void (*cb)(uint64_t, const char *)) {
    QObject::disconnect(v, &QWebEngineView::urlChanged, nullptr, nullptr);
    QObject::connect(v, &QWebEngineView::urlChanged, [id, cb](const QUrl &u) {
        QByteArray bytes = u.toString().toUtf8();
        cb(id, bytes.constData());
    });
}

extern "C" {

void *day_webview_new(const char *url, uint64_t id, void (*cb)(uint64_t, const char *),
                      uint64_t session) {
    DayWebView *w = new DayWebView();
    w->id = id;
    w->session = session;
    QVBoxLayout *lay = new QVBoxLayout(w);
    lay->setContentsMargins(0, 0, 0, 0);

    auto known = session != 0 ? g_sessions.find(session) : g_sessions.end();
    if (known != g_sessions.end()) {
        // Re-attach the retained engine: addWidget re-parents it. Deliberately NO load() — the
        // point is to return to the page as it was left.
        QWebEngineView *v = known->second;
        day_webview_connect_url(v, id, cb);
        lay->addWidget(v);
        w->view = v;
        return w;
    }

    QWebEngineView *v = new QWebEngineView();
    day_webview_connect_url(v, id, cb);
    lay->addWidget(v);
    w->view = v;
    if (session != 0)
        g_sessions[session] = v;
    w->load(QString::fromUtf8(url));
    return w;
}

void day_webview_set_eval_cb(void (*cb)(uint64_t, uint64_t, const char *)) { g_eval_cb = cb; }

void day_webview_eval(void *w, uint64_t req, const char *script) {
    DayWebView *self = static_cast<DayWebView *>(w);
    const uint64_t id = self->id;
    if (!self->view) {
        if (g_eval_cb) {
            const std::string e = day_webview_eval_error("no engine");
            g_eval_cb(id, req, e.c_str());
        }
        return;
    }
    // The main world (the default), matching WKWebView's `evaluateJavaScript`, so a script sees the
    // page's own globals on every backend alike.
    //
    // Capture PODs ONLY. Qt guarantees this callback runs even while the page is being destroyed,
    // and touching the page or view from there is undefined behaviour — the guarantee is what keeps
    // the Rust future from leaking, and the constraint is the price of it.
    self->view->page()->runJavaScript(QString::fromUtf8(script), [id, req](const QVariant &v) {
        if (!g_eval_cb)
            return;
        if (!v.isValid()) {
            const std::string e = day_webview_eval_error("no result (page discarded)");
            g_eval_cb(id, req, e.c_str());
            return;
        }
        const QByteArray bytes = v.toString().toUtf8();
        g_eval_cb(id, req, bytes.constData());
    });
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
    uint64_t id = 0;
    void load(const QString &url) {
        if (label)
            label->setText(url);
    }
};

extern "C" {

void *day_webview_new(const char *url, uint64_t id, void (*cb)(uint64_t, const char *),
                      uint64_t session) {
    (void)cb;      // no navigation to report without a real engine
    (void)session; // nothing to retain either
    DayWebView *w = new DayWebView();
    w->id = id;
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

void day_webview_set_eval_cb(void (*cb)(uint64_t, uint64_t, const char *)) { g_eval_cb = cb; }

// No engine to evaluate in, but the reply is still mandatory: the Rust future is only resolved by a
// callback, so staying silent here would hang the caller forever on windows-qt.
void day_webview_eval(void *w, uint64_t req, const char *script) {
    (void)script;
    if (g_eval_cb) {
        const std::string e = day_webview_eval_error("no Qt6 WebEngine in this build");
        g_eval_cb(static_cast<DayWebView *>(w)->id, req, e.c_str());
    }
}

} // extern "C"

#endif
