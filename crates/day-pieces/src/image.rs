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
