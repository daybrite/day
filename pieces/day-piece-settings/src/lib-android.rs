// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: the Material 3 preference screen (docs/settings.md). Every line of a settings page is
// the Java `DaySettingsRow` (src/DaySettingsRow.java, staged by `[package.metadata.day.android]`):
// a preference row with a headline, supporting text, an optional leading icon and a trailing
// chevron; the category header above a group of rows; the footnote under one; the large About
// header the root list starts with. Every tappable row (a group, About, navigate, link, action,
// a toggle, a copyable value, a choice that opens its spinner) carries the theme's
// `selectableItemBackground` ripple. Each row is a container piece (docs/extending.md §5): the
// Day control the composed row would build (the same `toggle`, `picker`, `slider`, `text_field`,
// carrying the row's element id and `enabled`) is mounted as the row's child and placed by
// `RowLayout` at the trailing edge, or under the texts (a slider always, the SeekBarPreference
// shape). The pages are full-bleed lists of these rows in a Day `scroll`, no cards: the shape
// the platform's own Settings app has.
// ---------------------------------------------------------------------------

use super::*;
use day_android::jni::objects::JString;
use day_android::{AHandle, Android, java};
use day_core::{BuildCx, Flex, Layout, LayoutOps, RNode, with_tree};
use day_spec::{NodeId, Proposal, Rect, Size};

/// The row's piece kind, registered under `day_android::RENDERERS`.
const ROW_KIND: &str = "day-piece-settings.row";
// This piece's Java class (src/DaySettingsRow.java, on the app classpath at build).
day_android::java_class! {
    DaySettingsRow => dev.daybrite.day.piece.settings.DaySettingsRow,
    methods {
        static fn makeRow(title: JString, caption: JString, icon: JString, style: jint,
            flags: jint, enabled: jboolean) -> android.view.View,
        static fn setRowTexts(view: android.view.View, title: JString, caption: JString),
        static fn setRowEnabled(view: android.view.View, on: jboolean),
        static fn rowTextWidth(view: android.view.View) -> jfloat,
        static fn rowTextHeight(view: android.view.View, width: jfloat) -> jfloat,
    },
}

/// M3 list-item metrics, in dp (the Java side holds the same constants for its drawing).
const PAD_H: f64 = 16.0;
const GAP: f64 = 16.0;
const CHEVRON: f64 = 24.0;
const ICON: f64 = 24.0;
const HEADER_ICON: f64 = 56.0;
const STACK_GAP: f64 = 8.0;
/// A one-line item is 56 dp, a two-line one 72, with 8 of vertical padding around taller content.
const ITEM_PAD_V: f64 = 8.0;
const ITEM_MIN: f64 = 56.0;
const ITEM_MIN_TWO_LINE: f64 = 72.0;
/// The About header: a three-line item around the 56 dp app icon.
const HEADER_PAD_V: f64 = 16.0;
const HEADER_MIN: f64 = 88.0;
/// A category header's padding (less above the first one on a page), a footnote's.
const CATEGORY_TOP: f64 = 24.0;
const CATEGORY_TOP_FIRST: f64 = 8.0;
const CATEGORY_BOTTOM: f64 = 8.0;
const NOTE_TOP: f64 = 8.0;
const NOTE_BOTTOM: f64 = 12.0;
/// The page's own inset above its first line and below its last.
const PAGE_TOP: f64 = 8.0;
const PAGE_BOTTOM: f64 = 24.0;
/// The width a row measures at when nothing proposes one (a phone's content width).
const FALLBACK_WIDTH: f64 = 360.0;
/// The About page's hero: the icon size and the padding around the centered block.
const HERO_ICON: f64 = 96.0;
const HERO_PAD: f64 = 24.0;

// Flag bits the Java side reads (`DaySettingsRow.F_*`).
const F_CHEVRON: i32 = 1;
const F_DESTRUCTIVE: i32 = 2;
const F_RIPPLE: i32 = 4;
const F_FORWARD: i32 = 8;
const F_TEMPLATE_ICON: i32 = 16;
const F_FIRST: i32 = 32;

/// What a line of the page is (`DaySettingsRow.STYLE_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    /// A preference row: headline, supporting text, the control or a chevron trailing.
    Item = 0,
    /// The category header above a group of rows.
    Category = 1,
    /// A footnote: supporting text alone (a section footer, a note row, the copyright line).
    Note = 2,
    /// The About header on the root list: the app icon, the name, the version.
    Header = 3,
}

/// Realize-time props of the native row.
#[derive(Clone, Debug, PartialEq)]
struct RowProps {
    title: String,
    caption: String,
    /// The leading glyph's drawable name ("" = none).
    icon: String,
    style: Style,
    flags: i32,
    enabled: bool,
}

/// Later changes: a relocalized text, the `enabled` condition.
#[derive(Clone, Debug, PartialEq)]
enum RowPatch {
    Texts { title: String, caption: String },
    Enabled(bool),
}

// ---------------------------------------------------------------------------
// Renderer
// ---------------------------------------------------------------------------

fn make(_backend: &mut Android, p: &RowProps, _id: NodeId) -> AHandle {
    day_android::realize("settings_row", |env| {
        let title = JString::from_str(env, &p.title)?;
        let caption = JString::from_str(env, &p.caption)?;
        let icon = JString::from_str(env, &p.icon)?;
        DaySettingsRow::makeRow(
            env,
            &title,
            &caption,
            &icon,
            p.style as i32,
            p.flags,
            p.enabled,
        )
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &RowPatch) {
    match patch {
        RowPatch::Texts { title, caption } => {
            java(|env| {
                let title = JString::from_str(env, title)?;
                let caption = JString::from_str(env, caption)?;
                DaySettingsRow::setRowTexts(env, h.0.as_obj(), &title, &caption)
            });
        }
        RowPatch::Enabled(on) => {
            java(|env| DaySettingsRow::setRowEnabled(env, h.0.as_obj(), *on));
        }
    }
}

/// The text block alone: its natural width when unconstrained, else its wrapped height at the
/// proposed width. `RowLayout` asks through `cx.measure_leaf`; the row's full size is the
/// layout's business, since it depends on the Day child too.
fn measure(_backend: &mut Android, h: &AHandle, p: Proposal) -> Size {
    let width = match p.width {
        Some(w) => w,
        None => java(|env| DaySettingsRow::rowTextWidth(env, h.0.as_obj())).unwrap_or(0.0) as f64,
    };
    let height = java(|env| DaySettingsRow::rowTextHeight(env, h.0.as_obj(), width as f32))
        .unwrap_or(0.0) as f64;
    Size::new(width, height)
}

day_pieces::renderer!(day_android::RENDERERS, Android,
    kind: ROW_KIND, props: RowProps, patch: RowPatch,
    make: make, update: update, measure: measure);

// ---------------------------------------------------------------------------
// Layout: texts leading (drawn natively), the Day child trailing or stacked under them
// ---------------------------------------------------------------------------

/// How the row arranges itself: the texts take the leading column (past the icon slot); the
/// single Day child sits at the trailing edge when it fits beside the texts' natural width,
/// else under them (a slider always). A control that grows (a slider, a text field) takes
/// whatever width is left in either arrangement. A category header and a footnote are a text
/// block with its own padding, never a control.
struct RowLayout {
    style: Style,
    first: bool,
    icon: bool,
    chevron: bool,
    stack: bool,
    two_line: bool,
}

struct Plan {
    size: Size,
    control: Option<Rect>,
}

impl RowLayout {
    fn leading(&self) -> f64 {
        let icon = match (self.icon, self.style) {
            (false, _) => 0.0,
            (true, Style::Header) => HEADER_ICON + GAP,
            (true, _) => ICON + GAP,
        };
        PAD_H + icon
    }

    fn plan(&self, cx: &mut dyn LayoutOps, children: &[RNode], width: f64) -> Plan {
        let trailing_reserve = if self.chevron { CHEVRON + GAP } else { 0.0 };
        let leading = self.leading();
        let inner = (width - leading - PAD_H - trailing_reserve).max(0.0);
        match self.style {
            Style::Category => {
                let text = cx.measure_leaf(Proposal::new(Some(inner), None));
                let top = if self.first {
                    CATEGORY_TOP_FIRST
                } else {
                    CATEGORY_TOP
                };
                return Plan {
                    size: Size::new(width, top + text.height + CATEGORY_BOTTOM),
                    control: None,
                };
            }
            Style::Note => {
                let text = cx.measure_leaf(Proposal::new(Some(inner), None));
                return Plan {
                    size: Size::new(width, NOTE_TOP + text.height + NOTE_BOTTOM),
                    control: None,
                };
            }
            Style::Item | Style::Header => {}
        }
        let (pad_v, min) = match self.style {
            Style::Header => (HEADER_PAD_V, HEADER_MIN),
            _ if self.two_line => (ITEM_PAD_V, ITEM_MIN_TWO_LINE),
            _ => (ITEM_PAD_V, ITEM_MIN),
        };
        let Some(&control) = children.first() else {
            let text = cx.measure_leaf(Proposal::new(Some(inner), None));
            let height = (text.height + 2.0 * pad_v).max(min);
            return Plan {
                size: Size::new(width, height),
                control: None,
            };
        };
        let grow = cx.flex_of(control).grow_w;
        let stacked = |cx: &mut dyn LayoutOps| {
            // The texts across the row, the control under them at the leading edge.
            let text_h = cx.measure_leaf(Proposal::new(Some(inner), None)).height;
            let c = cx.measure_child(
                control,
                Proposal::new(if grow { Some(inner) } else { None }, None),
            );
            let cw = if grow { inner } else { c.width.min(inner) };
            let height = pad_v + text_h + STACK_GAP + c.height + pad_v;
            Plan {
                size: Size::new(width, height),
                control: Some(Rect::new(leading, pad_v + text_h + STACK_GAP, cw, c.height)),
            }
        };
        if self.stack {
            return stacked(cx);
        }
        let natural_text_w = cx.measure_leaf(Proposal::UNCONSTRAINED).width;
        let c_nat = cx.measure_child(control, Proposal::UNCONSTRAINED);
        if natural_text_w + GAP + c_nat.width <= inner {
            // Beside: the texts at their natural width, the control trailing (growing into the
            // rest when it asks to).
            let cw = if grow {
                inner - natural_text_w - GAP
            } else {
                c_nat.width
            };
            let c = if grow {
                cx.measure_child(control, Proposal::new(Some(cw), None))
            } else {
                c_nat
            };
            let text_w = (inner - cw - GAP).max(0.0);
            let text_h = cx.measure_leaf(Proposal::new(Some(text_w), None)).height;
            let height = (text_h.max(c.height) + 2.0 * pad_v).max(min);
            Plan {
                size: Size::new(width, height),
                control: Some(Rect::new(
                    width - PAD_H - trailing_reserve - cw,
                    (height - c.height) / 2.0,
                    cw,
                    c.height,
                )),
            }
        } else {
            stacked(cx)
        }
    }
}

impl Layout for RowLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        self.plan(cx, children, p.width.unwrap_or(FALLBACK_WIDTH))
            .size
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        let plan = self.plan(cx, children, bounds.size.width);
        if let (Some(&control), Some(rect)) = (children.first(), plan.control) {
            cx.place_child(control, rect);
        }
    }
}

// ---------------------------------------------------------------------------
// The row piece
// ---------------------------------------------------------------------------

type Enabled = Rc<dyn Fn() -> bool>;

/// A native line of the page hosting at most one Day child.
struct NativeRow {
    title: TextSource,
    caption: Option<TextSource>,
    icon: Option<String>,
    template_icon: bool,
    style: Style,
    first: bool,
    chevron: bool,
    destructive: bool,
    ripple: bool,
    forward_click: bool,
    stack: bool,
    /// Tracked condition under which the row is enabled (`None` = always).
    enabled: Option<Enabled>,
    control: Option<AnyPiece>,
}

impl NativeRow {
    fn new<M>(title: impl IntoText<M>) -> Self {
        NativeRow {
            title: title.into_text(),
            caption: None,
            icon: None,
            template_icon: false,
            style: Style::Item,
            first: false,
            chevron: false,
            destructive: false,
            ripple: false,
            forward_click: false,
            stack: false,
            enabled: None,
            control: None,
        }
    }
    /// A preference row for `r`: its title, caption and `enabled` condition.
    fn of(r: &Row) -> Self {
        let mut row = NativeRow::new(Resolved(r.title.clone()));
        row.caption = r.caption.clone();
        if let Some(e) = r.enabled.clone() {
            row.enabled = Some(Rc::new(move || e.get()));
        }
        row
    }
    fn caption(mut self, c: &TextSource) -> Self {
        self.caption = Some(c.clone());
        self
    }
    /// The leading glyph: a staged drawable, recolored to the list item's icon color when
    /// `template` (a group's vector or symbol), as authored otherwise (the app icon).
    fn icon(mut self, name: String, template: bool) -> Self {
        self.icon = Some(name);
        self.template_icon = template;
        self
    }
    fn style(mut self, s: Style) -> Self {
        self.style = s;
        self
    }
    fn first(mut self, first: bool) -> Self {
        self.first = first;
        self
    }
    fn control(mut self, c: AnyPiece) -> Self {
        self.control = Some(c);
        self
    }
    /// A row that pushes a page: a trailing chevron, and the ripple.
    fn chevron(mut self) -> Self {
        self.chevron = true;
        self.ripple = true;
        self
    }
    /// The whole row is a tap target (the caller attaches the `on_tap`): the ripple.
    fn ripple(mut self) -> Self {
        self.ripple = true;
        self
    }
    /// A tap on the row clicks its Day child (a choice row opens its spinner).
    fn forward_click(mut self) -> Self {
        self.forward_click = true;
        self
    }
    fn destructive(mut self, on: bool) -> Self {
        self.destructive = on;
        self
    }
    /// The control goes under the texts whatever the room beside them (a slider).
    fn stack(mut self) -> Self {
        self.stack = true;
        self
    }
    fn enabled(mut self, f: impl Fn() -> bool + 'static) -> Self {
        self.enabled = Some(Rc::new(f));
        self
    }
}

impl Piece for NativeRow {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let caption_src = self.caption.clone();
        let title_src = self.title.clone();
        let mut flags = 0;
        for (on, bit) in [
            (self.chevron, F_CHEVRON),
            (self.destructive, F_DESTRUCTIVE),
            (self.ripple, F_RIPPLE),
            (self.forward_click, F_FORWARD),
            (self.template_icon, F_TEMPLATE_ICON),
            (self.first, F_FIRST),
        ] {
            if on {
                flags |= bit;
            }
        }
        let caption_initial = caption_src
            .as_ref()
            .map(TextSource::initial)
            .unwrap_or_default();
        let props = RowProps {
            title: title_src.initial(),
            caption: caption_initial.clone(),
            icon: self.icon.clone().unwrap_or_default(),
            style: self.style,
            flags,
            enabled: self
                .enabled
                .as_ref()
                .is_none_or(|e| day_reactive::untrack(|| e())),
        };
        let node = cx.native(
            ROW_KIND,
            &props,
            Rc::new(RowLayout {
                style: self.style,
                first: self.first,
                icon: self.icon.is_some(),
                chevron: self.chevron,
                stack: self.stack,
                two_line: !caption_initial.is_empty(),
            }),
            Flex {
                grow_w: true,
                ..Default::default()
            },
            day_core::Boundary::No,
        );
        if let Some(control) = self.control {
            cx.under(node, |cx| {
                let _ = control.build(cx);
            });
        }
        // A relocalized title or caption (a `Dyn` source re-resolves under the locale signal),
        // or a live value: patch the texts and let the row re-measure.
        let dynamic = matches!(title_src, TextSource::Dyn(_))
            || matches!(caption_src, Some(TextSource::Dyn(_)));
        if dynamic {
            watch(
                move || {
                    (
                        tracked(&title_src),
                        caption_src.as_ref().map(tracked).unwrap_or_default(),
                    )
                },
                move |(title, caption), _| {
                    with_tree(|t| {
                        t.patch(
                            node,
                            Box::new(RowPatch::Texts {
                                title: title.clone(),
                                caption: caption.clone(),
                            }),
                            true,
                        )
                    });
                    day_core::invalidate_size(node);
                },
            );
        }
        if let Some(enabled) = self.enabled {
            watch(
                move || enabled(),
                move |on, _| {
                    with_tree(|t| t.patch(node, Box::new(RowPatch::Enabled(*on)), false));
                },
            );
        }
        node
    }
}

/// A tracked read of a text source: `TextSource::initial` untracks, which is right for a
/// snapshot and wrong for a watch that must re-run when the locale changes.
fn tracked(t: &TextSource) -> String {
    match t {
        TextSource::Static(s) => s.clone(),
        TextSource::Dyn(f) => f(),
    }
}

/// The drawable name of a group's icon: a bundled vector by its own name, a standard glyph by
/// the Material Symbol day-android ships for it.
fn icon_drawable(icon: &GroupIcon) -> String {
    match icon {
        GroupIcon::Image(name) => name.as_str().to_owned(),
        GroupIcon::Symbol(sym) => day_android::symbol_drawable(*sym),
    }
}

// ---------------------------------------------------------------------------
// The row hook the crate calls
// ---------------------------------------------------------------------------

/// The Material preference row for `r`, or `None` for a kind the composed row serves better (a
/// custom piece is already the app's own shape).
pub(super) fn row_piece(inst: Rc<Installed>, r: &Row, _desktop: bool) -> Option<AnyPiece> {
    let id = r.id.clone();
    let enabled = r.enabled.clone();
    let with_id = |p: AnyPiece| match &id {
        Some(id) => p.id(id.clone()).any(),
        None => p,
    };
    let enabled_fn = move || enabled.as_ref().is_none_or(|e| e.get());
    Some(match &r.kind {
        RowKind::Toggle(sig) => {
            // The SwitchPreference: the switch trailing, and the whole row flips it.
            let s = *sig;
            let ctl = with_id(toggle(s).enabled(enabled_fn.clone()).any());
            NativeRow::of(r)
                .control(ctl)
                .ripple()
                .on_tap(move || {
                    if enabled_fn() {
                        s.set(!s.get_untracked());
                    }
                })
                .any()
        }
        RowKind::Choice {
            labels,
            index,
            style,
        } => {
            let segmented = *style == ChoiceStyle::Segmented;
            let p = choice_picker(labels, *index, segmented, enabled_fn);
            let row = NativeRow::of(r);
            let row = if segmented {
                row
            } else {
                // The ListPreference: a tap anywhere on the row opens the choice.
                row.forward_click()
            };
            row.control(with_id(p.any())).any()
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
            // The id goes on the slider itself, under the layout wrappers: dayscript's
            // `set_value` must reach the control, not a wrapper. The stretch goes on the
            // outermost wrapper, so the slider spans the row the way a SeekBarPreference does.
            let ctl = match &id {
                Some(i) => s.id(i.clone()).min_width(140.0).grow_w().any(),
                None => s.min_width(140.0).grow_w().any(),
            };
            let ctl = match value_text {
                Some(f) => {
                    let f = f.clone();
                    let v = *value;
                    row((ctl, label(move || f(v.get())).tabular().secondary()))
                        .spacing(8.0)
                        .align(VAlign::Center)
                        .grow_w()
                        .any()
                }
                None => ctl,
            };
            // The SeekBarPreference: the slider spans the row under its title.
            NativeRow::of(r).stack().control(ctl).any()
        }
        RowKind::Stepper { value, range, step } => {
            let v = *value;
            let (dec, inc) = stepper_buttons(v, range, *step, enabled_fn);
            let (dec, inc) = (dec.compact(), inc.compact());
            let ctl = stepper_control(v, dec, inc, id.as_ref());
            NativeRow::of(r).control(ctl).any()
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
            // The id on the field itself (see the slider): `input` must reach the EditText.
            let f = f.enabled(enabled_fn);
            let ctl = match &id {
                Some(i) => f.id(i.clone()).grow_w().min_width(160.0).any(),
                None => f.grow_w().min_width(160.0).any(),
            };
            NativeRow::of(r).control(ctl).any()
        }
        RowKind::Action {
            on,
            destructive,
            confirm,
            confirm_label,
            busy,
        } => {
            // A command is a plain row whose tap runs it (red when destructive), the way
            // "Clear cache" is a row of the Settings app; a busy one spins at the trailing
            // edge and stops taking taps.
            let on = on.clone();
            let title = text_source_fn(&r.title);
            let confirm = confirm.clone();
            let confirm_label = confirm_label.clone();
            let busy = busy.clone();
            let busy_gate = busy.clone();
            let active = move || enabled_fn() && !busy_gate.as_ref().is_some_and(|b| b.get());
            let gate = active.clone();
            let run = move || {
                if !active() {
                    return;
                }
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
            let mut row = NativeRow::of(r)
                .destructive(*destructive)
                .ripple()
                .enabled(gate);
            if let Some(b) = busy {
                row = row.control(when(move || b.get(), spinner).any());
            }
            with_id(row.on_tap(run).any())
        }
        RowKind::Link(url) => {
            let url = url.clone();
            with_id(
                NativeRow::of(r)
                    .ripple()
                    .on_tap(move || day_core::open_url(&url))
                    .any(),
            )
        }
        RowKind::Navigate(build) => {
            let build = build.clone();
            let title = text_source_fn(&r.title);
            let open = move || {
                let build = build.clone();
                let title = title();
                push_subpage(inst.clone(), title, build);
            };
            with_id(NativeRow::of(r).chevron().on_tap(open).any())
        }
        RowKind::Info { value, copyable } => {
            // The value is the row's summary line (the "About phone" shape); a row that
            // already has a caption keeps it there and shows the value trailing.
            let row = match &r.caption {
                Some(_) => NativeRow::of(r).control(text_label(value).secondary().any()),
                None => NativeRow::of(r).caption(value),
            };
            if *copyable {
                let text = value.clone();
                with_id(
                    row.ripple()
                        .on_tap(move || {
                            day_part_clipboard::set_text(&text.initial());
                        })
                        .any(),
                )
            } else {
                with_id(row.any())
            }
        }
        RowKind::Note => with_id(footnote(&r.title).any()),
        RowKind::Custom(_) => return None,
    })
}

// ---------------------------------------------------------------------------
// The pages
// ---------------------------------------------------------------------------

/// A category header.
fn category(t: &TextSource, first: bool) -> NativeRow {
    NativeRow::new(Resolved(t.clone()))
        .style(Style::Category)
        .first(first)
}

/// A footnote line: supporting text alone.
fn footnote(t: &TextSource) -> NativeRow {
    NativeRow::new("").style(Style::Note).caption(t)
}

/// The lines of one section: its category header, its rows, its footer. Nothing when every
/// row is unavailable.
fn section_lines(inst: &Rc<Installed>, s: &Section, first: bool, out: &mut Vec<AnyPiece>) {
    let rows: Vec<&Row> = s.rows.iter().filter(|r| r.is_available()).collect();
    if rows.is_empty() {
        return;
    }
    if let Some(t) = &s.title {
        out.push(category(t, first).any());
    }
    for r in rows {
        let piece = crate::row_piece(inst.clone(), r, false);
        // A custom row is the app's own piece: give it the row's inset, since it draws no
        // padding of its own.
        out.push(if matches!(r.kind, RowKind::Custom(_)) {
            piece.padding(row_insets()).any()
        } else {
            piece
        });
    }
    if let Some(f) = &s.footer {
        out.push(footnote(f).any());
    }
}

/// The inset a composed piece takes to align with the rows around it.
fn row_insets() -> day_spec::Insets {
    day_spec::Insets {
        top: ITEM_PAD_V,
        leading: PAD_H,
        bottom: ITEM_PAD_V,
        trailing: PAD_H,
    }
}

/// A group's lines: every section, then its custom piece.
fn group_lines(inst: &Rc<Installed>, g: &Group, out: &mut Vec<AnyPiece>) {
    let first_line = out.len();
    for s in &g.sections {
        section_lines(inst, s, out.len() == first_line, out);
    }
    if let Some(custom) = &g.custom {
        out.push(custom().padding(row_insets()).any());
    }
}

/// The page: its lines in a full-bleed scrolling list.
fn page(lines: Vec<AnyPiece>, id: day_core::ElementId) -> AnyPiece {
    scroll(body(lines, id)).any()
}

/// A page's lines without the scroll.
fn body(lines: Vec<AnyPiece>, id: day_core::ElementId) -> AnyPiece {
    column(lines)
        .spacing(0.0)
        .align(HAlign::Leading)
        .padding(day_spec::Insets {
            top: PAGE_TOP,
            leading: 0.0,
            bottom: PAGE_BOTTOM,
            trailing: 0.0,
        })
        .grow_w()
        .id(id)
        .any()
}

/// The settings root as the Settings app's home: the About header, then one row per group with
/// its icon and subtitle (one group: its rows inline under the header).
pub(super) fn root_page(inst: Rc<Installed>) -> Option<AnyPiece> {
    let (about, groups) = root_entries(&inst);
    let mut lines: Vec<AnyPiece> = Vec::new();
    if let Some(a) = about {
        let open = a.open.clone();
        let mut row = NativeRow::new(Resolved(a.title.clone()))
            .style(Style::Header)
            .chevron();
        if let Some(sub) = &a.subtitle {
            row = row.caption(sub);
        }
        if let Some(icon) = &a.icon {
            row = row.icon(icon_drawable(icon), false);
        }
        lines.push(row.on_tap(move || open()).id(day_core::id!(settings_about)).any());
    }
    let visible = inst.visible_groups();
    if let [g] = visible[..] {
        group_lines(&inst, g, &mut lines);
    } else {
        for e in groups {
            let open = e.open.clone();
            let mut row = NativeRow::new(Resolved(e.title.clone())).chevron();
            if let Some(sub) = &e.subtitle {
                row = row.caption(sub);
            }
            if let Some(icon) = &e.icon {
                row = row.icon(icon_drawable(icon), true);
            }
            lines.push(
                row.on_tap(move || open())
                    .id(day_core::id!(prefs_group, e.id.clone()))
                    .any(),
            );
        }
    }
    Some(page(lines, day_core::id!(settings_root)))
}

/// One group's page: its sections as category headers, rows and footnotes.
pub(super) fn group_page(inst: Rc<Installed>, g: &Group) -> Option<AnyPiece> {
    let mut lines: Vec<AnyPiece> = Vec::new();
    group_lines(&inst, g, &mut lines);
    Some(page(lines, day_core::id!(prefs_group, g.id)))
}

/// One group's lines with no scroll, for a sheet's own `scroll`.
pub(super) fn group_body(inst: Rc<Installed>, g: &Group) -> Option<AnyPiece> {
    let mut lines: Vec<AnyPiece> = Vec::new();
    group_lines(&inst, g, &mut lines);
    Some(body(lines, day_core::id!(prefs_group, g.id)))
}

/// The About page: the app's identity centered at the top, then the links, the
/// Acknowledgements and Licenses rows and the copyright line as list rows.
pub(super) fn about_page(inst: Rc<Installed>) -> Option<AnyPiece> {
    let about = inst.about.as_ref()?;
    let mut hero: Vec<AnyPiece> = Vec::new();
    if let Some(n) = about.icon_name() {
        hero.push(
            image(n.clone())
                .frame(HERO_ICON, HERO_ICON)
                .corner_radius(HERO_ICON / 4.5)
                .any(),
        );
    }
    hero.push(
        label(about.display_name())
            .font(day_spec::Font::Title2)
            .bold()
            .id(day_core::id!(about_name))
            .any(),
    );
    if let Some(v) = about.version_line() {
        hero.push(label(v).secondary().id(day_core::id!(about_version)).any());
    }
    if let Some(t) = about.tagline_text() {
        hero.push(
            label(t)
                .align(day_spec::props::TextAlign::Center)
                .id(day_core::id!(about_tagline))
                .padding(day_spec::Insets {
                    top: 0.0,
                    leading: PAD_H,
                    bottom: 0.0,
                    trailing: PAD_H,
                })
                .any(),
        );
    }
    // The hero column stretches to the page (it must be the page column's direct child for
    // that: a padding wrapper would take the stretch and leave the column at its natural
    // width), so its vertical breathing room is two spacers.
    let mut lines: Vec<AnyPiece> = vec![
        spacer().frame(1.0, HERO_PAD).any(),
        column(hero)
            .spacing(6.0)
            .align(HAlign::Center)
            .grow_w()
            .any(),
        spacer().frame(1.0, HERO_PAD).any(),
    ];
    // Ids by position (`about-link-0`, …) and `about-privacy`: what a walkthrough asserts.
    let mut links: Vec<(TextSource, String, day_core::ElementId)> = about
        .links()
        .iter()
        .enumerate()
        .map(|(i, (t, url))| (t.clone(), url.clone(), day_core::id!(about_link, i)))
        .collect();
    if let Some(p) = about.privacy_url() {
        links.push((
            day_fluent::res::str::day_about_privacy().into_text(),
            p.to_string(),
            day_core::id!(about_privacy),
        ));
    }
    for (title, url, id) in links {
        lines.push(
            NativeRow::new(Resolved(title))
                .ripple()
                .on_tap(move || day_core::open_url(&url))
                .id(id)
                .any(),
        );
    }
    if let Some(c) = about.credits_text() {
        let credits = c.clone();
        let r = rows::navigate(day_fluent::res::str::day_about_acknowledgements(), move || {
            scroll(label(credits.clone()).padding(PAD_H))
        });
        lines.push(crate::row_piece(inst.clone(), &r, false));
    }
    if let Some(l) = about.licenses_text() {
        let licenses = l.clone();
        let r = rows::navigate(day_fluent::res::str::day_about_licenses(), move || {
            scroll(label(licenses.clone()).monospace().padding(PAD_H))
        });
        lines.push(crate::row_piece(inst.clone(), &r, false));
    }
    if let Some(c) = about.copyright_text() {
        lines.push(footnote(&TextSource::Static(c)).id(day_core::id!(about_copyright)).any());
    }
    Some(page(lines, day_core::id!(about_page)))
}
