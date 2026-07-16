use day::prelude::*;
use day_piece_media::media;

use crate::widgets::page;

/// A native media player (day-piece-media, an EXTERNAL standalone piece): AVPlayerView /
/// AVPlayerViewController / QMediaPlayer+QVideoWidget / android.widget.VideoView / GtkVideo.
/// Transport is imperative via `Trigger`s the piece watches; native chrome (where the toolkit
/// has one) offers its own controls too. On iOS/Android a bundled Lottie animation
/// (day-piece-lottie) joins the page.
pub(crate) fn media_page() -> AnyPiece {
    let url = Signal::new(
        "https://interactive-examples.mdn.mozilla.net/media/cc0-videos/flower.mp4".to_string(),
    );
    let play = Trigger::new();
    let pause = Trigger::new();
    let load = Trigger::new();
    let video = section((
        // muted: CI walkthroughs screenshot this page — don't blast audio on runners. The
        // fixed height keeps the 16:9 sample balanced against the transport row instead of
        // flooding the page with letterboxing.
        media(url)
            .looping(true)
            .muted(true)
            .play(play)
            .pause(pause)
            .load(load)
            .id("media")
            .height(300.0),
        row((
            button(crate::res::str::media_play())
                .prominent()
                .action(move || play.notify())
                .id("media-play"),
            button(crate::res::str::media_pause())
                .bordered()
                .action(move || pause.notify())
                .id("media-pause"),
            button(crate::res::str::media_load())
                .bordered()
                .action(move || load.notify())
                .id("media-load"),
        ))
        .spacing(8.0),
    ))
    .title(crate::res::str::media_player_section());
    #[cfg(any(target_os = "ios", target_os = "android"))]
    let body = form((video, lottie_section())).any();
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let body = form((video,)).any();
    page(
        crate::res::str::nav_media(),
        "media-title",
        Some(crate::res::str::media_caption()),
        body,
    )
}

/// A native Lottie animation (day-piece-lottie): a LottieAnimationView driven by airbnb's
/// lottie-ios (SwiftPM) / lottie-android (Gradle). Renders the bundled `hello.json`, looping.
#[cfg(any(target_os = "ios", target_os = "android"))]
fn lottie_section() -> impl Piece {
    // Playback rate, bound two ways: the slider drives it and `.speed(speed)` pushes it to the
    // native LottieAnimationView live (a `Speed` patch per change).
    let speed = Signal::new(1.0);
    section((
        label(crate::res::str::lottie_caption())
            .font(Font::Footnote)
            .id("lottie-caption"),
        lottie("hello")
            .speed(speed)
            .frame(220.0, 220.0)
            .id("lottie-view"),
        labeled(
            crate::res::str::lottie_speed(),
            row((
                slider(speed)
                    .range(0.25..=3.0)
                    .step(0.25)
                    .id("lottie-speed-slider"),
                label(move || format!("{:.2}×", speed.get())).id("lottie-speed-value"),
            ))
            .spacing(8.0),
        ),
    ))
    .title(crate::res::str::nav_lottie())
}

#[cfg(any(target_os = "ios", target_os = "android"))]
use day_piece_lottie::lottie;
