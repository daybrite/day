// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Images decoded from reactive bytes, with a placeholder and optional clipping.
//! Uses the core decoder and canvas; no toolkit-specific image pipeline or HTTP dependency.
use crate::{Decorate, Decorated, canvas};
use day_core::{BuildCx, Piece, RNode, TaskHandle};
use day_reactive::{Scope, Signal, watch};
use day_spec::{Color, Rect, Shape};
use std::{cell::RefCell, rc::Rc, sync::Arc};

pub use day_spec::props::ContentMode;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Clip {
    #[default]
    None,
    Circle,
    Rounded(f64),
}

/// A growing image surface. Empty and invalid sources show the placeholder.
pub struct AsyncImage {
    source: Signal<Option<Arc<Vec<u8>>>>,
    clip: Clip,
    mode: ContentMode,
    placeholder: Color,
}

/// Decode each source asynchronously. A replacement cancels the previous decode; disposal
/// cancels the pending task. A failed replacement clears the previous picture.
pub fn async_image(source: Signal<Option<Arc<Vec<u8>>>>) -> AsyncImage {
    AsyncImage {
        source,
        clip: Clip::None,
        mode: ContentMode::Fill,
        placeholder: Color::rgb(0.82, 0.82, 0.84),
    }
}

#[crate::builder(AsyncImageBuilder)]
impl AsyncImage {
    /// Clip to the centered circle whose diameter is the shorter side.
    pub fn circle(mut self) -> Self {
        self.clip = Clip::Circle;
        self
    }
    /// Clip to a rounded rectangle.
    pub fn rounded(mut self, radius: f64) -> Self {
        self.clip = Clip::Rounded(radius.max(0.0));
        self
    }
    /// Scale within the frame; defaults to aspect fill.
    pub fn content_mode(mut self, mode: ContentMode) -> Self {
        self.mode = mode;
        self
    }
    /// Color displayed while empty, loading, or invalid.
    pub fn placeholder_color(mut self, color: Color) -> Self {
        self.placeholder = color;
        self
    }
}

impl Piece for AsyncImage {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Self {
            source,
            clip,
            mode,
            placeholder,
        } = self;
        let bitmap: Signal<Option<day_core::Bitmap>> = Signal::new(None);
        let pending: Rc<RefCell<Option<TaskHandle>>> = Rc::new(RefCell::new(None));
        let cancel = pending.clone();
        Scope::current().on_cleanup(move || {
            if let Some(task) = cancel.borrow_mut().take() {
                task.abort();
            }
        });
        let load = move |bytes: Option<Arc<Vec<u8>>>| {
            if let Some(task) = pending.borrow_mut().take() {
                task.abort();
            }
            bitmap.set(None);
            if let Some(bytes) = bytes {
                let task = day_core::task(async move {
                    bitmap.set(day_core::decode_image(bytes).await.ok());
                });
                *pending.borrow_mut() = Some(task);
            }
        };
        load(source.get_untracked());
        watch(move || source.get(), move |bytes, _| load(bytes.clone()));
        canvas(move |d, size| {
            let bounds = Rect::from_size(size);
            let shape = match clip {
                Clip::None => Shape::Rect(bounds),
                Clip::Rounded(radius) => Shape::RoundedRect(bounds, radius),
                Clip::Circle => {
                    let side = size.width.min(size.height);
                    Shape::Ellipse(Rect::new(
                        (size.width - side) / 2.0,
                        (size.height - side) / 2.0,
                        side,
                        side,
                    ))
                }
            };
            d.clipped(shape, |d| {
                d.fill(Shape::Rect(bounds), placeholder);
                if let Some(image) = bitmap.get() {
                    let pixels = image.info().pixels;
                    if pixels.width <= 0.0 || pixels.height <= 0.0 {
                        return;
                    }
                    let rect = if mode == ContentMode::Stretch {
                        bounds
                    } else {
                        let x = size.width / pixels.width;
                        let y = size.height / pixels.height;
                        let scale = if mode == ContentMode::Fill {
                            x.max(y)
                        } else {
                            x.min(y)
                        };
                        let w = pixels.width * scale;
                        let h = pixels.height * scale;
                        Rect::new((size.width - w) / 2.0, (size.height - h) / 2.0, w, h)
                    };
                    d.image(&image, rect);
                }
            });
        })
        .grow()
        .build(cx)
    }
}

/// The remote image's conformance cases (docs/testing.md): the bytes path, which needs no
/// network.
#[cfg(feature = "conformance")]
pub mod conformance {
    use std::sync::Arc;

    use crate::*;
    use day_core::conformance::{Case, Drive};
    use day_reactive::Signal;
    use day_spec::{Cap, Color};

    use super::{ContentMode, async_image};

    /// A 4×2 PNG, red on the left half, blue on the right.
    const RED_BLUE: [u8; 76] = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0xf0,
        0xca, 0xea, 0x34, 0x00, 0x00, 0x00, 0x13, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8,
        0xcf, 0xc0, 0x00, 0x44, 0x60, 0xe2, 0x3f, 0x03, 0x32, 0x07, 0x00, 0x67, 0xb2, 0x07, 0xf9,
        0xf7, 0x92, 0x56, 0x82, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60,
        0x82,
    ];

    /// A 1×1 opaque green PNG, deliberately a different size as well as a different color.
    const GREEN: [u8; 69] = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60,
        0xf8, 0xcf, 0x00, 0x00, 0x02, 0x02, 0x01, 0x00, 0x7b, 0x09, 0x81, 0x78, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    /// A 60×20 PNG with equal red, green, blue vertical bands. Filling a square crops to
    /// green; fitting or stretching preserves the red/blue edges, so these modes differ.
    const STRIPES: [u8; 104] = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x3c, 0x00, 0x00, 0x00, 0x14, 0x08, 0x02, 0x00, 0x00, 0x00, 0x5e,
        0x61, 0xc9, 0x7e, 0x00, 0x00, 0x00, 0x2f, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0xed, 0xce,
        0xb1, 0x09, 0x00, 0x00, 0x0c, 0xc3, 0xb0, 0xfc, 0xff, 0x74, 0xfb, 0x43, 0x4a, 0x37, 0x81,
        0x57, 0x83, 0x32, 0x49, 0x5d, 0x7f, 0x1e, 0x67, 0x68, 0x68, 0x68, 0x68, 0x68, 0x68, 0x68,
        0x68, 0x68, 0x68, 0x68, 0x68, 0xe8, 0x8f, 0x79, 0x01, 0x39, 0x0f, 0xab, 0x8d, 0x85, 0x9c,
        0x89, 0x2d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    /// Until bytes arrive the placeholder color shows; once they do, the image draws.
    #[day_macros::test(day_core)]
    fn async_image_shows_bytes() -> Case {
        Case::new()
            .proves(day_spec::kinds::CANVAS)
            .requires(Cap::Snapshot)
            .page(|| {
                let source: Signal<Option<Arc<Vec<u8>>>> = Signal::new(None);
                column((
                    button("Load")
                        .action(move || source.set(Some(Arc::new(RED_BLUE.to_vec()))))
                        .id(day_core::id!(async_image::load)),
                    async_image(source)
                        .placeholder_color(Color::rgb(0.5, 0.5, 0.5))
                        .id(day_core::id!(async_image::image))
                        .frame(120.0, 60.0),
                ))
            })
            .drive(|d: Drive| async move {
                // Gray: a saturated placeholder shifts under a wide-gamut capture (iOS).
                d.sample_pixel("async_image::image", 0.5, 0.5, "#808080")
                    .await?;
                d.tap("async_image::load").await?;
                d.sample_pixel("async_image::image", 0.2, 0.5, "#ff0000")
                    .await?;
                d.sample_pixel("async_image::image", 0.8, 0.5, "#0000ff")
                    .await
            })
    }

    /// A replacement must not keep the old pixels; absent, empty and corrupt sources all
    /// restore the placeholder. Unmounting must release native resources and reactive watchers.
    #[day_macros::test(day_core)]
    fn async_image_replaces_clears_and_remounts() -> Case {
        Case::new()
            .proves(day_spec::kinds::CANVAS)
            .requires(Cap::Snapshot)
            .page(|| {
                let source = Signal::new(Some(Arc::new(RED_BLUE.to_vec())));
                let shown = Signal::new(true);
                column((
                    row((
                        button("Green")
                            .action(move || source.set(Some(Arc::new(GREEN.to_vec()))))
                            .id(day_core::id!(async_image::green)),
                        button("Clear")
                            .action(move || source.set(None))
                            .id(day_core::id!(async_image::clear)),
                        button("Empty")
                            .action(move || source.set(Some(Arc::new(Vec::new()))))
                            .id(day_core::id!(async_image::empty)),
                    )),
                    row((
                        button("Invalid")
                            .action(move || source.set(Some(Arc::new(vec![1, 2, 3]))))
                            .id(day_core::id!(async_image::invalid)),
                        button("Mount")
                            .action(move || shown.set(!shown.get()))
                            .id(day_core::id!(async_image::mount)),
                    )),
                    when(
                        move || shown.get(),
                        move || {
                            async_image(source)
                                .placeholder_color(Color::hex(0x808080))
                                .id(day_core::id!(async_image::image))
                                .frame(120.0, 60.0)
                        },
                    ),
                ))
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("async_image::image", 0.2, 0.5, "#ff0000")
                    .await?;
                for action in [
                    "async_image::clear",
                    "async_image::empty",
                    "async_image::invalid",
                ] {
                    d.tap("async_image::green").await?;
                    d.sample_pixel("async_image::image", 0.2, 0.5, "#00ff00")
                        .await?;
                    d.tap(action).await?;
                    d.sample_pixel("async_image::image", 0.2, 0.5, "#808080")
                        .await?;
                }
                for _ in 0..3 {
                    d.tap("async_image::mount").await?;
                    d.assert_missing("async_image::image").await?;
                    d.tap("async_image::green").await?; // Update while disposed: no stale native callback.
                    d.tap("async_image::mount").await?;
                    d.sample_pixel("async_image::image", 0.8, 0.5, "#00ff00")
                        .await?;
                    d.tap("async_image::clear").await?;
                    d.sample_pixel("async_image::image", 0.8, 0.5, "#808080")
                        .await?;
                }
                Ok(())
            })
    }

    /// Fit letterboxes; Fill covers the frame without stretching the image.
    #[day_macros::test(day_core)]
    fn async_image_fit_and_fill() -> Case {
        Case::new()
            .proves(day_spec::kinds::CANVAS)
            .requires(Cap::Snapshot)
            .page(|| {
                let source = Signal::new(Some(Arc::new(STRIPES.to_vec())));
                column((
                    async_image(source)
                        .content_mode(ContentMode::Fit)
                        .placeholder_color(Color::hex(0x808080))
                        .id(day_core::id!(async_image::fit))
                        .frame(100.0, 100.0),
                    async_image(source)
                        .content_mode(ContentMode::Fill)
                        .placeholder_color(Color::hex(0x808080))
                        .id(day_core::id!(async_image::fill))
                        .frame(100.0, 100.0),
                ))
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("async_image::fit", 0.2, 0.1, "#808080")
                    .await?;
                d.sample_pixel("async_image::fit", 0.2, 0.5, "#ff0000")
                    .await?;
                d.sample_pixel("async_image::fit", 0.8, 0.5, "#0000ff")
                    .await?;
                d.sample_pixel("async_image::fill", 0.1, 0.1, "#00ff00")
                    .await?;
                d.sample_pixel("async_image::fill", 0.9, 0.9, "#00ff00")
                    .await?;
                // Pixel assertions wait for the native render checkpoint; capture afterwards.
                d.shot("default").await
            })
    }

    /// A circle is centered with diameter min(width, height), not a capsule/ellipse. Both
    /// image and placeholder are clipped; a rounded rectangle must keep its straight edges.
    #[day_macros::test(day_core)]
    fn async_image_clips_image_and_placeholder() -> Case {
        Case::new()
            .proves(day_spec::kinds::CANVAS)
            .requires(Cap::Snapshot)
            .page(|| {
                let source = Signal::new(Some(Arc::new(GREEN.to_vec())));
                column((
                    button("Clear")
                        .action(move || source.set(None))
                        .id(day_core::id!(async_image::clear)),
                    async_image(source)
                        .circle()
                        .placeholder_color(Color::hex(0x808080))
                        .id(day_core::id!(async_image::wide))
                        .frame(120.0, 60.0),
                    async_image(source)
                        .circle()
                        .placeholder_color(Color::hex(0x808080))
                        .id(day_core::id!(async_image::tall))
                        .frame(60.0, 120.0),
                    async_image(source)
                        .rounded(20.0)
                        .placeholder_color(Color::hex(0x808080))
                        .id(day_core::id!(async_image::rounded))
                        .frame(120.0, 60.0),
                ))
                .background(Color::hex(0x000000))
            })
            .drive(|d: Drive| async move {
                for color in ["#00ff00", "#808080"] {
                    for id in [
                        "async_image::wide",
                        "async_image::tall",
                        "async_image::rounded",
                    ] {
                        d.sample_pixel(id, 0.5, 0.5, color).await?;
                        d.sample_pixel(id, 0.02, 0.02, "#000000").await?;
                    }
                    d.sample_pixel("async_image::wide", 0.1, 0.5, "#000000")
                        .await?;
                    d.sample_pixel("async_image::tall", 0.5, 0.1, "#000000")
                        .await?;
                    d.sample_pixel("async_image::rounded", 0.1, 0.5, color)
                        .await?;
                    d.shot(if color == "#00ff00" {
                        "default"
                    } else {
                        "placeholder"
                    })
                    .await?;
                    d.tap("async_image::clear").await?;
                }
                Ok(())
            })
    }

    /// Resizing the same native view must move its circular clip, without rebuilding it.
    #[day_macros::test(day_core)]
    fn async_image_circle_follows_layout() -> Case {
        Case::new()
            .proves(day_spec::kinds::CANVAS)
            .requires(Cap::Snapshot)
            .page(|| {
                let source = Signal::new(Some(Arc::new(GREEN.to_vec())));
                let narrow = Signal::new(false);
                // Keep the sampled view directly in the root row: sample_pixel uses the
                // Day frame, which is relative to the nearest native parent.
                row((
                    async_image(source)
                        .circle()
                        .id(day_core::id!(async_image::image)),
                    when(move || narrow.get(), || label("").frame(60.0, 60.0)),
                    button("Resize")
                        .action(move || narrow.set(!narrow.get()))
                        .id(day_core::id!(async_image::resize))
                        .frame(80.0, 60.0),
                ))
                .spacing(0.0)
                .frame(260.0, 60.0)
                .background(Color::hex(0x000000))
            })
            .drive(|d: Drive| async move {
                for width in [180.0, 120.0, 180.0] {
                    d.assert_size("async_image::image", width, 60.0).await?;
                    d.sample_pixel("async_image::image", 0.5, 0.5, "#00ff00")
                        .await?;
                    d.sample_pixel("async_image::image", 0.1, 0.5, "#000000")
                        .await?;
                    // A stale 180-wide clip would miss this point after shrinking to 120.
                    d.sample_pixel("async_image::image", 0.4, 0.5, "#00ff00")
                        .await?;
                    d.tap("async_image::resize").await?;
                }
                Ok(())
            })
    }
}
