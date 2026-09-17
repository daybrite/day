// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// iOS: UIPasteboard.generalPasteboard, via string/setString:/hasStrings. Unlike UIDevice this class
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

use crate::{Content, Error, MAX_BYTES, Representation};
use objc2_foundation::NSData;
fn native_type(mime: &str) -> objc2::rc::Retained<NSString> {
    NSString::from_str(match mime {
        "text/plain" => "public.utf8-plain-text",
        "image/png" => "public.png",
        "image/jpeg" => "public.jpeg",
        "image/tiff" => "public.tiff",
        "image/gif" => "com.compuserve.gif",
        "image/bmp" => "com.microsoft.bmp",
        "image/svg+xml" => "public.svg-image",
        "image/webp" => "org.webmproject.webp",
        other => other,
    })
}

pub fn write_content(content: &Content) -> Result<Vec<String>, Error> {
    use objc2_foundation::{NSArray, NSDictionary};
    let keys: Vec<_> = content.0.iter().map(|r| native_type(&r.mime)).collect();
    let values: Vec<_> = content
        .0
        .iter()
        .map(|r| NSData::with_bytes(&r.bytes))
        .collect();
    let key_refs: Vec<_> = keys.iter().map(|k| &**k).collect();
    let value_refs: Vec<_> = values
        .iter()
        .map(|v| &**v as &objc2::runtime::AnyObject)
        .collect();
    let item = NSDictionary::from_slices(&key_refs, &value_refs);
    let items = NSArray::from_slice(&[&*item]);
    unsafe {
        UIPasteboard::generalPasteboard().setItems(&items);
    }
    Ok(content.0.iter().map(|r| r.mime.clone()).collect())
}
pub fn read_content(preferred: &[&str]) -> Result<Option<Representation>, Error> {
    let pb = UIPasteboard::generalPasteboard();
    for mime in preferred {
        if let Some(data) = pb.dataForPasteboardType(&native_type(mime)) {
            if data.len() > MAX_BYTES {
                return Err(Error::TooLarge);
            }
            return Ok(Some(Representation::new(*mime, data.to_vec())));
        }
    }
    Ok(None)
}
