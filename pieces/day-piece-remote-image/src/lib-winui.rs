// ---------------------------------------------------------------------------
// WinUI: this crate's OWN shim (src/lib-winui-shim.cpp) — an Ellipse (circle) or Border+Image boxed
// into a day handle via day-winui-sys's `day_winui_box`/`day_winui_unbox` seam (like the picker/media
// winui pieces). Bytes cross as a pointer + length; a SetBytes patch re-decodes (or clears on
// `None`). Windows-only, built in CI, NOT verified locally (see the shim's header caveats).
// ---------------------------------------------------------------------------

use super::*;
use std::os::raw::{c_int, c_void};
use std::sync::Arc;

use day_spec::NodeId;
use day_winui::{WinHandle, WinUi};

unsafe extern "C" {
    fn day_remote_image_new(
        clip: c_int,
        radius: f64,
        mode: c_int,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) -> *mut c_void;
    fn day_remote_image_set_bytes(w: *mut c_void, data: *const u8, len: u64);
}

/// (clip discriminant, radius) as understood by the shim: 0 none, 1 circle, 2 rounded.
fn clip_args(clip: Clip) -> (c_int, f64) {
    match clip {
        Clip::None => (0, 0.0),
        Clip::Circle => (1, 0.0),
        Clip::Rounded(r) => (2, r),
    }
}

/// Content mode as understood by the shim: 1 fill (UniformToFill), 0 fit (Uniform).
fn mode_code(mode: ContentMode) -> c_int {
    match mode {
        ContentMode::Fill => 1,
        ContentMode::Fit => 0,
    }
}

fn set_bytes(h: &WinHandle, bytes: &Option<Arc<Vec<u8>>>) {
    match bytes {
        Some(b) => unsafe { day_remote_image_set_bytes(h.0, b.as_ptr(), b.len() as u64) },
        None => unsafe { day_remote_image_set_bytes(h.0, std::ptr::null(), 0) },
    }
}

fn make(_backend: &mut WinUi, p: &RemoteImageProps, _id: NodeId) -> WinHandle {
    let (clip, radius) = clip_args(p.clip);
    let c = p.placeholder;
    let h = WinHandle(unsafe {
        day_remote_image_new(clip, radius, mode_code(p.mode), c.r, c.g, c.b, c.a)
    });
    set_bytes(&h, &p.bytes);
    h
}

fn update(_backend: &mut WinUi, h: &WinHandle, patch: &RemoteImagePatch) {
    let RemoteImagePatch::SetBytes(bytes) = patch;
    set_bytes(h, bytes);
}

day_pieces::renderer!(day_winui::RENDERERS, WinUi,
    kind: KIND, props: RemoteImageProps, patch: RemoteImagePatch,
    make: make, update: update, measure: day_pieces::fill_measure);
