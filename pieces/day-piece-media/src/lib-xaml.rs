// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// XAML: this crate's OWN shim (src/lib-xaml-shim.cpp) wrapping a Windows.UI.Xaml.Controls
// MediaPlayerElement, boxed into a day handle via day-xaml-sys's `day_xaml_box`/`day_xaml_unbox`
// seam (like the picker/webview xaml pieces). MediaPlayerElement is core system XAML — no
// availability caveat like the EdgeHTML WebView. `.controls` maps to AreTransportControlsEnabled;
// looping/muted/autoplay/volume live on the backing MediaPlayer. A sound-only player is a
// collapsed element of no size. Playback state comes back through one file-static C callback the
// shim calls from the player's PlaybackSession. Windows-only; built + verified in CI.
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};

use day_spec::{NodeId, Proposal, Size};
use day_xaml::{WinHandle, Xaml};

unsafe extern "C" {
    fn day_media_xaml_new(
        id: u64,
        url: *const c_char,
        autoplay: c_int,
        looping: c_int,
        muted: c_int,
        controls: c_int,
        audio_only: c_int,
        volume: f64,
    ) -> *mut c_void;
    fn day_media_xaml_set_state_cb(cb: extern "C" fn(u64, c_int, *const c_char));
    fn day_media_xaml_load(w: *mut c_void, url: *const c_char);
    fn day_media_xaml_play(w: *mut c_void);
    fn day_media_xaml_pause(w: *mut c_void);
    fn day_media_xaml_stop(w: *mut c_void);
    fn day_media_xaml_set_volume(w: *mut c_void, volume: f64);
    fn day_media_xaml_is_audio_only(w: *mut c_void) -> c_int;
}

/// One state report from the shim: the code is the piece's own, the text an error's message.
extern "C" fn on_state(id: u64, code: c_int, text: *const c_char) {
    let text = if text.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    };
    day_xaml::emit(
        NodeId(id),
        Event::Custom {
            tag: report::TAG,
            num: code as f64,
            text,
        },
    );
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

fn make(_backend: &mut Xaml, p: &MediaProps, id: NodeId) -> WinHandle {
    static STATE_CB: std::sync::Once = std::sync::Once::new();
    STATE_CB.call_once(|| unsafe { day_media_xaml_set_state_cb(on_state) });
    WinHandle(unsafe {
        day_media_xaml_new(
            id.0,
            cstr(&p.url).as_ptr(),
            p.autoplay as c_int,
            p.looping as c_int,
            p.muted as c_int,
            p.controls as c_int,
            p.audio_only as c_int,
            p.volume.clamp(0.0, 1.0),
        )
    })
}

fn update(_backend: &mut Xaml, h: &WinHandle, patch: &MediaPatch) {
    unsafe {
        match patch {
            MediaPatch::Load(url) => day_media_xaml_load(h.0, cstr(url).as_ptr()),
            MediaPatch::Play => day_media_xaml_play(h.0),
            MediaPatch::Pause => day_media_xaml_pause(h.0),
            MediaPatch::Stop => day_media_xaml_stop(h.0),
            MediaPatch::Volume(v) => day_media_xaml_set_volume(h.0, v.clamp(0.0, 1.0)),
        }
    }
}

/// A sound-only player takes no room; a video fills what it is offered.
fn measure(backend: &mut Xaml, h: &WinHandle, proposal: Proposal) -> Size {
    if unsafe { day_media_xaml_is_audio_only(h.0) } != 0 {
        return Size::ZERO;
    }
    day_pieces::fill_measure(backend, h, proposal)
}

day_pieces::renderer!(day_xaml::RENDERERS, Xaml,
    kind: KIND, props: MediaProps, patch: MediaPatch,
    make: make, update: update, measure: measure);
