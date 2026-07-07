// ---------------------------------------------------------------------------
// UIKit: a UIImageView subclass. UIImageView has real aspect-fill/-fit via `contentMode`, and
// `clipsToBounds` + `layer.cornerRadius` gives the rounded/circle clip; `backgroundColor` is the
// placeholder shown while there's no image. The circle radius depends on the size, so a
// `layoutSubviews` override recomputes `layer.cornerRadius` on every resize. A SetBytes patch swaps
// the image (nil → clears to the placeholder). The layer is touched via `msg_send!` so this piece
// needs no objc2-quartz-core dependency.
// ---------------------------------------------------------------------------

use super::*;

use day_spec::NodeId;
use day_uikit::Uikit;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::{CGFloat, CGRect};
use objc2_foundation::NSData;
use objc2_ui_kit::{UIColor, UIImage, UIImageView, UIView, UIViewContentMode};

struct ImageIvars {
    clip: Clip,
}

define_class!(
    #[unsafe(super(UIImageView, UIView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayRemoteImageView"]
    #[ivars = ImageIvars]
    struct RemoteImageView;

    impl RemoteImageView {
        #[unsafe(method(layoutSubviews))]
        fn layout_subviews(&self) {
            let _: () = unsafe { msg_send![super(self), layoutSubviews] };
            let b: CGRect = self.bounds();
            let radius: CGFloat = match self.ivars().clip {
                Clip::None => 0.0,
                Clip::Circle => b.size.width.min(b.size.height) / 2.0,
                Clip::Rounded(r) => r,
            };
            let layer: Retained<AnyObject> = unsafe { msg_send![self, layer] };
            let _: () = unsafe { msg_send![&layer, setCornerRadius: radius] };
        }
    }
);

impl RemoteImageView {
    fn new(mtm: MainThreadMarker, p: &RemoteImageProps) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ImageIvars { clip: p.clip });
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        this.setContentMode(match p.mode {
            ContentMode::Fill => UIViewContentMode::ScaleAspectFill,
            ContentMode::Fit => UIViewContentMode::ScaleAspectFit,
        });
        this.setClipsToBounds(true);
        let c = p.placeholder;
        let color = UIColor::colorWithRed_green_blue_alpha(c.r, c.g, c.b, c.a);
        this.setBackgroundColor(Some(&color));
        if let Some(bytes) = &p.bytes
            && let Some(img) = decode(bytes)
        {
            this.setImage(Some(&img));
        }
        this
    }
}

/// Decode encoded (PNG/JPEG/…) bytes into a `UIImage` (returns `None` on undecodable data).
fn decode(bytes: &[u8]) -> Option<Retained<UIImage>> {
    let data = NSData::with_bytes(bytes);
    UIImage::imageWithData(&data)
}

fn make(_backend: &mut Uikit, p: &RemoteImageProps, _id: NodeId) -> Retained<UIView> {
    let mtm = MainThreadMarker::new().unwrap();
    let view = RemoteImageView::new(mtm, p);
    Retained::from(<RemoteImageView as AsRef<UIView>>::as_ref(&view))
}

fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &RemoteImagePatch) {
    let RemoteImagePatch::SetBytes(bytes) = patch;
    if let Some(view) = (**h).downcast_ref::<RemoteImageView>() {
        let img = bytes.as_ref().and_then(|b| decode(b));
        view.setImage(img.as_deref());
    }
}

day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
    kind: KIND, props: RemoteImageProps, patch: RemoteImagePatch,
    make: make, update: update, measure: day_pieces::fill_measure);
