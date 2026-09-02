// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The media piece's own Qt shim behind a flat C ABI. When Qt6MultimediaWidgets is available
// (build.rs probes pkg-config and defines DAY_MEDIA_QT_MM) this wraps QMediaPlayer + QAudioOutput
// (+ a QVideoWidget unless the player is sound-only). When it is NOT — some minimal Qt installs —
// it degrades to a QLabel showing the URL, so the app still builds/launches/screenshots (mirrors
// day-piece-webview's MSYS2 degrade). The C ABI is identical either way, so lib-qt.rs is
// unchanged. Note QVideoWidget ships no transport chrome — the piece's `.controls` flag is a
// no-op on Qt; playback is driven through day_media_play/pause/stop (the front-end's triggers).
// `Load` also starts playback, matching the other backends.
//
// Playback state goes back through one file-static callback (day_media_set_state_cb), fed by the
// player's playbackStateChanged / mediaStatusChanged / errorOccurred signals and reported as the
// piece's own codes: 0 idle, 1 loading, 2 playing, 3 paused, 4 ended, 5 error.

#include <QUrl>
#include <QVBoxLayout>
#include <QWidget>

#include <cstdint>

typedef void (*DayMediaStateCb)(uint64_t, int, const char *);
static DayMediaStateCb g_state_cb = nullptr;

extern "C" void day_media_set_state_cb(DayMediaStateCb cb) { g_state_cb = cb; }

#ifdef DAY_MEDIA_QT_MM

#include <QAudioOutput>
#include <QMediaPlayer>
#include <QVideoWidget>

class DayMedia : public QWidget {
public:
    uint64_t id = 0;
    bool audioOnly = false;
    QMediaPlayer *player = nullptr;
    QAudioOutput *audio = nullptr;

    void load(const QString &url) {
        if (player && !url.isEmpty())
            player->setSource(QUrl::fromUserInput(url)); // handles file paths AND http(s) URLs
    }

    void report(int code, const QString &text = QString()) {
        if (!g_state_cb)
            return;
        const QByteArray bytes = text.toUtf8();
        g_state_cb(id, code, bytes.constData());
    }

    // Both status enums move on a load, so the state is re-derived from both rather than from
    // whichever signal fired.
    void reportState() {
        if (!player)
            return;
        if (player->source().isEmpty()) {
            report(0);
            return;
        }
        if (player->error() != QMediaPlayer::NoError) {
            report(5, player->errorString());
            return;
        }
        switch (player->mediaStatus()) {
        case QMediaPlayer::EndOfMedia:
            report(4);
            return;
        case QMediaPlayer::InvalidMedia:
            report(5, player->errorString().isEmpty() ? QStringLiteral("invalid media")
                                                       : player->errorString());
            return;
        case QMediaPlayer::LoadingMedia:
        case QMediaPlayer::StalledMedia:
        case QMediaPlayer::BufferingMedia:
            report(1);
            return;
        default:
            break;
        }
        switch (player->playbackState()) {
        case QMediaPlayer::PlayingState:
            report(2);
            return;
        case QMediaPlayer::PausedState:
            report(3);
            return;
        default:
            // Stopped with a source still set: the player is idle but loaded.
            report(player->mediaStatus() == QMediaPlayer::NoMedia ? 0 : 3);
            return;
        }
    }
};

extern "C" {

void *day_media_new(uint64_t id, const char *url, int autoplay, int looping, int muted,
                    int audio_only, double volume) {
    DayMedia *w = new DayMedia();
    w->id = id;
    w->audioOnly = audio_only != 0;
    QVBoxLayout *lay = new QVBoxLayout(w);
    lay->setContentsMargins(0, 0, 0, 0);
    QMediaPlayer *player = new QMediaPlayer(w);
    QAudioOutput *audio = new QAudioOutput(w);
    audio->setMuted(muted != 0);
    audio->setVolume(static_cast<float>(volume));
    player->setAudioOutput(audio);
    if (!w->audioOnly) {
        QVideoWidget *video = new QVideoWidget();
        player->setVideoOutput(video);
        lay->addWidget(video);
    } else {
        w->hide();
    }
    if (looping != 0)
        player->setLoops(QMediaPlayer::Infinite);
    w->player = player;
    w->audio = audio;
    QObject::connect(player, &QMediaPlayer::playbackStateChanged, w, [w]() { w->reportState(); });
    QObject::connect(player, &QMediaPlayer::mediaStatusChanged, w, [w]() { w->reportState(); });
    QObject::connect(player, &QMediaPlayer::errorOccurred, w,
                     [w](QMediaPlayer::Error, const QString &text) { w->report(5, text); });
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
// Dropping the source is what lets a live stream's connection go; the status signal reports the
// resulting idle.
void day_media_stop(void *w) {
    DayMedia *m = static_cast<DayMedia *>(w);
    if (m->player) {
        m->player->stop();
        m->player->setSource(QUrl());
    }
    m->report(0);
}
void day_media_set_volume(void *w, double volume) {
    if (QAudioOutput *a = static_cast<DayMedia *>(w)->audio)
        a->setVolume(static_cast<float>(volume));
}
int day_media_is_audio_only(void *w) { return static_cast<DayMedia *>(w)->audioOnly ? 1 : 0; }

} // extern "C"

#else // no Qt6MultimediaWidgets — degrade to a URL label (QtWidgets only, already linked by day-qt-sys)

#include <QLabel>

class DayMedia : public QWidget {
public:
    uint64_t id = 0;
    bool audioOnly = false;
    QLabel *label = nullptr;
    void load(const QString &url) {
        if (label)
            label->setText(url);
    }
};

extern "C" {

void *day_media_new(uint64_t id, const char *url, int autoplay, int looping, int muted,
                    int audio_only, double volume) {
    (void)autoplay;
    (void)looping;
    (void)muted;
    (void)volume; // nothing to play without a media engine
    DayMedia *w = new DayMedia();
    w->id = id;
    w->audioOnly = audio_only != 0;
    QVBoxLayout *lay = new QVBoxLayout(w);
    lay->setContentsMargins(0, 0, 0, 0);
    QLabel *l = new QLabel();
    l->setText(QString::fromUtf8(url));
    l->setAlignment(Qt::AlignTop | Qt::AlignLeft);
    l->setTextInteractionFlags(Qt::TextSelectableByMouse);
    lay->addWidget(l);
    w->label = l;
    if (w->audioOnly)
        w->hide();
    // Say so on the state channel rather than staying silent: an app drawing a transport from
    // the state would otherwise wait forever for a player that does not exist.
    if (g_state_cb)
        g_state_cb(id, 5, "no media engine (Qt6MultimediaWidgets is not installed)");
    return w;
}

void day_media_load(void *w, const char *url) {
    static_cast<DayMedia *>(w)->load(QString::fromUtf8(url));
    if (g_state_cb)
        g_state_cb(static_cast<DayMedia *>(w)->id, 5,
                   "no media engine (Qt6MultimediaWidgets is not installed)");
}
void day_media_play(void *) {}
void day_media_pause(void *) {}
void day_media_stop(void *) {}
void day_media_set_volume(void *, double) {}
int day_media_is_audio_only(void *w) { return static_cast<DayMedia *>(w)->audioOnly ? 1 : 0; }

} // extern "C"

#endif
