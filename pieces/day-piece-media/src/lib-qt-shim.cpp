// The media piece's own Qt shim behind a flat C ABI. When Qt6MultimediaWidgets is available
// (build.rs probes pkg-config and defines DAY_MEDIA_QT_MM) this wraps QMediaPlayer + QAudioOutput +
// QVideoWidget. When it is NOT — some minimal Qt installs — it degrades to a QLabel showing the
// URL, so the app still builds/launches/screenshots (mirrors day-piece-webview's MSYS2 degrade).
// The C ABI is identical either way, so lib-qt.rs is unchanged. Note QVideoWidget ships no
// transport chrome — the piece's `.controls` flag is a no-op on Qt; playback is driven through
// day_media_play/pause (the front-end's triggers). `Load` also starts playback, matching the other
// backends.

#include <QUrl>
#include <QVBoxLayout>
#include <QWidget>

#ifdef DAY_MEDIA_QT_MM

#include <QAudioOutput>
#include <QMediaPlayer>
#include <QVideoWidget>

class DayMedia : public QWidget {
public:
    QMediaPlayer *player = nullptr;
    void load(const QString &url) {
        if (player && !url.isEmpty())
            player->setSource(QUrl::fromUserInput(url)); // handles file paths AND http(s) URLs
    }
};

extern "C" {

void *day_media_new(const char *url, int autoplay, int looping, int muted) {
    DayMedia *w = new DayMedia();
    QVBoxLayout *lay = new QVBoxLayout(w);
    lay->setContentsMargins(0, 0, 0, 0);
    QMediaPlayer *player = new QMediaPlayer(w);
    QAudioOutput *audio = new QAudioOutput(w);
    audio->setMuted(muted != 0);
    player->setAudioOutput(audio);
    QVideoWidget *video = new QVideoWidget();
    player->setVideoOutput(video);
    if (looping != 0)
        player->setLoops(QMediaPlayer::Infinite);
    lay->addWidget(video);
    w->player = player;
    w->load(QString::fromUtf8(url));
    if (autoplay != 0)
        player->play();
    return w;
}

void day_media_load(void *w, const char *url) {
    DayMedia *m = static_cast<DayMedia *>(w);
    m->load(QString::fromUtf8(url));
    if (m->player)
        m->player->play();
}
void day_media_play(void *w) {
    if (QMediaPlayer *p = static_cast<DayMedia *>(w)->player)
        p->play();
}
void day_media_pause(void *w) {
    if (QMediaPlayer *p = static_cast<DayMedia *>(w)->player)
        p->pause();
}

} // extern "C"

#else // no Qt6MultimediaWidgets — degrade to a URL label (QtWidgets only, already linked by day-qt-sys)

#include <QLabel>

class DayMedia : public QWidget {
public:
    QLabel *label = nullptr;
    void load(const QString &url) {
        if (label)
            label->setText(url);
    }
};

extern "C" {

void *day_media_new(const char *url, int autoplay, int looping, int muted) {
    (void)autoplay;
    (void)looping;
    (void)muted; // nothing to play without a media engine
    DayMedia *w = new DayMedia();
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

void day_media_load(void *w, const char *url) {
    static_cast<DayMedia *>(w)->load(QString::fromUtf8(url));
}
void day_media_play(void *) {}
void day_media_pause(void *) {}

} // extern "C"

#endif
