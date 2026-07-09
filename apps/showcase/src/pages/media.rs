use day::prelude::*;
use day_piece_media::media;

/// A native media player (day-piece-media, an EXTERNAL standalone piece): AVPlayerView /
/// AVPlayerViewController / QMediaPlayer+QVideoWidget / android.widget.VideoView / GtkVideo.
/// Transport is imperative via `Trigger`s the piece watches; native chrome (where the toolkit
/// has one) offers its own controls too. The player fills the remaining space (a growing leaf).
pub(crate) fn media_page() -> AnyPiece {
    let url = Signal::new(
        "https://interactive-examples.mdn.mozilla.net/media/cc0-videos/flower.mp4".to_string(),
    );
    let play = Trigger::new();
    let pause = Trigger::new();
    let load = Trigger::new();
    column((
        label(tr("nav-media")).font(Font::Title).id("media-title"),
        row((
            button(tr("media-play"))
                .action(move || play.notify())
                .id("media-play"),
            button(tr("media-pause"))
                .action(move || pause.notify())
                .id("media-pause"),
            button(tr("media-load"))
                .action(move || load.notify())
                .id("media-load"),
        ))
        .spacing(8.0),
        // muted: CI walkthroughs screenshot this page — don't blast audio on runners.
        media(url)
            .looping(true)
            .muted(true)
            .play(play)
            .pause(pause)
            .load(load)
            .id("media"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
