//! day-arkui-sys — raw `extern "C"` declarations for the ArkUI/NAPI C++ shim (src/shim.cpp),
//! compiled by build.rs against the OpenHarmony NDK. Handles are opaque `ArkUI_NodeHandle`s; the
//! ArkTS host owns the window, and day mounts its native tree into a `NodeContent` slot.
//!
//! Only meaningful on the `*-linux-ohos` targets; the declarations exist unconditionally so the
//! crate type-checks on the host, but nothing links them off-device.

#![allow(clippy::missing_safety_doc)]

use std::os::raw::{c_char, c_int, c_void};

unsafe extern "C" {
    /// One-time setup: resolve the ArkUI NodeAPI + register the global event receiver.
    pub fn day_ark_init();

    /// Create a node for a day kind (0=stack 1=text 2=button 3=text_input 4=toggle 5=slider
    /// 6=scroll 7=column 8=loading_progress 9=image). Returns an opaque `ArkUI_NodeHandle`.
    pub fn day_ark_node_new(kind: c_int) -> *mut c_void;
    pub fn day_ark_node_dispose(node: *mut c_void);
    pub fn day_ark_add_child(parent: *mut c_void, child: *mut c_void);
    pub fn day_ark_insert_child(parent: *mut c_void, child: *mut c_void, pos: c_int);
    pub fn day_ark_remove_child(parent: *mut c_void, child: *mut c_void);

    pub fn day_ark_set_text(node: *mut c_void, s: *const c_char);
    pub fn day_ark_set_button_label(node: *mut c_void, s: *const c_char);
    pub fn day_ark_set_input_text(node: *mut c_void, s: *const c_char);
    pub fn day_ark_set_placeholder(node: *mut c_void, s: *const c_char);
    pub fn day_ark_set_toggle(node: *mut c_void, on: c_int);
    pub fn day_ark_set_slider(node: *mut c_void, v: f64);

    /// Set an image node's source URI (`NODE_IMAGE_SRC`). `s` is a `resource://RAWFILE/<path>`
    /// string — the only resource root the OpenHarmony NDK can address from native code (§18.3).
    pub fn day_ark_set_image_src(node: *mut c_void, s: *const c_char);
    /// Set an image node's scaling (`NODE_IMAGE_OBJECT_FIT`): ArkUI_ObjectFit CONTAIN=0 / COVER=1 /
    /// FILL=3 (§18.3).
    pub fn day_ark_set_image_fit(node: *mut c_void, fit: c_int);

    /// Absolute frame (day owns layout): position + explicit size, in vp.
    pub fn day_ark_set_frame(node: *mut c_void, x: f64, y: f64, w: f64, h: f64);
    pub fn day_ark_set_bg_color(node: *mut c_void, argb: u32);
    pub fn day_ark_set_font_size(node: *mut c_void, vp: f64);
    pub fn day_ark_set_font_color(node: *mut c_void, argb: u32);
    pub fn day_ark_set_corner_radius(node: *mut c_void, vp: f64);

    /// Measure `node` under a proposal (`<=0` = unbounded); result in vp via the out-params.
    pub fn day_ark_measure(
        node: *mut c_void,
        max_w: f64,
        max_h: f64,
        out_w: *mut f64,
        out_h: *mut f64,
    );

    /// Register a native event (0=click 1=text 2=toggle 3=slider); `id` returns as the event userData.
    pub fn day_ark_register_event(node: *mut c_void, kind: c_int, id: u64);

    /// Mount `node` into the ArkTS `NodeContent` slot. Returns 0 on success.
    pub fn day_ark_content_add(content: *mut c_void, node: *mut c_void) -> c_int;

    /// Post a closure to the main (JS) thread via libuv.
    pub fn day_ark_post(cb: extern "C" fn(*mut c_void), data: *mut c_void);

    /// Display density (px per vp), captured from the ArkTS host at start.
    pub fn day_ark_density() -> f64;

    /// Invoke the ArkTS-registered file picker (docs/files.md). `mode` 0 = open, 1 = save; `name`
    /// is the suggested save name, `src` the Day-staged temp file to save, `filters` the flattened
    /// filter list. The ArkTS side answers by calling the module's `onFileResult(req, path)`, which
    /// re-enters Rust as a `day_arkui_on_event(req, 5, 0, path)` present result (empty = cancel).
    /// A no-op (immediate cancel) if no picker was registered.
    pub fn day_ark_present_file(
        req: u64,
        mode: c_int,
        name: *const c_char,
        src: *const c_char,
        filters: *const c_char,
    );

    /// Whether a native `NativeResourceManager` was captured from the ArkTS host (via the shim's
    /// `registerResourceManager` NAPI export). Returns 1 if the rawfile resource opener can serve
    /// reads, 0 otherwise. See [`day_ark_res_open`] (§18.3).
    pub fn day_ark_res_available() -> c_int;

    /// Open the rawfile at `path` (e.g. `"day/numbers.bin"`, relative to the rawfile root) for
    /// efficient read-only access. On success returns 1 and fills `*out_data`/`*out_len` with a
    /// zero-copy view (an mmap of the uncompressed `.hap` entry; a heap copy if mmap is unavailable)
    /// plus `*out_handle`, an opaque cleanup token to pass to [`day_ark_res_close`]. Returns 0 if no
    /// resource manager was registered or the file is missing.
    pub fn day_ark_res_open(
        path: *const c_char,
        out_data: *mut *const u8,
        out_len: *mut usize,
        out_handle: *mut *mut c_void,
    ) -> c_int;

    /// Release a view previously returned by [`day_ark_res_open`] (munmap or free, then drop the
    /// token). Safe to call with a null handle.
    pub fn day_ark_res_close(handle: *mut c_void);
}
