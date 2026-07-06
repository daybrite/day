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
    /// 6=scroll 7=column 8=loading_progress). Returns an opaque `ArkUI_NodeHandle`.
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
}
