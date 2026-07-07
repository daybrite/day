// ---------------------------------------------------------------------------
// Qt: this crate's OWN shim (src/lib-qt-shim.cpp) — a QWidget that paints a QPixmap decoded from the
// bytes (aspect fit/fill) under a circle / rounded / rect clip, over the placeholder color, behind a
// flat C ABI. Bytes cross as a pointer + length; a SetBytes patch re-decodes (or clears on `None`).
// ---------------------------------------------------------------------------

use super::*;
use std::os::raw::{c_int, c_void};
use std::sync::Arc;

use day_qt::{Qt, QtHandle};
use day_spec::NodeId;

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

/// Content mode as understood by the shim: 1 fill (cover), 0 fit (contain).
fn mode_code(mode: ContentMode) -> c_int {
    match mode {
        ContentMode::Fill => 1,
        ContentMode::Fit => 0,
    }
}

fn set_bytes(h: &QtHandle, bytes: &Option<Arc<Vec<u8>>>) {
    match bytes {
        Some(b) => unsafe { day_remote_image_set_bytes(h.0, b.as_ptr(), b.len() as u64) },
        None => unsafe { day_remote_image_set_bytes(h.0, std::ptr::null(), 0) },
    }
}

fn make(_backend: &mut Qt, p: &RemoteImageProps, _id: NodeId) -> QtHandle {
    let (clip, radius) = clip_args(p.clip);
    let c = p.placeholder;
    let h = QtHandle(unsafe {
        day_remote_image_new(clip, radius, mode_code(p.mode), c.r, c.g, c.b, c.a)
    });
    set_bytes(&h, &p.bytes);
    h
}

fn update(_backend: &mut Qt, h: &QtHandle, patch: &RemoteImagePatch) {
    let RemoteImagePatch::SetBytes(bytes) = patch;
    set_bytes(h, bytes);
}

day_pieces::renderer!(day_qt::RENDERERS, Qt,
    kind: KIND, props: RemoteImageProps, patch: RemoteImagePatch,
    make: make, update: update, measure: day_pieces::fill_measure);
