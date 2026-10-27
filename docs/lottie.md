---
title: "Lottie animations"
description: "Play bundled Lottie animations and inspect their timing and document structure."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Lottie animations

Use `day-piece-lottie` for animated illustrations, loading sequences, and motion graphics
exported as Lottie JSON. It plays a bundled animation in a Day layout and provides a separate
model API for inspecting animation metadata without opening a player.

## Add to your app

Add [`day-piece-lottie`](https://crates.io/crates/day-piece-lottie) to your app’s Cargo dependencies:

```toml
[dependencies]
day-piece-lottie = "0.4.4"
```

[Crate](https://crates.io/crates/day-piece-lottie) ·
[API reference](https://docs.rs/day-piece-lottie/0.4.4/day_piece_lottie/) ·
[Source](https://github.com/daybrite/day-piece-lottie).
Build with the [Day CLI](/docs/reference/cli), which selects the toolkit integration and stages
the native player dependencies and web player assets.

Place an animation at `resource/assets/hello.json`. The generated
`res::assets::hello_json` constant identifies that file; do not maintain a second literal path
in UI code. This component belongs in an app with generated [resources](/docs/reference/resources):

```rust
use day::prelude::*;
use day_piece_lottie::lottie;

pub fn animation_panel() -> impl Piece {
    let speed = Signal::new(1.0);
    let animation = res::assets::hello_json.as_str().to_owned();
    lottie(animation).speed(speed).frame(220.0, 220.0)
}
```

[`lottie()`](https://docs.rs/day-piece-lottie/0.4.4/day_piece_lottie/fn.lottie.html)
accepts a name, a string signal, or a closure. The generated asset name includes the `.json`
extension, which the player accepts. Give the piece an explicit size or a container that sizes it.

## Playback behavior

| Setting | Default and effect |
| --- | --- |
| Looping | Enabled; [`no_loop()`](https://docs.rs/day-piece-lottie/0.4.4/day_piece_lottie/struct.Lottie.html#method.no_loop) plays once. |
| Autoplay | Enabled; [`no_autoplay()`](https://docs.rs/day-piece-lottie/0.4.4/day_piece_lottie/struct.Lottie.html#method.no_autoplay) starts paused. |
| Speed | `1.0`; [`speed()`](https://docs.rs/day-piece-lottie/0.4.4/day_piece_lottie/struct.Lottie.html#method.speed) accepts a constant or reactive value. |
| Animation | A bundled JSON file; changing a reactive name asks the player to load another animation. |

Looping and autoplay are construction-time options. This release has no public seek command,
completion callback, or separate play/pause trigger. Removing the piece removes its player.
A web-backed player also has page-loading latency before its first animation frame.

## Platform support

| Platform / toolkit | Player |
| --- | --- |
| iOS / UIKit | [Lottie for iOS](https://github.com/airbnb/lottie-ios), integrated through SwiftPM. |
| Android | [Lottie for Android](https://github.com/airbnb/lottie-android), integrated through Gradle. Custom hosts must enable AndroidX. |
| AppKit, GTK, Qt, WinUI, ArkUI, Web | Bundled [lottie-web](https://github.com/airbnb/lottie-web) inside the [web view](/docs/reference/webview); requires that toolkit's web engine. |

Reactive animation and speed updates in the web player depend on
[JavaScript evaluation](/docs/reference/webview#javascript-interaction) support. A target without evaluation
uses the initial values until the piece is rebuilt. Check the actual web-view capability and
installed engine rather than assuming every desktop integration behaves identically.

On UIKit, avoid sending speed changes more often than a display frame. Check the intended
animation on each player: Lottie exporters and players support different effects, fonts, and
image arrangements. Honor the app's reduced-motion policy; a static illustration or a
construction-time paused alternative can be more appropriate.

## Inspect an animation

The [`LottieModel`](https://docs.rs/day-piece-lottie/0.4.4/day_piece_lottie/model/struct.LottieModel.html)
API parses a JSON document without creating a player. This helper accepts caller-provided text:

```rust
use day_piece_lottie::{LottieError, LottieModel};

pub fn animation_duration(json: &str) -> Result<f64, LottieError> {
    let model = LottieModel::parse(json)?;
    Ok(model.duration_secs())
}
```

[`verify()`](https://docs.rs/day-piece-lottie/0.4.4/day_piece_lottie/model/struct.LottieModel.html#method.verify)
reports selected document problems. An empty issue list does not establish complete player
compatibility. For a bundled document, open it through its generated asset constant and
[`day::resource`](https://daybrite.dev/api/day/fn.resource.html).

## Examples

Add an animation playground to an app without copying the example JSON files.
`day-piece-lottie-gallery` bundles the Lottie demo's pages, animations, and translations.

### Add the example library

```toml
[dependencies]
day-piece-lottie-gallery = "0.1"
```

[Crate](https://crates.io/crates/day-piece-lottie-gallery) ·
[API reference](https://docs.rs/day-piece-lottie-gallery/0.1.0/day_piece_lottie_gallery/) ·
[Source](https://github.com/daybrite/day-piece-lottie/tree/main/gallery).
The [Day CLI](/docs/reference/cli) stages the library's animation assets and selects the
underlying player's toolkit features.

### Show the playground

```rust
use day::prelude::*;
use day_piece_lottie_gallery::{LottiePage, gallery};

pub fn animation_examples() -> impl Piece {
    gallery(Signal::new(LottiePage::PinJump))
}
```

[`gallery()`](https://docs.rs/day-piece-lottie-gallery/0.1.0/day_piece_lottie_gallery/fn.gallery.html)
shows a picker and the selected page. Use
[`pages()`](https://docs.rs/day-piece-lottie-gallery/0.1.0/day_piece_lottie_gallery/fn.pages.html)
for a list of [`Entry`](https://docs.rs/day-piece-lottie-gallery/0.1.0/day_piece_lottie_gallery/struct.Entry.html)
values when building your own sidebar, or
[`page()`](https://docs.rs/day-piece-lottie-gallery/0.1.0/day_piece_lottie_gallery/fn.page.html)
to mount a specific [`LottiePage`](https://docs.rs/day-piece-lottie-gallery/0.1.0/day_piece_lottie_gallery/enum.LottiePage.html).
Titles and descriptions come from the library's catalogs and follow the app locale.

The animations belong to the library's resource bundle. Your app does not need to reconstruct
their internal paths or declare copies under its own assets. If extracting or redistributing
sample artwork independently, preserve the upstream licenses and attribution recorded with
[the animation sources](https://github.com/daybrite/day-piece-lottie/tree/main/gallery/animations).

### Platform behavior

The pages use the same native iOS/Android
players and web-view player on other toolkits. Web-engine availability, playback latency, and
reactive-update limits are the same as for the underlying piece. The gallery is a set of sample
pages, not a general animation editor.

Try the [browser demo](https://daybrite.github.io/day-piece-lottie/webapp/), or see
[Chart examples](/docs/reference/charts#examples) for another embeddable example library.

## Related documentation

- [Animation](/docs/reference/tween) animates values in ordinary Day controls.
- [Web views](/docs/reference/webview) describes the web-engine requirements for desktop playback.
