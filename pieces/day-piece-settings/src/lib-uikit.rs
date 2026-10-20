// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// UIKit: the iOS Settings surface, in two shapes.
//
// THE PAGES (`root_page`, `group_page`, `about_page`) are a real `UITableView` in the
// `.insetGrouped` style (`DaySettingsTable`): native `UITableViewCell`s configured through
// `UIListContentConfiguration` (title, secondary line, an image for a group icon), the system
// disclosure chevron on navigate rows, section header and footer text from the declaration, the
// cell's own selection highlight on touch with the standard deselect-on-select, and system row
// heights. Each row is still a Day node: its native view is a `DaySettingsSlot`, a plain host
// the cell adopts — as its `accessoryView` when the row's Day control (a `toggle`, a `picker`,
// the stepper pair, a `text_field`) sits at the trailing edge, inside the content view under the
// texts for a full-width control (a slider), or as the whole content for a custom row and the
// About header. So the control keeps the composed row's element id and `enabled` gate, a row
// with no control (navigate, link, action, info, a group, the About header) carries the id
// itself, and dayscript `tap:`/`toggle:`/`select:`/`set_value:` land on the same ids as every
// other backend. The table is static: one cell per Day row node, no recycling.
//
// THE ROW ALONE (`row_piece`) serves the composed `section()` card, which is what a sheet that
// stacks several groups and owns the scrolling asks for (`Prefs::group_body`): a container piece
// whose realized view, `DayPrefsRow`, draws what a `UICollectionViewListCell` draws — title,
// secondary line, trailing value, chevron or external-link glyph, the hairline separator to the
// next row, the red / tint-colored title of an action row — with the Day control as its one
// child, placed at the trailing edge by `RowLayout`.
//
// Colors are the semantic UIKit ones (label, secondaryLabel, separator, systemRed, the view's
// tint), so light and dark mode need no code here.
// ---------------------------------------------------------------------------

use super::*;

use std::cell::Cell;

use day_core::{Boundary, BuildCx, Flex, Layout, LayoutOps, RNode, with_tree};
use day_spec::{Event, NodeId, Proposal, Rect, Size};
use day_uikit::Uikit;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_core_foundation::{CGFloat, CGPoint, CGRect, CGSize};
use objc2_foundation::{NSIndexPath, NSInteger, NSString};
use objc2_ui_kit::{
    NSIndexPathUIKitAdditions, NSTextAlignment, UIBackgroundConfiguration, UIColor, UIFont,
    UIImage, UIImageRenderingMode, UIImageSymbolConfiguration, UIImageSymbolWeight, UIImageView,
    UILabel, UIListContentConfiguration, UIListContentTextAlignment, UIListContentView,
    UIScrollViewDelegate, UIScrollViewKeyboardDismissMode, UITableView, UITableViewCell,
    UITableViewCellAccessoryType, UITableViewCellSelectionStyle, UITableViewCellStyle,
    UITableViewDataSource, UITableViewDelegate, UITableViewStyle, UITraitEnvironment,
    UIUserInterfaceLayoutDirection, UIView, UIViewContentMode,
};

/// The standalone row's piece kind under `day_uikit::RENDERERS`.
const ROW_KIND: &str = "day-piece-settings.row";
/// The grouped table's piece kind.
const TABLE_KIND: &str = "day-piece-settings.table";
/// One table cell's piece kind.
const CELL_KIND: &str = "day-piece-settings.cell";

/// Horizontal text inset inside the standalone row: the section card already pads 14, the
/// system list insets cell content 16.
const INSET: f64 = 2.0;
/// The gap between the title column and whatever sits at the trailing edge.
const GAP: f64 = 12.0;
/// Vertical padding around the taller of title column and control.
const VPAD: f64 = 4.0;
/// The row's floor: with the card's 10-point spacing this gives the system list's 44-point
/// pitch between consecutive rows.
const MIN_HEIGHT: f64 = 34.0;
/// The space between the title and its secondary line.
const CAPTION_GAP: f64 = 2.0;
/// The widest the title column grows beside a stretching control (a slider, a text field).
const TITLE_SHARE: f64 = 0.45;

/// The system row pitch: the floor a controlless cell's Day frame reports.
const ROW_PITCH: f64 = 44.0;
/// The inset grouped style's card margin on a phone: what a cell's content width is, measured
/// from the table's.
const CARD_INSET: f64 = 20.0;
/// A cell's horizontal content margin.
const CELL_PAD: f64 = 16.0;
/// Vertical padding around a control that is the cell's whole content.
const FILL_VPAD: f64 = 10.0;
/// The space under a full-width control placed beneath the cell's texts.
const INLINE_BOTTOM: f64 = 8.0;
/// A stretching accessory control's (a text field's) share of the table width, and its floor.
const FIELD_SHARE: f64 = 0.5;
const FIELD_MIN: f64 = 160.0;
/// A group row's icon, the size an iOS list icon is.
const GROUP_ICON: f64 = 29.0;
/// The About header row's app icon.
const ABOUT_ICON: f64 = 40.0;
/// A nominal width for a table measured with no proposal (never the case under a nav page).
const NOMINAL_WIDTH: f64 = 390.0;

/// What a row shows at its trailing edge besides the Day control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Accessory {
    None,
    /// A navigate row: the disclosure chevron.
    Chevron,
    /// A link row: the external-link glyph.
    External,
}

/// How the title is colored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    /// The label color.
    Plain,
    /// The view's tint: an action or a link.
    Tint,
    /// System red, centered: a destructive action.
    Destructive,
    /// A footnote paragraph: the footnote style in the secondary color.
    Note,
}

/// Where a grouped-table cell puts the row's Day child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Host {
    /// No Day child: the cell's texts and system accessories alone.
    Empty,
    /// The child is the cell's `accessoryView`, trailing, vertically centered by the cell.
    Accessory,
    /// The child runs the full content width under the texts (a slider).
    Inline,
    /// The child is the cell's whole content (a custom row, the About header).
    Fill,
}

/// A cell's leading image.
#[derive(Clone, Debug, PartialEq)]
enum CellImage {
    /// A bundled image or vector, by the name the toolkit resolves (docs/vectors.md).
    Named(String),
    /// An SF Symbol, by name.
    Symbol(String),
}

// ---------------------------------------------------------------------------
// What a row contributes beyond its texts: shared by the standalone row and the table cell
// ---------------------------------------------------------------------------

/// The parts of a row both native shapes assemble: the Day control (if any) and where a cell
/// hosts it, what tapping the row does, how the title is toned, the trailing glyph, an info
/// row's value, the row's own enabled gate, and the element id when no control carries it.
struct RowParts {
    control: Option<AnyPiece>,
    host: Host,
    tap: Option<Rc<dyn Fn()>>,
    tone: Tone,
    accessory: Accessory,
    value: Option<TextSource>,
    gate: Option<Rc<dyn Fn() -> bool>>,
    row_id: Option<String>,
}

impl RowParts {
    fn new() -> Self {
        RowParts {
            control: None,
            host: Host::Empty,
            tap: None,
            tone: Tone::Plain,
            accessory: Accessory::None,
            value: None,
            gate: None,
            row_id: None,
        }
    }
    fn control(mut self, p: AnyPiece, host: Host) -> Self {
        self.control = Some(p);
        self.host = host;
        self
    }
    fn tap(mut self, f: impl Fn() + 'static) -> Self {
        self.tap = Some(Rc::new(f));
        self
    }
    fn gate(mut self, f: impl Fn() -> bool + 'static) -> Self {
        self.gate = Some(Rc::new(f));
        self
    }
}

/// Build the row's parts. `None` for a note or a custom row, which the standalone row leaves to
/// the composed version and the table draws as its own cell kinds.
fn row_parts(inst: Rc<Installed>, r: &Row) -> RowParts {
    let id = r.id.clone();
    let enabled = r.enabled;
    let enabled_fn = move || enabled.is_none_or(|e| e.get());
    let with_id = |p: AnyPiece| match &id {
        Some(id) => p.id(id.clone()).any(),
        None => p,
    };
    let parts = RowParts::new();
    match &r.kind {
        RowKind::Toggle(sig) => parts.control(
            with_id(toggle(*sig).enabled(enabled_fn).any()),
            Host::Accessory,
        ),
        RowKind::Choice {
            labels,
            index,
            style,
        } => {
            let p = choice_picker(labels, *index, *style == ChoiceStyle::Segmented, enabled_fn);
            parts.control(with_id(p.any()), Host::Accessory)
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
            // The slider fills the width its row leaves: the whole content width under the
            // title in a grouped cell, the title column's remainder in the standalone row.
            let ctl = match &id {
                Some(i) => s.id(i.clone()).grow_w().any(),
                None => s.grow_w().any(),
            };
            let ctl = match value_text {
                Some(f) => {
                    let f = f.clone();
                    let v = *value;
                    // The value label carries `<id>-value`, as the composed row's does.
                    let value_label = label(move || f(v.get())).tabular().secondary();
                    let value_label = match &id {
                        Some(i) => value_label.id(format!("{i}-value")).any(),
                        None => value_label.any(),
                    };
                    row((ctl, value_label))
                        .spacing(8.0)
                        .align(VAlign::Center)
                        .grow_w()
                        .any()
                }
                None => ctl,
            };
            parts.control(ctl, Host::Inline)
        }
        RowKind::Stepper { value, range, step } => {
            let v = *value;
            let (dec, inc) = stepper_buttons(v, range, *step, enabled_fn);
            let ctl = stepper_control(v, dec, inc, id.as_deref());
            parts.control(ctl, Host::Accessory)
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
            let ctl = match &id {
                Some(i) => f
                    .enabled(enabled_fn)
                    .id(i.clone())
                    .grow_w()
                    .min_width(FIELD_MIN)
                    .any(),
                None => f.enabled(enabled_fn).grow_w().min_width(FIELD_MIN).any(),
            };
            parts.control(ctl, Host::Accessory)
        }
        RowKind::Action {
            on,
            destructive,
            confirm,
            confirm_label,
            busy,
        } => {
            // The iOS action row IS its title: tinted text, red when destructive, the whole row
            // tappable. A spinner shows at the trailing edge while the action is busy.
            let on = on.clone();
            let title = text_source_fn(&r.title);
            let confirm = confirm.clone();
            let confirm_label = confirm_label.clone();
            let busy = *busy;
            let active = move || enabled_fn() && !busy.is_some_and(|b| b.get());
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
            let mut parts = parts.gate(active).tap(run);
            parts.tone = if *destructive {
                Tone::Destructive
            } else {
                Tone::Tint
            };
            parts.row_id = id;
            if let Some(b) = busy {
                parts = parts.control(when(move || b.get(), spinner).any(), Host::Accessory);
            }
            parts
        }
        RowKind::Link(url) => {
            let url = url.clone();
            let mut parts = parts.gate(enabled_fn).tap(move || {
                if enabled_fn() {
                    day_core::open_url(&url);
                }
            });
            parts.tone = Tone::Tint;
            parts.accessory = Accessory::External;
            parts.row_id = id;
            parts
        }
        RowKind::Navigate(build) => {
            let build = build.clone();
            let title = text_source_fn(&r.title);
            let mut parts = parts.gate(enabled_fn).tap(move || {
                let build = build.clone();
                let title = title();
                push_subpage(inst.clone(), title, build);
            });
            parts.accessory = Accessory::Chevron;
            parts.row_id = id;
            parts
        }
        RowKind::Info { value, copyable } => {
            let mut parts = parts;
            parts.value = Some(value.clone());
            if *copyable {
                let text = value.clone();
                parts = parts.tap(move || {
                    day_part_clipboard::set_text(&text.initial());
                });
            }
            parts.row_id = id;
            parts
        }
        RowKind::Note => {
            let mut parts = parts;
            parts.tone = Tone::Note;
            parts.row_id = id;
            parts
        }
        RowKind::Custom(build) => {
            let mut parts = parts.control(build(), Host::Fill);
            parts.row_id = id;
            parts
        }
    }
}

// ---------------------------------------------------------------------------
// The standalone row piece (the composed card's rows)
// ---------------------------------------------------------------------------

/// Realize-time payload of a standalone row.
#[derive(Clone, Debug, PartialEq)]
struct RowProps {
    title: String,
    caption: Option<String>,
    value: Option<String>,
    accessory: Accessory,
    tone: Tone,
    enabled: bool,
}

/// Changes after realize: the texts follow their reactive sources, the enabled gate its signal.
#[derive(Clone, Debug, PartialEq)]
enum RowPatch {
    Title(String),
    Caption(Option<String>),
    Value(Option<String>),
    Enabled(bool),
}

/// One native row: the texts and accessories the view draws, plus the optional Day control.
struct NativeRow {
    title: TextSource,
    caption: Option<TextSource>,
    value: Option<TextSource>,
    accessory: Accessory,
    tone: Tone,
    /// The row's enabled gate, bound after realize (`None` = always enabled).
    enabled: Option<Rc<dyn Fn() -> bool>>,
    control: Option<AnyPiece>,
}

impl Piece for NativeRow {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let NativeRow {
            title,
            caption,
            value,
            accessory,
            tone,
            enabled,
            control,
        } = self;
        let props = RowProps {
            title: title.initial(),
            caption: caption.as_ref().map(TextSource::initial),
            value: value.as_ref().map(TextSource::initial),
            accessory,
            tone,
            enabled: enabled
                .as_ref()
                .is_none_or(|f| day_reactive::untrack(|| f())),
        };
        let node = cx.native(
            ROW_KIND,
            &props,
            Rc::new(RowLayout),
            Flex {
                grow_w: true,
                ..Default::default()
            },
            Boundary::No,
        );
        if let Some(control) = control {
            cx.under(node, |cx| {
                let _ = control.build(cx);
            });
        }
        bind_text(node, &title, RowPatch::Title);
        if let Some(c) = &caption {
            bind_text(node, c, |s| RowPatch::Caption(Some(s)));
        }
        if let Some(v) = &value {
            bind_text(node, v, |s| RowPatch::Value(Some(s)));
        }
        bind_gate(node, props.enabled, enabled, RowPatch::Enabled);
        node
    }
}

/// Keep a native text in step with a `TextSource::Dyn` (a localized string re-resolving on a
/// locale change); a static source needs nothing after realize.
fn bind_text<P: 'static>(node: RNode, t: &TextSource, make: impl Fn(String) -> P + 'static) {
    let TextSource::Dyn(f) = t else {
        return;
    };
    let f = f.clone();
    let seed = day_reactive::untrack(|| f());
    day_reactive::bind_seeded(
        seed,
        move || f(),
        move |s| {
            with_tree(|t| t.patch(node, Box::new(make(s.clone())), true));
        },
    );
}

/// Keep a native enabled state in step with the row's gate.
fn bind_gate<P: 'static>(
    node: RNode,
    seed: bool,
    gate: Option<Rc<dyn Fn() -> bool>>,
    make: impl Fn(bool) -> P + 'static,
) {
    let Some(f) = gate else {
        return;
    };
    day_reactive::bind_seeded(
        seed,
        move || f(),
        move |on| {
            with_tree(|t| t.patch(node, Box::new(make(*on)), false));
        },
    );
}

/// The row's layout: the native view measures its own texts and accessories (the leaf), the
/// Day child sits at the trailing edge, vertically centered. A stretching child (a slider, a
/// text field) takes the width the title column leaves; any other hugs its natural size and
/// the title column wraps into what remains.
struct RowLayout;

impl RowLayout {
    /// Measure the leaf and the child against `width`, returning `(leaf, child)`.
    fn parts(
        &self,
        cx: &mut dyn LayoutOps,
        children: &[RNode],
        width: Option<f64>,
    ) -> (Size, Option<(RNode, Size)>) {
        let Some(&child) = children.first() else {
            return (cx.measure_leaf(Proposal::new(width, None)), None);
        };
        if cx.flex_of(child).grow_w {
            let natural = cx.measure_leaf(Proposal::UNCONSTRAINED);
            let cap = width.map_or(natural.width, |w| natural.width.min(w * TITLE_SHARE));
            let leaf = cx.measure_leaf(Proposal::new(Some(cap), None));
            let avail = width.map(|w| (w - leaf.width - GAP).max(0.0));
            let cs = cx.measure_child(child, Proposal::new(avail, None));
            (leaf, Some((child, cs)))
        } else {
            let cs = cx.measure_child(child, Proposal::UNCONSTRAINED);
            let avail = width.map(|w| (w - cs.width - GAP).max(0.0));
            let leaf = cx.measure_leaf(Proposal::new(avail, None));
            (leaf, Some((child, cs)))
        }
    }
}

impl Layout for RowLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let (leaf, child) = self.parts(cx, children, p.width);
        let (cw, ch) = child.map_or((0.0, 0.0), |(_, s)| (s.width, s.height));
        let natural_w = if child.is_some() {
            leaf.width + GAP + cw
        } else {
            leaf.width
        };
        let h = leaf.height.max(ch + 2.0 * VPAD).max(MIN_HEIGHT);
        Size::new(p.width.unwrap_or(natural_w), h)
    }

    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        let (_, child) = self.parts(cx, children, Some(bounds.size.width));
        let Some((c, s)) = child else {
            return;
        };
        let x = (bounds.size.width - INSET - s.width).max(0.0);
        let y = ((bounds.size.height - s.height) / 2.0).max(0.0);
        cx.place_child(c, Rect::new(x, y, s.width, s.height));
    }
}

/// The native row for `r`, or `None` for the kinds the composed row already draws the iOS way
/// (a footnote, a custom piece).
pub(super) fn row_piece(inst: Rc<Installed>, r: &Row, _desktop: bool) -> Option<AnyPiece> {
    if matches!(r.kind, RowKind::Note | RowKind::Custom(_)) {
        return None;
    }
    let parts = row_parts(inst, r);
    let native = NativeRow {
        title: r.title.clone(),
        caption: r.caption.clone(),
        value: parts.value,
        accessory: parts.accessory,
        tone: parts.tone,
        enabled: parts.gate,
        control: parts.control,
    };
    let piece = match parts.tap {
        Some(tap) => native.on_tap(move || tap()).any(),
        None => native.any(),
    };
    Some(match parts.row_id {
        Some(id) => piece.id(id).any(),
        None => piece,
    })
}

// ---------------------------------------------------------------------------
// The standalone row's native view
// ---------------------------------------------------------------------------

struct RowIvars {
    title: Retained<UILabel>,
    caption: Retained<UILabel>,
    value: Retained<UILabel>,
    accessory: Retained<UIImageView>,
    separator: Retained<UIView>,
    tone: Cell<Tone>,
    enabled: Cell<bool>,
}

define_class!(
    #[unsafe(super(UIView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayPrefsRow"]
    #[ivars = RowIvars]
    struct PrefsRow;

    impl PrefsRow {
        #[unsafe(method(layoutSubviews))]
        fn layout_subviews(&self) {
            // SAFETY: plain super call of a UIView method on the main thread.
            let _: () = unsafe { msg_send![super(self), layoutSubviews] };
            self.layout_content();
        }

        #[unsafe(method(sizeThatFits:))]
        fn size_that_fits(&self, size: CGSize) -> CGSize {
            self.natural_size(size.width)
        }

        /// A sibling row joined or left the card: every row's separator depends on whether a
        /// row follows it, so re-lay all of them.
        #[unsafe(method(didMoveToSuperview))]
        fn did_move_to_superview(&self) {
            // SAFETY: plain super call of a UIView method on the main thread.
            let _: () = unsafe { msg_send![super(self), didMoveToSuperview] };
            if let Some(sv) = self.superview() {
                for v in sv.subviews().iter() {
                    if v.downcast_ref::<PrefsRow>().is_some() {
                        v.setNeedsLayout();
                    }
                }
            }
        }

        #[unsafe(method(tintColorDidChange))]
        fn tint_color_did_change(&self) {
            // SAFETY: plain super call of a UIView method on the main thread.
            let _: () = unsafe { msg_send![super(self), tintColorDidChange] };
            self.apply_colors();
        }
    }
);

impl PrefsRow {
    fn new(mtm: MainThreadMarker, p: &RowProps) -> Retained<Self> {
        let title = UILabel::new(mtm);
        title.setNumberOfLines(0);
        let caption = UILabel::new(mtm);
        caption.setNumberOfLines(0);
        let value = UILabel::new(mtm);
        value.setNumberOfLines(1);
        value.setTextAlignment(NSTextAlignment::Right);
        // SAFETY: the text-style constants are immutable statics UIKit exports; reading them
        // on the main thread is what `preferredFontForTextStyle:` is documented for.
        unsafe {
            title.setFont(Some(&UIFont::preferredFontForTextStyle(
                objc2_ui_kit::UIFontTextStyleBody,
            )));
            caption.setFont(Some(&UIFont::preferredFontForTextStyle(
                objc2_ui_kit::UIFontTextStyleFootnote,
            )));
            value.setFont(Some(&UIFont::preferredFontForTextStyle(
                objc2_ui_kit::UIFontTextStyleBody,
            )));
        }
        let accessory = UIImageView::initWithImage(UIImageView::alloc(mtm), None);
        accessory.setContentMode(UIViewContentMode::ScaleAspectFit);
        let separator = UIView::new(mtm);
        separator.setBackgroundColor(Some(&UIColor::separatorColor()));
        separator.setHidden(true);

        let this = Self::alloc(mtm).set_ivars(RowIvars {
            title,
            caption,
            value,
            accessory,
            separator,
            tone: Cell::new(p.tone),
            enabled: Cell::new(p.enabled),
        });
        // SAFETY: designated UIView initializer on a freshly allocated instance.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        let iv = this.ivars();
        this.addSubview(&iv.title);
        this.addSubview(&iv.caption);
        this.addSubview(&iv.value);
        this.addSubview(&iv.accessory);
        this.addSubview(&iv.separator);
        this.set_title(&p.title);
        this.set_caption(p.caption.as_deref());
        this.set_value(p.value.as_deref());
        this.set_accessory(p.accessory);
        this.apply_colors();
        this
    }

    fn set_title(&self, s: &str) {
        self.ivars().title.setText(Some(&NSString::from_str(s)));
        self.invalidate();
    }

    fn set_caption(&self, s: Option<&str>) {
        set_optional_text(&self.ivars().caption, s);
        self.invalidate();
    }

    fn set_value(&self, s: Option<&str>) {
        set_optional_text(&self.ivars().value, s);
        self.invalidate();
    }

    fn set_accessory(&self, a: Accessory) {
        let iv = &self.ivars().accessory;
        let name = match a {
            Accessory::None => {
                iv.setImage(None);
                iv.setHidden(true);
                return;
            }
            Accessory::Chevron => "chevron.right",
            Accessory::External => "arrow.up.right",
        };
        let image = glyph(name, 14.0);
        iv.setImage(image.as_deref());
        iv.setHidden(image.is_none());
        // SAFETY: a plain property write on a view this row owns, on the main thread.
        unsafe { iv.setTintColor(Some(&UIColor::tertiaryLabelColor())) };
    }

    fn set_enabled(&self, on: bool) {
        self.ivars().enabled.set(on);
        self.apply_colors();
    }

    /// The title follows its tone (label / tint / red) and dims while disabled; the secondary
    /// texts are always secondary.
    fn apply_colors(&self) {
        let iv = self.ivars();
        let color = title_color(iv.tone.get(), iv.enabled.get(), self.tintColor());
        // SAFETY: plain property writes on labels this row owns, on the main thread.
        unsafe {
            iv.title.setTextColor(Some(&color));
            iv.caption
                .setTextColor(Some(&UIColor::secondaryLabelColor()));
            iv.value.setTextColor(Some(&UIColor::secondaryLabelColor()));
        }
        iv.title
            .setTextAlignment(if iv.tone.get() == Tone::Destructive {
                NSTextAlignment::Center
            } else {
                NSTextAlignment::Natural
            });
    }

    fn invalidate(&self) {
        self.setNeedsLayout();
    }

    fn rtl(&self) -> bool {
        self.effectiveUserInterfaceLayoutDirection() == UIUserInterfaceLayoutDirection::RightToLeft
    }

    /// The width the trailing accessories take (value label and glyph, with their gaps).
    fn accessory_width(&self) -> CGFloat {
        let iv = self.ivars();
        let mut w = 0.0;
        if !iv.value.isHidden() {
            w += iv.value.sizeThatFits(CGSize::new(1.0e6, 1.0e6)).width + GAP;
        }
        if !iv.accessory.isHidden() {
            w += self.glyph_size().width + GAP;
        }
        w
    }

    fn glyph_size(&self) -> CGSize {
        let iv = &self.ivars().accessory;
        match iv.image() {
            // SAFETY: reading the size of an image the image view holds, on the main thread.
            Some(img) => unsafe { img.size() },
            None => CGSize::new(0.0, 0.0),
        }
    }

    /// The natural size of the texts and accessories inside `width` (the Day child is not
    /// counted: `RowLayout` adds it).
    fn natural_size(&self, width: CGFloat) -> CGSize {
        let iv = self.ivars();
        let acc = self.accessory_width();
        let cap = (width - 2.0 * INSET - acc).max(0.0);
        let title_nat = iv.title.sizeThatFits(CGSize::new(1.0e6, 1.0e6));
        let caption_nat = if iv.caption.isHidden() {
            CGSize::new(0.0, 0.0)
        } else {
            iv.caption.sizeThatFits(CGSize::new(1.0e6, 1.0e6))
        };
        let col_w = title_nat.width.max(caption_nat.width).min(cap);
        let title = iv.title.sizeThatFits(CGSize::new(col_w, 1.0e6));
        let mut h = title.height;
        if !iv.caption.isHidden() {
            h += CAPTION_GAP + iv.caption.sizeThatFits(CGSize::new(col_w, 1.0e6)).height;
        }
        let h = (h + 2.0 * VPAD).max(MIN_HEIGHT);
        CGSize::new((2.0 * INSET + col_w + acc).ceil(), h.ceil())
    }

    /// Lay the texts and accessories out around the Day child (whatever subview is not one of
    /// the row's own), and draw the separator when another row follows in the card.
    fn layout_content(&self) {
        let iv = self.ivars();
        let b = self.bounds();
        let (w, h) = (b.size.width, b.size.height);
        let rtl = self.rtl();
        // The Day child's extent: what the title column must stay clear of.
        let own = [
            Retained::as_ptr(&iv.title).cast::<UIView>(),
            Retained::as_ptr(&iv.caption).cast::<UIView>(),
            Retained::as_ptr(&iv.value).cast::<UIView>(),
            Retained::as_ptr(&iv.accessory).cast::<UIView>(),
            Retained::as_ptr(&iv.separator).cast::<UIView>(),
        ];
        let mut child_w: CGFloat = 0.0;
        for v in self.subviews().iter() {
            let p: *const UIView = Retained::as_ptr(&v);
            if own.contains(&p) || v.isHidden() {
                continue;
            }
            child_w = child_w.max(v.frame().size.width);
        }
        let child_reserve = if child_w > 0.0 { child_w + GAP } else { 0.0 };
        // Trailing edge, moving inward: glyph, then value, then the child's reserve.
        let mut trailing = INSET + child_reserve;
        let glyph = self.glyph_size();
        if !iv.accessory.isHidden() {
            let x = if rtl {
                trailing
            } else {
                w - trailing - glyph.width
            };
            iv.accessory.setFrame(CGRect::new(
                CGPoint::new(x, ((h - glyph.height) / 2.0).round()),
                glyph,
            ));
            trailing += glyph.width + GAP;
        }
        if !iv.value.isHidden() {
            let s = iv.value.sizeThatFits(CGSize::new(1.0e6, 1.0e6));
            let vw = s.width.min((w - trailing - INSET - GAP).max(0.0));
            let x = if rtl { trailing } else { w - trailing - vw };
            iv.value.setFrame(CGRect::new(
                CGPoint::new(x, ((h - s.height) / 2.0).round()),
                CGSize::new(vw, s.height),
            ));
            iv.value.setTextAlignment(if rtl {
                NSTextAlignment::Left
            } else {
                NSTextAlignment::Right
            });
            trailing += vw + GAP;
        }
        // The title column takes what is left.
        let col_w = (w - INSET - trailing).max(0.0);
        let centered = iv.tone.get() == Tone::Destructive;
        let col_x = if rtl && !centered { trailing } else { INSET };
        let col_w = if centered { w - 2.0 * INSET } else { col_w };
        let title = iv.title.sizeThatFits(CGSize::new(col_w, 1.0e6));
        let caption = if iv.caption.isHidden() {
            CGSize::new(0.0, 0.0)
        } else {
            iv.caption.sizeThatFits(CGSize::new(col_w, 1.0e6))
        };
        let total = title.height
            + if iv.caption.isHidden() {
                0.0
            } else {
                CAPTION_GAP + caption.height
            };
        let mut y = ((h - total) / 2.0).round().max(0.0);
        iv.title.setFrame(CGRect::new(
            CGPoint::new(col_x, y),
            CGSize::new(col_w, title.height),
        ));
        y += title.height + CAPTION_GAP;
        if !iv.caption.isHidden() {
            iv.caption.setFrame(CGRect::new(
                CGPoint::new(col_x, y),
                CGSize::new(col_w, caption.height),
            ));
        }
        self.layout_separator(w, h);
    }

    /// The hairline between this row and the next `DayPrefsRow` in the same card: drawn half-way
    /// down the gap the card leaves between them, from the text's leading edge to the card's
    /// trailing edge (the card clips it there), as the system list does.
    fn layout_separator(&self, w: CGFloat, h: CGFloat) {
        let iv = self.ivars();
        let Some(sv) = self.superview() else {
            iv.separator.setHidden(true);
            return;
        };
        let me = self.frame();
        let my_ptr = (self as *const Self).cast::<UIView>();
        let mut next_top: Option<CGFloat> = None;
        for v in sv.subviews().iter() {
            let p: *const UIView = Retained::as_ptr(&v);
            if p == my_ptr || v.downcast_ref::<PrefsRow>().is_none() {
                continue;
            }
            let top = v.frame().origin.y;
            if top >= me.origin.y + me.size.height - 0.5 && next_top.is_none_or(|t| top < t) {
                next_top = Some(top);
            }
        }
        let Some(top) = next_top else {
            iv.separator.setHidden(true);
            return;
        };
        let gap = top - (me.origin.y + me.size.height);
        // SAFETY: the trait collection is read on the main thread for the view's own screen.
        let scale = unsafe { self.traitCollection().displayScale() };
        let thickness = if scale > 0.0 { 1.0 / scale } else { 0.5 };
        let card_w = sv.bounds().size.width;
        let (x, width) = if self.rtl() {
            (-me.origin.x, (w - INSET + me.origin.x).max(0.0))
        } else {
            (INSET, (card_w - me.origin.x - INSET).max(0.0))
        };
        iv.separator.setFrame(CGRect::new(
            CGPoint::new(x, h + (gap / 2.0).round() - thickness / 2.0),
            CGSize::new(width, thickness),
        ));
        iv.separator.setHidden(false);
    }
}

/// Show `s` in the label, or hide the label when there is nothing to show.
fn set_optional_text(l: &UILabel, s: Option<&str>) {
    match s {
        Some(s) if !s.is_empty() => {
            l.setText(Some(&NSString::from_str(s)));
            l.setHidden(false);
        }
        _ => {
            l.setText(None);
            l.setHidden(true);
        }
    }
}

/// An SF Symbol at `point_size`, semibold: the chevron and external-link glyphs.
fn glyph(name: &str, point_size: CGFloat) -> Option<Retained<UIImage>> {
    let config = UIImageSymbolConfiguration::configurationWithPointSize_weight(
        point_size,
        UIImageSymbolWeight::Semibold,
    );
    UIImage::systemImageNamed_withConfiguration(&NSString::from_str(name), Some(&config))
}

/// The title color for a tone and enabled state (`tint` is the view's, for `Tone::Tint`).
fn title_color(tone: Tone, enabled: bool, tint: Option<Retained<UIColor>>) -> Retained<UIColor> {
    match (tone, enabled) {
        (_, false) => UIColor::tertiaryLabelColor(),
        (Tone::Plain, true) => UIColor::labelColor(),
        (Tone::Tint, true) => tint.unwrap_or_else(UIColor::systemBlueColor),
        (Tone::Destructive, true) => UIColor::systemRedColor(),
        (Tone::Note, true) => UIColor::secondaryLabelColor(),
    }
}

fn make_row(_backend: &mut Uikit, p: &RowProps, _id: NodeId) -> Retained<UIView> {
    // The toolkit realizes every view on the main thread; the marker cannot fail here.
    let mtm = MainThreadMarker::new().unwrap();
    let row = PrefsRow::new(mtm, p);
    Retained::into_super(row)
}

fn update_row(_backend: &mut Uikit, h: &Retained<UIView>, patch: &RowPatch) {
    let Some(row) = (**h).downcast_ref::<PrefsRow>() else {
        return;
    };
    match patch {
        RowPatch::Title(t) => row.set_title(t),
        RowPatch::Caption(c) => row.set_caption(c.as_deref()),
        RowPatch::Value(v) => row.set_value(v.as_deref()),
        RowPatch::Enabled(on) => row.set_enabled(*on),
    }
}

/// The texts' and accessories' natural size inside the proposed width; `RowLayout` adds the
/// Day child and the row floor.
fn measure_row(_backend: &mut Uikit, h: &Retained<UIView>, p: Proposal) -> Size {
    let w = p.width.unwrap_or(1.0e6);
    let s = h.sizeThatFits(CGSize::new(w, 1.0e6));
    Size::new(s.width.min(w).ceil(), s.height.ceil())
}

mod row_renderer {
    use super::*;
    day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
        kind: ROW_KIND, props: RowProps, patch: RowPatch,
        make: make_row, update: update_row, measure: measure_row);
}

// ---------------------------------------------------------------------------
// The grouped table: Day pieces
// ---------------------------------------------------------------------------

/// A section's header and footer text, as the table shows them.
#[derive(Clone, Debug, PartialEq, Default)]
struct SectionTexts {
    header: Option<String>,
    footer: Option<String>,
}

/// Realize-time payload of the table: one entry per section.
#[derive(Clone, Debug, PartialEq)]
struct TableProps {
    sections: Vec<SectionTexts>,
}

/// Changes after realize: localized header/footer texts re-resolving.
#[derive(Clone, Debug, PartialEq)]
enum TablePatch {
    Sections(Vec<SectionTexts>),
}

/// Realize-time payload of a cell: what the cell configures itself from, and where in the table
/// it sits.
#[derive(Clone, Debug, PartialEq)]
struct CellProps {
    section: usize,
    row: usize,
    title: String,
    caption: Option<String>,
    value: Option<String>,
    image: Option<CellImage>,
    /// The image's side (`GROUP_ICON`, `ABOUT_ICON`) and corner radius.
    image_size: f64,
    image_radius: f64,
    /// A template image takes the tint; a raster (an app icon) keeps its colors.
    image_template: bool,
    tone: Tone,
    accessory: Accessory,
    /// Whether the row highlights on touch and reports a tap.
    tappable: bool,
    enabled: bool,
    host: Host,
    /// Drawn on the grouped background rather than in a card (the About header, a footer line).
    clear: bool,
}

/// Changes after realize.
#[derive(Clone, Debug, PartialEq)]
enum CellPatch {
    Title(String),
    Caption(Option<String>),
    Value(Option<String>),
    Enabled(bool),
}

/// One cell of the grouped table: a Day node whose native view the cell adopts.
struct GroupedCell {
    title: TextSource,
    caption: Option<TextSource>,
    value: Option<TextSource>,
    image: Option<CellImage>,
    image_size: f64,
    image_radius: f64,
    image_template: bool,
    tone: Tone,
    accessory: Accessory,
    gate: Option<Rc<dyn Fn() -> bool>>,
    child: Option<AnyPiece>,
    host: Host,
    tap: Option<Rc<dyn Fn()>>,
    clear: bool,
    id: Option<String>,
    /// Set by the table when it builds its cells.
    section: usize,
    row: usize,
}

impl GroupedCell {
    fn new(title: TextSource) -> Self {
        GroupedCell {
            title,
            caption: None,
            value: None,
            image: None,
            image_size: GROUP_ICON,
            image_radius: 0.0,
            image_template: true,
            tone: Tone::Plain,
            accessory: Accessory::None,
            gate: None,
            child: None,
            host: Host::Empty,
            tap: None,
            clear: false,
            id: None,
            section: 0,
            row: 0,
        }
    }
    /// A cell whose whole content is a Day piece (the About header, a footer line): clear
    /// background, no texts.
    fn filled(piece: AnyPiece) -> Self {
        let mut c = GroupedCell::new(TextSource::Static(String::new()));
        c.child = Some(piece);
        c.host = Host::Fill;
        c.clear = true;
        c
    }
    /// The cell a row of the declaration becomes.
    fn from_row(inst: Rc<Installed>, r: &Row) -> Self {
        let parts = row_parts(inst, r);
        let mut c = GroupedCell::new(r.title.clone());
        c.caption = r.caption.clone();
        c.value = parts.value;
        c.tone = parts.tone;
        c.accessory = parts.accessory;
        c.gate = parts.gate;
        c.host = parts.host;
        c.child = parts.control;
        c.tap = parts.tap;
        c.id = parts.row_id;
        c
    }
    fn icon(mut self, icon: Option<&GroupIcon>) -> Self {
        self.image = icon.map(|i| match i {
            GroupIcon::Image(name) => CellImage::Named(name.as_str().to_owned()),
            GroupIcon::Symbol(sym) => CellImage::Symbol(day_spec::sf_symbol_name(*sym).to_owned()),
        });
        self
    }
    fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
    fn tap(mut self, f: impl Fn() + 'static) -> Self {
        self.tap = Some(Rc::new(f));
        self
    }
}

impl Piece for GroupedCell {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let GroupedCell {
            title,
            caption,
            value,
            image,
            image_size,
            image_radius,
            image_template,
            tone,
            accessory,
            gate,
            child,
            host,
            tap,
            clear,
            id,
            section,
            row,
        } = self;
        let props = CellProps {
            section,
            row,
            title: title.initial(),
            caption: caption.as_ref().map(TextSource::initial),
            value: value.as_ref().map(TextSource::initial),
            image,
            image_size,
            image_radius,
            image_template,
            tone,
            accessory,
            tappable: tap.is_some(),
            enabled: gate.as_ref().is_none_or(|f| day_reactive::untrack(|| f())),
            host,
            clear,
        };
        let node = cx.native(
            CELL_KIND,
            &props,
            Rc::new(CellLayout { host }),
            Flex {
                grow_w: true,
                ..Default::default()
            },
            Boundary::No,
        );
        if let Some(id) = id {
            with_tree(|t| t.set_id(node, id));
        }
        if let Some(child) = child {
            cx.under(node, |cx| {
                let _ = child.build(cx);
            });
        }
        // The cell's selection (`didSelectRowAtIndexPath:`) and a dayscript `tap:` both arrive
        // as `Event::Tap` on this node.
        if let Some(tap) = tap {
            cx.on(node, move |ev| {
                if matches!(ev, Event::Tap(_)) {
                    tap();
                }
            });
        }
        bind_text(node, &title, CellPatch::Title);
        if let Some(c) = &caption {
            bind_text(node, c, |s| CellPatch::Caption(Some(s)));
        }
        if let Some(v) = &value {
            bind_text(node, v, |s| CellPatch::Value(Some(s)));
        }
        bind_gate(node, props.enabled, gate, CellPatch::Enabled);
        node
    }
}

/// One section of the grouped table.
struct GroupedSection {
    header: Option<TextSource>,
    footer: Option<TextSource>,
    cells: Vec<GroupedCell>,
}

impl GroupedSection {
    fn new(cells: Vec<GroupedCell>) -> Self {
        GroupedSection {
            header: None,
            footer: None,
            cells,
        }
    }
    fn texts(&self) -> SectionTexts {
        SectionTexts {
            header: self.header.as_ref().map(TextSource::initial),
            footer: self.footer.as_ref().map(TextSource::initial),
        }
    }
}

/// The inset grouped table: the page.
struct Grouped {
    sections: Vec<GroupedSection>,
    /// As tall as its rows inside a sheet's own `scroll` (`group_body`), rather than filling
    /// the height it is offered.
    hug: bool,
}

impl Piece for Grouped {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let texts: Vec<SectionTexts> = self.sections.iter().map(GroupedSection::texts).collect();
        let node = cx.native(
            TABLE_KIND,
            &TableProps {
                sections: texts.clone(),
            },
            Rc::new(TableLayout),
            Flex {
                grow_w: true,
                grow_h: !self.hug,
                ..Default::default()
            },
            // Not a layout boundary: a self-sizing table's height follows its cells, and the
            // scroll around it must learn of the change.
            Boundary::No,
        );
        let mut sources: Vec<(Option<TextSource>, Option<TextSource>)> = Vec::new();
        for (si, s) in self.sections.into_iter().enumerate() {
            sources.push((s.header.clone(), s.footer.clone()));
            for (ri, mut c) in s.cells.into_iter().enumerate() {
                c.section = si;
                c.row = ri;
                cx.under(node, |cx| {
                    let _ = c.build(cx);
                });
            }
        }
        // Localized headers and footers re-resolve on a locale change: one reaction over all
        // of them, re-sent whole (the table reloads its section chrome).
        let dynamic = sources.iter().any(|(h, f)| {
            matches!(h, Some(TextSource::Dyn(_))) || matches!(f, Some(TextSource::Dyn(_)))
        });
        if dynamic {
            day_reactive::bind_seeded(
                texts,
                move || {
                    sources
                        .iter()
                        .map(|(h, f)| SectionTexts {
                            header: h.as_ref().map(TextSource::initial),
                            footer: f.as_ref().map(TextSource::initial),
                        })
                        .collect::<Vec<_>>()
                },
                move |v| {
                    with_tree(|t| t.patch(node, Box::new(TablePatch::Sections(v.clone())), false));
                },
            );
        }
        node
    }
}

/// The table fills what the page offers (with no height offered, inside a Day `scroll`, it is
/// as tall as its content); the cells are measured against its width and given a frame of
/// their Day child's size at the origin (the native cell positions the host view itself: at
/// the trailing edge as the accessory, under the texts, or as the content).
struct TableLayout;

impl Layout for TableLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, _children: &[RNode], p: Proposal) -> Size {
        if p.height.is_none() {
            return cx.measure_leaf(p);
        }
        Size::new(
            p.width.unwrap_or(NOMINAL_WIDTH),
            p.height.unwrap_or(ROW_PITCH),
        )
    }

    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        for &c in children {
            let s = cx.measure_child(c, Proposal::new(Some(bounds.size.width), None));
            cx.place_child(c, Rect::new(0.0, 0.0, s.width, s.height));
        }
    }
}

/// A cell's Day frame is its child's: natural for an accessory (a stretching one, a text field,
/// takes a share of the table), the content width for an inline or filling child, and the row
/// pitch for a controlless cell (a positive frame is what a dayscript's visibility check reads).
struct CellLayout {
    host: Host,
}

impl Layout for CellLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let table_w = p.width.unwrap_or(NOMINAL_WIDTH);
        match (self.host, children.first()) {
            (Host::Empty, _) | (_, None) => Size::new(table_w, ROW_PITCH),
            (Host::Accessory, Some(&c)) => {
                if cx.flex_of(c).grow_w {
                    let w = (table_w * FIELD_SHARE).max(FIELD_MIN);
                    let s = cx.measure_child(c, Proposal::new(Some(w), None));
                    Size::new(w, s.height)
                } else {
                    cx.measure_child(c, Proposal::UNCONSTRAINED)
                }
            }
            (Host::Inline | Host::Fill, Some(&c)) => {
                let w = (table_w - 2.0 * (CARD_INSET + CELL_PAD)).max(0.0);
                let s = cx.measure_child(c, Proposal::new(Some(w), None));
                Size::new(w, s.height)
            }
        }
    }

    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        if let Some(&c) = children.first() {
            cx.place_child(
                c,
                Rect::new(0.0, 0.0, bounds.size.width, bounds.size.height),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The grouped table: native views
// ---------------------------------------------------------------------------

struct SlotIvars {
    node: NodeId,
    host: Host,
    section: usize,
    row: usize,
    tappable: Cell<bool>,
    enabled: Cell<bool>,
    /// The cell, owned here until the table takes it over (then reached weakly: the cell holds
    /// this view in its hierarchy, and a strong reference back would be a cycle).
    pending: RefCell<Option<Retained<SettingsCell>>>,
    cell: RefCell<Option<Weak<SettingsCell>>>,
    table: RefCell<Option<Weak<SettingsTable>>>,
    /// The frame size Day last placed: the child's size.
    day_size: Cell<CGSize>,
}

define_class!(
    #[unsafe(super(UIView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DaySettingsSlot"]
    #[ivars = SlotIvars]
    struct Slot;

    impl Slot {
        /// Day places this view at the origin with the child's size; the cell positions it.
        /// A new size means a new row height for a child under or instead of the texts.
        #[unsafe(method(setFrame:))]
        fn set_frame(&self, frame: CGRect) {
            // SAFETY: plain super call of a UIView method on the main thread.
            let _: () = unsafe { msg_send![super(self), setFrame: frame] };
            let iv = self.ivars();
            let old = iv.day_size.get();
            if old.width == frame.size.width && old.height == frame.size.height {
                return;
            }
            iv.day_size.set(frame.size);
            if let Some(cell) = self.cell() {
                cell.setNeedsLayout();
            }
            if matches!(iv.host, Host::Inline | Host::Fill)
                && let Some(table) = self.table()
            {
                table.heights_changed();
            }
        }
    }
);

impl Slot {
    fn new(
        mtm: MainThreadMarker,
        node: NodeId,
        p: &CellProps,
        cell: Retained<SettingsCell>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SlotIvars {
            node,
            host: p.host,
            section: p.section,
            row: p.row,
            tappable: Cell::new(p.tappable),
            enabled: Cell::new(p.enabled),
            pending: RefCell::new(Some(cell)),
            cell: RefCell::new(None),
            table: RefCell::new(None),
            day_size: Cell::new(CGSize::new(0.0, 0.0)),
        });
        // SAFETY: designated UIView initializer on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }

    fn cell(&self) -> Option<Retained<SettingsCell>> {
        let iv = self.ivars();
        if let Some(c) = iv.pending.borrow().as_ref() {
            return Some(c.clone());
        }
        iv.cell.borrow().as_ref().and_then(Weak::load)
    }

    fn table(&self) -> Option<Retained<SettingsTable>> {
        self.ivars().table.borrow().as_ref().and_then(Weak::load)
    }

    fn tappable(&self) -> bool {
        let iv = self.ivars();
        iv.tappable.get() && iv.enabled.get()
    }
}

struct CellIvars {
    host: Host,
    props: RefCell<CellProps>,
    /// `Host::Inline`: the list content view laid out by hand above the slot (the cell's own
    /// content configuration would pin it to every edge).
    content: RefCell<Option<Retained<UIListContentView>>>,
    /// The external-link glyph, when the row is a link.
    external: RefCell<Option<Retained<UIImageView>>>,
    slot: RefCell<Option<Weak<Slot>>>,
}

define_class!(
    #[unsafe(super(UITableViewCell))]
    #[thread_kind = MainThreadOnly]
    #[name = "DaySettingsCell"]
    #[ivars = CellIvars]
    struct SettingsCell;

    impl SettingsCell {
        #[unsafe(method(layoutSubviews))]
        fn layout_subviews(&self) {
            // SAFETY: plain super call of a UIView method on the main thread.
            let _: () = unsafe { msg_send![super(self), layoutSubviews] };
            self.layout_slot();
        }

        #[unsafe(method(tintColorDidChange))]
        fn tint_color_did_change(&self) {
            // SAFETY: plain super call of a UIView method on the main thread.
            let _: () = unsafe { msg_send![super(self), tintColorDidChange] };
            if self.ivars().props.borrow().tone == Tone::Tint {
                self.configure();
            }
        }
    }
);

impl SettingsCell {
    fn new(mtm: MainThreadMarker, p: &CellProps) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(CellIvars {
            host: p.host,
            props: RefCell::new(p.clone()),
            content: RefCell::new(None),
            external: RefCell::new(None),
            slot: RefCell::new(None),
        });
        let reuse: Option<&NSString> = None;
        // SAFETY: the designated UITableViewCell initializer on a freshly allocated instance.
        let this: Retained<Self> = unsafe {
            msg_send![super(this), initWithStyle: UITableViewCellStyle::Default, reuseIdentifier: reuse]
        };
        this.configure();
        this
    }

    fn slot(&self) -> Option<Retained<Slot>> {
        self.ivars().slot.borrow().as_ref().and_then(Weak::load)
    }

    /// Adopt the Day node's view: as the accessory, under the texts, as the content, or (no
    /// child) hidden inside the content view so it has a window like every other node's view.
    fn adopt(&self, slot: &Retained<Slot>) {
        *self.ivars().slot.borrow_mut() = Some(Weak::from_retained(slot));
        match self.ivars().host {
            Host::Accessory => {
                self.setAccessoryView(Some(slot));
            }
            Host::Inline | Host::Fill => {
                self.contentView().addSubview(slot);
            }
            Host::Empty => {
                slot.setHidden(true);
                slot.setUserInteractionEnabled(false);
                self.contentView().addSubview(slot);
            }
        }
        self.setNeedsLayout();
    }

    /// The texts, image, colors and accessories from the props: a content configuration for
    /// the cell (or, inline, for the hand-placed content view), the system chevron, the glyph,
    /// the selection style.
    fn configure(&self) {
        let mtm = self.mtm();
        let p = self.ivars().props.borrow().clone();
        if self.ivars().host != Host::Fill {
            let cfg = if p.caption.is_some() {
                UIListContentConfiguration::subtitleCellConfiguration(mtm)
            } else if p.value.is_some() {
                UIListContentConfiguration::valueCellConfiguration(mtm)
            } else {
                self.defaultContentConfiguration()
            };
            cfg.setText(Some(&NSString::from_str(&p.title)));
            let secondary = p.caption.as_deref().or(p.value.as_deref());
            cfg.setSecondaryText(secondary.map(NSString::from_str).as_deref());
            if let Some(img) = &p.image {
                let image = match img {
                    CellImage::Named(name) => day_uikit::image_named(name),
                    CellImage::Symbol(name) => UIImage::systemImageNamed(&NSString::from_str(name)),
                };
                let image = image.map(|i| {
                    if p.image_template {
                        i.imageWithRenderingMode(UIImageRenderingMode::AlwaysTemplate)
                    } else {
                        i
                    }
                });
                cfg.setImage(image.as_deref());
                let ip = cfg.imageProperties();
                let side = CGSize::new(p.image_size, p.image_size);
                ip.setMaximumSize(side);
                ip.setReservedLayoutSize(side);
                if p.image_radius > 0.0 {
                    ip.setCornerRadius(p.image_radius);
                }
            }
            let tp = cfg.textProperties();
            tp.setColor(&title_color(p.tone, p.enabled, self.tintColor()));
            match p.tone {
                Tone::Destructive => tp.setAlignment(UIListContentTextAlignment::Center),
                Tone::Note => {
                    // SAFETY: the text-style constant is an immutable static UIKit exports.
                    let font = unsafe {
                        UIFont::preferredFontForTextStyle(objc2_ui_kit::UIFontTextStyleFootnote)
                    };
                    tp.setFont(&font);
                    tp.setNumberOfLines(0);
                }
                Tone::Plain | Tone::Tint => {}
            }
            cfg.secondaryTextProperties()
                .setColor(&UIColor::secondaryLabelColor());
            if self.ivars().host == Host::Inline {
                let mut content = self.ivars().content.borrow_mut();
                match content.as_ref() {
                    Some(v) => v.setConfiguration(&cfg),
                    None => {
                        let v = UIListContentView::initWithConfiguration(
                            UIListContentView::alloc(mtm),
                            &cfg,
                        );
                        self.contentView().addSubview(&v);
                        *content = Some(v);
                    }
                }
            } else {
                self.setContentConfiguration(Some(ProtocolObject::from_ref(&*cfg)));
            }
        }
        self.setAccessoryType(if p.accessory == Accessory::Chevron {
            UITableViewCellAccessoryType::DisclosureIndicator
        } else {
            UITableViewCellAccessoryType::None
        });
        if p.accessory == Accessory::External && p.host == Host::Empty {
            let mut ext = self.ivars().external.borrow_mut();
            if ext.is_none() {
                let iv = UIImageView::initWithImage(
                    UIImageView::alloc(mtm),
                    glyph("arrow.up.right", 14.0).as_deref(),
                );
                // SAFETY: plain property writes on a view this cell owns, on the main thread.
                unsafe {
                    iv.setTintColor(Some(&UIColor::tertiaryLabelColor()));
                    iv.sizeToFit();
                }
                self.setAccessoryView(Some(&iv));
                *ext = Some(iv);
            }
        }
        self.setSelectionStyle(if p.tappable && p.enabled {
            UITableViewCellSelectionStyle::Default
        } else {
            UITableViewCellSelectionStyle::None
        });
        if p.clear {
            self.setBackgroundConfiguration(Some(&UIBackgroundConfiguration::clearConfiguration(
                mtm,
            )));
        }
        self.setNeedsLayout();
    }

    fn apply(&self, patch: &CellPatch) {
        {
            let mut p = self.ivars().props.borrow_mut();
            match patch {
                CellPatch::Title(t) => p.title = t.clone(),
                CellPatch::Caption(c) => p.caption = c.clone(),
                CellPatch::Value(v) => p.value = v.clone(),
                CellPatch::Enabled(on) => p.enabled = *on,
            }
        }
        if let (CellPatch::Enabled(on), Some(slot)) = (patch, self.slot()) {
            slot.ivars().enabled.set(*on);
        }
        self.configure();
        // A text change can change the row's height; the table re-asks.
        if !matches!(patch, CellPatch::Enabled(_))
            && let Some(table) = self.slot().and_then(|s| s.table())
        {
            table.heights_changed();
        }
    }

    /// The texts' height inside `width`, for a row whose child sits under them.
    fn content_height(&self, width: CGFloat) -> CGFloat {
        let Some(content) = self.ivars().content.borrow().clone() else {
            return 0.0;
        };
        let (required, fitting) = (
            objc2_ui_kit::UILayoutPriorityRequired,
            objc2_ui_kit::UILayoutPriorityFittingSizeLevel,
        );
        let s = content
            .systemLayoutSizeFittingSize_withHorizontalFittingPriority_verticalFittingPriority(
                CGSize::new(width, 0.0),
                required,
                fitting,
            );
        if s.height > 1.0 {
            s.height.ceil()
        } else {
            ROW_PITCH
        }
    }

    /// The row height the table asks for: the system's for a cell its content configuration
    /// sizes, the texts plus the child for an inline one, the child plus padding for a filled
    /// one.
    fn row_height(&self, table_width: CGFloat) -> CGFloat {
        let child = self
            .slot()
            .map(|s| s.ivars().day_size.get())
            .unwrap_or(CGSize::new(0.0, 0.0));
        match self.ivars().host {
            // SAFETY: an immutable static UIKit exports.
            Host::Empty | Host::Accessory => unsafe { objc2_ui_kit::UITableViewAutomaticDimension },
            Host::Inline => {
                let w = (table_width - 2.0 * CARD_INSET).max(0.0);
                self.content_height(w) + child.height + INLINE_BOTTOM
            }
            Host::Fill => (child.height + 2.0 * FILL_VPAD).max(ROW_PITCH),
        }
    }

    /// Place the hand-laid content view and the slot. An accessory slot is the cell's to
    /// position; the others keep the size Day gave them (the child is laid out to it) and sit
    /// centered in the content width.
    fn layout_slot(&self) {
        let cv = self.contentView();
        let b = cv.bounds();
        let Some(slot) = self.slot() else {
            return;
        };
        let size = slot.ivars().day_size.get();
        let x = ((b.size.width - size.width) / 2.0).max(0.0).round();
        match self.ivars().host {
            Host::Inline => {
                let ch = self.content_height(b.size.width);
                if let Some(content) = self.ivars().content.borrow().as_ref() {
                    content.setFrame(CGRect::new(
                        CGPoint::new(0.0, 0.0),
                        CGSize::new(b.size.width, ch),
                    ));
                }
                slot.setFrame(CGRect::new(CGPoint::new(x, ch), size));
            }
            Host::Fill => {
                let y = ((b.size.height - size.height) / 2.0).max(0.0).round();
                slot.setFrame(CGRect::new(CGPoint::new(x, y), size));
            }
            Host::Accessory | Host::Empty => {}
        }
    }
}

/// A filed row: the Day node's view and the cell that adopted it.
type Filed = (Retained<Slot>, Retained<SettingsCell>);

struct TableDataIvars {
    /// The cells by section and row, in order; the table owns them through here.
    sections: RefCell<Vec<Vec<Filed>>>,
    texts: RefCell<Vec<SectionTexts>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DaySettingsTableData"]
    #[ivars = TableDataIvars]
    struct TableData;

    unsafe impl NSObjectProtocol for TableData {}
    unsafe impl UIScrollViewDelegate for TableData {}

    unsafe impl UITableViewDataSource for TableData {
        #[unsafe(method(numberOfSectionsInTableView:))]
        fn number_of_sections(&self, _tv: &UITableView) -> NSInteger {
            let iv = self.ivars();
            iv.texts.borrow().len().max(iv.sections.borrow().len()) as NSInteger
        }

        #[unsafe(method(tableView:numberOfRowsInSection:))]
        fn rows_in_section(&self, _tv: &UITableView, section: NSInteger) -> NSInteger {
            self.ivars()
                .sections
                .borrow()
                .get(section.max(0) as usize)
                .map_or(0, |s| s.len() as NSInteger)
        }

        #[unsafe(method_id(tableView:cellForRowAtIndexPath:))]
        fn cell_for_row(&self, _tv: &UITableView, ip: &NSIndexPath) -> Retained<UITableViewCell> {
            match self.at(ip) {
                Some((_, cell)) => Retained::into_super(cell),
                None => {
                    let reuse: Option<&NSString> = None;
                    UITableViewCell::initWithStyle_reuseIdentifier(
                        UITableViewCell::alloc(self.mtm()),
                        UITableViewCellStyle::Default,
                        reuse,
                    )
                }
            }
        }

        #[unsafe(method_id(tableView:titleForHeaderInSection:))]
        fn title_for_header(
            &self,
            _tv: &UITableView,
            section: NSInteger,
        ) -> Option<Retained<NSString>> {
            self.ivars()
                .texts
                .borrow()
                .get(section.max(0) as usize)
                .and_then(|t| t.header.as_deref())
                .filter(|s| !s.is_empty())
                .map(NSString::from_str)
        }

        #[unsafe(method_id(tableView:titleForFooterInSection:))]
        fn title_for_footer(
            &self,
            _tv: &UITableView,
            section: NSInteger,
        ) -> Option<Retained<NSString>> {
            self.ivars()
                .texts
                .borrow()
                .get(section.max(0) as usize)
                .and_then(|t| t.footer.as_deref())
                .filter(|s| !s.is_empty())
                .map(NSString::from_str)
        }
    }

    unsafe impl UITableViewDelegate for TableData {
        #[unsafe(method(tableView:heightForRowAtIndexPath:))]
        fn height_for_row(&self, tv: &UITableView, ip: &NSIndexPath) -> CGFloat {
            match self.at(ip) {
                Some((_, cell)) => cell.row_height(tv.bounds().size.width),
                None => ROW_PITCH,
            }
        }

        #[unsafe(method(tableView:shouldHighlightRowAtIndexPath:))]
        fn should_highlight(&self, _tv: &UITableView, ip: &NSIndexPath) -> objc2::runtime::Bool {
            objc2::runtime::Bool::new(self.at(ip).is_some_and(|(slot, _)| slot.tappable()))
        }

        /// The standard flow: the row highlighted on touch, deselects as the page it opens
        /// pushes, and the Day node behind it gets the tap.
        #[unsafe(method(tableView:didSelectRowAtIndexPath:))]
        fn did_select(&self, tv: &UITableView, ip: &NSIndexPath) {
            tv.deselectRowAtIndexPath_animated(ip, true);
            let Some((slot, _)) = self.at(ip) else {
                return;
            };
            if !slot.tappable() {
                return;
            }
            let node = slot.ivars().node;
            log::debug!(
                "day-piece-settings: didSelectRowAtIndexPath section={} row={} -> Tap on node {}",
                ip.section(),
                ip.row(),
                node.0
            );
            day_spec::ffi_guard::contain((), || {
                day_uikit::emit(node, Event::Tap(day_spec::Point::ZERO));
            });
        }
    }
);

impl TableData {
    fn new(mtm: MainThreadMarker, texts: Vec<SectionTexts>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TableDataIvars {
            sections: RefCell::new(Vec::new()),
            texts: RefCell::new(texts),
        });
        // SAFETY: NSObject's designated initializer on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }

    fn at(&self, ip: &NSIndexPath) -> Option<Filed> {
        let (section, row) = (ip.section().max(0) as usize, ip.row().max(0) as usize);
        self.ivars()
            .sections
            .borrow()
            .get(section)
            .and_then(|s| s.get(row))
            .cloned()
    }

    /// File a cell under its section, in row order.
    fn insert(&self, slot: Retained<Slot>, cell: Retained<SettingsCell>) {
        let (section, row) = (slot.ivars().section, slot.ivars().row);
        let mut sections = self.ivars().sections.borrow_mut();
        while sections.len() <= section {
            sections.push(Vec::new());
        }
        let list = &mut sections[section];
        let at = list
            .iter()
            .position(|(s, _)| s.ivars().row > row)
            .unwrap_or(list.len());
        list.insert(at, (slot, cell));
    }
}

struct TableIvars {
    node: NodeId,
    data: Retained<TableData>,
    /// A child under or instead of the texts changed size: re-ask every row height at the next
    /// layout, where UIKit expects the table to change.
    heights_dirty: Cell<bool>,
    /// Measured with no height proposal (inside a Day `scroll`, where another page embeds the
    /// About content): the table is as tall as its content and never scrolls itself.
    self_sizing: Cell<bool>,
    /// The content height last reported to Day while self-sizing.
    reported: Cell<CGFloat>,
    /// Inside `self_size`: the layout pass that IS the report, not one to report.
    measuring: Cell<bool>,
}

define_class!(
    #[unsafe(super(UITableView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DaySettingsTable"]
    #[ivars = TableIvars]
    struct SettingsTable;

    impl SettingsTable {
        /// day-core mounts each cell node's view as a direct subview (generic `addSubview`);
        /// the table files it under its section and lets the cell adopt it instead.
        #[unsafe(method(didAddSubview:))]
        fn did_add_subview(&self, subview: &UIView) {
            // SAFETY: plain super call of a UIView method on the main thread.
            let _: () = unsafe { msg_send![super(self), didAddSubview: subview] };
            if let Some(slot) = subview.downcast_ref::<Slot>() {
                self.register(slot);
            }
        }

        #[unsafe(method(layoutSubviews))]
        fn layout_subviews(&self) {
            if self.ivars().heights_dirty.replace(false) {
                self.reloadData();
            }
            // SAFETY: plain super call of a UIView method on the main thread.
            let _: () = unsafe { msg_send![super(self), layoutSubviews] };
            // A self-sizing table whose content grew or shrank after it was measured (a child
            // placed under its texts, a relabel) asks Day for a new measure. From the next
            // run-loop turn: this can run inside Day's own layout pass, which holds the tree
            // (a task's first poll is synchronous; the sleep is what defers it).
            let iv = self.ivars();
            if iv.self_sizing.get() && !iv.measuring.get() {
                let h = self.content_height().ceil();
                if (h - iv.reported.get()).abs() > 0.5 {
                    iv.reported.set(h);
                    let node = iv.node;
                    day_core::task(async move {
                        day_core::sleep(0).await;
                        day_core::invalidate_size(day_core::id_to_rnode(node));
                    });
                }
            }
        }
    }
);

impl SettingsTable {
    fn new(mtm: MainThreadMarker, p: &TableProps, node: NodeId) -> Retained<Self> {
        let data = TableData::new(mtm, p.sections.clone());
        let this = Self::alloc(mtm).set_ivars(TableIvars {
            node,
            data: data.clone(),
            heights_dirty: Cell::new(false),
            self_sizing: Cell::new(false),
            reported: Cell::new(0.0),
            measuring: Cell::new(false),
        });
        // SAFETY: UITableView's designated initializer on a freshly allocated instance.
        let this: Retained<Self> = unsafe {
            msg_send![super(this), initWithFrame: CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0)), style: UITableViewStyle::InsetGrouped]
        };
        // SAFETY: the data source and delegate are retained by the table's ivars for its
        // whole life, which is what the weak properties require.
        unsafe {
            this.setDataSource(Some(ProtocolObject::from_ref(&*data)));
            this.setDelegate(Some(ProtocolObject::from_ref(&*data)));
            this.setKeyboardDismissMode(UIScrollViewKeyboardDismissMode::Interactive);
        }
        this
    }

    /// Take a freshly mounted cell node's view: file its cell, hand the view to the cell, and
    /// show the row.
    fn register(&self, slot: &Slot) {
        let Some(cell) = slot.ivars().pending.borrow_mut().take() else {
            return;
        };
        *slot.ivars().cell.borrow_mut() = Some(Weak::from_retained(&cell));
        *slot.ivars().table.borrow_mut() = Some(Weak::from_retained(&self.retain()));
        let slot = slot.retain();
        slot.removeFromSuperview();
        cell.adopt(&slot);
        self.ivars().data.insert(slot, cell);
        self.reloadData();
    }

    fn heights_changed(&self) {
        self.ivars().heights_dirty.set(true);
        self.setNeedsLayout();
    }

    fn set_texts(&self, texts: Vec<SectionTexts>) {
        *self.ivars().data.ivars().texts.borrow_mut() = texts;
        self.reloadData();
    }

    /// The height of every section with its chrome, as laid out.
    fn content_height(&self) -> CGFloat {
        let inset = self.adjustedContentInset();
        self.contentSize().height + inset.top + inset.bottom
    }

    /// Measured with no height to fill: the table becomes its content's height, laid out at
    /// `width` (every row realized, so the heights are the real ones, not estimates), and
    /// leaves scrolling to the Day `scroll` around it.
    fn self_size(&self, width: CGFloat) -> CGFloat {
        let iv = self.ivars();
        if !iv.self_sizing.replace(true) {
            self.setScrollEnabled(false);
        }
        // Tall enough for every row to be on screen while the heights are taken.
        self.setFrame(CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(width, 100_000.0),
        ));
        iv.measuring.set(true);
        self.layoutIfNeeded();
        iv.measuring.set(false);
        let h = self.content_height().ceil();
        iv.reported.set(h);
        h
    }
}

fn make_table(_backend: &mut Uikit, p: &TableProps, id: NodeId) -> Retained<UIView> {
    // The toolkit realizes every view on the main thread; the marker cannot fail here.
    let mtm = MainThreadMarker::new().unwrap();
    let table: Retained<UITableView> = Retained::into_super(SettingsTable::new(mtm, p, id));
    let scroll: Retained<objc2_ui_kit::UIScrollView> = Retained::into_super(table);
    Retained::into_super(scroll)
}

fn update_table(_backend: &mut Uikit, h: &Retained<UIView>, patch: &TablePatch) {
    let Some(table) = (**h).downcast_ref::<SettingsTable>() else {
        return;
    };
    match patch {
        TablePatch::Sections(texts) => table.set_texts(texts.clone()),
    }
}

/// What `TableLayout` asks when no height is proposed: the content's own height at the width.
fn measure_table(_backend: &mut Uikit, h: &Retained<UIView>, p: Proposal) -> Size {
    let w = p.width.unwrap_or(NOMINAL_WIDTH);
    match (p.height, (**h).downcast_ref::<SettingsTable>()) {
        (Some(h), _) => Size::new(w, h),
        (None, Some(table)) => Size::new(w, table.self_size(w)),
        (None, None) => Size::new(w, ROW_PITCH),
    }
}

mod table_renderer {
    use super::*;
    day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
        kind: TABLE_KIND, props: TableProps, patch: TablePatch,
        make: make_table, update: update_table, measure: measure_table);
}

fn make_cell(_backend: &mut Uikit, p: &CellProps, id: NodeId) -> Retained<UIView> {
    // The toolkit realizes every view on the main thread; the marker cannot fail here.
    let mtm = MainThreadMarker::new().unwrap();
    let cell = SettingsCell::new(mtm, p);
    let slot = Slot::new(mtm, id, p, cell);
    Retained::into_super(slot)
}

fn update_cell(_backend: &mut Uikit, h: &Retained<UIView>, patch: &CellPatch) {
    let Some(slot) = (**h).downcast_ref::<Slot>() else {
        return;
    };
    if let Some(cell) = slot.cell() {
        cell.apply(patch);
    }
}

/// A container measures through its layout; this is only reached for a cell with no child.
fn measure_cell(_backend: &mut Uikit, _h: &Retained<UIView>, p: Proposal) -> Size {
    Size::new(p.width.unwrap_or(NOMINAL_WIDTH), ROW_PITCH)
}

mod cell_renderer {
    use super::*;
    day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
        kind: CELL_KIND, props: CellProps, patch: CellPatch,
        make: make_cell, update: update_cell, measure: measure_cell);
}

// ---------------------------------------------------------------------------
// Native page hooks (docs/settings.md): `None` = the composed page.
// ---------------------------------------------------------------------------

/// A group's sections as table sections, available rows only, empty sections dropped.
fn group_sections(inst: &Rc<Installed>, g: &Group) -> Vec<GroupedSection> {
    g.sections
        .iter()
        .filter_map(|s| {
            let cells: Vec<GroupedCell> = s
                .rows
                .iter()
                .filter(|r| r.is_available())
                .map(|r| GroupedCell::from_row(inst.clone(), r))
                .collect();
            if cells.is_empty() {
                return None;
            }
            let mut sec = GroupedSection::new(cells);
            sec.header = s.title.clone();
            sec.footer = s.footer.clone();
            Some(sec)
        })
        .collect()
}

/// The root list: the About header in its own section, then one section of group rows with
/// their icons (one group: its rows inline under the header, as the composed root does).
pub(super) fn root_page(inst: Rc<Installed>) -> Option<AnyPiece> {
    let (about, groups) = root_entries(&inst);
    let mut sections: Vec<GroupedSection> = Vec::new();
    if let Some(a) = about {
        let open = a.open.clone();
        let mut cell = GroupedCell::new(a.title.clone())
            .icon(a.icon.as_ref())
            .with_id("settings-about")
            .tap(move || open());
        cell.caption = a.subtitle.clone();
        cell.image_size = ABOUT_ICON;
        cell.image_radius = 9.0;
        cell.image_template = false;
        cell.accessory = Accessory::Chevron;
        sections.push(GroupedSection::new(vec![cell]));
    }
    let visible = inst.visible_groups();
    if let [g] = visible.as_slice() {
        if g.custom.is_some() {
            return None;
        }
        sections.extend(group_sections(&inst, g));
    } else {
        let cells: Vec<GroupedCell> = groups
            .into_iter()
            .map(|e| {
                let open = e.open.clone();
                let mut cell = GroupedCell::new(e.title.clone())
                    .icon(e.icon.as_ref())
                    .with_id(format!("prefs-group-{}", e.id))
                    .tap(move || open());
                cell.caption = e.subtitle.clone();
                cell.accessory = Accessory::Chevron;
                cell
            })
            .collect();
        if !cells.is_empty() {
            sections.push(GroupedSection::new(cells));
        }
    }
    Some(
        Grouped {
            sections,
            hug: false,
        }
        .id("settings-root")
        .any(),
    )
}

/// One group's page: its sections as the table's. A group with a bespoke page piece keeps the
/// composed page, where that piece has the room it was designed for.
pub(super) fn group_page(inst: Rc<Installed>, g: &Group) -> Option<AnyPiece> {
    group_table(&inst, g, false)
}

/// One group's table as tall as its rows, for a sheet's own `scroll`.
pub(super) fn group_body(inst: Rc<Installed>, g: &Group) -> Option<AnyPiece> {
    group_table(&inst, g, true)
}

fn group_table(inst: &Rc<Installed>, g: &Group, hug: bool) -> Option<AnyPiece> {
    if g.custom.is_some() {
        return None;
    }
    let sections = group_sections(inst, g);
    Some(
        Grouped { sections, hug }
            .id(format!("prefs-group-{}", g.id))
            .any(),
    )
}

/// The About page: the identity header on the grouped background, a section of link rows, a
/// section of Acknowledgements / Licenses rows, the copyright as a footer line. The ids are
/// the composed page's (`about-page`, `about-name`, `about-version`, `about-tagline`,
/// `about-link-<n>`, `about-privacy`, `about-copyright`).
pub(super) fn about_page(inst: Rc<Installed>) -> Option<AnyPiece> {
    let about = inst.about.as_ref()?;
    let mut sections: Vec<GroupedSection> = Vec::new();

    let mut head: Vec<AnyPiece> = Vec::new();
    if let Some(n) = about.icon_name() {
        head.push(image(n.clone()).frame(96.0, 96.0).corner_radius(20.0).any());
    }
    head.push(
        label(about.display_name())
            .font(day_spec::Font::Title2)
            .bold()
            .id("about-name")
            .any(),
    );
    if let Some(v) = about.version_line() {
        head.push(label(v).secondary().id("about-version").any());
    }
    if let Some(t) = about.tagline_text() {
        head.push(
            label(t)
                .align(day_spec::props::TextAlign::Center)
                .id("about-tagline")
                .any(),
        );
    }
    sections.push(GroupedSection::new(vec![GroupedCell::filled(
        column(head)
            .spacing(6.0)
            .align(HAlign::Center)
            .grow_w()
            .any(),
    )]));

    let mut links: Vec<GroupedCell> = about
        .links()
        .iter()
        .enumerate()
        .map(|(i, (t, url))| {
            let url = url.clone();
            let mut cell = GroupedCell::new(t.clone())
                .with_id(format!("about-link-{i}"))
                .tap(move || day_core::open_url(&url));
            cell.tone = Tone::Tint;
            cell.accessory = Accessory::External;
            cell
        })
        .collect();
    if let Some(p) = about.privacy_url() {
        let url = p.to_string();
        let mut cell = GroupedCell::new(day_fluent::tr("day-about-privacy").into_text())
            .with_id("about-privacy")
            .tap(move || day_core::open_url(&url));
        cell.tone = Tone::Tint;
        cell.accessory = Accessory::External;
        links.push(cell);
    }
    if !links.is_empty() {
        sections.push(GroupedSection::new(links));
    }

    let mut notes: Vec<GroupedCell> = Vec::new();
    if let Some(c) = about.credits_text() {
        let credits = c.clone();
        let row = rows::navigate(day_fluent::tr("day-about-acknowledgements"), move || {
            scroll(label(credits.clone()).padding(16.0))
        });
        notes.push(GroupedCell::from_row(inst.clone(), &row));
    }
    if let Some(l) = about.licenses_text() {
        let licenses = l.clone();
        let row = rows::navigate(day_fluent::tr("day-about-licenses"), move || {
            scroll(label(licenses.clone()).monospace().padding(16.0))
        });
        notes.push(GroupedCell::from_row(inst.clone(), &row));
    }
    if !notes.is_empty() {
        sections.push(GroupedSection::new(notes));
    }

    if let Some(c) = about.copyright_text() {
        sections.push(GroupedSection::new(vec![GroupedCell::filled(
            label(c)
                .font(day_spec::Font::Footnote)
                .secondary()
                .align(day_spec::props::TextAlign::Center)
                .id("about-copyright")
                .grow_w()
                .any(),
        )]));
    }

    Some(
        Grouped {
            sections,
            hug: false,
        }
        .id("about-page")
        .any(),
    )
}
