// ---------------------------------------------------------------------------
// XAML: this crate's OWN shim (src/lib-xaml-shim.cpp) wrapping a Windows.UI.Xaml.Controls
// MediaPlayerElement, boxed into a day handle via day-xaml-sys's `day_xaml_box`/`day_xaml_unbox`
// seam (like the picker/webview xaml pieces). MediaPlayerElement is core system XAML — no
// availability caveat like the EdgeHTML WebView. `.controls` maps to AreTransportControlsEnabled;
// looping/muted/autoplay live on the backing MediaPlayer. Windows-only; built + verified in CI.
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};

use day_spec::NodeId;
use day_xaml::{WinHandle, Xaml};

unsafe extern "C" {
    fn day_media_xaml_new(
        url: *const c_char,
        autoplay: c_int,
        looping: c_int,
        muted: c_int,
        controls: c_int,
    ) -> *mut c_void;
    fn day_media_xaml_load(w: *mut c_void, url: *const c_char);
    fn day_media_xaml_play(w: *mut c_void);
    fn day_media_xaml_pause(w: *mut c_void);
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

fn make(_backend: &mut Xaml, p: &MediaProps, _id: NodeId) -> WinHandle {
    WinHandle(unsafe {
        day_media_xaml_new(
            cstr(&p.url).as_ptr(),
            p.autoplay as c_int,
            p.looping as c_int,
            p.muted as c_int,
            p.controls as c_int,
        )
    })
}

fn update(_backend: &mut Xaml, h: &WinHandle, patch: &MediaPatch) {
    unsafe {
        match patch {
            MediaPatch::Load(url) => day_media_xaml_load(h.0, cstr(url).as_ptr()),
            MediaPatch::Play => day_media_xaml_play(h.0),
            MediaPatch::Pause => day_media_xaml_pause(h.0),
        }
    }
}

day_pieces::renderer!(day_xaml::RENDERERS, Xaml,
    kind: KIND, props: MediaProps, patch: MediaPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
