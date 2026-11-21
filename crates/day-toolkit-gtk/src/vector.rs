// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! SVG geometry retained in a `GdkPaintable`, rendered at each requested snapshot size.
//! The small C binding uses the installed librsvg and the same GLib/Cairo ABI as gtk-rs.

use std::cell::{Cell, RefCell};
use std::ffi::CString;
use std::path::Path;

use gtk4::gdk::subclass::prelude::*;
use gtk4::glib::translate::*;
use gtk4::prelude::*;
use gtk4::{gdk, glib, graphene};

/// Icon-theme widgets require a symbolic bitmap: `source` rendered so its longer side is
/// `size` pixels, straight (not premultiplied) RGBA. Rendered by librsvg directly rather than
/// through a gdk-pixbuf loader: the loader sniffs the content even when the type is named, and
/// librsvg 2.60's loader rejects an SVG whose `<svg` start tag sits past the sniff window
/// behind a long license comment ("Unsupported image format"), which bundled icons have.
pub(super) fn symbolic_pixbuf(source: &[u8], size: i32) -> Option<gtk4::gdk_pixbuf::Pixbuf> {
    use gtk4::cairo;
    let mut error = std::ptr::null_mut();
    // SAFETY: `source` is live for the call and librsvg copies it. A successful handle is a
    // full GObject reference.
    let document: glib::Object = unsafe {
        let handle = rsvg_handle_new_from_data(source.as_ptr(), source.len(), &raw mut error);
        if handle.is_null() {
            if !error.is_null() {
                let error: glib::Error = from_glib_full(error);
                log::warn!("SVG decode failed: {error}");
            }
            return None;
        }
        from_glib_full(handle)
    };
    let (width, height) = intrinsic_size(&document)?;
    let scale = f64::from(size) / width.max(height);
    let pixel_width = (width * scale).round().max(1.0) as i32;
    let pixel_height = (height * scale).round().max(1.0) as i32;
    let mut surface =
        cairo::ImageSurface::create(cairo::Format::ARgb32, pixel_width, pixel_height).ok()?;
    {
        let cr = cairo::Context::new(&surface).ok()?;
        let viewport = Rectangle {
            width: f64::from(pixel_width),
            height: f64::from(pixel_height),
            ..Default::default()
        };
        let mut error = std::ptr::null_mut();
        // SAFETY: the owned librsvg GObject and Cairo context outlive this synchronous call.
        let rendered = unsafe {
            rsvg_handle_render_document(
                document.as_ptr(),
                cr.to_raw_none(),
                &raw const viewport,
                &raw mut error,
            )
        };
        if rendered == 0 {
            if !error.is_null() {
                // SAFETY: librsvg hands over ownership of the error it set.
                let error: glib::Error = unsafe { from_glib_full(error) };
                log::warn!("SVG render failed: {error}");
            }
            return None;
        }
    }
    surface.flush();
    let stride = usize::try_from(surface.stride()).ok()?;
    let row_bytes = usize::try_from(pixel_width).ok()? * 4;
    let rgba = unpremultiply_argb32(&surface.data().ok()?, stride, row_bytes);
    Some(gtk4::gdk_pixbuf::Pixbuf::from_bytes(
        &glib::Bytes::from_owned(rgba),
        gtk4::gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        pixel_width,
        pixel_height,
        i32::try_from(row_bytes).ok()?,
    ))
}

/// Cairo's `ARGB32` rows (native-endian premultiplied words, `stride` bytes apart) as tightly
/// packed straight-alpha RGBA bytes, `row_bytes` per row.
fn unpremultiply_argb32(data: &[u8], stride: usize, row_bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(row_bytes * data.len().div_ceil(stride.max(1)));
    for row in data.chunks(stride.max(1)) {
        for word in row[..row_bytes.min(row.len())].as_chunks::<4>().0 {
            let argb = u32::from_ne_bytes(*word);
            let [a, r, g, b] = argb.to_be_bytes();
            let straight = |c: u8| {
                if a == 0 {
                    0
                } else {
                    ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8
                }
            };
            out.extend_from_slice(&[straight(r), straight(g), straight(b), a]);
        }
    }
    out
}

/// The document's intrinsic size in CSS pixels: its width and height when it states them in
/// absolute units, else its view box. `None` for a document with neither.
fn intrinsic_size(document: &glib::Object) -> Option<(f64, f64)> {
    let (mut width, mut height) = (0.0, 0.0);
    // SAFETY: all outputs are live; unneeded optional outputs may be NULL per librsvg's API.
    unsafe {
        if rsvg_handle_get_intrinsic_size_in_pixels(
            document.as_ptr(),
            &raw mut width,
            &raw mut height,
        ) == 0
        {
            let mut has_viewbox = 0;
            let mut viewbox = Rectangle::default();
            rsvg_handle_get_intrinsic_dimensions(
                document.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &raw mut has_viewbox,
                &raw mut viewbox,
            );
            if has_viewbox == 0 {
                return None;
            }
            width = viewbox.width;
            height = viewbox.height;
        }
    }
    (width > 0.0 && height > 0.0).then_some((width, height))
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
    fn rsvg_handle_new_from_data(
        data: *const u8,
        data_len: usize,
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
                    &raw const viewport,
                    &raw mut error,
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
            let handle = rsvg_handle_new_from_file(filename.as_ptr(), &raw mut error);
            if handle.is_null() {
                if !error.is_null() {
                    let error: glib::Error = from_glib_full(error);
                    log::warn!("SVG load failed for {}: {error}", path.display());
                }
                return None;
            }
            from_glib_full(handle)
        };
        let (width, height) = intrinsic_size(&document)?;
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
    fn unpremultiply_drops_row_padding_and_restores_straight_color() {
        // Two rows of one pixel each, stride 8: half-transparent red, then fully clear.
        let half_red = 0x8080_0000_u32.to_ne_bytes();
        let clear = 0_u32.to_ne_bytes();
        let mut data = Vec::new();
        data.extend_from_slice(&half_red);
        data.extend_from_slice(&[9; 4]);
        data.extend_from_slice(&clear);
        data.extend_from_slice(&[9; 4]);
        assert_eq!(
            super::unpremultiply_argb32(&data, 8, 4),
            vec![255, 0, 0, 128, 0, 0, 0, 0]
        );
    }

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
