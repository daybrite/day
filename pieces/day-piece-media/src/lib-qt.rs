// ---------------------------------------------------------------------------
// Qt: this crate's OWN shim (src/lib-qt-shim.cpp) wrapping QMediaPlayer + QAudioOutput +
// QVideoWidget behind a flat C ABI. build.rs compiles it AND links Qt6MultimediaWidgets (which
// day-qt-sys does not); where that module is absent the shim degrades to a URL label (see the
// shim's #else). QVideoWidget has no built-in chrome, so `.controls` is a no-op on Qt — drive
// playback with the `.play()/.pause()` triggers.
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};

use day_qt::{Qt, QtHandle};
use day_spec::NodeId;

unsafe extern "C" {
    fn day_media_new(
        url: *const c_char,
        autoplay: c_int,
        looping: c_int,
        muted: c_int,
    ) -> *mut c_void;
    fn day_media_load(w: *mut c_void, url: *const c_char);
    fn day_media_play(w: *mut c_void);
    fn day_media_pause(w: *mut c_void);
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

fn make(_backend: &mut Qt, p: &MediaProps, _id: NodeId) -> QtHandle {
    QtHandle(unsafe {
        day_media_new(
            cstr(&p.url).as_ptr(),
            p.autoplay as c_int,
            p.looping as c_int,
            p.muted as c_int,
        )
    })
}

fn update(_backend: &mut Qt, h: &QtHandle, patch: &MediaPatch) {
    unsafe {
        match patch {
            MediaPatch::Load(url) => day_media_load(h.0, cstr(url).as_ptr()),
            MediaPatch::Play => day_media_play(h.0),
            MediaPatch::Pause => day_media_pause(h.0),
        }
    }
}

day_pieces::renderer!(day_qt::RENDERERS, Qt,
    kind: KIND, props: MediaProps, patch: MediaPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
