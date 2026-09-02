---
title: "Media player"
description: "Audio and video playback as an external piece over each platform's media framework."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Media player (external piece)

> **Status: implemented** as `day-piece-media`, an external Day Piece (like `day-piece-webview`),
> registered into each backend's renderer slice without touching day: link-time on the eight native
> backends, runtime on web-dom (wasm has no `linkme`, so the piece registers itself from `media()`).
> It wraps each toolkit's native media player for audio/video playback and fills the space it's
> offered (constrain it with `.frame(w, h)`), or draws nothing at all as a sound-only player.

## Authoring

```rust
use day_piece_media::{PlaybackState, media};

let url = Signal::new("https://interactive-examples.mdn.mozilla.net/media/cc0-videos/flower.mp4".to_string());
let (play, pause, stop, load) = (Trigger::new(), Trigger::new(), Trigger::new(), Trigger::new());
let volume = Signal::new(0.8);
let state = Signal::new(PlaybackState::Idle);

button("Play").action(move || play.notify());
button("Pause").action(move || pause.notify());
button("Stop").action(move || stop.notify());   // drops the source
button("Load").action(move || load.notify());   // re-reads `url` and plays it
slider(volume).range(0.0..=1.0);
label(move || format!("{:?}", state.get()));

media(url)
    .autoplay(true)     // default true: start as soon as the media is ready
    .looping(false)     // default false: restart from 0 at the end
    .muted(false)       // default false
    .controls(true)     // default true: native transport chrome where the toolkit has it
    .volume(volume)     // a constant, a Signal<f64>, or a closure; tracked
    .play(play)
    .pause(pause)
    .stop(stop)
    .load(load)
    .state(state)       // the native player's state, written by the piece
    .id("media")
```

`media(url)` takes a string, a `Signal<String>`, or a closure (the `IntoText` conversions). The one
string accepts either a local file path or an http(s)/file URL; each backend picks the right
loader (`fileURLWithPath` vs `URLWithString`, `QUrl::fromUserInput`, `Uri.parse`,
`gio::File::for_path/for_uri`), and anything containing `://` is treated as a URL. The initial value
loads when the view is created. (Web is the exception on paths: a page can only fetch a URL, so
web-dom needs an http(s) one; see the Web note below.)

Transport is imperative with `Copy` `Trigger`s: `.play()` / `.pause()` resume and pause, `.stop()`
pauses AND drops the source (a paused live stream keeps its connection open and its buffer
filling; a stopped one lets both go), and `.load()` re-reads the bound url and plays it (track
switching). `.volume(…)` is a tracked fraction, `0.0..=1.0`, patched through as it changes.

**State readback.** `.state(signal)` binds a `Signal<PlaybackState>` the piece writes on every
change the toolkit reports, whoever caused it — a trigger, the native chrome, or the network:

| `PlaybackState` | means |
|---|---|
| `Idle` | no source, or the source was stopped |
| `Loading` | a source is set and the player is connecting or buffering |
| `Playing` / `Paused` | what they say |
| `Ended` | the source played to its end (a file; a live stream never does) |
| `Error(String)` | the player gave up; the text is the toolkit's own message |

`PlaybackState::is_active()` is "sound is, or is about to be, coming out" — the state a play/pause
button draws itself from. The arms report on the node's `Event::Custom` channel with the codes in
`day_piece_media::report` (a cross-boundary Custom carries only `num` and `text`, so the code is
the discriminator); an app never sees the channel, only the signal.

**Sound only.** `.audio_only(true)` builds the toolkit's bare audio player rather than its video
view: the leaf draws nothing and measures ZERO, so a radio app can drop it anywhere in its tree
and build its own now-playing UI around it. `.controls` is meaningless there. `Media` implements
`Piece`, so `.id()`/`.a11y()`/`.frame()` chain via `Decorate`. A picture player is a growing leaf
(`Flex { grow_w, grow_h }` + `day_pieces::fill_measure`), so put it last in a `column` and it
fills the remaining space.

## Per-backend native realization

| | AppKit | UIKit | Qt | Android | GTK | XAML | HarmonyOS | Web |
|---|---|---|---|---|---|---|---|---|
| control | `AVPlayerView` + `AVPlayer` | `AVPlayerViewController` + `AVPlayer` | `QMediaPlayer` + `QAudioOutput` + `QVideoWidget` | `android.widget.VideoView` | `gtk4::Video` (GtkMediaFile) | `MediaPlayerElement` + `MediaPlayer` | ArkTS `Video` | `<video>` |
| sound only | hidden `NSView` + `AVPlayer` | hidden `UIView` + `AVPlayer` | `QMediaPlayer` + `QAudioOutput`, no widget | empty `View` + `MediaPlayer` | hidden `GtkBox` + `GtkMediaFile` | collapsed element | `AVPlayer` (@kit.MediaKit) | hidden `<audio>` |
| native code | objc2-av-kit / objc2-av-foundation | hand-rolled `extern_class!` + `msg_send!` (+ objc2-av-foundation) | `src/lib-qt-shim.cpp` (+ links `Qt6MultimediaWidgets`) | `android/java/…/DayMedia.java` | gtk4 crate (core widget) | `src/lib-xaml-shim.cpp` (cppwinrt) | `ohos/ets/Index.ets` | `src/lib-dom.rs` + the shim's media listener |
| chrome (`.controls`) | `controlsStyle` Inline/None | `showsPlaybackControls` | none (v1: drive with triggers) | `MediaController` | GtkVideo overlay (always on) | `AreTransportControlsEnabled` | `.controls()` | `controls` attribute |
| state readback | KVO on `timeControlStatus` + `currentItem.status`, end/failed notifications | same | `playbackStateChanged` / `mediaStatusChanged` / `errorOccurred` | `OnPrepared/Info/Completion/Error` listeners | `MediaStream` notify signals | `PlaybackSession.PlaybackStateChanged`, `MediaEnded`, `MediaFailed` | `Video` callbacks / `AVPlayer` `stateChange` | `playing`/`pause`/`waiting`/`ended`/`error` events |
| looping | end-notification observer → seek 0 | same | `QMediaPlayer::setLoops(Infinite)` | `MediaPlayer.setLooping` | `MediaStream::set_loop` | `IsLoopingEnabled` | `.loop()` / `player.loop` | `loop` attribute |
| volume | `AVPlayer.volume` | same | `QAudioOutput::setVolume` | `MediaPlayer.setVolume` (a `VideoView` applies it on the next prepare) | `MediaStream::set_volume` | `MediaPlayer.Volume` | `AVPlayer.setVolume` (a `Video` has mute only) | `volume` property |

**Backend notes:**

- **AppKit**: `objc2-av-kit`'s `AVPlayerView` (macOS-only binding) gives the full native transport
  bar. AVPlayer has no loop flag, so a small NSObject observer watches
  `AVPlayerItemDidPlayToEndTimeNotification` (object: nil so `.load()` swaps stay covered, then
  filtered to our player's current item) and seeks back to `kCMTimeZero`. The observer is retained
  in a thread_local (notification centers don't retain observers).
- **UIKit**: objc2-av-kit does not bind `AVPlayerViewController` on iOS (the WKWebView situation
  again), so the piece hand-rolls it via `extern_class!`/`msg_send!` and embeds `vc.view` as the
  leaf. The first player puts the process's `AVAudioSession` in the `playback` category and
  activates it (AVFAudio is linked through the crate's framework contribution): without that iOS
  treats the sound as ambient, silenced by the ring switch and stopped when the app leaves the
  foreground. Playing in the background additionally needs the app's own `UIBackgroundModes`
  `audio` entry in `platform/ios/Runner/Info.plist`. The controller is retained in a thread_local keyed by the view pointer. AVKit +
  AVFoundation must be linked for the ObjC classes to register; they're declared via
  `[package.metadata.day.ios] frameworks = ["AVKit", "AVFoundation"]` and linked by the generated
  DayPieces SwiftPM package. (The controller is not parented into the view-controller hierarchy;
  inline playback works, fullscreen presentation is out of v1 scope.)
- **Qt**: this crate's own C++ shim, compiled by build.rs with a `pkg-config
  Qt6MultimediaWidgets` probe (day-qt-sys links Widgets but not Multimedia; the shim emits those
  libs). Where the module is absent the shim degrades to a URL `QLabel` and build.rs prints a
  `cargo:warning`, so the app still builds/launches/screenshots. `QVideoWidget` ships no chrome,
  so `.controls` is a no-op on Qt; use the triggers. Linux CI wants `qt6-multimedia-dev`; Homebrew's
  Qt ships the AVFoundation `darwinmediaplugin`, so playback works on macos-qt out of the box.
- **Android**: framework `VideoView` + `MediaController` (native seek/play chrome) for pictures,
  and a bare `android.media.MediaPlayer` behind an empty `View` for sound only — prepared
  asynchronously, so a stream reports `Loading` and then `Playing` as it connects — with no Gradle
  dependencies; `looping`/`muted`/`volume` are applied in `onPrepared` (they live on the underlying
  `MediaPlayer`, which re-prepares on every load). The piece contributes
  `android.permission.INTERNET` via `[package.metadata.day.android] permissions`; plain `http://`
  sources additionally need the app's `android:usesCleartextTraffic="true"`. androidx.media3/
  ExoPlayer (HLS/DASH, a `MediaSession` for the lock screen) is the v2 upgrade via the
  `gradle-dependencies` key lottie already uses.
- **GTK**: `gtk4::Video` is a core widget so the feature compiles everywhere, but playback needs
  gtk4 built with a gstreamer media backend. Linux distro gtk4 has it (`-Dmedia-gstreamer`);
  Homebrew's gtk4 ships no media backend, so on macos-gtk GtkVideo shows its own "no media backend"
  error UI (the same caveat class as webkitgtk, a Linux-first backend). GtkVideo's overlay controls
  cannot be hidden, so `.controls(false)` is a no-op.
- **Web**: the browser supplies most of the player. A sound-only player is a hidden `<audio>`;
  the shim's media listener (`day_dom::listen::MEDIA`) turns the element's events into the state
  reports, and the volume property rides in `data-volume` and a `dayApplyVolume` method the
  listener hangs on the element (the shim's call surface is zero-argument methods). It provides transport chrome, buffering,
  scrubbing, fullscreen, captions, and picture-in-picture, and every `MediaProps` field is an
  attribute of the same name. Browser policy adds two web-only rules. A **file path will not
  load** (a page can only fetch a URL, and a cross-origin one needs CORS; serve it from the app's
  own `dist/` or use a permissive remote), and **autoplay with sound is blocked** until the user
  has interacted with the page, so `.autoplay(true)` starts only with `.muted(true)`.
  Registration is the other difference: `linkme`'s
  `#[distributed_slice]` does not compile for `wasm32-unknown-unknown`, so day-dom keeps a runtime
  registry (`day_dom::register_renderer`) and `media()` registers the renderer on its first call,
  which always precedes the node being realized.
- **XAML**: `MediaPlayerElement` over a `Windows.Media.Playback.MediaPlayer` through the crate's
  own cppwinrt shim, boxed via day-xaml-sys's `day_xaml_box` seam. Written blind (no Windows host
  here) and verified in CI; a creation failure degrades to a URL `TextBlock` and reports an error
  on the state channel so the app keeps running.
- **HarmonyOS**: no native node kind exists, so the piece ships ArkTS (`ohos/ets/Index.ets`,
  staged through `[package.metadata.day.ohos]`): an ArkUI `Video` component for pictures, driven
  through its `VideoController`, and a `media.createAVPlayer()` for sound only — which needs no
  XComponent surface, the reason this arm was once deferred. A `load` on the picture player
  rebuilds the component (its `src` is fixed at build). `Video` exposes mute but no volume.
- **mock**: the feature exists (so an app can enable `day-piece-media/<feature>` uniformly per
  backend) but registers no renderer; the media kind falls back to day's placeholder leaf.

## Testing

The crate's test boots the piece on the mock toolkit (which realizes unknown kinds as plain
widgets and ignores unknown patches, just like a backend built without the feature), fires every
trigger and a volume change, and must never panic; a second test pins the state codes:
`cargo test -p day-piece-media`.

Day Tunes (daybrite/Day-Tunes) is the sound-only player's live check: its walkthrough plays a
station and reads the state line the signal drives. For a picture, wire the showcase media page
to a small public sample (e.g. MDN's `flower.mp4`)
and use the webview walkthrough recipe: navigate to the route, `pause` (runner-side) so the first
frame arrives, then screenshot.
