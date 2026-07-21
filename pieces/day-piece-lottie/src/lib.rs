//! day-piece-lottie — an EXTERNAL Day Piece rendering a Lottie animation, on iOS + Android only. It is
//! the reference for a piece that pulls an EXTERNAL native package: the lottie-ios SwiftPM package on
//! iOS (via the `[package.metadata.day.ios]` mechanism this piece introduces) and
//! `com.airbnb.android:lottie` on Android. One Rust API, a native `LottieAnimationView` per platform,
//! registered link-time into each backend's renderer slice without touching day.
//!
//! `lottie("name")` loads `name`(.json), bundled with the app (iOS: the app bundle; Android: assets),
//! and plays it. It's a growing leaf, so constrain it with `.frame(w, h)`.

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_pieces::{IntoReactive, Reactive};
use day_reactive::bind_seeded;

pub const KIND: &str = "day.piece.lottie";

/// Full props (realize). `name`/`looping`/`autoplay` are set once at build; `speed` seeds the
/// playback rate and thereafter patches (see [`LottiePatch`]).
#[derive(Clone, Debug, PartialEq)]
pub struct LottieProps {
    /// The bundled animation name (without extension), e.g. `"hello"` → `hello.json`.
    pub name: String,
    /// Loop the animation (vs. play once).
    pub looping: bool,
    /// Start playing immediately on appear.
    pub autoplay: bool,
    /// Playback rate multiplier (1.0 = normal, 2.0 = double speed, 0.5 = half). Also patchable.
    pub speed: f64,
}

impl Default for LottieProps {
    fn default() -> Self {
        LottieProps {
            name: String::new(),
            looping: false,
            autoplay: false,
            speed: 1.0,
        }
    }
}

/// Sparse reconcile patch — only `speed` changes after build (name/looping/autoplay are fixed).
#[derive(Clone, Debug, PartialEq)]
pub enum LottiePatch {
    /// New playback rate multiplier — pushed whenever the bound speed signal changes.
    Speed(f64),
}

/// A native Lottie animation view. Configure with `.looping(false)` / `.autoplay(false)` and bind the
/// playback rate reactively with `.speed(signal)`.
pub struct Lottie {
    name: String,
    looping: bool,
    autoplay: bool,
    speed: Reactive<f64>,
}

/// `lottie("hello")` — render the bundled `hello.json` Lottie animation (looping, autoplaying, 1× speed).
pub fn lottie(name: impl Into<String>) -> Lottie {
    Lottie {
        name: name.into(),
        looping: true,
        autoplay: true,
        speed: Reactive::Const(1.0),
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
    /// Playback rate multiplier — a constant, a `Signal<f64>`, or a `Fn() -> f64`. When it's reactive
    /// the view's speed follows it live (e.g. bound to a slider). Default 1.0.
    pub fn speed<M>(mut self, speed: impl IntoReactive<f64, M>) -> Self {
        self.speed = speed.into_reactive();
        self
    }
}

impl Piece for Lottie {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let speed = self.speed;
        let seed = speed.get_untracked();
        let props = LottieProps {
            name: self.name,
            looping: self.looping,
            autoplay: self.autoplay,
            speed: seed,
        };
        // A Lottie animation fills the space it's offered (constrain via `.frame(w, h)`).
        let node = cx.leaf(
            KIND,
            &props,
            Flex {
                grow_w: true,
                grow_h: true,
                ..Default::default()
            },
        );
        // A `Const` speed reads the same value forever, so this seeds once and never patches; a
        // `Signal`/`Fn` speed re-runs and pushes a `Speed` patch on every change.
        bind_seeded(
            seed,
            move || speed.get(),
            move |v: &f64| {
                with_tree(|t| t.patch(node, Box::new(LottiePatch::Speed(*v)), false));
            },
        );
        node
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers — iOS + Android only. Each registers a `Renderer` link-time into its
// backend's `RENDERERS` slice; `#[cfg]` gates each to its feature + target.
// ---------------------------------------------------------------------------

day_pieces::glue_modules!(uikit, widget);

