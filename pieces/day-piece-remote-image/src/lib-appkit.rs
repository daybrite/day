// ---------------------------------------------------------------------------
// AppKit: a custom NSView subclass that draws the decoded NSImage (aspect fit/fill) inside a
// centered-circle / rounded / rectangular clip, on top of the placeholder color. Drawing it in
// `drawRect:` (rather than an NSImageView + CALayer) keeps the clip resize-correct for free — the
// clip path is recomputed against the current bounds every draw — and gives true aspect-fill, which
// NSImageView's `imageScaling` cannot. `setFrameSize:` invalidates so a resize redraws; a SetBytes
// patch swaps the ivar image and marks the view for display.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::RefCell;

use day_appkit::AppKit;
use day_spec::NodeId;
use objc2::rc::Retained;
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSBezierPath, NSColor, NSCompositingOperation, NSImage, NSView};
use objc2_foundation::{NSData, NSPoint, NSRect, NSSize};

struct ImageIvars {
    image: RefCell<Option<Retained<NSImage>>>,
    clip: Clip,
    mode: ContentMode,
    color: Color,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayRemoteImageView"]
    #[ivars = ImageIvars]
    struct RemoteImageView;

    impl RemoteImageView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let iv = self.ivars();
            let bounds = self.bounds();
            let path = clip_path(bounds, iv.clip);
            path.addClip();
            // Placeholder fill first — shows through the letterboxed margins in Fit mode and behind
            // any translucent image.
            let c = iv.color;
            let color = NSColor::colorWithSRGBRed_green_blue_alpha(c.r, c.g, c.b, c.a);
            color.setFill();
            path.fill();
            if let Some(img) = iv.image.borrow().as_ref() {
                let dest = dest_rect(bounds, img.size(), iv.mode);
                img.drawInRect_fromRect_operation_fraction(
                    dest,
                    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
                    NSCompositingOperation::SourceOver,
                    1.0,
                );
            }
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            // The clip (and aspect-fill crop) depends on the size, so a resize must redraw.
            self.setNeedsDisplay(true);
        }
    }
);

impl RemoteImageView {
    fn new(mtm: MainThreadMarker, p: &RemoteImageProps) -> Retained<Self> {
        let image = p.bytes.as_ref().and_then(|b| decode(b));
        let this = Self::alloc(mtm).set_ivars(ImageIvars {
            image: RefCell::new(image),
            clip: p.clip,
            mode: p.mode,
            color: p.placeholder,
        });
        unsafe { msg_send![super(this), init] }
    }
}

/// Decode encoded (PNG/JPEG/…) bytes into an `NSImage` (returns `None` on undecodable data).
fn decode(bytes: &[u8]) -> Option<Retained<NSImage>> {
    let data = NSData::with_bytes(bytes);
    NSImage::initWithData(NSImage::alloc(), &data)
}

/// The clip path for `bounds` under `clip`: a centered circle, a rounded rect, or the full rect.
fn clip_path(bounds: NSRect, clip: Clip) -> Retained<NSBezierPath> {
    match clip {
        Clip::None => NSBezierPath::bezierPathWithRect(bounds),
        Clip::Circle => {
            let d = bounds.size.width.min(bounds.size.height);
            let sq = NSRect::new(
                NSPoint::new(
                    bounds.origin.x + (bounds.size.width - d) / 2.0,
                    bounds.origin.y + (bounds.size.height - d) / 2.0,
                ),
                NSSize::new(d, d),
            );
            NSBezierPath::bezierPathWithOvalInRect(sq)
        }
        Clip::Rounded(r) => NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, r, r),
    }
}

/// Aspect-scaled, centered destination rect for an image of `img` size drawn into `bounds`.
fn dest_rect(bounds: NSRect, img: NSSize, mode: ContentMode) -> NSRect {
    let (bw, bh) = (bounds.size.width, bounds.size.height);
    let (iw, ih) = (img.width.max(1.0), img.height.max(1.0));
    let scale = match mode {
        ContentMode::Fill => (bw / iw).max(bh / ih),
        ContentMode::Fit => (bw / iw).min(bh / ih),
    };
    let (w, h) = (iw * scale, ih * scale);
    NSRect::new(
        NSPoint::new(
            bounds.origin.x + (bw - w) / 2.0,
            bounds.origin.y + (bh - h) / 2.0,
        ),
        NSSize::new(w, h),
    )
}

fn make(backend: &mut AppKit, p: &RemoteImageProps, _id: NodeId) -> Retained<NSView> {
    let view = RemoteImageView::new(backend.mtm(), p);
    Retained::from(<RemoteImageView as AsRef<NSView>>::as_ref(&view))
}

fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &RemoteImagePatch) {
    let RemoteImagePatch::SetBytes(bytes) = patch;
    if let Some(view) = h.downcast_ref::<RemoteImageView>() {
        let img = bytes.as_ref().and_then(|b| decode(b));
        *view.ivars().image.borrow_mut() = img;
        view.setNeedsDisplay(true);
    }
}

day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: RemoteImageProps, patch: RemoteImagePatch,
    make: make, update: update, measure: day_pieces::fill_measure);
