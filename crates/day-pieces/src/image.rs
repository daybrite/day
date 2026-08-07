//! The `image` piece — loads a named asset (resolved from the dev asset root, the app bundle, or
//! Android's `AssetManager`) with content-mode and aspect-ratio fitting.

use day_core::*;
use day_spec::kinds;
use day_spec::props::*;

// ---------------------------------------------------------------------------
// Image (§18.2, MVP): sources resolve via DAY_ASSET_ROOT (desktop dev), the app
// bundle (ios), or AssetManager (android).
// ---------------------------------------------------------------------------

/// A bundled image, resolved by name through the backend's native image pipeline (§18.3). Scales
/// with [`ContentMode::Fit`] by default (never stretches); tune with `.content_mode()` / `.fill()` /
/// `.stretch()`, and optionally constrain the frame with `.aspect_ratio(w/h)`.
pub struct Image {
    source: String,
    content_mode: ContentMode,
    aspect_ratio: Option<f64>,
    decorative: bool,
}

pub fn image(name: impl Into<day_spec::ImageName>) -> Image {
    Image {
        source: name.into().as_str().to_owned(),
        content_mode: ContentMode::default(),
        aspect_ratio: None,
        decorative: false,
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
        let props = ImageProps {
            source: self.source,
            decorative: self.decorative,
            content_mode: self.content_mode,
            aspect_ratio: self.aspect_ratio,
            tint: None,
        };
        match self.aspect_ratio {
            Some(ratio) => cx.native(
                kinds::IMAGE,
                &props,
                std::rc::Rc::new(AspectRatioLayout { ratio }),
                Flex::default(),
                day_core::Boundary::No,
            ),
            None => cx.leaf(kinds::IMAGE, &props, Flex::default()),
        }
    }
}

/// Self-measuring layout for `.aspect_ratio(r)`: reports the largest `width/height == r` box that
/// fits the proposal (SwiftUI's `.aspectRatio(_:contentMode: .fit)`).
struct AspectRatioLayout {
    ratio: f64,
}
impl day_core::Layout for AspectRatioLayout {
    fn measure(
        &self,
        cx: &mut dyn day_core::LayoutOps,
        _children: &[day_core::RNode],
        p: day_geometry::Proposal,
    ) -> day_geometry::Size {
        match (p.width, p.height) {
            (Some(w), Some(h)) => {
                if w / h > self.ratio {
                    day_geometry::Size::new(h * self.ratio, h)
                } else {
                    day_geometry::Size::new(w, w / self.ratio)
                }
            }
            (Some(w), None) => day_geometry::Size::new(w, w / self.ratio),
            (None, Some(h)) => day_geometry::Size::new(h * self.ratio, h),
            (None, None) => cx.measure_leaf(p),
        }
    }
    fn place(
        &self,
        _cx: &mut dyn day_core::LayoutOps,
        _children: &[day_core::RNode],
        _bounds: day_geometry::Rect,
    ) {
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
    tint: Option<day_spec::Color>,
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
    /// Recolor the glyph (monochrome art) to `color` where the backend supports tinting.
    pub fn tint(mut self, color: day_spec::Color) -> Self {
        self.tint = Some(color);
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
        let props = ImageProps {
            source,
            decorative: self.decorative,
            content_mode: ContentMode::Fit,
            aspect_ratio: None,
            tint: self.tint,
        };
        cx.leaf(kinds::IMAGE, &props, Flex::default())
    }
}
