// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! The app outside its windows (docs/status-item.md): menu-bar status items, progress on the
//! Dock tile, the Dock icon itself, and staying alive with no window open.
use super::*;
use objc2_app_kit::{NSStatusBar, NSStatusItem};

/// One shown item: the AppKit object, the spec it was built from, and its click target.
struct Shown {
    spec: day_spec::StatusItemSpec,
    item: Retained<NSStatusItem>,
    /// The item's menu, attached for the duration of a right click when the item also has a
    /// click action (AppKit opens an attached menu on every click).
    menu: Retained<NSMenu>,
    /// Held for the button, which keeps only a weak reference to its target.
    _target: Retained<StatusTarget>,
}

thread_local! {
    static SHOWN: RefCell<Vec<Shown>> = const { RefCell::new(Vec::new()) };
    /// Whether day-core asked the app to outlive its last window.
    static KEEP_RUNNING: Cell<bool> = const { Cell::new(false) };
    static PROGRESS: RefCell<Option<Retained<objc2_app_kit::NSProgressIndicator>>> =
        const { RefCell::new(None) };
}

/// Whether closing the first window should leave the process up (the window delegate asks).
pub(super) fn keeps_running() -> bool {
    KEEP_RUNNING.with(|k| k.get())
}

pub(super) fn set_keep_running(keep: bool) {
    KEEP_RUNNING.with(|k| k.set(keep));
}

struct TargetIvars {
    id: RefCell<String>,
}

define_class!(
    /// The status button's target: a left click runs the item's action, a right (or
    /// control-) click opens its menu.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayStatusTarget"]
    #[ivars = TargetIvars]
    struct StatusTarget;

    unsafe impl NSObjectProtocol for StatusTarget {}

    impl StatusTarget {
        #[unsafe(method(clicked:))]
        fn clicked(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            ffi_guard::contain((), || {
                let id = self.ivars().id.borrow().clone();
                let Some(mtm) = MainThreadMarker::new() else {
                    return;
                };
                let app = NSApplication::sharedApplication(mtm);
                let right = app.currentEvent().is_some_and(|e| {
                    (unsafe { e.r#type() }) == objc2_app_kit::NSEventType::RightMouseUp
                        || unsafe { e.modifierFlags() }
                            .contains(objc2_app_kit::NSEventModifierFlags::Control)
                });
                let found = SHOWN.with(|s| {
                    s.borrow()
                        .iter()
                        .find(|x| x.spec.id == id)
                        .map(|x| (x.spec.activate, x.item.clone(), x.menu.clone()))
                });
                let Some((activate, item, menu)) = found else {
                    return;
                };
                if right && activate != 0 {
                    // Attach, open, detach: `performClick:` runs the menu's tracking loop
                    // synchronously, and leaving the menu attached would turn the next left
                    // click into another menu.
                    unsafe {
                        item.setMenu(Some(&menu));
                        if let Some(button) = item.button(mtm) {
                            button.performClick(None);
                        }
                        item.setMenu(None);
                    }
                } else if activate != 0 {
                    day_core::dispatch_menu_action(activate);
                }
            })
        }
    }
);

fn new_target(mtm: MainThreadMarker, id: &str) -> Retained<StatusTarget> {
    let this = StatusTarget::alloc(mtm).set_ivars(TargetIvars {
        id: RefCell::new(id.to_owned()),
    });
    unsafe { msg_send![super(this), init] }
}

/// Own the native pixels in a representation, so no borrowed app buffer reaches AppKit.
fn raster_image(raster: &day_spec::StatusImage) -> Option<Retained<objc2_app_kit::NSImage>> {
    use objc2::AllocAnyThread as _;
    use objc2_app_kit::{NSBitmapFormat, NSBitmapImageRep, NSImage};
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(), std::ptr::null_mut(), raster.width() as isize,
            raster.height() as isize, 8, 4, true, false,
            objc2_app_kit::NSCalibratedRGBColorSpace, NSBitmapFormat::AlphaNonpremultiplied,
            raster.width() as isize * 4, 32,
        )
    }?;
    let data = rep.bitmapData();
    if data.is_null() {
        return None;
    }
    // StatusImage validated exact dimensions/length; explicit bytesPerRow means no padding.
    unsafe { std::ptr::copy_nonoverlapping(raster.pixels().as_ptr(), data, raster.pixels().len()) };
    let rep = rep
        .bitmapImageRepByRetaggingWithColorSpace(&objc2_app_kit::NSColorSpace::sRGBColorSpace())?;
    let size = raster.size();
    let fit = (18.0 / size.height).min(1.0);
    let size = NSSize::new(size.width * fit, size.height * fit);
    unsafe { rep.setSize(size) };
    let image = NSImage::initWithSize(NSImage::alloc(), size);
    unsafe { image.addRepresentation(&rep) };
    Some(image)
}

/// Apply `spec` to an existing item.
fn configure(mtm: MainThreadMarker, shown: &mut Shown, spec: &day_spec::StatusItemSpec) {
    let Some(button) = (unsafe { shown.item.button(mtm) }) else {
        return;
    };
    // Menu-only or tooltip updates do not reallocate the pixels. The image belongs to the
    // button, and `shown.spec` retains the app's owned source until replacement/removal.
    if shown.spec.raster != spec.raster
        || shown.spec.icon != spec.icon
        || shown.spec.template != spec.template
        || unsafe { button.image() }.is_none()
    {
        let image = spec.raster.as_ref().and_then(raster_image).or_else(|| {
            let image = spec
                .icon
                .as_ref()
                .and_then(|icon| crate::toolbar::image_for(icon, &spec.tooltip, mtm));
            if let Some(image) = &image {
                unsafe { image.setSize(NSSize::new(18.0, 18.0)) };
            }
            image
        });
        if let Some(image) = &image {
            unsafe { image.setTemplate(spec.template) };
        }
        unsafe { button.setImage(image.as_deref()) };
    }
    unsafe {
        button.setImagePosition(objc2_app_kit::NSCellImagePosition::ImageLeft);
    }
    button.setTitle(&NSString::from_str(&spec.title));
    let tip = (!spec.tooltip.is_empty()).then(|| NSString::from_str(&spec.tooltip));
    button.setToolTip(tip.as_deref());
    button.setAccessibilityLabel(tip.as_deref());
    shown.menu = build_ns_menu(mtm, "", &spec.menu);
    // Without a click action the item IS its menu: attach it for good, so a click, the keyboard
    // and VoiceOver's "show menu" all open it natively. With one, the menu is attached only for
    // the duration of a right click (`clicked:`), or every left click would open it too.
    unsafe {
        shown
            .item
            .setMenu((spec.activate == 0).then_some(&*shown.menu));
    }
    shown.spec = spec.clone();
}

/// Show `items`, replacing the previous set (the `set_status_items` duty).
pub(super) fn set_items(mtm: MainThreadMarker, items: &[day_spec::StatusItemSpec]) {
    SHOWN.with(|s| {
        let mut shown = s.borrow_mut();
        // Gone from the list: take it out of the menu bar.
        let bar = unsafe { NSStatusBar::systemStatusBar() };
        shown.retain(|x| {
            let keep = items.iter().any(|i| i.id == x.spec.id);
            if !keep {
                unsafe { bar.removeStatusItem(&x.item) };
            }
            keep
        });
        for spec in items {
            if let Some(x) = shown.iter_mut().find(|x| x.spec.id == spec.id) {
                if x.spec != *spec {
                    configure(mtm, x, spec);
                }
                continue;
            }
            let item =
                unsafe { bar.statusItemWithLength(objc2_app_kit::NSVariableStatusItemLength) };
            // macOS remembers where the user dragged the item, keyed by this name.
            unsafe { item.setAutosaveName(Some(&NSString::from_str(&spec.id))) };
            let target = new_target(mtm, &spec.id);
            if let Some(button) = unsafe { item.button(mtm) } {
                unsafe {
                    button.setTarget(Some(&target));
                    button.setAction(Some(sel!(clicked:)));
                    // Hear both buttons: the right one opens the menu.
                    button.sendActionOn(
                        objc2_app_kit::NSEventMask::LeftMouseUp
                            | objc2_app_kit::NSEventMask::RightMouseUp,
                    );
                }
            }
            let mut x = Shown {
                spec: spec.clone(),
                item,
                menu: NSMenu::new(mtm),
                _target: target,
            };
            configure(mtm, &mut x, spec);
            shown.push(x);
        }
    });
}

/// Draw `progress` on the Dock tile (the `set_app_progress` duty): the app's icon with a bar
/// along its bottom, as the Finder does for a copy. The tile is a picture, so an indeterminate
/// bar does not animate there.
pub(super) fn set_progress(mtm: MainThreadMarker, progress: day_spec::AppProgress) {
    use day_spec::AppProgress as P;
    let app = NSApplication::sharedApplication(mtm);
    let tile = app.dockTile();
    let value = match progress {
        P::None => {
            unsafe { tile.setContentView(None) };
            PROGRESS.with(|p| *p.borrow_mut() = None);
            tile.display();
            return;
        }
        P::Indeterminate => None,
        P::Value(v) | P::Paused(v) | P::Error(v) => Some(v.clamp(0.0, 1.0)),
    };
    let bar = PROGRESS.with(|p| p.borrow().clone()).unwrap_or_else(|| {
        let size = tile.size();
        let icon_view = objc2_app_kit::NSImageView::new(mtm);
        icon_view.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), size));
        unsafe { icon_view.setImage(app.applicationIconImage().as_deref()) };
        let bar = objc2_app_kit::NSProgressIndicator::new(mtm);
        unsafe {
            bar.setStyle(objc2_app_kit::NSProgressIndicatorStyle::Bar);
            bar.setMinValue(0.0);
            bar.setMaxValue(1.0);
        }
        bar.setFrame(NSRect::new(
            NSPoint::new(size.width * 0.1, size.height * 0.06),
            NSSize::new(size.width * 0.8, size.height * 0.14),
        ));
        icon_view.addSubview(&bar);
        unsafe { tile.setContentView(Some(&icon_view)) };
        PROGRESS.with(|p| *p.borrow_mut() = Some(bar.clone()));
        bar
    });
    unsafe {
        bar.setIndeterminate(value.is_none());
        bar.setDoubleValue(value.unwrap_or(0.0));
    }
    tile.display();
}

/// Show or hide the Dock icon (the `set_dock_visible` duty): the regular activation policy, or
/// the accessory one a menu-bar app runs under.
pub(super) fn set_dock_visible(mtm: MainThreadMarker, visible: bool) {
    let app = NSApplication::sharedApplication(mtm);
    let policy = if visible {
        objc2_app_kit::NSApplicationActivationPolicy::Regular
    } else {
        objc2_app_kit::NSApplicationActivationPolicy::Accessory
    };
    app.setActivationPolicy(policy);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_raster_keeps_wide_retina_size_and_straight_alpha_pixels() {
        objc2::rc::autoreleasepool(|_| {
            let mut pixels = vec![0; 120 * 36 * 4];
            pixels[..4].copy_from_slice(&[255, 0, 0, 128]);
            let source = day_spec::StatusImage::rgba(120, 36, 2.0, pixels.clone()).unwrap();
            let image = raster_image(&source).unwrap();
            assert_eq!(unsafe { image.size() }, NSSize::new(60.0, 18.0));
            let reps = unsafe { image.representations() };
            let rep = reps
                .objectAtIndex(0)
                .downcast::<objc2_app_kit::NSBitmapImageRep>()
                .unwrap();
            assert_eq!(unsafe { rep.pixelsWide() }, 120);
            assert_eq!(unsafe { rep.pixelsHigh() }, 36);
            assert_eq!(
                unsafe { std::slice::from_raw_parts(rep.bitmapData(), pixels.len()) },
                pixels
            );
            let taller = day_spec::StatusImage::rgba(120, 72, 2.0, vec![0; 120 * 72 * 4]).unwrap();
            assert_eq!(
                unsafe { raster_image(&taller).unwrap().size() },
                NSSize::new(30.0, 18.0)
            );
        });
    }
}
