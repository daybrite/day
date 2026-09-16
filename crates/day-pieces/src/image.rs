// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The `image` piece — loads a named asset (resolved from the dev asset root, the app bundle, or
//! Android's `AssetManager`) with content-mode and aspect-ratio fitting.

use day_core::*;
use day_spec::kinds;

use crate::Decorated;
use day_spec::props::*;

// ---------------------------------------------------------------------------
// Image (§18.2, MVP): sources resolve via DAY_ASSET_ROOT (desktop dev), the app
// bundle (ios), or AssetManager (android).
// ---------------------------------------------------------------------------

/// An image: a bundled asset resolved by name through the backend's native image pipeline
/// (§18.3), encoded bytes the app already holds, or an image decoded once and shared
/// (docs/images.md). Scales with [`ContentMode::Fit`] by default (never stretches); tune with
/// `.content_mode()` / `.fill()` / `.stretch()`, and optionally constrain the frame with
/// `.aspect_ratio(w/h)`.
pub struct Image {
    source: crate::Reactive<day_spec::ImageSource>,
    content_mode: ContentMode,
    aspect_ratio: Option<f64>,
    decorative: bool,
}

/// Draw an image from any [`ImageSource`](day_spec::ImageSource) — a staged asset name, encoded
/// bytes, or a decoded [`Bitmap`](day_core::Bitmap).
///
/// ```ignore
/// image(res::images::cover)                 // a staged asset, as it always was
/// image(png_bytes)                          // Vec<u8> / Arc<Vec<u8>> the app fetched or picked
/// image(&bitmap)                            // one decode, shared by several nodes
/// image(move || shot.get())                 // reactive: swapping the source patches in place
/// ```
///
/// Bytes and decoded sources need [`Cap::ImageDecode`](day_spec::Cap::ImageDecode); a backend
/// without it draws nothing rather than guessing. A name still resolves the way it always has,
/// on every backend.
pub fn image<M>(source: impl IntoImageSource<M>) -> Image {
    Image {
        source: source.into_image_source(),
        content_mode: ContentMode::default(),
        aspect_ratio: None,
        decorative: false,
    }
}

/// Disjoint-marker conversion into an image source (the same shape as
/// [`IntoText`](crate::IntoText), and for the same coherence reason): a staged name, a byte
/// buffer, a decoded [`Bitmap`](day_core::Bitmap), a `Signal`, or a closure all convert, each
/// under its own marker.
///
/// A blanket `impl<S: Into<ImageSource>>` over the existing
/// [`IntoReactive`](crate::IntoReactive) would be ambiguous for `ImageSource` itself — the
/// static blanket and the converting one would both apply and leave the marker unconstrained —
/// so the conversions are spelled per concrete type instead.
pub trait IntoImageSource<M> {
    fn into_image_source(self) -> crate::Reactive<day_spec::ImageSource>;
}

/// Marker for the by-value conversions (a name, bytes, a bitmap, a source).
pub struct ImageValueMark;
/// Marker for `Signal<ImageSource>`.
pub struct ImageSignalMark;
/// Marker for `Fn() -> ImageSource`.
pub struct ImageFnMark;

macro_rules! image_source_from_value {
    ($($t:ty),* $(,)?) => {
        $(impl IntoImageSource<ImageValueMark> for $t {
            fn into_image_source(self) -> crate::Reactive<day_spec::ImageSource> {
                crate::Reactive::Const(self.into())
            }
        })*
    };
}

image_source_from_value!(
    day_spec::ImageSource,
    day_spec::ImageName,
    day_spec::VectorName,
    day_spec::BitmapId,
    // An owned string is a runtime-computed NAME (`ImageName::dynamic`'s spelling), never bytes.
    // `&str` is deliberately absent, so `image("typo")` still fails to compile.
    String,
    Vec<u8>,
    std::sync::Arc<Vec<u8>>,
);

/// A decoded image draws by id, so several nodes (and the canvas) share one decode.
/// A decoded handle. The piece keeps a CLONE alive for the node's life: the closure below owns
/// it, the binding owns the closure, and the node's scope owns the binding — so the toolkit's
/// image outlives the view that shows it however the app juggles its own handle. Without this an
/// `image(&bitmap)` would carry only the id, and on the web the `<img>` would point at an object
/// URL that `release_image` had already revoked.
impl IntoImageSource<ImageValueMark> for &day_core::Bitmap {
    fn into_image_source(self) -> crate::Reactive<day_spec::ImageSource> {
        let keep = self.clone();
        crate::Reactive::Dyn(std::rc::Rc::new(move || {
            day_spec::ImageSource::Decoded(keep.id())
        }))
    }
}

impl IntoImageSource<ImageSignalMark> for day_reactive::Signal<day_spec::ImageSource> {
    fn into_image_source(self) -> crate::Reactive<day_spec::ImageSource> {
        crate::Reactive::Dyn(std::rc::Rc::new(move || self.get()))
    }
}

impl<F: Fn() -> day_spec::ImageSource + 'static> IntoImageSource<ImageFnMark> for F {
    fn into_image_source(self) -> crate::Reactive<day_spec::ImageSource> {
        crate::Reactive::Dyn(std::rc::Rc::new(self))
    }
}

impl Image {
    /// How the image scales within its frame (default [`ContentMode::Fit`]).
    pub fn content_mode(mut self, m: ContentMode) -> Self {
        self.content_mode = m;
        self
    }
    /// Scale to fit entirely inside the frame, preserving aspect ratio (the default).
    pub fn fit(self) -> Self {
        self.content_mode(ContentMode::Fit)
    }
    /// Scale to fill the frame, preserving aspect ratio and cropping the overflow.
    pub fn fill(self) -> Self {
        self.content_mode(ContentMode::Fill)
    }
    /// Stretch to fill the frame exactly, ignoring aspect ratio.
    pub fn stretch(self) -> Self {
        self.content_mode(ContentMode::Stretch)
    }
    /// Constrain the view to a `width / height` ratio (e.g. `16.0 / 9.0`).
    pub fn aspect_ratio(mut self, ratio: f64) -> Self {
        if ratio > 0.0 {
            self.aspect_ratio = Some(ratio);
        }
        self
    }
    /// Mark the image decorative (hidden from accessibility).
    pub fn decorative(mut self) -> Self {
        self.decorative = true;
        self
    }
}

impl Piece for Image {
    fn build(self, cx: &mut BuildCx) -> day_core::RNode {
        let source = self.source;
        let seed = source.get_untracked();
        let props = ImageProps {
            source: seed.clone(),
            decorative: self.decorative,
            content_mode: self.content_mode,
            aspect_ratio: self.aspect_ratio,
            tint: None,
        };
        let node = match self.aspect_ratio {
            Some(ratio) => cx.native(
                kinds::IMAGE,
                &props,
                std::rc::Rc::new(AspectRatioLayout { ratio }),
                Flex::default(),
                day_core::Boundary::No,
            ),
            None => cx.leaf(kinds::IMAGE, &props, Flex::default()),
        };
        // A constant source reads the same value forever, so this seeds once and never patches;
        // a signal or closure replaces the pixels in the view that is already on screen rather
        // than realizing a second one, which would flash (docs/images.md).
        day_reactive::bind_seeded(
            seed,
            move || source.get(),
            move |s: &day_spec::ImageSource| {
                day_core::with_tree(|t| {
                    t.patch(node, Box::new(ImagePatch::Source(s.clone())), true)
                });
            },
        );
        node
    }
}

// ---------------------------------------------------------------------------
// Vector (docs/vectors.md): a bundled vector glyph from `resource/vectors/`.
// ---------------------------------------------------------------------------

/// A bundled vector glyph, resolved by name through whatever form the backend loads natively
/// (§18.3: a VectorDrawable on Android, a catalog entry on Apple, an SVG on the web, a
/// build-rasterized PNG where the toolkit has no vector path). Distinct from [`image`] on
/// purpose: only a typed [`VectorName`](day_spec::VectorName) is accepted, and the modifiers
/// are the vector-appropriate ones — [`tint`](Vector::tint) recolors a monochrome glyph where
/// the backend can (template rendering on Apple, drawable tint on Android, pixel recolor on
/// GTK; backends without a tint path draw the authored colors).
pub struct Vector {
    source: String,
    tint: Option<crate::Reactive<day_spec::Color>>,
    weight: VectorWeight,
    decorative: bool,
}

/// A vector glyph's stroke weight (docs/vectors.md). Template-form sources (SF template SVGs,
/// `.symbolset` bundles) carry true per-weight art; plain SVGs alias every weight to the same
/// glyph, so `.weight(…)` degrades to Regular rather than to a missing asset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VectorWeight {
    Light,
    #[default]
    Regular,
    Bold,
}

pub fn vector(name: impl Into<day_spec::VectorName>) -> Vector {
    Vector {
        source: name.into().as_str().to_owned(),
        tint: None,
        weight: VectorWeight::Regular,
        decorative: false,
    }
}

impl Vector {
    /// Recolor the glyph (monochrome art) where the backend supports tinting.
    ///
    /// Takes a plain [`Color`](day_spec::Color) or anything reactive: a signal or closure repaints
    /// the realized glyph through [`ImagePatch::Tint`](day_spec::props::ImagePatch) instead of
    /// rebuilding it, so a glyph that follows the selection or the theme keeps its native view.
    pub fn tint<M>(mut self, color: impl crate::IntoReactive<day_spec::Color, M>) -> Self {
        self.tint = Some(color.into_reactive());
        self
    }
    /// Select the glyph's weight (template-form sources render true weights; plain SVGs
    /// degrade to Regular — see [`VectorWeight`]).
    pub fn weight(mut self, w: VectorWeight) -> Self {
        self.weight = w;
        self
    }
    /// Mark the glyph decorative (hidden from accessibility).
    pub fn decorative(mut self) -> Self {
        self.decorative = true;
        self
    }
}

impl Piece for Vector {
    fn build(self, cx: &mut BuildCx) -> day_core::RNode {
        // Weight variants stage under suffixed resolution names (docs/vectors.md).
        let source = match self.weight {
            VectorWeight::Regular => self.source,
            VectorWeight::Light => format!("{}__light", self.source),
            VectorWeight::Bold => format!("{}__bold", self.source),
        };
        let tint = self.tint;
        let seed = tint.as_ref().map(|t| t.get_untracked());
        let props = ImageProps {
            source: day_spec::ImageSource::Named(source),
            decorative: self.decorative,
            content_mode: ContentMode::Fit,
            aspect_ratio: None,
            tint: seed,
        };
        let node = cx.leaf(kinds::IMAGE, &props, Flex::default());
        // A constant tint reads the same value forever, so this seeds once and never patches; a
        // signal or closure re-runs and repaints the realized glyph.
        if let Some(tint) = tint
            && let Some(seed) = seed
        {
            day_reactive::bind_seeded(
                seed,
                move || tint.get(),
                move |c: &day_spec::Color| {
                    day_core::with_tree(|t| {
                        t.patch(
                            node,
                            Box::new(day_spec::props::ImagePatch::Tint(Some(*c))),
                            false,
                        )
                    });
                },
            );
        }
        node
    }
}

// --- Typed builders, forwarded through `Decorated` (docs/api-style.md) ---

/// [`Image`]'s own builders, reachable THROUGH a decoration (§5.2): `Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait ImageBuilder: Sized {
    fn content_mode(self, m: ContentMode) -> Self;
    fn fit(self) -> Self;
    fn fill(self) -> Self;
    fn stretch(self) -> Self;
    fn decorative(self) -> Self;
}

impl ImageBuilder for Image {
    fn content_mode(self, m: ContentMode) -> Self {
        Image::content_mode(self, m)
    }
    fn fit(self) -> Self {
        Image::fit(self)
    }
    fn fill(self) -> Self {
        Image::fill(self)
    }
    fn stretch(self) -> Self {
        Image::stretch(self)
    }
    fn decorative(self) -> Self {
        Image::decorative(self)
    }
}

impl<Inner: ImageBuilder + Piece> ImageBuilder for Decorated<Inner> {
    fn content_mode(self, m: ContentMode) -> Self {
        self.map_inner(|inner_piece| inner_piece.content_mode(m))
    }
    fn fit(self) -> Self {
        self.map_inner(|inner_piece| inner_piece.fit())
    }
    fn fill(self) -> Self {
        self.map_inner(|inner_piece| inner_piece.fill())
    }
    fn stretch(self) -> Self {
        self.map_inner(|inner_piece| inner_piece.stretch())
    }
    fn decorative(self) -> Self {
        self.map_inner(|inner_piece| inner_piece.decorative())
    }
}

/// [`Vector`]'s own builders, reachable THROUGH a decoration (§5.2): `Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait VectorBuilder: Sized {
    fn tint<M>(self, color: impl crate::IntoReactive<day_spec::Color, M>) -> Self;
    fn weight(self, w: VectorWeight) -> Self;
    fn decorative(self) -> Self;
}

impl VectorBuilder for Vector {
    fn tint<M>(self, color: impl crate::IntoReactive<day_spec::Color, M>) -> Self {
        Vector::tint(self, color)
    }
    fn weight(self, w: VectorWeight) -> Self {
        Vector::weight(self, w)
    }
    fn decorative(self) -> Self {
        Vector::decorative(self)
    }
}

impl<Inner: VectorBuilder + Piece> VectorBuilder for Decorated<Inner> {
    fn tint<M>(self, color: impl crate::IntoReactive<day_spec::Color, M>) -> Self {
        self.map_inner(|inner_piece| inner_piece.tint(color))
    }
    fn weight(self, w: VectorWeight) -> Self {
        self.map_inner(|inner_piece| inner_piece.weight(w))
    }
    fn decorative(self) -> Self {
        self.map_inner(|inner_piece| inner_piece.decorative())
    }
}
