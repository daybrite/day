//! day-qt-sys — raw `extern "C"` declarations for the Qt 6 shim compiled by build.rs.
//! Handles are opaque `QWidget*`; ownership stays with Qt's parent/child tree (Day's release
//! calls `day_qt_delete` = deleteLater, per §4.3's deferred-destruction allowance).

use std::os::raw::{c_char, c_double, c_int, c_void};

unsafe extern "C" {
    pub fn day_qt_app_new(app_name: *const c_char) -> *mut c_void;
    pub fn day_qt_app_run(app: *mut c_void);
    pub fn day_qt_window_new(title: *const c_char, w: c_int, h: c_int) -> *mut c_void;
    pub fn day_qt_window_show(win: *mut c_void);
    pub fn day_qt_window_on_resize(win: *mut c_void, cb: extern "C" fn(c_int, c_int));
    pub fn day_qt_container_new() -> *mut c_void;
    /// Apply a `background`/`corner_radius` surface via a scoped stylesheet (`#objName { ... }`
    /// so children don't inherit the fill) + `WA_StyledBackground`. `r,g,b` are 0..1, `a` is the
    /// alpha 0..1; `radius` in px; `clips != 0` requests rounded-child clipping (best-effort).
    pub fn day_qt_widget_set_surface(
        w: *mut c_void,
        r: c_double,
        g: c_double,
        b: c_double,
        a: c_double,
        radius: c_double,
        clips: c_int,
    );

    pub fn day_qt_label_new(text: *const c_char) -> *mut c_void;
    pub fn day_qt_label_set_text(w: *mut c_void, text: *const c_char);
    pub fn day_qt_label_set_font(w: *mut c_void, pt: c_double, weight: c_int, italic: c_int);
    /// Swap the label's font family to a bundled one (after `day_qt_label_set_font`).
    pub fn day_qt_label_set_font_family(w: *mut c_void, family: *const c_char);
    /// `QFontDatabase::addApplicationFont` — returns the font id (>= 0) or -1 on failure.
    /// Requires a constructed QApplication.
    pub fn day_qt_register_font(path: *const c_char) -> c_int;
    pub fn day_qt_label_height_for_width(w: *mut c_void, width: c_int) -> c_int;

    pub fn day_qt_button_new(title: *const c_char, id: u64, cb: extern "C" fn(u64)) -> *mut c_void;
    pub fn day_qt_button_set_title(w: *mut c_void, title: *const c_char);

    pub fn day_qt_checkbox_new(on: c_int, id: u64, cb: extern "C" fn(u64, c_int)) -> *mut c_void;
    pub fn day_qt_checkbox_set(w: *mut c_void, on: c_int);

    pub fn day_qt_slider_new(value: c_int, id: u64, cb: extern "C" fn(u64, c_int)) -> *mut c_void;
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

    pub fn day_qt_progress_new(determinate: c_int, value: c_int) -> *mut c_void;
    pub fn day_qt_progress_set(w: *mut c_void, value: c_int);

    pub fn day_qt_tabs_new(id: u64, cb: extern "C" fn(u64, c_int)) -> *mut c_void;
    pub fn day_qt_tabs_add_page(
        tabs: *mut c_void,
        page: *mut c_void,
        title: *const c_char,
        index: c_int,
    );
    pub fn day_qt_tabs_set_current(tabs: *mut c_void, index: c_int);
    pub fn day_qt_tabs_content_size(tabs: *mut c_void, w: *mut f64, h: *mut f64);

    pub fn day_qt_scroll_new() -> *mut c_void;
    pub fn day_qt_scroll_content(w: *mut c_void) -> *mut c_void;
    pub fn day_qt_scroll_set_content_size(w: *mut c_void, cw: c_int, ch: c_int);
    pub fn day_qt_scroll_to_bottom(w: *mut c_void);

    pub fn day_qt_add_child(parent: *mut c_void, child: *mut c_void);
    pub fn day_qt_remove_child(child: *mut c_void);
    pub fn day_qt_delete(w: *mut c_void);
    pub fn day_qt_set_geometry(w: *mut c_void, x: c_int, y: c_int, width: c_int, height: c_int);
    pub fn day_qt_size_hint(w: *mut c_void, out_w: *mut c_double, out_h: *mut c_double);
    pub fn day_qt_set_enabled(w: *mut c_void, enabled: c_int);
    pub fn day_qt_set_object_name(w: *mut c_void, name: *const c_char);
    pub fn day_qt_set_tooltip(w: *mut c_void, text: *const c_char);
    pub fn day_qt_set_accessible_name(w: *mut c_void, name: *const c_char);
    pub fn day_qt_set_accessible_description(w: *mut c_void, text: *const c_char);

    pub fn day_qt_canvas_new() -> *mut c_void;
    pub fn day_qt_canvas_set_ops(
        w: *mut c_void,
        nums: *const c_double,
        n: c_int,
        texts_joined: *const c_char,
    );
    pub fn day_qt_image_new(path: *const c_char, mode: c_int) -> *mut c_void;
    // App icon (§18.2): Dock icon on macOS, taskbar icon on Linux/Windows.
    pub fn day_qt_set_app_icon(path: *const c_char);
    // Native Qt Resource System (§18.3): register the .rcc blob; read data zero-copy.
    pub fn day_qt_register_resource(path: *const c_char);
    pub fn day_qt_resource_data(respath: *const c_char, out_len: *mut usize) -> *const c_void;
    pub fn day_qt_resource_exists(respath: *const c_char) -> c_int;
    pub fn day_qt_enable_gesture(
        w: *mut c_void,
        node: u64,
        is_drag: c_int,
        cb: extern "C" fn(u64, c_int, c_double, c_double, c_double, c_double),
    );
    pub fn day_qt_set_present_cb(cb: extern "C" fn(u64, c_int, i64, *const c_char));
    pub fn day_qt_present_dialog(
        req: u64,
        title: *const c_char,
        message: *const c_char,
        buttons_joined: *const c_char,
        roles_joined: *const c_char,
        parent: *mut c_void,
    );
    pub fn day_qt_present_prompt(
        req: u64,
        title: *const c_char,
        message: *const c_char,
        placeholder: *const c_char,
        initial: *const c_char,
        ok: *const c_char,
        cancel: *const c_char,
        parent: *mut c_void,
    );
    pub fn day_qt_present_file_open(
        req: u64,
        title: *const c_char,
        filters_joined: *const c_char,
        parent: *mut c_void,
    );
    pub fn day_qt_present_file_save(
        req: u64,
        title: *const c_char,
        suggested: *const c_char,
        filters_joined: *const c_char,
        parent: *mut c_void,
    );
    pub fn day_qt_dismiss_present(req: u64);
    pub fn day_qt_navlist_new(id: u64, cb: extern "C" fn(u64, c_int)) -> *mut c_void;
    pub fn day_qt_navlist_set_items(w: *mut c_void, joined: *const c_char);
    pub fn day_qt_navlist_set_selected(w: *mut c_void, idx: c_int);
    pub fn day_qt_splitter_new() -> *mut c_void;
    pub fn day_qt_splitter_pane(w: *mut c_void, index: c_int) -> *mut c_void;
    pub fn day_qt_splitter_on_moved(w: *mut c_void, cb: extern "C" fn(*mut c_void));
    pub fn day_qt_widget_size(w: *mut c_void, out_w: *mut c_double, out_h: *mut c_double);
    pub fn day_qt_set_visible(w: *mut c_void, visible: c_int);
    pub fn day_qt_post(cb: extern "C" fn(*mut c_void), data: *mut c_void);
    pub fn day_qt_snapshot_png(widget: *mut c_void, path: *const c_char) -> c_int;

    // Lifecycle (docs/lifecycle.md): phase codes match day_spec::Lifecycle order.
    pub fn day_qt_set_lifecycle_cb(cb: extern "C" fn(c_int));

    // Menus (docs/menus.md): a flat builder walked from the day-neutral MenuItem tree.
    pub fn day_qt_set_menu_cb(cb: extern "C" fn(u64));
    pub fn day_qt_window_menubar(win: *mut c_void) -> *mut c_void;
    pub fn day_qt_menubar_add_menu(bar: *mut c_void, label: *const c_char) -> *mut c_void;
    pub fn day_qt_menu_new() -> *mut c_void;
    pub fn day_qt_menu_add_submenu(menu: *mut c_void, label: *const c_char) -> *mut c_void;
    pub fn day_qt_menu_add_separator(menu: *mut c_void);
    pub fn day_qt_menu_add_action(
        menu: *mut c_void,
        label: *const c_char,
        id: u64,
        shortcut: *const c_char,
        enabled: c_int,
    );
    pub fn day_qt_menu_add_role(
        menu: *mut c_void,
        label: *const c_char,
        role: c_int,
        shortcut: *const c_char,
    );
    pub fn day_qt_set_context_menu(widget: *mut c_void, menu: *mut c_void);
}
