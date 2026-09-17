// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
use super::*;
use day_spec::transfer::*;
use objc2::ClassType;
use objc2_app_kit::{NSDragOperation, NSDraggingItem, NSPasteboardItem};
use objc2_foundation::NSURL;
use std::ffi::c_void;

// Use the same system MIME/UTI conversion as GTK, including dynamic UTIs for custom types.
#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    static kUTTagClassMIMEType: *const c_void;
    fn UTTypeCreatePreferredIdentifierForTag(
        class: *const c_void,
        tag: *const c_void,
        conforms: *const c_void,
    ) -> *mut NSString;
    fn UTTypeCopyPreferredTagWithClass(uti: *const c_void, class: *const c_void) -> *mut NSString;
}
fn native(mime: &str) -> Retained<NSString> {
    if mime == "text/uri-list" {
        return NSString::from_str("public.file-url");
    }
    let s = NSString::from_str(mime);
    unsafe {
        Retained::from_raw(UTTypeCreatePreferredIdentifierForTag(
            kUTTagClassMIMEType,
            (&*s as *const NSString).cast(),
            std::ptr::null(),
        ))
    }
    .unwrap_or(s)
}
fn mime(uti: &NSString) -> Option<String> {
    if uti.to_string() == "public.file-url" {
        return Some("text/uri-list".into());
    }
    unsafe {
        Retained::from_raw(UTTypeCopyPreferredTagWithClass(
            (uti as *const NSString).cast(),
            kUTTagClassMIMEType,
        ))
    }
    .map(|s| s.to_string())
}
day_core::tls_group! {
    static SOURCES: SideTable<Source> = SideTable::new();
    static TARGETS: SideTable<Target> = SideTable::new();
    static PRESSES: SideTable<NSPoint> = SideTable::new();
}
fn key(view: &NSView) -> usize {
    view as *const NSView as usize
}
pub fn source(view: &NSView, source: Source) {
    SOURCES.with(|t| t.insert(key(view), source));
}
pub fn target(view: &NSView, target: Target) {
    let mut types: Vec<_> = target.types.iter().map(|s| native(s)).collect();
    types.push(native(BUNDLE_MIME));
    view.registerForDraggedTypes(&NSArray::from_retained_slice(&types));
    TARGETS.with(|t| t.insert(key(view), target));
}
pub fn pressed(view: &NSView, event: &NSEvent) -> bool {
    if SOURCES.with(|t| t.get(key(view))).is_none() {
        return false;
    }
    PRESSES.with(|t| {
        t.insert(
            key(view),
            view.convertPoint_fromView(event.locationInWindow(), None),
        )
    });
    true
}
pub fn dragged(view: &DayFlipped, event: &NSEvent) -> bool {
    let Some(source) = SOURCES.with(|t| t.get(key(view))) else {
        return false;
    };
    let Some(start) = PRESSES.with(|t| t.get(key(view))) else {
        return true;
    };
    let p = view.convertPoint_fromView(event.locationInWindow(), None);
    if (p.x - start.x).hypot(p.y - start.y) < 4. {
        return true;
    }
    PRESSES.with(|t| t.remove(key(view)));
    ffi_guard::contain((), || {
        let Some(offer) = source(Point::new(start.x, start.y)) else {
            return;
        };
        let Some(packet) = offer.encode() else {
            return;
        };
        let pb = NSPasteboardItem::new();
        pb.setData_forType(&NSData::with_bytes(&packet), &native(BUNDLE_MIME));
        if let Some(item) = offer.items.first() {
            for r in &item.representations {
                if r.mime == "text/uri-list" {
                    if let Ok(s) = std::str::from_utf8(&r.bytes) {
                        if let Some(uri) = s.lines().find(|s| !s.is_empty() && !s.starts_with('#'))
                        {
                            pb.setString_forType(&NSString::from_str(uri), &native(&r.mime));
                        }
                    }
                } else {
                    pb.setData_forType(&NSData::with_bytes(&r.bytes), &native(&r.mime));
                }
            }
        }
        let item = NSDraggingItem::initWithPasteboardWriter(
            NSDraggingItem::alloc(),
            ProtocolObject::from_ref(&*pb),
        );
        let bounds = view.bounds();
        let preview = unsafe {
            objc2_app_kit::NSImage::initWithSize(objc2_app_kit::NSImage::alloc(), bounds.size)
        };
        if let Some(rep) = unsafe { view.bitmapImageRepForCachingDisplayInRect(bounds) } {
            unsafe {
                view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
                preview.addRepresentation(&rep);
            }
        }
        unsafe {
            item.setDraggingFrame_contents(bounds, Some(&preview));
        }
        view.beginDraggingSessionWithItems_event_source(
            &NSArray::from_slice(&[&*item]),
            event,
            ProtocolObject::from_ref(view),
        );
    });
    true
}
fn location(view: &NSView, info: &ProtocolObject<dyn objc2_app_kit::NSDraggingInfo>) -> Location {
    let p = unsafe { view.convertPoint_fromView(info.draggingLocation(), None) };
    let pb = unsafe { info.draggingPasteboard() };
    let types = pb
        .types()
        .map(|a| a.iter().filter_map(|s| mime(&s)).collect())
        .unwrap_or_default();
    let allowed = if unsafe { info.draggingSourceOperationMask() }.contains(NSDragOperation::Copy) {
        vec![Operation::Copy]
    } else {
        vec![]
    };
    Location {
        local: unsafe { info.draggingSource() }.is_some(),
        position: Point::new(p.x, p.y),
        types,
        allowed,
    }
}
pub fn proposal(
    view: &NSView,
    info: &ProtocolObject<dyn objc2_app_kit::NSDraggingInfo>,
) -> NSDragOperation {
    if TARGETS
        .with(|t| t.get(key(view)))
        .is_some_and(|t| t.proposal(&location(view, info)) == Operation::Copy)
    {
        NSDragOperation::Copy
    } else {
        NSDragOperation::None
    }
}
pub fn receive(view: &NSView, info: &ProtocolObject<dyn objc2_app_kit::NSDraggingInfo>) -> bool {
    ffi_guard::contain(false, || {
        let Some(target) = TARGETS.with(|t| t.get(key(view))) else {
            return false;
        };
        let at = location(view, info);
        if target.proposal(&at) == Operation::None {
            return false;
        }
        let pb = unsafe { info.draggingPasteboard() };
        // Read file objects through AppKit even when a Day bundle is also present, so
        // AppKit can acquire sandbox access. Raw public.file-url bytes are only locators;
        // NSURL must resolve Finder's file-reference URLs to ordinary file-path URLs.
        let _file_urls = unsafe {
            pb.readObjectsForClasses_options(&NSArray::from_slice(&[NSURL::class()]), None)
        };
        if let Some(data) = pb.dataForType(&native(BUNDLE_MIME)) {
            return data.len() <= MAX_BYTES
                && Offer::decode(&data.to_vec()).is_some_and(|o| target.deliver(at, o));
        }
        let mut items = Vec::new();
        let mut total = 0usize;
        if let Some(native_items) = pb.pasteboardItems() {
            for item in native_items.iter() {
                for ty in &target.types {
                    let bytes = if ty == "text/uri-list" {
                        // Resolve each item's own URL so mixed or unreadable items cannot
                        // shift the association between native objects and representations.
                        item.stringForType(&native(ty))
                            .and_then(|s| NSURL::URLWithString(&s))
                            .and_then(|url| file_uri(&url))
                            .map(|url| format!("{url}\r\n").into_bytes())
                    } else {
                        item.dataForType(&native(ty)).map(|data| data.to_vec())
                    };
                    if let Some(bytes) = bytes {
                        total = total.saturating_add(bytes.len());
                        if total > MAX_BYTES || items.len() >= MAX_ITEMS {
                            return false;
                        }
                        items.push(Item::new(vec![Representation::new(ty, bytes)]));
                        break;
                    }
                }
            }
        }
        !items.is_empty() && target.deliver(at, Offer { items })
    })
}

// Preserve URL escaping while resolving Finder's /.file/id=... references through Foundation.
fn file_uri(url: &NSURL) -> Option<String> {
    if !url.isFileURL() {
        return None;
    }
    url.filePathURL()?.absoluteString().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_reference_url_resolves_to_readable_uri_list_path() {
        let path = std::env::temp_dir().join(format!("day-drop-{} ü.png", std::process::id()));
        std::fs::write(&path, b"image fixture").unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str().unwrap()));
        let reference = url.fileReferenceURL().unwrap();
        let raw = reference.absoluteString().unwrap().to_string();
        assert!(raw.contains("/.file/id="), "expected a file-ID URL: {raw}");
        let resolved = file_uri(&reference).unwrap();
        let paths = file_paths(resolved.as_bytes()).unwrap();
        assert_eq!(paths, vec![path.clone()]);
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"image fixture");
        std::fs::remove_file(path).unwrap();
    }
}
