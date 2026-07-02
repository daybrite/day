//! day-qt-sys — raw `extern "C"` declarations for the Qt 6 shim compiled by build.rs.
//! Handles are opaque `QWidget*`; ownership stays with Qt's parent/child tree (day's release
//! calls `day_qt_delete` = deleteLater, per §4.3's deferred-destruction allowance).

use std::os::raw::{c_char, c_double, c_int, c_void};

unsafe extern "C" {
    pub fn day_qt_app_new() -> *mut c_void;
    pub fn day_qt_app_run(app: *mut c_void);
    pub fn day_qt_window_new(title: *const c_char, w: c_int, h: c_int) -> *mut c_void;
    pub fn day_qt_window_show(win: *mut c_void);
    pub fn day_qt_container_new() -> *mut c_void;

    pub fn day_qt_label_new(text: *const c_char) -> *mut c_void;
    pub fn day_qt_label_set_text(w: *mut c_void, text: *const c_char);
    pub fn day_qt_label_set_font(w: *mut c_void, pt: c_double, bold: c_int);
    pub fn day_qt_label_height_for_width(w: *mut c_void, width: c_int) -> c_int;

    pub fn day_qt_button_new(
        title: *const c_char,
        id: u64,
        cb: extern "C" fn(u64),
    ) -> *mut c_void;
    pub fn day_qt_button_set_title(w: *mut c_void, title: *const c_char);

    pub fn day_qt_checkbox_new(on: c_int, id: u64, cb: extern "C" fn(u64, c_int)) -> *mut c_void;
    pub fn day_qt_checkbox_set(w: *mut c_void, on: c_int);

    pub fn day_qt_slider_new(value: c_int, id: u64, cb: extern "C" fn(u64, c_int))
    -> *mut c_void;
    pub fn day_qt_slider_set(w: *mut c_void, value: c_int);

    pub fn day_qt_lineedit_new(
        text: *const c_char,
        placeholder: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, *const c_char),
    ) -> *mut c_void;
    pub fn day_qt_lineedit_set_text(w: *mut c_void, text: *const c_char);
    pub fn day_qt_lineedit_set_placeholder(w: *mut c_void, text: *const c_char);

    pub fn day_qt_separator_new() -> *mut c_void;

    pub fn day_qt_scroll_new() -> *mut c_void;
    pub fn day_qt_scroll_content(w: *mut c_void) -> *mut c_void;
    pub fn day_qt_scroll_set_content_size(w: *mut c_void, cw: c_int, ch: c_int);

    pub fn day_qt_add_child(parent: *mut c_void, child: *mut c_void);
    pub fn day_qt_remove_child(child: *mut c_void);
    pub fn day_qt_delete(w: *mut c_void);
    pub fn day_qt_set_geometry(w: *mut c_void, x: c_int, y: c_int, width: c_int, height: c_int);
    pub fn day_qt_size_hint(w: *mut c_void, out_w: *mut c_double, out_h: *mut c_double);
    pub fn day_qt_set_enabled(w: *mut c_void, enabled: c_int);
    pub fn day_qt_set_object_name(w: *mut c_void, name: *const c_char);
    pub fn day_qt_set_tooltip(w: *mut c_void, text: *const c_char);

    pub fn day_qt_canvas_new() -> *mut c_void;
    pub fn day_qt_canvas_set_ops(w: *mut c_void, nums: *const c_double, n: c_int, texts_joined: *const c_char);
    pub fn day_qt_image_new(path: *const c_char) -> *mut c_void;
    pub fn day_qt_post(cb: extern "C" fn(*mut c_void), data: *mut c_void);
    pub fn day_qt_snapshot_png(widget: *mut c_void, path: *const c_char) -> c_int;
}
