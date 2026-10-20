// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! SVG geometry retained in a GdkPaintable, rendered at each requested snapshot size.
//! The small C binding uses the installed librsvg and the same GLib/Cairo ABI as gtk-rs.

use std::cell::{Cell, RefCell};
use std::ffi::CString;
use std::path::Path;

use gtk4::gdk::subclass::prelude::*;
use gtk4::glib::translate::*;
use gtk4::prelude::*;
use gtk4::{gdk, glib, graphene};

/// Icon-theme widgets require a symbolic bitmap. Select the SVG decoder explicitly:
/// gdk-pixbuf's format sniffing misses SVGs with long leading license comments.
pub(super) fn symbolic_pixbuf(source: &[u8], size: i32) -> Option<gtk4::gdk_pixbuf::Pixbuf> {
    let loader = gtk4::gdk_pixbuf::PixbufLoader::with_type("svg").ok()?;
    loader.connect_size_prepared(move |loader, width, height| {
        if width > 0 && height > 0 {
            let scale = f64::from(size) / f64::from(width.max(height));
            loader.set_size(
                (f64::from(width) * scale).round().max(1.0) as i32,
                (f64::from(height) * scale).round().max(1.0) as i32,
            );
        }
    });
    let written = loader.write(source);
    let closed = loader.close();
    written.ok()?;
    closed.ok()?;
    loader.pixbuf()
}

#[repr(C)]
#[derive(Default)]
struct Rectangle {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

unsafe extern "C" {
    fn rsvg_handle_new_from_file(
        filename: *const std::ffi::c_char,
        error: *mut *mut glib::ffi::GError,
    ) -> *mut glib::gobject_ffi::GObject;
    fn rsvg_handle_get_intrinsic_size_in_pixels(
        handle: *mut glib::gobject_ffi::GObject,
        width: *mut f64,
        height: *mut f64,
    ) -> glib::ffi::gboolean;
    fn rsvg_handle_get_intrinsic_dimensions(
        handle: *mut glib::gobject_ffi::GObject,
        has_width: *mut glib::ffi::gboolean,
        width: *mut std::ffi::c_void,
        has_height: *mut glib::ffi::gboolean,
        height: *mut std::ffi::c_void,
        has_viewbox: *mut glib::ffi::gboolean,
        viewbox: *mut Rectangle,
    );
    fn rsvg_handle_render_document(
        handle: *mut glib::gobject_ffi::GObject,
        cr: *mut gtk4::cairo::ffi::cairo_t,
        viewport: *const Rectangle,
        error: *mut *mut glib::ffi::GError,
    ) -> glib::ffi::gboolean;
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Svg {
        pub document: RefCell<Option<glib::Object>>,
        pub source: RefCell<String>,
        pub size: Cell<(f64, f64)>,
        pub tint: Cell<Option<day_spec::Color>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Svg {
        const NAME: &'static str = "DaySvgPaintable";
        type Type = super::SvgPaintable;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for Svg {}

    impl PaintableImpl for Svg {
        fn flags(&self) -> gdk::PaintableFlags {
            gdk::PaintableFlags::STATIC_SIZE
        }
        fn intrinsic_width(&self) -> i32 {
            self.size.get().0.ceil() as i32
        }
        fn intrinsic_height(&self) -> i32 {
            self.size.get().1.ceil() as i32
        }
        fn intrinsic_aspect_ratio(&self) -> f64 {
            let (w, h) = self.size.get();
            if h > 0.0 { w / h } else { 1.0 }
        }
        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            let Some(snapshot) = snapshot.downcast_ref::<gtk4::Snapshot>() else {
                return;
            };
            let doc = self.document.borrow();
            let Some(doc) = doc.as_ref() else { return };
            if width <= 0.0 || height <= 0.0 {
                return;
            }
            // Tint the render node, not a fixed-resolution bitmap. The color matrix keeps
            // coverage alpha (including holes and antialiasing), replacing only the RGB.
            if let Some(t) = self.tint.get() {
                snapshot.push_color_matrix(
                    &graphene::Matrix::from_float([
                        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                        t.a as f32,
                    ]),
                    &graphene::Vec4::new(t.r as f32, t.g as f32, t.b as f32, 0.0),
                );
            }
            let cr =
                snapshot.append_cairo(&graphene::Rect::new(0.0, 0.0, width as f32, height as f32));
            let (w, h) = self.size.get();
            // GtkPicture already applies fit/fill/stretch. Render in the SVG's intrinsic
            // aspect ratio and scale that viewport to the requested paintable allocation.
            cr.scale(width / w, height / h);
            let viewport = Rectangle {
                width: w,
                height: h,
                ..Default::default()
            };
            let mut error = std::ptr::null_mut();
            // SAFETY: the owned librsvg GObject and Cairo context outlive this synchronous call.
            unsafe {
                if rsvg_handle_render_document(
                    doc.as_ptr(),
                    cr.to_raw_none(),
                    &viewport,
                    &mut error,
                ) == 0
                    && !error.is_null()
                {
                    let error: glib::Error = from_glib_full(error);
                    log::warn!("SVG render failed: {error}");
                }
            }
            drop(cr);
            if self.tint.get().is_some() {
                snapshot.pop();
            }
        }
    }
}

glib::wrapper! {
    pub struct SvgPaintable(ObjectSubclass<imp::Svg>) @implements gdk::Paintable;
}

impl SvgPaintable {
    pub fn load(
        path: &Path,
        tint: Option<day_spec::Color>,
        icon_size: Option<f64>,
    ) -> Option<Self> {
        let filename = CString::new(path.to_str()?).ok()?;
        let mut error = std::ptr::null_mut();
        // SAFETY: filename is NUL terminated. A successful handle is a full GObject reference.
        let document: glib::Object = unsafe {
            let handle = rsvg_handle_new_from_file(filename.as_ptr(), &mut error);
            if handle.is_null() {
                if !error.is_null() {
                    let error: glib::Error = from_glib_full(error);
                    log::warn!("SVG load failed for {}: {error}", path.display());
                }
                return None;
            }
            from_glib_full(handle)
        };
        let (mut width, mut height) = (0.0, 0.0);
        // SAFETY: all outputs are live; unneeded optional outputs may be NULL per librsvg's API.
        unsafe {
            if rsvg_handle_get_intrinsic_size_in_pixels(document.as_ptr(), &mut width, &mut height)
                == 0
            {
                let mut has_viewbox = 0;
                let mut viewbox = Rectangle::default();
                rsvg_handle_get_intrinsic_dimensions(
                    document.as_ptr(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut has_viewbox,
                    &mut viewbox,
                );
                if has_viewbox == 0 {
                    return None;
                }
                width = viewbox.width;
                height = viewbox.height;
            }
        }
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let obj: Self = glib::Object::new();
        let imp = obj.imp();
        // Icons occupy a logical-size box; device pixels never become GTK layout units.
        let scale = icon_size.map_or(1.0, |size| size / width.max(height));
        imp.size.set((width * scale, height * scale));
        imp.document.replace(Some(document));
        imp.source.replace(path.to_string_lossy().into_owned());
        imp.tint.set(tint);
        Some(obj)
    }

    pub fn source_is(&self, path: &Path) -> bool {
        *self.imp().source.borrow() == path.to_string_lossy()
    }

    pub fn set_tint(&self, tint: Option<day_spec::Color>) {
        self.imp().tint.set(tint);
        self.invalidate_contents();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn symbolic_svg_with_long_header_preserves_aspect_ratio_and_alpha() {
        let source = format!(
            "<!-- {} -->\n\
             <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"12\">\
             <path d=\"M0 0H12V12H0Z\"/></svg>",
            "license and author notes ".repeat(100)
        );
        let pixbuf = super::symbolic_pixbuf(source.as_bytes(), 80).expect("decode SVG");
        assert_eq!((pixbuf.width(), pixbuf.height()), (80, 40));
        assert!(pixbuf.has_alpha());
        let bytes = pixbuf.read_pixel_bytes();
        let alpha =
            |x: usize, y: usize| bytes.as_ref()[y * pixbuf.rowstride() as usize + x * 4 + 3];
        assert_eq!(alpha(10, 20), 255, "filled half survives");
        assert_eq!(alpha(60, 20), 0, "transparent half stays transparent");
    }
}
