// ---------------------------------------------------------------------------
// GTK: a small GtkWidget subclass that draws in `snapshot()` — a GdkTexture decoded from the bytes,
// aspect fit/fill, under a rounded/circle/rect clip, over the placeholder color. Doing it in
// `snapshot()` (via GskRoundedRect clips) makes the clip resize-correct for free and gives true
// aspect-fill, which a plain GtkPicture + CSS cannot. `GdkTexture::from_bytes` handles PNG/JPEG/…;
// an undecodable buffer leaves the texture empty (placeholder only). A SetBytes patch swaps the
// texture and queues a redraw.
// ---------------------------------------------------------------------------

use super::*;
use day_gtk::Gtk;
use day_spec::NodeId;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gdk, glib, graphene, gsk};

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct DayRemoteImage {
        pub texture: RefCell<Option<gdk::Texture>>,
        pub clip: Cell<Clip>,
        pub mode: Cell<ContentMode>,
        pub color: Cell<(f32, f32, f32, f32)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DayRemoteImage {
        const NAME: &'static str = "DayRemoteImage";
        type Type = super::DayRemoteImage;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for DayRemoteImage {}

    impl WidgetImpl for DayRemoteImage {
        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let obj = self.obj();
            let w = obj.width() as f32;
            let h = obj.height() as f32;
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            let full = graphene::Rect::new(0.0, 0.0, w, h);

            // Push the clip (a rounded clip for circle/rounded, a plain clip for none) — one push,
            // one pop.
            match self.clip.get() {
                Clip::None => snapshot.push_clip(&full),
                Clip::Circle => {
                    let d = w.min(h);
                    let sq = graphene::Rect::new((w - d) / 2.0, (h - d) / 2.0, d, d);
                    snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(sq, d / 2.0));
                }
                Clip::Rounded(r) => {
                    snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(full, r as f32));
                }
            }

            // Placeholder fill (shows in Fit's letterbox margins and while there's no texture).
            let (r, g, b, a) = self.color.get();
            snapshot.append_color(&gdk::RGBA::new(r, g, b, a), &full);

            if let Some(tex) = self.texture.borrow().as_ref() {
                let rect = fit_rect(
                    w,
                    h,
                    tex.width() as f32,
                    tex.height() as f32,
                    self.mode.get(),
                );
                snapshot.append_texture(tex, &rect);
            }

            snapshot.pop();
        }
    }

    /// Aspect-scaled, centered rect for a `iw`×`ih` texture drawn into a `bw`×`bh` box.
    fn fit_rect(bw: f32, bh: f32, iw: f32, ih: f32, mode: ContentMode) -> graphene::Rect {
        let iw = iw.max(1.0);
        let ih = ih.max(1.0);
        let scale = match mode {
            ContentMode::Fill => (bw / iw).max(bh / ih),
            ContentMode::Fit => (bw / iw).min(bh / ih),
        };
        let (w, h) = (iw * scale, ih * scale);
        graphene::Rect::new((bw - w) / 2.0, (bh - h) / 2.0, w, h)
    }
}

glib::wrapper! {
    pub struct DayRemoteImage(ObjectSubclass<imp::DayRemoteImage>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl DayRemoteImage {
    fn new(p: &RemoteImageProps) -> Self {
        let obj: Self = glib::Object::new();
        let imp = obj.imp();
        imp.clip.set(p.clip);
        imp.mode.set(p.mode);
        let c = p.placeholder;
        imp.color
            .set((c.r as f32, c.g as f32, c.b as f32, c.a as f32));
        obj.set_bytes(p.bytes.as_deref().map(|v| v.as_slice()));
        obj
    }

    fn set_bytes(&self, bytes: Option<&[u8]>) {
        let tex = bytes.and_then(|b| gdk::Texture::from_bytes(&glib::Bytes::from(b)).ok());
        *self.imp().texture.borrow_mut() = tex;
        self.queue_draw();
    }
}

fn make(_backend: &mut Gtk, p: &RemoteImageProps, _id: NodeId) -> gtk4::Widget {
    DayRemoteImage::new(p).upcast()
}

fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &RemoteImagePatch) {
    let RemoteImagePatch::SetBytes(bytes) = patch;
    if let Some(w) = h.downcast_ref::<DayRemoteImage>() {
        w.set_bytes(bytes.as_deref().map(|v| v.as_slice()));
    }
}

day_pieces::renderer!(day_gtk::RENDERERS, Gtk,
    kind: KIND, props: RemoteImageProps, patch: RemoteImagePatch,
    make: make, update: update, measure: day_pieces::fill_measure);
