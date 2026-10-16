// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// XAML / WinUI 3: the Windows 11 Settings row. Each row is a settings card (the look the
// CommunityToolkit's SettingsCard draws: a card with the header and description leading and the
// control at the trailing edge) composed in this crate's C++/WinRT shim (src/lib-xaml-shim.cpp)
// from stock XAML, so nothing new is linked (docs/extending.md §5, the day-piece-searchfield
// recipe for the shim and boxing contract). The card is a CONTAINER piece: its Canvas hosts the
// same Day control the composed row builds (`toggle`, `picker`, `slider`, `text_field`,
// `button`, …) as a child, placed at the trailing edge by `CardLayout`, so the control keeps its
// element id and its `enabled` binding, and dayscript drives it as on every other backend. A
// control the title alone would leave nameless to UI Automation (a toggle, a picker, a slider, a
// field) is labeled with the row title through the a11y decorator.
// Navigate and link rows host no control and are clickable cards with an action glyph: the shim
// realizes those as a WinUI `Button` styled as the card (a tab stop, Enter/Space activation, the
// system focus visual, a ButtonAutomationPeer with Invoke, the title as its automation name),
// whose Click reaches this arm as the same `Event::Tap` a pointer tap does. A row whose control
// already shows the title (an action's button, a captioned link) shows its caption alone in the
// description style, never the title twice; a note or a custom row has no card shape and stays
// composed. Windows cannot be built here: this arm is written against the toolkit's conventions
// and verified by the Windows CI leg only.
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::CString;
use std::os::raw::{c_char, c_void};

use day_core::{BuildCx, Flex, Layout, LayoutOps, RNode, with_tree};
use day_spec::{Event, NodeId, Proposal, Rect, Size};
use day_xaml::{WinHandle, Xaml};

const KIND: &str = "day-piece-settings.row";

/// SettingsCard metrics (the shim holds the same numbers): 16px padding all round, 16px between
/// the text column and the control, a 68px minimum height.
const PAD: f64 = 16.0;
const GAP: f64 = 16.0;
const MIN_HEIGHT: f64 = 68.0;
/// The row width assumed when nothing proposes one.
const DEFAULT_WIDTH: f64 = 480.0;
/// Segoe symbol code points for the action glyph: ChevronRightMed, OpenInNewWindow.
const GLYPH_CHEVRON: u32 = 0xE974;
const GLYPH_OPEN: u32 = 0xE8A7;

/// Navigation's image channel handles bundled art. Standard symbols use Day's shared vectors
/// here, including when a locale change replaces the NavigationViewItems.
pub(super) fn sidebar_symbols(nav: AnyPiece, groups: &[&Group]) -> AnyPiece {
    use day_xaml::XamlExt;
    let paths = groups
        .iter()
        .map(|g| match &g.icon {
            Some(GroupIcon::Symbol(symbol)) => symbol.outline_path().unwrap_or_default(),
            _ => "",
        })
        .collect::<Vec<_>>()
        .join("\u{1f}");
    nav.xaml_raw(move |abi, _| unsafe {
        day_settings_xaml_sidebar_symbols(abi, cstr(&paths).as_ptr());
    })
    .any()
}

unsafe extern "C" {
    fn day_settings_xaml_sidebar_symbols(abi: *mut c_void, paths: *const c_char);
    fn day_settings_xaml_card_new(
        header: *const c_char,
        description: *const c_char,
        glyph: u32,
        clickable: i32,
        destructive: i32,
        id: u64,
        cb: extern "C" fn(u64),
    ) -> *mut c_void;
    fn day_settings_xaml_card_set_text(
        w: *mut c_void,
        header: *const c_char,
        description: *const c_char,
    );
    fn day_settings_xaml_card_measure(
        w: *mut c_void,
        text_w: f64,
        out_w: *mut f64,
        out_h: *mut f64,
    );
}

/// What the shim builds once per row.
#[derive(Clone, Debug, PartialEq)]
struct CardProps {
    header: String,
    description: String,
    /// The trailing action glyph (0 = none).
    glyph: u32,
    /// The whole card is a Button that reports clicks as taps (navigate, link); it hosts no
    /// Day control.
    clickable: bool,
    /// Header in the critical color (a destructive action).
    destructive: bool,
}

/// Live text: a locale change re-resolves the row's sources.
#[derive(Clone, Debug, PartialEq)]
enum CardPatch {
    Text { header: String, description: String },
}

/// A clickable card's Button clicked (pointer, Enter/Space or a UI Automation Invoke).
extern "C" fn on_card_tap(id: u64) {
    day_xaml::emit(NodeId(id), Event::Tap(day_spec::Point::ZERO));
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

fn make(_backend: &mut Xaml, p: &CardProps, id: NodeId) -> WinHandle {
    WinHandle(unsafe {
        day_settings_xaml_card_new(
            cstr(&p.header).as_ptr(),
            cstr(&p.description).as_ptr(),
            p.glyph,
            i32::from(p.clickable),
            i32::from(p.destructive),
            id.0,
            on_card_tap,
        )
    })
}

fn update(_backend: &mut Xaml, h: &WinHandle, patch: &CardPatch) {
    let CardPatch::Text {
        header,
        description,
    } = patch;
    unsafe {
        day_settings_xaml_card_set_text(h.0, cstr(header).as_ptr(), cstr(description).as_ptr())
    };
}

/// The text column's natural size at the proposed width (`CardLayout` asks; the padding and the
/// control are the layout's business).
fn measure(_backend: &mut Xaml, h: &WinHandle, p: Proposal) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_settings_xaml_card_measure(h.0, p.width.unwrap_or(-1.0), &mut w, &mut hh) };
    Size::new(p.width.unwrap_or(w), hh)
}

day_pieces::renderer!(day_xaml::RENDERERS, Xaml,
    kind: KIND, props: CardProps, patch: CardPatch,
    make: make, update: update, measure: measure);

/// The card's layout: the native text column leading, the single Day child (the control) at the
/// trailing edge, vertically centered; the row is as tall as the taller of the two plus the
/// card padding, never under the SettingsCard minimum.
struct CardLayout;

impl CardLayout {
    /// The control's size at `row_w`: its natural size, or for a stretching control (a text
    /// field, a slider) a share of the content width that still leaves the text column room.
    fn control_size(cx: &mut dyn LayoutOps, c: RNode, row_w: f64) -> Size {
        let content_w = (row_w - 2.0 * PAD).max(0.0);
        let nat = cx.measure_child(c, Proposal::new(None, None));
        let w = if cx.flex_of(c).grow_w {
            let share = (content_w * 0.45).min((content_w - 140.0).max(0.0));
            nat.width.max(share).min(content_w)
        } else {
            nat.width.min(content_w)
        };
        let s = cx.measure_child(c, Proposal::new(Some(w), None));
        Size::new(w, s.height)
    }

    fn text_width(row_w: f64, control: Option<Size>) -> f64 {
        let content_w = (row_w - 2.0 * PAD).max(0.0);
        match control {
            Some(s) if s.width > 0.0 => (content_w - s.width - GAP).max(0.0),
            _ => content_w,
        }
    }
}

impl Layout for CardLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let row_w = p.width.unwrap_or(DEFAULT_WIDTH);
        let control = children.first().map(|&c| Self::control_size(cx, c, row_w));
        let text = cx.measure_leaf(Proposal::new(Some(Self::text_width(row_w, control)), None));
        let inner = text.height.max(control.map_or(0.0, |s| s.height));
        Size::new(row_w, (inner + 2.0 * PAD).max(MIN_HEIGHT).ceil())
    }

    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        let row_w = bounds.size.width;
        let control = children.first().map(|&c| Self::control_size(cx, c, row_w));
        // Measured again with the final width: the shim keeps the last measured width as the
        // text column's layout width, so the text wraps clear of the control.
        let _ = cx.measure_leaf(Proposal::new(Some(Self::text_width(row_w, control)), None));
        if let (Some(&c), Some(s)) = (children.first(), control) {
            let x = (row_w - PAD - s.width).max(PAD);
            let y = ((bounds.size.height - s.height) / 2.0).max(0.0);
            cx.place_child(c, Rect::new(x, y, s.width, s.height));
        }
    }
}

/// One settings card: the shim's row hosting an optional Day control.
struct Card {
    props: CardProps,
    title: TextSource,
    caption: Option<TextSource>,
    control: Option<AnyPiece>,
    on_tap: Option<Rc<dyn Fn()>>,
}

impl Piece for Card {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Card {
            props,
            title,
            caption,
            control,
            on_tap,
        } = self;
        let node = cx.native(
            KIND,
            &props,
            Rc::new(CardLayout),
            Flex {
                grow_w: true,
                ..Default::default()
            },
            day_core::Boundary::No,
        );
        if let Some(control) = control {
            cx.under(node, |cx| {
                let _ = control.build(cx);
            });
        }
        if let Some(on_tap) = on_tap {
            cx.on(node, move |ev| {
                if let Event::Tap(_) = ev {
                    on_tap();
                }
            });
        }
        // A dynamic source (a localized title) re-resolves under the locale signal, read
        // tracked so the watch subscribes to it; the watch runs on changes only, the initial
        // text went in through the props.
        let dynamic =
            matches!(title, TextSource::Dyn(_)) || matches!(caption, Some(TextSource::Dyn(_)));
        if dynamic {
            watch(
                move || (resolve_text(&title), caption.as_ref().map(resolve_text)),
                move |(header, description), _| {
                    let patch = CardPatch::Text {
                        header: header.clone(),
                        description: description.clone().unwrap_or_default(),
                    };
                    with_tree(|t| t.patch(node, Box::new(patch), true));
                },
            );
        }
        node
    }
}

/// The row as a settings card, or `None` for the kinds that have no card shape (a note, a
/// custom piece), which stay composed.
pub(super) fn row_piece(inst: Rc<Installed>, r: &Row, _desktop: bool) -> Option<AnyPiece> {
    let id = r.id.clone();
    let enabled = r.enabled;
    let with_id = |p: AnyPiece| match &id {
        Some(id) => p.id(id.clone()).any(),
        None => p,
    };
    let enabled_fn = move || enabled.is_none_or(|e| e.get());
    // The row title as the control's automation name: a ToggleSwitch with empty on/off content,
    // a ComboBox, a Slider or a TextBox has no name of its own, and the card's header TextBlock
    // is not associated with it the way a labeled control's header is.
    let named = |p: AnyPiece| {
        let title = text_source_fn(&r.title);
        p.a11y(move |a| a.label(title)).any()
    };
    let mut props = CardProps {
        header: r.title.initial(),
        description: r
            .caption
            .as_ref()
            .map(TextSource::initial)
            .unwrap_or_default(),
        glyph: 0,
        clickable: false,
        destructive: false,
    };
    let mut on_tap: Option<Rc<dyn Fn()>> = None;
    // A row whose control already shows the title (an action's button, a link): the card's text
    // column shows the caption alone, in the description style, never the title a second time.
    let mut caption_only = false;
    let mut show_caption_only = |props: &mut CardProps, c: &TextSource| {
        props.header = String::new();
        props.description = c.initial();
        caption_only = true;
    };
    // The control is the composed row's, unchanged: same piece, same id, same `enabled`.
    let control: Option<AnyPiece> = match &r.kind {
        RowKind::Toggle(sig) => Some(with_id(named(toggle(*sig).enabled(enabled_fn).any()))),
        RowKind::Choice {
            labels,
            index,
            style,
        } => {
            let p = choice_picker(labels, *index, *style == ChoiceStyle::Segmented, enabled_fn);
            Some(with_id(named(p.any())))
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
                Some(i) => named(s.id(i.clone()).grow_w().min_width(140.0).any()),
                None => named(s.grow_w().min_width(140.0).any()),
            };
            Some(match value_text {
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
            })
        }
        RowKind::Stepper { value, range, step } => {
            let v = *value;
            let (dec, inc) = stepper_buttons(v, range, *step, enabled_fn);
            Some(stepper_control(v, dec, inc, id.as_deref()))
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
            Some(named(match &id {
                Some(i) => f
                    .enabled(enabled_fn)
                    .id(i.clone())
                    .grow_w()
                    .min_width(160.0)
                    .any(),
                None => f.enabled(enabled_fn).grow_w().min_width(160.0).any(),
            }))
        }
        RowKind::Action {
            on,
            destructive,
            confirm,
            confirm_label,
            busy,
        } => {
            props.destructive = *destructive;
            let on = on.clone();
            let title = text_source_fn(&r.title);
            let confirm = confirm.clone();
            let confirm_label = confirm_label.clone();
            let busy = *busy;
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
            let b = button(text_source_fn(&r.title))
                .action(run)
                .enabled(move || enabled_fn() && !busy.is_some_and(|b| b.get()));
            let ctl = with_id(b.any());
            if let Some(c) = &r.caption {
                show_caption_only(&mut props, c);
            }
            Some(match busy {
                Some(b) => row((when(move || b.get(), spinner), ctl))
                    .spacing(8.0)
                    .align(VAlign::Center)
                    .any(),
                None => ctl,
            })
        }
        RowKind::Link(url) => {
            let url = url.clone();
            match &r.caption {
                // The Day link carries the title; the card shows the caption beside it.
                Some(c) => {
                    show_caption_only(&mut props, c);
                    Some(with_id(link(text_source_fn(&r.title), url).any()))
                }
                None => {
                    props.glyph = GLYPH_OPEN;
                    props.clickable = true;
                    on_tap = Some(Rc::new(move || day_core::open_url(&url)));
                    None
                }
            }
        }
        RowKind::Navigate(build) => {
            props.glyph = GLYPH_CHEVRON;
            props.clickable = true;
            let build = build.clone();
            let title = text_source_fn(&r.title);
            let inst = inst.clone();
            on_tap = Some(Rc::new(move || {
                push_subpage(inst.clone(), title(), build.clone());
            }));
            None
        }
        RowKind::Info { value, copyable } => {
            let v = text_label(value).secondary();
            let v = if *copyable {
                let text = value.clone();
                v.on_tap(move || {
                    day_part_clipboard::set_text(&text.initial());
                })
                .any()
            } else {
                v.any()
            };
            Some(with_id(v))
        }
        RowKind::Note | RowKind::Custom(_) => return None,
    };
    let (title, caption) = if caption_only {
        (TextSource::Static(String::new()), r.caption.clone())
    } else {
        (r.title.clone(), r.caption.clone())
    };
    let clickable = on_tap.is_some();
    let card = Card {
        props,
        title,
        caption,
        control,
        on_tap,
    };
    // A clickable card carries the row's id itself (no control to carry it), so a dayscript
    // `tap:` lands on the card: day-core enqueues `Event::Tap` on the node, the same event the
    // Button's Click raises through `on_card_tap`.
    let piece = AnyPiece::new(card);
    Some(match &id {
        Some(id) if clickable => piece.id(id.clone()).any(),
        _ => piece,
    })
}
