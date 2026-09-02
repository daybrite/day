// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The day-piece-media crate's OWN Android backend — bundled here and folded into the app's Gradle
// build via [package.metadata.day.android], with ZERO edits to day-android. It uses only
// day-android's PUBLIC Java surface: DayBridge.ctx (the Context) and DayBridge.nativeOnEvent (the
// event trampoline). android.widget.VideoView + MediaController and android.media.MediaPlayer are
// framework classes, so the piece adds no Gradle dependencies; it declares the INTERNET permission
// in Cargo.toml for network sources, which `day build` merges into the app manifest. See
// docs/extending.md.
package dev.daybrite.day.piece.media;

import android.media.AudioAttributes;
import android.media.MediaPlayer;
import android.net.Uri;
import android.os.Handler;
import android.os.Looper;
import android.view.View;
import android.widget.MediaController;
import android.widget.VideoView;

import java.io.IOException;
import java.util.Map;
import java.util.WeakHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

import dev.daybrite.day.bridge.DayBridge;

/**
 * Wraps android.widget.VideoView (with optional MediaController chrome) for pictures, or a bare
 * MediaPlayer behind an empty View for sound only. Playback state is reported to the piece's Rust
 * front-end through DayBridge.nativeOnEvent's Custom kind (12) as the piece's own codes.
 */
public final class DayMedia {
    private DayMedia() {}

    private static final int K_CUSTOM = 12;
    private static final int IDLE = 0, LOADING = 1, PLAYING = 2, PAUSED = 3, ENDED = 4, ERROR = 5;

    /** Everything a live player carries: the node it reports to and its MediaPlayer. */
    private static final class Live {
        final long id;
        final boolean audioOnly;
        final boolean looping;
        final boolean muted;
        float volume;
        /** The sound-only player; a VideoView owns its own MediaPlayer and this stays null. */
        MediaPlayer player;
        /** Whether the current source has finished preparing (so a play is a real start). */
        boolean prepared;
        /** Whether the app asked to play before the source was ready. */
        boolean playWhenReady;

        Live(long id, boolean audioOnly, boolean looping, boolean muted, float volume) {
            this.id = id;
            this.audioOnly = audioOnly;
            this.looping = looping;
            this.muted = muted;
            this.volume = volume;
        }

        void report(int code, String text) {
            DayBridge.nativeOnEvent(id, K_CUSTOM, (double) code, text == null ? "" : text);
        }

        void applyVolume(MediaPlayer mp) {
            float v = muted ? 0f : volume;
            mp.setVolume(v, v);
        }
    }

    // Weak keys: a released view must not pin itself here.
    private static final Map<View, Live> LIVE = new WeakHashMap<>();

    /** Where a source is opened and an old player torn down. `setDataSource` on an http URI
     *  resolves and connects BEFORE returning, and `reset`/`release` wait on the same socket:
     *  on the UI thread either one is an ANR on a slow network. The player itself is created
     *  on the UI thread, so its listeners still run there. */
    private static final ExecutorService IO = Executors.newSingleThreadExecutor();
    private static final Handler MAIN = new Handler(Looper.getMainLooper());

    private static String errorText(int what, int extra) {
        String base;
        switch (what) {
            case MediaPlayer.MEDIA_ERROR_SERVER_DIED:
                base = "the media server died";
                break;
            default:
                base = "playback failed";
                break;
        }
        switch (extra) {
            case MediaPlayer.MEDIA_ERROR_IO:
                return "network error reaching the stream";
            case MediaPlayer.MEDIA_ERROR_MALFORMED:
                return "stream is corrupt or unreadable";
            case MediaPlayer.MEDIA_ERROR_UNSUPPORTED:
                return "stream format not supported";
            case MediaPlayer.MEDIA_ERROR_TIMED_OUT:
                return "the stream timed out";
            default:
                return base + " (" + what + "/" + extra + ")";
        }
    }

    public static View makeMedia(
            long id, String url, boolean autoplay, boolean looping, boolean muted,
            boolean controls, boolean audioOnly, double volume) {
        final Live live = new Live(id, audioOnly, looping, muted, (float) volume);
        if (audioOnly) {
            View host = new View(DayBridge.ctx);
            host.setVisibility(View.GONE);
            LIVE.put(host, live);
            if (url != null && !url.isEmpty()) {
                loadAudio(live, url, autoplay);
            }
            return host;
        }
        VideoView video = new VideoView(DayBridge.ctx);
        LIVE.put(video, live);
        if (controls) {
            MediaController mc = new MediaController(DayBridge.ctx);
            mc.setAnchorView(video);
            video.setMediaController(mc);
        }
        // looping/muted/volume live on the underlying MediaPlayer, only reachable once prepared.
        // The listener re-fires for every setVideoURI (mediaCommand's load), keeping them sticky.
        video.setOnPreparedListener(new MediaPlayer.OnPreparedListener() {
            @Override
            public void onPrepared(MediaPlayer mp) {
                mp.setLooping(live.looping);
                live.applyVolume(mp);
                live.prepared = true;
                mp.setOnInfoListener(new MediaPlayer.OnInfoListener() {
                    @Override
                    public boolean onInfo(MediaPlayer p, int what, int extra) {
                        if (what == MediaPlayer.MEDIA_INFO_BUFFERING_START) {
                            live.report(LOADING, "");
                        } else if (what == MediaPlayer.MEDIA_INFO_BUFFERING_END) {
                            live.report(p.isPlaying() ? PLAYING : PAUSED, "");
                        }
                        return false;
                    }
                });
                live.report(mp.isPlaying() ? PLAYING : PAUSED, "");
            }
        });
        video.setOnCompletionListener(new MediaPlayer.OnCompletionListener() {
            @Override
            public void onCompletion(MediaPlayer mp) {
                if (!live.looping) {
                    live.report(ENDED, "");
                }
            }
        });
        video.setOnErrorListener(new MediaPlayer.OnErrorListener() {
            @Override
            public boolean onError(MediaPlayer mp, int what, int extra) {
                live.report(ERROR, errorText(what, extra));
                return true; // handled: no system "Can't play this video" dialog
            }
        });
        if (url != null && !url.isEmpty()) {
            // Uri.parse handles file paths AND http(s)/content URIs (setVideoPath is the same call).
            live.report(LOADING, "");
            video.setVideoURI(Uri.parse(url));
            if (autoplay) {
                video.start();
            }
        }
        return video;
    }

    /** Build (or rebuild) the sound-only MediaPlayer for `url`. Asynchronous: a stream prepares
     *  off the UI thread and reports its state as it goes. */
    private static void loadAudio(final Live live, String url, boolean playWhenReady) {
        releasePlayer(live);
        final MediaPlayer mp = new MediaPlayer();
        live.player = mp;
        live.prepared = false;
        live.playWhenReady = playWhenReady;
        mp.setAudioAttributes(new AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                .build());
        mp.setLooping(live.looping);
        live.applyVolume(mp);
        mp.setOnPreparedListener(new MediaPlayer.OnPreparedListener() {
            @Override
            public void onPrepared(MediaPlayer p) {
                if (live.player != p) {
                    return; // superseded by a later load
                }
                live.prepared = true;
                if (live.playWhenReady) {
                    p.start();
                    live.report(PLAYING, "");
                } else {
                    live.report(PAUSED, "");
                }
            }
        });
        mp.setOnInfoListener(new MediaPlayer.OnInfoListener() {
            @Override
            public boolean onInfo(MediaPlayer p, int what, int extra) {
                if (live.player != p) {
                    return false;
                }
                if (what == MediaPlayer.MEDIA_INFO_BUFFERING_START) {
                    live.report(LOADING, "");
                } else if (what == MediaPlayer.MEDIA_INFO_BUFFERING_END) {
                    live.report(p.isPlaying() ? PLAYING : PAUSED, "");
                }
                return false;
            }
        });
        mp.setOnCompletionListener(new MediaPlayer.OnCompletionListener() {
            @Override
            public void onCompletion(MediaPlayer p) {
                if (live.player == p && !live.looping) {
                    live.report(ENDED, "");
                }
            }
        });
        mp.setOnErrorListener(new MediaPlayer.OnErrorListener() {
            @Override
            public boolean onError(MediaPlayer p, int what, int extra) {
                if (live.player == p) {
                    live.report(ERROR, errorText(what, extra));
                }
                return true;
            }
        });
        live.report(LOADING, "");
        final Uri uri = Uri.parse(url);
        IO.execute(new Runnable() {
            @Override
            public void run() {
                try {
                    mp.setDataSource(DayBridge.ctx, uri);
                    mp.prepareAsync();
                } catch (IOException | IllegalArgumentException | IllegalStateException
                         | SecurityException e) {
                    final String text =
                            e.getMessage() == null ? "could not open the stream" : e.getMessage();
                    MAIN.post(new Runnable() {
                        @Override
                        public void run() {
                            if (live.player == mp) {
                                live.report(ERROR, text);
                            }
                        }
                    });
                }
            }
        });
    }

    private static void releasePlayer(Live live) {
        final MediaPlayer old = live.player;
        live.player = null;
        live.prepared = false;
        if (old != null) {
            IO.execute(new Runnable() {
                @Override
                public void run() {
                    try {
                        old.reset();
                    } catch (IllegalStateException ignored) {
                    }
                    old.release();
                }
            });
        }
    }

    /** Imperative commands: 0=load (and play), 1=play, 2=pause, 3=stop, 4=volume (`value`). */
    public static void mediaCommand(View view, int code, String url, double value) {
        Live live = LIVE.get(view);
        if (live == null) {
            return;
        }
        if (view instanceof VideoView) {
            VideoView video = (VideoView) view;
            switch (code) {
                case 0:
                    if (url != null && !url.isEmpty()) {
                        live.report(LOADING, "");
                        video.setVideoURI(Uri.parse(url));
                        video.start();
                    }
                    break;
                case 1:
                    video.start();
                    if (live.prepared) {
                        live.report(PLAYING, "");
                    }
                    break;
                case 2:
                    video.pause();
                    live.report(PAUSED, "");
                    break;
                case 3:
                    video.stopPlayback();
                    live.prepared = false;
                    live.report(IDLE, "");
                    break;
                case 4:
                    live.volume = (float) value;
                    // A VideoView keeps its MediaPlayer private; the next prepare applies it.
                    break;
                default:
                    break;
            }
            return;
        }
        switch (code) {
            case 0:
                if (url != null && !url.isEmpty()) {
                    loadAudio(live, url, true);
                }
                break;
            case 1:
                if (live.player != null && live.prepared) {
                    live.player.start();
                    live.report(PLAYING, "");
                } else {
                    live.playWhenReady = true;
                }
                break;
            case 2:
                live.playWhenReady = false;
                if (live.player != null && live.prepared) {
                    live.player.pause();
                    live.report(PAUSED, "");
                }
                break;
            case 3:
                // Dropping the player is what lets a live stream's connection go.
                releasePlayer(live);
                live.report(IDLE, "");
                break;
            case 4:
                live.volume = (float) value;
                if (live.player != null) {
                    live.applyVolume(live.player);
                }
                break;
            default:
                break;
        }
    }

    /** Whether the view is a sound-only player (which measures zero). */
    public static boolean isAudioOnly(View view) {
        Live live = LIVE.get(view);
        return live != null && live.audioOnly;
    }

    /** Release the sound-only player; a VideoView releases its own with its window. */
    public static void releaseMedia(View view) {
        Live live = LIVE.remove(view);
        if (live == null) {
            return;
        }
        if (view instanceof VideoView) {
            ((VideoView) view).stopPlayback();
        } else {
            releasePlayer(live);
        }
    }
}
