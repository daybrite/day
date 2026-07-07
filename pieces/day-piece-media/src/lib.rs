//! day-piece-media — an EXTERNAL Day Piece (DESIGN.md §15) wrapping each toolkit's NATIVE media
//! player: AVPlayerView on AppKit, AVPlayerViewController on UIKit, QMediaPlayer + QVideoWidget on
//! Qt, `android.widget.VideoView` on Android, GtkVideo on GTK. One Rust API registered link-time
//! into each backend's renderer slice with **zero edits** to day. Like the webview it carries both
//! a front-end AND its own native backends — including an Android manifest permission contribution
//! (INTERNET) and an iOS framework contribution (AVKit + AVFoundation), see docs/extending.md.
//!
//! The player is a growing leaf that fills its space (constrain it with `.frame(w, h)`). The `url`
//! source accepts a plain string, a `Signal<String>`, or a closure, and may name a local file path
//! OR an http(s)/file URL — every backend's loader takes both. Configure playback at build with
//! `.autoplay(bool)` / `.looping(bool)` / `.muted(bool)` / `.controls(bool)`; transport is
//! imperative and modeled with `Copy` `Trigger`s — `.play()` / `.pause()` drive playback and
//! `.load()` re-reads the bound url (then plays) — each `watch`ed to a `MediaPatch`.
//!
//! Native chrome (`.controls(true)`, the default) is free where the toolkit has it: AVPlayerView's
//! inline controls, AVPlayerViewController's playback controls, Android's MediaController. Qt's
//! QVideoWidget has no built-in chrome (drive it with triggers), and GtkVideo's overlay controls
//! are always on. See docs/media.md for the per-backend caveats.

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_pieces::{IntoText, TextSource};
use day_reactive::{Trigger, untrack, watch};

pub const KIND: &str = "day.piece.media";

/// Full props (realize). The initial `url` loads when the native view is created; the flags are
/// fixed at build time.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaProps {
    /// A local file path or an http(s)/file URL.
    pub url: String,
    /// Start playing as soon as the media is ready (default true).
    pub autoplay: bool,
    /// Restart from the beginning when playback reaches the end (default false).
    pub looping: bool,
    /// Silence the audio track (default false).
    pub muted: bool,
    /// Show the toolkit's native transport chrome where it has one (default true).
    pub controls: bool,
}

impl Default for MediaProps {
    fn default() -> Self {
        MediaProps {
            url: String::new(),
            autoplay: true,
            looping: false,
            muted: false,
            controls: true,
        }
    }
}

/// Sparse imperative commands sent to the native player after creation.
#[derive(Clone, Debug, PartialEq)]
pub enum MediaPatch {
    /// Load a url (from `.load()` — re-reads the bound source) and start playing it.
    Load(String),
    /// Resume/start playback (from `.play()`).
    Play,
    /// Pause playback (from `.pause()`).
    Pause,
}

/// A native media player bound to `url`. Attach command triggers with `.play()/.pause()/.load()`;
/// fire them (`Trigger::notify`) from buttons.
pub struct Media {
    url: TextSource,
    autoplay: bool,
    looping: bool,
    muted: bool,
    controls: bool,
    play: Option<Trigger>,
    pause: Option<Trigger>,
    load: Option<Trigger>,
}

/// `media(url)` — a native audio/video player for `url` (a string, `Signal<String>`, or closure;
/// a file path or an http(s) URL). The initial value loads on creation and autoplays by default;
/// call `.load(trigger)` and fire the trigger to (re)load whatever `url` currently holds.
pub fn media<M>(url: impl IntoText<M>) -> Media {
    Media {
        url: url.into_text(),
        autoplay: true,
        looping: false,
        muted: false,
        controls: true,
        play: None,
        pause: None,
        load: None,
    }
}

impl Media {
    /// Start playing as soon as the media is ready (default true).
    pub fn autoplay(mut self, autoplay: bool) -> Self {
        self.autoplay = autoplay;
        self
    }
    /// Restart from the beginning when playback reaches the end (default false).
    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }
    /// Silence the audio track (default false).
    pub fn muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }
    /// Show the toolkit's native transport chrome where it has one (default true). Qt has no free
    /// chrome (use triggers) and GtkVideo's overlay controls cannot be hidden — see docs/media.md.
    pub fn controls(mut self, controls: bool) -> Self {
        self.controls = controls;
        self
    }
    /// Resume/start playback whenever `trigger` fires.
    pub fn play(mut self, trigger: Trigger) -> Self {
        self.play = Some(trigger);
        self
    }
    /// Pause playback whenever `trigger` fires.
    pub fn pause(mut self, trigger: Trigger) -> Self {
        self.pause = Some(trigger);
        self
    }
    /// Re-read the bound `url` and load + play it whenever `trigger` fires.
    pub fn load(mut self, trigger: Trigger) -> Self {
        self.load = Some(trigger);
        self
    }
}

impl Piece for Media {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Media {
            url,
            autoplay,
            looping,
            muted,
            controls,
            play,
            pause,
            load,
        } = self;
        let initial = MediaProps {
            url: url.initial(),
            autoplay,
            looping,
            muted,
            controls,
        };
        // A media player has no intrinsic size — it fills whatever space its container offers.
        let node = cx.leaf(
            KIND,
            &initial,
            Flex {
                grow_w: true,
                grow_h: true,
                ..Default::default()
            },
        );

        let send = move |patch: MediaPatch| {
            with_tree(|t| t.patch(node, Box::new(patch), false));
        };

        // Each command trigger → one patch. `watch` never fires for the initial value, so wiring
        // these does not issue a spurious command at build time (the initial url loads via props).
        if let Some(play) = play {
            watch(move || play.track(), move |_, _| send(MediaPatch::Play));
        }
        if let Some(pause) = pause {
            watch(move || pause.track(), move |_, _| send(MediaPatch::Pause));
        }
        if let Some(load) = load {
            // Re-read the bound url when the trigger fires: a `Static` source re-loads the fixed
            // string (a restart-from-source), a `Signal`/closure source reads its current value.
            let read: std::rc::Rc<dyn Fn() -> String> = match url {
                TextSource::Static(s) => std::rc::Rc::new(move || s.clone()),
                TextSource::Dyn(f) => f,
            };
            watch(
                move || load.track(),
                move |_, _| send(MediaPatch::Load(untrack(|| read()))),
            );
        }
        node
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers — one file per backend. Each module registers a `Renderer`
// link-time into its backend's `RENDERERS` slice; `#[cfg]` gates each to its feature + target, and
// `#[path]` keeps the files grouped next to lib.rs. winui/mock register nothing (the media kind
// falls back to day's placeholder leaf there).
// ---------------------------------------------------------------------------

#[cfg(all(feature = "appkit", target_os = "macos"))]
#[path = "lib-appkit.rs"]
mod appkit_impl;

// GtkVideo is core GTK, so this compiles on every gtk host — but playback needs a gstreamer media
// backend in the gtk4 build (Linux default; Homebrew gtk4 has none, so macos-gtk shows GtkVideo's
// own error UI — see Cargo.toml + docs/media.md).
#[cfg(feature = "gtk")]
#[path = "lib-gtk.rs"]
mod gtk_impl;

#[cfg(feature = "qt")]
#[path = "lib-qt.rs"]
mod qt_impl;

#[cfg(all(feature = "uikit", target_os = "ios"))]
#[path = "lib-uikit.rs"]
mod uikit_impl;

#[cfg(all(feature = "widget", target_os = "android"))]
#[path = "lib-android.rs"]
mod android_impl;

#[cfg(test)]
mod tests {
    use super::*;
    use day_mock::MockToolkit;
    use day_reactive::{Signal, flush_sync};
    use day_spec::{Size, WindowOptions};

    // Building + driving the piece must never panic — even with no native renderer registered
    // (the mock toolkit realizes unknown kinds as plain widgets and ignores unknown patches,
    // exactly like a backend built without this piece's feature).
    #[test]
    fn build_and_commands_do_not_panic() {
        let url = Signal::new("https://example.com/flower.mp4".to_string());
        let play = Trigger::new();
        let pause = Trigger::new();
        let load = Trigger::new();

        day_core::uninstall_tree();
        let (mock, probe) = MockToolkit::new();
        let options = WindowOptions {
            title: "test".into(),
            size: Size::new(400.0, 300.0),
            ..Default::default()
        };
        day_core::launch_with(mock, options, move || {
            day_core::AnyPiece::new(
                media(url)
                    .autoplay(false)
                    .looping(true)
                    .muted(true)
                    .controls(false)
                    .play(play)
                    .pause(pause)
                    .load(load),
            )
        });

        let found = probe.find_by_kind(KIND);
        assert_eq!(found.len(), 1, "one media leaf realized");

        // Fire every command trigger; each becomes a MediaPatch the mock ignores gracefully.
        play.notify();
        pause.notify();
        url.set("file:///tmp/other.mp4".to_string());
        load.notify();
        flush_sync();
    }
}
