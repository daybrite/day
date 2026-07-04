//! day-piece-lottie — an EXTERNAL Day Piece rendering a Lottie animation, on iOS + Android only. It is
//! the reference for a piece that pulls an EXTERNAL native package: the lottie-ios SwiftPM package on
//! iOS (via the `[package.metadata.day.ios]` mechanism this piece introduces) and
//! `com.airbnb.android:lottie` on Android. One Rust API, a native `LottieAnimationView` per platform,
//! registered link-time into each backend's renderer slice with **zero edits** to day.
//!
//! `lottie("name")` loads `name`(.json), bundled with the app (iOS: the app bundle; Android: assets),
//! and plays it. It's a growing leaf, so constrain it with `.frame(w, h)`.

use day_core::{BuildCx, Flex, Piece, RNode};

pub const KIND: &str = "day.piece.lottie";

/// Full props (realize). All are set at build; there are no patches (the demo autoplays + loops).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LottieProps {
    /// The bundled animation name (without extension), e.g. `"hello"` → `hello.json`.
    pub name: String,
    /// Loop the animation (vs. play once).
    pub looping: bool,
    /// Start playing immediately on appear.
    pub autoplay: bool,
}

/// A native Lottie animation view. Configure with `.looping(false)` / `.autoplay(false)`.
pub struct Lottie {
    name: String,
    looping: bool,
    autoplay: bool,
}

/// `lottie("hello")` — render the bundled `hello.json` Lottie animation (looping, autoplaying).
pub fn lottie(name: impl Into<String>) -> Lottie {
    Lottie {
        name: name.into(),
        looping: true,
        autoplay: true,
    }
}

impl Lottie {
    /// Loop the animation (default true).
    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }
    /// Play immediately on appear (default true).
    pub fn autoplay(mut self, autoplay: bool) -> Self {
        self.autoplay = autoplay;
        self
    }
}

impl Piece for Lottie {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let props = LottieProps {
            name: self.name,
            looping: self.looping,
            autoplay: self.autoplay,
        };
        // A Lottie animation fills the space it's offered (constrain via `.frame(w, h)`).
        cx.leaf(
            KIND,
            &props,
            Flex {
                grow_w: true,
                grow_h: true,
                ..Default::default()
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers — iOS + Android only. Each registers a `Renderer` link-time into its
// backend's `RENDERERS` slice; `#[cfg]` gates each to its feature + target.
// ---------------------------------------------------------------------------

#[cfg(all(feature = "uikit", target_os = "ios"))]
#[path = "lib-uikit.rs"]
mod uikit_impl;

#[cfg(all(feature = "widget", target_os = "android"))]
#[path = "lib-android.rs"]
mod android_impl;
