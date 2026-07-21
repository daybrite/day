//! day-piece-remote-image — an EXTERNAL Day Piece (DESIGN.md §15): a NATIVE image view that decodes
//! encoded bytes (PNG/JPEG) supplied **reactively** and draws them, with a placeholder rectangle
//! while the source is empty. This is the image primitive a Matrix client needs — avatars and inline
//! image messages — where the app fetches `mxc://` bytes off the UI thread via the SDK and pushes
//! them into a signal; this piece only turns bytes into a native image.
//!
//! Unlike day's built-in `image` (which loads a bundled asset by name), the source here is
//! `Signal<Option<Arc<Vec<u8>>>>`: `None` shows the placeholder color; `Some(bytes)` decodes and
//! displays. The `Arc<Vec<u8>>` is shared, so pushing a large buffer from a background decode is a
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
use day_reactive::{Signal, watch};
use day_spec::Color;

pub const KIND: &str = "day.piece.remote_image";

/// How the decoded image is scaled into the piece's frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContentMode {
    /// Scale to fill the frame, preserving aspect ratio and cropping the overflow (the default —
    /// what avatars want). SwiftUI's `.scaledToFill`.
    #[default]
    Fill,
    /// Scale to fit entirely inside the frame, preserving aspect ratio; the placeholder color shows
    /// in the letterboxed margins. SwiftUI's `.scaledToFit`.
    Fit,
}

/// How the piece (image AND placeholder) is clipped to its frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Clip {
    /// No clipping — a plain rectangle (the default).
    #[default]
    None,
    /// A centered circle of diameter `min(width, height)` — for avatars.
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

// Debug that elides the (potentially megabyte) byte buffer — logs the length, not the contents.
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

/// `remote_image_url(url)` — [`remote_image`] with the fetch built in: the bytes are downloaded
/// once through the PLATFORM HTTP stack (`day-part-http` — system proxies/VPN/Low-Data aware,
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

/// `remote_image(source)` — a native image view that decodes and displays the bytes held in
/// `source`, showing the placeholder color whenever `source` is `None`.
pub fn remote_image(source: Signal<Option<Arc<Vec<u8>>>>) -> RemoteImage {
    RemoteImage {
        source,
        clip: Clip::None,
        mode: ContentMode::Fill,
        placeholder: DEFAULT_PLACEHOLDER,
    }
}

impl RemoteImage {
    /// Clip to a centered circle of diameter `min(width, height)` — for avatars.
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
        // Seed the native view with whatever the signal already holds; subsequent changes arrive as
        // SetBytes patches (watch never fires for this initial value — §4.2's no-duplicate-op rule).
        let initial = source.get_untracked();
        let node = cx.leaf(
            KIND,
            &RemoteImageProps {
                bytes: initial,
                clip,
                mode,
                placeholder,
            },
            // No intrinsic size — fills whatever frame its container offers (the app constrains it
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
// Per-toolkit native renderers — one file per backend. Every module registers a `Renderer`
// link-time into its backend's `RENDERERS` slice; the `#[cfg]` gates each to its feature + target,
// and `#[path]` keeps the files grouped next to lib.rs (the day-piece-searchfield layout).
// ---------------------------------------------------------------------------

day_pieces::glue_modules!(appkit, gtk, qt, uikit, widget, winui);

#[cfg(test)]
mod tests {
    use super::*;
    use day_mock::MockToolkit;
    use day_reactive::flush_sync;
    use day_spec::{Size, WindowOptions};

    // Building + pushing bytes must never panic — even with no native renderer registered (the mock
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

        // Push some (bogus) bytes, then clear — each becomes a SetBytes patch the mock ignores.
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
