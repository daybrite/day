// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! GTK snapshots omit Cocoa child views, including embedded WKWebView. Capture the
//! composited window and crop to the requested GTK widget without pumping the UI loop.
use gtk4::{glib::translate::ToGlibPtr, prelude::*};
use objc2::AllocAnyThread;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSColorSpace, NSWindow};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGImage;
use objc2_foundation::NSDictionary;
use std::{ffi::c_void, ptr::NonNull};

unsafe extern "C" {
    fn gdk_macos_surface_get_native_window(surface: *mut c_void) -> *mut NSWindow;
    // Obsoleted by the SDK but still present; a missing image uses the GTK fallback.
    fn CGWindowListCreateImage(
        bounds: CGRect,
        options: u32,
        window: u32,
        image_options: u32,
    ) -> *mut CGImage;
}

/// Whether `widget`'s window carries a Cocoa view of its own above GTK's drawing: a piece
/// that hosts an `NSView` in the window's content view (day-piece-webview's `WKWebView`).
/// GDK's content view holds nothing else, so any subview is such a surface. It paints over
/// everything GTK draws in the window, an `AdwDialog` included, which is why the callers
/// present a dialog as a window while one is there.
pub(super) fn has_native_surface(widget: &gtk4::Widget) -> bool {
    let Some(surface) = widget.native().and_then(|n| n.surface()) else {
        return false;
    };
    let ptr: *mut gtk4::gdk::ffi::GdkSurface = surface.to_glib_none().0;
    let Some(window) = (unsafe { gdk_macos_surface_get_native_window(ptr.cast()).as_ref() }) else {
        return false;
    };
    let Some(content) = window.contentView() else {
        return false;
    };
    !content.subviews().is_empty()
}

pub(super) fn snapshot(widget: &gtk4::Widget) -> Result<Vec<u8>, String> {
    let root = widget.root().ok_or("no GTK root")?;
    let bounds = widget
        .compute_bounds(root.upcast_ref::<gtk4::Widget>())
        .ok_or("no GTK bounds")?;
    let surface = widget
        .native()
        .and_then(|n| n.surface())
        .ok_or("no GDK surface")?;
    let ptr: *mut gtk4::gdk::ffi::GdkSurface = surface.to_glib_none().0;
    // GDK owns the NSWindow for the live surface, held above for the duration of capture.
    let window = unsafe { gdk_macos_surface_get_native_window(ptr.cast()).as_ref() }
        .ok_or("no Cocoa window")?;
    let content = window.contentView().ok_or("no Cocoa content view")?;
    let number = window.windowNumber();
    if number <= 0 {
        return Err("window is not on screen".into());
    }
    window.displayIfNeeded();
    let image = unsafe {
        CGWindowListCreateImage(
            CGRect::new(
                CGPoint::new(f64::INFINITY, f64::INFINITY),
                CGSize::new(0.0, 0.0),
            ),
            1 << 3,
            number as u32,
            1,
        )
    };
    let image = NonNull::new(image).ok_or("window server has no image")?;
    // Create rule: transfer the single owned reference into CFRetained.
    let image = unsafe { CFRetained::from_raw(image) };
    let frame = window.frame();
    if frame.size.width <= 0.0 {
        return Err("zero-size window".into());
    }
    let scale = CGImage::width(Some(&image)) as f64 / frame.size.width;
    let content_rect = content.convertRect_toView(content.bounds(), None);
    let top = frame.size.height - content_rect.origin.y - content_rect.size.height;
    let crop = CGRect::new(
        CGPoint::new(
            ((content_rect.origin.x + f64::from(bounds.x())) * scale).round(),
            ((top + f64::from(bounds.y())) * scale).round(),
        ),
        CGSize::new(
            (f64::from(bounds.width()) * scale).round(),
            (f64::from(bounds.height()) * scale).round(),
        ),
    );
    let image = CGImage::with_image_in_rect(Some(&image), crop).ok_or("GTK crop failed")?;
    let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &image);
    let srgb = rep.bitmapImageRepByConvertingToColorSpace_renderingIntent(
        &NSColorSpace::sRGBColorSpace(),
        objc2_app_kit::NSColorRenderingIntent::Default,
    );
    let rep = srgb.as_deref().unwrap_or(&rep);
    let png = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .ok_or("PNG encode failed")?;
    Ok(png.to_vec())
}
