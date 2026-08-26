// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-dom — the `web-dom` backend (DESIGN.md §9): the DOM is the toolkit. A `<button>` is
//! the web's native button, `<input type="range">` its slider, `<dialog>` its modal surface —
//! semantic HTML plus ARIA, never a canvas-painted imitation (§0.3).
//!
//! Architecture: the same trampoline shape as day-arkui, with JavaScript in place of C. A
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
//! report their size back through a ResizeObserver (`Event::FrameChanged`) — the DayNavPage
//! contract (docs/navigation.md).
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};

use day_spec::props::*;
use day_spec::{
    A11yProps, AnimSpec, Builtin, Cap, Curve, DrawOp, Event, EventSink, Font, FontSpec, FontWeight,
    GestureKind, Lifecycle, ListSource, MenuItem, NodeId, Paint, PieceKind, Platform, Point,
    Proposal, Rect, Registry, Renderer, Shape, Size, Support, TextAnchor, Toolkit, Transform,
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
    /// Create an element by TAG NAME — the escape hatch piece renderers build on, since day-dom's
    /// own `EL_*` kind codes only cover the built-in vocabulary.
    fn day_dom_create_tag(tag: *const u8, tag_len: usize) -> u32;
    /// Invoke a zero-argument method on an element (`play`, `pause`, `load`, …).
    fn day_dom_call(el: u32, method: *const u8, method_len: usize);
    /// Replace an element's markup, keeping the caret (docs/texteditor.md). The HTML comes from
    /// Day's own serializer, which escapes every character of app text.
    fn day_dom_set_html(el: u32, html: *const u8, html_len: usize);
    /// Put the selection at a BYTE range in a contenteditable element's flattened text.
    fn day_dom_editor_select(el: u32, start: u32, end: u32);
    fn day_dom_set_app_badge(count: i32);
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
    /// Attach shim listeners; `mask` bits: 1 click, 2 input, 4 change, 8 focus, 16 submit,
    /// 32 resize-observer, 64 scroll, 128 pointer-tap, 256 pointer-drag, 512 contenteditable.
    fn day_dom_listen(el: u32, mask: u32);
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
    /// Report this list's scrolling into `day_dom_list_scrolled`, so rows coming into view build.
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
    fn day_dom_present(req: u32, json: *const u8, len: usize);
    /// The modifier keys held right now, as the shim last observed them (bit0 shift,
    /// bit1 primary = meta|ctrl, bit2 alt).
    fn day_dom_modifiers() -> u32;
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
    // Window toolbar (docs/toolbars.md): the whole bar crosses as one JSON spec, the way the
    // nav menu does; targeted patches address an item by id. `day_dom_toolbar_sidebar` returns
    // 0 when the page has no split nav to toggle.
    fn day_dom_toolbar(json: *const u8, len: usize);
    fn day_dom_toolbar_patch(json: *const u8, len: usize);
    fn day_dom_toolbar_sidebar() -> u32;
    fn day_dom_navmenu_select(el: u32, idx: i32);
    fn day_dom_options(el: u32, json: *const u8, len: usize);
    fn day_dom_options_select(el: u32, idx: u32);
    fn day_dom_schedule_post();
    fn day_dom_schedule_delayed(token: u32, ms: u32);
    fn day_dom_request_frame();
    fn day_dom_set_title(ptr: *const u8, len: usize);
    fn day_dom_open_url(ptr: *const u8, len: usize);
    /// Mirror the app route into the URL hash. `replace` = rewrite the current history entry
    /// (the launch reflection) instead of pushing a new one (in-app navigation).
    fn day_dom_set_hash(ptr: *const u8, len: usize, replace: u32);
    /// One dayscript reply line out to the page's WebSocket (docs/web.md; the shim queues
    /// until the socket is open and drops the line when scripting is not armed).
    fn day_dom_script_send(ptr: *const u8, len: usize);
    fn day_dom_env(key: *const u8, kl: usize, out: *mut u8, cap: usize) -> usize;
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
/// Put a [`ButtonStyleSpec`] on a `<button>`, keeping it a `<button>`.
///
/// A tint sets two CSS custom properties and adds `.tinted`; day.css does the rest, including
/// the `:hover`/`:active`/`:disabled` rules every `.day-btn` already has. Painting the colors
/// through variables rather than inline `background` is what lets those state rules keep
/// working — an inline background would win over them.
fn apply_button_style(el: u32, style: ButtonStyleSpec) {
    // Clear the others first, so a patch between styles cannot leave two classes on.
    class(el, "prominent", false);
    class(el, "bordered", false);
    class(el, "tinted", false);
    match style {
        ButtonStyleSpec::Prominent => class(el, "prominent", true),
        ButtonStyleSpec::Bordered => class(el, "bordered", true),
        ButtonStyleSpec::Tinted(c) => {
            let css = |x: day_spec::Color| {
                format!(
                    "rgb({} {} {})",
                    (x.r.clamp(0.0, 1.0) * 255.0) as u8,
                    (x.g.clamp(0.0, 1.0) * 255.0) as u8,
                    (x.b.clamp(0.0, 1.0) * 255.0) as u8
                )
            };
            s(el, "--day-tint", &css(c));
            s(el, "--day-tint-fg", &css(ButtonStyleSpec::on_tint(c)));
            class(el, "tinted", true);
        }
        ButtonStyleSpec::Automatic => {}
    }
}

/// Set a label's text, as styled spans when it has runs (docs/text-runs.md).
///
/// Runs become `<span>` children (a link run an `<a>`), which is what lets the whole thing stay
/// ONE wrapping paragraph — the browser wraps across the spans as if they were plain text. The
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
            s(el, "color", &color_css(c));
        }
        if let Some(c) = r.background {
            s(el, "background-color", &color_css(c));
        }
        // ONE `text-decoration`, because the shorthand replaces itself: a run that is both
        // underlined and struck through needs both words in one declaration, and setting the
        // property twice would keep only the second.
        let deco = decoration_css(r);
        if !deco.is_empty() {
            s(el, "text-decoration", &deco);
        }
        if let Some(url) = r.link.as_deref() {
            unsafe { day_dom_link(el, parent, url.as_ptr(), url.len()) };
        }
    }
    unsafe { day_dom_insert(parent, el, u32::MAX) };
}

fn class(el: u32, c: &str, on: bool) {
    unsafe { day_dom_set_class(el, c.as_ptr(), c.len(), on as u32) };
}
fn warn(msg: &str) {
    unsafe { day_dom_warn(msg.as_ptr(), msg.len()) };
}
/// Read a host "environment" value (query params / navigator facts) as an app-facing
/// environment lookup: `day launch --env K=V` lands in the page URL's query string and is
/// read back here (docs/web.md). Empty/absent answers `None`. The page-fact keys (`vw`,
/// `vh`, `dpr`, `dark`, `locales`, `route`) are reserved by the shim.
pub fn host_env(key: &str) -> Option<String> {
    let v = env(key);
    if v.is_empty() { None } else { Some(v) }
}

/// Milliseconds since the Unix epoch, from the page's `Date.now()`. The wasm target has no
/// working `SystemTime::now()`, so time-of-day code asks the host page instead (day-part-timezone
/// routes through here on `web-dom`).
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
    let mut cap = 512usize;
    loop {
        let mut buf = vec![0u8; cap];
        let n = unsafe { day_dom_env(key.as_ptr(), key.len(), buf.as_mut_ptr(), buf.len()) };
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
    /// Each realized image's `src`, so a tint patch can rebuild the mask from the same URL. The
    /// DOM bridge writes attributes but does not read them back, and keeping the string here is
    /// cheaper than adding a getter to the shim for one caller.
    static IMAGE_SRC: RefCell<HashMap<u32, String>> = RefCell::new(HashMap::new());
}

/// Recolor a template glyph (docs/vectors.md "Tint").
///
/// The browser cannot recolor the pixels of an `<img>`, so a tinted glyph becomes a MASK painted
/// with the tint — the same technique the nav rows use for their icons. The element keeps its
/// `src` so the untinted path, the alt text and the layout are unchanged; a `None` tint puts the
/// image back exactly as it was.
fn apply_image_tint(el: u32, src: &str, fit: &str, tint: Option<day_spec::Color>) {
    match tint {
        Some(c) => {
            let mask = format!("url(\"{src}\")");
            s(el, "mask-image", &mask);
            s(el, "-webkit-mask-image", &mask);
            // The mask scales the way the untinted image would: `object-fit` and `mask-size`
            // share contain/cover, and Stretch's `fill` is `100% 100%`.
            let size = if fit == "fill" { "100% 100%" } else { fit };
            for prop in ["mask-size", "-webkit-mask-size"] {
                s(el, prop, size);
            }
            for prop in ["mask-repeat", "-webkit-mask-repeat"] {
                s(el, prop, "no-repeat");
            }
            for prop in ["mask-position", "-webkit-mask-position"] {
                s(el, prop, "center");
            }
            s(
                el,
                "background-color",
                &format!(
                    "#{:02x}{:02x}{:02x}",
                    (c.r * 255.0) as u8,
                    (c.g * 255.0) as u8,
                    (c.b * 255.0) as u8
                ),
            );
        }
        None => {
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
            ] {
                s(el, prop, "");
            }
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
    /// appears here FIRST; everywhere else they are the host's detail children in attach order
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
    node: NodeId,
    content: u32,
    row_height: f64,
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
    last_width: f64,
    selectable: bool,
    multi: bool,
    selected: BTreeSet<usize>,
    /// The FIXED end of a shifted range — the row the extension pivots on, set by a plain click
    /// or a plain arrow and left alone while shift moves the other end.
    anchor: Option<usize>,
    /// The MOVING end: where the keyboard is, and the row a shifted arrow walks. Separate from
    /// the anchor because a range has two ends and `shift+↓ ↓ ↑` has to grow twice and shrink
    /// once — one field cannot both stay put and move.
    lead: Option<usize>,
}

/// A pending `request_frame` callback (the timestamp is seconds, from rAF).
type FrameCb = Box<dyn FnOnce(f64) + 'static>;

thread_local! {
    static SINK: RefCell<Option<EventSink>> = const { RefCell::new(None) };
    /// Element id → day node id, for event routing (the shim only knows element ids).
    static NODE_OF: RefCell<HashMap<u32, NodeId>> = RefCell::new(HashMap::new());
    /// Elements whose frames are CSS-managed (nav/tab pages): `set_frame` skips them.
    static CSS_FRAMED: RefCell<std::collections::HashSet<u32>> = RefCell::new(Default::default());
    static NAV_STATE: RefCell<HashMap<u32, NavState>> = RefCell::new(HashMap::new());
    /// NAV_PAGE element → is-sidebar, recorded at realize for the nav `insert`.
    static PAGE_SIDEBAR: RefCell<HashMap<u32, bool>> = RefCell::new(HashMap::new());
    static LISTS: RefCell<HashMap<u32, ListEntry>> = RefCell::new(HashMap::new());
    /// List cell element → (list host element, row index) for selection clicks.
    static CELL_ROWS: RefCell<HashMap<u32, (u32, usize)>> = RefCell::new(HashMap::new());
    /// Spinner-vs-bar per PROGRESS element (progress with `None` renders as a spinner).
    static SPINNERS: RefCell<std::collections::HashSet<u32>> = RefCell::new(Default::default());
    static POSTED: RefCell<Vec<Box<dyn FnOnce() + Send>>> = RefCell::new(Vec::new());
    static DELAYED: RefCell<HashMap<u32, Box<dyn FnOnce() + Send>>> = RefCell::new(HashMap::new());
    static NEXT_DELAY: Cell<u32> = const { Cell::new(1) };
    static FRAME_CB: RefCell<Option<FrameCb>> = const { RefCell::new(None) };
    static SPLIT_MODE: Cell<bool> = const { Cell::new(true) };
    static DARK: Cell<bool> = const { Cell::new(false) };
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
        )
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
// Naming the variable rather than baking a number in is what keeps ONE definition of the size —
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
        Font::System(pt) => (pt / 16.0, 400),
        Font::Custom(_, pt) => (pt / 16.0, 400),
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
    let weight = f.weight.map(weight_css).unwrap_or(default_weight);
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

/// A run's `text-decoration`: the lines and, for the patterned underlines, the style word CSS
/// spells them with. Empty when the run has neither.
fn decoration_css(r: &day_spec::TextRun) -> String {
    use day_spec::Underline as U;
    let mut lines = String::new();
    if r.underline.is_on() {
        lines.push_str("underline");
    }
    if r.strikethrough {
        if !lines.is_empty() {
            lines.push(' ');
        }
        lines.push_str("line-through");
    }
    if lines.is_empty() {
        return lines;
    }
    match r.underline {
        U::Double => lines.push_str(" double"),
        U::Dotted => lines.push_str(" dotted"),
        U::Wavy => lines.push_str(" wavy"),
        U::Single | U::None => {}
    }
    lines
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

fn apply_font(el: u32, f: &FontSpec) {
    s(el, "font", &font_css(f));
    // `font-variant-numeric` is a separate property from the `font` shorthand — and the shorthand
    // RESETS it, so this has to be set after, not before.
    s(
        el,
        "font-variant-numeric",
        if f.tabular { "tabular-nums" } else { "normal" },
    );
}

// ---------------------------------------------------------------------------
// JSON writer (tiny, escapes only what the shim needs — no serde dependency).
// ---------------------------------------------------------------------------

/// Build the nav-menu JSON (`{items:[{title, icon?, tint?}], selected}`) the shim's
/// `day_dom_navmenu` consumes. Shared by NAV_MENU realize and the data-driven
/// `NavMenuPatch::Items` rebuild.
fn navmenu_json(
    items: &[String],
    icons: &[Option<String>],
    tints: &[Option<day_spec::Color>],
    badge_icons: &[Option<String>],
    badge_tints: &[Option<day_spec::Color>],
    selected: Option<usize>,
) -> String {
    let mut json = String::from("{\"items\":[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        json.push_str("{\"title\":");
        json_str(&mut json, item);
        if let Some(Some(icon)) = icons.get(i) {
            json.push_str(",\"icon\":");
            json_str(&mut json, &image_url(icon));
            // The row's own icon tint (docs/vectors.md): the shim paints the mask with this
            // instead of currentColor.
            if let Some(Some(t)) = tints.get(i) {
                json.push_str(",\"tint\":");
                json_str(
                    &mut json,
                    &format!(
                        "#{:02x}{:02x}{:02x}",
                        (t.r * 255.0) as u8,
                        (t.g * 255.0) as u8,
                        (t.b * 255.0) as u8
                    ),
                );
            }
        }
        // The trailing status glyph (docs/navigation.md), in the same shape as `icon`/`tint` so
        // the shim paints it with one code path at the other end of the row.
        if let Some(Some(badge)) = badge_icons.get(i) {
            json.push_str(",\"badgeIcon\":");
            json_str(&mut json, &image_url(badge));
            if let Some(Some(t)) = badge_tints.get(i) {
                json.push_str(",\"badgeTint\":");
                json_str(
                    &mut json,
                    &format!(
                        "#{:02x}{:02x}{:02x}",
                        (t.r * 255.0) as u8,
                        (t.g * 255.0) as u8,
                        (t.b * 255.0) as u8
                    ),
                );
            }
        }
        json.push('}');
    }
    json.push_str("],\"selected\":");
    json.push_str(&selected.map(|i| i.to_string()).unwrap_or("-1".into()));
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
fn toolbar_json(items: &[day_spec::ToolbarItem]) -> String {
    use day_spec::ToolbarItemKind as K;
    let mut json = String::from("{\"items\":[");
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        let kind = match &it.kind {
            K::Button => "B",
            K::Toggle { .. } => "T",
            K::SidebarToggle => "S",
            K::Menu { .. } => "M",
            K::Search { .. } => "F",
            K::Segmented { .. } => "G",
            K::Label => "L",
            K::Separator => "-",
            K::Space => "_",
            K::FlexibleSpace => ">",
        };
        json.push_str("{\"kind\":");
        json_str(&mut json, kind);
        json.push_str(",\"id\":");
        json_str(&mut json, &it.id);
        json.push_str(",\"label\":");
        json_str(&mut json, &it.label);
        json.push_str(",\"tip\":");
        json_str(&mut json, it.tooltip.as_deref().unwrap_or(&it.label));
        json.push_str(",\"action\":");
        json.push_str(&it.action.to_string());
        json.push_str(",\"enabled\":");
        json.push_str(if it.enabled { "true" } else { "false" });
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
                json_str(&mut json, &seg.title);
                let ic = match &seg.icon {
                    Some(day_spec::Icon::Image(name)) => Some(image_url(name)),
                    Some(day_spec::Icon::Symbol(sym)) => symbol_svg(*sym),
                    None => None,
                };
                if let Some(url) = ic {
                    json.push_str(",\"icon\":");
                    json_str(&mut json, &url);
                }
                json.push('}');
            }
            json.push(']');
        }
        if let K::Search {
            text,
            placeholder,
            suggestions,
        } = &it.kind
        {
            json.push_str(",\"text\":");
            json_str(&mut json, text.as_str());
            json.push_str(",\"placeholder\":");
            json_str(&mut json, placeholder.as_str());
            // A native <datalist>, so the browser draws the completion popup.
            json.push_str(",\"suggestions\":[");
            for (n, sug) in suggestions.iter().enumerate() {
                if n > 0 {
                    json.push(',');
                }
                json_str(&mut json, sug);
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
            None if matches!(it.kind, K::SidebarToggle) => symbol_svg(day_spec::Symbol::Sidebar),
            None => None,
        };
        if let Some(url) = icon {
            json.push_str(",\"icon\":");
            json_str(&mut json, &url);
        }
        json.push('}');
    }
    json.push_str("]}");
    json
}

fn json_str(out: &mut String, v: &str) {
    out.push('"');
    for ch in v.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
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
    /// them — day-dom cannot special-case a piece it does not (and must not) depend on.
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
    pub fn new() -> Self {
        Dom { root: 1 }
    }

    /// Create a DOM element of `tag` for a piece renderer, returning its handle.
    ///
    /// The public helper surface a `lib-dom.rs` builds on: piece crates cannot reach day-dom's
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
            AppBadge::None => -1,
            AppBadge::Count(0) => -1,
            AppBadge::Count(n) => (*n).min(i32::MAX as u32) as i32,
            AppBadge::Dot => 0,
            // No text badge on the web; clearing would be worse than leaving the last value, so
            // this is the one payload the arm ignores outright.
            AppBadge::Text(_) => return,
        };
        unsafe { day_dom_set_app_badge(encoded) };
    }

    fn capability(&self, cap: Cap) -> Support {
        match cap {
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
            // The browser's file input and download ARE its file dialogs; bytes ride the
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
            Cap::Toolbar => Support::Emulated,
            // The Badging API takes a number or, with no argument, a dot. Emulated rather than
            // Native because whether anything is DRAWN depends on the browser and on the page
            // being an installed app — the call itself always succeeds (docs/badge.md).
            Cap::AppBadgeCount | Cap::AppBadgeDot => Support::Emulated,
            Cap::TextEditable | Cap::TextSelectable | Cap::TextSpellCheck => Support::Native,
            Cap::ListRecycling => Support::Emulated,
            // Pointer-tracked drag with a CSS gap — the browser has no native list reorder.
            Cap::ListReorder => Support::Emulated,
            // A topmost fixed-position child — not a system modal (docs/cover.md).
            Cap::Cover => Support::Emulated,
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
                apply_surface(el, p.background, p.corner_radius, p.clips, p.role.is_some());
                el
            }
            Some(Builtin::Label) => {
                let Some(p) = day_spec::props_of::<LabelProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let el = unsafe { day_dom_create(EL_LABEL) };
                set_label_text(el, &p.text, &p.runs);
                apply_font(el, &p.font);
                if let Some(c) = p.color {
                    s(el, "color", &color_css(c));
                }
                if !p.wraps {
                    s(el, "white-space", "nowrap");
                }
                // `start`/`end` rather than `left`/`right`, so an RTL locale follows the writing
                // direction without the app asking (docs/localization.md).
                match p.align {
                    day_spec::props::TextAlign::Leading => {}
                    day_spec::props::TextAlign::Center => s(el, "text-align", "center"),
                    day_spec::props::TextAlign::Trailing => s(el, "text-align", "end"),
                }
                el
            }
            Some(Builtin::Button) => {
                let Some(p) = day_spec::props_of::<ButtonProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let el = unsafe { day_dom_create(EL_BUTTON) };
                text(el, &p.title);
                apply_button_style(el, p.style);
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
                unsafe { day_dom_set_checked(el, p.on as u32) };
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
                attr(
                    el,
                    "step",
                    &p.step.map(|s| s.to_string()).unwrap_or("any".into()),
                );
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
                match p.value {
                    Some(v) => {
                        attr(el, "max", "1");
                        unsafe { day_dom_set_value(el, v) };
                    }
                    // Which of the two this element is, is only knowable here — both render with
                    // no text, so `measure` cannot tell them apart from the DOM.
                    None => {
                        SPINNERS.with(|s| s.borrow_mut().insert(el));
                    }
                }
                el
            }
            Some(Builtin::Image) => {
                let Some(p) = day_spec::props_of::<ImageProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let src = if p.source.contains('/') {
                    p.source.clone()
                } else {
                    format!("assets/images/{}.{}", p.source, image_ext(&p.source))
                };
                let fit = match p.content_mode {
                    ContentMode::Fit => "contain",
                    ContentMode::Fill => "cover",
                    ContentMode::Stretch => "fill",
                };
                // A TINTED glyph is a masked div, not an `<img>`: the browser cannot recolor an
                // image's pixels, and an `<img>` paints its own art over whatever sits behind it,
                // so masking one only clips it — the tint shows through as a faint edge. Painting
                // the mask with the tint is what the nav rows do, and it is the only technique
                // here that actually recolors (docs/vectors.md "Tint").
                let el =
                    unsafe { day_dom_create(if p.tint.is_some() { EL_DIV } else { EL_IMAGE }) };
                if p.tint.is_some() {
                    if !p.decorative {
                        attr(el, "role", "img");
                    }
                } else {
                    attr(el, "src", &src);
                    s(el, "object-fit", fit);
                    if p.decorative {
                        attr(el, "alt", "");
                    }
                }
                if p.decorative {
                    attr(el, "aria-hidden", "true");
                }
                IMAGE_SRC.with(|m| m.borrow_mut().insert(el, src.clone()));
                apply_image_tint(el, &src, fit, p.tint);
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
                unsafe { day_dom_listen(el, 64) };
                el
            }
            Some(Builtin::Divider) => unsafe { day_dom_create(EL_DIVIDER) },
            Some(Builtin::Nav) => {
                let Some(p) = day_spec::props_of::<NavProps>(kind, "web-dom", props) else {
                    return realize_placeholder(kind, id);
                };
                let el = unsafe { day_dom_create(EL_NAV) };
                unsafe {
                    day_dom_nav_mode(
                        el,
                        nav_mode(p.presentation),
                        p.title.as_ptr(),
                        p.title.len(),
                    )
                };
                unsafe { day_dom_listen(el, 1) }; // the back bar's button reports via CLICK
                NAV_STATE.with(|m| {
                    m.borrow_mut().insert(
                        el,
                        NavState {
                            presentation: p.presentation,
                            pages: Vec::new(),
                            sidebar: None,
                            titles: vec![p.title.clone()],
                            selected: 0,
                        },
                    )
                });
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
                    &p.badge_icons,
                    &p.badge_tints,
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
                let row_height = match p.row_height {
                    RowHeight::Uniform(h) => h,
                    RowHeight::Automatic => 44.0,
                };
                LISTS.with(|m| {
                    m.borrow_mut().insert(
                        host,
                        ListEntry {
                            node: id,
                            content,
                            row_height,
                            source: None,
                            cells: Vec::new(),
                            bound: Vec::new(),
                            fill_pending: false,
                            last_width: -1.0,
                            selectable: p.selectable,
                            multi: p.multi_select,
                            selected: BTreeSet::new(),
                            anchor: None,
                            lead: None,
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
            Some(Builtin::ListCell)
            | Some(Builtin::Tree)
            | Some(Builtin::Inspector)
            | Some(Builtin::InspectorPane)
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
                if let Some(day_spec::props::ImagePatch::Tint(c)) =
                    patch.downcast_ref::<day_spec::props::ImagePatch>()
                {
                    // The mask needs the same URL the element already loads.
                    let src = IMAGE_SRC.with(|m| m.borrow().get(&el).cloned());
                    if let Some(src) = src {
                        // Only the color changes here: an element realized with a tint is
                        // already the masked div, so this repaints the mask's fill.
                        apply_image_tint(el, &src, "contain", *c);
                    }
                }
            }
            kinds::CONTAINER => {
                if let Some(ContainerPatch::Background(c)) = patch.downcast_ref::<ContainerPatch>()
                {
                    // `transition` persists on the element, so a non-animated patch must
                    // CLEAR it — otherwise one animated patch makes every later plain
                    // background change animate too.
                    match anim {
                        Some(a) => s(
                            el,
                            "transition",
                            &format!("background-color {}", css_anim(a)),
                        ),
                        None => s(el, "transition", ""),
                    }
                    match c {
                        Some(c) => s(el, "background-color", &color_css(*c)),
                        None => s(el, "background-color", "transparent"),
                    }
                }
            }
            kinds::LABEL => {
                if let Some(p) = patch.downcast_ref::<LabelPatch>() {
                    match p {
                        LabelPatch::Text(t) => {
                            text(el, t);
                            // The measure cache is keyed by element — new text, new metrics.
                            MEASURE_CACHE.with(|c| c.borrow_mut().retain(|(e, _), _| *e != el));
                        }
                        LabelPatch::Runs(t, runs) => {
                            set_label_text(el, t, runs);
                            MEASURE_CACHE.with(|c| c.borrow_mut().retain(|(e, _), _| *e != el));
                        }
                        LabelPatch::Color(c) => match c {
                            Some(c) => s(el, "color", &color_css(*c)),
                            None => s(el, "color", ""),
                        },
                        LabelPatch::Font(f) => {
                            apply_font(el, f);
                            MEASURE_CACHE.with(|c| c.borrow_mut().retain(|(e, _), _| *e != el));
                        }
                    }
                }
            }
            kinds::BUTTON => {
                if let Some(p) = patch.downcast_ref::<ButtonPatch>() {
                    match p {
                        ButtonPatch::Title(t) => text(el, t),
                        ButtonPatch::Enabled(e) => set_enabled(el, *e),
                        ButtonPatch::Style(st) => apply_button_style(el, *st),
                    }
                }
            }
            kinds::TOGGLE => {
                if let Some(p) = patch.downcast_ref::<TogglePatch>() {
                    match p {
                        TogglePatch::On(on) => unsafe { day_dom_set_checked(el, *on as u32) },
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
                            attr(el, "readonly", if *e { "" } else { "-" })
                        }
                        TextAreaPatch::SetSelectable(sel) => {
                            s(el, "user-select", if *sel { "text" } else { "none" })
                        }
                        TextAreaPatch::SetSpellCheck(sc) => {
                            attr(el, "spellcheck", if *sc { "true" } else { "false" })
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
                            style: Default::default(),
                        });
                        unsafe { day_dom_options(el, json.as_ptr(), json.len()) };
                        if SEG_COUNT.with(|m| m.borrow().contains_key(&el)) {
                            SEG_COUNT.with(|m| m.borrow_mut().insert(el, opts.len()));
                        }
                    }
                    None => {}
                }
            }
            kinds::PROGRESS => {
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
                            if let Some(bg) = background {
                                s(el, "background-color", &color_css(*bg));
                            }
                            unsafe { day_dom_insert(1, el, u32::MAX) };
                            class(el, "open", true);
                            // Seed the frame SYNCHRONOUSLY from the cached viewport (the cover is
                            // fixed/inset:0, so that IS its size). Without this, the content lays
                            // out at width 0 until the async ResizeObserver fires — invisible in
                            // LTR's top-left but off-screen to the LEFT under RTL (docs/cover.md).
                            if let Some(node) = node_of(el) {
                                let vp = LAST_VIEWPORT.with(|v| v.get());
                                if vp.width > 0.0 && vp.height > 0.0 {
                                    emit(node, Event::FrameChanged(vp));
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
                    badge_icons,
                    badge_tints,
                    selected,
                    ..
                }) = patch.downcast_ref::<NavMenuPatch>()
                {
                    let json =
                        navmenu_json(items, icons, tints, badge_icons, badge_tints, *selected);
                    unsafe { day_dom_navmenu(el, json.as_ptr(), json.len()) };
                } else if let Some(NavMenuPatch::Selected(sel)) =
                    patch.downcast_ref::<NavMenuPatch>()
                {
                    unsafe { day_dom_navmenu_select(el, sel.map(|i| i as i32).unwrap_or(-1)) };
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
        // and future — before the manual purges below (day_spec::sidetable; day-dom keys by
        // shim element id, widened to the sweeper's usize key space).
        day_spec::sidetable::sweep(el as usize);
        NODE_OF.with(|m| m.borrow_mut().remove(&el));
        IMAGE_SRC.with(|m| m.borrow_mut().remove(&el));
        CSS_FRAMED.with(|s| s.borrow_mut().remove(&el));
        AREA_HINTS.with(|m| m.borrow_mut().remove(&el));
        PAGE_SIDEBAR.with(|m| m.borrow_mut().remove(&el));
        NAV_STATE.with(|m| m.borrow_mut().remove(&el));
        SEG_COUNT.with(|m| m.borrow_mut().remove(&el));
        PICKER_SIZE.with(|m| m.borrow_mut().remove(&el));
        SPINNERS.with(|s| s.borrow_mut().remove(&el));
        MEASURE_CACHE.with(|c| c.borrow_mut().retain(|(e, _), _| *e != el));
        if let Some(list) = LISTS.with(|m| m.borrow_mut().remove(&el)) {
            CELL_ROWS.with(|m| {
                let mut m = m.borrow_mut();
                for c in list.cells {
                    m.remove(&c);
                }
            });
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
            unsafe { day_dom_nav_add_page(parent.0, child.0, to_chrome as u32) };
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
                let mut out = [0.0f64; 2];
                unsafe {
                    day_dom_measure_text(
                        std::ptr::null(),
                        el as usize, // shim: null text ⇒ measure element `len`'s own text/font
                        std::ptr::null(),
                        0,
                        max_w,
                        out.as_mut_ptr(),
                    )
                };
                let sz = Size::new(out[0].ceil().min(max_w), out[1].ceil());
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
                    )
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
                    p.height.unwrap_or((l as f64) * line_h + pad),
                )
            }
            kinds::PICKER => PICKER_SIZE
                .with(|m| m.borrow().get(&el).copied())
                .unwrap_or(Size::new(68.0, 26.0)),
            // A spinner is SQUARE — `.day-spinner` is a 50%-radius ring that rotates, so any
            // other aspect ratio spins as an ellipse sweeping the layout (docs/pieces.md).
            // Determinate progress is the wide, short bar.
            kinds::PROGRESS => {
                if SPINNERS.with(|s| s.borrow().contains(&el)) {
                    Size::new(22.0, 22.0)
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

    fn set_frame(&mut self, h: &DomHandle, frame: Rect, _anim: Option<&AnimSpec>) {
        if CSS_FRAMED.with(|s| s.borrow().contains(&h.0)) {
            return;
        }
        unsafe {
            day_dom_set_frame(
                h.0,
                frame.origin.x,
                frame.origin.y,
                frame.size.width,
                frame.size.height,
            )
        };
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
        // Set-or-clear: a lingering `transition` would animate every later plain patch.
        match anim {
            Some(a) => s(h.0, "transition", &format!("opacity {}", css_anim(a))),
            None => s(h.0, "transition", ""),
        }
        s(h.0, "opacity", &opacity.to_string());
    }

    fn set_transform(&mut self, h: &DomHandle, t: Transform, _size: Size, anim: Option<&AnimSpec>) {
        // Set-or-clear, as in `set_opacity`.
        match anim {
            Some(a) => s(h.0, "transition", &format!("transform {}", css_anim(a))),
            None => s(h.0, "transition", ""),
        }
        // Mark this element as a compositing root so its rounded-clip descendants get their own
        // clipped layer (day.css `.day-xform .day-clip`) — see the note in `apply_surface`.
        class(h.0, "day-xform", true);
        s(h.0, "transform-origin", "50% 50%");
        s(
            h.0,
            "transform",
            &format!(
                "translate({}px,{}px) rotate({}deg) scale({},{})",
                t.tx, t.ty, t.rotate_deg, t.sx, t.sy
            ),
        );
    }

    fn set_selectable(&mut self, h: &DomHandle, selectable: bool) -> Option<DomHandle> {
        // Nothing is selectable by default (#day-root sets `user-select: none`); the class opts
        // this element and its text back in (`.day-selectable` in day.css). See docs/text.md.
        class(h.0, "day-selectable", selectable);
        None
    }

    fn set_scroll_content(&mut self, h: &DomHandle, content: Size) {
        unsafe { day_dom_scroll_content(h.0, content.width, content.height) };
    }

    fn scroll_to(&mut self, h: &DomHandle, target: Rect, animated: bool) {
        unsafe { day_dom_scroll_to(h.0, target.origin.x, target.origin.y, animated as u32) };
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
        let mask = match kind {
            GestureKind::Tap => 128,
            GestureKind::Drag => 256,
            // Long-press, pinch, and pan are not delivered on this backend yet
            // (docs/canvas.md "Zoom and pan").
            GestureKind::LongPress | GestureKind::Pinch | GestureKind::Pan => 0,
        };
        if mask != 0 {
            unsafe { day_dom_listen(h.0, mask) };
        }
    }

    fn focus(&mut self, h: &DomHandle, _node: NodeId, focused: bool) {
        unsafe { day_dom_focus(h.0, focused as u32) };
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
        unsafe { day_dom_set_hash(route.as_ptr(), route.len(), replace as u32) };
    }

    fn set_toolbar(&mut self, _h: &DomHandle, items: &[day_spec::ToolbarItem]) {
        // The web has no window chrome, so the bar is a strip the shim docks at the top of the
        // document (docs/toolbars.md). One spec rebuilds the whole strip; `update_toolbar`
        // carries the targeted changes so a search in progress is not rebuilt out from under
        // the user.
        let json = toolbar_json(items);
        unsafe { day_dom_toolbar(json.as_ptr(), json.len()) };
    }

    fn update_toolbar(&mut self, _h: &DomHandle, patch: &day_spec::ToolbarPatch) {
        use day_spec::ToolbarPatch as P;
        let mut json = String::from("{\"item\":");
        match patch {
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
        }
        json.push('}');
        unsafe { day_dom_toolbar_patch(json.as_ptr(), json.len()) };
    }
    fn toggle_sidebar(&mut self) -> bool {
        // Same call the strip's own button makes, so a dayscript walkthrough drives the real
        // path (docs/toolbars.md).
        unsafe { day_dom_toolbar_sidebar() != 0 }
    }

    fn set_app_menu(&mut self, _items: &[MenuItem]) {
        // No menu bar on the web (Cap-honest no-op; docs/menus.md).
    }

    fn set_context_menu(&mut self, _h: &DomHandle, _node: NodeId, _items: &[MenuItem]) {
        // MVP: no emulated context popover yet (docs/menus.md matrix).
    }

    fn supports_lifecycle(&self, phase: Lifecycle) -> bool {
        matches!(
            phase,
            Lifecycle::WillLaunch
                | Lifecycle::DidLaunch
                | Lifecycle::DidBecomeActive
                | Lifecycle::WillResignActive
        )
    }

    fn set_a11y(&mut self, h: &DomHandle, a11y: &A11yProps) {
        if let Some(label) = &a11y.label {
            attr(h.0, "aria-label", label);
        }
        // The Day element id (`.id("counter-label")`) becomes the DOM id — the same duty that
        // sets accessibilityIdentifier on Apple backends. Day ids are app-unique by contract
        // (dayscript addresses by them), so they satisfy the DOM's uniqueness rule too. Only
        // nodes with native handles get one, exactly like every other backend.
        if let Some(id) = &a11y.identifier {
            attr(h.0, "id", id);
        }
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
            )
        };
    }

    fn ui_idle(&mut self) -> bool {
        true
    }

    fn modifiers(&mut self) -> day_spec::Modifiers {
        let mask = unsafe { day_dom_modifiers() };
        day_spec::Modifiers {
            shift: mask & 1 != 0,
            primary: mask & 2 != 0,
            alt: mask & 4 != 0,
        }
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
                day_dom_present_save(req as u32, j.as_ptr(), j.len(), bytes.as_ptr(), bytes.len())
            };
            return;
        }
        let json = present_json(spec);
        match json {
            Some(j) => unsafe { day_dom_present(req as u32, j.as_ptr(), j.len()) },
            None => {
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
    }

    fn dismiss(&mut self, req: u64) {
        unsafe { day_dom_dismiss(req as u32) };
    }

    fn open_url(&mut self, url: &str) {
        unsafe { day_dom_open_url(url.as_ptr(), url.len()) };
    }

    fn dark_mode(&mut self) -> bool {
        DARK.with(|d| d.get())
    }

    fn set_appearance(&mut self, dark: Option<bool>) {
        let mode = match dark {
            Some(false) => 0,
            Some(true) => 1,
            None => 2,
        };
        let effective = unsafe { day_dom_set_dark(mode) };
        DARK.with(|d| d.set(effective == 1));
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

    fn run(self, options: WindowOptions, ready: Box<dyn FnOnce(Self, DomHandle, Size)>) {
        unsafe { day_dom_set_title(options.title.as_ptr(), options.title.len()) };
        let w: f64 = env("vw").parse().unwrap_or(1000.0);
        let h: f64 = env("vh").parse().unwrap_or(700.0);
        LAST_VIEWPORT.with(|v| v.set(Size::new(w, h)));
        DARK.with(|d| d.set(env("dark") == "1"));
        // Root container: the shim pre-registers the `#day-root` element under this id.
        let root = self.root;
        ready(self, DomHandle(root), Size::new(w, h));
    }

    fn post(f: Box<dyn FnOnce() + Send>) {
        POSTED.with(|q| q.borrow_mut().push(f));
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

    fn request_frame(cb: Box<dyn FnOnce(f64) + 'static>) {
        FRAME_CB.with(|c| *c.borrow_mut() = Some(cb));
        unsafe { day_dom_request_frame() };
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

fn apply_surface(el: u32, bg: Option<day_spec::Color>, radius: f64, clips: bool, card: bool) {
    if let Some(c) = bg {
        s(el, "background-color", &color_css(c));
    }
    if radius > 0.0 {
        s(el, "border-radius", &format!("{radius}px"));
        // A rounded clip inside a transformed (animated) ancestor hits a WebKit bug: the clip
        // layer's backing paints its SHARP bounding box black behind the rounded content. The
        // `.day-xform .day-clip` rule (day.css) gives such a clip its own correctly-clipped layer;
        // the class is inert everywhere else, so static rounded surfaces aren't promoted.
        class(el, "day-clip", true);
    }
    if clips || radius > 0.0 {
        s(el, "overflow", "hidden");
    }
    if card {
        class(el, "day-card", true);
    }
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
    match p.style {
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
    }
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

fn css_anim(a: &AnimSpec) -> String {
    let ease = match a.curve {
        Curve::Linear => "linear",
        Curve::EaseIn => "ease-in",
        Curve::EaseOut => "ease-out",
        Curve::EaseInOut => "ease-in-out",
        // The fixed-duration overshoot convention every backend uses for springs (§8.4).
        Curve::Spring { .. } => "cubic-bezier(0.34, 1.4, 0.5, 1)",
    };
    format!("{}ms {} {}ms", a.duration_ms, ease, a.delay_ms)
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
            NavPatch::Presentation(_) => {}
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
            NavPatch::Title(t) => {
                if let Some(last) = state.titles.last_mut() {
                    *last = t.clone();
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
    // layer keeps the selected page and disposes the rest, so a backend that showed the LAST
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
    unsafe { day_dom_nav_back_bar(el, visible as u32, title.as_ptr(), title.len()) };
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
        // A source that SHRANK leaves realized cells past its end. They stay in the pool (index
        // == row, so a source that grows back reuses each for the row it always held) and are
        // simply hidden — the same append-only pool, minus the eager building.
        let n = st.source.as_ref().map_or(0, |src| (src.len)());
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
        }
    });
    list_fill_window(host);
}

/// Build the rows the viewport shows and that are not built already. Idempotent and cheap when
/// nothing moved, which is what lets every scroll event call it.
fn list_fill_window(host: u32) {
    let Some((content, rowh, source, work, n, width)) = LISTS.with(|m| {
        let mut m = m.borrow_mut();
        let st = m.get_mut(&host)?;
        let source = st.source.clone()?;
        let (content, rowh, selectable) = (st.content, st.row_height.max(1.0), st.selectable);
        let n = (source.len)();
        let width = unsafe { day_dom_width(host) }.max(1.0);
        // The rows on screen, plus the overscan. A list the browser has not laid out yet reports
        // no height — build a screen's worth then, and let the scroll that follows extend it.
        let mut view = [0.0_f64; 2];
        unsafe { day_dom_list_viewport(host, view.as_mut_ptr()) };
        let (offset, vh) = (view[0], if view[1] > 0.0 { view[1] } else { 600.0 });
        let first = ((offset / rowh).floor() as usize).saturating_sub(LIST_OVERSCAN);
        let last = (((offset + vh) / rowh).ceil() as usize + LIST_OVERSCAN).min(n);
        // Slots exist for every row (a Vec of zeros, not of elements): the cell for row i lives
        // at i for good, which is what keeps the click handler's recorded row honest.
        if st.cells.len() < n {
            st.cells.resize(n, 0);
            st.bound.resize(n, false);
        }
        let mut work: Vec<(usize, u32)> = Vec::new();
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
                work.push((i, st.cells[i]));
            }
        }
        st.last_width = width;
        Some((content, rowh, source, work, n, width))
    }) else {
        return;
    };
    for (i, cell) in work {
        unsafe {
            day_dom_set_frame(cell, 0.0, i as f64 * rowh, width, rowh);
        }
        s(cell, "display", "block");
        (source.bind_row)(i, cell as usize as day_spec::RawHandle);
    }
    s(content, "position", "relative");
    // The extent is the WHOLE source, built or not: the scrollbar is how the user reaches rows
    // that do not exist yet, so it cannot be sized to what happens to be realized.
    s(content, "height", &format!("{}px", n as f64 * rowh));
    // Rows realized just now start unpainted, and a reload can move which rows are selected
    // under a selection that never changed — so repaint from the entry's set on every fill.
    LISTS.with(|m| {
        if let Some(st) = m.borrow().get(&host) {
            list_paint_selection(st);
        }
    });
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
fn list_reveal_row(host: u32, row: usize, row_height: f64) {
    let mut view = [0.0_f64; 2];
    unsafe { day_dom_list_viewport(host, view.as_mut_ptr()) };
    let (offset, vh) = (view[0], view[1]);
    if vh <= 0.0 {
        return; // not laid out yet; the fill that follows builds from the top anyway
    }
    let (top, bottom) = (row as f64 * row_height, (row + 1) as f64 * row_height);
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
/// shifted, reveals the row and reports the same event a click on it would.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_list_key(host: u32, dir: u32, mods: u32) {
    day_spec::ffi_guard::contain((), || {
        let moved = LISTS.with(|m| {
            let mut m = m.borrow_mut();
            let st = m.get_mut(&host)?;
            let n = st.source.as_ref().map_or(0, |src| (src.len)());
            if !st.selectable || n == 0 {
                return None;
            }
            // Entering an unselected list picks the row the arrow points AT: down lands on the
            // first row, up on the last, which is what every desktop list does.
            let cursor = list_cursor(st);
            let row = match dir {
                0 => cursor.map_or(n - 1, |c| c.saturating_sub(1)),
                1 => cursor.map_or(0, |c| (c + 1).min(n - 1)),
                2 => 0,
                3 => n - 1,
                _ => return None,
            };
            let shift = mods as u8 & day_spec::KeyEvent::SHIFT != 0;
            if st.multi && shift {
                // Shift extends from the anchor and leaves it where it was, moving only the
                // lead — so a run of shifted arrows grows and shrinks ONE range instead of
                // starting a new one from wherever the last one ended.
                let a = st.anchor.unwrap_or(row);
                st.selected = (a.min(row)..=a.max(row)).collect();
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
            Some((st.node, ev, row, st.row_height.max(1.0)))
        });
        let Some((node, ev, row, row_height)) = moved else {
            return;
        };
        list_reveal_row(host, row, row_height);
        emit(node, ev);
    });
}

fn list_patch(el: u32, p: &ListPatch) {
    match p {
        ListPatch::Reload | ListPatch::Splice(_) => post_local(move || list_populate(el)),
        ListPatch::RowSizeInvalidated(_) => {}
        ListPatch::ScrollToEnd => unsafe { day_dom_scroll_edge(el, 1, 1) },
        ListPatch::ScrollToRow(row) => {
            let y = LISTS.with(|m| {
                m.borrow()
                    .get(&el)
                    .map(|st| *row as f64 * st.row_height)
                    .unwrap_or(0.0)
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

/// A press on a list cell (docs/list.md): same semantics as the Qt emulated list.
fn list_cell_click(cell: u32, mods: u32) {
    let Some((host, row)) = CELL_ROWS.with(|m| m.borrow().get(&cell).copied()) else {
        return;
    };
    let emit_ev = LISTS.with(|m| {
        let mut m = m.borrow_mut();
        let st = m.get_mut(&host)?;
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
            st.selected = (a.min(row)..=a.max(row)).collect();
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
        Shape::Arc {
            rect,
            start_deg,
            sweep_deg,
        } => buf.extend([
            3.0,
            rect.origin.x,
            rect.origin.y,
            rect.size.width,
            rect.size.height,
            *start_deg,
            *sweep_deg,
        ]),
        Shape::Line(a, b) => buf.extend([4.0, a.x, a.y, b.x, b.y]),
        Shape::Polygon(pts) => {
            buf.extend([5.0, pts.len() as f64]);
            for p in pts {
                buf.extend([p.x, p.y]);
            }
        }
        // Path: [6, rule, segCount, then per segment: kind + its points]. Self-describing, so
        // the shim walks it without a length table.
        Shape::Path(path) => {
            buf.extend([
                6.0,
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
                        buf.extend([3.0, c1.x, c1.y, c2.x, c2.y, a.x, a.y])
                    }
                    day_spec::PathSeg::Close => buf.push(4.0),
                }
            }
        }
    }
}

fn push_paint(buf: &mut Vec<f64>, paint: &Paint, bounds: Rect) {
    match paint {
        Paint::Solid(c) => buf.extend([0.0, pack_color(*c)]),
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
                buf.extend([*off, pack_color(*c)]);
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
                buf.extend([*off, pack_color(*c)]);
            }
        }
    }
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
            DrawOp::Stroke(shape, paint, style) => {
                // [1, width, cap, join, miter, dashPhase, dashCount, dashes…] then paint, then
                // shape. Style is inline rather than a separate record: this encoder is already
                // variable-length, so there is nothing to gain from a modifier record here.
                buf.extend([
                    1.0,
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
            } => {
                let off = strs.len() as f64;
                strs.extend_from_slice(text.as_bytes());
                buf.extend([
                    2.0,
                    pack_color(*color),
                    *size,
                    match anchor {
                        TextAnchor::Leading => 0.0,
                        TextAnchor::Centered => 1.0,
                    },
                    at.x,
                    at.y,
                    off,
                    text.len() as f64,
                ]);
            }
            DrawOp::Save => buf.push(3.0),
            DrawOp::Restore => buf.push(4.0),
            DrawOp::Concat(m) => buf.extend([5.0, m.a, m.b, m.c, m.d, m.tx, m.ty]),
        }
    }
    (buf, strs)
}

// ---------------------------------------------------------------------------
// Presentation (docs/dialogs.md): <dialog>-backed alert/confirm/sheet/prompt.
// ---------------------------------------------------------------------------

fn present_json(spec: &PresentSpec) -> Option<String> {
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
            j.push_str(&format!(",\"sheet\":{sheet},\"buttons\":["));
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
        // The open picker: the shim clicks a hidden `<input type=file>` and answers with the
        // chosen file's name and bytes through `day_dom_present_files`.
        PresentSpec::OpenFile { title, filters } => {
            j.push_str("\"kind\":\"open\",\"title\":");
            json_str(&mut j, title);
            let accept: Vec<String> = filters
                .iter()
                .flat_map(|f| f.extensions.iter())
                .map(|e| format!(".{e}"))
                .collect();
            j.push_str(",\"accept\":");
            json_str(&mut j, &accept.join(","));
        }
        // Save rides its own FFI arm (bytes attached); reaching here means a non-web build.
        PresentSpec::SaveFile { .. } => return None,
    }
    j.push('}');
    Some(j)
}

fn push_button(j: &mut String, b: &PresentButton) {
    j.push_str("{\"label\":");
    json_str(j, &b.label);
    let role = match b.role {
        day_spec::present::ButtonRole::Default => "default",
        day_spec::present::ButtonRole::Cancel => "cancel",
        day_spec::present::ButtonRole::Destructive => "destructive",
    };
    j.push_str(&format!(",\"role\":\"{role}\"}}"));
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

fn take_string(ptr: *mut u8, len: usize) -> String {
    // SAFETY: the shim wrote exactly `len` bytes into a `day_dom_alloc(len)` allocation.
    let v = unsafe { Vec::from_raw_parts(ptr, len, len) };
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

fn day_dom_event_inner(el: u32, kind: u32, a: f64, b: f64, c: f64, d: f64) {
    if kind == ev::CLICK && CELL_ROWS.with(|m| m.borrow().contains_key(&el)) {
        list_cell_click(el, a as u32);
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
        ev::SCROLL => Event::ScrollChanged(Point::new(a, b)),
        ev::RESIZED => Event::FrameChanged(Size::new(a, b)),
        ev::NAV_BACK => Event::NavBack {
            already_popped: false,
        },
        _ => return,
    };
    emit(node, event);
}

/// A piece-defined Custom event from the shim (docs/extending.md §8.2's open channel): `num` is
/// the piece's own discriminator — the inline web view's link reports use -1 (docs/webview.md) —
/// and `text` the payload. The mirror of the Android bridge's kind-12 and ArkUI's `pieceEvent`.
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
                    _ => Event::TextChanged(t),
                },
            );
        }
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

/// A file picker's answer: `name` is the chosen file's display name; `bytes` its content for
/// an open flow (len 0 for a save flow, whose bytes already left as a download). The bytes
/// land in the `web_files` store under `/day-web/<name>`, and that virtual path answers the
/// awaiting flow as `PresentResult::Files` — the pieces layer reads it back like a local path
/// (docs/files.md).
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
    let bytes = if bytes_len > 0 {
        // SAFETY: the shim wrote exactly `bytes_len` bytes into a `day_dom_alloc` allocation.
        Some(unsafe { Vec::from_raw_parts(bytes_ptr, bytes_len, bytes_len) })
    } else {
        None
    };
    day_spec::ffi_guard::contain((), move || {
        // The last path component only — a hostile/odd name must not escape the store's prefix.
        let leaf = name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("file")
            .to_string();
        let path = format!("/day-web/{leaf}");
        #[cfg(all(target_family = "wasm", target_os = "unknown"))]
        if let Some(b) = bytes {
            day_spec::present::web_files::write(&path, b);
        }
        #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
        drop(bytes);
        emit(
            day_spec::WINDOW_NODE,
            Event::PresentResult {
                req: u64::from(req),
                result: PresentResult::Files(vec![path]),
            },
        );
    });
}

/// A document-level clipboard event (the browser's own ⌘X/⌘C/⌘V route, or the Edit menu of
/// the browser itself) that no editable element claimed — the shim's `copy`/`cut`/`paste`
/// listeners forward it here while the event is still live, so day-part-clipboard's
/// synchronous calls inside the app's handler read and write the event's `clipboardData`
/// (docs/menus.md).
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

/// An arrow pressed while THIS canvas has focus (docs/menus.md). Returns whether the app
/// claimed it — a canvas nobody hung a key handler on keeps none of them, so the browser's own
/// scrolling still works underneath it.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_canvas_key(el: u32, code: u32, modifiers: u32) -> u32 {
    day_spec::ffi_guard::contain(0, || {
        let key = match code {
            0 => "ArrowLeft",
            1 => "ArrowRight",
            2 => "ArrowUp",
            3 => "ArrowDown",
            4 => "Delete",
            _ => "Backspace",
        };
        let Some(node) = node_of(el) else {
            return 0;
        };
        if !day_spec::keys::handled(node) {
            return 0;
        }
        emit(
            node,
            Event::Key(day_spec::KeyEvent {
                key: key.to_string(),
                modifiers: modifiers as u8,
            }),
        );
        1
    })
}

/// The platform-standard undo shortcut (⌘Z / Ctrl+Z, shift or Ctrl+Y for redo) pressed with
/// no editable element focused — the shim's keydown route (the browser has no document-level
/// undo of its own to integrate with, so the standard keys ARE the platform affordance).
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

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_posted() {
    let q: Vec<_> = POSTED.with(|q| q.borrow_mut().drain(..).collect());
    // Contained per closure, so one panicking post cannot drop the ones queued behind it.
    for f in q {
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
pub extern "C" fn day_dom_frame(ts: f64) {
    if let Some(cb) = FRAME_CB.with(|c| c.borrow_mut().take()) {
        day_spec::ffi_guard::contain((), move || cb(ts));
    }
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

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_lifecycle(phase: u32) {
    let phase = match phase {
        0 => Lifecycle::DidBecomeActive,
        1 => Lifecycle::WillResignActive,
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
pub fn launch_route() -> Option<String> {
    let route = env("route");
    (!route.is_empty()).then_some(route)
}

/// The page's dayscript token (`?dayscript=`), the query-parameter spelling of
/// `DAYSCRIPT_TOKEN` — present only when a `day launch` scripted/drivable session serves the
/// page (docs/web.md). The `day::web` glue arms the engine's web transport with it.
pub fn dayscript_token() -> Option<String> {
    let token = env("dayscript");
    (!token.is_empty()).then_some(token)
}

/// Send one dayscript reply line to the page (the engine's web sender — docs/web.md).
pub fn script_send(line: &str) {
    unsafe { day_dom_script_send(line.as_ptr(), line.len()) };
}

/// Reclaim a shim-written string (a `day_dom_alloc` buffer) — for exports OUTSIDE this crate
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
/// `day launch -p web-dom --env DAY_LOG=debug` reaches this.
pub fn launch_log_level() -> Option<log::LevelFilter> {
    host_env("DAY_LOG").and_then(|v| v.parse().ok())
}
