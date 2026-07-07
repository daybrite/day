// iOS: UIPasteboard.generalPasteboard — string/setString:/hasStrings. Unlike UIDevice this class
// is not MainThreadOnly, so the calls work from any thread. On iOS 14+ reading the pasteboard shows
// the system "app pasted from …" banner; hasStrings does not (it's the sanctioned pre-check).

use objc2_foundation::NSString;
use objc2_ui_kit::UIPasteboard;

pub fn set_text(text: &str) -> bool {
    let pb = UIPasteboard::generalPasteboard();
    unsafe { pb.setString(Some(&NSString::from_str(text))) };
    true
}

pub fn get_text() -> Option<String> {
    let pb = UIPasteboard::generalPasteboard();
    unsafe { pb.string() }.map(|s| s.to_string())
}

pub fn has_text() -> bool {
    let pb = UIPasteboard::generalPasteboard();
    unsafe { pb.hasStrings() }
}
