// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// HarmonyOS (ArkUI): the settings pages are the platform's own `List` (docs/settings.md). The
// root page and each group page is an `ARKUI_NODE_LIST` of `ARKUI_NODE_LIST_ITEM_GROUP` cards,
// one per section, each with its header and footer texts set through the group's own
// header/footer slots, and every row an `ARKUI_NODE_LIST_ITEM` painted as a card segment (the
// first rounded at the top, the last at the bottom, hairline dividers between) that shows the
// pressed fill a tappable HarmonyOS list item shows. The row is a container piece
// (docs/extending.md §5): the item node this module creates hosts the SAME Day control the
// composed row builds (`toggle`, `picker`, `slider`, `text_field`, `button`, carrying the row's
// element id and `enabled`), positioned by `RowLayout`, so dayscript, accessibility and the
// two-way bindings are untouched. The texts (title, caption, a read-only value, the chevron of a
// row that pushes a page) are ArkUI Text nodes of the row's own.
//
// Day owns layout and frames every node absolutely (`NODE_POSITION`), which takes an ArkUI node
// out of its parent's flow; a List measures its extent from the flow. So every list-mode node
// (an item, a group, the host of a composed piece) resets its own position in its `place`,
// right after the engine framed it: it keeps the size Day gave it and the List stacks it where
// Day computed it would be, since both lay the rows out top to bottom with the same heights.
// The same rows also serve outside a list (`Prefs::group_body` in an app's sheet): there the
// row's root is a plain Stack inside the shared `section()` card.
//
// Compile-checked only (the OHOS emulator is not run here, by ruling).
// ---------------------------------------------------------------------------

use super::*;
use day_arkui::arkui_sys::native_node::{
    ArkUI_NodeAttributeType as Attr, ArkUI_NodeEvent, ArkUI_NodeEventType as Ev, ArkUI_NodeType,
    OH_ArkUI_NodeEvent_GetEventType, OH_ArkUI_NodeEvent_GetInputEvent,
    OH_ArkUI_NodeEvent_GetNodeHandle,
};
use day_arkui::arkui_sys::native_type::ArkUI_TextOverflow;
use day_arkui::arkui_sys::ui_input_event::OH_ArkUI_UIInputEvent_GetAction;
use day_arkui::node::Handle;
use day_arkui::{AHandle, ArkUi, font_vp, node};
use day_core::{Boundary, Flex, Layout, LayoutOps, RNode, with_tree};
use day_spec::{Event, Font, FontSpec, NodeId, Proposal, Rect, Size};

const KIND: &str = "day-piece-settings.row";
const GROUP_KIND: &str = "day-piece-settings.group";
const PAGE_KIND: &str = "day-piece-settings.page";
const HOST_KIND: &str = "day-piece-settings.item";

// The HarmonyOS list-item metrics, in vp: a single-line item is 48 high, a two-line one 64,
// with 8 of vertical padding around taller content; 12 between the text and the trailing part,
// and 12 inside a card's edges.
const ROW_MIN: f64 = 48.0;
const ROW_MIN_TWO_LINE: f64 = 64.0;
const PAD_V: f64 = 8.0;
const GAP: f64 = 12.0;
const LINE_GAP: f64 = 2.0;
const CARD_PAD_H: f64 = 12.0;
const CARD_RADIUS: f64 = 16.0;
/// The list's inset from the page edges, the gap between cards, and the page's top and
/// bottom breathing room.
const LIST_INSET: f64 = 12.0;
const CARD_SPACE: f64 = 12.0;
const PAGE_TOP: f64 = 8.0;
const PAGE_BOTTOM: f64 = 24.0;
/// A section header above its card and a footer under it: the subheader shape, inset to the
/// card's text column.
const HEADER_TOP: f64 = 16.0;
const HEADER_BOTTOM: f64 = 8.0;
const FOOTER_TOP: f64 = 8.0;
const FOOTER_BOTTOM: f64 = 4.0;
/// The widest the title column goes while a stretching control (a slider, a field) takes the
/// rest; a short title leaves the control more.
const TITLE_SHARE: f64 = 0.5;
/// The leading glyph of a root-list row, and the About header's icon.
const ICON: f64 = 24.0;
const ABOUT_ICON: f64 = 40.0;

// The toolkit's neutral text paints (day-arkui's label colors), the HarmonyOS accent, the card
// fill of a settings list, the hairline between its items, and the translucent pressed fill a
// list item shows; translucent so it reads on both modes.
const TEXT_LIGHT: u32 = 0xE500_0000;
const TEXT_DARK: u32 = 0xE6FF_FFFF;
const SECONDARY_LIGHT: u32 = 0x9900_0000;
const SECONDARY_DARK: u32 = 0x99FF_FFFF;
const ACCENT_LIGHT: u32 = 0xFF00_7DFF;
const ACCENT_DARK: u32 = 0xFF3E_9BFF;
const CARD_LIGHT: u32 = 0xFFFF_FFFF;
const CARD_DARK: u32 = 0xFF2E_3033;
const DIVIDER: u32 = 0x1F80_8080;
const PRESSED: u32 = 0x1480_8080;
const CLEAR: u32 = 0x0000_0000;

// `OH_ArkUI_UIInputEvent_GetAction` for a touch (ui_input_event.h `UI_TOUCH_EVENT_ACTION_*`);
// the bindings this SDK level ships leave the anonymous enum out.
const TOUCH_CANCEL: i32 = 0;
const TOUCH_DOWN: i32 = 1;
const TOUCH_UP: i32 = 3;

/// Where a row sits in its card, for the corner radii and the fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct CardPos {
    first: bool,
    last: bool,
}

/// Full props (realize) for the native row.
#[derive(Clone, Debug, PartialEq)]
struct RowProps {
    /// Pairs the realized node with its `RowLayout` (see [`ROWS`]).
    key: u64,
    title: String,
    caption: Option<String>,
    /// A read-only row's value, trailing.
    value: Option<String>,
    /// Title in the accent color (a link row).
    accent: bool,
    /// The title is the row's caption alone, in the footnote size and the secondary color: the
    /// trailing control (a button, a link) already shows the real title.
    secondary: bool,
    /// A trailing chevron (a row that pushes a page).
    chevron: bool,
    /// Whole-row tap: pressed highlight and `Event::Pressed`.
    activatable: bool,
    /// A `ListItem` painted as a card segment (inside a list page), else a plain Stack.
    item: Option<CardPos>,
    enabled: bool,
    dark: bool,
}

/// Sparse command sent to the native row after creation.
#[derive(Clone, Debug, PartialEq)]
enum RowPatch {
    Title(String),
    Caption(String),
    Value(String),
    Enabled(bool),
    Dark(bool),
}

/// The native nodes of one row, by the row's key: `make` registers them, `RowLayout`
/// measures and places them, `release` disposes them (the toolkit disposes only the row node
/// Day holds).
struct Native {
    root: Handle,
    title: Handle,
    caption: Option<Handle>,
    value: Option<Handle>,
    chevron: Option<Handle>,
    accent: bool,
    secondary: bool,
    item: Option<CardPos>,
}

thread_local! {
    static ROWS: RefCell<Vec<(u64, Native)>> = const { RefCell::new(Vec::new()) };
    static GROUPS: RefCell<Vec<(u64, GroupNative)>> = const { RefCell::new(Vec::new()) };
    static NEXT_KEY: Cell<u64> = const { Cell::new(0) };
    /// Set while a list page builds its rows: the rows realize as `ListItem`s.
    static IN_LIST: Cell<bool> = const { Cell::new(false) };
}

fn next_key() -> u64 {
    NEXT_KEY.with(|k| {
        let v = k.get() + 1;
        k.set(v);
        v
    })
}

fn with_row<R>(key: u64, f: impl FnOnce(&Native) -> R) -> Option<R> {
    ROWS.with(|r| {
        r.borrow()
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, n)| f(n))
    })
}

fn with_row_of<R>(h: Handle, f: impl FnOnce(&Native) -> R) -> Option<R> {
    ROWS.with(|r| {
        r.borrow()
            .iter()
            .find(|(_, n)| n.root == h)
            .map(|(_, n)| f(n))
    })
}

fn theme(dark: bool, light: u32, dark_argb: u32) -> u32 {
    if dark { dark_argb } else { light }
}

fn dark_now() -> bool {
    day_reactive::untrack(day_core::dark_mode)
}

/// The fill a row rests on: the card color inside a list page, nothing elsewhere (the shared
/// section card paints behind it).
fn base_fill(n: &Native, dark: bool) -> u32 {
    match n.item {
        Some(_) => theme(dark, CARD_LIGHT, CARD_DARK),
        None => CLEAR,
    }
}

/// Color every text node and the fill for the mode (ArkUI's C nodes don't re-theme an explicit
/// color, so a mode switch repaints through `RowPatch::Dark`).
fn paint(n: &Native, dark: bool) {
    let secondary = theme(dark, SECONDARY_LIGHT, SECONDARY_DARK);
    let primary = if n.accent {
        theme(dark, ACCENT_LIGHT, ACCENT_DARK)
    } else if n.secondary {
        secondary
    } else {
        theme(dark, TEXT_LIGHT, TEXT_DARK)
    };
    node::set_font_color(n.title, primary);
    for t in [n.caption, n.value, n.chevron].into_iter().flatten() {
        node::set_font_color(t, secondary);
    }
    node::set_bg_color(n.root, base_fill(n, dark));
}

/// A disabled row dims its text the way its grayed control does.
fn set_enabled(n: &Native, on: bool) {
    let opacity = if on { 1.0 } else { 0.4 };
    for t in [Some(n.title), n.caption, n.value].into_iter().flatten() {
        node::set_opacity(t, opacity);
    }
}

fn text_node(parent: Handle, s: &str, font: Font) -> Handle {
    let t = node::create(node::TEXT);
    node::set_text(t, s);
    node::set_font_size(t, font_vp(FontSpec::from(font)));
    node::add_child(parent, t);
    t
}

/// A title wraps to two lines at most, then ellipsizes, like a list item's.
fn two_lines(t: Handle) {
    node::set_i32(t, Attr::NODE_TEXT_MAX_LINES, 2);
    node::set_i32(
        t,
        Attr::NODE_TEXT_OVERFLOW,
        ArkUI_TextOverflow::ARKUI_TEXT_OVERFLOW_ELLIPSIS.0 as i32,
    );
}

/// The card segment's corners: rounded where the row is the card's first or last item.
fn card_corners(n: Handle, pos: CardPos) {
    let top = (if pos.first { CARD_RADIUS } else { 0.0 }) as f32;
    let bottom = (if pos.last { CARD_RADIUS } else { 0.0 }) as f32;
    node::set_values(
        n,
        Attr::NODE_BORDER_RADIUS,
        &[
            node::f32v(top),
            node::f32v(top),
            node::f32v(bottom),
            node::f32v(bottom),
        ],
    );
}

/// Back into the List's flow (see the module comment): the engine framed this node a moment
/// ago with a position, which an ArkUI List does not count; its size stays.
fn unposition(h: Handle) {
    node::reset(h, Attr::NODE_POSITION);
}

/// The pressed highlight of a tappable row: the fill goes on at touch-down and back to the
/// row's own fill at the finger's lift or cancel. Additive per-node receiver (the toolkit's
/// global one is untouched); the click itself reaches Day through the global receiver as
/// `Event::Pressed`.
unsafe extern "C" fn on_touch(ev: *mut ArkUI_NodeEvent) {
    day_spec::ffi_guard::contain((), || {
        // SAFETY: a live node event for the callback's duration; the accessors are ArkUI's.
        let (kind, n, action) = unsafe {
            let kind = OH_ArkUI_NodeEvent_GetEventType(ev);
            let n = OH_ArkUI_NodeEvent_GetNodeHandle(ev);
            let input = OH_ArkUI_NodeEvent_GetInputEvent(ev);
            let action = if input.is_null() {
                -1
            } else {
                OH_ArkUI_UIInputEvent_GetAction(input)
            };
            (kind, n, action)
        };
        if kind != Ev::NODE_TOUCH_EVENT || n.is_null() {
            return;
        }
        match action {
            TOUCH_DOWN => node::set_bg_color(n, PRESSED),
            TOUCH_UP | TOUCH_CANCEL => {
                let dark = dark_now();
                let fill = with_row_of(n, |r| base_fill(r, dark)).unwrap_or(CLEAR);
                node::set_bg_color(n, fill);
            }
            _ => {}
        }
    });
}

fn make(_backend: &mut ArkUi, p: &RowProps, id: NodeId) -> AHandle {
    let root = node::create(if p.item.is_some() {
        node::LIST_ITEM
    } else {
        node::STACK
    });
    // Null = the node API is unavailable; day falls back to a placeholder per docs.
    if root.is_null() {
        return AHandle(root);
    }
    let title_font = if p.secondary {
        Font::Footnote
    } else {
        Font::Body
    };
    let title = text_node(root, &p.title, title_font);
    two_lines(title);
    let caption = p.caption.as_deref().map(|c| {
        let t = text_node(root, c, Font::Footnote);
        two_lines(t);
        t
    });
    let value = p.value.as_deref().map(|v| {
        let t = text_node(root, v, Font::Body);
        node::label_single_line(t);
        t
    });
    let chevron = p.chevron.then(|| text_node(root, "›", Font::Title3));
    node::set_a11y_text(root, &p.title);
    if p.activatable {
        node::register_event(root, node::EV_CLICK, id.0);
        node::register_event(root, node::EV_TOUCH, id.0);
        node::add_event_receiver(root, on_touch);
    }
    if let Some(pos) = p.item {
        card_corners(root, pos);
        node::set_clip(root, true);
    }
    let n = Native {
        root,
        title,
        caption,
        value,
        chevron,
        accent: p.accent,
        secondary: p.secondary,
        item: p.item,
    };
    paint(&n, p.dark);
    set_enabled(&n, p.enabled);
    ROWS.with(|r| r.borrow_mut().push((p.key, n)));
    AHandle(root)
}

fn update(_backend: &mut ArkUi, h: &AHandle, patch: &RowPatch) {
    with_row_of(h.0, |n| match patch {
        RowPatch::Title(s) => {
            node::set_text(n.title, s);
            node::set_a11y_text(n.root, s);
        }
        RowPatch::Caption(s) => {
            if let Some(c) = n.caption {
                node::set_text(c, s);
            }
        }
        RowPatch::Value(s) => {
            if let Some(v) = n.value {
                node::set_text(v, s);
            }
        }
        RowPatch::Enabled(on) => set_enabled(n, *on),
        RowPatch::Dark(dark) => paint(n, *dark),
    });
}

/// The row's own text nodes are this module's, which Day never releases: dispose them with the
/// row (the toolkit disposes the item itself right after).
fn release(_backend: &mut ArkUi, h: &AHandle) {
    let taken = ROWS.with(|r| {
        let mut r = r.borrow_mut();
        let at = r.iter().position(|(_, n)| n.root == h.0)?;
        Some(r.remove(at).1)
    });
    if let Some(n) = taken {
        for t in [Some(n.title), n.caption, n.value, n.chevron]
            .into_iter()
            .flatten()
        {
            node::remove_child(n.root, t);
            node::dispose_tree(t);
        }
    }
}

day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
    kind: KIND, props: RowProps, patch: RowPatch,
    make: make, update: update, measure: day_pieces::fill_measure, release: release);

// ---------------------------------------------------------------------------
// Layout: a leading glyph (a root-list row), the text column, then the control, the value and
// the chevron trailing, everything centered on the row's height. Native parts are framed
// directly (mirrored by hand under RTL, as `place_child` does for the Day children).
// ---------------------------------------------------------------------------

struct RowLayout {
    key: u64,
    /// The first Day child is a leading glyph, the control (if any) the second.
    leading: bool,
    /// Inside a card: the row pads its own edges.
    item: bool,
}

/// One pass of measurement: every part's size, and the row's.
struct Arranged {
    width: f64,
    height: f64,
    glyph: Option<(RNode, Size)>,
    title: Size,
    caption: Option<Size>,
    value: Option<Size>,
    chevron: Option<Size>,
    control: Option<(RNode, Size)>,
}

/// A text node's natural size within `max_w` (0 = unconstrained).
fn text_size(t: Handle, max_w: f64) -> Size {
    let (w, h) = node::measure_label(t, max_w, 0.0);
    Size::new(w, h)
}

impl RowLayout {
    fn pad_h(&self) -> f64 {
        if self.item { CARD_PAD_H } else { 0.0 }
    }

    fn arrange(
        &self,
        cx: &mut dyn LayoutOps,
        children: &[RNode],
        width: Option<f64>,
    ) -> Option<Arranged> {
        let (title, caption, value, chevron) =
            with_row(self.key, |n| (n.title, n.caption, n.value, n.chevron))?;
        let (glyph, control) = if self.leading {
            (children.first().copied(), children.get(1).copied())
        } else {
            (None, children.first().copied())
        };
        let has_caption = caption.is_some();
        let pad_h = self.pad_h();

        // The leading glyph at its own size, then the trailing fixed parts: the chevron, then
        // the value (capped so a long value leaves the title room; it ellipsizes).
        let glyph = glyph.map(|g| (g, cx.measure_child(g, Proposal::UNCONSTRAINED)));
        let leading = glyph.map_or(0.0, |(_, s)| s.width + GAP);
        let chevron = chevron.map(|c| text_size(c, 0.0));
        let value_cap = width.map_or(0.0, |w| (w * 0.6).max(1.0));
        let value = value.map(|v| text_size(v, value_cap));
        let mut trailing = 0.0;
        for s in [chevron, value].into_iter().flatten() {
            trailing += s.width + GAP;
        }

        // The control and the title column share what is left.
        let title_natural = text_size(title, 0.0);
        let caption_natural = caption.map(|c| text_size(c, 0.0));
        let text_natural = title_natural
            .width
            .max(caption_natural.map_or(0.0, |c| c.width));
        let (text_w, control, width) = match (width, control) {
            (Some(w), Some(c)) => {
                let room = (w - 2.0 * pad_h - leading - trailing).max(0.0);
                if cx.flex_of(c).grow_w {
                    let text_w = text_natural.min(room * TITLE_SHARE);
                    let ctl_w = (room - text_w - GAP).max(0.0);
                    let cs = cx.measure_child(c, Proposal::new(Some(ctl_w), None));
                    (text_w, Some((c, Size::new(ctl_w, cs.height))), w)
                } else {
                    let cs = cx.measure_child(c, Proposal::new(None, None));
                    ((room - cs.width - GAP).max(0.0), Some((c, cs)), w)
                }
            }
            (Some(w), None) => ((w - 2.0 * pad_h - leading - trailing).max(0.0), None, w),
            (None, Some(c)) => {
                let cs = cx.measure_child(c, Proposal::new(None, None));
                let w = 2.0 * pad_h + leading + text_natural + GAP + cs.width + trailing;
                (text_natural, Some((c, cs)), w)
            }
            (None, None) => (
                text_natural,
                None,
                2.0 * pad_h + leading + text_natural + trailing,
            ),
        };
        let title = text_size(title, text_w.max(1.0));
        let caption = caption.map(|c| text_size(c, text_w.max(1.0)));
        let text_h = title.height + caption.map_or(0.0, |c| c.height + LINE_GAP);
        let min = if has_caption {
            ROW_MIN_TWO_LINE
        } else {
            ROW_MIN
        };
        let height = min
            .max(text_h + 2.0 * PAD_V)
            .max(control.map_or(0.0, |(_, s)| s.height + 2.0 * PAD_V))
            .max(glyph.map_or(0.0, |(_, s)| s.height + 2.0 * PAD_V));
        Some(Arranged {
            width,
            height,
            glyph,
            title,
            caption,
            value,
            chevron,
            control,
        })
    }
}

/// Frame a native part at `x` (leading-edge coordinates), mirrored under RTL.
fn place_native(h: Handle, x: f64, y: f64, s: Size, row_w: f64) {
    let x = if day_core::is_rtl() {
        row_w - x - s.width
    } else {
        x
    };
    node::set_frame(h, x, y, s.width, s.height);
}

impl Layout for RowLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        match self.arrange(cx, children, p.width) {
            Some(a) => Size::new(a.width, a.height),
            None => Size::new(p.width.unwrap_or(0.0), ROW_MIN),
        }
    }

    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        let w = bounds.size.width;
        let Some(a) = self.arrange(cx, children, Some(w)) else {
            return;
        };
        let Some((root, title, caption, value, chevron)) = with_row(self.key, |n| {
            (n.root, n.title, n.caption, n.value, n.chevron)
        }) else {
            return;
        };
        if self.item {
            unposition(root);
        }
        let h = bounds.size.height;
        let center = |s: Size| ((h - s.height) / 2.0).max(0.0);
        let pad_h = self.pad_h();

        // The leading glyph, then the text column, top-aligned within its centered block.
        let mut x = pad_h;
        if let Some((g, gs)) = a.glyph {
            cx.place_child(g, Rect::new(x, center(gs), gs.width, gs.height));
            x += gs.width + GAP;
        }
        let text_h = a.title.height + a.caption.map_or(0.0, |c| c.height + LINE_GAP);
        let top = ((h - text_h) / 2.0).max(0.0);
        place_native(title, x, top, a.title, w);
        if let (Some(c), Some(cs)) = (caption, a.caption) {
            place_native(c, x, top + a.title.height + LINE_GAP, cs, w);
        }

        // Trailing parts, from the trailing edge inward.
        let mut cursor = w - pad_h;
        if let (Some(c), Some(cs)) = (chevron, a.chevron) {
            cursor -= cs.width;
            place_native(c, cursor, center(cs), cs, w);
            cursor -= GAP;
        }
        if let (Some(v), Some(vs)) = (value, a.value) {
            cursor -= vs.width;
            place_native(v, cursor, center(vs), vs, w);
            cursor -= GAP;
        }
        if let Some((c, cs)) = a.control {
            cursor -= cs.width;
            cx.place_child(
                c,
                Rect::new(cursor.max(0.0), center(cs), cs.width, cs.height),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The row piece: one native row around the composed row's control.
// ---------------------------------------------------------------------------

struct NativeRow {
    title: TextSource,
    caption: Option<TextSource>,
    value: Option<TextSource>,
    accent: bool,
    secondary: bool,
    chevron: bool,
    enabled: Option<Reactive<bool>>,
    /// A leading glyph (a root-list row's icon), built before the control.
    leading: Option<AnyPiece>,
    control: Option<AnyPiece>,
    /// Whole-row tap handler (a navigate, link or copyable info row).
    activate: Option<Rc<dyn Fn()>>,
    /// Where the row sits in its card, when it is a list item.
    card: Option<CardPos>,
}

fn native_row(r: &Row) -> NativeRow {
    let mut row = plain_row(Resolved(r.title.clone()));
    row.caption = r.caption.clone();
    row.enabled = r.enabled.clone();
    row
}

fn plain_row<M>(title: impl IntoText<M>) -> NativeRow {
    NativeRow {
        title: title.into_text(),
        caption: None,
        value: None,
        accent: false,
        secondary: false,
        chevron: false,
        enabled: None,
        leading: None,
        control: None,
        activate: None,
        card: None,
    }
}

impl NativeRow {
    /// The caption alone as the row's text, in the secondary style: the trailing control (a
    /// button, a link) carries the title, which the row never shows a second time.
    fn caption_only(mut self, c: &TextSource) -> Self {
        self.title = c.clone();
        self.caption = None;
        self.secondary = true;
        self
    }
}

fn patch(node: RNode, p: RowPatch, affects_size: bool) {
    with_tree(|t| t.patch(node, Box::new(p), affects_size));
}

/// Keep a localized (or otherwise live) text on the native node.
fn bind_text(node: RNode, source: &TextSource, make: fn(String) -> RowPatch) {
    if let TextSource::Dyn(f) = source {
        let f = f.clone();
        watch(move || f(), move |s, _| patch(node, make(s.clone()), true));
    }
}

impl Piece for NativeRow {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let key = next_key();
        // Inside a list page the row is a card segment; `card` says where in the card.
        let item = if IN_LIST.with(|l| l.get()) {
            Some(self.card.unwrap_or_default())
        } else {
            None
        };
        let props = RowProps {
            key,
            title: self.title.initial(),
            caption: self.caption.as_ref().map(TextSource::initial),
            value: self.value.as_ref().map(TextSource::initial),
            accent: self.accent,
            secondary: self.secondary,
            chevron: self.chevron,
            activatable: self.activate.is_some(),
            item,
            enabled: self.enabled.as_ref().is_none_or(|e| e.get_untracked()),
            dark: dark_now(),
        };
        let node = cx.native(
            KIND,
            &props,
            Rc::new(RowLayout {
                key,
                leading: self.leading.is_some(),
                item: item.is_some(),
            }),
            Flex {
                grow_w: true,
                ..Default::default()
            },
            Boundary::No,
        );
        if let Some(leading) = self.leading {
            cx.under(node, |cx| {
                let _ = leading.build(cx);
            });
        }
        if let Some(control) = self.control {
            cx.under(node, |cx| {
                let _ = control.build(cx);
            });
        }
        if let Some(run) = self.activate {
            cx.on(node, move |ev| {
                if matches!(ev, Event::Pressed) {
                    run();
                }
            });
        }
        bind_text(node, &self.title, RowPatch::Title);
        if let Some(c) = &self.caption {
            bind_text(node, c, RowPatch::Caption);
        }
        if let Some(v) = &self.value {
            bind_text(node, v, RowPatch::Value);
        }
        if let Some(e) = self.enabled {
            watch(
                move || e.get(),
                move |on, _| patch(node, RowPatch::Enabled(*on), false),
            );
        }
        watch(day_core::dark_mode, move |dark, _| {
            patch(node, RowPatch::Dark(*dark), false)
        });
        node
    }
}

/// The HarmonyOS row for `r`, or `None` for the rows that keep the composed shape.
pub(super) fn row_piece(inst: Rc<Installed>, r: &Row, _desktop: bool) -> Option<AnyPiece> {
    row_at(inst, r, CardPos::default())
}

/// [`row_piece`] for a row at a known place in its card.
fn row_at(inst: Rc<Installed>, r: &Row, card: CardPos) -> Option<AnyPiece> {
    let id = r.id.clone();
    let enabled = r.enabled.clone();
    let with_id = |p: AnyPiece| match &id {
        Some(id) => p.id(id.clone()).any(),
        None => p,
    };
    let enabled_fn = move || enabled.as_ref().is_none_or(|e| e.get());
    let mut row = native_row(r);
    row.card = Some(card);
    match &r.kind {
        RowKind::Toggle(sig) => {
            row.control = Some(with_id(toggle(*sig).enabled(enabled_fn).any()));
        }
        RowKind::Choice {
            labels,
            index,
            style,
        } => {
            let p = choice_picker(labels, *index, *style == ChoiceStyle::Segmented, enabled_fn);
            row.control = Some(with_id(p.any()));
        }
        RowKind::Slider {
            value,
            range,
            step,
            value_text,
        } => {
            let mut s = slider(*value).range(range.clone()).enabled(enabled_fn);
            if let Some(st) = step {
                s = s.step(*st);
            }
            let ctl = match &id {
                Some(i) => s.id(i.clone()).grow_w().min_width(140.0).any(),
                None => s.grow_w().min_width(140.0).any(),
            };
            row.control = Some(match value_text {
                Some(f) => {
                    let f = f.clone();
                    let v = *value;
                    day_pieces::prelude::row((ctl, label(move || f(v.get())).tabular().secondary()))
                        .spacing(8.0)
                        .align(VAlign::Center)
                        .grow_w()
                        .any()
                }
                None => ctl,
            });
        }
        RowKind::Stepper { value, range, step } => {
            let v = *value;
            let (dec, inc) = stepper_buttons(v, range, *step, enabled_fn);
            row.control = Some(stepper_control(v, dec, inc, id.as_ref()));
        }
        RowKind::Text {
            value,
            placeholder,
            secure,
        } => {
            let mut f = if *secure {
                secure_field(*value)
            } else {
                text_field(*value)
            };
            if let Some(ph) = placeholder {
                f = f.placeholder(text_source_fn(ph));
            }
            row.control = Some(match &id {
                Some(i) => f
                    .enabled(enabled_fn)
                    .id(i.clone())
                    .grow_w()
                    .min_width(160.0)
                    .any(),
                None => f.enabled(enabled_fn).grow_w().min_width(160.0).any(),
            });
        }
        RowKind::Action {
            on,
            destructive,
            confirm,
            confirm_label,
            busy,
        } => {
            let on = on.clone();
            let title = text_source_fn(&r.title);
            let confirm = confirm.clone();
            let confirm_label = confirm_label.clone();
            let busy = busy.clone();
            let busy_gate = busy.clone();
            let run = move || {
                let on = on.clone();
                match &confirm {
                    Some(message) => {
                        let message = message.initial();
                        let title = title();
                        let label = confirm_label.as_ref().map(TextSource::initial);
                        day_core::task(async move {
                            if crate::confirm_dialog(&title, &message, label).await {
                                on();
                            }
                        });
                    }
                    None => on(),
                }
            };
            let mut b = button(text_source_fn(&r.title))
                .action(run)
                .enabled(move || enabled_fn() && !busy_gate.as_ref().is_some_and(|b| b.get()));
            if *destructive {
                b = b.tint(day_spec::Color::rgb(0.89, 0.18, 0.16));
            }
            let ctl = with_id(b.any());
            let ctl = match busy {
                Some(b) => day_pieces::prelude::row((when(move || b.get(), spinner), ctl))
                    .spacing(8.0)
                    .align(VAlign::Center)
                    .any(),
                None => ctl,
            };
            match &r.caption {
                Some(caption) => {
                    row = row.caption_only(caption);
                    row.control = Some(ctl);
                }
                // A lone button (no caption) is already the platform's shape: composed, and
                // hosted as an item of the card inside a list page.
                None => return Some(host_if_listed(ctl, card)),
            }
        }
        RowKind::Link(url) => {
            let url = url.clone();
            match &r.caption {
                // The Day link carries the title; the row shows the caption beside it.
                Some(c) => {
                    row = row.caption_only(c);
                    row.control = Some(with_id(link(text_source_fn(&r.title), url).any()));
                }
                None => {
                    row.accent = true;
                    row.activate = Some(Rc::new(move || day_core::open_url(&url)));
                    return Some(with_id(row.any()));
                }
            }
        }
        RowKind::Navigate(build) => {
            let build = build.clone();
            let title = text_source_fn(&r.title);
            row.chevron = true;
            row.activate = Some(Rc::new(move || {
                push_subpage(inst.clone(), title(), build.clone());
            }));
            return Some(with_id(row.any()));
        }
        RowKind::Info { value, copyable } => {
            row.value = Some(value.clone());
            if *copyable {
                let text = value.clone();
                row.activate = Some(Rc::new(move || {
                    day_part_clipboard::set_text(&text.initial());
                }));
            }
            return Some(with_id(row.any()));
        }
        // A footnote: the secondary text alone, as a row of the card.
        RowKind::Note => {
            row = row.caption_only(&r.title);
            return Some(with_id(row.any()));
        }
        // The app's own piece: composed, hosted as an item inside a list page.
        RowKind::Custom(build) => return Some(host_if_listed(with_id(build()), card)),
    }
    Some(row.any())
}

/// Inside a list page, `p` rides in a `ListItem` host of its own; elsewhere it is itself.
fn host_if_listed(p: AnyPiece, card: CardPos) -> AnyPiece {
    if IN_LIST.with(|l| l.get()) {
        ItemHost { content: p, card }.any()
    } else {
        p
    }
}

// ---------------------------------------------------------------------------
// The item host: a `ListItem` around a composed piece (a custom row, a group's custom page).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct HostProps {
    /// Pairs the realized node with its `HostLayout` (see [`HOSTS`]).
    key: u64,
    card: CardPos,
    dark: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum HostPatch {
    Dark(bool),
}

fn host_make(_backend: &mut ArkUi, p: &HostProps, _id: NodeId) -> AHandle {
    let n = node::create(node::LIST_ITEM);
    if n.is_null() {
        return AHandle(n);
    }
    card_corners(n, p.card);
    node::set_clip(n, true);
    node::set_bg_color(n, theme(p.dark, CARD_LIGHT, CARD_DARK));
    HOSTS.with(|m| m.borrow_mut().push((p.key, n)));
    AHandle(n)
}

fn host_release(_backend: &mut ArkUi, h: &AHandle) {
    HOSTS.with(|m| m.borrow_mut().retain(|(_, n)| *n != h.0));
}

fn host_update(_backend: &mut ArkUi, h: &AHandle, patch: &HostPatch) {
    match patch {
        HostPatch::Dark(dark) => node::set_bg_color(h.0, theme(*dark, CARD_LIGHT, CARD_DARK)),
    }
}

// `renderer!` registers one entry per module: each extra kind lives in a module of its own.
mod host_renderer {
    use super::*;
    day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
        kind: HOST_KIND, props: HostProps, patch: HostPatch,
        make: host_make, update: host_update, measure: day_pieces::fill_measure,
        release: host_release);
}

/// The content padded to the card's text column, the host sized to it; back into the List's
/// flow after the engine frames it.
struct HostLayout {
    key: u64,
}

thread_local! {
    static HOSTS: RefCell<Vec<(u64, Handle)>> = const { RefCell::new(Vec::new()) };
}

impl Layout for HostLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let inner = p.width.map(|w| (w - 2.0 * CARD_PAD_H).max(0.0));
        let s = match children.first() {
            Some(&c) => cx.measure_child(c, Proposal::new(inner, None)),
            None => Size::ZERO,
        };
        Size::new(
            p.width.unwrap_or(s.width + 2.0 * CARD_PAD_H),
            s.height + 2.0 * PAD_V,
        )
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        if let Some(h) = HOSTS.with(|m| {
            m.borrow()
                .iter()
                .find(|(k, _)| *k == self.key)
                .map(|(_, h)| *h)
        }) {
            unposition(h);
        }
        if let Some(&c) = children.first() {
            let inner = (bounds.size.width - 2.0 * CARD_PAD_H).max(0.0);
            let s = cx.measure_child(c, Proposal::new(Some(inner), None));
            cx.place_child(c, Rect::new(CARD_PAD_H, PAD_V, inner, s.height));
        }
    }
}

struct ItemHost {
    content: AnyPiece,
    card: CardPos,
}

impl Piece for ItemHost {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let key = next_key();
        let node = cx.native(
            HOST_KIND,
            &HostProps {
                key,
                card: self.card,
                dark: dark_now(),
            },
            Rc::new(HostLayout { key }),
            Flex {
                grow_w: true,
                ..Default::default()
            },
            Boundary::No,
        );
        let content = self.content;
        cx.under(node, |cx| {
            let _ = content.build(cx);
        });
        watch(day_core::dark_mode, move |dark, _| {
            with_tree(|t| t.patch(node, Box::new(HostPatch::Dark(*dark)), false))
        });
        node
    }
}

// ---------------------------------------------------------------------------
// The section group: a `ListItemGroup` with its header and footer texts, its rows as items.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct GroupProps {
    key: u64,
    title: Option<String>,
    footer: Option<String>,
    dark: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum GroupPatch {
    Title(String),
    Footer(String),
    Dark(bool),
}

/// A group's own nodes: the header and footer slots, each a sized Stack holding one Text.
struct GroupNative {
    root: Handle,
    header: Option<(Handle, Handle)>,
    footer: Option<(Handle, Handle)>,
}

fn with_group_of<R>(h: Handle, f: impl FnOnce(&GroupNative) -> R) -> Option<R> {
    GROUPS.with(|g| {
        g.borrow()
            .iter()
            .find(|(_, n)| n.root == h)
            .map(|(_, n)| f(n))
    })
}

fn with_group<R>(key: u64, f: impl FnOnce(&GroupNative) -> R) -> Option<R> {
    GROUPS.with(|g| {
        g.borrow()
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, n)| f(n))
    })
}

/// A header or footer slot: a Stack the layout sizes, the text inside it at the card's text
/// column.
fn slot(root: Handle, text: &str, attr: Attr) -> (Handle, Handle) {
    let stack = node::create(node::STACK);
    let t = text_node(stack, text, Font::Footnote);
    node::set_object(root, attr, stack.cast());
    (stack, t)
}

fn group_paint(n: &GroupNative, dark: bool) {
    let secondary = theme(dark, SECONDARY_LIGHT, SECONDARY_DARK);
    for (_, t) in [n.header, n.footer].into_iter().flatten() {
        node::set_font_color(t, secondary);
    }
}

fn group_make(_backend: &mut ArkUi, p: &GroupProps, _id: NodeId) -> AHandle {
    let root = node::create(ArkUI_NodeType::ARKUI_NODE_LIST_ITEM_GROUP);
    if root.is_null() {
        return AHandle(root);
    }
    let header = p
        .title
        .as_deref()
        .map(|t| slot(root, t, Attr::NODE_LIST_ITEM_GROUP_SET_HEADER));
    let footer = p
        .footer
        .as_deref()
        .map(|t| slot(root, t, Attr::NODE_LIST_ITEM_GROUP_SET_FOOTER));
    // The hairline between the card's items, inset to its text column.
    node::set_values(
        root,
        Attr::NODE_LIST_ITEM_GROUP_SET_DIVIDER,
        &[
            node::u32v(DIVIDER),
            node::f32v(0.5),
            node::f32v(CARD_PAD_H as f32),
            node::f32v(CARD_PAD_H as f32),
        ],
    );
    let n = GroupNative {
        root,
        header,
        footer,
    };
    group_paint(&n, p.dark);
    GROUPS.with(|g| g.borrow_mut().push((p.key, n)));
    AHandle(root)
}

fn group_update(_backend: &mut ArkUi, h: &AHandle, patch: &GroupPatch) {
    with_group_of(h.0, |n| match patch {
        GroupPatch::Title(s) => {
            if let Some((_, t)) = n.header {
                node::set_text(t, s);
            }
        }
        GroupPatch::Footer(s) => {
            if let Some((_, t)) = n.footer {
                node::set_text(t, s);
            }
        }
        GroupPatch::Dark(dark) => group_paint(n, *dark),
    });
}

fn group_release(_backend: &mut ArkUi, h: &AHandle) {
    let taken = GROUPS.with(|g| {
        let mut g = g.borrow_mut();
        let at = g.iter().position(|(_, n)| n.root == h.0)?;
        Some(g.remove(at).1)
    });
    if let Some(n) = taken {
        if n.header.is_some() {
            node::reset(n.root, Attr::NODE_LIST_ITEM_GROUP_SET_HEADER);
        }
        if n.footer.is_some() {
            node::reset(n.root, Attr::NODE_LIST_ITEM_GROUP_SET_FOOTER);
        }
        for (stack, _) in [n.header, n.footer].into_iter().flatten() {
            node::dispose_tree(stack);
        }
    }
}

mod group_renderer {
    use super::*;
    day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
        kind: GROUP_KIND, props: GroupProps, patch: GroupPatch,
        make: group_make, update: group_update, measure: day_pieces::fill_measure,
        release: group_release);
}

/// The header text, the rows stacked, the footer text: the group is as tall as all of them,
/// and the List lays its parts out in that same order.
struct GroupLayout {
    key: u64,
}

/// A slot's height for its text wrapped to the card's text column.
fn slot_height(slot: Option<(Handle, Handle)>, width: f64, top: f64, bottom: f64) -> f64 {
    slot.map_or(0.0, |(_, t)| {
        let (_, h) = node::measure_label(t, (width - 2.0 * CARD_PAD_H).max(1.0), 0.0);
        top + h + bottom
    })
}

impl Layout for GroupLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let width = p.width.unwrap_or(0.0);
        let (header, footer) =
            with_group(self.key, |n| (n.header, n.footer)).unwrap_or((None, None));
        let mut h = slot_height(header, width, HEADER_TOP, HEADER_BOTTOM);
        for &c in children {
            h += cx.measure_child(c, Proposal::new(Some(width), None)).height;
        }
        h += slot_height(footer, width, FOOTER_TOP, FOOTER_BOTTOM);
        Size::new(width, h)
    }

    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        let w = bounds.size.width;
        let Some((root, header, footer)) = with_group(self.key, |n| (n.root, n.header, n.footer))
        else {
            return;
        };
        unposition(root);
        let text_w = (w - 2.0 * CARD_PAD_H).max(1.0);
        let mut y = 0.0;
        if let Some((stack, t)) = header {
            let (_, th) = node::measure_label(t, text_w, 0.0);
            let h = HEADER_TOP + th + HEADER_BOTTOM;
            node::set_size(stack, w, h);
            place_native(t, CARD_PAD_H, HEADER_TOP, Size::new(text_w, th), w);
            y += h;
        }
        for &c in children {
            let s = cx.measure_child(c, Proposal::new(Some(w), None));
            cx.place_child(c, Rect::new(0.0, y, w, s.height));
            y += s.height;
        }
        if let Some((stack, t)) = footer {
            let (_, th) = node::measure_label(t, text_w, 0.0);
            let h = FOOTER_TOP + th + FOOTER_BOTTOM;
            node::set_size(stack, w, h);
            place_native(t, CARD_PAD_H, FOOTER_TOP, Size::new(text_w, th), w);
        }
    }
}

/// One section as a card: the group node with its rows built inside it.
struct SectionGroup {
    title: Option<TextSource>,
    footer: Option<TextSource>,
    rows: Vec<AnyPiece>,
}

impl Piece for SectionGroup {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let key = next_key();
        let props = GroupProps {
            key,
            title: self.title.as_ref().map(TextSource::initial),
            footer: self.footer.as_ref().map(TextSource::initial),
            dark: dark_now(),
        };
        let node = cx.native(
            GROUP_KIND,
            &props,
            Rc::new(GroupLayout { key }),
            Flex {
                grow_w: true,
                ..Default::default()
            },
            Boundary::No,
        );
        cx.under(node, |cx| {
            for r in self.rows {
                let _ = r.build(cx);
            }
        });
        let bind = |node: RNode, src: &Option<TextSource>, make: fn(String) -> GroupPatch| {
            if let Some(TextSource::Dyn(f)) = src {
                let f = f.clone();
                watch(
                    move || f(),
                    move |s, _| with_tree(|t| t.patch(node, Box::new(make(s.clone())), true)),
                );
            }
        };
        bind(node, &self.title, GroupPatch::Title);
        bind(node, &self.footer, GroupPatch::Footer);
        watch(day_core::dark_mode, move |dark, _| {
            with_tree(|t| t.patch(node, Box::new(GroupPatch::Dark(*dark)), false))
        });
        node
    }
}

// ---------------------------------------------------------------------------
// The page: a `List` of the groups.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct PageProps {
    /// Sized to its groups inside a Day `scroll`, which does the scrolling (`group_body`).
    hug: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum PagePatch {}

fn page_make(_backend: &mut ArkUi, p: &PageProps, _id: NodeId) -> AHandle {
    let n = node::create(node::LIST);
    if n.is_null() {
        return AHandle(n);
    }
    // A list as tall as its groups has nothing to scroll; leave the drag to the scroll around it.
    if p.hug {
        node::set_i32(n, Attr::NODE_SCROLL_ENABLE_SCROLL_INTERACTION, 0);
    }
    node::set_f32(n, Attr::NODE_LIST_SPACE, CARD_SPACE as f32);
    node::set_values(
        n,
        Attr::NODE_PADDING,
        &[
            node::f32v(PAGE_TOP as f32),
            node::f32v(LIST_INSET as f32),
            node::f32v(PAGE_BOTTOM as f32),
            node::f32v(LIST_INSET as f32),
        ],
    );
    AHandle(n)
}

fn page_update(_backend: &mut ArkUi, _h: &AHandle, patch: &PagePatch) {
    match *patch {}
}

mod page_renderer {
    use super::*;
    day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
        kind: PAGE_KIND, props: PageProps, patch: PagePatch,
        make: page_make, update: page_update, measure: day_pieces::fill_measure);
}

/// The groups stacked inside the list's padding, a card's gap apart: the same order and
/// heights the List lays them out in once they are back in its flow. With no height offered
/// (inside a Day `scroll`), the page is as tall as its groups.
struct PageLayout;

impl Layout for PageLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let width = p.width.unwrap_or(0.0);
        if let Some(height) = p.height {
            return Size::new(width, height);
        }
        let w = (width - 2.0 * LIST_INSET).max(0.0);
        let groups: f64 = children
            .iter()
            .map(|&c| cx.measure_child(c, Proposal::new(Some(w), None)).height)
            .sum();
        let gaps = CARD_SPACE * children.len().saturating_sub(1) as f64;
        Size::new(width, PAGE_TOP + groups + gaps + PAGE_BOTTOM)
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        let w = (bounds.size.width - 2.0 * LIST_INSET).max(0.0);
        let mut y = PAGE_TOP;
        for &c in children {
            let s = cx.measure_child(c, Proposal::new(Some(w), None));
            cx.place_child(c, Rect::new(LIST_INSET, y, w, s.height));
            y += s.height + CARD_SPACE;
        }
    }
}

struct ListPage {
    groups: Vec<AnyPiece>,
    hug: bool,
}

impl Piece for ListPage {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let node = cx.native(
            PAGE_KIND,
            &PageProps { hug: self.hug },
            Rc::new(PageLayout),
            Flex {
                grow_w: true,
                grow_h: !self.hug,
                ..Default::default()
            },
            // A hugging page's height follows its rows, and the scroll around it must learn of
            // a change; a filling page is a boundary.
            if self.hug {
                Boundary::No
            } else {
                Boundary::Yes
            },
        );
        cx.under(node, |cx| {
            for g in self.groups {
                let _ = g.build(cx);
            }
        });
        node
    }
}

/// Build `f`'s rows as list items: every `NativeRow` built inside realizes as a `ListItem`.
fn listed<R>(f: impl FnOnce() -> R) -> R {
    let was = IN_LIST.with(|l| l.replace(true));
    let out = f();
    IN_LIST.with(|l| l.set(was));
    out
}

/// A section's card: its available rows as items, placed first-to-last in the card.
fn section_group(inst: &Rc<Installed>, s: &Section) -> Option<AnyPiece> {
    let rows: Vec<&Row> = s.rows.iter().filter(|r| r.is_available()).collect();
    if rows.is_empty() {
        return None;
    }
    let last = rows.len() - 1;
    let pieces: Vec<AnyPiece> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let card = CardPos {
                first: i == 0,
                last: i == last,
            };
            row_at(inst.clone(), r, card).unwrap_or_else(|| {
                host_if_listed(crate::composed_row(inst.clone(), r, false), card)
            })
        })
        .collect();
    Some(
        SectionGroup {
            title: s.title.clone(),
            footer: s.footer.clone(),
            rows: pieces,
        }
        .any(),
    )
}

/// A group's cards: every section, then its custom piece as a card of its own.
fn group_cards(inst: &Rc<Installed>, g: &Group, out: &mut Vec<AnyPiece>) {
    for s in &g.sections {
        if let Some(p) = section_group(inst, s) {
            out.push(p);
        }
    }
    if let Some(custom) = &g.custom {
        let whole = CardPos {
            first: true,
            last: true,
        };
        out.push(
            SectionGroup {
                title: None,
                footer: None,
                rows: vec![
                    ItemHost {
                        content: custom(),
                        card: whole,
                    }
                    .any(),
                ],
            }
            .any(),
        );
    }
}

/// A root-list row: the entry's title and subtitle, its icon leading, a chevron, the tap that
/// pushes its page.
fn entry_row(e: &RootEntry, card: CardPos, about: bool) -> AnyPiece {
    let mut row = plain_row(Resolved(e.title.clone()));
    row.caption = e.subtitle.clone();
    row.chevron = true;
    row.card = Some(card);
    let open = e.open.clone();
    row.activate = Some(Rc::new(move || open()));
    row.leading = e.icon.as_ref().map(|icon| match icon {
        // The app icon as authored, a group's vector in the label color; a standard glyph
        // draws through the one piece that renders a `Symbol` on every toolkit.
        GroupIcon::Image(name) if about => image(name.clone())
            .frame(ABOUT_ICON, ABOUT_ICON)
            .corner_radius(ABOUT_ICON / 5.0)
            .any(),
        GroupIcon::Image(name) => image(name.clone()).template().frame(ICON, ICON).any(),
        GroupIcon::Symbol(sym) => {
            let open = e.open.clone();
            button("")
                .icon(*sym)
                .icon_only()
                .plain()
                .action(move || open())
                .frame(ICON, ICON)
                .any()
        }
    });
    let id = if about {
        day_core::id!(settings_about)
    } else {
        day_core::id!(prefs_group, e.id.clone())
    };
    row.id(id).any()
}

/// The settings root: the About header as the first card, then the groups as one card of
/// chevron rows (one group: its sections as cards under the header).
pub(super) fn root_page(inst: Rc<Installed>) -> Option<AnyPiece> {
    let (about, groups) = root_entries(&inst);
    let whole = CardPos {
        first: true,
        last: true,
    };
    let cards = listed(|| {
        let mut cards: Vec<AnyPiece> = Vec::new();
        if let Some(a) = &about {
            cards.push(
                SectionGroup {
                    title: None,
                    footer: None,
                    rows: vec![entry_row(a, whole, true)],
                }
                .any(),
            );
        }
        let visible = inst.visible_groups();
        if let [g] = visible[..] {
            group_cards(&inst, g, &mut cards);
        } else if !groups.is_empty() {
            let last = groups.len() - 1;
            let rows = groups
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    entry_row(
                        e,
                        CardPos {
                            first: i == 0,
                            last: i == last,
                        },
                        false,
                    )
                })
                .collect();
            cards.push(
                SectionGroup {
                    title: None,
                    footer: None,
                    rows,
                }
                .any(),
            );
        }
        cards
    });
    Some(
        ListPage {
            groups: cards,
            hug: false,
        }
        .id(day_core::id!(settings_root))
        .any(),
    )
}

/// One group's page: its sections as cards.
pub(super) fn group_page(inst: Rc<Installed>, g: &Group) -> Option<AnyPiece> {
    // Let Day own the group's scroll viewport, as on Android. The native cards retain
    // their appearance while scroll_to can reveal custom controls through the tree.
    Some(scroll(group_list(&inst, g, true)).any())
}

/// One group's cards in a list as tall as they are, for a sheet's own `scroll`.
pub(super) fn group_body(inst: Rc<Installed>, g: &Group) -> Option<AnyPiece> {
    Some(group_list(&inst, g, true))
}

fn group_list(inst: &Rc<Installed>, g: &Group, hug: bool) -> AnyPiece {
    let cards = listed(|| {
        let mut cards: Vec<AnyPiece> = Vec::new();
        group_cards(inst, g, &mut cards);
        cards
    });
    ListPage { groups: cards, hug }
        .id(day_core::id!(prefs_group, g.id.clone()))
        .any()
}
