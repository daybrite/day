# Media player (external piece)

> **Status: implemented** as `day-piece-media` — an EXTERNAL Day Piece (like `day-piece-webview`),
> registered link-time into each backend's renderer slice with **zero edits** to day. It wraps each
> toolkit's NATIVE media player for audio/video playback and fills the space it's offered
> (constrain it with `.frame(w, h)`).

## Authoring

```rust
use day_piece_media::media;

let url = Signal::new("https://interactive-examples.mdn.mozilla.net/media/cc0-videos/flower.mp4".to_string());
let (play, pause, load) = (Trigger::new(), Trigger::new(), Trigger::new());

button("Play").action(move || play.notify());
button("Pause").action(move || pause.notify());
button("Load").action(move || load.notify()); // re-reads `url` and plays it

media(url)
    .autoplay(true)     // default true — start as soon as the media is ready
    .looping(false)     // default false — restart from 0 at the end
    .muted(false)       // default false
    .controls(true)     // default true — native transport chrome where the toolkit has it
    .play(play)
    .pause(pause)
    .load(load)
    .id("media")
```

`media(url)` takes a string, a `Signal<String>`, or a closure (the `IntoText` conversions). The one
string accepts **either a local file path or an http(s)/file URL** — each backend picks the right
loader (`fileURLWithPath` vs `URLWithString`, `QUrl::fromUserInput`, `Uri.parse`,
`gio::File::for_path/for_uri`); anything containing `://` is treated as a URL. The initial value
loads when the view is created; transport is imperative with `Copy` `Trigger`s — `.play()` /
`.pause()` resume and pause, `.load()` re-reads the bound url and plays it (track switching). There
is deliberately **no two-way "playing" binding** in v1: native chrome mutates play state behind
day's back, so state readback would need an observer rail on every backend (the `Event::custom`
channel is the seam if it's wanted later). `Media` implements `Piece`, so `.id()`/`.a11y()`/
`.frame()` chain via `Decorate`. It's a growing leaf (`Flex { grow_w, grow_h }` +
`day_pieces::fill_measure`), so put it last in a `column` and it fills the remaining space.

## Per-backend native realization

| | AppKit | UIKit | Qt | Android | GTK |
|---|---|---|---|---|---|
| control | `AVPlayerView` + `AVPlayer` | `AVPlayerViewController` + `AVPlayer` | `QMediaPlayer` + `QAudioOutput` + `QVideoWidget` | `android.widget.VideoView` | `gtk4::Video` (GtkMediaFile) |
| native code | objc2-av-kit / objc2-av-foundation | hand-rolled `extern_class!` + `msg_send!` (+ objc2-av-foundation) | `src/lib-qt-shim.cpp` (+ links `Qt6MultimediaWidgets`) | `android/java/…/DayMedia.java` | gtk4 crate (core widget) |
| chrome (`.controls`) | `controlsStyle` Inline/None | `showsPlaybackControls` | none (v1: drive with triggers) | `MediaController` | GtkVideo overlay (always on) |
| looping | end-notification observer → seek 0 | end-notification observer → seek 0 | `QMediaPlayer::setLoops(Infinite)` | `MediaPlayer.setLooping` | `Video::set_loop` |

**Backend notes:**

- **AppKit** — `objc2-av-kit`'s `AVPlayerView` (macOS-only binding) gives the full native transport
  bar. AVPlayer has no loop flag, so a small NSObject observer watches
  `AVPlayerItemDidPlayToEndTimeNotification` (object: nil so `.load()` swaps stay covered, then
  filtered to our player's current item) and seeks back to `kCMTimeZero`. The observer is retained
  in a thread_local (notification centers don't retain observers).
- **UIKit** — objc2-av-kit does NOT bind `AVPlayerViewController` on iOS (the WKWebView situation
  again), so the piece hand-rolls it via `extern_class!`/`msg_send!` and embeds `vc.view` as the
  leaf. The controller is retained in a thread_local keyed by the view pointer. AVKit +
  AVFoundation must be linked for the ObjC classes to register — declared via
  `[package.metadata.day.ios] frameworks = ["AVKit", "AVFoundation"]`, linked by the generated
  DayPieces SwiftPM package. (The controller is not parented into the view-controller hierarchy;
  inline playback works, fullscreen presentation is out of v1 scope.)
- **Qt** — this crate's OWN C++ shim, compiled by build.rs with a `pkg-config
  Qt6MultimediaWidgets` probe (day-qt-sys links Widgets but NOT Multimedia — the shim emits those
  libs). Where the module is absent the shim degrades to a URL `QLabel` and build.rs prints a
  `cargo:warning`, so the app still builds/launches/screenshots. `QVideoWidget` ships no chrome —
  `.controls` is a no-op on Qt; use the triggers. Linux CI wants `qt6-multimedia-dev`; Homebrew's
  Qt ships the AVFoundation `darwinmediaplugin`, so playback works on macos-qt out of the box.
- **Android** — framework `VideoView` + `MediaController` (free native seek/play chrome), **zero
  Gradle dependencies**; `looping`/`muted` are applied in `onPrepared` (they live on the underlying
  `MediaPlayer`, which re-prepares on every load). The piece contributes
  `android.permission.INTERNET` via `[package.metadata.day.android] permissions`. Known VideoView
  limits: audio-only files play against a black surface; androidx.media3/ExoPlayer (HLS/DASH,
  modern `PlayerView`) is the v2 upgrade via the lottie-proven `gradle-dependencies` key.
- **GTK** — `gtk4::Video` is a core widget so the feature compiles everywhere, **but playback needs
  gtk4 built with a gstreamer media backend**. Linux distro gtk4 has it (`-Dmedia-gstreamer`);
  Homebrew's gtk4 ships NO media backend, so on macos-gtk GtkVideo shows its own "no media backend"
  error UI — the same caveat class as webkitgtk (Linux-first backend). GtkVideo's overlay controls
  cannot be hidden, so `.controls(false)` is a no-op.
- **WinUI / mock** — the features exist (so an app can enable `day-piece-media/<feature>` uniformly
  per backend) but register no renderer; the media kind falls back to day's placeholder leaf.
  WinUI's eventual route is `MediaPlayerElement` via the cppwinrt shim pattern. HarmonyOS is
  deferred until day-arkui grows XComponent surface plumbing (`OH_AVPlayer_SetVideoSurface` needs
  an `OHNativeWindow`).

## Testing

The crate's smoke test boots the piece on the mock toolkit (which realizes unknown kinds as plain
widgets and ignores unknown patches — exactly like a backend built without the feature), fires all
three triggers, and must never panic: `cargo test -p day-piece-media`.

For a live check, wire the showcase media page to a small public sample (e.g. MDN's `flower.mp4`)
and use the webview walkthrough recipe: navigate to the route, `pause` (runner-side) so the first
frame arrives, then screenshot.
