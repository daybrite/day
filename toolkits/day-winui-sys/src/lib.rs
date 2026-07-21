//! day-winui-sys — raw `extern "C"` declarations for the C++/WinRT XAML-Islands shim
//! (src/shim.cpp) compiled by build.rs. Handles are opaque `Windows.UI.Xaml.UIElement*`
//! heap-boxed by the shim; `day_winui_delete` releases the WinRT reference.

#![cfg(windows)]

use std::os::raw::{c_char, c_double, c_int, c_void};

unsafe extern "C" {
    // window / app lifecycle
    pub fn day_winui_window_new(
        title: *const c_char,
        w: c_int,
        h: c_int,
        min_w: c_int,
        min_h: c_int,
    ) -> *mut c_void;
    pub fn day_winui_window_root(win: *mut c_void) -> *mut c_void;
    pub fn day_winui_window_show(win: *mut c_void);
    /// Top-level host HWND of the (single, v1) app window — for a piece that needs the window handle
    /// behind the XAML island. The WebView2 web view passes it as the composition controller's
    /// parentWindow (DPI / IME / input association) while rendering windowless into the XAML tree.
    pub fn day_winui_host_hwnd() -> *mut c_void;
    /// Title-bar + taskbar icon from a multi-size `.ico` (§18.2).
    pub fn day_winui_set_app_icon(win: *mut c_void, ico_path: *const c_char);
    pub fn day_winui_window_on_resize(win: *mut c_void, cb: extern "C" fn(c_int, c_int));
    pub fn day_winui_run(win: *mut c_void);
    pub fn day_winui_post(cb: extern "C" fn(*mut c_void), data: *mut c_void);

    // containers
    pub fn day_winui_container_new() -> *mut c_void;
    pub fn day_winui_container_set_card(h: *mut c_void, radius: f64);
    // A ScrollViewer (docs §7.6): returns the host; `out_content` receives the inner content Canvas
    // (day adds children there and reports the content extent via set_content_size).
    pub fn day_winui_scroll_new(out_content: *mut *mut c_void, horizontal: c_int) -> *mut c_void;
    pub fn day_winui_scroll_set_content_size(content: *mut c_void, w: c_int, h: c_int);
    pub fn day_winui_scroll_offset(sv: *mut c_void, out_x: *mut c_double, out_y: *mut c_double);
    pub fn day_winui_scroll_to(sv: *mut c_void, y: c_int, h: c_int, animated: c_int);
    pub fn day_winui_container_set_bg(w: *mut c_void, argb: u32);
    /// Best-effort rounded clip for a `corner_radius` container: a rounded `RectangleGeometry`
    /// Clip whose Rect tracks the element size (SizeChanged). Corner support is limited on a bare
    /// Canvas, so this is best-effort (docs).
    pub fn day_winui_container_set_corner(w: *mut c_void, radius: c_double);
    pub fn day_winui_canvas_new() -> *mut c_void;
    /// Render a canvas display list (day_spec::encode_ops output) into the Canvas.
    pub fn day_winui_canvas_set_ops(
        w: *mut c_void,
        nums: *const c_double,
        n: c_int,
        texts_joined: *const c_char,
    );

    // recycling list host (docs/list.md): a ScrollViewer + content Canvas
    pub fn day_winui_list_new(out_content: *mut *mut c_void) -> *mut c_void;
    pub fn day_winui_list_set_content_size(content: *mut c_void, w: c_int, h: c_int);

    // navigation sidebar menu (docs/navigation.md): a single-select ListView
    pub fn day_winui_navlist_new(id: u64, cb: extern "C" fn(u64, c_int)) -> *mut c_void;
    pub fn day_winui_navlist_set_items(w: *mut c_void, items_joined: *const c_char);
    pub fn day_winui_navlist_set_selected(w: *mut c_void, idx: c_int);

    // native NavigationView split nav (docs/navigation.md): the idiomatic Windows sidebar+header,
    // as in Settings. `out_content` receives the detail-page Canvas. Callbacks: sel(id, index) on a
    // user menu pick; size(id, region, w, h) on a region reflow (region 0 = content, 1 = pane header);
    // back(id) on the back button.
    pub fn day_winui_nav_new(
        id: u64,
        sel_cb: extern "C" fn(u64, c_int),
        size_cb: extern "C" fn(u64, c_int, c_int, c_int),
        back_cb: extern "C" fn(u64),
        out_content: *mut *mut c_void,
        stack: c_int,
    ) -> *mut c_void;
    pub fn day_winui_nav_set_items(
        nav: *mut c_void,
        items_joined: *const c_char,
        icons_joined: *const c_char,
    );
    pub fn day_winui_nav_set_selected(nav: *mut c_void, idx: c_int);
    pub fn day_winui_nav_set_header(nav: *mut c_void, title: *const c_char);
    pub fn day_winui_nav_set_pane_header(nav: *mut c_void, element: *mut c_void);
    pub fn day_winui_nav_set_back_visible(nav: *mut c_void, visible: c_int);

    // leaves
    pub fn day_winui_label_new(text: *const c_char) -> *mut c_void;
    pub fn day_winui_label_set_text(w: *mut c_void, text: *const c_char);
    pub fn day_winui_label_set_font(w: *mut c_void, pt: c_double, weight: c_int, italic: c_int);
    /// Bundled custom font (§18.4): `spec` is a `FontFamily` source of the form
    /// "ms-appx:///fonts/<file>#<family>" (the font staged under `<exe>/fonts/`).
    pub fn day_winui_label_set_font_family(w: *mut c_void, spec: *const c_char);
    /// TextBlock.Foreground = SolidColorBrush(argb); alpha 0 restores the inherited default.
    pub fn day_winui_label_set_color(w: *mut c_void, argb: u32);

    pub fn day_winui_button_new(
        title: *const c_char,
        id: u64,
        cb: extern "C" fn(u64),
    ) -> *mut c_void;
    pub fn day_winui_button_prominent(h: *mut c_void);
    pub fn day_winui_button_set_title(w: *mut c_void, title: *const c_char);

    pub fn day_winui_toggle_new(on: c_int, id: u64, cb: extern "C" fn(u64, c_int)) -> *mut c_void;
    pub fn day_winui_toggle_set(w: *mut c_void, on: c_int);

    pub fn day_winui_slider_new(
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        id: u64,
        cb: extern "C" fn(u64, f64),
    ) -> *mut c_void;
    pub fn day_winui_slider_set(w: *mut c_void, value: f64);

    pub fn day_winui_progress_new(determinate: c_int, value: c_int) -> *mut c_void;
    pub fn day_winui_progress_set(w: *mut c_void, value: c_int);

    pub fn day_winui_tabs_new(id: u64, cb: extern "C" fn(u64, c_int)) -> *mut c_void;
    pub fn day_winui_tabs_add_page(
        tabs: *mut c_void,
        page: *mut c_void,
        title: *const c_char,
        index: c_int,
    );
    pub fn day_winui_tabs_set_current(tabs: *mut c_void, index: c_int);
    pub fn day_winui_tabs_content_size(tabs: *mut c_void, w: *mut f64, h: *mut f64);

    pub fn day_winui_textbox_new(
        text: *const c_char,
        placeholder: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, *const c_char),
    ) -> *mut c_void;
    pub fn day_winui_textbox_set_text(w: *mut c_void, text: *const c_char);
    pub fn day_winui_textbox_set_placeholder(w: *mut c_void, text: *const c_char);

    pub fn day_winui_divider_new() -> *mut c_void;
    pub fn day_winui_image_new(uri: *const c_char, mode: c_int) -> *mut c_void;

    // External-piece / tweaks handle seam (docs/tweaks.md): box a WinRT ABI pointer into a day
    // handle, and borrow the ABI pointer back out. `day_winui_unbox` returns winrt::get_abi —
    // a BORROWED IUIElement*, valid while the handle's Node holds its reference; callers that
    // retain must copy_from_abi (AddRef) on their own side.
    pub fn day_winui_box(iinspectable_abi: *mut c_void) -> *mut c_void;
    pub fn day_winui_unbox(handle: *mut c_void) -> *mut c_void;

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
    pub fn day_winui_set_visible(w: *mut c_void, visible: c_int);
    pub fn day_winui_widget_size(w: *mut c_void, out_w: *mut f64, out_h: *mut f64);
    pub fn day_winui_set_name(w: *mut c_void, name: *const c_char);

    // gestures (docs/shapes.md): attach a native recognizer. kind 0 Tap / 1 LongPress / 2 Drag;
    // cb(id, phase, x, y, tx, ty) with phase 0 Tap, 1/2/3 Drag Began/Changed/Ended, 4 LongPress.
    pub fn day_winui_enable_gesture(
        elem: *mut c_void,
        id: u64,
        kind: c_int,
        cb: extern "C" fn(u64, c_int, c_double, c_double, c_double, c_double),
    );

    // focus (docs/focus.md): observe via GotFocus/LostFocus (kind 1 gained / 0 lost / 2
    // submitted); drive via Focus(Programmatic), resigning to the window's focus sink.
    pub fn day_winui_enable_focus(elem: *mut c_void, id: u64, cb: extern "C" fn(u64, c_int));
    pub fn day_winui_control_focus(elem: *mut c_void, focused: c_int);

    /// Capture the window's client area to a PNG file. Returns 0 on success.
    pub fn day_winui_snapshot_png(win: *mut c_void, path: *const c_char) -> c_int;

    // lifecycle (docs/lifecycle.md): phase codes match day_spec::Lifecycle order.
    pub fn day_winui_set_lifecycle_cb(cb: extern "C" fn(c_int));

    /// Open a URL in the system's default handler (the `link` piece's seam).
    pub fn day_winui_open_url(url: *const c_char);

    // menus (docs/menus.md): a tab/newline spec parsed by the shim into MenuFlyout / MenuBar.
    pub fn day_winui_set_menu_cb(cb: extern "C" fn(u64));
    pub fn day_winui_set_context_menu(elem: *mut c_void, spec: *const c_char);
    pub fn day_winui_set_app_menu(win: *mut c_void, spec: *const c_char);

    // present / dismiss (docs/dialogs.md): ContentDialog (alert/prompt) + WinRT file pickers.
    // The cb delivers a result as (req, tag, index, text) — tag matches PresentResult::decode.
    pub fn day_winui_set_present_cb(cb: extern "C" fn(u64, c_int, i64, *const c_char));
    pub fn day_winui_present_dialog(
        req: u64,
        title: *const c_char,
        message: *const c_char,
        buttons_joined: *const c_char,
        roles_joined: *const c_char,
        win: *mut c_void,
    );
    pub fn day_winui_present_prompt(
        req: u64,
        title: *const c_char,
        message: *const c_char,
        placeholder: *const c_char,
        initial: *const c_char,
        ok: *const c_char,
        cancel: *const c_char,
        win: *mut c_void,
    );
    pub fn day_winui_present_file_open(
        req: u64,
        title: *const c_char,
        filters_joined: *const c_char,
        win: *mut c_void,
    );
    pub fn day_winui_present_file_save(
        req: u64,
        title: *const c_char,
        suggested: *const c_char,
        filters_joined: *const c_char,
        win: *mut c_void,
    );
    pub fn day_winui_dismiss_present(req: u64);
}
