// Windows: the Win32 clipboard — OpenClipboard/EmptyClipboard/SetClipboardData with
// CF_UNICODETEXT (UTF-16, NUL-terminated, in a GMEM_MOVEABLE global that the clipboard takes
// ownership of) and GetClipboardData/GlobalLock to read. Raw FFI — no dependencies. Written blind
// (no Windows host); compiled only on the windows target.

use std::os::raw::{c_int, c_void};

type Handle = *mut c_void;

/// CF_UNICODETEXT — UTF-16 text. Windows synthesizes it from CF_TEXT and vice versa, so this one
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
            // The clipboard owns this handle — lock, copy out, unlock; never free it.
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
    // Format probe — no OpenClipboard needed.
    unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) != 0 }
}
