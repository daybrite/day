// The day-piece-media crate's OWN Android backend — bundled here and folded into the app's Gradle
// build via [package.metadata.day.android], with ZERO edits to day-android. It uses only
// day-android's PUBLIC Java surface: DayBridge.ctx (the Context). android.widget.VideoView +
// MediaController are framework widgets, so the piece adds no Gradle dependencies; it declares the
// INTERNET permission in Cargo.toml for network sources, which `day build` merges into the app
// manifest. See docs/extending.md.
package dev.daybrite.day.piece.media;

import android.media.MediaPlayer;
import android.net.Uri;
import android.view.View;
import android.widget.MediaController;
import android.widget.VideoView;

import dev.daybrite.day.bridge.DayBridge;

/** Wraps android.widget.VideoView with optional MediaController chrome. */
public final class DayMedia {
    private DayMedia() {}

    public static View makeMedia(
            String url, boolean autoplay, boolean looping, boolean muted, boolean controls) {
        VideoView video = new VideoView(DayBridge.ctx);
        if (controls) {
            MediaController mc = new MediaController(DayBridge.ctx);
            mc.setAnchorView(video);
            video.setMediaController(mc);
        }
        // looping/muted live on the underlying MediaPlayer, only reachable once prepared. The
        // listener re-fires for every setVideoURI (mediaCommand's load), keeping both sticky.
        video.setOnPreparedListener(new MediaPlayer.OnPreparedListener() {
            @Override
            public void onPrepared(MediaPlayer mp) {
                mp.setLooping(looping);
                if (muted) {
                    mp.setVolume(0f, 0f);
                }
            }
        });
        if (url != null && !url.isEmpty()) {
            // Uri.parse handles file paths AND http(s)/content URIs (setVideoPath is the same call).
            video.setVideoURI(Uri.parse(url));
            if (autoplay) {
                video.start();
            }
        }
        return video;
    }

    /** Imperative commands: 0=load (and play), 1=play, 2=pause. */
    public static void mediaCommand(View view, int code, String url) {
        if (!(view instanceof VideoView)) {
            return;
        }
        VideoView video = (VideoView) view;
        switch (code) {
            case 0:
                if (url != null && !url.isEmpty()) {
                    video.setVideoURI(Uri.parse(url));
                    video.start();
                }
                break;
            case 1:
                video.start();
                break;
            case 2:
                video.pause();
                break;
            default:
                break;
        }
    }
}
