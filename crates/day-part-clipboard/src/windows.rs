// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Windows: the Win32 clipboard. OpenClipboard/EmptyClipboard/SetClipboardData with
// CF_UNICODETEXT (UTF-16, NUL-terminated, in a GMEM_MOVEABLE global that the clipboard takes
// ownership of) and GetClipboardData/GlobalLock to read. Raw FFI, no dependencies. Written blind
// (no Windows host); compiled only on the windows target.

use std::os::raw::{c_int, c_void};

type Handle = *mut c_void;

/// CF_UNICODETEXT is UTF-16 text. Windows synthesizes it from CF_TEXT and vice versa, so this one
/// format covers any text on the clipboard.
const CF_UNICODETEXT: u32 = 13;
const GMEM_MOVEABLE: u32 = 0x0002;

#[link(name = "user32")]
unsafe extern "system" {
    fn OpenClipboard(hwnd: Handle) -> c_int;
    fn CloseClipboard() -> c_int;
    fn EmptyClipboard() -> c_int;
    fn SetClipboardData(format: u32, mem: Handle) -> Handle;
    fn GetClipboardData(format: u32) -> Handle;
    fn IsClipboardFormatAvailable(format: u32) -> c_int;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
    fn GlobalFree(mem: Handle) -> Handle;
    fn GlobalLock(mem: Handle) -> *mut c_void;
    fn GlobalUnlock(mem: Handle) -> c_int;
}

pub fn set_text(text: &str) -> bool {
    // UTF-16 with the required trailing NUL.
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return false; // another app holds the clipboard open
        }
        let ok = (|| {
            if EmptyClipboard() == 0 {
                return false;
            }
            let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
            if mem.is_null() {
                return false;
            }
            let dst = GlobalLock(mem);
            if dst.is_null() {
                GlobalFree(mem);
                return false;
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), dst as *mut u16, wide.len());
            GlobalUnlock(mem);
            if SetClipboardData(CF_UNICODETEXT, mem).is_null() {
                GlobalFree(mem); // ownership only transfers on success
                return false;
            }
            true
        })();
        CloseClipboard();
        ok
    }
}

pub fn get_text() -> Option<String> {
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return None;
        }
        let result = (|| {
            // The clipboard owns this handle: lock, copy out, unlock; never free it.
            let mem = GetClipboardData(CF_UNICODETEXT);
            if mem.is_null() {
                return None;
            }
            let p = GlobalLock(mem) as *const u16;
            if p.is_null() {
                return None;
            }
            let mut len = 0usize;
            while *p.add(len) != 0 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            GlobalUnlock(mem as Handle);
            Some(s)
        })();
        CloseClipboard();
        result
    }
}

pub fn has_text() -> bool {
    // Format probe; no OpenClipboard needed.
    unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) != 0 }
}

use crate::{Content, Error, MAX_BYTES, Representation};
#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterClipboardFormatW(name: *const u16) -> u32;
    fn GetActiveWindow() -> Handle;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalSize(mem: Handle) -> usize;
}
fn format_id(mime: &str) -> u32 {
    if mime == "text/plain" {
        return CF_UNICODETEXT;
    }
    if mime == "image/bmp" {
        return 8;
    }
    let name = match mime {
        "image/png" => "PNG",
        "image/svg+xml" => "image/svg+xml",
        m => m,
    };
    let wide: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
    unsafe { RegisterClipboardFormatW(wide.as_ptr()) }
}
pub fn write_content(content: &Content) -> Result<Vec<String>, Error> {
    unsafe {
        if OpenClipboard(GetActiveWindow()) == 0 {
            return Err(Error::Unavailable);
        }
        let result = (|| {
            if EmptyClipboard() == 0 {
                return Err(Error::Unavailable);
            }
            let mut written = Vec::new();
            for r in &content.0 {
                let format = format_id(&r.mime);
                if format == 0 {
                    continue;
                }
                let data = if r.mime == "text/plain" {
                    let Ok(text) = std::str::from_utf8(&r.bytes) else {
                        continue;
                    };
                    text.encode_utf16()
                        .chain(Some(0))
                        .flat_map(u16::to_le_bytes)
                        .collect::<Vec<_>>()
                } else if r.mime == "image/bmp" {
                    if !r.bytes.starts_with(b"BM") || r.bytes.len() < 26 {
                        continue;
                    }
                    r.bytes[14..].to_vec()
                } else {
                    r.bytes.as_ref().clone()
                };
                let mem = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1));
                if mem.is_null() {
                    continue;
                }
                let dst = GlobalLock(mem);
                if dst.is_null() {
                    GlobalFree(mem);
                    continue;
                }
                std::ptr::copy_nonoverlapping(data.as_ptr(), dst.cast(), data.len());
                GlobalUnlock(mem);
                if SetClipboardData(format, mem).is_null() {
                    GlobalFree(mem);
                } else {
                    written.push(r.mime.clone());
                }
            }
            if written.is_empty() {
                Err(Error::Unavailable)
            } else {
                let packed = crate::content::pack(content);
                let mut exact = (packed.len() as u32).to_le_bytes().to_vec();
                exact.extend_from_slice(&packed);
                let mem = GlobalAlloc(GMEM_MOVEABLE, exact.len());
                if !mem.is_null() {
                    let dst = GlobalLock(mem);
                    if !dst.is_null() {
                        std::ptr::copy_nonoverlapping(exact.as_ptr(), dst.cast(), exact.len());
                        GlobalUnlock(mem);
                        if SetClipboardData(format_id("application/x-day-clipboard-v1"), mem)
                            .is_null()
                        {
                            GlobalFree(mem);
                        }
                    } else {
                        GlobalFree(mem);
                    }
                }
                Ok(written)
            }
        })();
        CloseClipboard();
        result
    }
}
pub fn read_content(preferred: &[&str]) -> Result<Option<Representation>, Error> {
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err(Error::Unavailable);
        }
        let result = (|| {
            let exact = GetClipboardData(format_id("application/x-day-clipboard-v1"));
            if !exact.is_null() {
                let size = GlobalSize(exact);
                if size >= 4 && size <= MAX_BYTES + 16384 {
                    let p = GlobalLock(exact);
                    if !p.is_null() {
                        let all = std::slice::from_raw_parts(p.cast::<u8>(), size);
                        let len = u32::from_le_bytes(all[..4].try_into().unwrap()) as usize;
                        let content = all
                            .get(4..4usize.saturating_add(len))
                            .and_then(|b| crate::content::unpack(b).ok());
                        GlobalUnlock(exact);
                        if let Some(content) = content {
                            for mime in preferred {
                                if let Some(r) = content.0.iter().find(|r| r.mime == *mime) {
                                    return Ok(Some(r.clone()));
                                }
                            }
                        }
                    }
                }
            }
            for mime in preferred {
                let mem = GetClipboardData(format_id(mime));
                if mem.is_null() {
                    continue;
                }
                let len = GlobalSize(mem);
                if len > MAX_BYTES {
                    return Err(Error::TooLarge);
                }
                let p = GlobalLock(mem);
                if p.is_null() {
                    continue;
                }
                let mut bytes = std::slice::from_raw_parts(p.cast::<u8>(), len).to_vec();
                GlobalUnlock(mem);
                if *mime == "image/bmp" {
                    bytes = crate::content::dib_to_bmp(&bytes)?;
                }
                if *mime == "text/plain" {
                    let wide: Vec<_> = bytes
                        .chunks_exact(2)
                        .map(|s| u16::from_le_bytes([s[0], s[1]]))
                        .take_while(|c| *c != 0)
                        .collect();
                    bytes = String::from_utf16_lossy(&wide).into_bytes();
                }
                return Ok(Some(Representation::new(*mime, bytes)));
            }
            Ok(None)
        })();
        CloseClipboard();
        result
    }
}
