// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-piece-remote-image: an external Day Piece (DESIGN.md §15), a native image view that decodes
//! encoded bytes (PNG/JPEG) supplied **reactively** and draws them, with a placeholder rectangle
//! while the source is empty. This is the image primitive a Matrix client needs (avatars and inline
//! image messages), where the app fetches `mxc://` bytes off the UI thread via the SDK and pushes
//! them into a signal; this piece only turns bytes into a native image.
//!
//! The source is `Signal<Option<Arc<Vec<u8>>>>`: `None` shows the placeholder color;
//! `Some(bytes)` decodes and displays, restoring the placeholder if decoding fails. Day's
//! built-in `image` also accepts bytes, but keeps the preceding image on a failed replacement.
//! The `Arc<Vec<u8>>` is shared, so pushing a large buffer from a background fetch is a
//! refcount bump, not a copy. The piece is a growing leaf that fills its frame (constrain it with
//! `.frame(w, h)`); avatars occupy their box even before any bytes arrive.
//!
//! ```ignore
//! let avatar: Signal<Option<Arc<Vec<u8>>>> = Signal::new(None);
//! // …app pushes decoded bytes later: avatar.set(Some(Arc::new(png_bytes)));…
//! remote_image(avatar).circle().placeholder_color(Color::hex(0xD0D0D0)).frame(40.0, 40.0)
//! ```

use std::sync::Arc;

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_pieces::Decorated;
use day_reactive::{Signal, watch};
use day_spec::Color;

pub const KIND: &str = "day.piece.remote_image";

/// How the decoded image is scaled into the piece's frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContentMode {
    /// Scale to fill the frame, preserving aspect ratio and cropping the overflow (the default,
    /// what avatars want). SwiftUI's `.scaledToFill`.
    #[default]
    Fill,
    /// Scale to fit entirely inside the frame, preserving aspect ratio; the placeholder color shows
    /// in the letterboxed margins. SwiftUI's `.scaledToFit`.
    Fit,
}

/// How the piece (image and placeholder) is clipped to its frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Clip {
    /// No clipping: a plain rectangle (the default).
    #[default]
    None,
    /// A centered circle of diameter `min(width, height)`, for avatars.
    Circle,
    /// A rounded rectangle with the given uniform corner radius (points).
    Rounded(f64),
}

/// A light neutral gray shown while the source is `None` or a decode fails, so an avatar always
/// occupies its space. Override with [`RemoteImage::placeholder_color`].
const DEFAULT_PLACEHOLDER: Color = Color::rgb(0.82, 0.82, 0.84);

/// Full props (realize). `bytes` seeds the view; all four are applied at build. Only `bytes` changes
/// after build, via [`RemoteImagePatch::SetBytes`].
#[derive(Clone, PartialEq)]
pub struct RemoteImageProps {
    /// The initial encoded (PNG/JPEG) image bytes, or `None` for the placeholder.
    pub bytes: Option<Arc<Vec<u8>>>,
    /// Clip shape for both the image and the placeholder.
    pub clip: Clip,
    /// Aspect scaling of the decoded image within the frame.
    pub mode: ContentMode,
    /// Fill color shown while `bytes` is `None` or on decode failure.
    pub placeholder: Color,
}

impl Default for RemoteImageProps {
    fn default() -> Self {
        RemoteImageProps {
            bytes: None,
            clip: Clip::None,
            mode: ContentMode::Fill,
            placeholder: DEFAULT_PLACEHOLDER,
        }
    }
}

// Debug that elides the (potentially megabyte) byte buffer: logs the length, not the contents.
impl std::fmt::Debug for RemoteImageProps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteImageProps")
            .field("bytes", &self.bytes.as_ref().map(|b| b.len()))
            .field("clip", &self.clip)
            .field("mode", &self.mode)
            .field("placeholder", &self.placeholder)
            .finish()
    }
}

/// The single imperative update: replace the displayed bytes (or clear to the placeholder).
#[derive(Clone, PartialEq)]
pub enum RemoteImagePatch {
    SetBytes(Option<Arc<Vec<u8>>>),
}

impl std::fmt::Debug for RemoteImagePatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let RemoteImagePatch::SetBytes(b) = self;
        f.debug_tuple("SetBytes")
            .field(&b.as_ref().map(|b| b.len()))
            .finish()
    }
}

/// A native image view whose content mirrors `source` (a `Signal<Option<Arc<Vec<u8>>>>`). Shape it
/// with `.circle()` / `.rounded(r)`, scale it with `.content_mode(_)`, and set the empty-state color
/// with `.placeholder_color(_)`.
pub struct RemoteImage {
    source: Signal<Option<Arc<Vec<u8>>>>,
    clip: Clip,
    mode: ContentMode,
    placeholder: Color,
}

/// `remote_image_url(url)`: [`remote_image`] with the fetch built in. The bytes are downloaded
/// once through the platform HTTP stack (`day-part-http`, system proxies/VPN/Low-Data aware,
/// docs/http.md) on a background completion, and pushed into the piece's own signal via a
/// `Setter` (so a late arrival after the piece is disposed is a harmless no-op). The placeholder
/// shows until the bytes land; a failed fetch (or non-2xx status) leaves the placeholder.
///
/// ```ignore
/// remote_image_url("https://example.com/logo.png").rounded(8.0).frame(96.0, 96.0)
/// ```
pub fn remote_image_url(url: impl Into<String>) -> RemoteImage {
    let source: Signal<Option<Arc<Vec<u8>>>> = Signal::new(None);
    let done = source.setter();
    day_part_http::fetch_async(day_part_http::Request::get(url), move |result| {
        if let Ok(resp) = result
            && (200..300).contains(&resp.status)
        {
            done.set(Some(Arc::new(resp.body)));
        }
    });
    remote_image(source)
}

/// `remote_image(source)`: a native image view that decodes and displays the bytes held in
/// `source`, showing the placeholder color whenever `source` is `None`.
pub fn remote_image(source: Signal<Option<Arc<Vec<u8>>>>) -> RemoteImage {
    RemoteImage {
        source,
        clip: Clip::None,
        mode: ContentMode::Fill,
        placeholder: DEFAULT_PLACEHOLDER,
    }
}

#[day_pieces::builder(RemoteImageBuilder)]
impl RemoteImage {
    /// Clip to a centered circle of diameter `min(width, height)`, for avatars.
    pub fn circle(mut self) -> Self {
        self.clip = Clip::Circle;
        self
    }

    /// Clip to a rounded rectangle with the given uniform corner radius (points).
    pub fn rounded(mut self, radius: f64) -> Self {
        self.clip = Clip::Rounded(radius);
        self
    }

    /// Aspect scaling of the decoded image within the frame (default [`ContentMode::Fill`]).
    pub fn content_mode(mut self, mode: ContentMode) -> Self {
        self.mode = mode;
        self
    }

    /// Fill color shown while the source is `None` or a decode fails (default a light gray).
    pub fn placeholder_color(mut self, color: Color) -> Self {
        self.placeholder = color;
        self
    }
}

impl Piece for RemoteImage {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let RemoteImage {
            source,
            clip,
            mode,
            placeholder,
        } = self;
        // Seed the native view with whatever the signal already holds; subsequent changes arrive
        // as SetBytes patches (watch never fires for this initial value: §4.2's no-duplicate-op
        // rule).
        let initial = source.get_untracked();
        let node = cx.leaf(
            KIND,
            &RemoteImageProps {
                bytes: initial,
                clip,
                mode,
                placeholder,
            },
            // No intrinsic size; fills whatever frame its container offers (the app constrains it
            // with `.frame`, e.g. a 40×40 avatar).
            Flex {
                grow_w: true,
                grow_h: true,
                ..Default::default()
            },
        );

        // Push each new buffer from the signal into the native view. A background decode setting the
        // *same* Arc again (identity-equal) is a no-op, so we don't re-decode on redundant pushes.
        watch(
            move || source.get(),
            move |new: &Option<Arc<Vec<u8>>>, old: Option<&Option<Arc<Vec<u8>>>>| {
                if let (Some(n), Some(o)) = (new.as_ref(), old.and_then(|o| o.as_ref()))
                    && Arc::ptr_eq(n, o)
                {
                    return;
                }
                with_tree(|t| {
                    t.patch(
                        node,
                        Box::new(RemoteImagePatch::SetBytes(new.clone())),
                        false,
                    )
                });
            },
        );
        node
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers, one file per backend. Every module registers a `Renderer`
// link-time into its backend's `RENDERERS` slice; the `#[cfg]` gates each to its feature + target,
// and `#[path]` keeps the files grouped next to lib.rs (the day-piece-searchfield layout).
// ---------------------------------------------------------------------------

day_pieces::glue_modules!(appkit, gtk, qt, uikit, mdc, arkui, xaml);

// --- Typed builders, forwarded through `Decorated` (docs/api-style.md) ---

/// The remote image's conformance cases (docs/testing.md): the bytes path, which needs no
/// network.
#[cfg(feature = "conformance")]
pub mod conformance {
    use std::sync::Arc;

    use day_core::conformance::{Case, Drive};
    use day_pieces::*;
    use day_reactive::Signal;
    use day_spec::{Cap, Color};

    use super::{ContentMode, KIND, remote_image};

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
    fn remote_image_shows_bytes() -> Case {
        Case::new()
            .proves(KIND)
            .requires(Cap::Snapshot)
            .page(|| {
                let source: Signal<Option<Arc<Vec<u8>>>> = Signal::new(None);
                column((
                    button("Load")
                        .action(move || source.set(Some(Arc::new(RED_BLUE.to_vec()))))
                        .id(day_core::id!(remote_image::load)),
                    remote_image(source)
                        .placeholder_color(Color::rgb(0.5, 0.5, 0.5))
                        .id(day_core::id!(remote_image::image))
                        .frame(120.0, 60.0),
                ))
            })
            .drive(|d: Drive| async move {
                // Gray: a saturated placeholder shifts under a wide-gamut capture (iOS).
                d.sample_pixel("remote_image::image", 0.5, 0.5, "#808080")
                    .await?;
                d.tap("remote_image::load").await?;
                d.sample_pixel("remote_image::image", 0.2, 0.5, "#ff0000")
                    .await?;
                d.sample_pixel("remote_image::image", 0.8, 0.5, "#0000ff")
                    .await
            })
    }

    /// A replacement must not keep the old pixels; absent, empty and corrupt sources all
    /// restore the placeholder. Unmounting must release native resources and reactive watchers.
    #[day_macros::test(day_core)]
    fn remote_image_replaces_clears_and_remounts() -> Case {
        Case::new()
            .proves(KIND)
            .requires(Cap::Snapshot)
            .page(|| {
                let source = Signal::new(Some(Arc::new(RED_BLUE.to_vec())));
                let shown = Signal::new(true);
                column((
                    row((
                        button("Green")
                            .action(move || source.set(Some(Arc::new(GREEN.to_vec()))))
                            .id(day_core::id!(remote_image::green)),
                        button("Clear")
                            .action(move || source.set(None))
                            .id(day_core::id!(remote_image::clear)),
                        button("Empty")
                            .action(move || source.set(Some(Arc::new(Vec::new()))))
                            .id(day_core::id!(remote_image::empty)),
                    )),
                    row((
                        button("Invalid")
                            .action(move || source.set(Some(Arc::new(vec![1, 2, 3]))))
                            .id(day_core::id!(remote_image::invalid)),
                        button("Mount")
                            .action(move || shown.set(!shown.get()))
                            .id(day_core::id!(remote_image::mount)),
                    )),
                    when(
                        move || shown.get(),
                        move || {
                            remote_image(source)
                                .placeholder_color(Color::hex(0x808080))
                                .id(day_core::id!(remote_image::image))
                                .frame(120.0, 60.0)
                        },
                    ),
                ))
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("remote_image::image", 0.2, 0.5, "#ff0000")
                    .await?;
                for action in [
                    "remote_image::clear",
                    "remote_image::empty",
                    "remote_image::invalid",
                ] {
                    d.tap("remote_image::green").await?;
                    d.sample_pixel("remote_image::image", 0.2, 0.5, "#00ff00")
                        .await?;
                    d.tap(action).await?;
                    d.sample_pixel("remote_image::image", 0.2, 0.5, "#808080")
                        .await?;
                }
                for _ in 0..3 {
                    d.tap("remote_image::mount").await?;
                    d.assert_missing("remote_image::image").await?;
                    d.tap("remote_image::green").await?; // Update while disposed: no stale native callback.
                    d.tap("remote_image::mount").await?;
                    d.sample_pixel("remote_image::image", 0.8, 0.5, "#00ff00")
                        .await?;
                    d.tap("remote_image::clear").await?;
                    d.sample_pixel("remote_image::image", 0.8, 0.5, "#808080")
                        .await?;
                }
                Ok(())
            })
    }

    /// Fit letterboxes; Fill covers the frame without stretching the image.
    #[day_macros::test(day_core)]
    fn remote_image_fit_and_fill() -> Case {
        Case::new()
            .proves(KIND)
            .requires(Cap::Snapshot)
            .page(|| {
                let source = Signal::new(Some(Arc::new(STRIPES.to_vec())));
                column((
                    remote_image(source)
                        .content_mode(ContentMode::Fit)
                        .placeholder_color(Color::hex(0x808080))
                        .id(day_core::id!(remote_image::fit))
                        .frame(100.0, 100.0),
                    remote_image(source)
                        .content_mode(ContentMode::Fill)
                        .placeholder_color(Color::hex(0x808080))
                        .id(day_core::id!(remote_image::fill))
                        .frame(100.0, 100.0),
                ))
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("remote_image::fit", 0.2, 0.1, "#808080")
                    .await?;
                d.sample_pixel("remote_image::fit", 0.2, 0.5, "#ff0000")
                    .await?;
                d.sample_pixel("remote_image::fit", 0.8, 0.5, "#0000ff")
                    .await?;
                d.sample_pixel("remote_image::fill", 0.1, 0.1, "#00ff00")
                    .await?;
                d.sample_pixel("remote_image::fill", 0.9, 0.9, "#00ff00")
                    .await?;
                // Pixel assertions wait for the native render checkpoint; capture afterwards.
                d.shot("default").await
            })
    }

    /// A circle is centered with diameter min(width, height), not a capsule/ellipse. Both
    /// image and placeholder are clipped; a rounded rectangle must keep its straight edges.
    #[day_macros::test(day_core)]
    fn remote_image_clips_image_and_placeholder() -> Case {
        Case::new()
            .proves(KIND)
            .requires(Cap::Snapshot)
            .page(|| {
                let source = Signal::new(Some(Arc::new(GREEN.to_vec())));
                column((
                    button("Clear")
                        .action(move || source.set(None))
                        .id(day_core::id!(remote_image::clear)),
                    remote_image(source)
                        .circle()
                        .placeholder_color(Color::hex(0x808080))
                        .id(day_core::id!(remote_image::wide))
                        .frame(120.0, 60.0),
                    remote_image(source)
                        .circle()
                        .placeholder_color(Color::hex(0x808080))
                        .id(day_core::id!(remote_image::tall))
                        .frame(60.0, 120.0),
                    remote_image(source)
                        .rounded(20.0)
                        .placeholder_color(Color::hex(0x808080))
                        .id(day_core::id!(remote_image::rounded))
                        .frame(120.0, 60.0),
                ))
                .background(Color::hex(0x000000))
            })
            .drive(|d: Drive| async move {
                for color in ["#00ff00", "#808080"] {
                    for id in [
                        "remote_image::wide",
                        "remote_image::tall",
                        "remote_image::rounded",
                    ] {
                        d.sample_pixel(id, 0.5, 0.5, color).await?;
                        d.sample_pixel(id, 0.02, 0.02, "#000000").await?;
                    }
                    d.sample_pixel("remote_image::wide", 0.1, 0.5, "#000000")
                        .await?;
                    d.sample_pixel("remote_image::tall", 0.5, 0.1, "#000000")
                        .await?;
                    d.sample_pixel("remote_image::rounded", 0.1, 0.5, color)
                        .await?;
                    d.shot(if color == "#00ff00" {
                        "default"
                    } else {
                        "placeholder"
                    })
                    .await?;
                    d.tap("remote_image::clear").await?;
                }
                Ok(())
            })
    }

    /// Resizing the same native view must move its circular clip, without rebuilding it.
    #[day_macros::test(day_core)]
    fn remote_image_circle_follows_layout() -> Case {
        Case::new()
            .proves(KIND)
            .requires(Cap::Snapshot)
            .page(|| {
                let source = Signal::new(Some(Arc::new(GREEN.to_vec())));
                let narrow = Signal::new(false);
                // Keep the sampled view directly in the root row: sample_pixel uses the
                // Day frame, which is relative to the nearest native parent.
                row((
                    remote_image(source)
                        .circle()
                        .id(day_core::id!(remote_image::image)),
                    when(move || narrow.get(), || label("").frame(60.0, 60.0)),
                    button("Resize")
                        .action(move || narrow.set(!narrow.get()))
                        .id(day_core::id!(remote_image::resize))
                        .frame(80.0, 60.0),
                ))
                .spacing(0.0)
                .frame(260.0, 60.0)
                .background(Color::hex(0x000000))
            })
            .drive(|d: Drive| async move {
                for width in [180.0, 120.0, 180.0] {
                    d.assert_size("remote_image::image", width, 60.0).await?;
                    d.sample_pixel("remote_image::image", 0.5, 0.5, "#00ff00")
                        .await?;
                    d.sample_pixel("remote_image::image", 0.1, 0.5, "#000000")
                        .await?;
                    // A stale 180-wide clip would miss this point after shrinking to 120.
                    d.sample_pixel("remote_image::image", 0.4, 0.5, "#00ff00")
                        .await?;
                    d.tap("remote_image::resize").await?;
                }
                Ok(())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use day_mock::MockToolkit;
    use day_reactive::flush_sync;
    use day_spec::{Size, WindowOptions};

    // Building + pushing bytes must never panic, even with no native renderer registered (the mock
    // toolkit realizes unknown kinds as plain widgets and ignores unknown patches, exactly like a
    // backend built without this piece's feature).
    #[test]
    fn build_and_push_do_not_panic() {
        let src: Signal<Option<Arc<Vec<u8>>>> = Signal::new(None);

        day_core::uninstall_tree();
        let (mock, probe) = MockToolkit::new();
        let options = WindowOptions {
            title: "test".into(),
            size: Size::new(400.0, 300.0),
            ..Default::default()
        };
        day_core::launch_with(mock, options, move || {
            day_core::AnyPiece::new(
                remote_image(src)
                    .circle()
                    .content_mode(ContentMode::Fit)
                    .placeholder_color(Color::hex(0x808080)),
            )
        });

        let found = probe.find_by_kind(KIND);
        assert_eq!(found.len(), 1, "one remote-image leaf realized");

        // Push some (bogus) bytes, then clear; each becomes a SetBytes patch the mock ignores.
        src.set(Some(Arc::new(vec![0x89, 0x50, 0x4e, 0x47])));
        flush_sync();
        src.set(None);
        flush_sync();
    }

    #[test]
    fn builder_defaults_and_overrides() {
        let src: Signal<Option<Arc<Vec<u8>>>> = Signal::new(None);
        let base = remote_image(src);
        assert_eq!(base.clip, Clip::None);
        assert_eq!(base.mode, ContentMode::Fill);

        let shaped = remote_image(src).rounded(8.0);
        assert_eq!(shaped.clip, Clip::Rounded(8.0));
    }
}
