// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The `image` piece: loads a named asset (resolved from the dev asset root, the app bundle, or
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
    template: bool,
}

/// Draw an image from any [`ImageSource`](day_spec::ImageSource): a staged asset name, encoded
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
        template: false,
    }
}

/// Disjoint-marker conversion into an image source (the same shape as
/// [`IntoText`](crate::IntoText), and for the same coherence reason): a staged name, a byte
/// buffer, a decoded [`Bitmap`](day_core::Bitmap), a `Signal`, or a closure all convert, each
/// under its own marker.
///
/// A blanket `impl<S: Into<ImageSource>>` over the existing
/// [`IntoReactive`](crate::IntoReactive) would be ambiguous for `ImageSource` itself (the
/// static blanket and the converting one would both apply and leave the marker unconstrained),
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
    // An owned string is a runtime-computed name (`ImageName::dynamic`'s spelling), never bytes.
    // `&str` is absent so that `image("typo")` still fails to compile.
    String,
    Vec<u8>,
    std::sync::Arc<Vec<u8>>,
);

/// A decoded image draws by id, so several nodes (and the canvas) share one decode.
/// A decoded handle. The piece keeps a clone alive for the node's life: the closure below owns
/// it, the binding owns the closure, and the node's scope owns the binding, so the toolkit's
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
    /// Draw a monochrome glyph as a template (docs/vectors.md "Template"): in the foreground
    /// of the surface it sits on, following the theme, rather than in its authored colors.
    /// What a row icon in a sidebar wants; a photo does not.
    pub fn template(mut self) -> Self {
        self.template = true;
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
            template: self.template,
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
/// are the vector-appropriate ones: [`tint`](Vector::tint) recolors a monochrome glyph where
/// the backend can (template rendering on Apple, drawable tint on Android, pixel recolor on
/// GTK; backends without a tint path draw the authored colors).
pub struct Vector {
    source: String,
    tint: Option<crate::Reactive<day_spec::Color>>,
    weight: VectorWeight,
    decorative: bool,
    template: bool,
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
        template: false,
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
    /// degrade to Regular; see [`VectorWeight`]).
    pub fn weight(mut self, w: VectorWeight) -> Self {
        self.weight = w;
        self
    }
    /// Mark the glyph decorative (hidden from accessibility).
    pub fn decorative(mut self) -> Self {
        self.decorative = true;
        self
    }
    /// Draw the glyph as a template (docs/vectors.md "Template"): untinted, it takes the
    /// foreground of the surface it sits on and follows the theme, instead of its authored
    /// colors. A `.tint(…)` still wins while it is set.
    pub fn template(mut self) -> Self {
        self.template = true;
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
            template: self.template,
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

/// [`Image`]'s own builders, reachable through a decoration (§5.2): `Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait ImageBuilder: Sized {
    fn content_mode(self, m: ContentMode) -> Self;
    fn fit(self) -> Self;
    fn fill(self) -> Self;
    fn stretch(self) -> Self;
    fn decorative(self) -> Self;
    fn template(self) -> Self;
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
    fn template(self) -> Self {
        Image::template(self)
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
    fn template(self) -> Self {
        self.map_inner(|inner_piece| inner_piece.template())
    }
}

/// [`Vector`]'s own builders, reachable through a decoration (§5.2): `Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait VectorBuilder: Sized {
    fn tint<M>(self, color: impl crate::IntoReactive<day_spec::Color, M>) -> Self;
    fn weight(self, w: VectorWeight) -> Self;
    fn decorative(self) -> Self;
    fn template(self) -> Self;
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
    fn template(self) -> Self {
        Vector::template(self)
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
    fn template(self) -> Self {
        self.map_inner(|inner_piece| inner_piece.template())
    }
}

// ---------------------------------------------------------------------------
// Conformance cases (docs/testing.md)
// ---------------------------------------------------------------------------

/// The image piece's `#[day::test]` cases, next to its constructor.
#[cfg(feature = "conformance")]
pub(crate) mod conformance {
    use day_core::conformance::{Case, Drive};
    use day_spec::{Cap, kinds};

    use crate::*;

    /// A 4×2 PNG: the left half red, the right half blue. Encoded once by hand (zlib level 9),
    /// so the case needs no image resource and every toolkit decodes the same bytes.
    const RED_BLUE: [u8; 76] = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0xf0,
        0xca, 0xea, 0x34, 0x00, 0x00, 0x00, 0x13, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8,
        0xcf, 0xc0, 0x00, 0x44, 0x60, 0xe2, 0x3f, 0x03, 0x32, 0x07, 0x00, 0x67, 0xb2, 0x07, 0xf9,
        0xf7, 0x92, 0x56, 0x82, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60,
        0x82,
    ];

    /// Encoded bytes decode natively and draw at the frame Day gives them, each half its color.
    #[day_macros::test(day_core)]
    fn image_bytes() -> Case {
        Case::new()
            .proves(kinds::IMAGE)
            .proves_cap(Cap::ImageDecode)
            .proves_modifier("frame")
            .page(|| {
                image(RED_BLUE.to_vec())
                    .stretch()
                    .id("image")
                    .frame(120.0, 60.0)
            })
            .shot("default")
            .drive(|d: Drive| async move {
                d.assert_size("image", 120.0, 60.0).await?;
                d.sample_pixel("image", 0.2, 0.5, "#ff0000").await?;
                d.sample_pixel("image", 0.8, 0.5, "#0000ff").await
            })
    }

    /// Fit keeps the image's 2:1 shape inside a square frame: the middle row still shows both
    /// halves, in order.
    #[day_macros::test(day_core)]
    fn image_fit() -> Case {
        Case::new()
            .proves(kinds::IMAGE)
            .page(|| {
                image(RED_BLUE.to_vec())
                    .fit()
                    .id("image")
                    .frame(100.0, 100.0)
            })
            .drive(|d: Drive| async move {
                d.assert_size("image", 100.0, 100.0).await?;
                d.sample_pixel("image", 0.25, 0.5, "#ff0000").await?;
                d.sample_pixel("image", 0.75, 0.5, "#0000ff").await
            })
    }

    /// A decorative image is hidden from assistive technology; a described one is announced.
    #[day_macros::test(day_core)]
    fn image_a11y() -> Case {
        Case::new()
            .proves(kinds::IMAGE)
            .page(|| {
                image(RED_BLUE.to_vec())
                    .a11y(|a| a.label("Red and blue"))
                    .id("image")
                    .frame(40.0, 20.0)
            })
            .drive(|d: Drive| async move { d.a11y_audit(Some("image")).await })
    }

    day_core::tests! { image_bytes, image_fit, image_a11y }
}
