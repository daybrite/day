//! day-winui-sys — raw `extern "C"` declarations for the C++/WinRT XAML-Islands shim
//! (src/shim.cpp) compiled by build.rs. Handles are opaque `Windows.UI.Xaml.UIElement*`
//! heap-boxed by the shim; `day_winui_delete` releases the WinRT reference.

#![cfg(windows)]

use std::os::raw::{c_char, c_double, c_int, c_void};

unsafe extern "C" {
    // window / app lifecycle
    pub fn day_winui_window_new(title: *const c_char, w: c_int, h: c_int) -> *mut c_void;
    pub fn day_winui_window_root(win: *mut c_void) -> *mut c_void;
    pub fn day_winui_window_show(win: *mut c_void);
    pub fn day_winui_window_on_resize(win: *mut c_void, cb: extern "C" fn(c_int, c_int));
    pub fn day_winui_run(win: *mut c_void);
    pub fn day_winui_post(cb: extern "C" fn(*mut c_void), data: *mut c_void);

    // containers
    pub fn day_winui_container_new() -> *mut c_void;
    pub fn day_winui_scroll_new() -> *mut c_void;
    pub fn day_winui_container_set_bg(w: *mut c_void, argb: u32);
    pub fn day_winui_canvas_new() -> *mut c_void;

    // leaves
    pub fn day_winui_label_new(text: *const c_char) -> *mut c_void;
    pub fn day_winui_label_set_text(w: *mut c_void, text: *const c_char);
    pub fn day_winui_label_set_font(w: *mut c_void, pt: c_double, bold: c_int);

    pub fn day_winui_button_new(
        title: *const c_char,
        id: u64,
        cb: extern "C" fn(u64),
    ) -> *mut c_void;
    pub fn day_winui_button_set_title(w: *mut c_void, title: *const c_char);

    pub fn day_winui_toggle_new(on: c_int, id: u64, cb: extern "C" fn(u64, c_int)) -> *mut c_void;
    pub fn day_winui_toggle_set(w: *mut c_void, on: c_int);

    pub fn day_winui_slider_new(
        value: c_int,
        id: u64,
        cb: extern "C" fn(u64, c_int),
    ) -> *mut c_void;
    pub fn day_winui_slider_set(w: *mut c_void, value: c_int);

    pub fn day_winui_progress_new(determinate: c_int, value: c_int) -> *mut c_void;
    pub fn day_winui_progress_set(w: *mut c_void, value: c_int);

    pub fn day_winui_textbox_new(
        text: *const c_char,
        placeholder: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, *const c_char),
    ) -> *mut c_void;
    pub fn day_winui_textbox_set_text(w: *mut c_void, text: *const c_char);
    pub fn day_winui_textbox_set_placeholder(w: *mut c_void, text: *const c_char);

    pub fn day_winui_divider_new() -> *mut c_void;
    pub fn day_winui_image_new(uri: *const c_char) -> *mut c_void;

    pub fn day_winui_combo_new(
        items_joined: *const c_char,
        selected: c_int,
        id: u64,
        cb: extern "C" fn(u64, c_int),
    ) -> *mut c_void;
    pub fn day_winui_combo_set_items(w: *mut c_void, items_joined: *const c_char);
    pub fn day_winui_combo_set_selected(w: *mut c_void, idx: c_int);

    // tree / geometry / props
    pub fn day_winui_add_child(parent: *mut c_void, child: *mut c_void);
    pub fn day_winui_remove_child(parent: *mut c_void, child: *mut c_void);
    pub fn day_winui_delete(w: *mut c_void);
    pub fn day_winui_set_geometry(w: *mut c_void, x: c_int, y: c_int, width: c_int, height: c_int);
    pub fn day_winui_measure(
        w: *mut c_void,
        avail_w: c_double,
        avail_h: c_double,
        out_w: *mut c_double,
        out_h: *mut c_double,
    );
    pub fn day_winui_set_enabled(w: *mut c_void, enabled: c_int);
    pub fn day_winui_set_name(w: *mut c_void, name: *const c_char);
    /// Capture the window's client area to a PNG file. Returns 0 on success.
    pub fn day_winui_snapshot_png(win: *mut c_void, path: *const c_char) -> c_int;
}
