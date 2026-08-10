// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

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
    pub fn day_ark_log(msg: *const std::ffi::c_char);
    // Secondary windows (docs/windows.md): the ArkTS-registered multiton launchers.
    pub fn day_ark_has_windows() -> c_int;
    pub fn day_ark_open_window(node: u64, title: *const std::ffi::c_char) -> c_int;
    pub fn day_ark_close_window(node: u64);

    /// Create a node for a day kind (0=stack 1=text 2=button 3=text_input 4=toggle 5=slider
    /// 6=scroll 7=column 8=loading_progress 9=image). Returns an opaque `ArkUI_NodeHandle`.
    pub fn day_ark_node_new(kind: c_int) -> *mut c_void;
    pub fn day_ark_node_dispose(node: *mut c_void);
    pub fn day_ark_add_child(parent: *mut c_void, child: *mut c_void);
    pub fn day_ark_scroll_direction(n: *mut c_void, horizontal: c_int);
    pub fn day_ark_scroll_to_rect(n: *mut c_void, x: f32, y: f32, w: f32, h: f32, animated: c_int);
    pub fn day_ark_insert_child(parent: *mut c_void, child: *mut c_void, pos: c_int);
    pub fn day_ark_remove_child(parent: *mut c_void, child: *mut c_void);
    /// Navigation bridge (docs/navigation.md): mount `page` into a fresh ArkTS NodeContent and
    /// push a NavDestination for it (0 = ok); pop the top destination; retitle the top; unmount
    /// a popped page's node from its content before disposal.
    pub fn day_ark_nav_push(page: *mut c_void, key: u64, title: *const c_char) -> i32;
    pub fn day_ark_nav_pop();
    pub fn day_ark_nav_set_title(title: *const c_char);
    pub fn day_ark_nav_set_guard(on: i32);
    /// Set the trailing title-bar action (NavProps::bar_action, docs/navigation.md): its bundled
    /// image name, accessible label, and dispatch id. No-op if the ArkTS host predates the seam.
    pub fn day_ark_nav_set_menu(icon: *const c_char, label: *const c_char, action: u64);
    /// Open a URL via the ArkTS opener (the `link` piece's seam). No-op if unregistered.
    pub fn day_ark_open_url(url: *const c_char);
    pub fn day_ark_nav_remove(key: u64, page: *mut c_void);
    pub fn day_ark_nav_forget(key: u64);
    /// ArkTS-built piece components (docs/extending.md): build one and return its FrameNode as an
    /// `ArkUI_NodeHandle` (null when nothing is registered or the factory declined `kind`); send it
    /// a command; release its BuilderNode. `props`/`cmd`/`arg` are opaque to the bridge — the piece
    /// owns both ends. Prefer the safe `day_arkui::piece` wrappers over these.
    pub fn day_ark_piece_make(kind: *const c_char, id: u64, props: *const c_char) -> *mut c_void;
    pub fn day_ark_piece_update(id: u64, cmd: *const c_char, arg: *const c_char);
    pub fn day_ark_piece_dispose(id: u64);
    /// Menu styling: flex-grow within a Row/Column; a conventional hairline list separator.
    pub fn day_ark_set_flex_grow(n: *mut c_void, g: f64);
    pub fn day_ark_menu_separator(n: *mut c_void, argb: u32);

    pub fn day_ark_set_text(node: *mut c_void, s: *const c_char);
    /// Make a Text node's text user-selectable (the `.selectable()` modifier). No-op on non-text.
    pub fn day_ark_label_set_selectable(node: *mut c_void, on: c_int);
    pub fn day_ark_set_button_label(node: *mut c_void, s: *const c_char);
    pub fn day_ark_set_input_text(node: *mut c_void, s: *const c_char);
    pub fn day_ark_set_placeholder(node: *mut c_void, s: *const c_char);
    pub fn day_ark_set_textarea_text(node: *mut c_void, s: *const c_char);
    pub fn day_ark_set_textarea_placeholder(node: *mut c_void, s: *const c_char);
    pub fn day_ark_set_picker(node: *mut c_void, options_semi: *const c_char, selected: u32);
    pub fn day_ark_set_picker_selected(node: *mut c_void, selected: u32);
    pub fn day_ark_set_toggle(node: *mut c_void, on: c_int);
    pub fn day_ark_set_slider(node: *mut c_void, v: f64);

    /// Set an image node's source URI (`NODE_IMAGE_SRC`). `s` is a `resource://RAWFILE/<path>`
    /// string — the only resource root the OpenHarmony NDK can address from native code (§18.3).
    pub fn day_ark_set_image_src(node: *mut c_void, s: *const c_char);
    /// Set an image node's scaling (`NODE_IMAGE_OBJECT_FIT`): ArkUI_ObjectFit CONTAIN=0 / COVER=1 /
    /// FILL=3 (§18.3).
    pub fn day_ark_set_image_fit(node: *mut c_void, fit: c_int);
    /// SVG-only recolor (`NODE_IMAGE_FILL_COLOR`): repaints every path of an SVG src with `argb`;
    /// raster sources ignore it (docs/vectors.md).
    pub fn day_ark_set_image_fill(node: *mut c_void, argb: u32);
    /// Whether rawfile `path` (e.g. "day/home.svg") exists in the app package; 0 before the entry
    /// ability registers the resource manager (docs/vectors.md).
    pub fn day_ark_rawfile_exists(path: *const c_char) -> i32;
    /// One margin (vp) on all four sides (`NODE_MARGIN`) — symmetric, so RTL needs no flip.
    pub fn day_ark_set_margin(node: *mut c_void, vp: f64);

    /// Absolute frame (day owns layout): position + explicit size, in vp.
    pub fn day_ark_set_frame(node: *mut c_void, x: f64, y: f64, w: f64, h: f64);
    /// Explicit size only (no position) — for children whose parent places them (Swiper pages).
    pub fn day_ark_set_size(node: *mut c_void, w: f64, h: f64);
    pub fn day_ark_set_bg_color(node: *mut c_void, argb: u32);
    pub fn day_ark_set_font_size(node: *mut c_void, vp: f64);
    pub fn day_ark_set_font_color(node: *mut c_void, argb: u32);
    /// Bundled custom font family (§18.4; registered by the platform/harmony scaffold's EntryAbility).
    pub fn day_ark_set_font_family(node: *mut c_void, family: *const c_char);
    pub fn day_ark_set_font_feature(node: *mut c_void, feature: *const c_char);
    pub fn day_ark_set_corner_radius(node: *mut c_void, vp: f64);
    /// Determinate progress fraction (0..1), mapped onto ArkUI's NODE_PROGRESS_VALUE/TOTAL.
    pub fn day_ark_set_progress(node: *mut c_void, fraction: f64);
    /// Visibility: 0 = VISIBLE, else NONE (removed from layout — one TABS page shown at a time).
    pub fn day_ark_set_visibility(node: *mut c_void, visible: c_int);
    /// The active page index of a Swiper (`NODE_SWIPER_INDEX`).
    pub fn day_ark_set_swiper_index(node: *mut c_void, i: c_int);
    /// Configure a Swiper as a tab pager (show indicator, disable looping).
    pub fn day_ark_swiper_setup(node: *mut c_void);
    /// Accessibility (§13): screen-reader `label`; `hidden` drops the node + subtree from the tree.
    pub fn day_ark_set_a11y(node: *mut c_void, label: *const c_char, hidden: c_int);

    /// Measure `node` under a proposal (`<=0` = unbounded); result in vp via the out-params.
    pub fn day_ark_measure(
        node: *mut c_void,
        max_w: f64,
        max_h: f64,
        out_w: *mut f64,
        out_h: *mut f64,
    );

    /// Register a native event (0=click 1=text 2=toggle 3=slider 6=swiper); `id` returns as userData.
    pub fn day_ark_register_event(node: *mut c_void, kind: c_int, id: u64);
    // Focus (docs/focus.md): observe gain/blur (+ text-input submit); drive via
    // OH_ArkUI_FocusRequest / OH_ArkUI_FocusClear.
    pub fn day_ark_enable_focus(node: *mut c_void, id: u64, is_text_input: c_int);
    pub fn day_ark_focus(node: *mut c_void, focused: c_int);

    /// Canvas (§11): register the custom node's on-draw receiver.
    pub fn day_ark_canvas_init(node: *mut c_void);
    /// Store a canvas node's encoded display list (`nums`/`count` + a 0x1F-joined `texts`) and
    /// request a repaint. The buffers are copied; the caller keeps ownership.
    pub fn day_ark_set_canvas_ops(
        node: *mut c_void,
        nums: *const f64,
        count: u32,
        texts: *const c_char,
    );

    /// Recycling list (docs/list.md): create the node's `NodeAdapter`, binding it to `host_id` so
    /// the row callbacks (`day_arkui_list_count` / `_bind`) can find the source. `row_h_vp` is the
    /// uniform row height in vp (0 = content-sized). With `reorderable`, cells become draggable
    /// and drops route through `day_arkui_list_can_move` / `day_arkui_list_move` (docs/list.md).
    pub fn day_ark_list_init(node: *mut c_void, host_id: u64, row_h_vp: f64, reorderable: u32);
    /// Re-query the list's row count (the adapter re-fetches its visible cells).
    pub fn day_ark_list_reload(node: *mut c_void);
    /// Scroll the list so its last row is fully visible (docs/list.md).
    pub fn day_ark_list_scroll_to_end(node: *mut c_void);
    /// Scroll the list so row `index` is visible (clamped by ArkUI; docs/list.md).
    pub fn day_ark_list_scroll_to_row(node: *mut c_void, index: u32);
    /// Style a NAV_MENU / tab-bar row: full width, `height_vp` tall, left-aligned padded text.
    pub fn day_ark_style_row(node: *mut c_void, height_vp: f64);

    /// Mount `node` into the ArkTS `NodeContent` slot. Returns 0 on success.
    pub fn day_ark_content_add(content: *mut c_void, node: *mut c_void) -> c_int;

    /// Post a closure to the main (JS) thread via libuv.
    pub fn day_ark_post(cb: extern "C" fn(*mut c_void), data: *mut c_void);

    /// Post a closure to the main (JS) thread after `ms` milliseconds (a one-shot uv_timer) —
    /// the frame clock's tick source (§8.4). JS thread only.
    pub fn day_ark_post_delayed(cb: extern "C" fn(*mut c_void), data: *mut c_void, ms: u32);

    /// Attach a native pan recognizer to `node` (docs/shapes.md): drag phases reach Rust as
    /// kind-11 gesture events against `id` ("x,y,tx,ty" px; 1=began 2=changed 3=ended).
    pub fn day_ark_enable_pan(node: *mut c_void, id: u64);

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

/// Parity test for the event-kind wire table: shim.cpp's `DAY_K_*` defines must mirror
/// `day_spec::bridge::BridgeKind`. Host-runnable — pure text against the enum.
#[cfg(test)]
mod bridge_kinds_parity {
    #[test]
    fn shim_defines_match_the_rust_enum() {
        use day_spec::bridge::BridgeKind;
        let shim = include_str!("shim.cpp");
        let mut found = std::collections::BTreeMap::new();
        for line in shim.lines() {
            if let Some(rest) = line.trim().strip_prefix("#define DAY_K_")
                && let Some((name, value)) = rest.split_once(' ')
            {
                let value: i32 = value
                    .trim()
                    .parse()
                    .unwrap_or_else(|_| panic!("unparsable DAY_K_{name} line: {line}"));
                assert!(
                    found.insert(format!("DAY_K_{name}"), value).is_none(),
                    "duplicate define DAY_K_{name}"
                );
            }
        }
        let expect = [
            ("DAY_K_PRESSED", BridgeKind::Pressed),
            ("DAY_K_TEXT_CHANGED", BridgeKind::TextChanged),
            ("DAY_K_TOGGLE_CHANGED", BridgeKind::ToggleChanged),
            ("DAY_K_VALUE_CHANGED", BridgeKind::ValueChanged),
            ("DAY_K_SELECTION_CHANGED", BridgeKind::SelectionChanged),
            ("DAY_K_GESTURE", BridgeKind::Gesture),
            ("DAY_K_CUSTOM", BridgeKind::Custom),
            ("DAY_K_PRESENT_FILE", BridgeKind::PresentFile),
            ("DAY_K_FOCUS_CHANGED", BridgeKind::FocusChanged),
            ("DAY_K_SUBMITTED", BridgeKind::Submitted),
            ("DAY_K_VALUE_COMMITTED", BridgeKind::ValueCommitted),
        ];
        assert_eq!(found.len(), expect.len(), "define count drifted: {found:?}");
        for (name, kind) in expect {
            assert_eq!(
                found.get(name).copied(),
                Some(kind as i32),
                "{name} drifted from BridgeKind::{kind:?}"
            );
        }
    }
}
