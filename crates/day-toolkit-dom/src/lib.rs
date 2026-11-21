// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-toolkit-dom — the `web-dom` backend (DESIGN.md §9): the DOM is the toolkit. A `<button>` is
//! the web's native button, `<input type="range">` its slider, `<dialog>` its modal surface —
//! semantic HTML plus ARIA, never a canvas-painted imitation (§0.3).
//!
//! Architecture: the same trampoline shape as day-toolkit-arkui, with JavaScript in place of C. A
//! small shim owns every real DOM call, keyed by numeric element ids; Rust passes ids and
//! UTF-8 (ptr, len) pairs across plain `extern "C"` imports, and the shim calls back through
//! a handful of exports (`day_dom_event`, `day_dom_posted`, …). No wasm-bindgen, no bundler:
//! the host page is a plain ES module that instantiates the wasm.
//!
//! **The shim lives in `crates/day-cli/resources/web/`** (`shim.js`, `index.html`, `day.css`),
//! not here: `day-cli` embeds the trio with `include_str!` so an installed CLI serves a
//! self-contained `dist/`, and `include_str!` may not reach outside its own package. Every
//! `extern "C"` import below has its implementation there — change one, change both, and
//! rebuild the CLI for the edit to reach a served page.
//!
//! Layout stays day-core-owned: `set_frame` writes absolute positions, exactly like the Qt
//! and ArkUI backends. The exceptions are nav/tab pages, whose panes are CSS-managed and
//! report their size back through a `ResizeObserver` (`Event::FrameChanged`) — the `DayNavPage`
//! contract (docs/navigation.md).
#![cfg(target_arch = "wasm32")]

mod controls;

mod transfer;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, VecDeque};

use day_spec::props::*;
use day_spec::{
    A11yProps, AnimSpec, Builtin, Cap, Cursor, Curve, DrawOp, Event, EventSink, Font, FontSpec,
    FontWeight, GestureKind, Lifecycle, ListSource, MenuItem, NodeId, Paint, PieceKind, Platform,
    Point, Proposal, Rect, Registry, Renderer, Shape, Size, Support, Toolkit, Transform,
    WindowOptions, kinds,
    present::{PresentButton, PresentResult, PresentSpec},
};

// ---------------------------------------------------------------------------
// Shim imports: every real DOM call. `el` values are shim-side element ids.
// The attribute makes these wasm imports from the instantiation's `env` object
// (shim.js supplies them) rather than link-time-resolved symbols.
// ---------------------------------------------------------------------------

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn day_dom_create(kind: u32) -> u32;
    /// Create an element by TAG NAME — the escape hatch piece renderers build on, since day-toolkit-dom's
    /// own `EL_*` kind codes only cover the built-in vocabulary.
    fn day_dom_create_tag(tag: *const u8, tag_len: usize) -> u32;
    /// Invoke a zero-argument method on an element (`play`, `pause`, `load`, …).
    fn day_dom_call(el: u32, method: *const u8, method_len: usize);
    /// Replace an element's markup, keeping the caret (docs/texteditor.md). The HTML comes from
    /// Day's own serializer, which escapes every character of app text.
    fn day_dom_set_html(el: u32, html: *const u8, html_len: usize);
    /// Put the selection at a BYTE range in a contenteditable element's flattened text.
    fn day_dom_editor_select(el: u32, start: u32, end: u32);
    /// Insert text at a BYTE offset of a contenteditable element's flattened text through the
    /// browser's editing command, so undo records it and the `input` event reports it.
    fn day_dom_editor_insert(el: u32, at: u32, text: *const u8, text_len: usize);
    fn day_dom_set_app_badge(count: i32);
    /// Enter (`on` = 1) or leave (0) fullscreen through the Fullscreen API on the document
    /// element (docs/windows.md "Window properties"). Returns -1 when a request is in flight,
    /// whose outcome comes back through `day_dom_window_state`; otherwise nothing was asked of
    /// the browser (no Fullscreen API, or already in that state) and the answer is the state the
    /// page is in, as a `WindowState` code.
    fn day_dom_set_fullscreen(on: u32) -> i32;
    /// The primary window's chrome, once at boot (docs/window-chrome.md): `overlay` 1 runs the
    /// content under an installed app's window-controls overlay, answers its height now (0 while
    /// hidden) and reports every later change through `day_dom_titlebar_inset`; 0 keeps the
    /// content below it. Also installs the drag-region rules `set_drag_region`'s class relies on.
    fn day_dom_window_chrome(overlay: u32) -> f64;
    /// The browser window's outer frame (`screenX`, `screenY`, `outerWidth`, `outerHeight`) into
    /// `out[0..4]`, in CSS px on the desktop. Returns 0 when the browser reports no frame.
    fn day_dom_window_frame(out: *mut f64) -> u32;
    /// The screen the page is on into `out[0..9]`: its frame (`screen.left`/`top`, else 0, then
    /// `width`, `height`), its work area (`availLeft`/`availTop`, else the frame's origin, then
    /// `availWidth`, `availHeight`), and `devicePixelRatio`.
    fn day_dom_screen(out: *mut f64);
    /// Speak `text` through the shim's ARIA live regions (docs/accessibility.md); `urgent`
    /// picks the assertive one, which interrupts, over the polite one, which queues.
    fn day_dom_announce(ptr: *const u8, len: usize, urgent: u32);
    fn day_dom_insert(parent: u32, child: u32, index: u32);
    fn day_dom_remove(child: u32);
    fn day_dom_release(el: u32);
    fn day_dom_set_frame(el: u32, x: f64, y: f64, w: f64, h: f64);
    fn day_dom_set_text(el: u32, ptr: *const u8, len: usize);
    fn day_dom_set_style(el: u32, p: *const u8, pl: usize, v: *const u8, vl: usize);
    fn day_dom_set_attr(el: u32, a: *const u8, al: usize, v: *const u8, vl: usize);
    fn day_dom_set_class(el: u32, ptr: *const u8, len: usize, on: u32);
    /// Wire a run anchor's click to `owner`'s node (docs/text-runs.md): the spans are not nodes,
    /// so the LABEL element is what the event reports against.
    fn day_dom_link(el: u32, owner: u32, p: *const u8, l: usize);
    fn day_dom_set_value(el: u32, v: f64);
    fn day_dom_set_checked(el: u32, on: u32);
    /// Select characters (Unicode scalar values) `start..end` of a text field's `<input>`,
    /// held to its value, caret at `end` (docs/textfield.md "Selection").
    fn day_dom_field_select(el: u32, start: u32, end: u32);
    /// Text entry traits on a text field's `<input>` (docs/textfield.md): `json` is one object
    /// of attribute name to value, where an empty value removes the attribute and `type` is
    /// assigned as the property, keeping the caret. Any other element is left alone.
    fn day_dom_input_traits(el: u32, json: *const u8, len: usize);
    /// Attach shim listeners; `mask` bits: 1 click, 2 input, 4 change, 8 focus, 16 submit,
    /// 32 resize-observer, 64 scroll, 128 pointer-tap, 256 pointer-drag, 512 contenteditable.
    fn day_dom_listen(el: u32, mask: u32);
    /// Set an animatable property (0 opacity, 1 transform, 2 background, 3 frame) to the
    /// numbers at `to`, animated over one cycle of `samples` (`[offset, progress]` pairs) when
    /// `duration_ms` is positive; `iterations` -1 for ever (docs/tween.md).
    fn day_dom_animate(
        el: u32,
        prop: u32,
        to: *const f64,
        to_len: usize,
        samples: *const f64,
        samples_len: usize,
        duration_ms: f64,
        delay_ms: f64,
        iterations: f64,
        alternate: u32,
    );
    /// What `el` displays now: `[opacity, x, y, w, h, tx, ty, r, g, b, a]` into `out`; returns 1
    /// when it shows a background color of its own.
    fn day_dom_displayed(el: u32, out: *mut f64) -> u32;
    /// A scroll view's preferences: `flags` bit 0 hidden indicators, bit 1 no bounce, bit 2
    /// phase reports; `snap` 0 none, 1 page, 2 every `interval` points.
    fn day_dom_scroll_setup(el: u32, flags: u32, snap: u32, interval: f64);
    /// `navigator.maxTouchPoints`.
    fn day_dom_touch_points() -> u32;
    /// Measure a label element in its own computed style (font, letter spacing, line height,
    /// line clamp, wrapping): its own content when `text_len` is 0, else `text` in its place.
    /// `max_w` caps the width (a huge value for none); `out` receives `[width, height]`.
    fn day_dom_measure_label(el: u32, text: *const u8, text_len: usize, max_w: f64, out: *mut f64);
    /// The browser's default font size against the one Day's ramp is drawn for (docs/text.md
    /// "Text size").
    fn day_dom_text_scale() -> f64;
    /// Grow `el`'s pointer hit area by the four insets, in PHYSICAL sides (top, left, bottom,
    /// right); zeros restore its bounds (docs/gestures.md "Hit slop").
    fn day_dom_hit_slop(el: u32, top: f64, left: f64, bottom: f64, right: f64);
    /// The axis `el`'s drag follows: 0 any, 1 horizontal, 2 vertical.
    fn day_dom_drag_axis(el: u32, axis: u32);
    /// One disabled group around `el` more (`on` 0) or fewer (`on` 1).
    fn day_dom_subtree_enabled(el: u32, on: u32);
    /// The kind of pointer last used: 0 touch, 1 mouse, 2 stylus.
    fn day_dom_pointer_kind() -> u32;
    /// Make a pass-through element a pointer target (docs/gestures.md "Pass-through").
    fn day_dom_takes_pointer(el: u32);
    fn day_dom_measure_text(
        t: *const u8,
        tl: usize,
        f: *const u8,
        fl: usize,
        max_w: f64,
        out: *mut f64,
    );
    fn day_dom_width(el: u32) -> f64;
    /// First text baseline from the element's top for a box `box_h` tall; `-1` ⇒ no text
    /// (docs/baseline.md).
    fn day_dom_baseline(el: u32, box_h: f64) -> f64;
    fn day_dom_scroll_to(el: u32, x: f64, y: f64, animated: u32);
    fn day_dom_scroll_edge(el: u32, edge: u32, animated: u32);
    /// Arm the shim's pointer-drag reorder on a list host (docs/list.md): the drag calls back
    /// into `day_dom_list_can_move`/`day_dom_list_move` synchronously.
    fn day_dom_list_reorder(el: u32);
    fn day_dom_scroll_offset(el: u32, out: *mut f64);
    /// The emulated list's scrolled offset and visible height — which rows have to exist.
    fn day_dom_list_viewport(el: u32, out: *mut f64);
    /// 1 while the list's own smooth scroll is still moving toward where Day sent it; one that
    /// has not moved for half a second no longer counts (`Toolkit::ui_idle`).
    fn day_dom_list_gliding(el: u32) -> u32;
    /// Report this list's scrolling into `day_dom_list_scrolled`, so rows coming into view build,
    /// and the scrolls the reader starts into `day_dom_list_reader`.
    fn day_dom_list_on_scroll(el: u32);
    /// Give the emulated list a tab stop, the listbox role, and the arrow/Home/End route back
    /// into `day_dom_list_key` (docs/list.md). A browser has no native list to inherit keyboard
    /// selection from, so the one the app sees is the one this builds.
    fn day_dom_list_keynav(el: u32, multi: u32);
    /// Give a canvas a tab stop, focus-on-press, and the arrow route back into
    /// `day_dom_canvas_key` (docs/menus.md) — the web half of "keys follow focus".
    fn day_dom_canvas_keynav(el: u32);
    fn day_dom_scroll_content(el: u32, w: f64, h: f64);
    fn day_dom_focus(el: u32, focused: u32);
    fn day_dom_canvas_replay(
        el: u32,
        ops: *const f64,
        ops_len: usize,
        strs: *const u8,
        strs_len: usize,
        w: f64,
        h: f64,
    );
    // Raster images from bytes (docs/images.md). A browser decodes AND encodes asynchronously —
    // the only backend here where that is true — so neither call answers; the shim calls the
    // exported `day_dom_image_decoded` / `day_dom_image_encoded` when it resolves. Ids ride as
    // `f64`: a wasm `i64` reaches JS as a `BigInt`, and an f64 carries a minted id exactly well
    // past any count a page can reach.
    fn day_dom_image_decode(req: u32, id: f64, ptr: *const u8, len: usize);
    fn day_dom_image_encode(
        req: u32,
        id: f64,
        fmt: *const u8,
        fmt_len: usize,
        quality: f64,
        fit_w: f64,
        fit_h: f64,
    );
    fn day_dom_image_release(id: f64);
    /// Paint `ops` into a detached `pw` × `ph` pixel canvas scaled by `scale`, and keep the
    /// result under `id` beside the decoded bitmaps (docs/canvas.md "Exporting"). The shim
    /// answers through the exported `day_dom_bitmap_rendered` once `createImageBitmap` resolves.
    fn day_dom_canvas_render_bitmap(
        req: u32,
        id: f64,
        ops: *const f64,
        ops_len: usize,
        strs: *const u8,
        strs_len: usize,
        pw: f64,
        ph: f64,
        scale: f64,
    );
    /// Whether the engine writes this MIME type — `toBlob` would otherwise substitute PNG.
    fn day_dom_image_can_encode(fmt: *const u8, fmt_len: usize) -> u32;
    /// Point a realized `<img>` at a decoded bitmap's object URL.
    fn day_dom_image_set_bitmap(el: u32, id: f64);
    /// Point a realized `<img>` at raw encoded bytes, through a blob URL the shim owns.
    fn day_dom_image_set_bytes(el: u32, ptr: *const u8, len: usize);
    /// Revoke the blob URL an element made, when the element goes.
    fn day_dom_image_revoke(el: u32);
    /// Turn an image element's mask on or off for a source whose URL the shim owns (bytes, a
    /// decoded bitmap): on, the mask is cut from that URL now and again at each source swap.
    fn day_dom_image_mask(el: u32, on: u32);
    fn day_dom_present(req: u32, json: *const u8, len: usize);
    /// The modifier keys held right now, as the shim last observed them (bit0 shift,
    /// bit1 primary = meta|ctrl, bit2 alt).
    fn day_dom_modifiers() -> u32;
    fn day_dom_prepare_snapshot(host: u32, revision: u32) -> u32;
    /// Present a save flow: `json` names it, `bytes` are the staged content the shim turns
    /// into a download (docs/files.md).
    fn day_dom_present_save(req: u32, json: *const u8, len: usize, bytes: *const u8, blen: usize);
    fn day_dom_dismiss(req: u32);
    /// `mode` is [`nav_mode`]: 0 split, 1 stack, 2 tabs, 3 rail.
    fn day_dom_nav_mode(el: u32, mode: u32, title: *const u8, tl: usize);
    /// Rebuild a live host's chrome for another presentation, detaching its pages (which stay
    /// alive in the shim's registry) for Day to re-home.
    fn day_dom_nav_present(el: u32, mode: u32);
    /// `chrome` puts the page in the sidebar / tab-bar / rail slot rather than the detail area.
    fn day_dom_nav_add_page(nav: u32, page: u32, chrome: u32);
    fn day_dom_nav_back_bar(nav: u32, visible: u32, t: *const u8, tl: usize);
    fn day_dom_navmenu(el: u32, json: *const u8, len: usize);
    fn day_dom_navmenu_rows(el: u32, json: *const u8, len: usize);
    // Window toolbar (docs/toolbars.md): each edit crosses as one JSON list of ops, and targeted
    // patches address an item by id. `day_dom_toolbar_sidebar` returns 0 when the page has no
    // split nav to toggle.
    // `host` is the window root the edit belongs to: a cover element gets a strip of its own
    // over the cover (docs/cover.md), anything else addresses the page's strip.
    fn day_dom_toolbar_edit(host: u32, json: *const u8, len: usize);
    fn day_dom_toolbar_patch(host: u32, json: *const u8, len: usize);
    fn day_dom_toolbar_height(host: u32) -> f64;
    fn day_dom_toolbar_sidebar() -> u32;
    // The emulated app menu (docs/menus.md): the model crosses as one JSON document and the
    // shim docks a trailing menu button on the toolbar strip; an empty item list removes it.
    fn day_dom_app_menu(json: *const u8, len: usize);
    /// Open (or, pressed again, close) a menu button's menu under the button `el` (docs/menus.md
    /// "Menu buttons"): `json` is the item list a toolbar pull-down takes, shown by the same
    /// popover.
    fn day_dom_button_menu(el: u32, json: *const u8, len: usize);
    fn day_dom_navmenu_select(el: u32, idx: i32);
    fn day_dom_options(el: u32, json: *const u8, len: usize);
    fn day_dom_options_select(el: u32, idx: u32);
    fn day_dom_schedule_post();
    fn day_dom_schedule_delayed(token: u32, ms: u32);
    fn day_dom_request_frame(token: f64) -> u32;
    fn day_dom_cancel_frame(id: u32);
    fn day_dom_set_title(ptr: *const u8, len: usize);
    fn day_dom_open_url(ptr: *const u8, len: usize);
    fn day_dom_share_support() -> bool;
    fn day_dom_share_url(ptr: *const u8, len: usize, title: *const u8, title_len: usize) -> bool;
    /// Mirror the app route into the URL hash. `replace` = rewrite the current history entry
    /// (the launch reflection) instead of pushing a new one (in-app navigation).
    fn day_dom_set_hash(ptr: *const u8, len: usize, replace: u32);
    /// One dayscript reply line out to the page's WebSocket (docs/web.md; the shim queues
    /// until the socket is open and drops the line when scripting is not armed).
    fn day_dom_script_send(ptr: *const u8, len: usize);
    fn day_dom_env(key: *const u8, kl: usize, out: *mut u8, cap: usize) -> usize;
    /// An element read back for `read_native`, by the `day_dom_env` buffer protocol: its
    /// `role`, `aria-label`, value (`aria-valuetext`, else the description) and `id`, then
    /// number, checked, enabled, visible, the frame's x, y, w, h, and last the displayed text
    /// (`=`-prefixed), joined by U+001F; an empty field is one the element does not have.
    /// Empty when the element is gone.
    fn day_dom_read_native(el: u32, out: *mut u8, cap: usize) -> usize;
    /// The page's font list in Day's list text (docs/fonts.md) — the CSS generic families plus
    /// the bundled `document.fonts` faces — by the `day_dom_env` buffer protocol.
    fn day_dom_fonts(out: *mut u8, cap: usize) -> usize;
    /// Measure one line of canvas text with the CSS font the canvas replay draws it in;
    /// `out` receives the eight f64 `TextMetrics::from_slots` reads — width, the line box, the
    /// ascent, the cap height, then the ink box (docs/fonts.md).
    fn day_dom_canvas_measure_text(
        text: *const u8,
        len: usize,
        size: f64,
        weight: u32,
        italic: u32,
        fam: *const u8,
        fam_len: usize,
        out: *mut f64,
    );
    /// Apply an appearance override on the page: 0 light, 1 dark, 2 follow the browser's
    /// `prefers-color-scheme`. Returns the effective mode (0/1) after applying.
    fn day_dom_set_dark(mode: u32) -> u32;
    fn day_dom_warn(ptr: *const u8, len: usize);
    /// One log line to the browser console (docs/logging.md), at `log`'s level ordering —
    /// 1 Error … 5 Trace — so it lands on the matching `console.*` method and the devtools level
    /// filter applies to Day's output too.
    fn day_dom_log(level: u32, ptr: *const u8, len: usize);
    /// The page's wall clock (`Date.now()`), in milliseconds since the Unix epoch. This is the
    /// only wall clock on `wasm32-unknown-unknown` — `std::time::SystemTime::now()` aborts there.
    fn day_dom_now_ms() -> f64;
    /// `performance.now()`: milliseconds on the page's monotonic clock.
    fn day_dom_perf_now() -> f64;
    /// Fill `len` bytes at `ptr` with browser entropy (`crypto.getRandomValues`) — the
    /// getrandom bridge below answers the crate's custom-backend hook with it.
    fn day_dom_entropy(ptr: *mut u8, len: usize);
}

/// getrandom v0.3's custom-backend hook (its `custom.rs` contract): `day build` compiles every
/// web app with `--cfg getrandom_backend="custom"`, so any crate in the graph that asks
/// getrandom for entropy (uuid, rand, …) lands here instead of failing to compile — the
/// raw-wasm pipeline carries no wasm-bindgen runtime for getrandom's own `wasm_js` backend.
#[unsafe(no_mangle)]
#[expect(
    clippy::unnecessary_wraps,
    reason = "getrandom's custom-backend contract fixes this signature"
)]
unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error> {
    // SAFETY: getrandom hands us a valid, writable buffer of exactly `len` bytes; the shim
    // writes that many and no more.
    unsafe { day_dom_entropy(dest, len) };
    Ok(())
}

fn s(el: u32, prop: &str, val: &str) {
    unsafe { day_dom_set_style(el, prop.as_ptr(), prop.len(), val.as_ptr(), val.len()) };
}
fn attr(el: u32, a: &str, v: &str) {
    unsafe { day_dom_set_attr(el, a.as_ptr(), a.len(), v.as_ptr(), v.len()) };
}
fn text(el: u32, t: &str) {
    unsafe { day_dom_set_text(el, t.as_ptr(), t.len()) };
}
/// The number an accessibility value leads with ("72%" → 72, "3 of 5" → 3), for
/// `aria-valuenow`; `None` when the text starts with something else.
fn leading_number(value: &str) -> Option<f64> {
    let value = value.trim_start();
    let end = value
        .char_indices()
        .find(|(i, c)| !(c.is_ascii_digit() || *c == '.' || (*i == 0 && (*c == '-' || *c == '+'))))
        .map_or(value.len(), |(i, _)| i);
    value[..end].parse().ok()
}
fn apply_button_content(el: u32, title: &str, icon: Option<&day_spec::Icon>, icon_only: bool) {
    fn escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#39;")
    }
    let url = match icon {
        Some(day_spec::Icon::Symbol(s)) => symbol_svg(*s),
        Some(day_spec::Icon::Image(name)) => Some(image_url(name)),
        None => None,
    };
    // The title is the accessible name (an icon-only button shows none), unless the app named
    // the button itself (`set_a11y`); overwriting that replaced app labels on every title change.
    if !APP_NAMED.with(|t| t.contains(el as usize)) {
        attr(el, "aria-label", title);
    }
    attr(
        el,
        "title",
        if icon_only && url.is_some() {
            title
        } else {
            ""
        },
    );
    let html = match url {
        Some(url) => format!(
            "<span aria-hidden=\"true\" style=\"display:inline-block;width:20px;height:20px;vertical-align:middle;background:currentColor;mask:url('{}') center/contain no-repeat\"></span>{}",
            escape(&url.replace('\'', "%27").replace('"', "%22")),
            if icon_only {
                String::new()
            } else {
                format!(" <span>{}</span>", escape(title))
            }
        ),
        None => escape(title),
    };
    unsafe { day_dom_set_html(el, html.as_ptr(), html.len()) };
}

/// Put a [`ButtonStyleSpec`] on a `<button>`, keeping it a `<button>`.
///
/// A tint sets two CSS custom properties and adds `.tinted`; day.css does the rest, including
/// the `:hover`/`:active`/`:disabled` rules every `.day-btn` already has. Painting the colors
/// through variables rather than inline `background` is what lets those state rules keep
/// working — an inline background would win over them.
/// `ButtonProps::selected`: `aria-pressed` is the state a screen reader reads, and the class is
/// what day.css draws (the accent fill a chosen item gets); the element stays a `<button>`.
fn set_button_selected(el: u32, on: bool) {
    attr(el, "aria-pressed", if on { "true" } else { "false" });
    class(el, "selected", on);
}

fn apply_button_style(el: u32, style: ButtonStyleSpec) {
    // Clear the others first, so a patch between styles cannot leave two classes on.
    class(el, "prominent", false);
    class(el, "bordered", false);
    class(el, "tinted", false);
    class(el, "compact", false);
    match style {
        ButtonStyleSpec::Prominent => class(el, "prominent", true),
        ButtonStyleSpec::Bordered => class(el, "bordered", true),
        ButtonStyleSpec::Compact => class(el, "compact", true),
        ButtonStyleSpec::Tinted(c) => {
            let opaque = |x: day_spec::Color| day_spec::Color { a: 1.0, ..x };
            // The tint follows the appearance through CSS; the readable title color is
            // decided per side, from the tint each appearance shows.
            let (light, dark) = day_spec::Palette::both();
            let (on_light, on_dark) = c.pair(&light, &dark);
            let fill = match c {
                day_spec::Ink::Fixed(x) => color_css(opaque(x)),
                day_spec::Ink::Adaptive { light, dark } => pair_css(opaque(light), opaque(dark)),
                day_spec::Ink::System { role, .. } => ink_css(day_spec::Ink::System {
                    role,
                    alpha: Some(1.0),
                }),
            };
            s(el, "--day-tint", &fill);
            s(
                el,
                "--day-tint-fg",
                &pair_css(
                    ButtonStyleSpec::on_tint(on_light),
                    ButtonStyleSpec::on_tint(on_dark),
                ),
            );
            class(el, "tinted", true);
        }
        ButtonStyleSpec::Automatic | ButtonStyleSpec::Plain => {}
    }
}

/// Set a label's text, as styled spans when it has runs (docs/text-runs.md).
///
/// Runs become `<span>` children (a link run an `<a>`), which is what lets the whole thing stay
/// One wrapping paragraph — the browser wraps across the spans as if they were plain text. The
/// text goes through `textContent` per span rather than any HTML string, so a translated string
/// containing `<` or `&` is inert.
fn set_label_text(el: u32, s: &str, runs: &[day_spec::TextRun]) {
    if runs.is_empty() {
        text(el, s);
        return;
    }
    // Clear and rebuild: a label's runs change wholesale (LabelPatch::Runs carries both), so
    // there is nothing to diff against.
    text(el, "");
    let mut at = 0usize;
    for r in runs {
        // Same rule as the markup serializer: a run that does not address this string is
        // skipped without advancing, so the text survives even when the styling does not.
        let Some(styled) = s.get(r.range.clone()) else {
            continue;
        };
        if r.range.start > at
            && let Some(plain) = s.get(at..r.range.start)
        {
            append_span(el, plain, None);
        }
        append_span(el, styled, Some(r));
        at = r.range.end;
    }
    if let Some(tail) = s.get(at..) {
        append_span(el, tail, None);
    }
}

/// One `<span>` (or `<a>`) child carrying a run's styling.
fn append_span(parent: u32, content: &str, run: Option<&day_spec::TextRun>) {
    let kind = if run.is_some_and(|r| r.link.is_some()) {
        EL_LINK
    } else {
        EL_SPAN
    };
    let el = unsafe { day_dom_create(kind) };
    text(el, content);
    if let Some(r) = run {
        apply_font(el, &r.font);
        if let Some(c) = r.color {
            s(el, "color", &ink_css(c));
        }
        if let Some(c) = r.background {
            s(el, "background-color", &ink_css(c));
        }
        if let Some(url) = r.link.as_deref() {
            unsafe { day_dom_link(el, parent, url.as_ptr(), url.len()) };
        }
        // One element has one decoration style and color for all its lines: an underline and
        // a strikethrough that differ in either take an inner span for the strikethrough,
        // which inherits the outer underline's drawing (CSS propagates decorations to inline
        // descendants).
        match (r.underline, r.strikethrough) {
            (Some(u), Some(st)) if u != st => {
                apply_decoration(el, "underline", Some(u));
                let inner = unsafe { day_dom_create(EL_SPAN) };
                text(el, "");
                text(inner, content);
                apply_decoration(inner, "line-through", Some(st));
                unsafe { day_dom_insert(el, inner, u32::MAX) };
            }
            (Some(u), Some(_)) => apply_decoration(el, "underline line-through", Some(u)),
            (Some(u), None) => apply_decoration(el, "underline", Some(u)),
            (None, Some(st)) => apply_decoration(el, "line-through", Some(st)),
            (None, None) => {}
        }
    }
    unsafe { day_dom_insert(parent, el, u32::MAX) };
}

/// A decoration's lines, style and color on one element: `text-decoration-line`, `-style`
/// (`solid`, `double`, `dotted`, `wavy`) and `-color` (the text's own where the run names none,
/// docs/text-runs.md "Decorations").
fn apply_decoration(el: u32, lines: &str, d: Option<day_spec::Decoration>) {
    use day_spec::DecorationStyle as D;
    let Some(d) = d else { return };
    s(el, "text-decoration-line", lines);
    let style = match d.style {
        D::Single => "solid",
        D::Double => "double",
        D::Dotted => "dotted",
        D::Wavy => "wavy",
    };
    s(el, "text-decoration-style", style);
    s(
        el,
        "text-decoration-color",
        &d.color.map_or_else(|| "currentcolor".to_owned(), ink_css),
    );
}

fn class(el: u32, c: &str, on: bool) {
    unsafe { day_dom_set_class(el, c.as_ptr(), c.len(), u32::from(on)) };
}
fn warn(msg: &str) {
    unsafe { day_dom_warn(msg.as_ptr(), msg.len()) };
}
/// Read a host "environment" value (query params / navigator facts) as an app-facing
/// environment lookup: `day launch --env K=V` lands in the page URL's query string and is
/// read back here (docs/web.md). Empty/absent answers `None`. The page-fact keys (`vw`,
/// `vh`, `dpr`, `dark`, `reduce_motion`, `locales`, `route`) are reserved by the shim.
#[must_use]
pub fn host_env(key: &str) -> Option<String> {
    let v = env(key);
    if v.is_empty() { None } else { Some(v) }
}

/// Seconds on the page's monotonic clock (`performance.now()`), for `day_core::frame::now_secs`
/// on a target without `Instant`.
#[must_use]
pub fn monotonic_secs() -> f64 {
    unsafe { day_dom_perf_now() / 1000.0 }
}

/// Milliseconds since the Unix epoch, from the page's `Date.now()`. The wasm target has no
/// working `SystemTime::now()`, so time-of-day code asks the host page instead (a zone-aware app's
/// clock routes through here on `web-dom`).
#[must_use]
pub fn now_epoch_ms() -> u64 {
    let ms = unsafe { day_dom_now_ms() };
    if ms.is_finite() && ms > 0.0 {
        ms as u64
    } else {
        0
    }
}

/// Read a host "environment" value (query params / navigator facts) into a String. A value
/// longer than the first buffer gets a right-sized retry instead of a mid-UTF-8 truncation:
/// a shim that reports the value's full length triggers the `n > cap` branch directly, and
/// one that clamps its return to the buffer (writing exactly `cap` bytes) grows until there
/// is headroom.
fn env(key: &str) -> String {
    read_buffer(|out, cap| unsafe { day_dom_env(key.as_ptr(), key.len(), out, cap) })
}

/// The `day_dom_env` buffer protocol for any string the shim answers: `fill` writes into the
/// buffer and returns the length, and a full buffer means retry with a larger one.
fn read_buffer(fill: impl Fn(*mut u8, usize) -> usize) -> String {
    let mut cap = 512usize;
    loop {
        let mut buf = vec![0u8; cap];
        let n = fill(buf.as_mut_ptr(), buf.len());
        if n > cap {
            cap = n;
            continue;
        }
        if n == cap {
            cap *= 2;
            continue;
        }
        buf.truncate(n);
        return String::from_utf8_lossy(&buf).into_owned();
    }
}

thread_local! {
    /// Elements the app gave an accessibility label (`set_a11y`), whose `aria-label` a button's
    /// title change must leave alone. A `SideTable`, so release sweeps it.
    static APP_NAMED: day_spec::sidetable::SideTable<()> = day_spec::sidetable::SideTable::new();
    /// Menu button element → the provider its press opens (`set_button_menu`, docs/menus.md
    /// "Menu buttons"). A `SideTable`, so release sweeps it.
    static BUTTON_MENUS: day_spec::sidetable::SideTable<day_spec::ContextMenuFn> =
        day_spec::sidetable::SideTable::new();
}

/// What a realized image was given, so a patch can rebuild its mask from the same URL and
/// scaling (`apply_image_tint`). The DOM bridge writes attributes but does not read them back,
/// and keeping this here is cheaper than adding a getter to the shim for one caller.
#[derive(Clone)]
struct ImageState {
    src: String,
    /// The `object-fit` keyword the content mode mapped to, which the mask scales by.
    fit: &'static str,
    tint: Option<day_spec::Ink>,
    template: bool,
    /// Realized as an `<img>` (no tint, not a template) rather than as the masked div.
    is_img: bool,
}

impl ImageState {
    /// A masked element rather than an `<img>`: a tint or a template paints the mask.
    fn masked(&self) -> bool {
        self.tint.is_some() || self.template
    }
}

thread_local! {
    /// The linear gradient of each surface filled with one (docs/surfaces.md) and the size it
    /// was last written for, kept so `set_frame` can resolve its unit points against the new
    /// size. A `SideTable`, so release sweeps it.
    static SURFACE_GRADIENTS: day_spec::sidetable::SideTable<(day_spec::LinearGradient, Size)> =
        day_spec::sidetable::SideTable::new();

    /// The realize-time role of each surface given one, which a `ContainerPatch::Surface`
    /// keeps (its own `role` is ignored).
    static SURFACE_ROLES: day_spec::sidetable::SideTable<day_spec::SurfaceRole> =
        day_spec::sidetable::SideTable::new();

    /// Each realized image's [`ImageState`].
    static IMAGE_SRC: RefCell<HashMap<u32, ImageState>> = RefCell::new(HashMap::new());

    /// `BitmapId` → what the decode reported (docs/images.md). Seeded when the decode is
    /// REQUESTED, not when it answers: the container format comes from the bytes on this side and
    /// the alpha answer follows from it, because an `ImageBitmap` exposes neither. The browser
    /// fills in only the pixel size.
    static BITMAP_INFO: RefCell<HashMap<u64, day_spec::BitmapInfo>> = RefCell::new(HashMap::new());
}

/// Recolor an image (docs/vectors.md "Tint", "Template"; docs/images.md "Tinting").
///
/// The browser cannot recolor the pixels of an `<img>`, so a tinted image becomes a MASK painted
/// with the tint — the same technique the nav rows use for their icons. A CSS mask is cut from
/// the image's alpha, so this is the source-in recolor the spec asks for, for a raster as much
/// as for a glyph. A template with no tint paints the mask with `currentColor`, so it follows
/// the text color around it (light on a dark sidebar, white on a selected row) the way those
/// rows do.
///
/// An element realized tinted (or as a template) is a div and only ever shows the mask. One
/// realized plain is an `<img>`; a tint that arrives later masks it too and moves its own
/// pixels out of its box (`object-position`), so only the painted mask shows, and taking the
/// tint away puts it back exactly as it was.
fn apply_image_tint(el: u32, state: &ImageState) {
    if state.masked() {
        if state.src.is_empty() {
            // Bytes or a decoded bitmap: the shim owns the URL and cuts the mask from it, now
            // and at every later source swap.
            // SAFETY: an element id the shim either knows or does not.
            unsafe { day_dom_image_mask(el, 1) };
        } else {
            let mask = format!("url(\"{}\")", state.src);
            s(el, "mask-image", &mask);
            s(el, "-webkit-mask-image", &mask);
        }
        // The mask scales the way the untinted image would: `object-fit` and `mask-size`
        // share contain/cover, and Stretch's `fill` is `100% 100%`.
        let size = if state.fit == "fill" {
            "100% 100%"
        } else {
            state.fit
        };
        for prop in ["mask-size", "-webkit-mask-size"] {
            s(el, prop, size);
        }
        for prop in ["mask-repeat", "-webkit-mask-repeat"] {
            s(el, prop, "no-repeat");
        }
        for prop in ["mask-position", "-webkit-mask-position"] {
            s(el, prop, "center");
        }
        let paint = state
            .tint
            .map_or_else(|| "currentColor".to_owned(), ink_css);
        s(el, "background-color", &paint);
        if state.is_img {
            s(el, "object-position", "-100000px -100000px");
        }
    } else {
        // SAFETY: as above.
        unsafe { day_dom_image_mask(el, 0) };
        for prop in [
            "mask-image",
            "-webkit-mask-image",
            "mask-size",
            "-webkit-mask-size",
            "mask-repeat",
            "-webkit-mask-repeat",
            "mask-position",
            "-webkit-mask-position",
            "background-color",
            "object-position",
        ] {
            s(el, prop, "");
        }
    }
}

/// The staged file extension for a bare image NAME: `svg` when the name is a bundled vector
/// glyph (docs/vectors.md — the browser then renders it at display size), `png` otherwise.
/// The page carries the vector-name list (`window.__DAY_VECTORS`, injected at assemble) and
/// answers through the env channel's reserved `vector:` keys, so an older host page simply
/// answers empty and the raster fallback still resolves.
fn image_ext(name: &str) -> &'static str {
    if env(&format!("vector:{name}")).is_empty() {
        "png"
    } else {
        "svg"
    }
}

/// The staged glyph a name resolves to. A weight variant with no art of its own falls back to its
/// base glyph (docs/vectors.md): only SF-template sources stage `__light`/`__bold`, so a plain
/// SVG's weight name would otherwise request an asset that was never staged. The base must itself
/// be a known vector before we strip, so an ORDINARY image that happens to end in `__bold` is
/// left alone.
fn resolved_name(name: &str) -> String {
    if !env(&format!("vector:{name}")).is_empty() {
        return name.to_string();
    }
    for suffix in ["__light", "__bold"] {
        if let Some(base) = name.strip_suffix(suffix)
            && !base.is_empty()
            && !env(&format!("vector:{base}")).is_empty()
        {
            return base.to_string();
        }
    }
    name.to_string()
}

/// The page-relative URL for an image or vector NAME, alias-resolved.
fn image_url(name: &str) -> String {
    let n = resolved_name(name);
    format!("assets/images/{n}.{}", image_ext(&n))
}

/// A [`Symbol`](day_spec::Symbol) as an inline-SVG `data:` URL, for the same CSS mask the shim
/// draws a bundled image through — so a toolbar looks the same whether its items asked for a
/// standard symbol or shipped their own art.
///
/// The web has no system icon set. Every other backend hands a `Symbol` to one the OS supplies
/// (SF Symbols, freedesktop names, Fluent glyphs); here the alternative to drawing them is what
/// this used to do, which was to draw nothing and let the label carry the item — leaving a bar
/// where some items had icons and some did not, depending on whether they happened to use a
/// bundled image.
///
/// The paths are deliberately plain geometry on a 24×24 grid, authored here rather than taken
/// from an icon set: it keeps day free of a third-party icon license, and at toolbar size the
/// shapes are what read, not their styling. `None` for a symbol with no glyph yet — the item
/// falls back to its label, which is what the whole set did before.
fn symbol_svg(sym: day_spec::Symbol) -> Option<String> {
    // The shared outline table (day-spec), inlined as a data: URL. Single quotes in the markup
    // and a percent-encoded `<`, `>` and `#` keep it valid in both the CSS value and the
    // attribute the shim assigns it to.
    let d = sym.outline_path()?;
    Some(format!(
        "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'%3E%3Cpath fill-rule='evenodd' d='{d}'/%3E%3C/svg%3E"
    ))
}

// ---------------------------------------------------------------------------
// Element kinds the shim knows how to create (shim.js `create()` mirrors this table).
// ---------------------------------------------------------------------------

const EL_DIV: u32 = 0;
const EL_LABEL: u32 = 1;
const EL_BUTTON: u32 = 2;
const EL_TOGGLE: u32 = 3;
const EL_SLIDER: u32 = 4;
const EL_FIELD: u32 = 5;
const EL_AREA: u32 = 6;
const EL_SELECT: u32 = 7;
const EL_PROGRESS: u32 = 8;
const EL_SPINNER: u32 = 9;
const EL_IMAGE: u32 = 10;
const EL_CANVAS: u32 = 11;
const EL_SCROLL: u32 = 12;
const EL_DIVIDER: u32 = 13;
const EL_NAV: u32 = 14;
const EL_PAGE: u32 = 15;
const EL_NAVMENU: u32 = 16;
const EL_CELL: u32 = 18;
const EL_SEGMENTED: u32 = 19;
const EL_RADIOS: u32 = 20;
/// A styled run inside a label (docs/text-runs.md), and the same as a link.
const EL_SPAN: u32 = 21;
const EL_LINK: u32 = 22;

/// Wire event kinds the shim reports through `day_dom_event` (shim.js mirrors this table).
mod ev {
    pub const CLICK: u32 = 1; // a = ctrl/cmd bit0 + shift bit1 modifier mask
    pub const SUBMIT: u32 = 3;
    pub const TOGGLE: u32 = 4; // a = 0/1
    pub const VALUE: u32 = 5; // a
    pub const SELECT: u32 = 6; // a = index
    pub const FOCUS: u32 = 7; // a = 0/1
    pub const TAP: u32 = 8; // a,b = local point
    pub const DRAG_BEGAN: u32 = 9; // a,b = location; c,d = translation
    pub const DRAG_MOVED: u32 = 10;
    pub const DRAG_ENDED: u32 = 11;
    pub const SCROLL: u32 = 12; // a,b = offset
    pub const RESIZED: u32 = 13; // a,b = size (ResizeObserver)
    pub const NAV_BACK: u32 = 14;
    /// a = the settled value. The DOM already separates the two: `input` fires as the thumb
    /// moves, `change` once the user lets go (day-spec `Event::ValueCommitted`).
    pub const VALUE_COMMITTED: u32 = 15;
    /// a,b = local point; c,d = the same point in viewport coordinates. The browser's
    /// `contextmenu` (right-click / long-press), default prevented; the pieces layer
    /// presents the composed menu at the viewport point (docs/menus.md).
    pub const CONTEXT: u32 = 16;
    /// a,b = local point. Pointer entry, motion and exit over the element
    /// (docs/canvas.md "Interaction"); the exit carries the last point seen inside.
    pub const HOVER_ENTER: u32 = 17;
    pub const HOVER_MOVED: u32 = 18;
    pub const HOVER_LEFT: u32 = 19;
    pub const PAN: u32 = 20;
    /// a,b = local centroid; c = scale, cumulative since `PINCH_BEGAN` for a two-finger
    /// touch, the tick's own factor for a trackpad pinch (a ctrl+wheel, which arrives as a
    /// lone `PINCH_MOVED`; docs/canvas.md "Zoom and pan").
    pub const PINCH_BEGAN: u32 = 21;
    pub const PINCH_MOVED: u32 = 22;
    pub const PINCH_ENDED: u32 = 23;
    /// a,b = local centroid; c = rotation in radians, clockwise positive, cumulative since
    /// `ROTATE_BEGAN` (two fingers, or Safari's trackpad gesture events; docs/canvas.md "Zoom
    /// and pan").
    pub const ROTATE_BEGAN: u32 = 24;
    pub const ROTATE_MOVED: u32 = 25;
    pub const ROTATE_ENDED: u32 = 26;
    /// a,b = the press point; fired once while the pointer is still down (docs/gestures.md
    /// "Long press").
    pub const LONG_PRESS: u32 = 27;
    /// a,b = the second tap's point (`dblclick`).
    pub const DOUBLE_TAP: u32 = 28;
    /// a,b = the point; c = 1 for the press, 0 for its end (docs/gestures.md "Press").
    pub const PRESS: u32 = 29;
    /// a = the user's scroll phase: 0 idle, 1 dragging, 2 coasting (docs/scroll.md "Phases").
    pub const SCROLL_PHASE: u32 = 30;
}

/// Listen-mask bits for the recognizers `enable_gesture` attaches (shim.js `listen`), beyond
/// the public [`listen`] constants.
mod gesture_mask {
    pub const TAP: u32 = 128;
    pub const DRAG: u32 = 256;
    pub const CONTEXT_MENU: u32 = 1024;
    pub const PAN: u32 = 2048;
    pub const PINCH: u32 = 4096;
    pub const ROTATE: u32 = 8192;
    pub const LONG_PRESS: u32 = 1 << 14;
    pub const DOUBLE_TAP: u32 = 1 << 15;
    pub const PRESS: u32 = 1 << 16;
    pub const TOUCHES: u32 = 1 << 17;
    pub const HOVER: u32 = 1 << 18;
}

// ---------------------------------------------------------------------------
// Toolkit state
// ---------------------------------------------------------------------------

/// A shim element id. The shim keeps `els[id] = Element`; Rust only ever sees the number.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DomHandle(pub u32);

struct NavState {
    presentation: NavPresentation,
    /// Detail pages. In `Stack` they are a push stack and the sidebar page is the root, so it
    /// appears here first; everywhere else they are the host's detail children in attach order
    /// and the sidebar page lives in the chrome slot instead. A re-present moves it between the
    /// two (see `nav_present`).
    pages: Vec<u32>,
    /// The host's sidebar page, once it has one. Tracked by identity so a re-present can re-home
    /// it without depending on where it currently sits.
    sidebar: Option<u32>,
    titles: Vec<String>,
    /// Which detail page is showing in a presentation whose rows are chrome — an index into
    /// `pages`. Meaningless while stacked, where the top of the stack is always the last page.
    selected: usize,
}

/// The shim's presentation encoding. The FFI edge speaks numbers, so this is the one place the
/// mapping lives; the shim's `navChrome` switches on the same four.
fn nav_mode(p: NavPresentation) -> u32 {
    match p {
        NavPresentation::Split => 0,
        NavPresentation::Stack => 1,
        NavPresentation::Tabs => 2,
        NavPresentation::Rail => 3,
    }
}

struct ListEntry {
    activatable: bool,
    node: NodeId,
    content: u32,
    /// The declared pitch of a `Uniform` list; for an `Automatic` one, the estimate a row
    /// stands at until it has been measured.
    row_height: f64,
    /// `RowHeight::Automatic`: each row takes the height Day laid it at
    /// (`ListSource::cell_height`), and rows stack by those heights rather than a pitch.
    automatic: bool,
    /// A self-sizing list's measured row heights, by row; `0.0` for a row never bound, which
    /// stands at [`ListEntry::estimate`]. Empty for a `Uniform` list.
    heights: Vec<f64>,
    source: Option<ListSource>,
    /// One slot per row, 0 until that row has been shown: cell index == row index, so a slot's
    /// identity never changes and a realized cell stays that row's for good.
    cells: Vec<u32>,
    /// Which rows' content is currently built and current. Cleared wholesale when the source
    /// changes under the cells; individual rows go true as they are bound.
    bound: Vec<bool>,
    /// A scroll-driven fill is already posted — a flick emits a stream of scroll events, and
    /// they would each post a pass that does the same work.
    fill_pending: bool,
    /// A row jump (`ListPatch::ScrollToRow`) for the next fill to repeat once the content has
    /// its full height.
    pending_scroll: Option<f64>,
    /// The offset, visible height and row count the last fill read: the viewport its report
    /// describes. A list that now reads otherwise owes a report (`list_settled`).
    seen: Option<(f64, f64, usize)>,
    /// `ui_idle` polls in a row that found a report owed (`list_settled`): past
    /// [`LIST_OWED_POLLS`] the report stops counting as owed.
    owed_polls: u32,
    last_width: f64,
    selectable: bool,
    /// Host-drawn row separators (docs/list.md): a bottom border on each cell, so the line
    /// sits exactly at the row boundary the cell frame defines.
    separators: bool,
    multi: bool,
    selected: BTreeSet<usize>,
    /// The FIXED end of a shifted range — the row the extension pivots on, set by a plain click
    /// or a plain arrow and left alone while shift moves the other end.
    anchor: Option<usize>,
    /// The MOVING end: where the keyboard is, and the row a shifted arrow walks. Separate from
    /// the anchor because a range has two ends and `shift+↓ ↓ ↑` has to grow twice and shrink
    /// once — one field cannot both stay put and move.
    lead: Option<usize>,
    /// A sectioned list's pinned header (docs/list.md "Sections"): a sticky wrapper at the top
    /// of the scroll content holding ONE extra cell, bound to the header of the section under
    /// the top edge. `(wrapper, cell)`, both 0 until a header first scrolls off.
    pin: (u32, u32),
    /// The header row bound into the pinned cell, so it rebinds only when the section changes.
    pin_row: Option<usize>,
}

impl ListEntry {
    /// Whether row `row` is a section header (docs/list.md "Sections"); always false on an
    /// unsectioned list.
    fn is_header(&self, row: usize) -> bool {
        self.source
            .as_ref()
            .and_then(|src| src.is_header.as_ref())
            .is_some_and(|is_header| is_header(row))
    }

    fn sectioned(&self) -> bool {
        self.source
            .as_ref()
            .is_some_and(|src| src.is_header.is_some())
    }

    /// Whether row `row` is a header, footer, empty state or padding row Day added
    /// (docs/list.md "Header, footer and empty state"): sized like a header, with no row chrome.
    fn is_supplement(&self, row: usize) -> bool {
        self.source
            .as_ref()
            .and_then(|src| src.is_supplement.as_ref())
            .is_some_and(|f| f(row))
    }

    fn has_supplements(&self) -> bool {
        self.source
            .as_ref()
            .is_some_and(|src| src.is_supplement.is_some())
    }

    /// A row the reader can select, activate and walk to: neither a header nor a supplement.
    fn is_item(&self, row: usize) -> bool {
        !self.is_header(row) && !self.is_supplement(row)
    }

    /// Rows stand at their own heights rather than a pitch: a self-sizing list, or one with
    /// headers or supplementary rows, which take their laid height even when it is `Uniform`.
    fn stacked(&self) -> bool {
        self.automatic || self.sectioned() || self.has_supplements()
    }

    /// The header of the section holding `row`: the row itself, or the nearest header above.
    fn header_of(&self, row: usize) -> Option<usize> {
        (0..=row).rev().find(|i| self.is_header(*i))
    }

    /// What the pinned header covers above `row`: the height of its section's header, so a
    /// row brought into view lands below the pin instead of under it. 0 for a header row or
    /// an unsectioned list.
    fn pinned_above(&self, row: usize) -> f64 {
        if !self.sectioned() || self.is_header(row) {
            return 0.0;
        }
        self.header_of(row).map_or(0.0, |h| {
            let (top, bottom) = self.row_span(h);
            bottom - top
        })
    }

    /// What an unmeasured row of a self-sizing list stands at: the mean of the rows measured
    /// so far, so the scroll extent tracks the real rows, or the declared fallback before any.
    fn estimate(&self) -> f64 {
        let (sum, count) = self
            .heights
            .iter()
            .filter(|h| **h > 0.0)
            .fold((0.0, 0usize), |(s, c), h| (s + h, c + 1));
        if count == 0 {
            self.row_height.max(1.0)
        } else {
            sum / count as f64
        }
    }

    /// Every row's top for `n` rows, then the whole extent: `n + 1` entries. A uniform list's
    /// are multiples of its pitch; a self-sizing list's add up its measured heights, standing
    /// each unmeasured row at the estimate.
    fn tops(&self, n: usize) -> Vec<f64> {
        let estimate = self.estimate();
        let mut tops = Vec::with_capacity(n + 1);
        let mut y = 0.0;
        for i in 0..n {
            tops.push(y);
            y += self.height_of(i, estimate);
        }
        tops.push(y);
        tops
    }

    /// Row `i`'s height: the pitch, or its measured height, or `estimate`. A section header
    /// is measured even in a uniform list, whose `heights` then hold only headers.
    fn height_of(&self, i: usize, estimate: f64) -> f64 {
        if !self.automatic && self.is_item(i) {
            return self.row_height.max(1.0);
        }
        match self.heights.get(i) {
            Some(h) if *h > 0.0 => *h,
            _ => estimate,
        }
    }

    /// Row `row`'s top and bottom edge in the list's content.
    fn row_span(&self, row: usize) -> (f64, f64) {
        if !self.stacked() {
            let h = self.row_height.max(1.0);
            return (row as f64 * h, (row + 1) as f64 * h);
        }
        let estimate = self.estimate();
        let top: f64 = (0..row).map(|i| self.height_of(i, estimate)).sum();
        (top, top + self.height_of(row, estimate))
    }
}

/// The grabber strip above a half sheet's content, in CSS px (`.day-cover.day-half`'s
/// `padding-top` in day.css).
const HALF_SHEET_GRABBER: f64 = 20.0;

thread_local! {
    static SINK: RefCell<Option<EventSink>> = const { RefCell::new(None) };
    /// The page's display state as last reported (`report_window_state`): Normal or Fullscreen.
    static WINDOW_STATE: Cell<day_spec::WindowState> =
        const { Cell::new(day_spec::WindowState::Normal) };
    /// Element id → day node id, for event routing (the shim only knows element ids).
    static NODE_OF: RefCell<HashMap<u32, NodeId>> = RefCell::new(HashMap::new());
    /// Elements whose frames are CSS-managed (nav/tab pages): `set_frame` skips them.
    static CSS_FRAMED: RefCell<std::collections::HashSet<u32>> = RefCell::new(std::collections::HashSet::default());
    /// Covers presented as half sheets (`CoverProps::half`, docs/cover.md): the bottom half
    /// of the viewport, under a grabber strip the stylesheet pads for.
    static COVER_HALF: RefCell<std::collections::HashSet<u32>> = RefCell::new(std::collections::HashSet::default());
    /// Covers built as modal sheets (`CoverProps::sheet`, docs/cover.md "Modal sheet"). Whether
    /// one shows the composed card is decided at each present (a compact window keeps the full
    /// cover), so this only says it may.
    static COVER_SHEET: RefCell<std::collections::HashSet<u32>> = RefCell::new(std::collections::HashSet::default());
    static NAV_STATE: RefCell<HashMap<u32, NavState>> = RefCell::new(HashMap::new());
    /// NAV_PAGE element → is-sidebar, recorded at realize for the nav `insert`.
    static PAGE_SIDEBAR: RefCell<HashMap<u32, bool>> = RefCell::new(HashMap::new());
    static LISTS: RefCell<HashMap<u32, ListEntry>> = RefCell::new(HashMap::new());
    /// List cell element → (list host element, row index) for selection clicks.
    static CELL_ROWS: RefCell<HashMap<u32, (u32, usize)>> = RefCell::new(HashMap::new());
    /// Spinner-vs-bar per PROGRESS element (progress with `None` renders as a spinner).
    static SPINNERS: RefCell<std::collections::HashMap<u32, f64>> = RefCell::new(HashMap::default());
    static POSTED: RefCell<VecDeque<Box<dyn FnOnce() + Send>>> = RefCell::new(VecDeque::new());
    static DELAYED: RefCell<HashMap<u32, Box<dyn FnOnce() + Send>>> = RefCell::new(HashMap::new());
    static NEXT_DELAY: Cell<u32> = const { Cell::new(1) };

    static SPLIT_MODE: Cell<bool> = const { Cell::new(true) };
    static DARK: Cell<bool> = const { Cell::new(false) };
    /// The app-wide appearance (`Toolkit::set_appearance`) and the window's own override
    /// (`WindowChange::Appearance`); the page is the one window, so the override wins.
    static APPEARANCE: Cell<(Option<bool>, Option<bool>)> = const { Cell::new((None, None)) };
    /// The latest viewport size (updated on resize, seeded at launch). A `cover` fills the
    /// viewport (`position:fixed; inset:0`), so presenting it seeds its frame from here
    /// SYNCHRONOUSLY — without waiting for the async ResizeObserver, whose gap otherwise leaves
    /// RTL content laid out at width 0 (i.e. off-screen to the left) until the observer fires
    /// (docs/cover.md).
    static LAST_VIEWPORT: Cell<Size> = const { Cell::new(Size::new(0.0, 0.0)) };
    /// True until the first `set_route` — the launch reflection replaces the history entry.
    static FIRST_ROUTE: Cell<bool> = const { Cell::new(true) };
    /// The showcase's `select:`-driven pickers: segmented/radio groups keep their option
    /// count so a programmatic Selected patch can re-style the active option.
    static SEG_COUNT: RefCell<HashMap<u32, usize>> = RefCell::new(HashMap::new());
    /// Each picker's last programmatic selection, so a change of OPTIONS can keep it: the
    /// shim's option builder rewrites the element, and the DOM's own selectedIndex goes with
    /// it.
    static PICKER_SELECTED: RefCell<HashMap<u32, usize>> = RefCell::new(HashMap::new());
    /// Per-picker intrinsic size, computed from its option strings at realize (the element's
    /// own textContent concatenates every option, so measuring it lies about width — and a
    /// vertical radio group needs a per-row height the one-line measure can't produce).
    static PICKER_SIZE: RefCell<HashMap<u32, Size>> = RefCell::new(HashMap::new());
    // Text-area sizing hints (docs/textarea.md): `(min_lines, max_lines, content_lines)`, kept
    // so `measure` can honor the auto-growing-height contract the other backends implement.
    // `content_lines` counts the day-driven text's hard line breaks (realize + SetText) — an
    // approximation that ignores soft wrapping, noted in `measure`.
    static AREA_HINTS: RefCell<HashMap<u32, (u32, u32, u32)>> = RefCell::new(HashMap::new());
    /// Per label element: the text it stands for and its paragraph attributes (docs/text.md).
    static LABELS: RefCell<HashMap<u32, LabelState>> = RefCell::new(HashMap::new());
    /// The recognizer bits already attached per element (`enable_gesture` is idempotent).
    static GESTURES: RefCell<HashMap<u32, u32>> = RefCell::new(HashMap::new());
    /// The reading direction `set_layout_direction` gave an element (`true` = right to left);
    /// an element it never named follows the app's.
    static DIRECTION: RefCell<HashMap<u32, bool>> = RefCell::new(HashMap::new());
}

/// Attach the shim listeners for `mask`'s bits not yet attached to `el`: a recognizer is a set of
/// listeners, and a second `listen` would double what it reports.
fn listen_once(el: u32, mask: u32) {
    let fresh = GESTURES.with(|g| {
        let mut g = g.borrow_mut();
        let had = g.entry(el).or_insert(0);
        let fresh = mask & !*had;
        *had |= mask;
        fresh
    });
    if fresh != 0 {
        unsafe { day_dom_listen(el, fresh) };
    }
}

/// What a label element shows and how (docs/text.md): kept beside the element because a head or
/// middle truncation replaces the element's text with an elided copy, and every later change
/// (width, font, paragraph, text) elides the full text again.
struct LabelState {
    /// The full text: `LabelProps::text` or the latest text patch.
    text: String,
    /// Styled runs are spans: elision (plain text only) falls back to the tail there.
    runs: bool,
    wraps: bool,
    para: LabelParagraph,
    /// The width of the last frame, which head and middle truncation elide to.
    width: f64,
    /// Whether the element shows an elided copy rather than `text`.
    elided: bool,
    /// Whether a nonempty selection was reported last (`LabelProps::reports_selection`).
    selected: bool,
}

impl LabelState {
    /// One line, where every truncation mode applies (docs/text.md "Truncation").
    fn one_line(&self) -> bool {
        !self.wraps || self.para.max_lines == 1
    }

    /// Whether the element must show `day_spec::elide`'s copy: CSS truncates only the tail.
    fn elides(&self) -> bool {
        self.one_line() && !self.runs && self.para.truncation != Truncation::Tail
    }
}

/// A label's new text (a text or runs patch); a selection the patch discards is reported
/// cleared.
fn set_label_state_text(el: u32, t: &str, runs: bool) {
    let cleared = LABELS.with(|tab| {
        let mut tab = tab.borrow_mut();
        let Some(st) = tab.get_mut(&el) else {
            return false;
        };
        t.clone_into(&mut st.text);
        st.runs = runs;
        std::mem::take(&mut st.selected)
    });
    if cleared && let Some(node) = node_of(el) {
        emit(node, Event::TextSelected { start: 0, end: 0 });
    }
}

/// A line height a label can use: a finite positive multiple of the font size.
fn usable_line_height(multiple: f64) -> Option<f64> {
    (multiple.is_finite() && multiple > 0.0).then_some(multiple)
}

/// The paragraph attributes on a label element (docs/text.md): letter spacing in `em`, a
/// unitless `line-height` (exactly Day's multiple-of-the-font-size model), and the line limit
/// as `-webkit-line-clamp` on a vertical `-webkit-box`, which the measuring copy in the shim
/// carries too. Idempotent; called at realize, on every paragraph or font patch (the `font`
/// shorthand resets `line-height`), and the elision follows.
fn apply_paragraph(el: u32) {
    let Some((wraps, para)) = LABELS.with(|t| t.borrow().get(&el).map(|st| (st.wraps, st.para)))
    else {
        return;
    };
    let ls = if para.letter_spacing.is_finite() && para.letter_spacing != 0.0 {
        format!("{}em", para.letter_spacing)
    } else {
        String::new()
    };
    s(el, "letter-spacing", &ls);
    // Natural is what `font_css`'s shorthand says: 1.3.
    let lh =
        usable_line_height(para.line_height).map_or_else(|| "1.3".to_owned(), |m| m.to_string());
    s(el, "line-height", &lh);
    if wraps && para.max_lines > 0 {
        let n = para.max_lines.to_string();
        s(el, "display", "-webkit-box");
        s(el, "-webkit-box-orient", "vertical");
        s(el, "-webkit-line-clamp", &n);
        s(el, "line-clamp", &n);
    } else {
        s(el, "display", "");
        s(el, "-webkit-box-orient", "");
        s(el, "-webkit-line-clamp", "");
        s(el, "line-clamp", "");
    }
    if wraps && para.max_lines == 1 {
        // One clamped line wraps nowhere: the tail ellipsis of a single line.
        s(el, "white-space", "nowrap");
    } else if wraps {
        s(el, "white-space", "");
    }
    MEASURE_CACHE.with(|c| c.borrow_mut().retain(|(e, _), _| *e != el));
    elide_label(el);
}

/// Show `day_spec::elide`'s copy of a one-line label truncated at the head or the middle, or the
/// full text again where none is needed. The full text stays the element's `data-day-text` (what
/// `read_native` reports) and, unless the app named the element, its `aria-label`.
fn elide_label(el: u32) {
    let Some((full, elides, was, width, mode)) = LABELS.with(|t| {
        t.borrow().get(&el).map(|st| {
            (
                st.text.clone(),
                st.elides(),
                st.elided,
                st.width,
                st.para.truncation,
            )
        })
    }) else {
        return;
    };
    let shown = if elides && width > 0.0 {
        day_spec::elide(&full, mode, width, |candidate| {
            label_size(el, Some(candidate), f64::MAX).width
        })
    } else {
        full.clone()
    };
    let now = shown != full;
    if now || was {
        text(el, &shown);
    }
    if now || was {
        attr(el, "data-day-text", if now { &full } else { "" });
        if APP_NAMED.with(|t| t.get(el as usize)).is_none() {
            attr(el, "aria-label", if now { &full } else { "" });
        }
    }
    LABELS.with(|t| {
        if let Some(st) = t.borrow_mut().get_mut(&el) {
            st.elided = now;
        }
    });
}

/// A label's size in its own style, for `text` in place of its content when given.
fn label_size(el: u32, text: Option<&str>, max_w: f64) -> Size {
    let mut out = [0.0f64; 2];
    let (ptr, len) = text.map_or((std::ptr::null(), 0), |t| (t.as_ptr(), t.len()));
    // SAFETY: `text` outlives the call and `out` has room for the two values the shim writes.
    unsafe { day_dom_measure_label(el, ptr, len, max_w, out.as_mut_ptr()) };
    Size::new(out[0], out[1])
}

/// The byte offset in `text` of a UTF-16 offset (the DOM's `Range` counts), clamped to the
/// text and landing on a character boundary.
fn utf16_to_byte(text: &str, units: usize) -> usize {
    let mut seen = 0usize;
    for (i, ch) in text.char_indices() {
        if seen >= units {
            return i;
        }
        seen += ch.len_utf16();
    }
    text.len()
}

/// Whether `el` reads right to left: its own direction once told, else the app's.
fn element_rtl(el: u32) -> bool {
    DIRECTION
        .with(|d| d.borrow().get(&el).copied())
        .unwrap_or_else(day_core::is_rtl)
}

/// The shim's pointer-kind code (0 touch, 1 mouse, 2 stylus) as Day's kind.
fn pointer_kind(code: u32) -> day_spec::PointerKind {
    match code {
        0 => day_spec::PointerKind::Touch,
        2 => day_spec::PointerKind::Stylus,
        _ => day_spec::PointerKind::Mouse,
    }
}

/// One contact of the shim's touches wire, `"id,phase,x,y,pressure,kind"` (phase 0 began,
/// 1 moved, 2 stationary, 3 ended, 4 cancelled; kind 0 touch, 1 mouse, 2 stylus), or `None`
/// for a malformed entry.
fn parse_touch(entry: &str) -> Option<day_spec::Touch> {
    use day_spec::{Touch, TouchPhase};
    let mut f = entry.split(',');
    let id: u64 = f.next()?.trim().parse().ok()?;
    let phase = match f.next()?.trim() {
        "0" => TouchPhase::Began,
        "1" => TouchPhase::Moved,
        "2" => TouchPhase::Stationary,
        "3" => TouchPhase::Ended,
        "4" => TouchPhase::Cancelled,
        _ => return None,
    };
    let x: f64 = f.next()?.trim().parse().ok()?;
    let y: f64 = f.next()?.trim().parse().ok()?;
    let pressure: f64 = f.next()?.trim().parse().ok()?;
    let kind = pointer_kind(f.next()?.trim().parse().ok()?);
    Some(
        Touch::new(id, phase, Point::new(x, y))
            .with_pressure(pressure)
            .with_kind(kind),
    )
}

/// The touches wire's whole list (entries separated by `;`), skipping malformed ones.
fn parse_touches(wire: &str) -> Vec<day_spec::Touch> {
    wire.split(';')
        .filter(|e| !e.is_empty())
        .filter_map(parse_touch)
        .collect()
}

/// A key event from the shim's modifier word: the day mask in the low byte, the phase in bits 8
/// and 9 ([`day_spec::KeyPhase::from_wire`]). A release carries no text.
fn phased_key(key: &str, text: Option<String>, word: u32) -> day_spec::KeyEvent {
    let phase = day_spec::KeyPhase::from_wire(f64::from(word));
    let modifiers = (word & 0xff) as u8;
    match text {
        Some(text) if phase != day_spec::KeyPhase::Up => {
            day_spec::KeyEvent::typed_by(key, text, modifiers).with_phase(phase)
        }
        _ => day_spec::KeyEvent::named(key, modifiers).with_phase(phase),
    }
}

/// Measure one string in the control font with no wrap limit. The expression is day.css's `body`
/// font-size, named rather than copied — controls (`font: inherit`) render at that size, so
/// measuring at anything else would mis-size pickers. The shim measures with a real DOM element,
/// so `var(--day-text-scale)` resolves against the document exactly as it does for the control.
/// Hard line breaks in a text area's day-driven content — the growth unit its `measure` clamps
/// between the `min_lines`/`max_lines` hints.
fn content_lines(t: &str) -> u32 {
    (t.split('\n').count() as u32).max(1)
}

fn measure_str(txt: &str) -> Size {
    let css = format!("{} {SYSTEM_STACK}", scaled_rem(1.0));
    let mut out = [0.0f64; 2];
    unsafe {
        day_dom_measure_text(
            txt.as_ptr(),
            txt.len(),
            css.as_ptr(),
            css.len(),
            1.0e6,
            out.as_mut_ptr(),
        );
    };
    Size::new(out[0].ceil(), out[1].ceil())
}

fn emit(node: NodeId, event: Event) {
    SINK.with(|s| {
        if let Some(sink) = s.borrow().as_ref() {
            sink(node, event);
        }
    });
}

fn node_of(el: u32) -> Option<NodeId> {
    NODE_OF.with(|m| m.borrow().get(&el).copied())
}

fn remember(el: u32, id: NodeId) {
    NODE_OF.with(|m| m.borrow_mut().insert(el, id));
}

/// Realize fallback for a builtin arm whose props failed to downcast
/// ([`day_spec::props_of`] already reported it): the same visible `⟨kind⟩` label the
/// missing-renderer path builds, so one mismatched piece cannot take down the tree build.
fn realize_placeholder(kind: PieceKind, id: NodeId) -> DomHandle {
    let el = unsafe { day_dom_create(EL_LABEL) };
    text(el, &format!("⟨{kind}⟩"));
    class(el, "placeholder", true);
    remember(el, id);
    DomHandle(el)
}

// ---------------------------------------------------------------------------
// Fonts: FontSpec → a CSS font shorthand. The ramp is the Apple text-style ratios with Body = 1,
// and a step becomes a length through day.css's `--day-text-scale`: `calc(<step>rem * var(…))`.
// Naming the variable rather than baking a number in is what keeps one definition of the size —
// the stylesheet's — and lets it differ per form factor (docs/web.md): a desktop scale of 0.8125
// puts Body on 13px, one CSS pixel per Apple point, while a touch browser anchors `html` to
// `-apple-system-body` and takes a scale of 1, so every step lands on the iOS ramp and tracks the
// user's Dynamic Type setting. Either way `1rem` is the browser's own preference, so page zoom and
// a larger default font scale the whole UI.
//
// An explicit `System(pt)`/`Custom(_, pt)` size is NOT a ramp step: it means that many pixels at
// the default preference (the Apple pt == logical-px convention), so it skips the scale and stays
// in rem — still scaling with the browser preference, per docs/text.md.
// ---------------------------------------------------------------------------

const SYSTEM_STACK: &str =
    "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif";

/// The fixed-pitch counterpart of [`SYSTEM_STACK`], for `FontSpec::monospace`. `ui-monospace`
/// first so the browser's own fixed face wins where it has one; the generic `monospace` last so
/// there is always something fixed-pitch to fall back to.
const MONO_STACK: &str =
    "ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', monospace";

/// A ramp step as a CSS length: the step in rem, scaled by the stylesheet's `--day-text-scale`.
fn scaled_rem(step: f64) -> String {
    format!("calc({step}rem * var(--day-text-scale))")
}

fn font_rem(style: Font) -> (f64, u32) {
    match style {
        Font::LargeTitle => (2.0, 700),
        Font::Title => (1.625, 700),
        Font::Title2 => (1.25, 600),
        Font::Title3 => (1.125, 600),
        Font::Headline => (1.0, 600),
        Font::Subheadline => (0.875, 400),
        Font::Body => (1.0, 400),
        Font::Callout => (0.9375, 400),
        Font::Footnote => (0.8125, 400),
        Font::Caption => (0.75, 400),
        Font::Caption2 => (0.6875, 400),
        // An explicit point size means that many px at the default preference (matching the
        // Apple pt == logical-px convention) — expressed in rem so it still scales with the
        // browser preference, per docs/text.md's rule that custom sizes never opt out.
        Font::System(pt) | Font::Custom(_, pt) => (pt / 16.0, 400),
    }
}

fn weight_css(w: FontWeight) -> u32 {
    match w {
        FontWeight::UltraLight => 100,
        FontWeight::Thin => 200,
        FontWeight::Light => 300,
        FontWeight::Regular => 400,
        FontWeight::Medium => 500,
        FontWeight::Semibold => 600,
        FontWeight::Bold => 700,
        FontWeight::Heavy => 800,
        FontWeight::Black => 900,
    }
}

/// `font` CSS shorthand: `style weight size/line-height family`. The unitless line-height
/// (1.3, matching the old px ramp's ratio) rides the size, so it scales too.
fn font_css(f: &FontSpec) -> String {
    let (rem, default_weight) = font_rem(f.style);
    // A ramp step takes the stylesheet's scale; an explicit pt size is already a size.
    // `FontSpec::scale` multiplies either — the same relative-size idea CSS spells `em`, so it
    // folds into the rem figure rather than needing a property of its own.
    let rem = rem * f.scale;
    let size = match f.style {
        Font::System(_) | Font::Custom(_, _) => format!("{rem}rem"),
        _ => scaled_rem(rem),
    };
    let weight = f.weight.map_or(default_weight, weight_css);
    let italic = if f.italic { "italic " } else { "" };
    // A bundled family names itself first and falls back to the stack; `monospace` picks the
    // fixed-pitch stack, which is what a code run asks for.
    let stack = if f.monospace {
        MONO_STACK
    } else {
        SYSTEM_STACK
    };
    let family = match f.style {
        Font::Custom(name, _) => format!("'{name}', {stack}"),
        _ => stack.to_string(),
    };
    format!("{italic}{weight} {size}/1.3 {family}")
}

fn color_css(c: day_spec::Color) -> String {
    format!(
        "rgba({},{},{},{})",
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
        c.a
    )
}

/// An ink as a CSS color the browser itself resolves for the page's appearance (docs/color.md),
/// so a view follows a light/dark switch with no patch and no Rust code running: a fixed color
/// is `rgba(…)`, a pair `light-dark(…)`, a role the stylesheet's `var(--day-<role>)` (day.css
/// defines all sixteen for both appearances), and a role at an alpha that alpha over the role's
/// channels (relative color syntax, which replaces the role's own alpha exactly).
///
/// Engines without those functions get equivalents that still follow: a pair through the
/// stylesheet's appearance toggles (`--day-if-light`/`--day-if-dark`, day.css), and a role's alpha
/// through `color-mix` toward transparent, which multiplies the role's own alpha instead of
/// replacing it (the same on the opaque roles).
fn ink_css(ink: day_spec::Ink) -> String {
    use day_spec::Ink;
    match ink {
        Ink::Fixed(c) => color_css(c),
        Ink::Adaptive { light, dark } => pair_css(light, dark),
        Ink::System { role, alpha: None } => format!("var({})", role_var(role)),
        Ink::System {
            role,
            alpha: Some(a),
        } => {
            let (var, a) = (role_var(role), a.clamp(0.0, 1.0));
            if css_colors().1 {
                format!("rgb(from var({var}) r g b / {a})")
            } else {
                format!("color-mix(in srgb, var({var}) {}%, transparent)", a * 100.0)
            }
        }
    }
}

/// A light/dark pair as one CSS color (see [`ink_css`]).
fn pair_css(light: day_spec::Color, dark: day_spec::Color) -> String {
    let (l, d) = (color_css(light), color_css(dark));
    if l == d {
        l
    } else if css_colors().0 {
        format!("light-dark({l},{d})")
    } else {
        // Exactly one of the two toggles is the guaranteed-invalid `initial` (its fallback is
        // used) and the other is empty, so the value is one color and a space.
        format!("var(--day-if-light,{l}) var(--day-if-dark,{d})")
    }
}

/// The stylesheet's custom property for a role (day.css; the brief's table in docs/color.md).
fn role_var(role: day_spec::SystemColor) -> &'static str {
    use day_spec::SystemColor as R;
    match role {
        R::Accent => "--day-accent",
        R::Text => "--day-fg",
        R::SecondaryText => "--day-fg-2",
        R::Background => "--day-bg",
        R::Surface => "--day-surface",
        R::TertiaryText => "--day-fg-3",
        R::Placeholder => "--day-placeholder",
        R::Link => "--day-link",
        R::Separator => "--day-separator",
        R::ControlFill => "--day-control",
        R::GroupedBackground => "--day-grouped",
        R::ElevatedSurface => "--day-elevated",
        R::Selection => "--day-selection",
        R::Destructive => "--day-destructive",
        R::Success => "--day-success",
        R::Warning => "--day-warning",
    }
}

/// Whether the engine parses `light-dark()` and relative color syntax, asked once.
fn css_colors() -> (bool, bool) {
    thread_local! {
        static SUPPORT: std::cell::OnceCell<(bool, bool)> = const { std::cell::OnceCell::new() };
    }
    SUPPORT.with(|c| {
        *c.get_or_init(|| {
            let v = env("css_colors");
            (v.contains('d'), v.contains('r'))
        })
    })
}

fn apply_font(el: u32, f: &FontSpec) {
    s(el, "font", &font_css(f));
    // The `font` shorthand RESETS every `font-variant-*` longhand to normal, so only the
    // tabular case writes one back — after the shorthand, never before. The reset is also why
    // a re-applied font needs no explicit "normal": the shorthand already cleared a previous
    // tabular-nums. Skipping the redundant write keeps the style attribute serializing as the
    // one compact `font:` declaration — touching any longhand of the family makes the browser
    // spell out every longhand individually, empty values and all.
    // The numeric features share one property, so tabular and old-style figures go in one
    // declaration; small capitals are `smcp`, never synthesized from scaled capitals by Day.
    let numeric = match (f.tabular, f.oldstyle_figures) {
        (true, true) => "tabular-nums oldstyle-nums",
        (true, false) => "tabular-nums",
        (false, true) => "oldstyle-nums",
        (false, false) => "",
    };
    if !numeric.is_empty() {
        s(el, "font-variant-numeric", numeric);
    }
    if f.small_caps {
        s(el, "font-variant-caps", "small-caps");
        // A face without `smcp` draws its ordinary lowercase, as on every other toolkit,
        // rather than capitals the browser scales down.
        s(el, "font-synthesis-small-caps", "none");
    }
}

// ---------------------------------------------------------------------------
// JSON writer (tiny, escapes only what the shim needs — no serde dependency).
// ---------------------------------------------------------------------------

/// Build the nav-menu JSON (`{items:[{title, section?, icon?, tint?}], selected}`) the shim's
/// `day_dom_navmenu` consumes. Shared by `NAV_MENU` realize and the data-driven
/// `NavMenuPatch::Items` rebuild.
#[allow(clippy::too_many_arguments)]
fn navmenu_json(
    items: &[String],
    icons: &[Option<String>],
    tints: &[Option<day_spec::Ink>],
    badges: &[Option<String>],
    badge_icons: &[Option<String>],
    badge_tints: &[Option<day_spec::Ink>],
    sections: &[Option<String>],
    selected: Option<usize>,
) -> String {
    let mut json = String::from("{\"items\":[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        json.push_str("{\"title\":");
        json_str(&mut json, item);
        // The section heading that opens a group before this row (docs/navigation.md). It rides
        // the row rather than the list, so the shim draws it between rows and every index that
        // addresses rows stays Day's.
        if let Some(Some(section)) = sections.get(i) {
            json.push_str(",\"section\":");
            json_str(&mut json, section);
        }
        if let Some(Some(icon)) = icons.get(i) {
            json.push_str(",\"icon\":");
            json_str(&mut json, &image_url(icon));
            // The row's own icon tint (docs/vectors.md): the shim paints the mask with this
            // instead of currentColor.
            if let Some(Some(t)) = tints.get(i) {
                json.push_str(",\"tint\":");
                json_str(&mut json, &ink_css(*t));
            }
        }
        // The row's count (docs/navigation.md `badge`), drawn dim at the trailing edge.
        if let Some(Some(count)) = badges.get(i) {
            json.push_str(",\"count\":");
            json_str(&mut json, count);
        }
        // The trailing status glyph (docs/navigation.md), in the same shape as `icon`/`tint` so
        // the shim paints it with one code path at the other end of the row.
        if let Some(Some(badge)) = badge_icons.get(i) {
            json.push_str(",\"badgeIcon\":");
            json_str(&mut json, &image_url(badge));
            if let Some(Some(t)) = badge_tints.get(i) {
                json.push_str(",\"badgeTint\":");
                json_str(&mut json, &ink_css(*t));
            }
        }
        json.push('}');
    }
    json.push_str("],\"selected\":");
    json.push_str(&selected.map_or("-1".into(), |i| i.to_string()));
    json.push('}');
    json
}

/// Build the toolbar JSON the shim's `day_dom_toolbar` consumes (docs/toolbars.md). One spec
/// per install, the way the nav menu crosses: the shim rebuilds the whole strip, and targeted
/// updates go through `day_dom_toolbar_patch` so a search in progress is undisturbed.
///
/// `kind` mirrors the XAML serializer's one-letter vocabulary so the two stay readable together:
/// B button, T toggle, S sidebar toggle, M menu, F search field, L label, `-` separator,
/// `_` space, `>` flexible space.
/// A toolbar pull-down's item list, for the shim's popup (docs/toolbars.md): label, action
/// id, enabled and icon per command; separators as `{"sep":true}`; submenus recursive (the
/// shim inlines them, indented, like the composed context menu). Activation rides the same
/// `day_dom_toolbar_action` route a plain button takes — the ids are registered menu-action
/// ids, and that export emits `Event::MenuAction`.
fn menu_items_json(json: &mut String, items: &[day_spec::MenuItem]) {
    use std::fmt::Write as _;
    json.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        match item {
            day_spec::MenuItem::Separator => json.push_str("{\"sep\":true}"),
            day_spec::MenuItem::Submenu { label, items, .. } => {
                json.push_str("{\"label\":");
                json_str(json, label);
                json.push_str(",\"items\":");
                menu_items_json(json, items);
                json.push('}');
            }
            day_spec::MenuItem::Action {
                id,
                action,
                label,
                shortcut,
                enabled,
                checked,
                role,
                icon,
            } => {
                json.push_str("{\"label\":");
                json_str(json, label);
                json.push_str(",\"id\":");
                json.push_str(&action.to_string());
                // The app's own name for the item (`.id`, docs/menus.md), so a browser test
                // can address the row the way a dayscript `menu:` step does.
                if let Some(name) = id {
                    json.push_str(",\"name\":");
                    json_str(json, name);
                }
                // The accelerator, for the app menu's key route and the hint beside the row.
                // `sys` marks the roles whose keys the browser's own routes already carry
                // (the clipboard events, the shim's undo/select-all keys): shown, never
                // registered a second time.
                if let Some(sc) = shortcut {
                    json.push_str(",\"shortcut\":{\"key\":");
                    json_str(json, &sc.key);
                    let _ = write!(
                        json,
                        ",\"primary\":{},\"shift\":{},\"alt\":{},\"control\":{}}}",
                        sc.primary, sc.shift, sc.alt, sc.control
                    );
                }
                if matches!(
                    role,
                    Some(
                        day_spec::MenuRole::Cut
                            | day_spec::MenuRole::Copy
                            | day_spec::MenuRole::Paste
                            | day_spec::MenuRole::SelectAll
                            | day_spec::MenuRole::Undo
                            | day_spec::MenuRole::Redo
                    )
                ) {
                    json.push_str(",\"sys\":true");
                }
                json.push_str(",\"enabled\":");
                json.push_str(if *enabled { "true" } else { "false" });
                // Absent for a plain command; true/false makes the row checkable and says which
                // way, so the shim can reserve the mark's column for a whole run of choices.
                if let Some(on) = checked {
                    json.push_str(",\"checked\":");
                    json.push_str(if *on { "true" } else { "false" });
                }
                let url = match icon {
                    Some(day_spec::Icon::Image(name)) => Some(image_url(name)),
                    Some(day_spec::Icon::Symbol(sym)) => symbol_svg(*sym),
                    None => None,
                };
                if let Some(url) = url {
                    json.push_str(",\"icon\":");
                    json_str(json, &url);
                }
                json.push('}');
            }
        }
    }
    json.push(']');
}

/// Send the app menu to the shim's trailing toolbar button (docs/menus.md "The composed
/// presentation (web-dom)"): the model reduced to what the web can answer, as one JSON document
/// with the button's localized name. An empty web model removes the button.
fn install_app_menu(items: &[day_spec::MenuItem]) {
    use day_spec::MenuBarRole as B;
    // The standard slots in the desktops' order, and the stock Help menu for an app that
    // claimed none: that is where About and Check for Updates… live on every desktop whose
    // bar day-core assembles, and the web has no other place to put them. The other stock
    // menus are native selectors (Edit ▸ Cut, View ▸ Enter Full Screen) the web filter would
    // drop whole, so they are not asked for.
    let bar = day_core::menu::standard_menu_bar(
        items.to_vec(),
        &[B::File, B::Edit, B::View],
        &[B::Help],
        |role| match role {
            B::Help => day_core::menu::MenuBarStyle::Gnome.stock(B::Help),
            _ => None,
        },
    );
    let items = web_menu_items(&bar);
    let mut json = String::from("{\"label\":");
    json_str(
        &mut json,
        &day_localization::t(day_localization::key::day_menu),
    );
    json.push_str(",\"items\":");
    menu_items_json(&mut json, &items);
    if let Some(url) = symbol_svg(day_spec::Symbol::More) {
        json.push_str(",\"icon\":");
        json_str(&mut json, &url);
    }
    json.push('}');
    unsafe { day_dom_app_menu(json.as_ptr(), json.len()) };
}

/// The menu an app that installed no `app_menu` still gets: the registered Settings… command
/// (docs/settings.md), the item a desktop's default bar carries; [`install_app_menu`] adds the
/// stock Help menu's About and Check for Updates… the same way it does for an installed model.
fn registration_menu() -> Vec<day_spec::MenuItem> {
    let prefs = day_core::windows::preferences_action_id();
    if prefs == 0 {
        return Vec::new();
    }
    vec![day_spec::MenuItem::Action {
        id: None,
        action: prefs,
        label: String::new(),
        shortcut: None,
        enabled: true,
        checked: None,
        role: Some(day_spec::MenuRole::Preferences),
        icon: None,
    }]
}

/// The app menu reduced to what the web can answer (docs/menus.md "Standard roles", the web
/// column). A role whose meaning is the window or the process (Quit, Minimize, Fullscreen,
/// New Window, Close Window) has nothing behind it in a browser tab, and a role that lowered
/// without a day action (About with no registration, Delete) would be a row that does nothing:
/// both are left out. Every other item keeps its action id, so the popover dispatches through
/// `day_dom_toolbar_action` exactly as a toolbar pull-down does. A role item lowered with an
/// empty label (day-core's injected Settings…/About) takes its catalog label here, as every
/// backend does. Settings… loses its ⌘, / Ctrl+, shortcut: the browser answers that key with its
/// own settings before the page sees it, so the hint would promise what the key never does.
/// Submenus left empty by the pruning go, and separators never lead, trail, or
/// double up.
fn web_menu_items(items: &[day_spec::MenuItem]) -> Vec<day_spec::MenuItem> {
    use day_spec::{MenuItem, MenuRole};
    let mut out: Vec<MenuItem> = Vec::new();
    for item in items {
        match item {
            MenuItem::Separator => {
                if !matches!(out.last(), None | Some(MenuItem::Separator)) {
                    out.push(MenuItem::Separator);
                }
            }
            MenuItem::Submenu { label, items, role } => {
                let items = web_menu_items(items);
                if !items.is_empty() {
                    out.push(MenuItem::Submenu {
                        label: label.clone(),
                        items,
                        role: *role,
                    });
                }
            }
            MenuItem::Action {
                role: Some(role),
                action,
                ..
            } if *action == 0
                || matches!(
                    role,
                    MenuRole::Quit
                        | MenuRole::Minimize
                        | MenuRole::Fullscreen
                        | MenuRole::NewWindow
                        | MenuRole::CloseWindow
                ) => {}
            MenuItem::Action { .. } => {
                let mut item = item.clone();
                if let MenuItem::Action {
                    label,
                    shortcut,
                    role: Some(role),
                    ..
                } = &mut item
                {
                    if label.is_empty() {
                        *label = day_localization::t(role_catalog_key(*role));
                    }
                    if *role == MenuRole::Preferences {
                        *shortcut = None;
                    }
                }
                out.push(item);
            }
        }
    }
    while matches!(out.last(), Some(MenuItem::Separator)) {
        out.pop();
    }
    out
}

/// The core-catalog key for a standard menu command's label (docs/localization.md), the same
/// table the pieces layer resolves `menu_role` labels from.
fn role_catalog_key(role: day_spec::MenuRole) -> &'static str {
    use day_spec::MenuRole as R;
    match role {
        R::Cut => "day_cut",
        R::Copy => "day_copy",
        R::Paste => "day_paste",
        R::SelectAll => "day_select_all",
        R::Undo => "day_undo",
        R::Redo => "day_redo",
        R::Delete => "day_delete",
        R::About => "day_about",
        R::Quit => "day_quit",
        // The web says "Settings", like macOS, Windows and the phones (docs/settings.md);
        // "Preferences" is the GNOME/KDE word.
        R::Preferences => "day_settings",
        R::Minimize => "day_minimize",
        R::CloseWindow => "day_close",
        R::Fullscreen => "day_fullscreen",
        R::NewWindow => "day_new_window",
        R::CheckForUpdates => "day_check_for_updates",
    }
}

/// A toolbar edit (docs/toolbars.md) as the shim's `day_dom_toolbar_edit` reads it: the ops in
/// order, each insert carrying its item.
fn toolbar_edit_json(ops: &[day_spec::ToolbarOp]) -> String {
    let mut json = String::from("{\"ops\":[");
    for (i, op) in ops.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        match op {
            day_spec::ToolbarOp::Remove { id } => {
                json.push_str("{\"op\":\"remove\",\"id\":");
                json_str(&mut json, id);
                json.push('}');
            }
            day_spec::ToolbarOp::Insert { index, item } => {
                json.push_str("{\"op\":\"insert\",\"index\":");
                json.push_str(&index.to_string());
                json.push_str(",\"item\":");
                toolbar_item_json(&mut json, item);
                json.push('}');
            }
        }
    }
    json.push_str("]}");
    json
}

fn toolbar_item_json(json: &mut String, it: &day_spec::ToolbarItem) {
    use day_spec::ToolbarItemKind as K;
    {
        let kind = match &it.kind {
            K::Button => "B",
            K::Toggle { .. } => "T",
            K::Menu { .. } => "M",
            K::Search { .. } => "F",
            K::Segmented { .. } => "G",
            K::Label => "L",
            K::Separator => "-",
        };
        json.push_str("{\"kind\":");
        json_str(json, kind);
        json.push_str(",\"id\":");
        json_str(json, &it.id);
        json.push_str(",\"label\":");
        json_str(json, &it.label);
        json.push_str(",\"tip\":");
        json_str(json, it.tooltip.as_deref().unwrap_or(&it.label));
        json.push_str(",\"action\":");
        json.push_str(&it.action.to_string());
        json.push_str(",\"enabled\":");
        json.push_str(if it.enabled { "true" } else { "false" });
        // Placement and column (docs/toolbars.md). Day draws this strip itself, so it can honor
        // both: the columns become three tracks whose widths follow the split's own panes, and
        // within each the roles pack leading, center and trailing.
        json.push_str(",\"place\":");
        json_str(
            json,
            match it.placement {
                day_spec::ToolbarPlacement::Navigation => "nav",
                day_spec::ToolbarPlacement::Principal => "mid",
                day_spec::ToolbarPlacement::Primary => "primary",
                day_spec::ToolbarPlacement::Secondary | day_spec::ToolbarPlacement::Bottom => {
                    "secondary"
                }
                day_spec::ToolbarPlacement::Automatic => "auto",
            },
        );
        json.push_str(",\"col\":");
        json_str(
            json,
            match it.column {
                day_spec::ToolbarColumn::Sidebar => "sidebar",
                day_spec::ToolbarColumn::List => "list",
                day_spec::ToolbarColumn::Detail | day_spec::ToolbarColumn::Window => "detail",
            },
        );
        if let K::Toggle { on } = it.kind {
            json.push_str(",\"on\":");
            json.push_str(if on { "true" } else { "false" });
        }
        if let K::Segmented { segments, selected } = &it.kind {
            json.push_str(",\"selected\":");
            json.push_str(&selected.to_string());
            json.push_str(",\"segments\":[");
            for (n, seg) in segments.iter().enumerate() {
                if n > 0 {
                    json.push(',');
                }
                json.push_str("{\"title\":");
                json_str(json, &seg.title);
                let ic = match &seg.icon {
                    Some(day_spec::Icon::Image(name)) => Some(image_url(name)),
                    Some(day_spec::Icon::Symbol(sym)) => symbol_svg(*sym),
                    None => None,
                };
                if let Some(url) = ic {
                    json.push_str(",\"icon\":");
                    json_str(json, &url);
                }
                json.push('}');
            }
            json.push(']');
        }
        if let K::Menu { items } = &it.kind {
            json.push_str(",\"menu\":");
            menu_items_json(json, items);
        }
        if let K::Search {
            text,
            placeholder,
            suggestions,
        } = &it.kind
        {
            json.push_str(",\"text\":");
            json_str(json, text.as_str());
            json.push_str(",\"placeholder\":");
            json_str(json, placeholder.as_str());
            // A native <datalist>, so the browser draws the completion popup.
            json.push_str(",\"suggestions\":[");
            for (n, sug) in suggestions.iter().enumerate() {
                if n > 0 {
                    json.push(',');
                }
                json_str(json, sug);
            }
            json.push(']');
        }
        // A bundled image crosses as a staged URL; a standard symbol as an inline-SVG data URL
        // (`symbol_svg`). Both reach the shim as one `icon` field and draw through the same CSS
        // mask, so a bar mixing the two looks like one bar.
        let icon = match &it.icon {
            Some(day_spec::Icon::Image(name)) => Some(image_url(name)),
            Some(day_spec::Icon::Symbol(s)) => symbol_svg(*s),
            // The sidebar toggle carries no icon from the app — every other toolkit draws its
            // own platform glyph for it (docs/toolbars.md). The web has none to draw, so it takes
            // the standard one here rather than being the one item in the bar showing bare text.
            None => None,
        };
        if let Some(url) = icon {
            json.push_str(",\"icon\":");
            json_str(json, &url);
        }
        json.push('}');
    }
}

fn json_str(out: &mut String, v: &str) {
    use std::fmt::Write as _;
    out.push('"');
    for ch in v.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

// ---------------------------------------------------------------------------
// The toolkit
// ---------------------------------------------------------------------------

thread_local! {
    /// Renderers registered by external Day Piece crates (§8.2, docs/extending.md).
    ///
    /// Every other backend exposes this seam as a `linkme` distributed slice populated at link
    /// time. **That mechanism does not exist on wasm** — `#[distributed_slice]` fails to compile
    /// for `wasm32-unknown-unknown` with "distributed_slice is not implemented for this platform"
    /// (checked against linkme 0.3.37) — so web-dom registers at RUNTIME instead, and a piece
    /// self-registers the first time its constructor runs. That happens before its node is
    /// realized, so the renderer is always in place by the time it is needed.
    ///
    /// Without this seam a piece could not render on the web at all: `realize` receives `&dyn Any`
    /// props whose concrete type lives in the piece crate, so only the piece itself can downcast
    /// them — day-toolkit-dom cannot special-case a piece it does not (and must not) depend on.
    static REGISTRY: RefCell<Registry<Dom>> = RefCell::new(Registry::default());
}

/// Register an external piece's web renderer. Idempotent: a piece calls this from its constructor,
/// which may run many times.
pub fn register_renderer(make: fn() -> Renderer<Dom>) {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let renderer = make();
        if r.get(renderer.kind).is_none() {
            r.register(renderer);
        }
    });
}

/// Look up a registered renderer's `make`/`update`/`measure`, if any.
fn registered<T>(kind: PieceKind, f: impl FnOnce(&Renderer<Dom>) -> T) -> Option<T> {
    REGISTRY.with(|r| r.borrow().get(kind).map(f))
}

pub struct Dom {
    root: u32,
}

impl Dom {
    #[allow(clippy::new_without_default)]
    #[must_use]
    pub fn new() -> Self {
        controls::register();
        // The two launch overrides a process environment would carry (docs/accessibility.md),
        // read from the page's query string instead.
        #[cfg(target_arch = "wasm32")]
        day_core::testing::init_fast_animations(
            host_env("DAY_TEST_FAST").as_deref() == Some("1")
                || host_env("DAY_REDUCE_MOTION").as_deref() == Some("1"),
        );
        #[cfg(target_arch = "wasm32")]
        day_core::init_presentation_mode(
            if host_env("DAY_TEST_DIALOGS").as_deref() == Some("scripted") {
                day_core::PresentationMode::Scripted
            } else {
                day_core::PresentationMode::Native
            },
        );
        Dom { root: 1 }
    }

    /// Create a DOM element of `tag` for a piece renderer, returning its handle.
    ///
    /// The public helper surface a `lib-dom.rs` builds on: piece crates cannot reach day-toolkit-dom's
    /// private shim imports, so element creation, attributes and method calls go through these.
    pub fn element(&mut self, tag: &str) -> DomHandle {
        DomHandle(unsafe { day_dom_create_tag(tag.as_ptr(), tag.len()) })
    }

    /// Set an attribute. The boolean-attribute convention applies: `""` removes, `"-"` sets
    /// (see the shim's `day_dom_set_attr`).
    pub fn set_attr(&mut self, h: &DomHandle, name: &str, value: &str) {
        attr(h.0, name, value);
    }

    /// Call a zero-argument method on the element (`play`, `pause`, `load`, …).
    pub fn call(&mut self, h: &DomHandle, method: &str) {
        unsafe { day_dom_call(h.0, method.as_ptr(), method.len()) };
    }

    /// Replace the element's markup, KEEPING the caret and the selection where they were
    /// (docs/texteditor.md) — which is what lets a styled-text editor repaint its attributes on
    /// every keystroke without fighting the user.
    ///
    /// `html` must come from Day's own serializer ([`day_spec::styled_to_html`]), which escapes
    /// every character of app text; this is not a hole to hand arbitrary markup through.
    pub fn set_html(&mut self, h: &DomHandle, html: &str) {
        unsafe { day_dom_set_html(h.0, html.as_ptr(), html.len()) };
    }

    /// Select a BYTE range of a contenteditable element's flattened text — the same offsets
    /// [`listen::EDITABLE`] reports.
    pub fn set_editor_selection(&mut self, h: &DomHandle, start: usize, end: usize) {
        unsafe { day_dom_editor_select(h.0, start as u32, end as u32) };
    }

    /// Insert `text` at a BYTE offset of a contenteditable element's flattened text, as typing
    /// would: through the browser's editing command, so its undo stack records the insertion
    /// and the element's `input` event reports the result (docs/texteditor.md).
    pub fn insert_editor_text(&mut self, h: &DomHandle, at: usize, text: &str) {
        unsafe { day_dom_editor_insert(h.0, at as u32, text.as_ptr(), text.len()) };
    }

    /// Attach the shim's DOM listeners to a piece's element, so it can report back.
    ///
    /// The built-in kinds get this from their own `realize` arms; a piece renderer that needs
    /// events (`day-piece-colorpicker`'s `<input type="color">`) asks here, with a mask built
    /// from [`listen`]'s constants. The element the piece's `make` returned is already bound to
    /// its `NodeId`, so what the shim reports arrives at the piece's `cx.on` as the ordinary
    /// [`Event`] for that bit — `listen::INPUT` on an `<input>` becomes `Event::TextChanged`.
    ///
    /// Idempotent it is NOT: each call adds listeners, so call it once, from `make`.
    pub fn listen(&mut self, h: &DomHandle, mask: u32) {
        unsafe { day_dom_listen(h.0, mask) };
    }
}

/// Listener bits for [`Dom::listen`] — which DOM events the shim wires to a piece's element, and
/// the [`Event`] each becomes. The numbers are the shim's own table (`listen(id, mask)` in
/// `shim.js`); naming them keeps a piece from passing a magic constant that silently means
/// something else after a shim change.
pub mod listen {
    // Nothing here names `Event` in code — the import is what makes the `[`Event::…`]` links in
    // the constants' docs below resolve to day-spec's type instead of rendering as plain text.
    #[allow(unused_imports)]
    use day_spec::Event;
    /// `click` → [`Event::Pressed`].
    pub const CLICK: u32 = 1;
    /// `input` → [`Event::TextChanged`] with the element's value (or [`Event::ValueChanged`] on a
    /// `type="range"`).
    pub const INPUT: u32 = 2;
    /// `change` → [`Event::ToggleChanged`] on a checkbox, [`Event::SelectionChanged`] on a
    /// `<select>`, [`Event::ValueCommitted`] on a range.
    pub const CHANGE: u32 = 4;
    /// `focus`/`blur` → [`Event::FocusChanged`].
    pub const FOCUS: u32 = 8;
    /// Enter `keydown` → [`Event::Submitted`].
    pub const SUBMIT: u32 = 16;
    /// A `ResizeObserver` → [`Event::FrameChanged`].
    pub const RESIZE: u32 = 32;
    /// `scroll` → [`Event::ScrollChanged`].
    pub const SCROLL: u32 = 64;
    /// A press released within the slop → [`Event::Tap`] (at the press point, on release —
    /// the shim's recognizer, matching the native toolkits).
    pub const POINTER: u32 = 128;
    /// The pointer-capture trio → [`Event::Drag`].
    pub const DRAG: u32 = 256;
    /// A `contenteditable` element's editing listeners (docs/texteditor.md): `input` (and an IME
    /// `compositionend`) → [`Event::TextChanged`] carrying the FLATTENED text, and
    /// `selectionchange` → [`Event::Custom`] carrying `"sel <start> <end>"` in byte offsets.
    pub const EDITABLE: u32 = 512;
}

impl Toolkit for Dom {
    type Handle = DomHandle;

    /// `navigator.setAppBadge(n)` / `clearAppBadge()`. A dot is the no-argument form, which is why
    /// the count is encoded: negative clears, zero means "dot", positive is the number.
    fn set_app_badge(&mut self, badge: &day_spec::AppBadge) {
        use day_spec::AppBadge;
        let encoded: i32 = match badge {
            AppBadge::None | AppBadge::Count(0) => -1,
            AppBadge::Count(n) => (*n).min(i32::MAX as u32) as i32,
            AppBadge::Dot => 0,
            // No text badge on the web; clearing would be worse than leaving the last value, so
            // this is the one payload the arm ignores outright.
            AppBadge::Text(_) => return,
        };
        unsafe { day_dom_set_app_badge(encoded) };
    }

    /// The page's one window: fullscreen through the Fullscreen API, Normal by leaving it.
    /// Minimize and maximize are not the page's to ask for, so they are declined at once by
    /// reporting the state the page is in. Content protection has no web mechanism.
    fn apply_window(&mut self, _host: &DomHandle, change: &day_spec::WindowChange) {
        use day_spec::{WindowChange, WindowState};
        let state = match change {
            WindowChange::State(state) => state,
            // The page is the app's one window, so its own appearance is the page's: the
            // override replaces the app-wide one until it is cleared.
            WindowChange::Appearance(dark) => {
                APPEARANCE.with(|a| a.set((a.get().0, *dark)));
                apply_appearance();
                // day-core's dark signal re-reads `dark_mode()`, which needs the tree this call
                // is borrowed from: one hop later.
                post_local(day_core::note_appearance_changed);
                return;
            }
            // A page cannot move, resize, restack, hide or flash the browser window: `moveTo`
            // and `resizeTo` work only on popups the script opened.
            _ => return,
        };
        let answer = match state {
            WindowState::Fullscreen => unsafe { day_dom_set_fullscreen(1) },
            WindowState::Normal => unsafe { day_dom_set_fullscreen(0) },
            WindowState::Minimized | WindowState::Maximized => {
                WINDOW_STATE.with(std::cell::Cell::get).code()
            }
        };
        // -1: the browser has the request; `fullscreenchange` or the rejection reports back.
        if answer >= 0 {
            report_window_state(WindowState::from_code(answer), true);
        }
    }

    /// `app-region: drag` on the element and `no-drag` on the controls inside it, through the
    /// rules `day_dom_window_chrome` installed. Live only in an installed app's window-controls
    /// overlay, where Chromium moves the window and maximizes it on a double click.
    /// `screenX`/`screenY`/`outerWidth`/`outerHeight`: the browser window's outer frame on the
    /// desktop, in CSS px (points at 100% zoom).
    fn window_frame(&self, _host: &DomHandle) -> Option<Rect> {
        let mut f = [0f64; 4];
        let ok = unsafe { day_dom_window_frame(f.as_mut_ptr()) } != 0;
        (ok && f[2] > 0.0 && f[3] > 0.0).then(|| Rect::new(f[0], f[1], f[2], f[3]))
    }

    /// The one screen the page is on, from `window.screen`. The Window Management API would
    /// list them all, but only asynchronously and behind a permission prompt.
    fn monitors(&self) -> Vec<day_spec::Monitor> {
        let mut v = [0f64; 9];
        unsafe { day_dom_screen(v.as_mut_ptr()) };
        if v[2] <= 0.0 || v[3] <= 0.0 {
            return Vec::new();
        }
        vec![day_spec::Monitor {
            id: "0".into(),
            name: String::new(),
            frame: Rect::new(v[0], v[1], v[2], v[3]),
            work_area: Rect::new(v[4], v[5], v[6], v[7]),
            scale: if v[8] > 0.0 { v[8] } else { 1.0 },
            primary: true,
        }]
    }

    fn set_drag_region(&mut self, h: &DomHandle, drag: bool) {
        class(h.0, "day-drag-region", drag);
        if drag {
            unsafe { day_dom_takes_pointer(h.0) };
        }
    }

    fn set_drag_source(&mut self, h: &DomHandle, source: day_spec::transfer::Source) {
        transfer::source(h.0, source);
    }
    fn set_drop_target(&mut self, h: &DomHandle, target: day_spec::transfer::Target) {
        transfer::target(h.0, target);
    }

    fn set_presented(&mut self, handle: &Self::Handle, visible: bool) {
        attr(
            handle.0,
            "data-day-presented",
            if visible { "true" } else { "false" },
        );
    }

    #[expect(
        clippy::match_same_arms,
        reason = "one arm per capability, each documented with the browser mechanism behind it"
    )]
    fn capability(&self, cap: Cap) -> Support {
        match cap {
            Cap::SearchField => Support::Native,
            Cap::ComboBox => Support::Native,
            Cap::Stepper => Support::Emulated,
            Cap::DateTimePicker => Support::Native,
            Cap::ColorPicker => Support::Native,
            Cap::PullRefresh => Support::Emulated,
            Cap::Pager => Support::Emulated,
            Cap::PullRefreshGesture => Support::Unsupported,
            // `requestFullscreen` on the document element. A page in an iframe without
            // `allow="fullscreen"`, and Safari on iPhone, have no element fullscreen, which
            // `document.fullscreenEnabled` says. The browser grants the request only during a
            // user gesture; a button action runs inside the click handler, so a press qualifies,
            // and a refused request reads back as the state the page kept.
            Cap::WindowFullscreen => {
                if fullscreen_enabled() {
                    Support::Native
                } else {
                    Support::Unsupported
                }
            }
            // A page cannot minimize or zoom the browser window, and nothing keeps it out of a
            // screenshot or a shared tab.
            Cap::WindowStates | Cap::ContentProtection => Support::Unsupported,
            // Window Controls Overlay (docs/window-chrome.md): only an installed Chromium app on
            // the desktop, and only once the user hides its title bar, runs content under the
            // overlay and honors `app-region: drag`. Emulated where the API exists at all.
            Cap::OverlayTitleBar | Cap::DragRegion => {
                if window_controls_overlay() {
                    Support::Emulated
                } else {
                    Support::Unsupported
                }
            }
            // The browser owns the window's frame and what is behind it.
            Cap::FramelessWindow | Cap::TransparentWindow | Cap::WindowMaterial => {
                Support::Unsupported
            }
            // `window.screen`: the one screen the page is on.
            Cap::Monitors => Support::Emulated,
            // The page is the app's one window, so its appearance is the app's: the override
            // goes through the same path as `set_appearance`.
            Cap::WindowAppearance => Support::Emulated,
            // Browsers resize and move only the popups a script opened; nothing stacks a tab
            // above other apps or flashes it. The manifest's `shortcuts` are static: day-cli
            // writes the Day.toml ones, and a page cannot add any while it runs.
            Cap::WindowGeometry
            | Cap::WindowPosition
            | Cap::WindowLevel
            | Cap::RequestAttention
            | Cap::DynamicShortcuts => Support::Unsupported,
            // Composed, not read: the CSS generic families plus the bundled `document.fonts`
            // faces. A browser lists local fonts only through `queryLocalFonts()` — Chromium
            // only, asynchronous, behind a permission prompt — so the list is what CSS can
            // name for certain (docs/fonts.md).
            Cap::FontList => Support::Emulated,
            // An inline `cursor` style per element (docs/cursor.md); a coarse pointer never shows it.
            Cap::Cursor | Cap::DragDrop | Cap::DragExternalImport | Cap::DragMultipleItems => {
                Support::Native
            }
            // A statement about the toolkit, not about the current window: web-dom can always
            // draw two panes. Whether a given host does follows from the window's size class,
            // which `run`/`day_dom_resized` report and the pieces layer resolves against
            // (docs/size-classes.md).
            Cap::NavSplit => Support::Native,
            // A re-present is a DOM re-home: the shim rebuilds the host's chrome and the page
            // elements move between containers intact, keeping their subtrees, scroll offsets,
            // and focus (docs/size-classes.md).
            Cap::NavRepresent => Support::Native,
            // Composed rather than a native tab widget — the browser ships none — but a tab
            // bar all the same: the same `NAV_MENU` element the sidebar uses, laid out
            // horizontally in the chrome slot (docs/navigation.md).
            Cap::NavTabs => Support::Emulated,
            // A browser window ranges from a phone to a desktop, which is exactly the case
            // adaptive navigation exists for — so unlike the desktop toolkits, web-dom does
            // grow a tab bar as the viewport narrows (docs/navigation.md).
            Cap::NavTabsAdaptive => Support::Emulated,
            Cap::Appearance | Cap::Dialogs | Cap::Animation => Support::Native,
            // The `prefers-reduced-motion` media query, read and watched by the shim
            // (docs/accessibility.md).
            Cap::ReduceMotion => Support::Native,
            // `createImageBitmap` decodes every format the engine reads; a canvas writes back
            // PNG/JPEG/WebP (docs/images.md). `Cap::ImageProperties` is deliberately NOT here:
            // the browser exposes no metadata reader, and an empty struct would read as "this
            // file records nothing" rather than "nobody looked".
            Cap::ImageDecode | Cap::ImageEncode => Support::Native,
            // A drawing exports through the on-screen painter into a detached `<canvas>`, kept
            // as an `ImageBitmap` (docs/canvas.md "Exporting"). No PDF: a browser writes none.
            Cap::CanvasBitmap => Support::Native,
            // The browser's file input and download are its file dialogs; bytes ride the
            // `web_files` store instead of a filesystem (docs/files.md).
            Cap::FileDialogs => Support::Native,
            // The browser's own clipboard events (⌘X/C/V) are the native edit route; the
            // shim's document listeners forward them when no editable element claims them
            // (docs/menus.md).
            Cap::EditBridge => Support::Native,
            // Exact metrics from a canvas TextMetrics, but derived rather than read off a
            // baseline the platform publishes (docs/baseline.md).
            Cap::BaselineAlignment => Support::Emulated,
            // Runs are `<span>` children; a link run is an `<a>`, whose click the shim cancels
            // and reports so the app's `.on_link()` decides (docs/text-runs.md).
            Cap::TextRuns | Cap::TextLinks => Support::Native,
            // A strip docked above the app root, not window chrome the OS draws — a browser tab
            // has no title bar to hang one on. Emulated is the honest answer, and it is enough
            // for an app to decide the commands belong in the bar rather than in the content
            // (docs/toolbars.md).
            Cap::Toolbar | Cap::ToolbarSearch => Support::Emulated,
            // The app menu is a pull-down on that same strip, presented by the shim's popover
            // rather than a menu bar the browser has no way to draw (docs/menus.md). Emulated
            // says the commands are reachable, which is what an app probing the capability
            // wants to know before it duplicates Settings… or About somewhere in its content.
            Cap::AppMenu => Support::Emulated,
            // The Badging API takes a number or, with no argument, a dot. Emulated rather than
            // Native because whether anything is DRAWN depends on the browser and on the page
            // being an installed app — the call itself always succeeds (docs/badge.md).
            Cap::AppBadgeCount | Cap::AppBadgeDot => Support::Emulated,
            // An ARIA live region the shim owns, not an announcement request: the browser
            // has none (docs/accessibility.md).
            Cap::Announce => Support::Emulated,
            Cap::TextEditable | Cap::TextSelectable | Cap::TextSpellCheck => Support::Native,
            Cap::ListRecycling => Support::Emulated,
            // The rows are absolutely framed, so a header row cannot be `position: sticky`
            // itself: the host keeps one extra cell in a sticky strip at the top of the list
            // and binds the current section's header into it (docs/list.md "Sections").
            Cap::ListSections => Support::Emulated,
            // The COMPOSED tree (docs/tree.md M2): the piece flattens onto the emulated
            // list; disclosure, indentation, and row menus are day pieces. No native drag,
            // so `Cap::TreeMove` stays Unsupported.
            Cap::Tree | Cap::TreeSections => Support::Emulated,
            // Pointer-tracked drag with a CSS gap — the browser has no native list reorder.
            Cap::NavReorder | Cap::ListReorder => Support::Emulated,
            // A topmost fixed-position child — not a system modal (docs/cover.md).
            Cap::Cover => Support::Emulated,
            // A `tabindex` stop with the key route, focused by a press in the capture phase,
            // `:focus-visible` for the ring (docs/focus.md).
            Cap::Focusable => Support::Native,
            // `KeyboardEvent.repeat` and `keyup` (docs/menus.md "Keys").
            Cap::KeyPhases => Support::Native,
            // A grown pseudo-element, or a forwarding sibling where the element has none
            // (docs/gestures.md "Hit slop").
            Cap::HitSlop => Support::Native,
            // The document's `selectionchange`, mapped to the label's text (docs/text.md).
            Cap::LabelSelection => Support::Native,
            // The list reports its visible rows and end distance from its own row table on
            // every scroll, resize and data change, and scrolls rows to any alignment with the
            // browser's own smooth scrolling (docs/list.md).
            Cap::ListViewport | Cap::ListScrollAlign => Support::Native,
            // A list scroll under a finger, with the scroll bar held, or right after a wheel or
            // a key that scrolls is the reader's; `scrollTo` comes with none (docs/list.md
            // "Following the end").
            Cap::ListReaderScroll => Support::Native,
            // A scroll view reports its offset per frame and at rest, and the user's phases from
            // touch, wheel, keyboard and scroll-bar input with `scrollend` (docs/scroll.md).
            Cap::ScrollOffset | Cap::ScrollPhases => Support::Native,
            // CSS scroll snap on markers at each stop (`scroll-snap-type: mandatory`).
            Cap::ScrollSnap => Support::Native,
            // Blurring the field dismisses a touch screen's keyboard: the scroll piece resigns
            // focus when a drag begins. A desktop browser has no software keyboard.
            Cap::ScrollKeyboardDismiss => {
                if unsafe { day_dom_touch_points() } > 0 {
                    Support::Emulated
                } else {
                    Support::Unsupported
                }
            }
            _ => Support::Unsupported,
        }
    }

    fn realize(&mut self, kind: PieceKind, props: &dyn std::any::Any, id: NodeId) -> DomHandle {
        let el = match Builtin::from_key(kind) {
            Some(Builtin::Container) => {
                // Here and in every arm below: a props-type mismatch degrades to the visible
                // placeholder label (`props_of` reported it) instead of panicking out of the
                // wasm export that drove this realize.
                let Some(p) = day_spec::props_of::<ContainerProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let el = unsafe { day_dom_create(EL_DIV) };
                if let Some(role) = p.role {
                    SURFACE_ROLES.with(|t| t.insert(el as usize, role));
                }
                apply_surface(el, p, p.role, None, true);
                // Pass-through until something listens (docs/gestures.md "Pass-through").
                class(el, "day-passive-host", true);
                el
            }
            Some(Builtin::Label) => {
                let Some(p) = day_spec::props_of::<LabelProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let el = unsafe { day_dom_create(EL_LABEL) };
                class(el, "day-passive-leaf", true);
                set_label_text(el, &p.text, &p.runs);
                apply_font(el, &p.font);
                if let Some(c) = p.color {
                    s(el, "color", &ink_css(c));
                } else if p.role == day_spec::props::TextRole::Secondary {
                    // No system "secondary label" on the web, so it is derived from the text color
                    // in force — which is what the light/dark switch already drives. Mixing toward
                    // transparent rather than naming a grey is what keeps it correct on both.
                    s(
                        el,
                        "color",
                        "color-mix(in srgb, currentColor 62%, transparent)",
                    );
                }
                if !p.wraps {
                    s(el, "white-space", "nowrap");
                    s(el, "overflow", "hidden");
                    s(el, "text-overflow", "ellipsis");
                }
                // `start`/`end` rather than `left`/`right`, so an RTL locale follows the writing
                // direction without the app asking (docs/localization.md).
                match p.align {
                    day_spec::props::TextAlign::Leading => {}
                    // Every line but the last stretched edge to edge; the last at the start.
                    day_spec::props::TextAlign::Justified => s(el, "text-align", "justify"),
                    day_spec::props::TextAlign::Center => s(el, "text-align", "center"),
                    day_spec::props::TextAlign::Trailing => s(el, "text-align", "end"),
                }
                LABELS.with(|t| {
                    t.borrow_mut().insert(
                        el,
                        LabelState {
                            text: p.text.clone(),
                            runs: !p.runs.is_empty(),
                            wraps: p.wraps,
                            para: p.paragraph(),
                            width: 0.0,
                            elided: false,
                            selected: false,
                        },
                    );
                });
                apply_paragraph(el);
                // Selection reports (docs/text.md "Selection"): the shim's `selectionchange`
                // route reports ranges inside elements carrying this flag.
                if p.reports_selection {
                    attr(el, "data-day-reports-selection", "true");
                }
                el
            }
            Some(Builtin::Button) => {
                let Some(p) = day_spec::props_of::<ButtonProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let el = unsafe { day_dom_create(EL_BUTTON) };
                if p.icon.is_some() {
                    apply_button_content(el, &p.title, p.icon.as_ref(), p.icon_only);
                } else {
                    text(el, &p.title);
                }
                apply_button_style(el, p.style);
                set_button_selected(el, p.selected);
                if !p.enabled {
                    attr(el, "disabled", "-");
                }
                unsafe { day_dom_listen(el, 1) };
                el
            }
            Some(Builtin::Toggle) => {
                let Some(p) = day_spec::props_of::<ToggleProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let el = unsafe { day_dom_create(EL_TOGGLE) };
                unsafe { day_dom_set_checked(el, u32::from(p.on)) };
                if !p.enabled {
                    attr(el, "disabled", "-");
                }
                unsafe { day_dom_listen(el, 4) };
                el
            }
            Some(Builtin::Slider) => {
                let Some(p) = day_spec::props_of::<SliderProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let el = unsafe { day_dom_create(EL_SLIDER) };
                attr(el, "min", &p.min.to_string());
                attr(el, "max", &p.max.to_string());
                attr(el, "step", &p.step.map_or("any".into(), |s| s.to_string()));
                unsafe { day_dom_set_value(el, p.value) };
                if !p.enabled {
                    attr(el, "disabled", "-");
                }
                // 2 = `input` (live, every motion) | 4 = `change` (settled, on release) — the
                // two facts a drag produces, which the DOM already separates for us.
                unsafe { day_dom_listen(el, 2 | 4) };
                el
            }
            Some(Builtin::TextField) => {
                let Some(p) = day_spec::props_of::<TextFieldProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let el = unsafe { day_dom_create(EL_FIELD) };
                attr(el, "value", &p.text);
                attr(el, "placeholder", &p.placeholder);
                if !p.enabled {
                    attr(el, "disabled", "-");
                }
                unsafe { day_dom_listen(el, 2 | 8 | 16) };
                el
            }
            Some(Builtin::TextArea) => {
                let Some(p) = day_spec::props_of::<TextAreaProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let el = unsafe { day_dom_create(EL_AREA) };
                text(el, &p.text);
                attr(el, "placeholder", &p.placeholder);
                apply_area_attrs(el, p.editable, p.selectable, p.spellcheck);
                AREA_HINTS.with(|m| {
                    m.borrow_mut()
                        .insert(el, (p.min_lines, p.max_lines, content_lines(&p.text)))
                });
                unsafe { day_dom_listen(el, 2 | 8) };
                el
            }
            Some(Builtin::Picker) => {
                let Some(p) = day_spec::props_of::<PickerProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                realize_picker(p)
            }
            Some(Builtin::Progress) => {
                let Some(p) = day_spec::props_of::<ProgressProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let el = unsafe {
                    day_dom_create(if p.value.is_some() {
                        EL_PROGRESS
                    } else {
                        EL_SPINNER
                    })
                };
                if let Some(v) = p.value {
                    attr(el, "max", "1");
                    unsafe { day_dom_set_value(el, v) };
                } else {
                    // Which of the two this element is, is only knowable here — both render with
                    // no text, so `measure` cannot tell them apart from the DOM.
                    SPINNERS.with(|s| s.borrow_mut().insert(el, if p.large { 48.0 } else { 22.0 }));
                    attr(
                        el,
                        "data-day-animating",
                        if p.animating { "true" } else { "false" },
                    );
                    if p.large {
                        s(el, "width", "48px");
                        s(el, "height", "48px");
                    }
                }
                el
            }
            Some(Builtin::Image) => {
                let Some(p) = day_spec::props_of::<ImageProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                // Bytes and shared decodes arrive through `decode_image` (docs/images.md); this
                // arm is the staged-asset path, unchanged — including the one backend quirk that
                // a slash-carrying name is passed straight through as a URL.
                let named = match &p.source {
                    day_spec::ImageSource::Named(name) => name.clone(),
                    _ => String::new(),
                };
                // Empty for bytes and shared decodes: the shim owns their URL (a blob), and
                // sets it on the element itself, mask or `<img>`.
                let src = if named.is_empty() {
                    String::new()
                } else if named.contains('/') {
                    named.clone()
                } else {
                    format!("assets/images/{}.{}", named, image_ext(&named))
                };
                let fit = match p.content_mode {
                    ContentMode::Fit => "contain",
                    ContentMode::Fill => "cover",
                    ContentMode::Stretch => "fill",
                };
                // A TINTED or TEMPLATE glyph is a masked div, not an `<img>`: the browser
                // cannot recolor an image's pixels, and an `<img>` paints its own art over
                // whatever sits behind it, so masking one only clips it — the tint shows through
                // as a faint edge. Painting the mask with the tint (or `currentColor` for a
                // template) is what the nav rows do, and it is the only technique here that
                // actually recolors (docs/vectors.md "Tint").
                let state = ImageState {
                    src: src.clone(),
                    fit,
                    tint: p.tint,
                    template: p.template,
                    is_img: p.tint.is_none() && !p.template,
                };
                let el = unsafe { day_dom_create(if state.masked() { EL_DIV } else { EL_IMAGE }) };
                if state.masked() {
                    if !p.decorative {
                        attr(el, "role", "img");
                    }
                    // A mask cut from bytes the app holds (a `Symbol`'s outline SVG, a settings
                    // row's glyph, a tinted photo) or from a shared decode: the shim makes or
                    // finds the blob URL and sets it as this div's mask, since a URL is the
                    // one thing a mask can be cut from.
                    match &p.source {
                        day_spec::ImageSource::Named(_) => {}
                        day_spec::ImageSource::Bytes(b) => unsafe {
                            day_dom_image_set_bytes(el, b.as_ptr(), b.len());
                        },
                        day_spec::ImageSource::Decoded(id) => unsafe {
                            day_dom_image_set_bitmap(el, id.0 as f64);
                        },
                    }
                } else {
                    // Bytes and shared decodes never name a file (docs/images.md): the shim owns
                    // a blob URL for them, and setting the staged `src` first would fire a
                    // request for an asset that does not exist.
                    match &p.source {
                        day_spec::ImageSource::Named(_) => attr(el, "src", &src),
                        day_spec::ImageSource::Bytes(b) => unsafe {
                            day_dom_image_set_bytes(el, b.as_ptr(), b.len());
                        },
                        day_spec::ImageSource::Decoded(id) => unsafe {
                            day_dom_image_set_bitmap(el, id.0 as f64);
                        },
                    }
                    s(el, "object-fit", fit);
                    if p.decorative {
                        attr(el, "alt", "");
                    }
                }
                if p.decorative {
                    attr(el, "aria-hidden", "true");
                }
                if state.masked() {
                    apply_image_tint(el, &state);
                }
                IMAGE_SRC.with(|m| m.borrow_mut().insert(el, state));
                // Pass-through until something listens (docs/gestures.md "Pass-through").
                class(el, "day-passive-leaf", true);
                el
            }
            Some(Builtin::Canvas) => {
                let el = unsafe { day_dom_create(EL_CANVAS) };
                // A canvas is the one built-in piece with no native control under it, so it
                // needs a tab stop of its own before focus — and with focus, the keys
                // (docs/menus.md) — can reach what it draws.
                unsafe { day_dom_canvas_keynav(el) };
                // …and reports focus both ways (mask 8), so `.focused(signal)` binds two-way
                // and dayscript's `assert_focused` can see where the keyboard is.
                unsafe { day_dom_listen(el, 8) };
                // Pass-through (docs/gestures.md): a canvas with no handler lets a press reach
                // what is behind it. A key handler is declared while the piece builds, after
                // this realize, so whether one came is asked once the build is over: a canvas
                // that hears keys takes the press that focuses it.
                class(el, "day-passive-leaf", true);
                post_local(move || {
                    if day_spec::keys::handled(id) || day_spec::keys::text(id) {
                        unsafe { day_dom_takes_pointer(el) };
                    }
                });
                el
            }
            Some(Builtin::Scroll) => {
                let Some(p) = day_spec::props_of::<ScrollProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let el = unsafe { day_dom_create(EL_SCROLL) };
                if p.horizontal {
                    class(el, "horizontal", true);
                }
                // Offsets only for a piece that reads them (docs/scroll.md "Offset").
                if p.track_offset {
                    unsafe { day_dom_listen(el, listen::SCROLL) };
                }
                let flags = u32::from(p.indicators == ScrollIndicators::Hidden)
                    | (u32::from(p.bounce == Some(false)) << 1)
                    | (u32::from(p.track_phase) << 2);
                let (snap, interval) = match p.snap {
                    ScrollSnap::Page => (1, 0.0),
                    ScrollSnap::Interval(points) if points > 0.0 => (2, points),
                    // No snapping, or an interval that names no stops.
                    ScrollSnap::None | ScrollSnap::Interval(_) => (0, 0.0),
                };
                if flags != 0 || snap != 0 {
                    unsafe { day_dom_scroll_setup(el, flags, snap, interval) };
                }
                el
            }
            Some(Builtin::Divider) => {
                let el = unsafe { day_dom_create(EL_DIVIDER) };
                class(el, "day-passive-leaf", true);
                el
            }
            Some(Builtin::Nav) => {
                let Some(p) = day_spec::props_of::<NavProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let el = unsafe { day_dom_create(EL_NAV) };
                class(el, "document-tabs", p.documents.is_some());
                unsafe {
                    day_dom_nav_mode(
                        el,
                        nav_mode(p.presentation),
                        p.title.as_ptr(),
                        p.title.len(),
                    );
                };
                unsafe { day_dom_listen(el, 1) }; // the back bar's button reports via CLICK
                let state = NavState {
                    presentation: p.presentation,
                    pages: Vec::new(),
                    sidebar: None,
                    titles: vec![p.title.clone()],
                    selected: 0,
                };
                // The bar's initial state (hidden at the root, the host's title): a cover's
                // strip that takes the bar over later starts from it.
                sync_back_bar(el, &state);
                NAV_STATE.with(|m| m.borrow_mut().insert(el, state));
                el
            }
            Some(Builtin::NavPage) => {
                let Some(p) = day_spec::props_of::<NavPageProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let el = unsafe { day_dom_create(EL_PAGE) };
                PAGE_SIDEBAR.with(|m| {
                    m.borrow_mut()
                        .insert(el, p.pane == day_spec::props::Pane::Sidebar)
                });
                CSS_FRAMED.with(|set| set.borrow_mut().insert(el));
                unsafe { day_dom_listen(el, 32) };
                el
            }
            // Emulated fullscreen cover (docs/cover.md): a fixed-position overlay, hidden
            // until presented. CSS-framed (inset:0) and observer-reported, like nav pages.
            Some(Builtin::Cover) => {
                let el = unsafe { day_dom_create(EL_PAGE) };
                class(el, "day-cover", true);
                // The half sheet: a bottom-anchored half panel (`.day-half`, day.css), the
                // page above it on show and clickable.
                let p = day_spec::props_of::<CoverProps>(kind, "web-dom", props);
                if p.is_some_and(|p| p.half) {
                    class(el, "day-half", true);
                    COVER_HALF.with(|set| set.borrow_mut().insert(el));
                } else if p.is_some_and(|p| p.sheet) {
                    COVER_SHEET.with(|set| set.borrow_mut().insert(el));
                }
                CSS_FRAMED.with(|set| set.borrow_mut().insert(el));
                unsafe { day_dom_listen(el, 32) };
                el
            }
            Some(Builtin::NavMenu) => {
                let Some(p) = day_spec::props_of::<NavMenuProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let el = unsafe { day_dom_create(EL_NAVMENU) };
                let json = navmenu_json(
                    &p.items,
                    &p.icons,
                    &p.tints,
                    &p.badges,
                    &p.badge_icons,
                    &p.badge_tints,
                    &p.sections,
                    p.selected,
                );
                unsafe { day_dom_navmenu(el, json.as_ptr(), json.len()) };
                el
            }
            Some(Builtin::List) => {
                let Some(p) = day_spec::props_of::<ListProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let host = unsafe { day_dom_create(EL_SCROLL) };
                class(host, "day-list", true);
                if p.reorderable {
                    unsafe { day_dom_list_reorder(host) };
                }
                let content = unsafe { day_dom_create(EL_DIV) };
                unsafe { day_dom_insert(host, content, 0) };
                let (row_height, automatic) = match p.row_height {
                    RowHeight::Uniform(h) => (h, false),
                    RowHeight::Automatic => (44.0, true),
                };
                LISTS.with(|m| {
                    m.borrow_mut().insert(
                        host,
                        ListEntry {
                            node: id,
                            content,
                            row_height,
                            automatic,
                            heights: Vec::new(),
                            source: None,
                            cells: Vec::new(),
                            bound: Vec::new(),
                            fill_pending: false,
                            pending_scroll: None,
                            seen: None,
                            owed_polls: 0,
                            last_width: -1.0,
                            activatable: p.activatable,
                            selectable: p.selectable,
                            separators: p.separators == Some(true),
                            multi: p.multi_select,
                            selected: BTreeSet::new(),
                            anchor: None,
                            lead: None,
                            pin: (0, 0),
                            pin_row: None,
                        },
                    )
                });
                // Rows are built as they scroll in, so the list has to hear about scrolling.
                unsafe { day_dom_list_on_scroll(host) };
                if p.selectable {
                    unsafe { day_dom_list_keynav(host, u32::from(p.multi_select)) };
                }
                host
            }
            // A recycled list cell is ADOPTED from the native list, never realized through
            // this path; the inspector kinds never arrive (`Cap::Inspector` is Unsupported
            // here, so the piece composes its pane instead — docs/inspector.md); anything
            // else is an extension piece.
            Some(
                Builtin::ListCell
                | Builtin::ListItem
                | Builtin::Tree
                | Builtin::Inspector
                | Builtin::InspectorPane
                | Builtin::Split
                | Builtin::SplitPane,
            )
            | None => {
                // An external piece's own dom renderer, if one registered for this kind.
                if let Some(make) = registered(kind, |r| r.make) {
                    let h = make(self, props, id);
                    remember(h.0, id);
                    return h;
                }
                // `warn` reaches the browser console; `report` records it for
                // dayscript's assert_no_placeholders (eprintln goes nowhere on wasm).
                warn(&format!(
                    "day: no renderer for piece kind \"{kind}\" on web-dom (rendering a placeholder)"
                ));
                day_spec::placeholder::report(kind, "web-dom");
                let el = unsafe { day_dom_create(EL_LABEL) };
                text(el, &format!("⟨{kind}⟩"));
                class(el, "placeholder", true);
                el
            }
        };
        remember(el, id);
        DomHandle(el)
    }

    fn update(
        &mut self,
        h: &DomHandle,
        kind: PieceKind,
        patch: &dyn std::any::Any,
        anim: Option<&AnimSpec>,
    ) {
        let el = h.0;
        match kind {
            kinds::IMAGE => {
                if let Some(p) = patch.downcast_ref::<day_spec::props::ImagePatch>() {
                    match p {
                        day_spec::props::ImagePatch::Tint(c) => {
                            // The mask needs the same URL the element already loads.
                            let state = IMAGE_SRC.with(|m| {
                                let mut m = m.borrow_mut();
                                let state = m.get_mut(&el)?;
                                state.tint = *c;
                                Some(state.clone())
                            });
                            if let Some(state) = state {
                                // An element realized with a tint or as a template is already
                                // the masked div, so this repaints the mask's fill (a template
                                // with no tint goes back to `currentColor`); an `<img>` gains
                                // or loses its mask.
                                apply_image_tint(el, &state);
                            }
                        }
                        // A source swap repaints the same element (docs/images.md), so an
                        // `image()` bound to a signal shows new pixels without rebuilding its
                        // subtree. A masked image (tinted or a template) re-cuts its mask from
                        // the new source and keeps its tint: from the new name here — a
                        // recycled sidebar row shows the glyph it was rebound to — or, for
                        // bytes and decodes, in the shim, which owns their URL.
                        day_spec::props::ImagePatch::Source(source) => {
                            let src = match source {
                                day_spec::ImageSource::Named(name) if name.contains('/') => {
                                    name.clone()
                                }
                                day_spec::ImageSource::Named(name) => {
                                    format!("assets/images/{}.{}", name, image_ext(name))
                                }
                                _ => String::new(),
                            };
                            let state = IMAGE_SRC.with(|m| {
                                let mut m = m.borrow_mut();
                                let state = m.get_mut(&el)?;
                                state.src.clone_from(&src);
                                Some(state.clone())
                            });
                            match source {
                                day_spec::ImageSource::Named(_) => {
                                    if state.as_ref().is_none_or(|s| s.is_img) {
                                        attr(el, "src", &src);
                                    }
                                }
                                day_spec::ImageSource::Bytes(b) => unsafe {
                                    day_dom_image_set_bytes(el, b.as_ptr(), b.len());
                                },
                                day_spec::ImageSource::Decoded(id) => unsafe {
                                    day_dom_image_set_bitmap(el, id.0 as f64);
                                },
                            }
                            if let Some(state) = state.filter(ImageState::masked) {
                                apply_image_tint(el, &state);
                            }
                        }
                    }
                }
            }
            kinds::CONTAINER => match patch.downcast_ref::<ContainerPatch>() {
                Some(ContainerPatch::Fill(fill)) => apply_fill(el, fill.as_ref(), anim, false),
                Some(ContainerPatch::Surface(p)) => {
                    let role = SURFACE_ROLES.with(|t| t.get(el as usize));
                    apply_surface(el, p, role, anim, false);
                }
                None => {}
            },
            kinds::LABEL => {
                if let Some(p) = patch.downcast_ref::<LabelPatch>() {
                    match p {
                        LabelPatch::Paragraph(para) => {
                            LABELS.with(|t| {
                                if let Some(st) = t.borrow_mut().get_mut(&el) {
                                    st.para = *para;
                                }
                            });
                            apply_paragraph(el);
                        }
                        LabelPatch::Text(t) => {
                            set_label_state_text(el, t, false);
                            text(el, t);
                            // The measure cache is keyed by element — new text, new metrics.
                            MEASURE_CACHE.with(|c| c.borrow_mut().retain(|(e, _), _| *e != el));
                            elide_label(el);
                        }
                        LabelPatch::Runs(t, runs) => {
                            set_label_state_text(el, t, !runs.is_empty());
                            set_label_text(el, t, runs);
                            MEASURE_CACHE.with(|c| c.borrow_mut().retain(|(e, _), _| *e != el));
                            elide_label(el);
                        }
                        LabelPatch::Color(c) => match c {
                            Some(c) => s(el, "color", &ink_css(*c)),
                            None => s(el, "color", ""),
                        },
                        LabelPatch::Font(f) => {
                            apply_font(el, f);
                            // The `font` shorthand reset the line height.
                            apply_paragraph(el);
                        }
                    }
                }
            }
            kinds::BUTTON => {
                if let Some(p) = patch.downcast_ref::<ButtonPatch>() {
                    match p {
                        ButtonPatch::Content(c) => {
                            apply_button_content(el, &c.title, c.icon.as_ref(), c.icon_only);
                        }
                        ButtonPatch::Title(t) => text(el, t),
                        ButtonPatch::Enabled(e) => set_enabled(el, *e),
                        ButtonPatch::Selected(on) => set_button_selected(el, *on),
                        ButtonPatch::Style(st) => apply_button_style(el, *st),
                    }
                }
            }
            kinds::TOGGLE => {
                if let Some(p) = patch.downcast_ref::<TogglePatch>() {
                    match p {
                        TogglePatch::On(on) => unsafe { day_dom_set_checked(el, u32::from(*on)) },
                        TogglePatch::Enabled(e) => set_enabled(el, *e),
                    }
                }
            }
            kinds::SLIDER => {
                if let Some(p) = patch.downcast_ref::<SliderPatch>() {
                    match p {
                        SliderPatch::Value(v) => unsafe { day_dom_set_value(el, *v) },
                        SliderPatch::Enabled(e) => set_enabled(el, *e),
                    }
                }
            }
            kinds::TEXT_FIELD => {
                if let Some(p) = patch.downcast_ref::<TextFieldPatch>() {
                    match p {
                        TextFieldPatch::Text {
                            text: t,
                            from_native,
                        } => {
                            if !*from_native {
                                attr(el, "value", t);
                            }
                        }
                        TextFieldPatch::Placeholder(t) => attr(el, "placeholder", t),
                        TextFieldPatch::Enabled(e) => set_enabled(el, *e),
                        TextFieldPatch::Select(sel) => {
                            let at = |n: usize| n.min(u32::MAX as usize) as u32;
                            unsafe { day_dom_field_select(el, at(sel.start), at(sel.end)) }
                        }
                    }
                }
            }
            kinds::TEXT_AREA => {
                if let Some(TextAreaPatch::SetText(t)) = patch.downcast_ref::<TextAreaPatch>() {
                    // Day-driven content changed: refresh the line count `measure` grows by.
                    AREA_HINTS.with(|m| {
                        if let Some(h) = m.borrow_mut().get_mut(&el) {
                            h.2 = content_lines(t);
                        }
                    });
                }
                if let Some(p) = patch.downcast_ref::<TextAreaPatch>() {
                    match p {
                        TextAreaPatch::SetText(t) => text(el, t),
                        // `readonly` is the INVERSE of editable, and the marker convention is
                        // "-" sets / "" removes (see the shim's day_dom_set_attr): editable ⇒ no
                        // readonly attribute.
                        TextAreaPatch::SetEditable(e) => {
                            attr(el, "readonly", if *e { "" } else { "-" });
                        }
                        TextAreaPatch::SetSelectable(sel) => {
                            s(el, "user-select", if *sel { "text" } else { "none" });
                        }
                        TextAreaPatch::SetSpellCheck(sc) => {
                            attr(el, "spellcheck", if *sc { "true" } else { "false" });
                        }
                    }
                }
            }
            kinds::PICKER => {
                // Programmatic DOM value sets fire no events — echo-free by construction.
                match patch.downcast_ref::<PickerPatch>() {
                    Some(PickerPatch::Selected(i)) => {
                        PICKER_SELECTED.with(|m| m.borrow_mut().insert(el, *i));
                        let seg = SEG_COUNT.with(|m| m.borrow().get(&el).copied());
                        match seg {
                            Some(_) => unsafe { day_dom_options_select(el, *i as u32) },
                            None => unsafe { day_dom_set_value(el, *i as f64) },
                        }
                    }
                    // New labels: the shim's own builder rewrites the choices, so the same
                    // call that filled them at build refills them here. The selected index
                    // rides along, clamped to the new list.
                    Some(PickerPatch::Options(opts)) => {
                        let keep = PICKER_SELECTED.with(|m| m.borrow().get(&el).copied());
                        let selected = keep.unwrap_or(0).min(opts.len().saturating_sub(1));
                        let json = picker_json(&PickerProps {
                            options: opts.clone(),
                            selected,
                            ..Default::default()
                        });
                        unsafe { day_dom_options(el, json.as_ptr(), json.len()) };
                        if SEG_COUNT.with(|m| m.borrow().contains_key(&el)) {
                            SEG_COUNT.with(|m| m.borrow_mut().insert(el, opts.len()));
                        }
                    }
                    // A segmented or inline picker is a group of buttons; the shim passes the
                    // group's `disabled` to each one (and to the ones a later Options rebuild
                    // makes).
                    Some(PickerPatch::Enabled(on)) => set_enabled(el, *on),
                    None => {}
                }
            }
            kinds::PROGRESS => {
                if let Some(ProgressPatch::Animating(on)) = patch.downcast_ref::<ProgressPatch>() {
                    attr(el, "data-day-animating", if *on { "true" } else { "false" });
                }

                if let Some(ProgressPatch::Value(v)) = patch.downcast_ref::<ProgressPatch>()
                    && let Some(v) = v
                {
                    unsafe { day_dom_set_value(el, *v) };
                }
            }
            kinds::NAV => {
                if let Some(p) = patch.downcast_ref::<NavPatch>() {
                    nav_patch(el, p);
                }
            }
            // Emulated cover (docs/cover.md): present = re-home under #day-root (position:fixed
            // escapes ancestor transforms only from a clean containing block) and show; the
            // ResizeObserver reports the frame. Dismiss = hide + `CoverHidden` at once.
            kinds::COVER => {
                if let Some(p) = patch.downcast_ref::<CoverPatch>() {
                    match p {
                        CoverPatch::Present { background, .. } => {
                            // Each present sets its own fill or returns to the stylesheet's: the
                            // element outlives one presentation. A composed sheet's cover is
                            // clear (docs/cover.md "Modal sheet"), and here it is `.day-sheet`:
                            // the stylesheet frosts the page under it, and its toolbar strip
                            // docks on the card rather than across the window (shim.js).
                            let sheet = COVER_SHEET.with(|set| set.borrow().contains(&el))
                                && *background == Some(day_spec::Color::CLEAR.into());
                            class(el, "day-sheet", sheet);
                            match background {
                                Some(bg) if !sheet => {
                                    s(el, "background-color", &ink_css(*bg));
                                }
                                _ => s(el, "background-color", ""),
                            }
                            unsafe { day_dom_insert(1, el, u32::MAX) };
                            class(el, "open", true);
                            // Seed the frame SYNCHRONOUSLY from the cached viewport (the cover is
                            // fixed/inset:0, so that IS its size). Without this, the content lays
                            // out at width 0 until the async ResizeObserver fires — invisible in
                            // LTR's top-left but off-screen to the LEFT under RTL (docs/cover.md).
                            if let Some(node) = node_of(el) {
                                let vp = LAST_VIEWPORT.with(std::cell::Cell::get);
                                // Less the cover's own toolbar strip, which sits above it.
                                let strip = unsafe { day_dom_toolbar_height(el) };
                                // A half sheet is the bottom half, less the grabber strip
                                // the stylesheet pads for (`.day-half`); the observer
                                // reports the content box too.
                                let half = COVER_HALF.with(|s| s.borrow().contains(&el));
                                let size = if half {
                                    Size::new(vp.width, vp.height * 0.5 - HALF_SHEET_GRABBER)
                                } else {
                                    Size::new(vp.width, vp.height - strip)
                                };
                                if size.width > 0.0 && size.height > 0.0 {
                                    emit(node, Event::FrameChanged(size));
                                }
                            }
                        }
                        CoverPatch::DismissDisabled(_) => {}
                        CoverPatch::Dismiss => {
                            class(el, "open", false);
                            if let Some(node) = node_of(el) {
                                emit(node, Event::CoverHidden);
                            }
                        }
                    }
                }
            }
            kinds::NAV_MENU => {
                if let Some(NavMenuPatch::Items {
                    items,
                    icons,
                    tints,
                    badges,
                    badge_icons,
                    badge_tints,
                    sections,
                    selected,
                    rows_changed,
                    ..
                }) = patch.downcast_ref::<NavMenuPatch>()
                {
                    let mut json = navmenu_json(
                        items,
                        icons,
                        tints,
                        badges,
                        badge_icons,
                        badge_tints,
                        sections,
                        *selected,
                    );
                    match rows_changed {
                        // The same rows with new words or counts: refill those rows in place.
                        Some(rows) => {
                            json.pop();
                            json.push_str(",\"rows\":[");
                            for (n, r) in rows.iter().enumerate() {
                                if n > 0 {
                                    json.push(',');
                                }
                                json.push_str(&r.to_string());
                            }
                            json.push_str("]}");
                            unsafe { day_dom_navmenu_rows(el, json.as_ptr(), json.len()) };
                        }
                        None => unsafe { day_dom_navmenu(el, json.as_ptr(), json.len()) },
                    }
                } else if let Some(NavMenuPatch::Selected(sel)) =
                    patch.downcast_ref::<NavMenuPatch>()
                {
                    unsafe { day_dom_navmenu_select(el, sel.map_or(-1, |i| i as i32)) };
                }
            }
            kinds::LIST => {
                if let Some(p) = patch.downcast_ref::<ListPatch>() {
                    list_patch(el, p);
                }
            }
            other => {
                if let Some(update) = registered(other, |r| r.update) {
                    update(self, h, patch);
                }
            }
        }
    }

    /// Offer a satellite piece its teardown hook before `release` frees the handle (§15.2).
    /// web-dom's registry is a runtime `RefCell`, so the hook is copied out before the call —
    /// the borrow must end before a piece re-enters the registry.
    fn release_piece(&mut self, kind: day_spec::PieceKind, h: &Self::Handle) {
        if let Some(Some(f)) = registered(kind, |r| r.release) {
            f(self, h);
        }
    }

    fn release(&mut self, h: DomHandle) {
        let el = h.0;
        // One sweep drops this element's entry from every registered `SideTable` — present
        // and future — before the manual purges below (day_spec::sidetable; day-toolkit-dom keys by
        // shim element id, widened to the sweeper's usize key space).
        day_spec::sidetable::sweep(el as usize);
        NODE_OF.with(|m| m.borrow_mut().remove(&el));
        IMAGE_SRC.with(|m| m.borrow_mut().remove(&el));
        // An object URL this element made for `ImageSource::Bytes` is not reachable by the GC: it
        // lives on the document until revoked, so the element's release is the moment to do it.
        // SAFETY: an element id the shim either knows or does not; revoking nothing is a no-op.
        unsafe { day_dom_image_revoke(el) };
        CSS_FRAMED.with(|s| s.borrow_mut().remove(&el));
        COVER_HALF.with(|s| s.borrow_mut().remove(&el));
        COVER_SHEET.with(|s| s.borrow_mut().remove(&el));
        AREA_HINTS.with(|m| m.borrow_mut().remove(&el));
        PAGE_SIDEBAR.with(|m| m.borrow_mut().remove(&el));
        NAV_STATE.with(|m| m.borrow_mut().remove(&el));
        SEG_COUNT.with(|m| m.borrow_mut().remove(&el));
        PICKER_SIZE.with(|m| m.borrow_mut().remove(&el));
        SPINNERS.with(|s| s.borrow_mut().remove(&el));
        LABELS.with(|t| t.borrow_mut().remove(&el));
        GESTURES.with(|t| t.borrow_mut().remove(&el));
        DIRECTION.with(|t| t.borrow_mut().remove(&el));
        MEASURE_CACHE.with(|c| c.borrow_mut().retain(|(e, _), _| *e != el));
        if let Some(list) = LISTS.with(|m| m.borrow_mut().remove(&el)) {
            CELL_ROWS.with(|m| {
                let mut m = m.borrow_mut();
                for c in list.cells {
                    m.remove(&c);
                }
            });
            // The pinned header's cell is the host's alone (no row slot holds it), so nothing
            // else tells day-core to dispose the header built there.
            if list.pin.1 != 0
                && let Some(source) = &list.source
            {
                (source.release)(list.pin.1 as usize as day_spec::RawHandle);
            }
        }
        unsafe { day_dom_release(el) };
    }

    fn insert(&mut self, parent: &DomHandle, child: &DomHandle, index: usize) {
        // Nav host: route pages into the right pane; everything else is plain DOM insert.
        let routed = NAV_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.get_mut(&parent.0) else {
                return false;
            };
            let sidebar = PAGE_SIDEBAR
                .with(|p| p.borrow().get(&child.0).copied())
                .unwrap_or(false);
            if sidebar {
                state.sidebar = Some(child.0);
            }
            // The sidebar page belongs to the CHROME slot in every presentation except `Stack`,
            // where it is the stack's root and so joins the detail pages instead.
            let to_chrome = sidebar && state.presentation != NavPresentation::Stack;
            unsafe { day_dom_nav_add_page(parent.0, child.0, u32::from(to_chrome)) };
            if !to_chrome {
                state.pages.push(child.0);
            }
            true
        });
        if routed {
            return;
        }
        unsafe { day_dom_insert(parent.0, child.0, index as u32) };
    }

    fn remove(&mut self, parent: &DomHandle, child: &DomHandle) {
        NAV_STATE.with(|m| {
            if let Some(state) = m.borrow_mut().get_mut(&parent.0) {
                state.pages.retain(|p| *p != child.0);
                // Dropping a page shifts every index after it, and `selected` is an index. Clamp
                // rather than leave it dangling past the end — a stale one would hide every page.
                state.selected = state.selected.min(state.pages.len().saturating_sub(1));
            }
        });
        unsafe { day_dom_remove(child.0) };
    }

    fn move_child(&mut self, parent: &DomHandle, child: &DomHandle, to: usize) {
        unsafe { day_dom_insert(parent.0, child.0, to as u32) };
    }

    fn measure(&mut self, h: &DomHandle, kind: PieceKind, p: Proposal) -> Size {
        let el = h.0;
        let max_w = p.width.unwrap_or(1.0e6);
        match kind {
            kinds::LABEL => MEASURE_CACHE.with(|cache| {
                let key = (el, (max_w * 4.0) as i64);
                if let Some(sz) = cache.borrow().get(&key) {
                    return *sz;
                }
                // In the label's own style, so the line limit, the letter spacing and the line
                // height count; an elided label measures the text it stands for, never the
                // shorter copy it shows at its current width.
                let full = LABELS.with(|t| {
                    t.borrow()
                        .get(&el)
                        .filter(|st| st.elided)
                        .map(|st| st.text.clone())
                });
                let m = label_size(el, full.as_deref(), max_w);
                let sz = Size::new(m.width.ceil().min(max_w), m.height.ceil());
                cache.borrow_mut().insert(key, sz);
                sz
            }),
            kinds::BUTTON => {
                let mut out = [0.0f64; 2];
                unsafe {
                    day_dom_measure_text(
                        std::ptr::null(),
                        el as usize,
                        std::ptr::null(),
                        0,
                        1.0e6,
                        out.as_mut_ptr(),
                    );
                };
                Size::new(out[0].ceil() + 26.0, 28.0)
            }
            kinds::TOGGLE => Size::new(40.0, 24.0),
            kinds::SLIDER => Size::new(p.width.unwrap_or(180.0), 24.0),
            kinds::TEXT_FIELD => Size::new(p.width.unwrap_or(200.0), 30.0),
            kinds::TEXT_AREA => {
                // The auto-growing-height contract (docs/textarea.md): content height clamped
                // between `min_lines` and `max_lines`, like the uikit arm's sizeThatFits path.
                // The line height comes from a REAL measurement in the control font, so it rides
                // `--day-text-scale` and the browser's font-size preference exactly as the
                // rendered control does; `content_lines` counts hard breaks only (soft wrapping
                // is not simulated — the area scrolls where wrapping would have grown it).
                let (min_l, max_l, lines) =
                    AREA_HINTS.with(|m| m.borrow().get(&el).copied().unwrap_or((1, 0, 1)));
                let line_h = measure_str("x").height.max(16.0);
                let pad = 14.0; // .day-area: 6px top/bottom padding + 1px borders (day.css)
                let min_l = min_l.max(1);
                let mut l = lines.max(min_l);
                if max_l > 0 {
                    l = l.min(max_l.max(min_l));
                }
                Size::new(
                    p.width.unwrap_or(240.0),
                    p.height.unwrap_or(f64::from(l) * line_h + pad),
                )
            }
            kinds::PICKER => PICKER_SIZE
                .with(|m| m.borrow().get(&el).copied())
                .unwrap_or(Size::new(68.0, 26.0)),
            // A spinner is SQUARE — `.day-spinner` is a 50%-radius ring that rotates, so any
            // other aspect ratio spins as an ellipse sweeping the layout (docs/pieces.md).
            // Determinate progress is the wide, short bar.
            kinds::PROGRESS => {
                if let Some(side) = SPINNERS.with(|s| s.borrow().get(&el).copied()) {
                    Size::new(side, side)
                } else {
                    Size::new(p.width.unwrap_or(160.0), 8.0)
                }
            }
            kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
            kinds::IMAGE => Size::new(p.width.unwrap_or(100.0), p.height.unwrap_or(100.0)),
            other => {
                if let Some(measure) = registered(other, |r| r.measure).flatten() {
                    return measure(self, h, p);
                }
                Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0))
            }
        }
    }

    /// The browser reports exact font metrics through a canvas `TextMetrics`, and the
    /// element's own border/padding says where its text box begins — so an `<input>` reports a
    /// lower baseline than a bare `<div>`, which is what makes a row line up (docs/baseline.md).
    /// `Emulated` rather than `Native`: CSS `align-items: baseline` would do this natively, but
    /// day positions every element absolutely and needs the number, not the alignment mode.
    fn first_baseline(&mut self, h: &DomHandle, kind: PieceKind, size: Size) -> Option<f64> {
        if !day_spec::kind_has_baseline(kind) {
            return None;
        }
        let b = unsafe { day_dom_baseline(h.0, size.height) };
        (b >= 0.0).then_some(b)
    }

    fn set_frame(&mut self, h: &DomHandle, frame: Rect, anim: Option<&AnimSpec>) {
        if CSS_FRAMED.with(|s| s.borrow().contains(&h.0)) {
            return;
        }
        match anim {
            Some(_) => animate(
                h.0,
                anim_prop::FRAME,
                &[
                    frame.origin.x,
                    frame.origin.y,
                    frame.size.width,
                    frame.size.height,
                ],
                anim,
            ),
            None => unsafe {
                day_dom_set_frame(
                    h.0,
                    frame.origin.x,
                    frame.origin.y,
                    frame.size.width,
                    frame.size.height,
                );
            },
        }
        let relabel = LABELS.with(|t| {
            t.borrow_mut().get_mut(&h.0).is_some_and(|st| {
                let changed = (st.width - frame.size.width).abs() >= 0.5;
                st.width = frame.size.width;
                changed && st.elides()
            })
        });
        if relabel {
            elide_label(h.0);
        }
        let gradient = SURFACE_GRADIENTS.with(|t| {
            t.with(h.0 as usize, |(g, size)| {
                *size = frame.size;
                linear_gradient_css(g, frame.size)
            })
        });
        if let Some(css) = gradient {
            s(h.0, "background-image", &css);
        }
        // A list host framed: (re)populate when its width actually changed.
        let repopulate = LISTS.with(|m| {
            let mut m = m.borrow_mut();
            let Some(st) = m.get_mut(&h.0) else {
                return false;
            };
            if (st.last_width - frame.size.width).abs() < 0.5 {
                return false;
            }
            st.last_width = frame.size.width;
            true
        });
        if repopulate {
            let host = h.0;
            post_local(move || list_populate(host));
        }
    }

    fn set_opacity(&mut self, h: &DomHandle, opacity: f64, anim: Option<&AnimSpec>) {
        animate(h.0, anim_prop::OPACITY, &[opacity], anim);
    }

    fn set_transform(&mut self, h: &DomHandle, t: Transform, _size: Size, anim: Option<&AnimSpec>) {
        // Mark this element as a compositing root so its rounded-clip descendants get their own
        // clipped layer (day.css `.day-xform .day-clip`) — see the note in `apply_surface`.
        class(h.0, "day-xform", true);
        s(h.0, "transform-origin", "50% 50%");
        animate(
            h.0,
            anim_prop::TRANSFORM,
            &[t.tx, t.ty, t.rotation.degrees(), t.sx, t.sy],
            anim,
        );
    }

    fn set_selectable(&mut self, h: &DomHandle, selectable: bool) -> Option<DomHandle> {
        // Nothing is selectable by default (#day-root sets `user-select: none`); the class opts
        // this element and its text back in (`.day-selectable` in day.css). See docs/text.md.
        class(h.0, "day-selectable", selectable);
        if selectable {
            unsafe { day_dom_takes_pointer(h.0) };
        }
        None
    }

    fn set_input_traits(
        &mut self,
        h: &DomHandle,
        traits: &day_spec::InputTraits,
    ) -> Option<DomHandle> {
        use day_spec::{InputPurpose as P, SubmitLabel as L};
        // One `<input>` covers every member: each is an attribute, and all of them are written
        // on every call (an empty value removes one), so a flip back to the defaults leaves no
        // attribute behind. The type is `text` for every purpose: the typed ones (`email`,
        // `url`, `number`) trim or coerce what the user types and validate it, and a controlled
        // input has to hold exactly the text it reports. `inputmode` alone picks the keyboard.
        let (inputmode, autocomplete, capitalize) = match traits.purpose {
            P::Text => ("", "", ""),
            P::Name => ("", "name", "words"),
            P::Email => ("email", "email", "none"),
            P::Url => ("url", "url", "none"),
            P::Phone => ("tel", "tel", ""),
            P::Number => ("numeric", "", "none"),
            P::Decimal => ("decimal", "", "none"),
            P::Username => ("", "username", "none"),
            P::Password => ("", "current-password", "none"),
            P::NewPassword => ("", "new-password", "none"),
            P::OneTimeCode => ("", "one-time-code", "none"),
        };
        // Hidden characters are never capitalized, corrected or handed to a spell checker,
        // whatever the purpose; free text and a phone number keep the browser's own handling.
        let plain = !traits.secure && matches!(traits.purpose, P::Text | P::Phone);
        let kind = if traits.secure { "password" } else { "text" };
        let capitalize = if traits.secure { "none" } else { capitalize };
        let (autocorrect, spellcheck) = if plain { ("", "") } else { ("off", "false") };
        let enterkeyhint = match traits.submit_label {
            L::Return => "",
            L::Done => "done",
            L::Go => "go",
            L::Next => "next",
            L::Search => "search",
            L::Send => "send",
        };
        // `max_length` takes no attribute: `maxlength` counts UTF-16 units and the bound is in
        // characters, so the native limit would refuse text the bound allows. day-pieces holds
        // it.
        let readonly = if traits.read_only { "readonly" } else { "" };
        // Every value is one of the fixed words above, so the object needs no escaping.
        let json = format!(
            r#"{{"type":"{kind}","inputmode":"{inputmode}","autocomplete":"{autocomplete}","autocapitalize":"{capitalize}","autocorrect":"{autocorrect}","spellcheck":"{spellcheck}","enterkeyhint":"{enterkeyhint}","readonly":"{readonly}"}}"#
        );
        unsafe { day_dom_input_traits(h.0, json.as_ptr(), json.len()) };
        None
    }

    fn set_cursor(&mut self, h: &DomHandle, cursor: Cursor) {
        // The CSS vocabulary is the contract (docs/cursor.md); an inline `cursor` on the element
        // wins over the stylesheet's affordance rules, and an empty value removes it.
        let v = if cursor == Cursor::Default {
            ""
        } else {
            cursor.css_name()
        };
        let p = "cursor";
        unsafe { day_dom_set_style(h.0, p.as_ptr(), p.len(), v.as_ptr(), v.len()) };
        // A browser shows a cursor only over an element the pointer hits, so a cursor makes a
        // pass-through element a pointer target (the one place the web departs from
        // docs/gestures.md "Pass-through").
        if cursor != Cursor::Default {
            unsafe { day_dom_takes_pointer(h.0) };
        }
    }

    fn set_scroll_content(&mut self, h: &DomHandle, content: Size) {
        unsafe { day_dom_scroll_content(h.0, content.width, content.height) };
    }

    /// A web list row sized for touch: 16px sides, 44px tall.
    fn list_row_metrics(&mut self, _h: &DomHandle) -> day_spec::RowMetrics {
        day_spec::RowMetrics {
            insets: day_spec::Insets {
                top: 10.0,
                leading: 16.0,
                bottom: 10.0,
                trailing: 16.0,
            },
            min_height: 44.0,
        }
    }

    /// Queued one turn, behind the populate a splice or reload in the same turn posted, so the
    /// content has its new extent; then the browser scrolls (`behavior: smooth` when animated)
    /// and its scroll events refill the window and send the viewport report.
    fn list_scroll(&mut self, h: &DomHandle, to: day_spec::ListScroll) -> bool {
        let el = h.0;
        if !LISTS.with(|m| m.borrow().contains_key(&el)) {
            return false;
        }
        post_local(move || {
            // The list can be released before its turn comes: a page swapped out in the same
            // task (a test run moving to its next case) takes the element with it, and the
            // shim would throw reading a viewport it no longer has.
            if !LISTS.with(|m| m.borrow().contains_key(&el)) {
                return;
            }
            let mut view = [0.0_f64; 2];
            unsafe { day_dom_list_viewport(el, view.as_mut_ptr()) };
            let target = LISTS.with(|m| {
                let m = m.borrow();
                let st = m.get(&el)?;
                let n = st.source.as_ref().map_or(0, |src| (src.len)());
                let extent = if st.stacked() {
                    st.tops(n)[n]
                } else {
                    n as f64 * st.row_height.max(1.0)
                };
                match to.row {
                    None => Some((extent - view[1]).max(0.0)),
                    Some(row) if row < n => row_scroll_target(
                        st.row_span(row),
                        st.pinned_above(row),
                        to.align,
                        view[0],
                        view[1],
                        extent,
                    ),
                    Some(_) => None,
                }
            });
            match target {
                Some(y) => {
                    // A list not laid out yet has nothing to scroll: the next fill repeats it.
                    if view[1] <= 0.0 {
                        LISTS.with(|m| {
                            if let Some(st) = m.borrow_mut().get_mut(&el) {
                                st.pending_scroll = Some(y);
                            }
                        });
                    }
                    unsafe { day_dom_scroll_to(el, 0.0, y, u32::from(to.animated)) };
                    // An instant jump has moved the offset already, but its `scroll` event
                    // waits for the next frame: build and report the rows it shows now.
                    if !to.animated && view[1] > 0.0 {
                        list_fill_window(el);
                    }
                }
                // Nothing moves: the report still follows the request.
                None => list_fill_window(el),
            }
        });
        true
    }

    fn scroll_to(&mut self, h: &DomHandle, target: Rect, animated: bool) {
        unsafe { day_dom_scroll_to(h.0, target.origin.x, target.origin.y, u32::from(animated)) };
    }

    fn scroll_offset(&mut self, h: &DomHandle) -> Point {
        let mut out = [0.0f64; 2];
        unsafe { day_dom_scroll_offset(h.0, out.as_mut_ptr()) };
        Point::new(out[0], out[1])
    }

    fn set_event_sink(&mut self, sink: EventSink) {
        SINK.with(|s| *s.borrow_mut() = Some(sink));
    }

    fn enable_gesture(&mut self, h: &DomHandle, node: NodeId, kind: GestureKind) {
        remember(h.0, node);
        // Idempotent per (element, kind): each shim recognizer is a set of listeners, which a
        // second `listen` would double.
        let mask = match kind {
            GestureKind::Tap => gesture_mask::TAP,
            GestureKind::Drag => gesture_mask::DRAG,
            GestureKind::Hover => gesture_mask::HOVER,
            GestureKind::LongPress => gesture_mask::LONG_PRESS,
            GestureKind::DoubleTap => gesture_mask::DOUBLE_TAP,
            GestureKind::Press => gesture_mask::PRESS,
            GestureKind::Touches => gesture_mask::TOUCHES,
            GestureKind::Pan => gesture_mask::PAN,
            GestureKind::Pinch => gesture_mask::PINCH,
            GestureKind::Rotate => gesture_mask::ROTATE,
        };
        listen_once(h.0, mask);
    }

    /// Every kind is the shim's own recognizer over pointer events (docs/gestures.md
    /// "Recognizers"): long press and press from `pointerdown`, double tap from the browser's
    /// `dblclick`, touches from the pointer stream of every contact.
    fn gesture_support(&self, _kind: GestureKind) -> Support {
        Support::Native
    }

    /// Leading and trailing follow the element's reading direction (`set_layout_direction`, else
    /// the app's).
    fn set_hit_slop(&mut self, h: &DomHandle, node: NodeId, slop: day_spec::Insets) {
        remember(h.0, node);
        let (left, right) = if element_rtl(h.0) {
            (slop.trailing, slop.leading)
        } else {
            (slop.leading, slop.trailing)
        };
        unsafe {
            day_dom_hit_slop(
                h.0,
                slop.top.max(0.0),
                left.max(0.0),
                slop.bottom.max(0.0),
                right.max(0.0),
            );
        }
    }

    /// `touch-action: pan-y` / `pan-x` on the element, so the browser scrolls across the axis
    /// natively, and the shim's drag stands down at the slop when a mouse sets off across it.
    fn set_drag_axis(&mut self, h: &DomHandle, _node: NodeId, axis: Option<day_spec::DragAxis>) {
        let code = match axis {
            None => 0,
            Some(day_spec::DragAxis::Horizontal) => 1,
            Some(day_spec::DragAxis::Vertical) => 2,
        };
        unsafe { day_dom_drag_axis(h.0, code) };
    }

    /// `inert` on the element (input, focus and the accessibility tree at once) plus
    /// `aria-disabled`; form controls keep their own `disabled` from day-core's patches.
    fn set_subtree_enabled(&mut self, h: &DomHandle, _node: NodeId, enabled: bool) {
        unsafe { day_dom_subtree_enabled(h.0, u32::from(enabled)) };
    }

    /// The computed style mid-run (`getComputedStyle` reports what a Web Animation shows):
    /// opacity, the frame from `left`/`top`/`width`/`height` in the root's space, the
    /// transform's translation, and the fill of an element that has one (docs/tween.md).
    fn read_displayed(&self, h: &DomHandle) -> day_spec::Displayed {
        let mut v = [0.0f64; 11];
        // SAFETY: `v` has room for the eleven values the shim writes.
        let fill = unsafe { day_dom_displayed(h.0, v.as_mut_ptr()) } != 0;
        day_spec::Displayed {
            opacity: Some(v[0]),
            frame: Some(Rect::new(v[1], v[2], v[3], v[4])),
            translation: Some(Point::new(v[5], v[6])),
            background: fill.then(|| day_spec::Color::rgba(v[7], v[8], v[9], v[10])),
        }
    }

    /// The browser's default font size against the 16 px (17 px under iOS Dynamic Type) Day's
    /// ramp is drawn for; one factor for every style.
    fn text_scale(&mut self, _style: Font) -> f64 {
        unsafe { day_dom_text_scale() }
    }

    /// `dir` on the element (docs/localization.md "Direction of a subtree"): a label's or a
    /// field's base direction and what `start`/`end` alignment mean, a range's fill side. Day
    /// places children with `left`, so a container's own `dir` moves nothing; scrolling hosts
    /// keep theirs, since an RTL scroller counts its offsets from the right.
    fn set_layout_direction(
        &mut self,
        h: &DomHandle,
        kind: PieceKind,
        direction: day_spec::LayoutDirection,
    ) {
        if kind == kinds::SCROLL || kind == kinds::LIST || kind == kinds::NAV {
            return;
        }
        let rtl = direction == day_spec::LayoutDirection::Rtl;
        DIRECTION.with(|d| d.borrow_mut().insert(h.0, rtl));
        attr(h.0, "dir", if rtl { "rtl" } else { "ltr" });
        MEASURE_CACHE.with(|c| c.borrow_mut().retain(|(e, _), _| *e != h.0));
    }

    /// `(pointer: coarse)` at launch, then the `pointerType` of each pointer down or hover.
    fn primary_pointer(&self) -> day_spec::PointerKind {
        pointer_kind(unsafe { day_dom_pointer_kind() })
    }

    fn set_focusable(&mut self, h: &DomHandle, node: NodeId, focusable: bool) {
        remember(h.0, node);
        if focusable {
            unsafe {
                day_dom_canvas_keynav(h.0);
                // The key route reports focus both ways itself (shim `reportFocus`), and a
                // press focuses the element, so it is a pointer target from now on.
                day_dom_takes_pointer(h.0);
            };
        }
    }

    fn focus(&mut self, h: &DomHandle, _node: NodeId, focused: bool) {
        unsafe { day_dom_focus(h.0, u32::from(focused)) };
    }

    fn attach_list(&mut self, host: &DomHandle, source: ListSource) {
        let el = host.0;
        LISTS.with(|m| {
            if let Some(st) = m.borrow_mut().get_mut(&el) {
                st.source = Some(source);
            }
        });
        post_local(move || list_populate(el));
    }

    fn set_route(&mut self, route: &str) {
        // First reflection rewrites the current entry (a fresh page load shouldn't grow
        // history just for showing its own launch route); every later change pushes one, so
        // browser back/forward walk the app's navigation.
        let replace = FIRST_ROUTE.with(|c| c.replace(false));
        unsafe { day_dom_set_hash(route.as_ptr(), route.len(), u32::from(replace)) };
    }

    fn edit_toolbar(&mut self, h: &DomHandle, ops: &[day_spec::ToolbarOp]) -> bool {
        // The web has no window chrome, so the bar is a strip the shim docks at the top of the
        // document (docs/toolbars.md), edited one item at a time like every native bar. A
        // cover-tier window's items go on a strip over the cover, where its content can reach
        // them (the page's strip is underneath it).
        let json = toolbar_edit_json(ops);
        unsafe { day_dom_toolbar_edit(h.0, json.as_ptr(), json.len()) };
        true
    }

    fn update_toolbar(&mut self, h: &DomHandle, patch: &day_spec::ToolbarPatch) {
        use day_spec::ToolbarPatch as P;
        let mut json = String::from("{\"item\":");
        match patch {
            P::Focus { item } => {
                json_str(&mut json, item);
                json.push_str(",\"focus\":true");
            }
            P::Text { item, text } => {
                json_str(&mut json, item);
                json.push_str(",\"text\":");
                json_str(&mut json, text);
            }
            P::On { item, on } => {
                json_str(&mut json, item);
                json.push_str(",\"on\":");
                json.push_str(if *on { "true" } else { "false" });
            }
            P::Selected { item, index } => {
                json_str(&mut json, item);
                json.push_str(",\"selected\":");
                json.push_str(&index.to_string());
            }
            P::Enabled { item, on } => {
                json_str(&mut json, item);
                json.push_str(",\"enabled\":");
                json.push_str(if *on { "true" } else { "false" });
            }
            P::Suggestions { item, list } => {
                json_str(&mut json, item);
                json.push_str(",\"suggestions\":[");
                for (n, sug) in list.iter().enumerate() {
                    if n > 0 {
                        json.push(',');
                    }
                    json_str(&mut json, sug);
                }
                json.push(']');
            }
            P::Label {
                item,
                label,
                tooltip,
            } => {
                json_str(&mut json, item);
                json.push_str(",\"label\":");
                json_str(&mut json, label);
                if let Some(tip) = tooltip {
                    json.push_str(",\"tip\":");
                    json_str(&mut json, tip);
                }
            }
            P::Placeholder { item, text } => {
                json_str(&mut json, item);
                json.push_str(",\"placeholder\":");
                json_str(&mut json, text);
            }
        }
        json.push('}');
        unsafe { day_dom_toolbar_patch(h.0, json.as_ptr(), json.len()) };
    }
    fn toggle_sidebar(&mut self, _host: &DomHandle) -> bool {
        // One window, one split: the host is implied. Same call the strip's own button makes,
        // so a dayscript walkthrough drives the real path (docs/toolbars.md).
        unsafe { day_dom_toolbar_sidebar() != 0 }
    }

    fn set_app_menu(&mut self, items: &[MenuItem]) {
        install_app_menu(items);
    }

    fn set_button_menu(&mut self, h: &DomHandle, _node: NodeId, f: day_spec::ContextMenuFn) {
        // The button's click already reaches `day_dom_event`, which opens the menu instead of
        // reporting a press. Announce the popup the way the app menu's button does.
        BUTTON_MENUS.with(|t| t.insert(h.0 as usize, f));
        attr(h.0, "aria-haspopup", "menu");
    }

    fn set_context_menu(&mut self, h: &DomHandle, node: NodeId, _items: &[MenuItem]) {
        // The browser has no native menu to hand day's model to: arm the `contextmenu`
        // listener and report `Event::ContextMenu`; the pieces layer presents the COMPOSED
        // menu from its own copy of the items (docs/menus.md), so none are stored here.
        remember(h.0, node);
        listen_once(h.0, gesture_mask::CONTEXT_MENU);
    }

    fn set_context_menu_fn(&mut self, h: &DomHandle, node: NodeId, _f: day_spec::ContextMenuFn) {
        // Summon-time menus take the same route as the static ones above: the event reaches
        // the piece that owns the provider, and the composed presenter calls it there.
        remember(h.0, node);
        listen_once(h.0, gesture_mask::CONTEXT_MENU);
    }

    fn supports_lifecycle(&self, phase: Lifecycle) -> bool {
        matches!(
            phase,
            Lifecycle::WillLaunch
                | Lifecycle::DidLaunch
                | Lifecycle::DidBecomeActive
                | Lifecycle::WillResignActive
                | Lifecycle::DidExit
        )
    }

    fn set_a11y(&mut self, h: &DomHandle, a11y: &A11yProps) {
        use day_spec::Role;
        if let Some(label) = &a11y.label {
            attr(h.0, "aria-label", label);
            APP_NAMED.with(|t| t.insert(h.0 as usize, ()));
        }
        // `aria-description`, not `title`: a title is also a tooltip, and the hint is for the
        // screen reader alone.
        if let Some(hint) = &a11y.hint {
            attr(h.0, "aria-description", hint);
        }
        // An explicit role retypes the element; a heading also carries its level. The shim's
        // `role` branch gives a retyped button its tab stop and keyboard activation.
        let role = match a11y.role {
            Role::None => None,
            Role::Button => Some("button"),
            Role::Toggle => Some("switch"),
            Role::Slider => Some("slider"),
            Role::TextInput => Some("textbox"),
            Role::Heading(_) => Some("heading"),
            Role::Image => Some("img"),
            Role::Meter => Some("meter"),
            Role::Group => Some("group"),
            Role::Tree => Some("tree"),
            Role::TreeItem => Some("treeitem"),
            Role::Tab => Some("tab"),
            Role::TabList => Some("tablist"),
        };
        if let Some(role) = role {
            attr(h.0, "role", role);
        }
        if let Role::Heading(level) = a11y.role {
            attr(h.0, "aria-level", &level.to_string());
        }
        if let Some(on) = a11y.selected {
            attr(h.0, "aria-selected", if on { "true" } else { "false" });
        }
        // A range role reads its value through `aria-valuetext`, with `aria-valuenow` beside it
        // when the text leads with a number ("72%" is 72). Any other role has no value
        // attribute, so the value rides the description, after the hint when there is one.
        if let Some(value) = &a11y.value {
            match a11y.role {
                Role::Meter | Role::Slider => {
                    attr(h.0, "aria-valuetext", value);
                    if let Some(n) = leading_number(value) {
                        attr(h.0, "aria-valuenow", &n.to_string());
                    }
                }
                _ => {
                    let described = match &a11y.hint {
                        Some(hint) => format!("{hint}. {value}"),
                        None => value.clone(),
                    };
                    attr(h.0, "aria-description", &described);
                }
            }
        }
        // Hidden takes the subtree out of the accessibility tree; the shim's `aria-hidden`
        // branch also takes it out of the tab order, which the attribute alone would not.
        if a11y.hidden || a11y.decorative {
            attr(h.0, "aria-hidden", "true");
        }
        // The Day element id (`.id(day_core::id!(counter_label))`) becomes the DOM id — the same duty that
        // sets accessibilityIdentifier on Apple backends. Day ids are app-unique by contract
        // (dayscript addresses by them), so they satisfy the DOM's uniqueness rule too. Only
        // nodes with native handles get one, exactly like every other backend.
        if let Some(id) = &a11y.identifier {
            attr(h.0, "id", id);
        }
    }

    fn announce(&mut self, text: &str, urgent: bool) {
        // The shim's live regions (docs/accessibility.md): one per politeness, so an urgent
        // sentence interrupts and a polite one waits its turn.
        unsafe { day_dom_announce(text.as_ptr(), text.len(), u32::from(urgent)) };
    }

    /// The element read back off the page, so `a11y_audit` and `assert_native` run on the web
    /// too. The DOM is the accessibility tree here: what `set_a11y` wrote is what a screen
    /// reader reads, and an element the page lost answers `found = false`. The rest is the
    /// control's own state as the browser holds it (the shim's `day_dom_read_native` says which
    /// property backs which field); a field the element does not carry stays `None`.
    fn read_native(&self, h: &DomHandle) -> day_spec::NativeSnapshot {
        use day_spec::Role;
        let el = h.0;
        let raw = read_buffer(|out, cap| unsafe { day_dom_read_native(el, out, cap) });
        if raw.is_empty() {
            return day_spec::NativeSnapshot::default();
        }
        // Twelve fields before the text, which goes last and whole: a field's contents may
        // hold the separator itself.
        let mut parts = raw.splitn(13, '\u{1f}');
        let mut next = || parts.next().filter(|s| !s.is_empty()).map(str::to_owned);
        let role = match next().as_deref() {
            Some("button") => Role::Button,
            Some("switch") => Role::Toggle,
            Some("slider") => Role::Slider,
            Some("textbox") => Role::TextInput,
            // The level is a second attribute and the audit ignores it.
            Some("heading") => Role::Heading(0),
            Some("img") => Role::Image,
            Some("meter") => Role::Meter,
            Some("group") => Role::Group,
            Some("tree") => Role::Tree,
            Some("treeitem") => Role::TreeItem,
            Some("tab") => Role::Tab,
            Some("tablist") => Role::TabList,
            _ => Role::None,
        };
        let label = next();
        let value = next();
        let identifier = next();
        let number = next().and_then(|s| s.parse::<f64>().ok());
        let flag = |s: Option<String>| match s.as_deref() {
            Some("1") => Some(true),
            Some("0") => Some(false),
            _ => None,
        };
        let checked = flag(next());
        let enabled = flag(next());
        let visible = flag(next());
        let mut coord = || next().and_then(|s| s.parse::<f64>().ok());
        let (x, y, w, hgt) = (coord(), coord(), coord(), coord());
        let frame = match (x, y, w, hgt) {
            (Some(x), Some(y), Some(w), Some(hgt)) => Some(Rect::new(x, y, w, hgt)),
            _ => None,
        };
        // "=" marks text the element shows, so an empty field still reads as `Some("")`.
        let text = next().and_then(|s| s.strip_prefix('=').map(str::to_owned));
        day_spec::NativeSnapshot {
            found: true,
            role,
            label,
            value,
            identifier,
            text,
            number,
            checked,
            enabled,
            visible,
            frame,
        }
    }

    /// Hand the bytes to the browser's own decoder (docs/images.md).
    ///
    /// `createImageBitmap` is ASYNCHRONOUS, so nothing is answered here; the shim calls
    /// `day_dom_image_decoded` when it resolves. Every other backend completes inline — this is
    /// the one the request-id design was actually needed for.
    fn decode_image(&mut self, req: u64, id: day_spec::BitmapId, bytes: &[u8]) {
        // Seeded now because the browser reports neither: an `ImageBitmap` has no format and no
        // way to ask about alpha. Unknown counts as "may have alpha" — compositing an opaque
        // image as if it had one is harmless, the reverse loses transparency.
        let format = day_spec::ImageFormat::sniff(bytes);
        let has_alpha = !matches!(
            format,
            Some(day_spec::ImageFormat::Jpeg | day_spec::ImageFormat::Bmp)
        );
        BITMAP_INFO.with(|m| {
            m.borrow_mut().insert(
                id.0,
                day_spec::BitmapInfo {
                    pixels: Size::new(0.0, 0.0),
                    // Decoded bytes carry no density — a PNG is simply its pixels.
                    scale: 1.0,
                    format,
                    has_alpha,
                },
            )
        });
        // SAFETY: the shim copies the span out of wasm memory before the first await point.
        unsafe { day_dom_image_decode(req as u32, id.0 as f64, bytes.as_ptr(), bytes.len()) };
    }

    fn image_info(&mut self, id: day_spec::BitmapId) -> Option<day_spec::BitmapInfo> {
        BITMAP_INFO.with(|m| m.borrow().get(&id.0).copied())
    }

    fn encode_image(&mut self, req: u64, id: day_spec::BitmapId, spec: &day_spec::EncodeSpec) {
        use day_spec::ImageFormat as F;
        let refuse = |err: day_spec::ImageError| {
            emit(
                day_spec::WINDOW_NODE,
                Event::ImageEncoded {
                    req,
                    result: Err(err),
                },
            );
        };
        // An id this backend never decoded, or has released, is `Gone` here as everywhere else.
        if !BITMAP_INFO.with(|m| m.borrow().contains_key(&id.0)) {
            refuse(day_spec::ImageError::Gone);
            return;
        }
        let mime = match spec.format {
            F::Png => "image/png",
            F::Jpeg => "image/jpeg",
            F::Webp => "image/webp",
            // A canvas writes no TIFF, GIF or BMP; `encode_formats` says so before an app asks.
            _ => {
                refuse(day_spec::ImageError::Encode);
                return;
            }
        };
        // Refuse a type this engine cannot write rather than let `toBlob` substitute PNG for it.
        // SAFETY: `mime` is a 'static str the shim reads and does not keep.
        if unsafe { day_dom_image_can_encode(mime.as_ptr(), mime.len()) } == 0 {
            refuse(day_spec::ImageError::Encode);
            return;
        }
        let (fit_w, fit_h) = spec.fit.map_or((0.0, 0.0), |f| (f.width, f.height));
        // -1 means "the type's own default", which is what `toBlob` does with an absent quality.
        let quality = spec.quality.map_or(-1.0, |q| q.clamp(0.0, 1.0));
        // SAFETY: `mime` is a 'static str; the shim reads `fmt_len` bytes and copies what it keeps.
        unsafe {
            day_dom_image_encode(
                req as u32,
                id.0 as f64,
                mime.as_ptr(),
                mime.len(),
                quality,
                fit_w,
                fit_h,
            );
        };
    }

    /// What a canvas WRITES (`toBlob` / `convertToBlob`). A browser READS far more than this —
    /// GIF, BMP, often AVIF — and the asymmetry is the platform's own, which is why this duty
    /// exists rather than letting `Cap::ImageEncode` imply everything decodable is encodable.
    fn encode_formats(&mut self) -> Vec<day_spec::ImageFormat> {
        use day_spec::ImageFormat::{Jpeg, Png, Webp};
        // Asked of the engine, not assumed: WebKit — the browser this project tests with — reads
        // WebP but does not write it, and `toBlob` would silently hand back PNG instead.
        [Png, Jpeg, Webp]
            .into_iter()
            .filter(|f| {
                let mime = f.mime();
                // SAFETY: a 'static str the shim reads and does not keep.
                unsafe { day_dom_image_can_encode(mime.as_ptr(), mime.len()) != 0 }
            })
            .collect()
    }

    fn release_image(&mut self, id: day_spec::BitmapId) {
        BITMAP_INFO.with(|m| {
            m.borrow_mut().remove(&id.0);
        });
        // SAFETY: an id the shim either knows or does not; releasing an absent one is a no-op.
        unsafe { day_dom_image_release(id.0 as f64) };
    }

    /// Replay a drawing into an offscreen bitmap (docs/canvas.md "Exporting") through the same
    /// painter the on-screen canvas uses. Like a decode, the browser answers on a later turn
    /// (`createImageBitmap` is a promise), through `day_dom_bitmap_rendered`.
    fn render_bitmap(
        &mut self,
        req: u64,
        id: day_spec::BitmapId,
        ops: &[DrawOp],
        size: Size,
        scale: f64,
    ) {
        let (pw, ph) = day_spec::export_pixels(size, scale);
        // Seeded now, as a decode is: the answer only confirms it or takes it back out.
        BITMAP_INFO.with(|m| {
            m.borrow_mut().insert(
                id.0,
                day_spec::BitmapInfo {
                    pixels: Size::new(f64::from(pw), f64::from(ph)),
                    scale,
                    format: None,
                    has_alpha: true,
                },
            )
        });
        let (buf, strs) = encode_ops(ops);
        // SAFETY: both spans outlive the call, and the shim paints from them synchronously
        // before its first await point.
        unsafe {
            day_dom_canvas_render_bitmap(
                req as u32,
                id.0 as f64,
                buf.as_ptr(),
                buf.len(),
                strs.as_ptr(),
                strs.len(),
                f64::from(pw),
                f64::from(ph),
                scale,
            );
        };
    }

    fn request_frame(
        &mut self,
        _host: &DomHandle,
        cb: day_spec::FrameCallback,
    ) -> day_spec::CancelFrame {
        let token = day_core::frame::native::register(cb);
        let request = unsafe { day_dom_request_frame(token as f64) };
        Box::new(move || {
            day_core::frame::native::cancel(token);
            unsafe { day_dom_cancel_frame(request) };
        })
    }

    fn replay(&mut self, h: &DomHandle, ops: &[DrawOp], size: Size) {
        let (buf, strs) = encode_ops(ops);
        unsafe {
            day_dom_canvas_replay(
                h.0,
                buf.as_ptr(),
                buf.len(),
                strs.as_ptr(),
                strs.len(),
                size.width,
                size.height,
            );
        };
    }

    /// The shim's list (docs/fonts.md): CSS generic families with synthesized faces (a browser
    /// synthesizes bold and italic for any family) plus each bundled `FontFace`, read by the
    /// same grow-and-retry protocol as `env`.
    fn font_families(&mut self) -> Vec<day_spec::FontFamilyInfo> {
        let mut cap = 4096usize;
        let text = loop {
            let mut buf = vec![0u8; cap];
            let n = unsafe { day_dom_fonts(buf.as_mut_ptr(), buf.len()) };
            if n > cap {
                cap = n;
                continue;
            }
            if n == cap {
                cap *= 2;
                continue;
            }
            buf.truncate(n);
            break String::from_utf8_lossy(&buf).into_owned();
        };
        day_spec::parse_font_list(&text)
    }

    /// `CanvasRenderingContext2D.measureText` with the CSS font the replay draws in
    /// (docs/fonts.md).
    fn measure_text(
        &mut self,
        text: &str,
        size: f64,
        font: &day_spec::CanvasFont,
    ) -> Option<day_spec::TextMetrics> {
        let family = font.family_str();
        let mut out = [0.0f64; 8];
        unsafe {
            day_dom_canvas_measure_text(
                text.as_ptr(),
                text.len(),
                size,
                u32::from(font.css_weight()),
                u32::from(font.italic),
                family.as_ptr(),
                family.len(),
                out.as_mut_ptr(),
            );
        };
        Some(day_spec::TextMetrics::from_slots(&out))
    }

    fn prepare_snapshot(
        &mut self,
        host: Option<&Self::Handle>,
        revision: u32,
    ) -> Result<day_spec::capture::Readiness, String> {
        Ok(
            if unsafe { day_dom_prepare_snapshot(host.map_or(0, |h| h.0), revision) } != 0 {
                day_spec::capture::Readiness::Ready
            } else {
                day_spec::capture::Readiness::Pending
            },
        )
    }

    /// Settled once no shown list is still gliding to where Day scrolled it and every shown list
    /// has reported the viewport it shows (`list_settled`): a scripted step after "scroll to
    /// row" or a row change then sees the rows the list shows now.
    fn ui_idle(&mut self) -> bool {
        list_settled()
    }

    fn modifiers(&mut self) -> day_spec::Modifiers {
        let mask = unsafe { day_dom_modifiers() };
        // The shim's mask uses the wire bits (`Modifiers::bits`).
        day_spec::Modifiers::from_bits((mask & 7) as u8)
    }

    fn present(&mut self, req: u64, spec: &PresentSpec) {
        // A save flow carries its staged bytes out with the request — the shim wraps them in a
        // Blob and clicks a download link, the browser's native "save" (docs/files.md).
        #[cfg(all(target_family = "wasm", target_os = "unknown"))]
        if let PresentSpec::SaveFile {
            title,
            suggested_name,
            src_path,
            ..
        } = spec
        {
            let bytes = day_spec::present::web_files::read(src_path).unwrap_or_default();
            let mut j = String::from("{\"kind\":\"save\",\"title\":");
            json_str(&mut j, title);
            j.push_str(",\"name\":");
            json_str(&mut j, suggested_name);
            j.push('}');
            unsafe {
                day_dom_present_save(req as u32, j.as_ptr(), j.len(), bytes.as_ptr(), bytes.len());
            };
            return;
        }
        let json = present_json(spec);
        if let Some(j) = json {
            unsafe { day_dom_present(req as u32, j.as_ptr(), j.len()) }
        } else {
            // Unsupported spec: answer dismissed so the await resolves.
            let node = day_spec::WINDOW_NODE;
            emit(
                node,
                Event::PresentResult {
                    req,
                    result: PresentResult::Dismissed,
                },
            );
        }
    }

    fn dismiss(&mut self, req: u64) {
        unsafe { day_dom_dismiss(req as u32) };
    }

    fn share_support(&self) -> day_spec::Support {
        if unsafe { day_dom_share_support() } {
            day_spec::Support::Native
        } else {
            day_spec::Support::Unsupported
        }
    }
    fn share(&mut self, spec: &day_spec::share::ShareSpec, _anchor: Option<&Self::Handle>) -> bool {
        let (url, title) = (&spec.url, &spec.title);
        unsafe { day_dom_share_url(url.as_ptr(), url.len(), title.as_ptr(), title.len()) }
    }
    fn open_url(&mut self, url: &str) {
        unsafe { day_dom_open_url(url.as_ptr(), url.len()) };
    }

    fn system_color(&mut self, role: day_spec::SystemColor) -> day_spec::Color {
        let key = format!("system_color_{}", role as u8);
        let rgba: Vec<f64> = env(&key)
            .split(',')
            .filter_map(|s| s.parse().ok())
            .collect();
        if let [r, g, b, a] = rgba.as_slice() {
            day_spec::Color::rgba(*r, *g, *b, *a)
        } else {
            role.fallback(self.dark_mode())
        }
    }

    fn dark_mode(&mut self) -> bool {
        DARK.with(std::cell::Cell::get)
    }

    /// The browser's `prefers-reduced-motion: reduce` (docs/accessibility.md), read live: the
    /// shim's media query answers, and its `change` event reaches [`day_dom_motion_changed`].
    fn reduce_motion(&mut self) -> bool {
        env("reduce_motion") == "1"
    }

    fn set_appearance(&mut self, dark: Option<bool>) {
        APPEARANCE.with(|a| a.set((dark, a.get().1)));
        apply_appearance();
    }

    fn adopt(&mut self, raw: day_spec::RawHandle) -> DomHandle {
        DomHandle(raw as u32)
    }
}

thread_local! {
    static MEASURE_CACHE: RefCell<HashMap<(u32, i64), Size>> = RefCell::new(HashMap::new());
}

impl Platform for Dom {
    const TARGET: &'static str = "web-dom";
    const TOOLKIT: &'static str = "dom";
    // The host owns the loop: `run` hands back at once, and DidExit is its last callback.
    const RUN_ENDS_APP: bool = false;

    fn run(self, options: WindowOptions, ready: Box<dyn FnOnce(Self, DomHandle, Size)>) {
        unsafe { day_dom_set_title(options.title.as_ptr(), options.title.len()) };
        // Before the viewport is read: the Standard chrome's rules can move the root down.
        // Frameless, the backgrounds, placement and resizability are the browser's to decide.
        let overlay = options.chrome == day_spec::WindowChrome::Overlay;
        let inset = unsafe { day_dom_window_chrome(u32::from(overlay)) };
        // The root's content reads its safe area while it builds, before any tree exists to
        // report into: seed it, as the first window's inset.
        if overlay && inset > 0.0 {
            log::debug!("web: title bar overlay inset {inset} at boot");
            day_core::seed_safe_area(day_spec::Insets {
                top: inset,
                ..Default::default()
            });
        }
        let w: f64 = env("vw").parse().unwrap_or(1000.0);
        let h: f64 = env("vh").parse().unwrap_or(700.0);
        LAST_VIEWPORT.with(|v| v.set(Size::new(w, h)));
        DARK.with(|d| d.set(env("dark") == "1"));
        // Root container: the shim pre-registers the `#day-root` element under this id.
        let root = self.root;
        ready(self, DomHandle(root), Size::new(w, h));
        // `ready` ran the app's root(), so its registrations are in. An app with its own
        // `app_menu` model reached the strip through `set_app_menu` with Settings…/About
        // injected by day-core; one without still gets the button when something registered,
        // the way a desktop's default bar carries those items (day-toolkit-appkit rebuilds its default
        // menu at this same point).
        if !day_core::has_app_menu() {
            install_app_menu(&registration_menu());
        }
    }

    fn post(f: Box<dyn FnOnce() + Send>) {
        POSTED.with(|q| q.borrow_mut().push_back(f));
        unsafe { day_dom_schedule_post() };
    }

    fn post_delayed(ms: u32, f: Box<dyn FnOnce() + Send>) {
        let token = NEXT_DELAY.with(|c| {
            let t = c.get();
            c.set(t.wrapping_add(1).max(1));
            t
        });
        DELAYED.with(|m| m.borrow_mut().insert(token, f));
        unsafe { day_dom_schedule_delayed(token, ms) };
    }

    fn locale_hints(&self) -> Vec<String> {
        env("locales")
            .split(',')
            .map(str::to_owned)
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// Post onto the (single-threaded) main loop without the `Send` bound.
fn post_local(f: impl FnOnce() + 'static) {
    // SAFETY-BY-CONSTRUCTION: wasm is single-threaded — the queue never crosses threads;
    // the Send bound on the queue is satisfied by wrapping in a type that is only ever
    // touched on this thread.
    struct NotSendButFine(Option<Box<dyn FnOnce()>>);
    unsafe impl Send for NotSendButFine {}
    let wrapped = NotSendButFine(Some(Box::new(f)));
    Dom::post(Box::new(move || {
        // Move the whole wrapper (not just its field) so the closure captures the Send type.
        let mut w = wrapped;
        if let Some(f) = w.0.take() {
            f();
        }
    }));
}

// ---------------------------------------------------------------------------
// Kind helpers
// ---------------------------------------------------------------------------

fn set_enabled(el: u32, enabled: bool) {
    attr(el, "disabled", if enabled { "" } else { "-" });
}

/// A surface's styling on its element (docs/surfaces.md): fill, corners, clipping, border and
/// shadow. Idempotent: every property it owns is written and an absent one cleared, so a
/// `ContainerPatch::Surface` runs it again on the realized element. `role` is the realize-time
/// one ([`SURFACE_ROLES`]); a patch's own `role` is ignored. `anim` animates a solid fill;
/// `fresh` is the realize (see [`apply_fill`]).
fn apply_surface(
    el: u32,
    p: &ContainerProps,
    role: Option<day_spec::SurfaceRole>,
    anim: Option<&AnimSpec>,
    fresh: bool,
) {
    // A fresh element has nothing to clear: skip the empty writes.
    let set = |prop: &str, value: &str| {
        if !(fresh && value.is_empty()) {
            s(el, prop, value);
        }
    };
    let flag = |name: &str, on: bool| {
        if on || !fresh {
            class(el, name, on);
        }
    };
    let radius = p.corner_radius;
    match role {
        // The dimming under a composed sheet: the stylesheet frosts the page (`.day-scrim`,
        // day.css) instead of laying the plain color over it.
        Some(day_spec::SurfaceRole::Scrim) => class(el, "day-scrim", true),
        // The window's own surface: the page background a cover paints (`.day-cover`, day.css).
        Some(day_spec::SurfaceRole::Window) => s(el, "background-color", "var(--day-bg)"),
        _ => apply_fill(el, p.fill.as_ref(), anim, fresh),
    }
    // A shadowed surface does not clip (docs/surfaces.md): the shadow itself would survive
    // `overflow: hidden`, which cuts descendants only, but the rule is that content is not cut.
    let cuts = p.clips || (radius > 0.0 && p.shadow.is_none());
    let radii = if radius <= 0.0 {
        String::new()
    } else if p.corners == day_spec::Corners::ALL {
        format!("{radius}px")
    } else {
        let r = p.corners.radii(radius);
        format!(
            "{}px {}px {}px {}px",
            r.top_left, r.top_right, r.bottom_right, r.bottom_left
        )
    };
    set("border-radius", &radii);
    // A rounded clip inside a transformed (animated) ancestor hits a WebKit bug: the clip
    // layer's backing paints its SHARP bounding box black behind the rounded content. The
    // `.day-xform .day-clip` rule (day.css) gives such a clip its own correctly-clipped layer;
    // the class is inert everywhere else, so static rounded surfaces aren't promoted.
    flag("day-clip", radius > 0.0 && cuts);
    set("overflow", if cuts { "hidden" } else { "" });
    if role == Some(day_spec::SurfaceRole::SectionCard) {
        class(el, "day-card", true);
    }
    // The border is a layer of its own over the surface AND its content (`.day-bordered::after`,
    // day.css: the last box painted in the element) rather than a CSS `border`: children are
    // placed from the padding box, so a real border would move every one of them inward by its
    // width, and it would be painted under them.
    if let Some(b) = p.border.filter(|b| b.width > 0.0) {
        s(el, "--day-border-width", &format!("{}px", b.width));
        s(el, "--day-border-color", &ink_css(b.color));
        class(el, "day-bordered", true);
    } else {
        flag("day-bordered", false);
        set("--day-border-width", "");
        set("--day-border-color", "");
    }
    // `box-shadow` is the spec's shadow exactly: the blur radius is the same number, it follows
    // the per-corner radii, it paints outside the border box only, takes no layout room and no
    // pointer events.
    //
    // The second, invisible layer is for WebKit: a single blurred shadow in an opaque color
    // under an opaque fill takes a fast path there that paints the FILL color across the
    // shadow's middle, leaving only a rim of shadow. Two layers take the general path.
    let shadow = p.shadow.map_or_else(String::new, |sh| {
        format!(
            "{}px {}px {}px {},0 0 0 transparent",
            sh.dx,
            sh.dy,
            sh.blur.max(0.0),
            ink_css(sh.color)
        )
    });
    set("box-shadow", &shadow);
}

/// A surface's fill (`ContainerProps::fill`, `ContainerPatch::Fill`): a solid ink is the
/// background color, animated with `anim` (docs/tween.md rule 8) and written as the ink's own
/// CSS so it follows the appearance; a gradient is the background image, with the color
/// cleared; `None` clears both, back to what the stylesheet gives the element. `fresh` is a
/// just-created element, which has nothing to clear and no animation to cancel.
fn apply_fill(el: u32, fill: Option<&day_spec::Paint>, anim: Option<&AnimSpec>, fresh: bool) {
    use day_spec::Paint;
    match fill {
        Some(Paint::Solid(ink)) if fresh => s(el, "background-color", &ink_css(*ink)),
        Some(Paint::Solid(ink)) => {
            // From the color shown to the new one under the whole contract; the shim writes
            // the resolved target and runs from what is displayed. A dynamic ink then replaces
            // the inline target with its own CSS, which the running animation overrides until
            // it ends.
            animate(
                el,
                anim_prop::BACKGROUND,
                &color_parts(ink.resolved()),
                anim,
            );
            if ink.is_dynamic() {
                s(el, "background-color", &ink_css(*ink));
            }
        }
        _ if fresh => {}
        _ => {
            animate(el, anim_prop::BACKGROUND, &[0.0; 4], None);
            s(el, "background-color", "");
        }
    }
    // A linear gradient depends on the frame (`linear_gradient_css`), so `set_frame` writes it
    // again at every size; radial and conic ones are given in fractions of the box.
    let image = match fill {
        Some(Paint::Linear(g)) => {
            let size = SURFACE_GRADIENTS
                .with(|t| t.get(el as usize))
                .map_or(Size::new(1.0, 1.0), |(_, size)| size);
            SURFACE_GRADIENTS.with(|t| t.insert(el as usize, (g.clone(), size)));
            linear_gradient_css(g, size)
        }
        Some(Paint::Radial(g)) => radial_gradient_css(g),
        Some(Paint::Conic(g)) => conic_gradient_css(g),
        Some(Paint::Solid(_)) | None => String::new(),
    };
    if !matches!(fill, Some(Paint::Linear(_))) {
        SURFACE_GRADIENTS.with(|t| t.remove(el as usize));
    }
    if !(fresh && image.is_empty()) {
        s(el, "background-image", &image);
    }
}

/// Gradient stops as CSS color stops: each stop's ink (following the appearance) at its offset
/// as a percentage of the gradient line or ray.
fn css_stops(stops: &[(f64, day_spec::Ink)]) -> String {
    stops
        .iter()
        .map(|(off, c)| format!("{} {}%", ink_css(*c), off.clamp(0.0, 1.0) * 100.0))
        .collect::<Vec<_>>()
        .join(",")
}

/// A surface's radial gradient as CSS (docs/surfaces.md): an ellipse whose radii are `radius` of
/// the box's width and of its height, so it is a circle in a square frame and stretches with a
/// non-square one, as the unit space of the frame does. The stops run along that radius, and the
/// last color holds beyond it.
fn radial_gradient_css(g: &day_spec::RadialGradient) -> String {
    if g.stops.len() < 2 {
        return flat_gradient_css(&g.stops);
    }
    let r = g.radius.max(0.0) * 100.0;
    format!(
        "radial-gradient(ellipse {r}% {r}% at {}% {}%,{})",
        g.center.x * 100.0,
        g.center.y * 100.0,
        css_stops(&g.stops)
    )
}

/// A surface's conic gradient as CSS (docs/surfaces.md). Day's angles turn clockwise from +x; a
/// CSS conic gradient's turn clockwise from up, so the start moves a quarter turn on.
fn conic_gradient_css(g: &day_spec::ConicGradient) -> String {
    if g.stops.len() < 2 {
        return flat_gradient_css(&g.stops);
    }
    format!(
        "conic-gradient(from {}deg at {}% {}%,{})",
        g.start.degrees() + 90.0,
        g.center.x * 100.0,
        g.center.y * 100.0,
        css_stops(&g.stops)
    )
}

/// A gradient with nothing to run between: one flat color, the last stop's (none if empty).
fn flat_gradient_css(stops: &[(f64, day_spec::Ink)]) -> String {
    let c = stops
        .last()
        .map_or_else(|| color_css(day_spec::Color::CLEAR), |s| ink_css(s.1));
    format!("linear-gradient({c},{c})")
}

/// A surface's gradient as a CSS `linear-gradient(…)` for a box of `size`.
///
/// CSS places a gradient by an ANGLE through the box's center, with a length that makes the
/// end colors land in the corners; the spec places it by two unit points resolved against the
/// frame, which is how a canvas gradient is given. So this resolves the points, takes the angle
/// of the line through them, and moves each stop to where it falls on the CSS gradient line
/// (positions outside 0–100% are allowed, and the end colors hold flat beyond them as they do
/// on a canvas). The answer changes with the box's shape for any gradient that is not
/// axis-aligned, which is why `set_frame` asks again.
fn linear_gradient_css(g: &day_spec::LinearGradient, size: Size) -> String {
    let (w, h) = (
        if size.width > 0.0 { size.width } else { 1.0 },
        if size.height > 0.0 { size.height } else { 1.0 },
    );
    let (x0, y0) = (g.start.x * w, g.start.y * h);
    let (dx, dy) = (g.end.x * w - x0, g.end.y * h - y0);
    let len = dx.hypot(dy);
    if len < 1e-9 || g.stops.len() < 2 {
        // No direction, or nothing to run between: one flat color, the last stop's.
        return flat_gradient_css(&g.stops);
    }
    let (ux, uy) = (dx / len, dy / len);
    // CSS: 0deg points up and angles turn clockwise, and the line's length is
    // |w·sin| + |h·cos|, centered on the box.
    let angle = ux.atan2(-uy).to_degrees();
    let line = (w * ux).abs() + (h * uy).abs();
    let t0 = ((x0 - w / 2.0) * ux + (y0 - h / 2.0) * uy + line / 2.0) / line;
    let span = len / line;
    let stops: Vec<String> = g
        .stops
        .iter()
        .map(|(off, c)| {
            let at = (t0 + off.clamp(0.0, 1.0) * span) * 100.0;
            format!("{} {at}%", ink_css(*c))
        })
        .collect();
    format!("linear-gradient({angle}deg,{})", stops.join(","))
}

fn apply_area_attrs(el: u32, editable: bool, selectable: bool, spellcheck: bool) {
    if !editable {
        attr(el, "readonly", "-");
    }
    if !selectable {
        s(el, "user-select", "none");
    }
    attr(el, "spellcheck", if spellcheck { "true" } else { "false" });
}

fn realize_picker(p: &PickerProps) -> u32 {
    let json = picker_json(p);
    // Intrinsic size from the option strings — measuring the element itself would concatenate
    // every option's text into one line.
    let longest = p
        .options
        .iter()
        .map(|o| measure_str(o).width)
        .fold(0.0f64, f64::max);
    let n = p.options.len() as f64;
    let el = match p.style {
        PickerStyle::Menu => {
            let el = unsafe { day_dom_create(EL_SELECT) };
            PICKER_SELECTED.with(|m| m.borrow_mut().insert(el, p.selected));
            unsafe { day_dom_options(el, json.as_ptr(), json.len()) }; // shim fills <option>s
            unsafe { day_dom_listen(el, 4) };
            PICKER_SIZE.with(|m| m.borrow_mut().insert(el, Size::new(longest + 38.0, 26.0)));
            el
        }
        PickerStyle::Segmented | PickerStyle::Inline => {
            let segmented = p.style == PickerStyle::Segmented;
            let el = unsafe { day_dom_create(if segmented { EL_SEGMENTED } else { EL_RADIOS }) };
            PICKER_SELECTED.with(|m| m.borrow_mut().insert(el, p.selected));
            unsafe { day_dom_options(el, json.as_ptr(), json.len()) };
            SEG_COUNT.with(|m| m.borrow_mut().insert(el, p.options.len()));
            let size = if segmented {
                let total: f64 = p.options.iter().map(|o| measure_str(o).width + 28.0).sum();
                Size::new(total + 4.0, 28.0)
            } else {
                Size::new(longest + 28.0, n * 24.0 + (n - 1.0).max(0.0) * 4.0)
            };
            PICKER_SIZE.with(|m| m.borrow_mut().insert(el, size));
            el
        }
    };
    // After the options exist, so a segmented or inline group passes it to its buttons.
    if !p.enabled {
        set_enabled(el, false);
    }
    el
}

fn picker_json(p: &PickerProps) -> String {
    let mut json = String::from("{\"options\":[");
    for (i, o) in p.options.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        json_str(&mut json, o);
    }
    json.push_str("],\"selected\":");
    json.push_str(&p.selected.to_string());
    json.push('}');
    json
}

/// The properties the shim animates (`animateProp` in shim.js).
mod anim_prop {
    pub const OPACITY: u32 = 0;
    pub const TRANSFORM: u32 = 1;
    pub const BACKGROUND: u32 = 2;
    pub const FRAME: u32 = 3;
}

/// Set `prop` of `el` to `to`, playing the whole `AnimSpec` contract when `anim` is given
/// (docs/tween.md): Day's own samples of one cycle as the keyframes (two for `Linear`, which
/// needs no more), the delay, the cycles and the alternate direction. `None` sets at once and
/// cancels a run on that property.
fn animate(el: u32, prop: u32, to: &[f64], anim: Option<&AnimSpec>) {
    let (samples, duration, delay, iterations, alternate) = match anim {
        Some(a) if a.duration_ms > 0 => {
            let samples: Vec<f64> = if matches!(a.curve, Curve::Linear) {
                vec![0.0, 0.0, 1.0, 1.0]
            } else {
                a.keyframes()
                    .iter()
                    .flat_map(|k| [k.offset, k.progress])
                    .collect()
            };
            let iterations = a.cycles().map_or(-1.0, f64::from);
            (
                samples,
                f64::from(a.duration_ms),
                f64::from(a.delay_ms),
                iterations,
                u32::from(a.autoreverse),
            )
        }
        _ => (Vec::new(), 0.0, 0.0, 1.0, 0),
    };
    // SAFETY: both slices outlive the call; the shim copies them before returning.
    unsafe {
        day_dom_animate(
            el,
            prop,
            to.as_ptr(),
            to.len(),
            samples.as_ptr(),
            samples.len(),
            duration,
            delay,
            iterations,
            alternate,
        );
    }
}

/// A color as the shim's `[r, g, b, a]`.
fn color_parts(c: day_spec::Color) -> [f64; 4] {
    [c.r, c.g, c.b, c.a]
}

fn nav_patch(el: u32, p: &NavPatch) {
    // Re-present rebuilds chrome and re-homes pages — it needs the state map unborrowed while it
    // calls back into the shim, so it runs outside the borrow below.
    if let NavPatch::Presentation(next) = p {
        nav_present(el, *next);
        return;
    }
    NAV_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let Some(state) = m.get_mut(&el) else { return };
        match p {
            // Handled above, before the borrow.
            #[expect(
                clippy::match_same_arms,
                reason = "each group of no-op patches keeps its own arm and the comment saying why"
            )]
            NavPatch::Presentation(_) => {}
            // Never arrives: this backend answers `Cap::NavContentList` Unsupported, so the
            // pieces layer composes the pane itself (docs/navigation.md).
            #[expect(
                clippy::match_same_arms,
                reason = "each group of no-op patches keeps its own arm and the comment saying why"
            )]
            NavPatch::ListVisible(_) | NavPatch::ListInStack(_) => {}
            // Resident-page switch (docs/navigation.md): every page stays in the DOM and only
            // one is displayed, so a tab switch costs a `display` flip and keeps the other
            // tabs' scroll offsets and focused fields exactly as the user left them.
            NavPatch::Select(i) => {
                state.selected = *i;
                for (n, page) in state.pages.iter().enumerate() {
                    s(*page, "display", if n == *i { "block" } else { "none" });
                }
                sync_back_bar(el, state);
            }
            NavPatch::Pushed { title, .. } => {
                state.titles.push(title.clone());
                let last = state.pages.len().saturating_sub(1);
                state.selected = last;
                for (i, page) in state.pages.iter().enumerate() {
                    s(*page, "display", if i == last { "block" } else { "none" });
                }
                sync_back_bar(el, state);
            }
            NavPatch::Popped => {
                if state.titles.len() > 1 {
                    state.titles.pop();
                }
                let n = state.pages.len();
                if let Some(top) = state.pages.last() {
                    s(*top, "display", "none");
                }
                if n >= 2 {
                    s(state.pages[n - 2], "display", "block");
                }
                state.selected = n.saturating_sub(2);
                sync_back_bar_at(el, state, n.saturating_sub(1));
            }
            // The custom back bar routes back through Day; no native auto-pop to suppress.
            NavPatch::GuardTop(_) => {}
            NavPatch::Reorder(order) => {
                let old = state.pages.clone();
                state.pages = order.iter().filter_map(|i| old.get(*i).copied()).collect();
            }
            NavPatch::Title(t) => {
                if let Some(last) = state.titles.last_mut() {
                    last.clone_from(t);
                }
                sync_back_bar(el, state);
            }
        }
    });
}

fn sync_back_bar(el: u32, state: &NavState) {
    sync_back_bar_at(el, state, state.pages.len());
}

/// Re-present a live nav host (docs/size-classes.md): the window crossed a breakpoint, so the
/// chrome changes but the pages do not.
///
/// The whole point is that no page is rebuilt. The shim rebuilds the host's own chrome and leaves
/// the page elements detached-but-alive; this function then re-homes each one by its PANE, which
/// is the only thing that differs between the two presentations:
///
/// - `Split` — the sidebar page gets its own pane; `pages` holds detail pages alone.
/// - `Stack` — the sidebar page is the stack's root, so it heads `pages`.
/// - `Tabs` / `Rail` — the sidebar page moves into the CHROME slot, where its `NAV_MENU` lays
///   out as a bar or a strip. It is the same element with the same click handlers and the same
///   selection sync; only its container and its CSS class change, which is why a morph costs no
///   rebuilding on either side of the boundary.
fn nav_present(el: u32, next: NavPresentation) {
    let Some((sidebar, mut pages, was, selected)) = NAV_STATE.with(|m| {
        let st = m.borrow();
        let s = st.get(&el)?;
        Some((s.sidebar, s.pages.clone(), s.presentation, s.selected))
    }) else {
        return;
    };
    if was == next {
        return;
    }
    // Which page is on screen, by IDENTITY rather than index: moving the sidebar page in or out
    // of the detail list below shifts every index past it, so an index captured now would point
    // at the wrong page afterwards.
    let shown_node = pages.get(selected).copied();
    // The sidebar page is a stack ROOT only while stacked; everywhere else it is chrome or its
    // own pane, and so leaves the detail list.
    if let Some(side) = sidebar {
        pages.retain(|p| *p != side);
        if next == NavPresentation::Stack {
            pages.insert(0, side);
        }
    }
    unsafe { day_dom_nav_present(el, nav_mode(next)) };
    if let Some(side) = sidebar
        && next != NavPresentation::Stack
    {
        unsafe { day_dom_nav_add_page(el, side, 1) };
        // The chrome slot always shows its page; it may have been hidden as a stack root under
        // a pushed detail.
        s(side, "display", "block");
    }
    // The page the user is looking at STAYS the page the user is looking at. `selected` tracks
    // exactly that in every presentation — push and pop maintain it too — so carrying it across
    // is the whole of "re-present without rebuilding".
    //
    // Getting this from the page ORDER instead does not work when leaving a tab bar: the pieces
    // layer keeps the selected page and disposes the rest, so a backend that showed the last
    // page would hide the one survivor and the detail pane would come up empty.
    //
    // Its index in the REARRANGED list. Falling back to the last page covers the one case where
    // it is not there any more: a stack showing only its root, whose root has just left the
    // detail list to become a sidebar pane.
    let shown = shown_node
        .and_then(|n| pages.iter().position(|p| *p == n))
        .unwrap_or_else(|| pages.len().saturating_sub(1));
    for (i, page) in pages.iter().enumerate() {
        unsafe { day_dom_nav_add_page(el, *page, 0) };
        s(*page, "display", if i == shown { "block" } else { "none" });
    }
    // The back bar belongs to the stack presentation alone, and its visibility depends on the
    // page count we just settled — so commit the state first, then sync from it.
    NAV_STATE.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(st) = m.get_mut(&el) {
            st.presentation = next;
            st.pages = pages;
            st.selected = shown;
            sync_back_bar(el, st);
        }
    });
}

/// Stack presentation: the back bar shows while pushed pages are on top (`depth` counts the
/// pages that will remain after the in-flight patch). Every other presentation has its own way
/// out — a sidebar row, a tab — and never shows it.
fn sync_back_bar_at(el: u32, state: &NavState, depth: usize) {
    let visible =
        state.presentation == NavPresentation::Stack && depth >= 1 && state.titles.len() > 1;
    let title = state.titles.last().cloned().unwrap_or_default();
    unsafe { day_dom_nav_back_bar(el, u32::from(visible), title.as_ptr(), title.len()) };
}

// ---------------------------------------------------------------------------
// Emulated list (docs/list.md): cells over the ListSource pull contract, the Qt shape — and,
// like Qt and XAML, only the rows the viewport SHOWS are realized. That is the promise `list`
// makes over `each` ("builds only the rows the native widget currently shows"); building all of
// them is what a ten-thousand-row query cost before: ten thousand elements and ten thousand row
// layouts, in wasm, before the first paint. Cell index stays == row index for the cell's whole
// life, so nothing about selection or the click handler's row changes.
// ---------------------------------------------------------------------------

/// Rows built beyond each edge of the viewport, so a flick has something to show before the
/// scroll event lands.
const LIST_OVERSCAN: usize = 8;

/// A source change under the cells: everything realized is now showing the wrong row's data, so
/// mark it all dirty and refill the window. Reload and splice come through here — the callers
/// that used to rebind all n rows.
fn list_populate(host: u32) {
    LISTS.with(|m| {
        let mut m = m.borrow_mut();
        let Some(st) = m.get_mut(&host) else {
            return;
        };
        st.bound.iter_mut().for_each(|b| *b = false);
        // The pinned header may now be a different section's, or an item's row.
        st.pin_row = None;
        // A source that SHRANK leaves realized cells past its end. They stay in the pool (index
        // == row, so a source that grows back reuses each for the row it always held) and are
        // simply hidden — the same append-only pool, minus the eager building.
        let n = st.source.as_ref().map_or(0, |src| (src.len)());
        let recycle = st.source.as_ref().map(|src| src.recycle.clone());
        let stale: Vec<u32> = st
            .cells
            .iter()
            .skip(n)
            .copied()
            .filter(|c| *c != 0)
            .collect();
        drop(m);
        for cell in stale {
            s(cell, "display", "none");
            // A hidden row past the shrunk source keeps its pooled cell but must stop
            // answering dayscript lookups — clear its element ids (day-core's recycle).
            if let Some(recycle) = &recycle {
                recycle(cell as usize as day_spec::RawHandle);
            }
        }
    });
    list_fill_window(host);
}

/// Build the rows the viewport shows and that are not built already. Idempotent and cheap when
/// nothing moved, which is what lets every scroll event call it.
fn list_fill_window(host: u32) {
    let Some((content, rowh, source, work, n, width, separators, selectable, seen, tops)) = LISTS
        .with(|m| {
            let mut m = m.borrow_mut();
            let st = m.get_mut(&host)?;
            let source = st.source.clone()?;
            let (content, rowh, selectable) = (st.content, st.row_height.max(1.0), st.selectable);
            let separators = st.separators;
            let n = (source.len)();
            let width = unsafe { day_dom_width(host) }.max(1.0);
            // The rows on screen, plus the overscan. A list the browser has not laid out yet reports
            // no height — build a screen's worth then, and let the scroll that follows extend it.
            let mut view = [0.0_f64; 2];
            unsafe { day_dom_list_viewport(host, view.as_mut_ptr()) };
            let laid_out = view[1] > 0.0;
            let (offset, vh) = (view[0], if laid_out { view[1] } else { 600.0 });
            // A self-sizing list's rows stand where their heights add up to; a uniform list's
            // at multiples of the pitch, with no per-row table to build.
            let tops = st.stacked().then(|| st.tops(n));
            // (first visible, last visible): rows with any part in the viewport, overscan
            // excluded; `end` is one past the last.
            let (first_visible, end) = match &tops {
                Some(tops) => {
                    // The row whose span holds `offset`, and the first row starting below the
                    // viewport's bottom edge.
                    let at = tops[..n]
                        .partition_point(|t| *t <= offset)
                        .saturating_sub(1);
                    let end = tops[..n].partition_point(|t| *t < offset + vh);
                    (at.min(n.saturating_sub(1)), end)
                }
                None => (
                    ((offset / rowh).floor() as usize).min(n.saturating_sub(1)),
                    (((offset + vh) / rowh).ceil() as usize).min(n),
                ),
            };
            let (first, last) = (
                first_visible.saturating_sub(LIST_OVERSCAN),
                (end + LIST_OVERSCAN).min(n),
            );
            let extent = tops.as_ref().map_or(n as f64 * rowh, |t| t[n]);
            if laid_out {
                st.seen = Some((offset, vh, n));
            }
            // What the viewport report says (docs/list.md "Viewport"), for a laid-out list
            // with rows: the visible range and how far the content runs past the bottom edge.
            let seen = (laid_out && n > 0).then(|| day_spec::ListViewport {
                first: first_visible,
                last: end.saturating_sub(1).clamp(first_visible, n - 1),
                end_distance: (extent - (offset + vh)).max(0.0),
            });
            // Slots exist for every row (a Vec of zeros, not of elements): the cell for row i lives
            // at i for good, which is what keeps the click handler's recorded row honest.
            if st.cells.len() < n {
                st.cells.resize(n, 0);
                st.bound.resize(n, false);
            }
            if st.stacked() && st.heights.len() < n {
                st.heights.resize(n, 0.0);
            }
            // (row, cell, is a section header, is a supplementary row): both take their laid
            // height even in a uniform list, and neither is drawn or wired as an item.
            let mut work: Vec<(usize, u32, bool, bool)> = Vec::new();
            for i in first..last {
                if st.cells[i] == 0 {
                    let cell = unsafe { day_dom_create(EL_CELL) };
                    // Appended, not inserted at the row index: cells are absolutely framed, so
                    // document order says nothing about where a row appears — and a window filled
                    // out of order (scroll down, then back up) has no meaningful index to insert at.
                    unsafe { day_dom_insert(content, cell, u32::MAX) };
                    if selectable {
                        unsafe { day_dom_listen(cell, 1) };
                        // The role pairs with the host's `listbox` (day_dom_list_keynav): it is what
                        // makes the arrow keys below mean something to a screen reader, and what
                        // gives `aria-selected` somewhere to live.
                        attr(cell, "role", "option");
                        CELL_ROWS.with(|m| m.borrow_mut().insert(cell, (host, i)));
                    }
                    st.cells[i] = cell;
                }
                if !st.bound[i] {
                    st.bound[i] = true;
                    work.push((i, st.cells[i], st.is_header(i), st.is_supplement(i)));
                }
            }
            st.last_width = width;
            Some((
                content, rowh, source, work, n, width, separators, selectable, seen, tops,
            ))
        })
    else {
        return;
    };
    // Application observers can synchronously rebuild overlays and relayout this list.
    // Report only after releasing LISTS, just like bind_row below. Layout can hold the core
    // tree borrow, so the report goes out on the next loop turn, after both the tree and the
    // list registry are free. The full viewport report supersedes the first-row one: both feed
    // the same state, and the first-only one would overwrite the range with a single row.
    if let Some(seen) = seen
        && (source.viewport.is_some() || source.first_visible.is_some())
    {
        post_local(move || {
            let source = LISTS.with(|m| m.borrow().get(&host).and_then(|st| st.source.clone()));
            let Some(source) = source else { return };
            if let Some(report) = &source.viewport {
                report(seen);
            } else if let Some(report) = &source.first_visible {
                report(seen.first.min((source.len)().saturating_sub(1)));
            }
        });
    }
    let mut measured = false;
    let automatic = LISTS.with(|m| m.borrow().get(&host).is_some_and(|st| st.automatic));
    for &(i, cell, header, supplement) in &work {
        let (top, height) = match &tops {
            Some(t) => (t[i], t[i + 1] - t[i]),
            None => (i as f64 * rowh, rowh),
        };
        unsafe {
            day_dom_set_frame(cell, 0.0, top, width, height);
        }
        s(cell, "display", "block");
        // Cell index is row index, so a reload that moves the section breaks can turn an
        // item's cell into a header's and back: set (or clear) the header treatment every bind.
        if source.is_header.is_some() || source.is_supplement.is_some() {
            list_style_header(cell, header, selectable);
            list_style_supplement(cell, supplement);
        }
        if separators && !header && !supplement {
            // Inside the frame (border-box), so the line lands exactly on the row boundary
            // without growing the cell past its pitch.
            s(cell, "box-sizing", "border-box");
            s(
                cell,
                "border-bottom",
                "1px solid color-mix(in srgb, currentColor 18%, transparent)",
            );
        }
        (source.bind_row)(i, cell as usize as day_spec::RawHandle);
        // A self-sizing row takes the height Day just laid it at. `0.0` means the tree was
        // busy; the row keeps its estimate until a resize reaches it. A uniform list's items
        // keep the pitch, so only its headers are measured.
        if tops.is_some() && (automatic || header || supplement) {
            let laid = (source.cell_height)(cell as usize as day_spec::RawHandle);
            if laid > 0.0 {
                measured |= LISTS.with(|m| {
                    m.borrow_mut()
                        .get_mut(&host)
                        .and_then(|st| st.heights.get_mut(i))
                        .is_some_and(|h| {
                            let changed = (*h - laid).abs() > 0.25;
                            *h = laid;
                            changed
                        })
                });
            }
        }
    }
    s(content, "position", "relative");
    // The extent is the whole source, built or not: the scrollbar is how the user reaches rows
    // that do not exist yet, so it cannot be sized to what happens to be realized.
    match &tops {
        Some(t) => s(content, "height", &format!("{}px", t[n])),
        None => s(content, "height", &format!("{}px", n as f64 * rowh)),
    }
    // Measured rows moved every row after them, and rows the estimate placed off screen may
    // now be in view: restack, then fill again (a no-op once every row in view is built).
    if measured {
        list_restack(host);
        post_local(move || list_fill_window(host));
    }
    // A row jump that arrived before this extent existed lands now; its scroll event builds the
    // rows it reveals.
    let pending = LISTS.with(|m| {
        m.borrow_mut()
            .get_mut(&host)
            .and_then(|st| st.pending_scroll.take())
    });
    if let Some(y) = pending {
        unsafe { day_dom_scroll_to(host, 0.0, y, 0) };
    }
    // Rows realized just now start unpainted, and a reload can move which rows are selected
    // under a selection that never changed — so repaint from the entry's set on every fill.
    LISTS.with(|m| {
        if let Some(st) = m.borrow().get(&host) {
            list_paint_selection(st);
        }
    });
    list_pin_header(host);
}

/// The look and wiring of a sectioned list's row: a header reads as a group heading (an opaque
/// band in the sidebar tone, which also beats the stylesheet's row hover), carries the heading
/// role instead of `option`, and draws no separator; an item gets its row role back.
fn list_style_header(cell: u32, header: bool, selectable: bool) {
    if header {
        s(cell, "background", "var(--day-sidebar-bg)");
        s(cell, "border-bottom", "");
        attr(cell, "role", "heading");
        attr(cell, "aria-level", "2");
        attr(cell, "aria-selected", "");
    } else {
        s(cell, "background", "");
        attr(cell, "role", if selectable { "option" } else { "" });
        attr(cell, "aria-level", "");
    }
}

/// A supplementary row (docs/list.md "Header, footer and empty state"): no row role, no
/// separator, no hover or selection highlight. Runs after `list_style_header`, which gives an
/// item row its role back.
fn list_style_supplement(cell: u32, supplement: bool) {
    class(cell, "day-supplement", supplement);
    if supplement {
        attr(cell, "role", "");
        attr(cell, "aria-selected", "");
        s(cell, "border-bottom", "");
    }
}

/// Pin the header of the section under the list's top edge (docs/list.md "Sections"). The
/// rows are absolutely framed inside the content, so the real header row cannot stick; the
/// host keeps one extra cell in a strip that does: `position: sticky` at the top of the scroll
/// content, with a negative bottom margin so it takes no room from the rows. The browser
/// keeps the strip pinned while scrolling, so only a change of section, and the push-up as the
/// next header arrives, wait for this pass (a microtask after the scroll event, before paint).
fn list_pin_header(host: u32) {
    // Read the placement under the borrow; create, bind and style after it, since a bind
    // re-enters day-core and its patches can come back here.
    let Some(plan) = LISTS.with(|m| {
        let m = m.borrow();
        let st = m.get(&host)?;
        let pin = st.pin;
        let hidden = Some((pin, None));
        if !st.sectioned() {
            return if pin.0 != 0 { hidden } else { None };
        }
        let n = st.source.as_ref().map_or(0, |src| (src.len)());
        if n == 0 {
            return hidden;
        }
        let mut view = [0.0_f64; 2];
        unsafe { day_dom_list_viewport(host, view.as_mut_ptr()) };
        let offset = view[0];
        let tops = st.tops(n);
        let at = tops[..n]
            .partition_point(|t| *t <= offset)
            .saturating_sub(1)
            .min(n - 1);
        let Some(h) = st.header_of(at) else {
            return hidden;
        };
        // The real header is still at (or below) the top edge: it shows itself.
        if tops[h] >= offset - 0.5 {
            return hidden;
        }
        let height = tops[h + 1] - tops[h];
        // The next section's header pushes the pin up rather than sliding under it.
        let next = (h + 1..n).find(|i| st.is_header(*i));
        let push = next.map_or(0.0, |k| (tops[k] - offset - height).min(0.0));
        let source = st.source.clone()?;
        let rebind = st.pin_row != Some(h);
        Some((pin, Some((source, h, height, push, rebind))))
    }) else {
        return;
    };
    let ((mut wrapper, mut cell), show) = plan;
    let Some((source, h, height, push, rebind)) = show else {
        if wrapper != 0 {
            s(wrapper, "display", "none");
        }
        return;
    };
    if wrapper == 0 {
        wrapper = unsafe { day_dom_create(EL_DIV) };
        cell = unsafe { day_dom_create(EL_CELL) };
        unsafe { day_dom_insert(wrapper, cell, 0) };
        // Before the rows' content, so the sticky strip's natural place is the list's top.
        unsafe { day_dom_insert(host, wrapper, 0) };
        s(wrapper, "position", "sticky");
        s(wrapper, "top", "0");
        s(wrapper, "z-index", "2");
        // A copy of a header the list already has: assistive tech reads the real one.
        attr(wrapper, "aria-hidden", "true");
        list_style_header(cell, true, false);
        LISTS.with(|m| {
            if let Some(st) = m.borrow_mut().get_mut(&host) {
                st.pin = (wrapper, cell);
            }
        });
    }
    let width = unsafe { day_dom_width(host) }.max(1.0);
    s(wrapper, "display", "block");
    s(wrapper, "height", &format!("{height}px"));
    s(wrapper, "margin-bottom", &format!("{}px", -height));
    s(wrapper, "transform", &format!("translateY({push}px)"));
    unsafe { day_dom_set_frame(cell, 0.0, 0.0, width, height) };
    if rebind {
        LISTS.with(|m| {
            if let Some(st) = m.borrow_mut().get_mut(&host) {
                st.pin_row = Some(h);
            }
        });
        (source.bind_pinned)(h, cell as usize as day_spec::RawHandle);
    }
}

/// A self-sizing list's rows moved: frame every realized row at the top its predecessors' heights
/// add up to, and size the content to the new extent. A no-op on a uniform list, whose rows
/// never move.
fn list_restack(host: u32) {
    LISTS.with(|m| {
        let m = m.borrow();
        let Some(st) = m.get(&host) else {
            return;
        };
        let Some(source) = st.source.as_ref().filter(|_| st.stacked()) else {
            return;
        };
        let n = (source.len)();
        let tops = st.tops(n);
        let width = unsafe { day_dom_width(host) }.max(1.0);
        for (i, &cell) in st.cells.iter().enumerate().take(n) {
            if cell != 0 {
                unsafe { day_dom_set_frame(cell, 0.0, tops[i], width, tops[i + 1] - tops[i]) };
            }
        }
        s(st.content, "height", &format!("{}px", tops[n]));
    });
}

/// How many `ui_idle` polls in a row a list may find its viewport report owed before it stops
/// holding scripts up: a fill that cannot catch up must never wedge one.
const LIST_OWED_POLLS: u32 = 30;

/// Whether every shown list has settled (`Toolkit::ui_idle`, docs/list.md "Visible rows"): its
/// own smooth scroll has arrived, and its last viewport report describes the offset, height and
/// rows it has now. A browser delivers `scroll` events with the next frame, so an offset that
/// moved this frame (a row jump, a reload that clamped it) is reported only then; until it is,
/// the app reads the rows the list showed before. A fill is posted for a list found behind, so
/// the next poll finds it reported.
fn list_settled() -> bool {
    let (settled, behind) = LISTS.with(|m| {
        let mut m = m.borrow_mut();
        let (mut settled, mut behind) = (true, Vec::new());
        for (&host, st) in m.iter_mut() {
            let Some(source) = &st.source else { continue };
            if source.viewport.is_none() && source.first_visible.is_none() {
                continue;
            }
            let mut view = [0.0_f64; 2];
            unsafe { day_dom_list_viewport(host, view.as_mut_ptr()) };
            let n = (source.len)();
            // A list that is not shown reports nothing, so it owes nothing.
            let late = view[1] > 0.0
                && n > 0
                && (st.fill_pending
                    || st.seen.is_none_or(|(offset, height, rows)| {
                        (offset - view[0]).abs() > 0.5
                            || (height - view[1]).abs() > 0.5
                            || rows != n
                    }));
            let gliding = view[1] > 0.0 && unsafe { day_dom_list_gliding(host) } != 0;
            if !(late || gliding) {
                st.owed_polls = 0;
            } else if st.owed_polls < LIST_OWED_POLLS {
                st.owed_polls += 1;
                settled = false;
                if late {
                    behind.push(host);
                }
            }
        }
        (settled, behind)
    });
    for host in behind {
        day_dom_list_scrolled(host);
    }
    settled
}

/// A list scrolled (the shim's `scroll` listener): build whatever rows just came into view, on
/// the next turn and at most once per turn however many events arrive.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_list_scrolled(host: u32) {
    day_spec::ffi_guard::contain((), || {
        let post = LISTS.with(|m| {
            let mut m = m.borrow_mut();
            let Some(st) = m.get_mut(&host) else {
                return false;
            };
            let first = !st.fill_pending;
            st.fill_pending = true;
            first
        });
        if post {
            post_local(move || {
                LISTS.with(|m| {
                    if let Some(st) = m.borrow_mut().get_mut(&host) {
                        st.fill_pending = false;
                    }
                });
                list_fill_window(host);
            });
        }
    });
}

/// The reader scrolled a list themselves (the shim's input watch, docs/list.md "Following the
/// end"): a finger, the scroll bar, the wheel or a key moved it, not the app.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_list_reader(host: u32) {
    day_spec::ffi_guard::contain((), || {
        let report = LISTS.with(|m| {
            m.borrow()
                .get(&host)
                .and_then(|st| st.source.as_ref()?.reader_scroll.clone())
        });
        if let Some(report) = report {
            report();
        }
    });
}

fn list_paint_selection(entry: &ListEntry) {
    for (i, &cell) in entry.cells.iter().enumerate() {
        // Unrealized rows have no cell to paint; they pick the treatment up when they are built
        // (this runs at the end of every fill, so a row scrolled into a selection lands painted).
        if cell == 0 {
            continue;
        }
        let on = entry.selected.contains(&i);
        class(cell, "selected", on);
        if entry.selectable {
            attr(cell, "aria-selected", if on { "true" } else { "false" });
        }
    }
}

/// Where the keyboard is in the list — the row an arrow moves from. That is the lead, the end a
/// shifted range last moved; failing that the anchor, and failing that the selection's last row
/// (a list whose selection the app set without either). With nothing selected there is no
/// cursor, and the caller decides which end to enter the list from.
fn list_cursor(entry: &ListEntry) -> Option<usize> {
    entry
        .lead
        .or(entry.anchor)
        .or_else(|| entry.selected.iter().next_back().copied())
}

/// Scroll `row` into view if it is not fully there, the way a native list does when the
/// keyboard walks off the visible edge. Whichever edge it went past is the one it comes back
/// to, so a held arrow key scrolls a line at a time instead of recentering on every step.
fn list_reveal_row(host: u32, (top, bottom): (f64, f64)) {
    let mut view = [0.0_f64; 2];
    unsafe { day_dom_list_viewport(host, view.as_mut_ptr()) };
    let (offset, vh) = (view[0], view[1]);
    if vh <= 0.0 {
        return; // not laid out yet; the fill that follows builds from the top anyway
    }
    let y = if top < offset {
        top
    } else if bottom > offset + vh {
        bottom - vh
    } else {
        return;
    };
    unsafe { day_dom_scroll_to(host, 0.0, y, 0) };
}

/// An arrow, Home or End the shim's list keyboard route claims for a focused list
/// (docs/list.md): `dir` is 0 up, 1 down, 2 home, 3 end, and `mods` is a `KeyEvent` mask. Moves
/// the selection one row (or to an end), extends the range instead when a multi-select list is
/// shifted, reveals the row and reports the same event a click on it would. `dir == 4`
/// activates the selected row. Returns 1 when handled; 0 lets Enter reach the default action.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_list_key(host: u32, dir: u32, mods: u32) -> u32 {
    day_spec::ffi_guard::contain(0, || {
        if dir == 4 {
            let active = LISTS.with(|m| {
                m.borrow()
                    .get(&host)
                    .filter(|s| s.activatable && !s.selected.is_empty())
                    .and_then(|s| list_cursor(s).map(|row| (s.node, row)))
            });
            if let Some((node, row)) = active {
                emit(node, Event::ListActivated(row));
                return 1;
            }
            return 0;
        }
        let moved = LISTS.with(|m| {
            let mut m = m.borrow_mut();
            let st = m.get_mut(&host)?;
            let n = st.source.as_ref().map_or(0, |src| (src.len)());
            if !st.selectable || n == 0 {
                return None;
            }
            // Entering an unselected list picks the row the arrow points AT: down lands on the
            // first row, up on the last, which is what every desktop list does. Section headers
            // are not rows the keyboard can stand on, so every move steps over them
            // (docs/list.md "Sections").
            // Supplementary rows (header, footer, empty state, padding) are skipped the same way.
            let is_header = st.source.as_ref().and_then(|src| src.is_header.clone());
            let is_supplement = st.source.as_ref().and_then(|src| src.is_supplement.clone());
            let item = |i: &usize| {
                !is_header.as_ref().is_some_and(|f| f(*i))
                    && !is_supplement.as_ref().is_some_and(|f| f(*i))
            };
            let cursor = list_cursor(st).map(|c| c.min(n - 1));
            let row = match (dir, cursor) {
                (0, Some(c)) => (0..c).rev().find(item).unwrap_or(c),
                (1, Some(c)) => (c + 1..n).find(item).unwrap_or(c),
                (0 | 3, None) | (3, _) => (0..n).rev().find(item)?,
                (1 | 2, None) | (2, _) => (0..n).find(item)?,
                _ => return None,
            };
            let shift = mods as u8 & day_spec::Modifiers::SHIFT.bits() != 0;
            if st.multi && shift {
                // Shift extends from the anchor and leaves it where it was, moving only the
                // lead — so a run of shifted arrows grows and shrinks one range instead of
                // starting a new one from wherever the last one ended.
                let a = st.anchor.unwrap_or(row);
                st.selected = (a.min(row)..=a.max(row)).filter(item).collect();
                st.anchor = Some(a);
                st.lead = Some(row);
            } else {
                st.selected = std::iter::once(row).collect();
                st.anchor = Some(row);
                st.lead = Some(row);
            }
            list_paint_selection(st);
            let ev = if st.multi {
                Event::SelectionSet(st.selected.iter().map(|r| *r as i64).collect())
            } else {
                Event::SelectionChanged(row as i64)
            };
            // Revealed below the pinned header, which would otherwise cover a row the keyboard
            // walked up onto.
            let (top, bottom) = st.row_span(row);
            Some((st.node, ev, (top - st.pinned_above(row), bottom)))
        });
        let Some((node, ev, span)) = moved else {
            return 0;
        };
        list_reveal_row(host, span);
        emit(node, ev);
        1
    })
}

fn list_patch(el: u32, p: &ListPatch) {
    match p {
        ListPatch::Reload => post_local(move || list_populate(el)),
        // Cell index is row index, so every cell rebinds as on a reload; what must follow the
        // deltas is the measured heights of a stacked list, which belong to rows, not slots.
        ListPatch::Splice(deltas) => {
            LISTS.with(|m| {
                if let Some(st) = m.borrow_mut().get_mut(&el)
                    && st.stacked()
                {
                    splice_heights(&mut st.heights, deltas);
                }
            });
            post_local(move || list_populate(el));
        }
        // No editing mode (Cap::ListEditing is Unsupported).
        ListPatch::Editing(_) => {}
        // A self-sizing row re-laid at a new height. This arrives inside day-core's layout
        // pass, so the row moves on the next turn: record the height Day gave it (cell index
        // is row index here), restack the rows after it, and build any the move revealed.
        &ListPatch::RowSizeInvalidated { cell, height } => post_local(move || {
            let moved = LISTS.with(|m| {
                let mut m = m.borrow_mut();
                let Some(st) = m.get_mut(&el).filter(|st| st.stacked()) else {
                    return false;
                };
                let Some(row) = st.cells.iter().position(|c| *c as usize == cell) else {
                    return false;
                };
                // A uniform sectioned list measures only its headers; its items keep the pitch.
                if !st.automatic && !st.is_header(row) {
                    return false;
                }
                if st.heights.len() <= row {
                    st.heights.resize(row + 1, 0.0);
                }
                st.heights[row] = height;
                true
            });
            if moved {
                list_restack(el);
                list_fill_window(el);
            }
        }),
        ListPatch::ScrollToEnd => unsafe { day_dom_scroll_edge(el, 1, 1) },
        ListPatch::ScrollToRow(row) => {
            let y = LISTS.with(|m| {
                m.borrow().get(&el).map_or(0.0, |st| {
                    (st.row_span(*row).0 - st.pinned_above(*row)).max(0.0)
                })
            });
            // Also held for the next fill: a jump asked for before the first one (a list the
            // page scrolls as it builds) meets content with no height yet, so the browser has
            // nothing to scroll and no scroll event comes to build the rows.
            LISTS.with(|m| {
                if let Some(st) = m.borrow_mut().get_mut(&el) {
                    st.pending_scroll = Some(y);
                }
            });
            unsafe { day_dom_scroll_to(el, 0.0, y, 1) };
        }
        ListPatch::Selected(rows) => {
            LISTS.with(|m| {
                if let Some(st) = m.borrow_mut().get_mut(&el) {
                    let incoming: BTreeSet<usize> = rows.iter().copied().collect();
                    // An app-driven selection lands the cursor on its last row. The ECHO of a
                    // selection this list just made is NOT app-driven — a `selected_rows`
                    // binding sends the same rows straight back — and taking the cursor from it
                    // would drag the anchor onto the end of the range the user is extending, so
                    // the next shifted arrow would restart the range instead of growing it.
                    if incoming != st.selected {
                        st.selected = incoming;
                        st.anchor = rows.last().copied();
                        st.lead = st.anchor;
                        list_paint_selection(st);
                    }
                }
            });
        }
    }
}

/// Apply sequential row deltas to a per-row table (docs/list.md "Splice"): each index is valid
/// against the table the previous deltas left; an inserted row is unmeasured (0).
fn splice_heights(heights: &mut Vec<f64>, deltas: &[day_spec::props::RowDelta]) {
    use day_spec::props::RowDelta;
    for d in deltas {
        match *d {
            RowDelta::Insert(i) => heights.insert(i.min(heights.len()), 0.0),
            RowDelta::Remove(i) => {
                if i < heights.len() {
                    heights.remove(i);
                }
            }
            RowDelta::Move(from, to) => {
                if from < heights.len() {
                    let h = heights.remove(from);
                    heights.insert(to.min(heights.len()), h);
                }
            }
        }
    }
}

/// Where a row scroll leaves the viewport's top (docs/list.md "Scrolling to a row"), clamped to
/// the scroll range; `None` for `Nearest` on a row already fully in view. `span` is the row's
/// top and bottom in the content, `pin` what a pinned section header covers above it.
fn row_scroll_target(
    span: (f64, f64),
    pin: f64,
    align: day_spec::RowAlign,
    offset: f64,
    view: f64,
    extent: f64,
) -> Option<f64> {
    use day_spec::RowAlign;
    let (top, bottom) = span;
    let y = match align {
        RowAlign::Start => top - pin,
        RowAlign::Center => f64::midpoint(top, bottom) - view / 2.0,
        RowAlign::End => bottom - view,
        RowAlign::Nearest => {
            if top - pin < offset {
                top - pin
            } else if bottom > offset + view {
                bottom - view
            } else {
                return None;
            }
        }
    };
    Some(y.clamp(0.0, (extent - view).max(0.0)))
}

/// A press on a list cell (docs/list.md): same semantics as the Qt emulated list.
fn list_cell_click(cell: u32, mods: u32) {
    let Some((host, row)) = CELL_ROWS.with(|m| m.borrow().get(&cell).copied()) else {
        return;
    };
    let emit_ev = LISTS.with(|m| {
        let mut m = m.borrow_mut();
        // A section header or a supplementary row is not selectable (docs/list.md).
        let st = m.get_mut(&host).filter(|st| st.is_item(row))?;
        let (ctrl, shift) = (mods & 1 != 0, mods & 2 != 0);
        if st.multi && ctrl {
            if !st.selected.remove(&row) {
                st.selected.insert(row);
            }
            st.anchor = Some(row);
            st.lead = Some(row);
        } else if st.multi && shift {
            // Shift-click extends from the anchor the same way a shifted arrow does: the pivot
            // stays, the lead comes to the clicked row.
            let a = st.anchor.unwrap_or(row);
            st.selected = (a.min(row)..=a.max(row))
                .filter(|i| st.is_item(*i))
                .collect();
            st.anchor = Some(a);
            st.lead = Some(row);
        } else {
            st.selected = std::iter::once(row).collect();
            st.anchor = Some(row);
            st.lead = Some(row);
        }
        list_paint_selection(st);
        Some((
            st.node,
            if st.multi {
                Event::SelectionSet(st.selected.iter().map(|r| *r as i64).collect())
            } else {
                Event::SelectionChanged(row as i64)
            },
        ))
    });
    if let Some((node, ev)) = emit_ev {
        emit(node, ev);
    }
}

// ---------------------------------------------------------------------------
// Canvas display-list encoding (§11): a flat f64 stream + a UTF-8 string blob; shim.js
// interprets it onto a 2-D context. Colors pack as u32 rgba; gradients resolve to absolute
// geometry Rust-side so the interpreter stays dumb.
// ---------------------------------------------------------------------------

fn pack_color(c: day_spec::Color) -> f64 {
    let r = (c.r.clamp(0.0, 1.0) * 255.0).round() as u32;
    let g = (c.g.clamp(0.0, 1.0) * 255.0).round() as u32;
    let b = (c.b.clamp(0.0, 1.0) * 255.0).round() as u32;
    let a = (c.a.clamp(0.0, 1.0) * 255.0).round() as u32;
    f64::from((r << 24) | (g << 16) | (b << 8) | a)
}

fn push_shape(buf: &mut Vec<f64>, shape: &Shape) {
    match shape {
        Shape::Rect(r) => buf.extend([0.0, r.origin.x, r.origin.y, r.size.width, r.size.height]),
        Shape::RoundedRect(r, rad) => buf.extend([
            1.0,
            r.origin.x,
            r.origin.y,
            r.size.width,
            r.size.height,
            *rad,
        ]),
        Shape::Ellipse(r) => buf.extend([2.0, r.origin.x, r.origin.y, r.size.width, r.size.height]),
        Shape::Arc { rect, start, sweep } => buf.extend([
            3.0,
            rect.origin.x,
            rect.origin.y,
            rect.size.width,
            rect.size.height,
            start.degrees(),
            sweep.degrees(),
        ]),
        Shape::Line(a, b) => buf.extend([4.0, a.x, a.y, b.x, b.y]),
        // The two variable-length kinds lead with their `geometry_key` and the length of what
        // follows: a shim that has decoded this key before keeps the `Path2D` it built and JUMPS
        // the payload. The length is what makes that possible — the geometry rides this buffer
        // rather than a side channel, so a hit has to know how far to skip.
        Shape::Polygon(pts) => {
            buf.extend([5.0, day_spec::geometry_key(shape), 0.0]);
            let len_at = buf.len() - 1;
            buf.push(pts.len() as f64);
            for p in pts {
                buf.extend([p.x, p.y]);
            }
            buf[len_at] = (buf.len() - len_at - 1) as f64;
        }
        // Path: [6, key, len, rule, segCount, then per segment: kind + its points].
        Shape::Path(path) => {
            buf.extend([6.0, day_spec::geometry_key(shape), 0.0]);
            let len_at = buf.len() - 1;
            buf.extend([
                match path.rule {
                    day_spec::FillRule::EvenOdd => 1.0,
                    day_spec::FillRule::NonZero => 0.0,
                },
                path.segs.len() as f64,
            ]);
            for seg in &path.segs {
                match seg {
                    day_spec::PathSeg::Move(a) => buf.extend([0.0, a.x, a.y]),
                    day_spec::PathSeg::Line(a) => buf.extend([1.0, a.x, a.y]),
                    day_spec::PathSeg::Quad(c, a) => buf.extend([2.0, c.x, c.y, a.x, a.y]),
                    day_spec::PathSeg::Cubic(c1, c2, a) => {
                        buf.extend([3.0, c1.x, c1.y, c2.x, c2.y, a.x, a.y]);
                    }
                    day_spec::PathSeg::Close => buf.push(4.0),
                }
            }
            buf[len_at] = (buf.len() - len_at - 1) as f64;
        }
    }
}

fn push_paint(buf: &mut Vec<f64>, paint: &Paint, bounds: Rect) {
    match paint {
        Paint::Solid(c) => buf.extend([0.0, pack_color((*c).resolved())]),
        Paint::Linear(g) => {
            let (o, sz) = (bounds.origin, bounds.size);
            buf.extend([
                1.0,
                o.x + g.start.x * sz.width,
                o.y + g.start.y * sz.height,
                o.x + g.end.x * sz.width,
                o.y + g.end.y * sz.height,
                g.stops.len() as f64,
            ]);
            for (off, c) in &g.stops {
                buf.extend([*off, pack_color((*c).resolved())]);
            }
        }
        Paint::Radial(g) => {
            let (o, sz) = (bounds.origin, bounds.size);
            buf.extend([
                2.0,
                o.x + g.center.x * sz.width,
                o.y + g.center.y * sz.height,
                g.radius * sz.width,
                g.radius * sz.height,
                g.stops.len() as f64,
            ]);
            for (off, c) in &g.stops {
                buf.extend([*off, pack_color((*c).resolved())]);
            }
        }
        // [3, cx, cy, startDeg, stopCount, stops…] (docs/canvas.md "Conic gradients"): the
        // center in canvas points, the start angle as recorded. The shim hands it to
        // `createConicGradient`, whose angle runs clockwise from +x as the spec's does.
        Paint::Conic(g) => {
            let (o, sz) = (bounds.origin, bounds.size);
            buf.extend([
                3.0,
                o.x + g.center.x * sz.width,
                o.y + g.center.y * sz.height,
                g.start.degrees(),
                g.stops.len() as f64,
            ]);
            for (off, c) in &g.stops {
                buf.extend([*off, pack_color((*c).resolved())]);
            }
        }
    }
}

/// The stroke style bytes shared by `Stroke` and a stroked `Stamp`: width, cap, join, miter,
/// dash phase, dash count, then the dashes.
fn push_stroke_style(buf: &mut Vec<f64>, style: &day_spec::StrokeStyle) {
    buf.extend([
        style.width,
        match style.cap {
            day_spec::LineCap::Butt => 0.0,
            day_spec::LineCap::Round => 1.0,
            day_spec::LineCap::Square => 2.0,
        },
        match style.join {
            day_spec::LineJoin::Miter => 0.0,
            day_spec::LineJoin::Round => 1.0,
            day_spec::LineJoin::Bevel => 2.0,
        },
        style.miter_limit,
        style.dash_phase,
        style.dash.len() as f64,
    ]);
    buf.extend(style.dash.iter().copied());
}

fn encode_ops(ops: &[DrawOp]) -> (Vec<f64>, Vec<u8>) {
    let mut buf = Vec::with_capacity(ops.len() * 8);
    let mut strs: Vec<u8> = Vec::new();
    for op in ops {
        match op {
            DrawOp::Fill(shape, paint) => {
                buf.push(0.0);
                push_paint(&mut buf, paint, shape.bounds());
                push_shape(&mut buf, shape);
            }
            DrawOp::Stamp(st) => {
                // [7, strokeFlag, (stroke style…)?, count, x0,y0 …, paint, template shape].
                // This encoder is variable-length already, so the coordinates ride it directly —
                // no separate record kind and no padding (docs/canvas.md "Stamping").
                buf.push(7.0);
                match &st.stroke {
                    None => buf.push(0.0),
                    Some(style) => {
                        buf.push(1.0);
                        push_stroke_style(&mut buf, style);
                    }
                }
                buf.push(st.at.len() as f64);
                for p in &st.at {
                    buf.push(p.x);
                    buf.push(p.y);
                }
                // The paint resolves against the TEMPLATE's bounds, which is the batch's own
                // geometry repeated — a gradient therefore shades each copy identically rather
                // than across the group, matching what a per-mark fill would have done.
                push_paint(&mut buf, &st.paint, st.shape.bounds());
                push_shape(&mut buf, &st.shape);
            }
            DrawOp::Stroke(shape, paint, style) => {
                // [1, width, cap, join, miter, dashPhase, dashCount, dashes…] then paint, then
                // shape. Style is inline rather than a separate record: this encoder is already
                // variable-length, so there is nothing to gain from a modifier record here.
                buf.push(1.0);
                push_stroke_style(&mut buf, style);
                push_paint(&mut buf, paint, shape.bounds());
                push_shape(&mut buf, shape);
            }
            DrawOp::Clip(shape) => {
                buf.push(6.0);
                push_shape(&mut buf, shape);
            }
            DrawOp::Text {
                text,
                at,
                size,
                color,
                anchor,
                font,
            } => {
                let off = strs.len() as f64;
                strs.extend_from_slice(text.as_bytes());
                let fam_off = strs.len() as f64;
                strs.extend_from_slice(font.family_str().as_bytes());
                buf.extend([
                    2.0,
                    pack_color(*color),
                    *size,
                    anchor.pack(),
                    at.x,
                    at.y,
                    off,
                    text.len() as f64,
                    // The font (docs/fonts.md): CSS weight (0 = default), italic, family bytes.
                    f64::from(font.css_weight()),
                    f64::from(u8::from(font.italic)),
                    fam_off,
                    font.family_str().len() as f64,
                ]);
            }
            DrawOp::Save => buf.push(3.0),
            DrawOp::Restore => buf.push(4.0),
            // [8, id, x, y, w, h, opacity] (docs/images.md). The id is a number, so it rides
            // this buffer like any coordinate. Note this encoder's numbering is its OWN — it is
            // not day-spec's frozen wire table, and the two have never agreed.
            DrawOp::Image {
                image,
                rect,
                opacity,
                source: None,
            } => buf.extend([
                8.0,
                image.0 as f64,
                rect.origin.x,
                rect.origin.y,
                rect.size.width,
                rect.size.height,
                *opacity,
            ]),
            // [9, id, x, y, w, h, opacity, sx, sy, sw, sh]: the same with a source region in
            // bitmap pixels, the 9-argument `drawImage`. Its own code rather than four more
            // slots on 8, so a whole-image draw encodes exactly as it always did.
            DrawOp::Image {
                image,
                rect,
                opacity,
                source: Some(src),
            } => buf.extend([
                9.0,
                image.0 as f64,
                rect.origin.x,
                rect.origin.y,
                rect.size.width,
                rect.size.height,
                *opacity,
                src.origin.x,
                src.origin.y,
                src.size.width,
                src.size.height,
            ]),
            DrawOp::Concat(m) => buf.extend([5.0, m.a, m.b, m.c, m.d, m.tx, m.ty]),
            // [10, dx, dy, blur, color] (docs/canvas.md "Shadows"): points, which the shim
            // turns into backing-store pixels. A clear color is "no shadow", so `None` needs
            // no code of its own.
            DrawOp::Shadow(shadow) => {
                let sh = shadow.unwrap_or(day_spec::Shadow::new(day_spec::Color::CLEAR, 0.0));
                buf.extend([10.0, sh.dx, sh.dy, sh.blur, pack_color(sh.color.resolved())]);
            }
            // [11, mode] (docs/canvas.md "Blend modes"), numbered as `BlendMode::wire`.
            DrawOp::Blend(mode) => buf.extend([11.0, f64::from(mode.wire())]),
        }
    }
    (buf, strs)
}

// ---------------------------------------------------------------------------
// Presentation (docs/dialogs.md): <dialog>-backed alert/confirm/sheet/prompt.
// ---------------------------------------------------------------------------

fn present_json(spec: &PresentSpec) -> Option<String> {
    use std::fmt::Write as _;
    let mut j = String::from("{");
    match spec {
        PresentSpec::Dialog {
            title,
            message,
            buttons,
            sheet,
        } => {
            j.push_str("\"kind\":\"dialog\",\"title\":");
            json_str(&mut j, title);
            if let Some(m) = message {
                j.push_str(",\"message\":");
                json_str(&mut j, m);
            }
            let _ = write!(j, ",\"sheet\":{sheet},\"buttons\":[");
            for (i, b) in buttons.iter().enumerate() {
                if i > 0 {
                    j.push(',');
                }
                push_button(&mut j, b);
            }
            j.push(']');
        }
        PresentSpec::Prompt {
            title,
            message,
            placeholder,
            initial,
            ok,
            cancel,
        } => {
            j.push_str("\"kind\":\"prompt\",\"title\":");
            json_str(&mut j, title);
            if let Some(m) = message {
                j.push_str(",\"message\":");
                json_str(&mut j, m);
            }
            j.push_str(",\"placeholder\":");
            json_str(&mut j, placeholder);
            j.push_str(",\"initial\":");
            json_str(&mut j, initial);
            j.push_str(",\"ok\":");
            json_str(&mut j, ok);
            j.push_str(",\"cancel\":");
            json_str(&mut j, cancel);
        }
        // The open picker: the shim clicks a hidden `<input type=file>` (with `multiple` set
        // for a several-files pick) and answers with each chosen file's name and bytes through
        // `day_dom_present_file_stage` / `day_dom_present_files`.
        PresentSpec::OpenFile {
            title,
            filters,
            multiple,
        } => {
            j.push_str("\"kind\":\"open\",\"title\":");
            json_str(&mut j, title);
            let accept: Vec<String> = filters
                .iter()
                .flat_map(|f| f.extensions.iter())
                .map(|e| format!(".{e}"))
                .collect();
            j.push_str(",\"accept\":");
            json_str(&mut j, &accept.join(","));
            j.push_str(if *multiple {
                ",\"multiple\":true"
            } else {
                ",\"multiple\":false"
            });
        }
        // A browser has no folder chooser that hands back a path (`Cap::FolderDialog` is
        // Unsupported): the request resolves as Dismissed.
        #[expect(
            clippy::match_same_arms,
            reason = "the two flows resolve here for different, separately documented reasons"
        )]
        PresentSpec::OpenFolder { .. } => return None,
        // Save rides its own FFI arm (bytes attached); reaching here means a non-web build.
        PresentSpec::SaveFile { .. } => return None,
    }
    j.push('}');
    Some(j)
}

fn push_button(j: &mut String, b: &PresentButton) {
    use std::fmt::Write as _;
    j.push_str("{\"label\":");
    json_str(j, &b.label);
    let role = match b.role {
        day_spec::present::ButtonRole::Default => "default",
        day_spec::present::ButtonRole::Cancel => "cancel",
        day_spec::present::ButtonRole::Destructive => "destructive",
    };
    let _ = write!(j, ",\"role\":\"{role}\"}}");
}

// ---------------------------------------------------------------------------
// Exports the shim calls back through.
// ---------------------------------------------------------------------------

/// Allocate `len` bytes inside wasm memory for the shim to write UTF-8 into (freed by the
/// export that consumes the pointer).
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len);
    let ptr = v.as_mut_ptr();
    std::mem::forget(v);
    ptr
}

/// Consume the final module's declaration sections before the application starts.
#[allow(clippy::not_unsafe_ptr_arg_deref)] // allocated by day_dom_alloc in the trusted shim
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_install_ids(ptr: *mut u8, len: usize) {
    // SAFETY: the shim allocated and initialized exactly len bytes with day_dom_alloc.
    let bytes = unsafe { take_bytes(ptr, len) };
    if let Err(error) = day_core::element_id::install_web_registry(&bytes) {
        log::error!("element registry: {error}");
    }
}

/// Install expectations captured before linking and run registration constructors once.
#[allow(clippy::not_unsafe_ptr_arg_deref)] // allocated by the trusted shim
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_install_tests(ptr: *mut u8, len: usize) {
    // SAFETY: the shim initialized exactly len bytes from day_dom_alloc.
    let bytes = unsafe { take_bytes(ptr, len) };
    if let Err(error) = day_core::conformance::install_web_registry(&bytes) {
        // Keep the host alive to report a nonretryable dayscript infrastructure error.
        log::error!("{error}");
    }
}

/// Take ownership of a `day_dom_alloc(len)` allocation the shim filled.
///
/// # Safety
///
/// `ptr` must come from `day_dom_alloc(len)`, hold `len` initialized bytes, and not be used
/// again by the caller.
#[expect(
    clippy::same_length_and_capacity,
    reason = "`day_dom_alloc(len)` made the allocation with capacity `len`"
)]
pub(crate) unsafe fn take_bytes(ptr: *mut u8, len: usize) -> Vec<u8> {
    // SAFETY: the caller's contract, stated above.
    unsafe { Vec::from_raw_parts(ptr, len, len) }
}

fn take_string(ptr: *mut u8, len: usize) -> String {
    // SAFETY: the shim wrote exactly `len` bytes into a `day_dom_alloc(len)` allocation.
    let v = unsafe { take_bytes(ptr, len) };
    String::from_utf8_lossy(&v).into_owned()
}

/// The reorder guard's verdict for a hovered drop, called synchronously by the shim's drag
/// handler (docs/list.md): the ACCEPTED target index, or -1 to deny (also when out of bounds or
/// the list has no reorder seam). The Rc is cloned out first, so the app's guard runs with no
/// `LISTS` borrow held.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_list_can_move(host: u32, from: u32, to: u32) -> i32 {
    // Every export below that runs closures or dispatches events is contained
    // (day_spec::ffi_guard): a panic unwinding a wasm `extern "C"` frame traps the
    // instance, so a caught panic reports and returns the arm's safe default instead.
    day_spec::ffi_guard::contain(-1, || {
        let r = LISTS.with(|m| {
            m.borrow()
                .get(&host)
                .and_then(|st| st.source.as_ref().map(|s| ((s.len)(), s.reorder.clone())))
        });
        let Some((len, Some(r))) = r else { return -1 };
        let (from, to) = (from as usize, to as usize);
        if from >= len || to >= len {
            return -1;
        }
        ((r.can_move)(from, to) as i32).min(len.saturating_sub(1) as i32)
    })
}

/// The row whose span holds content offset `y`, clamped to the rows; -1 for an empty or
/// unknown list. The shim's pointer-drag reorder reads its rows through this, since rows of a
/// self-sizing or sectioned list stand at their own heights rather than a pitch.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_list_row_at(host: u32, y: f64) -> i32 {
    day_spec::ffi_guard::contain(-1, || {
        LISTS.with(|m| {
            let m = m.borrow();
            let Some(st) = m.get(&host) else { return -1 };
            let n = st.source.as_ref().map_or(0, |src| (src.len)());
            if n == 0 {
                return -1;
            }
            let row = if st.stacked() {
                st.tops(n)
                    .partition_point(|top| *top <= y)
                    .saturating_sub(1)
            } else {
                (y.max(0.0) / st.row_height.max(1.0)) as usize
            };
            row.min(n - 1) as i32
        })
    })
}

/// Commit a drop the guard accepts: rotate Day's snapshot (deferring the app callback), then
/// reposition + rebind the pooled cells to the new order. Returns 1 on commit, 0 on deny.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_list_move(host: u32, from: u32, to: u32) -> u32 {
    day_spec::ffi_guard::contain(0, || {
        let accepted = day_dom_list_can_move(host, from, to);
        if accepted < 0 {
            return 0;
        }
        let r = LISTS.with(|m| {
            m.borrow()
                .get(&host)
                .and_then(|st| st.source.as_ref().and_then(|s| s.reorder.clone()))
        });
        let Some(r) = r else { return 0 };
        if accepted as u32 != from {
            (r.move_row)(from as usize, accepted as usize);
        }
        // Re-bind every pooled cell in the rotated order (the emulated "move animation").
        post_local(move || list_populate(host));
        1
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_event(el: u32, kind: u32, a: f64, b: f64, c: f64, d: f64) {
    day_spec::ffi_guard::contain((), || day_dom_event_inner(el, kind, a, b, c, d));
}

/// A menu button's press (docs/menus.md "Menu buttons"): ask the provider now, so the entries
/// are current, and hand them to the toolbar pull-down's popover under the button. Reduced to
/// what the web can answer, as the app menu is. Called from the shim's click, outside any
/// day-core borrow.
fn open_button_menu(el: u32, f: &day_spec::ContextMenuFn) {
    // The provider may remove this button; then no menu opens (docs/menus.md).
    let Some(items) =
        day_spec::resolve_button_menu(f, || BUTTON_MENUS.with(|t| t.get(el as usize)).is_some())
    else {
        return;
    };
    let items = web_menu_items(&items);
    if items.is_empty() {
        return;
    }
    let mut json = String::new();
    menu_items_json(&mut json, &items);
    // SAFETY: `json` outlives the call; the shim copies the string out before returning.
    unsafe { day_dom_button_menu(el, json.as_ptr(), json.len()) };
}

fn day_dom_event_inner(el: u32, kind: u32, a: f64, b: f64, c: f64, d: f64) {
    if kind == ev::CLICK && CELL_ROWS.with(|m| m.borrow().contains_key(&el)) {
        #[expect(
            clippy::float_cmp,
            reason = "the shim sends these flags as exact small integers"
        )]
        let (double, activates) = (b == 2.0, c == 1.0);
        if !double {
            list_cell_click(el, a as u32);
        }
        if double || activates {
            // A section header activates nothing (docs/list.md "Sections").
            let row = CELL_ROWS
                .with(|m| m.borrow().get(&el).copied())
                .filter(|(host, row)| {
                    LISTS.with(|m| m.borrow().get(host).is_some_and(|st| st.is_item(*row)))
                });
            if let Some((host, row)) = row
                && let Some(node) = node_of(host)
            {
                emit(node, Event::ListActivated(row));
            }
        }
        return;
    }
    if kind == ev::CLICK
        && let Some(f) = BUTTON_MENUS.with(|t| t.get(el as usize))
    {
        open_button_menu(el, &f);
        return;
    }
    let Some(node) = node_of(el) else { return };
    let event = match kind {
        ev::CLICK => Event::Pressed,
        ev::SUBMIT => Event::Submitted,
        ev::TOGGLE => Event::ToggleChanged(a != 0.0),
        ev::VALUE => Event::ValueChanged(a),
        ev::VALUE_COMMITTED => Event::ValueCommitted(a),
        ev::SELECT => Event::SelectionChanged(a as i64),
        ev::FOCUS => Event::FocusChanged(a != 0.0),
        ev::TAP => Event::Tap(Point::new(a, b)),
        ev::DRAG_BEGAN | ev::DRAG_MOVED | ev::DRAG_ENDED => Event::Drag {
            phase: match kind {
                ev::DRAG_BEGAN => day_spec::DragPhase::Began,
                ev::DRAG_MOVED => day_spec::DragPhase::Changed,
                _ => day_spec::DragPhase::Ended,
            },
            location: Point::new(a, b),
            translation: Point::new(c, d),
        },
        ev::HOVER_ENTER | ev::HOVER_MOVED | ev::HOVER_LEFT => Event::Hover {
            phase: match kind {
                ev::HOVER_ENTER => day_spec::DragPhase::Began,
                ev::HOVER_LEFT => day_spec::DragPhase::Ended,
                _ => day_spec::DragPhase::Changed,
            },
            location: Point::new(a, b),
        },
        ev::PAN => Event::Pan {
            phase: day_spec::DragPhase::Changed,
            location: Point::new(a, b),
            delta: Point::new(c, d),
        },
        ev::PINCH_BEGAN | ev::PINCH_MOVED | ev::PINCH_ENDED => Event::Pinch {
            phase: match kind {
                ev::PINCH_BEGAN => day_spec::DragPhase::Began,
                ev::PINCH_ENDED => day_spec::DragPhase::Ended,
                _ => day_spec::DragPhase::Changed,
            },
            scale: c,
            location: Point::new(a, b),
        },
        ev::ROTATE_BEGAN | ev::ROTATE_MOVED | ev::ROTATE_ENDED => Event::Rotate {
            phase: match kind {
                ev::ROTATE_BEGAN => day_spec::DragPhase::Began,
                ev::ROTATE_ENDED => day_spec::DragPhase::Ended,
                _ => day_spec::DragPhase::Changed,
            },
            rotation: c,
            location: Point::new(a, b),
        },
        ev::LONG_PRESS => Event::LongPress(Point::new(a, b)),
        ev::DOUBLE_TAP => Event::DoubleTap(Point::new(a, b)),
        ev::PRESS => Event::Press {
            pressed: c != 0.0,
            location: Point::new(a, b),
        },
        ev::SCROLL => Event::ScrollChanged(Point::new(a, b)),
        ev::SCROLL_PHASE => Event::ScrollPhase(match a as u32 {
            1 => day_spec::ScrollPhase::Dragging,
            2 => day_spec::ScrollPhase::Coasting,
            _ => day_spec::ScrollPhase::Idle,
        }),
        ev::CONTEXT => Event::ContextMenu {
            local: Point::new(a, b),
            window: Point::new(c, d),
        },
        ev::RESIZED => Event::FrameChanged(Size::new(a, b)),
        ev::NAV_BACK => Event::NavBack {
            already_popped: false,
        },
        _ => return,
    };
    emit(node, event);
}

/// A piece-defined Custom event from the shim (docs/extending.md §8.2's open channel): `num` is
/// the piece's own discriminator (the inline web view's link reports use -1, per
/// day-piece-webview's docs/webview.md) and `text` the payload. The mirror of the Android
/// bridge's kind-12 and ArkUI's `pieceEvent`.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_piece_event(el: u32, num: f64, ptr: *mut u8, len: usize) {
    let t = take_string(ptr, len);
    day_spec::ffi_guard::contain((), move || {
        if let Some(node) = node_of(el) {
            emit(
                node,
                Event::Custom {
                    tag: "",
                    num,
                    text: t,
                },
            );
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_event_text(el: u32, kind: u32, ptr: *mut u8, len: usize) {
    let t = take_string(ptr, len);
    day_spec::ffi_guard::contain((), move || {
        if let Some(node) = node_of(el) {
            // The kinds that carry a string. `16` is a styled run's link (docs/text-runs.md):
            // the anchor's own navigation is cancelled in the shim, so the app decides.
            emit(
                node,
                match kind {
                    16 => Event::LinkActivated(t),
                    // The piece channel (§8.2), as the Android and ArkTS bridges spell it: the
                    // payload IS the event, since no tag survives the boundary.
                    17 => Event::Custom {
                        tag: "",
                        num: 0.0,
                        text: t,
                    },
                    // Raw contacts (docs/gestures.md "Touches"), the shim's touches wire.
                    18 => Event::Touches(parse_touches(&t)),
                    _ => Event::TextChanged(t),
                },
            );
        }
    });
}

/// The browser answering a decode (docs/images.md): `w`/`h` are the decoded PIXEL size, or 0 when
/// the bytes were not an image this engine reads.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_image_decoded(req: u32, id: f64, w: f64, h: f64) {
    let id = day_spec::BitmapId(id as u64);
    day_spec::ffi_guard::contain((), move || {
        let event = if w > 0.0 && h > 0.0 {
            // The format and alpha were recorded when the decode was asked for; only the size
            // was unknown until now.
            let info = BITMAP_INFO.with(|m| {
                let mut m = m.borrow_mut();
                let info = m.entry(id.0).or_insert(day_spec::BitmapInfo {
                    pixels: Size::new(0.0, 0.0),
                    scale: 1.0,
                    format: None,
                    has_alpha: true,
                });
                info.pixels = Size::new(w, h);
                *info
            });
            Event::ImageDecoded {
                req: u64::from(req),
                result: Ok(info),
            }
        } else {
            BITMAP_INFO.with(|m| {
                m.borrow_mut().remove(&id.0);
            });
            Event::ImageDecoded {
                req: u64::from(req),
                result: Err(day_spec::ImageError::Decode),
            }
        };
        emit(day_spec::WINDOW_NODE, event);
    });
}

/// The browser answering a canvas export (docs/canvas.md "Exporting"): `ok` is 1 when the bitmap
/// is stored under `id`; otherwise `ptr`/`len` are a `day_dom_alloc` buffer holding the engine's
/// error text (or empty), to take ownership of.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_bitmap_rendered(req: u32, id: f64, ok: u32, ptr: *mut u8, len: usize) {
    let id = day_spec::BitmapId(id as u64);
    let why = if ok == 0 && !ptr.is_null() && len > 0 {
        take_string(ptr, len)
    } else {
        String::new()
    };
    day_spec::ffi_guard::contain((), move || {
        let result = if ok != 0 {
            // `render_bitmap` recorded the size and scale when it asked. An id released while
            // the browser was still rendering has no record left; the shim's fresh entry goes too.
            BITMAP_INFO
                .with(|m| m.borrow().get(&id.0).copied())
                .ok_or_else(|| {
                    // SAFETY: an id the shim has just stored; releasing it is always valid.
                    unsafe { day_dom_image_release(id.0 as f64) };
                    day_spec::ExportError::Failed(
                        "the bitmap was released before it rendered".into(),
                    )
                })
        } else {
            BITMAP_INFO.with(|m| {
                m.borrow_mut().remove(&id.0);
            });
            Err(day_spec::ExportError::Failed(if why.is_empty() {
                "the canvas could not render".into()
            } else {
                why
            }))
        };
        emit(
            day_spec::WINDOW_NODE,
            Event::BitmapRendered {
                req: u64::from(req),
                result,
            },
        );
    });
}

/// The browser answering an encode: `ptr`/`len` are a `day_dom_alloc` buffer to take ownership
/// of, or 0 when the canvas could not produce that type.
#[allow(clippy::not_unsafe_ptr_arg_deref)] // a live `day_dom_alloc` allocation from the shim
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_image_encoded(req: u32, ptr: *mut u8, len: usize) {
    let result = if ptr.is_null() || len == 0 {
        Err(day_spec::ImageError::Encode)
    } else {
        // SAFETY: the shim wrote exactly `len` bytes into a `day_dom_alloc(len)` allocation, and
        // ownership passes here — the same contract `take_string` relies on.
        Ok(unsafe { take_bytes(ptr, len) })
    };
    day_spec::ffi_guard::contain((), move || {
        emit(
            day_spec::WINDOW_NODE,
            Event::ImageEncoded {
                req: u64::from(req),
                result,
            },
        );
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_present_result(req: u32, which: i32, ptr: *mut u8, len: usize) {
    let result = if which == -1 {
        PresentResult::Dismissed
    } else if len > 0 {
        PresentResult::Text(take_string(ptr, len))
    } else {
        PresentResult::Button(i64::from(which))
    };
    day_spec::ffi_guard::contain((), move || {
        emit(
            day_spec::WINDOW_NODE,
            Event::PresentResult {
                req: u64::from(req),
                result,
            },
        );
    });
}

thread_local! {
    /// The files of a several-files pick the shim has handed over so far, by request: each
    /// `day_dom_present_file_stage` adds one, and the closing `day_dom_present_files` drains
    /// them into the one `PresentResult::Files` answer (docs/files.md).
    static STAGED_FILES: RefCell<HashMap<u32, Vec<String>>> = RefCell::new(HashMap::new());
}

/// Park a picked file's bytes in the `web_files` store under a fresh `/day-web/<id>/<leaf>`
/// path and return that path. `bytes` is `None` for a save flow, whose bytes already left as a
/// download.
fn store_picked_file(name: &str, bytes: Option<Vec<u8>>) -> String {
    static NEXT_FILE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    // The last path component only — a hostile/odd name must not escape the store's prefix.
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or("file");
    let id = NEXT_FILE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = format!("/day-web/{id}/{leaf}");
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    if let Some(b) = bytes {
        day_spec::present::web_files::write(&path, b);
    }
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    drop(bytes);
    path
}

/// One file of a several-files pick, every one but the last: parked under its virtual path and
/// held for `req` until `day_dom_present_files` closes the pick with the final file.
#[allow(clippy::not_unsafe_ptr_arg_deref)] // both buffers are live `day_dom_alloc` allocations from the shim
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_present_file_stage(
    req: u32,
    name_ptr: *mut u8,
    name_len: usize,
    bytes_ptr: *mut u8,
    bytes_len: usize,
) {
    let name = take_string(name_ptr, name_len);
    // SAFETY: the shim wrote exactly `bytes_len` bytes into a `day_dom_alloc` allocation.
    let bytes = (bytes_len > 0).then(|| unsafe { take_bytes(bytes_ptr, bytes_len) });
    day_spec::ffi_guard::contain((), move || {
        let path = store_picked_file(&name, bytes);
        STAGED_FILES.with(|s| s.borrow_mut().entry(req).or_default().push(path));
    });
}

/// A file picker's answer: `name` is the chosen file's display name; `bytes` its content for
/// an open flow (len 0 for a save flow, whose bytes already left as a download). The bytes
/// land in the `web_files` store under `/day-web/<name>`, and that virtual path — after any
/// files staged for the same request — answers the awaiting flow as `PresentResult::Files`;
/// the pieces layer reads each back like a local path (docs/files.md).
#[allow(clippy::not_unsafe_ptr_arg_deref)] // both buffers are live `day_dom_alloc` allocations from the shim
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_present_files(
    req: u32,
    name_ptr: *mut u8,
    name_len: usize,
    bytes_ptr: *mut u8,
    bytes_len: usize,
) {
    let name = take_string(name_ptr, name_len);
    // SAFETY: the shim wrote exactly `bytes_len` bytes into a `day_dom_alloc` allocation.
    let bytes = (bytes_len > 0).then(|| unsafe { take_bytes(bytes_ptr, bytes_len) });
    day_spec::ffi_guard::contain((), move || {
        let path = store_picked_file(&name, bytes);
        let mut paths = STAGED_FILES
            .with(|s| s.borrow_mut().remove(&req))
            .unwrap_or_default();
        paths.push(path);
        if req == 0 {
            day_core::request_open_files(paths);
            return;
        }
        emit(
            day_spec::WINDOW_NODE,
            Event::PresentResult {
                req: u64::from(req),
                result: PresentResult::Files(paths),
            },
        );
    });
}

/// A document-level clipboard event (the browser's own ⌘X/⌘C/⌘V route, or the Edit menu of
/// the browser itself) that no editable element claimed — the shim's `copy`/`cut`/`paste`
/// listeners forward it here while the event is still live, so day-part-clipboard's
/// synchronous calls inside the app's handler read and write the event's `clipboardData`
/// (docs/menus.md).
/// The browser's text size moved (a resize or a return to the page re-read it): every tracked
/// reader and every capped label follow (docs/text.md "Text size").
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_text_scale_changed() {
    day_spec::ffi_guard::contain((), || {
        MEASURE_CACHE.with(|c| c.borrow_mut().clear());
        post_local(day_core::note_text_size_changed);
    });
}

/// The reader's selection in a reporting label changed (docs/text.md "Selection"): `start` and
/// `end` are UTF-16 offsets into the element's text, equal when the selection left it.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_label_selected(el: u32, start: u32, end: u32) {
    day_spec::ffi_guard::contain((), || {
        let Some((start, end)) = LABELS.with(|t| {
            let mut t = t.borrow_mut();
            let st = t.get_mut(&el)?;
            let a = utf16_to_byte(&st.text, start as usize);
            let b = utf16_to_byte(&st.text, end as usize).max(a);
            st.selected = b > a;
            Some((a, b))
        }) else {
            return;
        };
        if let Some(node) = node_of(el) {
            emit(node, Event::TextSelected { start, end });
        }
    });
}

/// The device behind the latest pointer down or hover changed (docs/gestures.md "Input kind"):
/// `kind` 0 touch, 1 mouse, 2 stylus.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_pointer_kind_changed(kind: u32) {
    day_spec::ffi_guard::contain((), || {
        emit(
            day_spec::WINDOW_NODE,
            Event::PointerKindChanged(pointer_kind(kind)),
        );
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_edit(op: u32) {
    day_spec::ffi_guard::contain((), || {
        let op = match op {
            0 => day_spec::EditOp::Cut,
            1 => day_spec::EditOp::Copy,
            2 => day_spec::EditOp::Paste,
            _ => day_spec::EditOp::SelectAll,
        };
        emit(day_spec::WINDOW_NODE, Event::Edit(op));
    });
}

/// A non-text key pressed while this canvas has focus (docs/menus.md): `code` 0–5 is an arrow
/// or a delete key, 10–19 a digit; `modifiers` the day mask, with bit 8 for a repeat and bit 9
/// for the release of a press this element claimed. Returns whether the app claimed it — a
/// canvas nobody hung a key handler on keeps none of them, so the browser's own scrolling still
/// works underneath it.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_canvas_key(el: u32, code: u32, modifiers: u32) -> u32 {
    day_spec::ffi_guard::contain(0, || {
        let key = match code {
            0 => "ArrowLeft",
            1 => "ArrowRight",
            2 => "ArrowUp",
            3 => "ArrowDown",
            4 => "Delete",
            5 => "Backspace",
            10..=19 => day_spec::KeyEvent::DIGITS[(code - 10) as usize],
            _ => return 0,
        };
        let Some(node) = node_of(el) else {
            return 0;
        };
        if !day_spec::keys::handled(node) {
            return 0;
        }
        emit(node, Event::Key(phased_key(key, None, modifiers)));
        1
    })
}

/// A text field's Up, Down or Escape (docs/textfield.md "Keys"): `code` 0 is `ArrowUp`, 1
/// `ArrowDown`, 2 Escape, `modifiers` the day mask with the phase in bits 8 and 9. Returns whether the field claimed it, having
/// reported it; the shim then keeps the browser from acting on the key.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_field_key(el: u32, code: u32, modifiers: u32) -> u32 {
    day_spec::ffi_guard::contain(0, || {
        let Some(&key) = day_spec::keys::FIELD_KEYS.get(code as usize) else {
            return 0;
        };
        let Some(node) = node_of(el) else {
            return 0;
        };
        // A release follows the press the field claimed (the shim sends only those); a press
        // or a repeat asks again.
        let phase = day_spec::KeyPhase::from_wire(f64::from(modifiers));
        if phase != day_spec::KeyPhase::Up
            && !day_spec::keys::field_claims(node, key, (modifiers & 0xff) as u8)
        {
            return 0;
        }
        emit(
            node,
            Event::Key(day_spec::KeyEvent::named(key, 0).with_phase(phase)),
        );
        1
    })
}

/// Whether this canvas took the whole keyboard (`Decorate::keyboard`, docs/menus.md): the
/// shim asks on every key, before it names one.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_canvas_text(el: u32) -> u32 {
    day_spec::ffi_guard::contain(0, || {
        u32::from(node_of(el).is_some_and(day_spec::keys::text))
    })
}

/// A key on the full route (docs/menus.md "The full route"): `key`/`key_len` the browser's
/// `KeyboardEvent.key`, which is day's own vocabulary (empty for text no single key is
/// behind), `text`/`text_len` what it typed (`text_len` 0 for a key that typed nothing),
/// `modifiers` the day mask with the phase in bits 8 and 9. Returns whether the app claimed it.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_canvas_text_key(
    el: u32,
    key: *mut u8,
    key_len: usize,
    text: *mut u8,
    text_len: usize,
    modifiers: u32,
) -> u32 {
    let key = take_string(key, key_len);
    let text = (text_len > 0).then(|| take_string(text, text_len));
    day_spec::ffi_guard::contain(0, move || {
        let Some(node) = node_of(el) else {
            return 0;
        };
        if !day_spec::keys::handled(node) {
            return 0;
        }
        emit(node, Event::Key(phased_key(&key, text, modifiers)));
        1
    })
}

/// The platform-standard undo shortcut (⌘Z / Ctrl+Z, shift or Ctrl+Y for redo) pressed with
/// no editable element focused — the shim's keydown route (the browser has no document-level
/// undo of its own to integrate with, so the standard keys are the platform affordance).
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_undo(redo: u32) {
    day_spec::ffi_guard::contain((), || {
        emit(day_spec::WINDOW_NODE, Event::Undo { redo: redo != 0 });
    });
}

/// A toolbar button or menu entry was chosen — the same `MenuAction` every other backend
/// emits. The action id crosses as an f64: it is a small counter, far inside the range an f64
/// represents exactly.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_toolbar_action(action: f64) {
    day_spec::ffi_guard::contain((), || {
        emit(day_spec::WINDOW_NODE, Event::MenuAction(action as u64));
    });
}

/// A toolbar toggle flipped.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_toolbar_on(action: f64, on: u32) {
    day_spec::ffi_guard::contain((), || {
        emit(
            day_spec::WINDOW_NODE,
            Event::ToolbarChanged {
                action: action as u64,
                value: day_spec::ToolbarValue::On(on != 0),
            },
        );
    });
}

/// A toolbar segmented control's choice changed.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_toolbar_value(action: f64, index: u32) {
    day_spec::ffi_guard::contain((), || {
        emit(
            day_spec::WINDOW_NODE,
            Event::ToolbarChanged {
                action: action as u64,
                value: day_spec::ToolbarValue::Selected(index as usize),
            },
        );
    });
}

/// A toolbar search field's text changed. `take_string` takes ownership of the shim's
/// allocation, the way the other text callbacks here do.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_toolbar_text(action: f64, ptr: *mut u8, len: usize) {
    let text = take_string(ptr, len);
    day_spec::ffi_guard::contain((), move || {
        emit(
            day_spec::WINDOW_NODE,
            Event::ToolbarChanged {
                action: action as u64,
                value: day_spec::ToolbarValue::Text(text),
            },
        );
    });
}

/// A toolbar search field was submitted with Return (docs/search.md). The shim has already sent
/// the field's latest text through [`day_dom_toolbar_text`] when it differed from the last one.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_toolbar_submit(action: f64, ptr: *mut u8, len: usize) {
    let text = take_string(ptr, len);
    day_spec::ffi_guard::contain((), move || {
        emit(
            day_spec::WINDOW_NODE,
            Event::ToolbarChanged {
                action: action as u64,
                value: day_spec::ToolbarValue::Submit(text),
            },
        );
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_posted() {
    // The closures queued when this turn began, taken one at a time and each contained, so one
    // that fails cannot drop the ones queued behind it. A panic is caught here. A JavaScript
    // exception from an import is not: it unwinds through every wasm frame up to the shim's
    // microtask without running a destructor. The closures behind it must then still be in
    // the queue, where the turns already scheduled for them (one per post) run them. Drained
    // up front, they would be lost, and with them day-reactive's coalesced drain, whose
    // "posted" flag then stays set: no later signal write would reach its effects. Closures
    // posted from here wait for their own turn.
    let queued = POSTED.with(|q| q.borrow().len());
    for _ in 0..queued {
        let Some(f) = POSTED.with(|q| q.borrow_mut().pop_front()) else {
            break;
        };
        day_spec::ffi_guard::contain((), f);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_delayed(token: u32) {
    if let Some(f) = DELAYED.with(|m| m.borrow_mut().remove(&token)) {
        day_spec::ffi_guard::contain((), f);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_frame(token: f64, ts: f64) {
    day_core::frame::native::deliver(token as u64, day_spec::FrameStamp::new(ts));
}

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_resized(w: f64, h: f64) {
    day_spec::ffi_guard::contain((), || {
        LAST_VIEWPORT.with(|v| v.set(Size::new(w, h)));
        // day-core re-buckets the window's size class from this (docs/size-classes.md) — a
        // backend reports geometry, not classes, so there is one breakpoint table rather than
        // nine.
        emit(day_spec::WINDOW_NODE, Event::WindowResized(Size::new(w, h)));
    });
}

/// The page's display state, as the browser last reported it.
///
/// Called by the shim on `fullscreenchange` (entering, leaving, the user's Esc) with
/// `declined` = 0, and when the browser refuses a request (no user gesture, a permissions
/// policy) with `declined` = 1 and the state the page kept.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_window_state(code: i32, declined: u32) {
    day_spec::ffi_guard::contain((), || {
        report_window_state(day_spec::WindowState::from_code(code), declined != 0);
    });
}

/// Report the page's display state on `WINDOW_NODE`, deduped against the last report. A
/// declined request reports even when nothing changed, so day-core's optimistic `state()`
/// signal settles back on the truth (docs/windows.md "Window properties").
fn report_window_state(state: day_spec::WindowState, declined: bool) {
    let was = WINDOW_STATE.with(|c| c.replace(state));
    if declined {
        log::debug!("web: window state request declined, still {state:?}");
    } else if was == state {
        return;
    } else {
        log::debug!("web: window state now {state:?}");
    }
    emit(day_spec::WINDOW_NODE, Event::WindowStateChanged(state));
}

/// How far an installed app's window-controls overlay reaches into the page, from the shim on
/// every `geometrychange` after boot (`run` seeds the boot value); 0 while the user shows the
/// title bar. Only an Overlay primary reports, as its safe-area top inset
/// (docs/window-chrome.md). A fresh browser task: the tree is free.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_titlebar_inset(top: f64) {
    day_spec::ffi_guard::contain((), || {
        log::debug!("web: title bar overlay inset {top}");
        day_core::set_safe_area(day_spec::Insets {
            top,
            ..Default::default()
        });
    });
}

/// Put the effective appearance on the page: the window's override, else the app-wide one,
/// else the browser's `prefers-color-scheme`.
fn apply_appearance() {
    let (app, window) = APPEARANCE.with(std::cell::Cell::get);
    let mode = match window.or(app) {
        Some(false) => 0,
        Some(true) => 1,
        None => 2,
    };
    let effective = unsafe { day_dom_set_dark(mode) };
    DARK.with(|d| d.set(effective == 1));
}

/// Whether the browser has the Window Controls Overlay API, read once.
fn window_controls_overlay() -> bool {
    thread_local! {
        static WCO: std::cell::OnceCell<bool> = const { std::cell::OnceCell::new() };
    }
    WCO.with(|c| *c.get_or_init(|| env("wco") == "1"))
}

/// `document.fullscreenEnabled`, read once: whether this page can go fullscreen at all.
fn fullscreen_enabled() -> bool {
    thread_local! {
        static ENABLED: std::cell::OnceCell<bool> = const { std::cell::OnceCell::new() };
    }
    ENABLED.with(|c| *c.get_or_init(|| env("fullscreen") == "1"))
}

/// The reduce-motion media query flipped under the app (docs/accessibility.md): day-core
/// re-reads the setting through `Toolkit::reduce_motion` and re-gates its transitions.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_motion_changed() {
    day_spec::ffi_guard::contain((), day_core::note_motion_changed);
}

/// The browser's `prefers-color-scheme` flipped while the page follows it (no override): the
/// shim has switched the page already, and every ink on it followed in CSS. Day re-reads its
/// palette and dark signal, which redraws canvases and re-runs appearance readers.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_appearance_changed(dark: u32) {
    DARK.with(|d| d.set(dark == 1));
    day_spec::ffi_guard::contain((), day_core::note_appearance_changed);
}

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_lifecycle(phase: u32) {
    let phase = match phase {
        0 => Lifecycle::DidBecomeActive,
        1 => Lifecycle::WillResignActive,
        // `pagehide`: the page is going away (a navigation, a reload, a closed tab), which is
        // the last moment the app runs (docs/lifecycle.md).
        2 => Lifecycle::DidExit,
        _ => return,
    };
    day_spec::ffi_guard::contain((), || {
        emit(day_spec::WINDOW_NODE, Event::Lifecycle(phase));
    });
}

/// The host page's launch locale (`?locale=` else `navigator.languages`), for the `day::web`
/// glue to hand to `set_launch_locale` before the app installs its catalogs — wasm has no
/// process environment for a `DAY_LOCALE` variable to live in.
pub fn launch_locale() -> Option<String> {
    env("locales")
        .split(',')
        .next()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
}

/// The page's launch route (the URL hash, else `?route=`), for the `day::web` glue to hand to
/// `set_launch_deeplink` — the web spelling of `DAY_DEEPLINK` (docs/navigation.md).
/// Original external protocol URL delivered by an installed PWA protocol handler.
#[must_use]
pub fn launch_open_url() -> Option<String> {
    let url = env("day_url");
    (!url.is_empty()).then_some(url)
}
#[must_use]
pub fn launch_route() -> Option<String> {
    let route = env("route");
    (!route.is_empty()).then_some(route)
}

/// The page's dayscript token (`?dayscript=`), the query-parameter spelling of
/// `DAYSCRIPT_TOKEN` — present only when a `day launch` scripted/drivable session serves the
/// page (docs/web.md). The `day::web` glue arms the engine's web transport with it.
#[must_use]
pub fn dayscript_token() -> Option<String> {
    let token = env("dayscript");
    (!token.is_empty()).then_some(token)
}

/// Send one dayscript reply line to the page (the engine's web sender — docs/web.md).
pub fn script_send(line: &str) {
    unsafe { day_dom_script_send(line.as_ptr(), line.len()) };
}

/// Reclaim a shim-written string (a `day_dom_alloc` buffer) — for exports outside this crate
/// (the `day` umbrella defines `day_dom_script_line`, which routes to day-script; a backend
/// cannot depend on the engine).
pub fn take_alloc_string(ptr: *mut u8, len: usize) -> String {
    take_string(ptr, len)
}

/// The URL hash changed under the app (browser back/forward, or a hand-edited hash). The shim
/// suppresses the echo of our own `day_dom_set_hash`, so every call here is a real request.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_hash_changed(ptr: *mut u8, len: usize) {
    let route = take_string(ptr, len);
    day_spec::ffi_guard::contain((), move || {
        emit(day_spec::WINDOW_NODE, Event::RouteRequested(route));
    });
}

/// Install a panic hook that reports through the shim's console before the trap.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        warn(&format!("day panic: {info}"));
    }));
}

/// The browser-console sink for Day's logger (docs/logging.md).
///
/// Hand-rolled rather than `console_log` or `wasm-logger`: both require `web-sys` (and
/// wasm-logger `wasm-bindgen`), which this backend deliberately does without — the whole shim is
/// numeric ids across `extern "C"`, with no bundler and no npm. Routing an already-formatted line
/// to `console.*` is one call, so the dependency would buy nothing and cost the toolchain.
///
/// Pass this to `day_core::set_log_sink`; the facade does it in `day::web::start`.
pub fn console_sink(level: log::Level, line: &str) {
    unsafe { day_dom_log(level as u32, line.as_ptr(), line.len()) };
}

/// The level named by `?DAY_LOG=` in the page URL, if any. The web has no process environment for
/// `DAY_LOG` to live in, so the launch server forwards it as a query parameter (docs/web.md) —
/// `day launch -t web-dom --env DAY_LOG=debug` reaches this.
#[must_use]
pub fn launch_log_level() -> Option<log::LevelFilter> {
    host_env("DAY_LOG").and_then(|v| v.parse().ok())
}
