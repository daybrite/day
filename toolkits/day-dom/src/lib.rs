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
    fn day_dom_set_app_badge(count: i32);
    fn day_dom_insert(parent: u32, child: u32, index: u32);
    fn day_dom_remove(child: u32);
    fn day_dom_release(el: u32);
    fn day_dom_set_frame(el: u32, x: f64, y: f64, w: f64, h: f64);
    fn day_dom_set_text(el: u32, ptr: *const u8, len: usize);
    fn day_dom_set_style(el: u32, p: *const u8, pl: usize, v: *const u8, vl: usize);
    fn day_dom_set_attr(el: u32, a: *const u8, al: usize, v: *const u8, vl: usize);
    fn day_dom_set_class(el: u32, ptr: *const u8, len: usize, on: u32);
    fn day_dom_set_value(el: u32, v: f64);
    fn day_dom_set_checked(el: u32, on: u32);
    /// Attach shim listeners; `mask` bits: 1 click, 2 input, 4 change, 8 focus, 16 submit,
    /// 32 resize-observer, 64 scroll, 128 pointer-tap, 256 pointer-drag.
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
    fn day_dom_dismiss(req: u32);
    fn day_dom_nav_mode(el: u32, split: u32, title: *const u8, tl: usize);
    /// Rebuild a live host's chrome for the other presentation, detaching its pages (which stay
    /// alive in the shim's registry) for Day to re-home.
    fn day_dom_nav_present(el: u32, split: u32);
    fn day_dom_nav_add_page(nav: u32, page: u32, sidebar: u32);
    fn day_dom_nav_back_bar(nav: u32, visible: u32, t: *const u8, tl: usize);
    fn day_dom_navmenu(el: u32, json: *const u8, len: usize);
    // Window toolbar (docs/toolbars.md): the whole bar crosses as one JSON spec, the way the
    // nav menu does; targeted patches address an item by id. `day_dom_toolbar_sidebar` returns
    // 0 when the page has no split nav to toggle.
    fn day_dom_toolbar(json: *const u8, len: usize);
    fn day_dom_toolbar_patch(json: *const u8, len: usize);
    fn day_dom_toolbar_sidebar() -> u32;
    fn day_dom_navmenu_select(el: u32, idx: i32);
    fn day_dom_tabs(el: u32, json: *const u8, len: usize);
    fn day_dom_tabs_select(el: u32, idx: u32);
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

/// Read a host "environment" value (query params / navigator facts) into a String.
fn env(key: &str) -> String {
    let mut buf = vec![0u8; 512];
    let n = unsafe { day_dom_env(key.as_ptr(), key.len(), buf.as_mut_ptr(), buf.len()) };
    buf.truncate(n.min(512));
    String::from_utf8_lossy(&buf).into_owned()
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
const EL_TABS: u32 = 17;
const EL_CELL: u32 = 18;
const EL_SEGMENTED: u32 = 19;
const EL_RADIOS: u32 = 20;

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
    split: bool,
    /// Detail (stack) pages in push order. In a `Stack` presentation the sidebar page is the
    /// stack's root and so appears here FIRST; in `Split` it lives in its own pane and does not.
    /// A re-present moves it between the two (see `nav_present`).
    pages: Vec<u32>,
    /// The host's sidebar page, once it has one. Tracked by identity so a re-present can re-home
    /// it without depending on where it currently sits.
    sidebar: Option<u32>,
    titles: Vec<String>,
}

struct ListEntry {
    node: NodeId,
    content: u32,
    row_height: f64,
    source: Option<ListSource>,
    cells: Vec<u32>,
    last_width: f64,
    selectable: bool,
    multi: bool,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
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
    let size = match f.style {
        Font::System(_) | Font::Custom(_, _) => format!("{rem}rem"),
        _ => scaled_rem(rem),
    };
    let weight = f.weight.map(weight_css).unwrap_or(default_weight);
    let italic = if f.italic { "italic " } else { "" };
    let family = match f.style {
        Font::Custom(name, _) => format!("'{name}', {SYSTEM_STACK}"),
        _ => SYSTEM_STACK.to_string(),
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

/// Build the tabs JSON (`{titles, selected}`) the shim's `day_dom_tabs` consumes (idempotent —
/// it rebuilds the tab strip), shared by TABS realize and `TabsPatch::Items`.
fn tabs_json(titles: &[String], selected: usize) -> String {
    let mut json = String::from("{\"titles\":[");
    for (i, t) in titles.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        json_str(&mut json, t);
    }
    json.push_str("],\"selected\":");
    json.push_str(&selected.to_string());
    json.push('}');
    json
}

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
            json_str(
                &mut json,
                &format!("assets/images/{icon}.{}", image_ext(icon)),
            );
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
            json_str(
                &mut json,
                &format!("assets/images/{badge}.{}", image_ext(badge)),
            );
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
        // A bundled image crosses as a staged URL; a Symbol has no web icon set to draw from,
        // so the label carries the item on its own rather than inventing a glyph.
        if let Some(day_spec::Icon::Image(name)) = &it.icon {
            json.push_str(",\"icon\":");
            json_str(
                &mut json,
                &format!("assets/images/{name}.{}", image_ext(name)),
            );
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
            Cap::Appearance | Cap::Dialogs | Cap::Animation => Support::Native,
            // Exact metrics from a canvas TextMetrics, but derived rather than read off a
            // baseline the platform publishes (docs/baseline.md).
            Cap::BaselineAlignment => Support::Emulated,
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
                let p = props.downcast_ref::<ContainerProps>().unwrap();
                let el = unsafe { day_dom_create(EL_DIV) };
                apply_surface(el, p.background, p.corner_radius, p.clips, p.role.is_some());
                el
            }
            Some(Builtin::Label) => {
                let p = props.downcast_ref::<LabelProps>().unwrap();
                let el = unsafe { day_dom_create(EL_LABEL) };
                text(el, &p.text);
                apply_font(el, &p.font);
                if let Some(c) = p.color {
                    s(el, "color", &color_css(c));
                }
                if !p.wraps {
                    s(el, "white-space", "nowrap");
                }
                el
            }
            Some(Builtin::Button) => {
                let p = props.downcast_ref::<ButtonProps>().unwrap();
                let el = unsafe { day_dom_create(EL_BUTTON) };
                text(el, &p.title);
                match p.style {
                    ButtonStyleSpec::Prominent => class(el, "prominent", true),
                    ButtonStyleSpec::Bordered => class(el, "bordered", true),
                    ButtonStyleSpec::Automatic => {}
                }
                if !p.enabled {
                    attr(el, "disabled", "-");
                }
                unsafe { day_dom_listen(el, 1) };
                el
            }
            Some(Builtin::Toggle) => {
                let p = props.downcast_ref::<ToggleProps>().unwrap();
                let el = unsafe { day_dom_create(EL_TOGGLE) };
                unsafe { day_dom_set_checked(el, p.on as u32) };
                if !p.enabled {
                    attr(el, "disabled", "-");
                }
                unsafe { day_dom_listen(el, 4) };
                el
            }
            Some(Builtin::Slider) => {
                let p = props.downcast_ref::<SliderProps>().unwrap();
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
                let p = props.downcast_ref::<TextFieldProps>().unwrap();
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
                let p = props.downcast_ref::<TextAreaProps>().unwrap();
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
                let p = props.downcast_ref::<PickerProps>().unwrap();
                realize_picker(p)
            }
            Some(Builtin::Progress) => {
                let p = props.downcast_ref::<ProgressProps>().unwrap();
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
                let p = props.downcast_ref::<ImageProps>().unwrap();
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
            Some(Builtin::Canvas) => unsafe { day_dom_create(EL_CANVAS) },
            Some(Builtin::Scroll) => {
                let p = props.downcast_ref::<ScrollProps>().unwrap();
                let el = unsafe { day_dom_create(EL_SCROLL) };
                if p.horizontal {
                    class(el, "horizontal", true);
                }
                unsafe { day_dom_listen(el, 64) };
                el
            }
            Some(Builtin::Divider) => unsafe { day_dom_create(EL_DIVIDER) },
            Some(Builtin::Nav) => {
                let p = props.downcast_ref::<NavProps>().unwrap();
                let split = p.presentation.is_split();
                let el = unsafe { day_dom_create(EL_NAV) };
                unsafe { day_dom_nav_mode(el, split as u32, p.title.as_ptr(), p.title.len()) };
                unsafe { day_dom_listen(el, 1) }; // the back bar's button reports via CLICK
                NAV_STATE.with(|m| {
                    m.borrow_mut().insert(
                        el,
                        NavState {
                            split,
                            pages: Vec::new(),
                            sidebar: None,
                            titles: vec![p.title.clone()],
                        },
                    )
                });
                el
            }
            Some(Builtin::NavPage) => {
                let p = props.downcast_ref::<NavPageProps>().unwrap();
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
                let p = props.downcast_ref::<NavMenuProps>().unwrap();
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
            Some(Builtin::Tabs) => {
                let p = props.downcast_ref::<TabsProps>().unwrap();
                let el = unsafe { day_dom_create(EL_TABS) };
                let json = tabs_json(&p.titles, p.selected);
                unsafe { day_dom_tabs(el, json.as_ptr(), json.len()) };
                el
            }
            Some(Builtin::TabsPage) => {
                let el = unsafe { day_dom_create(EL_PAGE) };
                CSS_FRAMED.with(|set| set.borrow_mut().insert(el));
                unsafe { day_dom_listen(el, 32) };
                el
            }
            Some(Builtin::List) => {
                let p = props.downcast_ref::<ListProps>().unwrap();
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
                            last_width: -1.0,
                            selectable: p.selectable,
                            multi: p.multi_select,
                            selected: BTreeSet::new(),
                            anchor: None,
                        },
                    )
                });
                host
            }
            // A recycled list cell is ADOPTED from the native list, never realized through
            // this path; anything else is an extension piece.
            Some(Builtin::ListCell) | None => {
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
                        // Only the colour changes here: an element realized with a tint is
                        // already the masked div, so this repaints the mask's fill.
                        apply_image_tint(el, &src, "contain", *c);
                    }
                }
            }
            kinds::CONTAINER => {
                if let Some(ContainerPatch::Background(c)) = patch.downcast_ref::<ContainerPatch>()
                {
                    if let Some(a) = anim {
                        s(
                            el,
                            "transition",
                            &format!("background-color {}", css_anim(a)),
                        );
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
                if let Some(PickerPatch::Selected(i)) = patch.downcast_ref::<PickerPatch>() {
                    // Programmatic DOM value sets fire no events — echo-free by construction.
                    let seg = SEG_COUNT.with(|m| m.borrow().get(&el).copied());
                    match seg {
                        Some(_) => unsafe { day_dom_tabs_select(el, *i as u32) },
                        None => unsafe { day_dom_set_value(el, *i as f64) },
                    }
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
            // ResizeObserver reports the frame. Dismiss = hide + "cover-hidden" at once.
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
                                emit(node, Event::custom("cover-hidden", ""));
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
            kinds::TABS => {
                if let Some(TabsPatch::Items {
                    titles, selected, ..
                }) = patch.downcast_ref::<TabsPatch>()
                {
                    // Pages were added/removed via insert/remove; rebuild the strip + select.
                    let json = tabs_json(titles, *selected);
                    unsafe { day_dom_tabs(el, json.as_ptr(), json.len()) };
                } else if let Some(TabsPatch::Selected(i)) = patch.downcast_ref::<TabsPatch>() {
                    unsafe { day_dom_tabs_select(el, *i as u32) };
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
            unsafe { day_dom_nav_add_page(parent.0, child.0, (state.split && sidebar) as u32) };
            if !(state.split && sidebar) {
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
        if let Some(a) = anim {
            s(h.0, "transition", &format!("opacity {}", css_anim(a)));
        }
        s(h.0, "opacity", &opacity.to_string());
    }

    fn set_transform(&mut self, h: &DomHandle, t: Transform, _size: Size, anim: Option<&AnimSpec>) {
        if let Some(a) = anim {
            s(h.0, "transition", &format!("transform {}", css_anim(a)));
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
            GestureKind::LongPress => 0,
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

    fn present(&mut self, req: u64, spec: &PresentSpec) {
        let json = present_json(spec);
        match json {
            Some(j) => unsafe { day_dom_present(req as u32, j.as_ptr(), j.len()) },
            None => {
                // Unsupported spec (file pickers, MVP): answer dismissed so the await resolves.
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
            unsafe { day_dom_tabs(el, json.as_ptr(), json.len()) }; // shim fills <option>s
            unsafe { day_dom_listen(el, 4) };
            PICKER_SIZE.with(|m| m.borrow_mut().insert(el, Size::new(longest + 38.0, 26.0)));
            el
        }
        PickerStyle::Segmented | PickerStyle::Inline => {
            let segmented = p.style == PickerStyle::Segmented;
            let el = unsafe { day_dom_create(if segmented { EL_SEGMENTED } else { EL_RADIOS }) };
            unsafe { day_dom_tabs(el, json.as_ptr(), json.len()) };
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
        nav_present(el, next.is_split());
        return;
    }
    NAV_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let Some(state) = m.get_mut(&el) else { return };
        match p {
            // Handled above, before the borrow.
            NavPatch::Presentation(_) => {}
            NavPatch::Pushed { title, .. } => {
                state.titles.push(title.clone());
                let last = state.pages.len().saturating_sub(1);
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
fn nav_present(el: u32, split: bool) {
    let Some((sidebar, mut pages, was_split)) = NAV_STATE.with(|m| {
        let st = m.borrow();
        let s = st.get(&el)?;
        Some((s.sidebar, s.pages.clone(), s.split))
    }) else {
        return;
    };
    if was_split == split {
        return;
    }
    // Move the sidebar page in or out of the stack's page list.
    if let Some(side) = sidebar {
        pages.retain(|p| *p != side);
        if !split {
            pages.insert(0, side);
        }
    }
    unsafe { day_dom_nav_present(el, split as u32) };
    if let Some(side) = sidebar
        && split
    {
        unsafe { day_dom_nav_add_page(el, side, 1) };
        // A sidebar pane always shows its page; it may have been hidden as a stack root under a
        // pushed detail.
        s(side, "display", "block");
    }
    let last = pages.len().saturating_sub(1);
    for (i, page) in pages.iter().enumerate() {
        unsafe { day_dom_nav_add_page(el, *page, 0) };
        s(*page, "display", if i == last { "block" } else { "none" });
    }
    // The back bar belongs to the stack presentation alone, and its visibility depends on the
    // page count we just settled — so commit the state first, then sync from it.
    NAV_STATE.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(st) = m.get_mut(&el) {
            st.split = split;
            st.pages = pages;
            sync_back_bar(el, st);
        }
    });
}

/// Stack presentation: the back bar shows while pushed pages are on top (`depth` counts the
/// pages that will remain after the in-flight patch). Split mode never shows it.
fn sync_back_bar_at(el: u32, state: &NavState, depth: usize) {
    let visible = !state.split && depth >= 1 && state.titles.len() > 1;
    let title = state.titles.last().cloned().unwrap_or_default();
    unsafe { day_dom_nav_back_bar(el, visible as u32, title.as_ptr(), title.len()) };
}

// ---------------------------------------------------------------------------
// Emulated list (docs/list.md): eager cells over the ListSource pull contract, the Qt shape.
// ---------------------------------------------------------------------------

fn list_populate(host: u32) {
    let Some((content, rowh, source, mut cells, _node, selectable)) = LISTS.with(|m| {
        let mut m = m.borrow_mut();
        let st = m.get_mut(&host)?;
        let source = st.source.clone()?;
        Some((
            st.content,
            st.row_height,
            source,
            st.cells.clone(),
            st.node,
            st.selectable,
        ))
    }) else {
        return;
    };
    let n = (source.len)();
    let width = unsafe { day_dom_width(host) }.max(1.0);
    while cells.len() < n {
        let cell = unsafe { day_dom_create(EL_CELL) };
        unsafe { day_dom_insert(content, cell, cells.len() as u32) };
        if selectable {
            unsafe { day_dom_listen(cell, 1) };
            CELL_ROWS.with(|m| m.borrow_mut().insert(cell, (host, cells.len())));
        }
        cells.push(cell);
    }
    for (i, &cell) in cells.iter().enumerate().take(n) {
        unsafe {
            day_dom_set_frame(cell, 0.0, i as f64 * rowh, width, rowh);
        }
        s(cell, "display", "block");
        (source.bind_row)(i, cell as usize as day_spec::RawHandle);
    }
    for &cell in cells.iter().skip(n) {
        s(cell, "display", "none");
    }
    s(content, "position", "relative");
    s(content, "height", &format!("{}px", n as f64 * rowh));
    LISTS.with(|m| {
        if let Some(st) = m.borrow_mut().get_mut(&host) {
            st.cells = cells;
        }
    });
}

fn list_paint_selection(entry: &ListEntry) {
    for (i, &cell) in entry.cells.iter().enumerate() {
        class(cell, "selected", entry.selected.contains(&i));
    }
}

fn list_patch(el: u32, p: &ListPatch) {
    match p {
        ListPatch::Reload => post_local(move || list_populate(el)),
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
                    st.selected = rows.iter().copied().collect();
                    st.anchor = rows.last().copied();
                    list_paint_selection(st);
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
        } else if st.multi && shift {
            let a = st.anchor.unwrap_or(row);
            st.selected = (a.min(row)..=a.max(row)).collect();
        } else {
            st.selected = std::iter::once(row).collect();
            st.anchor = Some(row);
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
            DrawOp::Stroke(shape, color, width) => {
                buf.extend([1.0, pack_color(*color), *width]);
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
        PresentSpec::OpenFile { .. } | PresentSpec::SaveFile { .. } => return None,
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
}

/// Commit a drop the guard accepts: rotate Day's snapshot (deferring the app callback), then
/// reposition + rebind the pooled cells to the new order. Returns 1 on commit, 0 on deny.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_list_move(host: u32, from: u32, to: u32) -> u32 {
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
}

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_event(el: u32, kind: u32, a: f64, b: f64, c: f64, d: f64) {
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
}

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_event_text(el: u32, _kind: u32, ptr: *mut u8, len: usize) {
    let t = take_string(ptr, len);
    if let Some(node) = node_of(el) {
        emit(node, Event::TextChanged(t));
    }
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
    emit(
        day_spec::WINDOW_NODE,
        Event::PresentResult {
            req: u64::from(req),
            result,
        },
    );
}

/// A toolbar button or menu entry was chosen — the same `MenuAction` every other backend
/// emits. The action id crosses as an f64: it is a small counter, far inside the range an f64
/// represents exactly.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_toolbar_action(action: f64) {
    emit(day_spec::WINDOW_NODE, Event::MenuAction(action as u64));
}

/// A toolbar toggle flipped.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_toolbar_on(action: f64, on: u32) {
    emit(
        day_spec::WINDOW_NODE,
        Event::ToolbarChanged {
            action: action as u64,
            value: day_spec::ToolbarValue::On(on != 0),
        },
    );
}

/// A toolbar search field's text changed. `take_string` takes ownership of the shim's
/// allocation, the way the other text callbacks here do.
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_toolbar_text(action: f64, ptr: *mut u8, len: usize) {
    let text = take_string(ptr, len);
    emit(
        day_spec::WINDOW_NODE,
        Event::ToolbarChanged {
            action: action as u64,
            value: day_spec::ToolbarValue::Text(text),
        },
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_posted() {
    let q: Vec<_> = POSTED.with(|q| q.borrow_mut().drain(..).collect());
    for f in q {
        f();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_delayed(token: u32) {
    if let Some(f) = DELAYED.with(|m| m.borrow_mut().remove(&token)) {
        f();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_frame(ts: f64) {
    if let Some(cb) = FRAME_CB.with(|c| c.borrow_mut().take()) {
        cb(ts);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_resized(w: f64, h: f64) {
    LAST_VIEWPORT.with(|v| v.set(Size::new(w, h)));
    // day-core re-buckets the window's size class from this (docs/size-classes.md) — a backend
    // reports geometry, not classes, so there is one breakpoint table rather than nine.
    emit(day_spec::WINDOW_NODE, Event::WindowResized(Size::new(w, h)));
}

#[unsafe(no_mangle)]
pub extern "C" fn day_dom_lifecycle(phase: u32) {
    let phase = match phase {
        0 => Lifecycle::DidBecomeActive,
        1 => Lifecycle::WillResignActive,
        _ => return,
    };
    emit(day_spec::WINDOW_NODE, Event::Lifecycle(phase));
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
    emit(day_spec::WINDOW_NODE, Event::RouteRequested(route));
}

/// Install a panic hook that reports through the shim's console before the trap.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        warn(&format!("day panic: {info}"));
    }));
}
