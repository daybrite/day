//! day-xaml-sys — raw `extern "C"` declarations for the C++/WinRT XAML-Islands shim
//! (src/shim.cpp) compiled by build.rs. Handles are opaque `Windows.UI.Xaml.UIElement*`
//! heap-boxed by the shim; `day_xaml_delete` releases the WinRT reference.

#![cfg(windows)]

use std::os::raw::{c_char, c_double, c_int, c_void};

unsafe extern "C" {
    // window / app lifecycle
    pub fn day_xaml_window_new(
        title: *const c_char,
        w: c_int,
        h: c_int,
        min_w: c_int,
        min_h: c_int,
    ) -> *mut c_void;
    pub fn day_xaml_window_root(win: *mut c_void) -> *mut c_void;
    pub fn day_xaml_window_show(win: *mut c_void);
    // Secondary windows (docs/windows.md): per-window event callbacks keyed by day node id.
    pub fn day_xaml_set_window_events_cb(
        resized: extern "C" fn(u64, c_int, c_int),
        closed: extern "C" fn(u64),
        focused: extern "C" fn(u64, c_int),
    );
    pub fn day_xaml_window_new2(
        title: *const c_char,
        w: c_int,
        h: c_int,
        node: u64,
        fixed: c_int,
    ) -> *mut c_void;
    pub fn day_xaml_window_content2(win: *mut c_void) -> *mut c_void;
    pub fn day_xaml_window_close2(win: *mut c_void);
    pub fn day_xaml_window_raise2(win: *mut c_void);
    pub fn day_xaml_window_set_title2(win: *mut c_void, title: *const c_char);
    pub fn day_xaml_window_destroy2(win: *mut c_void);
    /// Top-level host HWND of the (single, v1) app window — for a piece that needs the window handle
    /// behind the XAML island. The WebView2 web view passes it as the composition controller's
    /// parentWindow (DPI / IME / input association) while rendering windowless into the XAML tree.
    pub fn day_xaml_host_hwnd() -> *mut c_void;
    /// Title-bar + taskbar icon from a multi-size `.ico` (§18.2).
    pub fn day_xaml_set_app_icon(win: *mut c_void, ico_path: *const c_char);
    pub fn day_xaml_window_on_resize(win: *mut c_void, cb: extern "C" fn(c_int, c_int));
    pub fn day_xaml_run(win: *mut c_void);
    /// End the app — day-core's close policy decided the last primary window is gone.
    pub fn day_xaml_quit();
    pub fn day_xaml_post(cb: extern "C" fn(*mut c_void), data: *mut c_void);

    // containers
    pub fn day_xaml_container_new() -> *mut c_void;
    pub fn day_xaml_container_set_card(h: *mut c_void, radius: f64);
    // A ScrollViewer (docs §7.6): returns the host; `out_content` receives the inner content Canvas
    // (day adds children there and reports the content extent via set_content_size).
    pub fn day_xaml_scroll_new(out_content: *mut *mut c_void, horizontal: c_int) -> *mut c_void;
    pub fn day_xaml_scroll_set_content_size(content: *mut c_void, w: c_int, h: c_int);
    pub fn day_xaml_scroll_offset(sv: *mut c_void, out_x: *mut c_double, out_y: *mut c_double);
    pub fn day_xaml_scroll_to(sv: *mut c_void, y: c_int, h: c_int, animated: c_int);
    pub fn day_xaml_container_set_bg(w: *mut c_void, argb: u32);
    // Backend-executed animation (DESIGN.md §8.4): day passes a target value + intent and XAML's
    // own compositor runs it. `dur_ms <= 0` sets the value outright. `curve`: 0 linear, 1 ease-in,
    // 2 ease-out, 3 ease-in-out, 4 spring — the same encoding day-qt's shim takes.
    pub fn day_xaml_set_opacity(w: *mut c_void, opacity: c_double, dur_ms: c_int, curve: c_int);
    #[allow(clippy::too_many_arguments)]
    pub fn day_xaml_set_transform(
        w: *mut c_void,
        tx: c_double,
        ty: c_double,
        sx: c_double,
        sy: c_double,
        rotate_deg: c_double,
        dur_ms: c_int,
        curve: c_int,
    );
    // Animated fill. XAML is the only desktop backend that can tween a background colour (§8.4).
    pub fn day_xaml_container_animate_bg(w: *mut c_void, argb: u32, dur_ms: c_int, curve: c_int);
    pub fn day_xaml_cover_ground(w: *mut c_void);
    /// Best-effort rounded clip for a `corner_radius` container: a rounded `RectangleGeometry`
    /// Clip whose Rect tracks the element size (SizeChanged). Corner support is limited on a bare
    /// Canvas, so this is best-effort (docs).
    pub fn day_xaml_container_set_corner(w: *mut c_void, radius: c_double);
    pub fn day_xaml_canvas_new() -> *mut c_void;
    /// Render a canvas display list (day_spec::encode_ops output) into the Canvas.
    pub fn day_xaml_canvas_set_ops(
        w: *mut c_void,
        nums: *const c_double,
        n: c_int,
        texts_joined: *const c_char,
    );

    // recycling list host (docs/list.md): a ScrollViewer + content Canvas
    pub fn day_xaml_list_new(out_content: *mut *mut c_void) -> *mut c_void;
    pub fn day_xaml_list_set_content_size(content: *mut c_void, w: c_int, h: c_int);
    // Emulated list drag-to-reorder (docs/list.md): the content Canvas accepts day-row drops —
    // every hovered slot is vetted synchronously through `can` (accepted index or -1; the system
    // shows the no-drop cursor on -1), and the drop commits via `mv`.
    pub fn day_xaml_list_enable_reorder(
        content: *mut c_void,
        id: u64,
        row_h: c_int,
        can: extern "C" fn(u64, c_int, c_int) -> c_int,
        mv: extern "C" fn(u64, c_int, c_int),
    );
    // Arm the WinRT drag start on one cell (cell index == row for its whole life).
    pub fn day_xaml_cell_drag(cell: *mut c_void, id: u64, row: c_int);
    // Emulated list row selection (docs/list.md): report a press on one cell as (node, row, mods)
    // — mods bit 0 = ctrl (toggle), bit 1 = shift (range) — leaving the semantics to Rust. Also
    // makes the cell hit-testable across its whole band.
    pub fn day_xaml_list_cell_click(
        cell: *mut c_void,
        id: u64,
        row: c_int,
        cb: extern "C" fn(u64, c_int, c_int),
    );
    // Paint one cell's selected treatment (0 clears it).
    pub fn day_xaml_cell_set_selected(cell: *mut c_void, on: c_int);

    // navigation sidebar menu (docs/navigation.md): a single-select ListView
    pub fn day_xaml_navlist_new(id: u64, cb: extern "C" fn(u64, c_int)) -> *mut c_void;
    pub fn day_xaml_navlist_set_items(w: *mut c_void, items_joined: *const c_char);
    pub fn day_xaml_navlist_set_selected(w: *mut c_void, idx: c_int);

    // native NavigationView split nav (docs/navigation.md): the idiomatic Windows sidebar+header,
    // as in Settings. `out_content` receives the detail-page Canvas. Callbacks: sel(id, index) on a
    // user menu pick; size(id, region, w, h) on a region reflow (region 0 = content, 1 = pane header);
    // back(id) on the back button.
    pub fn day_xaml_nav_new(
        id: u64,
        sel_cb: extern "C" fn(u64, c_int),
        size_cb: extern "C" fn(u64, c_int, c_int, c_int),
        back_cb: extern "C" fn(u64),
        out_content: *mut *mut c_void,
        stack: c_int,
    ) -> *mut c_void;
    pub fn day_xaml_nav_set_items(
        nav: *mut c_void,
        items_joined: *const c_char,
        icons_joined: *const c_char,
        geoms_joined: *const c_char,
        tints_joined: *const c_char,
    );
    pub fn day_xaml_nav_set_selected(nav: *mut c_void, idx: c_int);
    pub fn day_xaml_nav_set_header(nav: *mut c_void, title: *const c_char);
    pub fn day_xaml_nav_set_pane_header(nav: *mut c_void, element: *mut c_void);
    pub fn day_xaml_nav_set_back_visible(nav: *mut c_void, visible: c_int);

    // leaves
    pub fn day_xaml_label_new(text: *const c_char) -> *mut c_void;
    pub fn day_xaml_label_set_text(w: *mut c_void, text: *const c_char);
    pub fn day_xaml_label_set_font(
        w: *mut c_void,
        pt: c_double,
        weight: c_int,
        italic: c_int,
        tabular: c_int,
    );
    /// Bundled custom font (§18.4): `spec` is a `FontFamily` source of the form
    /// "ms-appx:///fonts/<file>#<family>" (the font staged under `<exe>/fonts/`).
    pub fn day_xaml_label_set_font_family(w: *mut c_void, spec: *const c_char);
    /// Make a label's text user-selectable (the `.selectable()` modifier). No-op on non-labels.
    pub fn day_xaml_label_set_selectable(w: *mut c_void, on: c_int);
    /// TextBlock.Foreground = SolidColorBrush(argb); alpha 0 restores the inherited default.
    pub fn day_xaml_label_set_color(w: *mut c_void, argb: u32);

    pub fn day_xaml_button_new(
        title: *const c_char,
        id: u64,
        cb: extern "C" fn(u64),
    ) -> *mut c_void;
    pub fn day_xaml_button_prominent(h: *mut c_void);
    pub fn day_xaml_button_set_title(w: *mut c_void, title: *const c_char);

    pub fn day_xaml_toggle_new(on: c_int, id: u64, cb: extern "C" fn(u64, c_int)) -> *mut c_void;
    pub fn day_xaml_toggle_set(w: *mut c_void, on: c_int);

    pub fn day_xaml_slider_new(
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        id: u64,
        cb: extern "C" fn(u64, f64, c_int),
    ) -> *mut c_void;
    pub fn day_xaml_slider_set(w: *mut c_void, value: f64);

    pub fn day_xaml_progress_new(determinate: c_int, value: c_int) -> *mut c_void;
    pub fn day_xaml_progress_set(w: *mut c_void, value: c_int);

    pub fn day_xaml_tabs_new(id: u64, cb: extern "C" fn(u64, c_int)) -> *mut c_void;
    pub fn day_xaml_tabs_add_page(
        tabs: *mut c_void,
        page: *mut c_void,
        title: *const c_char,
        index: c_int,
    );
    pub fn day_xaml_tabs_set_current(tabs: *mut c_void, index: c_int);
    pub fn day_xaml_tabs_content_size(tabs: *mut c_void, w: *mut f64, h: *mut f64);

    pub fn day_xaml_textbox_new(
        text: *const c_char,
        placeholder: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, *const c_char),
    ) -> *mut c_void;
    pub fn day_xaml_textbox_set_text(w: *mut c_void, text: *const c_char);
    pub fn day_xaml_textbox_set_placeholder(w: *mut c_void, text: *const c_char);

    pub fn day_xaml_divider_new() -> *mut c_void;
    pub fn day_xaml_image_new(uri: *const c_char, mode: c_int) -> *mut c_void;
    /// A vector glyph as real XAML `Path` geometry inside a scaling `Viewbox` (docs/vectors.md):
    /// resolution-independent, and `tinted` composes `argb` over the shapes as a brush. Null
    /// when the spec carried no drawable geometry, so the caller falls back to the raster.
    pub fn day_xaml_vector_new(
        spec: *const c_char,
        mode: c_int,
        argb: u32,
        tinted: c_int,
    ) -> *mut c_void;
    /// A tinted vector glyph as a monochrome `BitmapIcon` — the raster fallback for art that
    /// could not be converted to geometry; null when unresolved or the tint is transparent.
    pub fn day_xaml_image_tinted_new(
        icon_file: *const c_char,
        mode: c_int,
        argb: u32,
    ) -> *mut c_void;

    // External-piece / tweaks handle seam (docs/tweaks.md): box a WinRT ABI pointer into a day
    // handle, and borrow the ABI pointer back out. `day_xaml_unbox` returns winrt::get_abi —
    // a BORROWED IUIElement*, valid while the handle's Node holds its reference; callers that
    // retain must copy_from_abi (AddRef) on their own side.
    pub fn day_xaml_box(iinspectable_abi: *mut c_void) -> *mut c_void;
    pub fn day_xaml_unbox(handle: *mut c_void) -> *mut c_void;

    // tree / geometry / props
    pub fn day_xaml_add_child(parent: *mut c_void, child: *mut c_void);
    pub fn day_xaml_remove_child(parent: *mut c_void, child: *mut c_void);
    pub fn day_xaml_delete(w: *mut c_void);
    pub fn day_xaml_set_geometry(w: *mut c_void, x: c_int, y: c_int, width: c_int, height: c_int);
    pub fn day_xaml_measure(
        w: *mut c_void,
        avail_w: c_double,
        avail_h: c_double,
        out_w: *mut c_double,
        out_h: *mut c_double,
    );
    pub fn day_xaml_set_enabled(w: *mut c_void, enabled: c_int);
    pub fn day_xaml_set_visible(w: *mut c_void, visible: c_int);
    pub fn day_xaml_widget_size(w: *mut c_void, out_w: *mut f64, out_h: *mut f64);
    pub fn day_xaml_set_name(w: *mut c_void, name: *const c_char);

    // gestures (docs/shapes.md): attach a native recognizer. kind 0 Tap / 1 LongPress / 2 Drag;
    // cb(id, phase, x, y, tx, ty) with phase 0 Tap, 1/2/3 Drag Began/Changed/Ended, 4 LongPress.
    pub fn day_xaml_enable_gesture(
        elem: *mut c_void,
        id: u64,
        kind: c_int,
        cb: extern "C" fn(u64, c_int, c_double, c_double, c_double, c_double),
    );

    // focus (docs/focus.md): observe via GotFocus/LostFocus (kind 1 gained / 0 lost / 2
    // submitted); drive via Focus(Programmatic), resigning to the window's focus sink.
    pub fn day_xaml_enable_focus(elem: *mut c_void, id: u64, cb: extern "C" fn(u64, c_int));
    pub fn day_xaml_control_focus(elem: *mut c_void, focused: c_int);

    /// Capture the window's client area to a PNG file. Returns 0 on success.
    pub fn day_xaml_snapshot_png(win: *mut c_void, path: *const c_char) -> c_int;

    // lifecycle (docs/lifecycle.md): phase codes match day_spec::Lifecycle order.
    pub fn day_xaml_set_lifecycle_cb(cb: extern "C" fn(c_int));

    /// Open a URL in the system's default handler (the `link` piece's seam).
    pub fn day_xaml_open_url(url: *const c_char);

    // menus (docs/menus.md): a tab/newline spec parsed by the shim into MenuFlyout / MenuBar.
    pub fn day_xaml_set_menu_cb(cb: extern "C" fn(u64));
    pub fn day_xaml_set_context_menu(elem: *mut c_void, spec: *const c_char);
    pub fn day_xaml_set_app_menu(win: *mut c_void, spec: *const c_char);

    // Window toolbar (docs/toolbars.md): a CommandBar under the menu bar, built from one
    // tab/newline spec the same way the menus are (the format is documented on both sides —
    // `serialize_toolbar` in day-xaml, the parser in shim.cpp). Buttons ride
    // `day_xaml_set_menu_cb`; values arrive on the toolbar callback as (action, kind, on, text)
    // with kind 0 = toggle, 1 = search text. An empty spec removes the bar.
    pub fn day_xaml_set_toolbar_cb(cb: extern "C" fn(u64, c_int, c_int, *const c_char));
    pub fn day_xaml_set_toolbar(win: *mut c_void, spec: *const c_char);
    // Targeted patches, addressed by the item's id (no-op if the bar has no such item).
    // Targeted item patches. `win` is the window whose toolbar owns the item: every window
    // installs the same item ids, so a patch has to name the window as well as the id.
    pub fn day_xaml_toolbar_set_text(win: *mut c_void, id: *const c_char, text: *const c_char);
    pub fn day_xaml_toolbar_set_checked(win: *mut c_void, id: *const c_char, on: c_int);
    pub fn day_xaml_toolbar_set_enabled(win: *mut c_void, id: *const c_char, on: c_int);
    /// The app menu and a toolbar docked in a SECONDARY window (docs/windows.md): day's app menu
    /// has no window parameter, so the same spec is installed into each window that opens.
    pub fn day_xaml_window_set_menu2(win: *mut c_void, spec: *const c_char);
    pub fn day_xaml_window_set_toolbar2(win: *mut c_void, spec: *const c_char);
    // Show/hide the split NavigationView's pane — the `SidebarToggle` item's behaviour, also
    // reachable from dayscript through the toolkit duty. 0 = no split nav in this window.
    pub fn day_xaml_toggle_sidebar() -> c_int;

    // present / dismiss (docs/dialogs.md): ContentDialog (alert/prompt) + WinRT file pickers.
    // The cb delivers a result as (req, tag, index, text) — tag matches PresentResult::decode.
    pub fn day_xaml_set_present_cb(cb: extern "C" fn(u64, c_int, i64, *const c_char));
    pub fn day_xaml_present_dialog(
        req: u64,
        title: *const c_char,
        message: *const c_char,
        buttons_joined: *const c_char,
        roles_joined: *const c_char,
        win: *mut c_void,
    );
    pub fn day_xaml_present_prompt(
        req: u64,
        title: *const c_char,
        message: *const c_char,
        placeholder: *const c_char,
        initial: *const c_char,
        ok: *const c_char,
        cancel: *const c_char,
        win: *mut c_void,
    );
    pub fn day_xaml_present_file_open(
        req: u64,
        title: *const c_char,
        filters_joined: *const c_char,
        win: *mut c_void,
    );
    pub fn day_xaml_present_file_save(
        req: u64,
        title: *const c_char,
        suggested: *const c_char,
        filters_joined: *const c_char,
        win: *mut c_void,
    );
    pub fn day_xaml_dismiss_present(req: u64);
}
