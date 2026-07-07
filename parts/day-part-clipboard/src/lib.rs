//! day-part-clipboard — a HEADLESS cross-platform plain-text clipboard API. No UI; any Rust code can
//! depend on this crate and call [`set_text`] / [`get_text`] / [`has_text`] to reach the system
//! clipboard through the platform's NATIVE API.
//!
//! ```no_run
//! day_part_clipboard::set_text("hello");
//! if let Some(text) = day_part_clipboard::get_text() {
//!     println!("clipboard holds: {text}");
//! }
//! ```
//!
//! Platform selection is purely `#[cfg(target_os)]`/`#[cfg(target_env)]` (the clipboard is an OS
//! concern, not a widget-toolkit one): macOS uses `NSPasteboard` (toolkit-independent — it works
//! under day-qt binaries too), iOS `UIPasteboard`, Windows the Win32 clipboard (`CF_UNICODETEXT`),
//! desktop Linux shells out to `wl-copy`/`wl-paste` (Wayland) with an `xclip` (X11) fallback,
//! HarmonyOS the native Pasteboard/UDMF C API, and Android `ClipboardManager` (via a Java shim
//! staged by `day build`). Platforms without a clipboard API return `false`/`None`.
//!
//! Platform caveats: Android 10+ only lets the app read the clipboard while it has input focus —
//! [`get_text`] returns `None` in the background. Desktop Linux requires `wl-clipboard` or `xclip`
//! to be installed (both are ubiquitous distro packages).

/// Place `text` on the system clipboard as plain text, replacing the previous contents.
/// Returns `true` on success, `false` when the platform has no clipboard API or the write failed
/// (e.g. no `wl-copy`/`xclip` binary on Linux).
pub fn set_text(text: &str) -> bool {
    imp::set_text(text)
}

/// Read the current clipboard contents as plain text. Returns `None` when the clipboard is empty,
/// holds no text representation, or the platform denies access (e.g. an unfocused Android app).
pub fn get_text() -> Option<String> {
    imp::get_text()
}

/// Whether the clipboard currently holds text. Cheap where the platform offers a dedicated check
/// (`UIPasteboard.hasStrings`, Win32 `IsClipboardFormatAvailable`, …); otherwise it reads the text.
pub fn has_text() -> bool {
    imp::has_text()
}

// ---------------------------------------------------------------------------
// Per-OS implementations. Each exposes `set_text` / `get_text` / `has_text`.
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;

#[cfg(target_os = "ios")]
#[path = "ios.rs"]
mod imp;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;

// Desktop Linux shells out to the session clipboard tools; HarmonyOS (also `target_os = "linux"`)
// has no such tools and uses its own native Pasteboard C API instead.
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
#[path = "linux.rs"]
mod imp;

#[cfg(all(target_os = "linux", target_env = "ohos"))]
#[path = "ohos.rs"]
mod imp;

#[cfg(target_os = "android")]
#[path = "android.rs"]
mod imp;

// Any other platform: no clipboard API.
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
)))]
mod imp {
    pub fn set_text(_text: &str) -> bool {
        false
    }
    pub fn get_text() -> Option<String> {
        None
    }
    pub fn has_text() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    // Reading must never panic, whether or not the host has a clipboard (headless CI runners often
    // have no session clipboard at all). Read-only so it can't race the roundtrip test below.
    #[test]
    fn read_does_not_panic() {
        let text = super::get_text();
        let has = super::has_text();
        // When text is present, the two views of the clipboard must agree.
        if text.is_some() {
            assert!(has);
        }
    }

    // macOS: a real write→read roundtrip through NSPasteboard. Tolerant of headless failure
    // (a session-less runner may refuse pasteboard access), and restores the previous contents.
    #[cfg(target_os = "macos")]
    #[test]
    fn roundtrip_macos() {
        let previous = super::get_text();
        let marker = format!("day-part-clipboard roundtrip {}", std::process::id());
        if super::set_text(&marker) {
            assert_eq!(super::get_text().as_deref(), Some(marker.as_str()));
            assert!(super::has_text());
            // Be polite: put back whatever was on the user's clipboard.
            if let Some(prev) = previous {
                super::set_text(&prev);
            }
        }
    }
}
