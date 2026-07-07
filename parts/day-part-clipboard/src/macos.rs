// macOS: NSPasteboard (AppKit). The general pasteboard is toolkit-independent — it needs no
// NSApplication, run loop, or window — so this works in day-qt binaries and plain `cargo test`
// processes just as well as under day-appkit. Write is the standard clearContents() +
// setString:forType:NSPasteboardTypeString pair; read is stringForType:, which also serves rich
// clipboard contents that carry a plain-text representation.
//
// NSPasteboard is NOT thread-safe: two threads touching the general pasteboard concurrently can
// segfault inside AppKit (observed with parallel `cargo test` threads). A process-wide mutex
// serializes this crate's accesses; the calls themselves work fine off the main thread.

use std::sync::Mutex;

use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;

static PASTEBOARD_LOCK: Mutex<()> = Mutex::new(());

pub fn set_text(text: &str) -> bool {
    let _guard = PASTEBOARD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let pb = NSPasteboard::generalPasteboard();
    // clearContents() takes ownership of the pasteboard (bumps the change count); without it,
    // setString:forType: fails on a pasteboard another app owns.
    pb.clearContents();
    pb.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString })
}

pub fn get_text() -> Option<String> {
    let _guard = PASTEBOARD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let pb = NSPasteboard::generalPasteboard();
    pb.stringForType(unsafe { NSPasteboardTypeString })
        .map(|s| s.to_string())
}

pub fn has_text() -> bool {
    // stringForType: is nil-or-string; presence is the check (AppKit has no cheaper boolean).
    let _guard = PASTEBOARD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let pb = NSPasteboard::generalPasteboard();
    pb.stringForType(unsafe { NSPasteboardTypeString })
        .is_some()
}
