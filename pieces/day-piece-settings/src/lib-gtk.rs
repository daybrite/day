// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// GTK: libadwaita's settings idiom (docs/settings.md), in two layers.
//
// The Preferences surface (`window_shell`, second half of this file) is libadwaita's own
// `AdwPreferencesDialog`: day-gtk presents one over the app window for Day's `day.preferences`
// singleton (`open_preferences` focuses it, Escape closes it, dayscript captures it by key), and
// the shell fills it with one `AdwPreferencesPage` per group (icon + title; the dialog shows its
// view switcher and search for two or more), each a column of `AdwPreferencesGroup`s (title and
// description from the section's title and footer) holding `.boxed-list` rows. The Day control
// a row carries is mounted INTO the native row as its suffix. A navigate row pushes its Day
// sub-page into the dialog as an `AdwNavigationPage` (`push_subpage`). The dialog floats in a
// wide window and becomes a bottom sheet in a narrow one, as libadwaita decides. With no app
// window to present over (day-gtk then opens a window), the same pages go in an `AdwViewStack`
// under an `AdwViewSwitcher` in the window's content.
//
// A group page shown on its own (`Prefs::group_page`, a reader's settings sheet) keeps the
// shared section card and gets one `AdwActionRow` per row (`row_piece`, first half): the native
// row is a container piece (docs/extending.md §5) realized as a `gtk4::Fixed` host holding a
// one-row `GtkListBox`, and `RowLayout` places the Day control over the row's suffix position,
// so the control keeps its element id, its enabled binding and its scripting surface. The
// row's text column stops short of the control: a suffix spacer in the `AdwActionRow`, sized by
// the layout to the control's width, reserves its room, so a long title or caption wraps beside
// the control instead of running under it. Activatable rows (navigate, link, toggle) report
// `row-activated` through the backend's event sink as an `Event::Custom`; the piece side turns
// it into the action.
//
// About is `AdwAboutDialog` built from the `About` getters and presented over the active window;
// an `About` with a custom piece keeps the composed Day window instead.
// ---------------------------------------------------------------------------

use super::*;

use day_core::{Boundary, BuildCx, Flex, Layout, LayoutOps, RNode, with_tree};
use day_gtk::{Gtk, GtkExt};
use day_spec::ffi_guard;
use day_spec::{Event, NodeId, Proposal, Rect, Size};
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

/// The native row kind (docs/extending.md §2).
const ROW_KIND: &str = "day-piece-settings.row";
/// The `Event::Custom` tag a row's native activation arrives under.
const ACTIVATE_TAG: &str = "day-piece-settings:activate";

/// An `AdwActionRow` ends 12px before the row's edge; the control sits flush with that line.
const TRAILING_INSET: f64 = 12.0;
/// Room kept between the title column and a stretching control.
const GAP: f64 = 12.0;
/// Breathing space above and below a control taller than the row's text.
const CONTROL_PAD: f64 = 6.0;
/// The row never measures shorter than libadwaita's single-line row.
const ROW_MIN_H: f64 = 48.0;

/// Realize-time props of the native row.
#[derive(Clone, Debug, PartialEq)]
struct RowProps {
    /// Pairs the realized row with its `RowLayout` (see [`RESERVES`]).
    key: u64,
    title: String,
    subtitle: String,
    /// Whether clicking the row itself does something (navigate, link, toggle).
    activatable: bool,
    /// A themed icon drawn at the suffix position (`go-next-symbolic` for a navigate row).
    suffix_icon: Option<&'static str>,
    /// The title is the row's caption alone (the suffix button or link carries the real title),
    /// so it draws in the subtitle's secondary style.
    secondary_title: bool,
    enabled: bool,
}

/// Sparse updates after realization.
#[derive(Clone, Debug, PartialEq)]
enum RowPatch {
    Title(String),
    Subtitle(String),
    Enabled(bool),
}

struct RowState {
    row: adw::ActionRow,
    /// The suffix spacer holding the Day control's room (see [`RESERVES`]).
    reserve: gtk4::Box,
    key: u64,
}

day_core::tls_group! {
    /// The `AdwActionRow` behind each host widget, keyed by the host's address.
    static ROWS: RefCell<HashMap<usize, RowState>> = RefCell::new(HashMap::new());
    /// Each row's suffix spacer by the row's key: `RowLayout` sizes it to the control it places
    /// over the suffix position, so the row's own text wraps clear of the control.
    static RESERVES: RefCell<HashMap<u64, gtk4::Box>> = RefCell::new(HashMap::new());
    static NEXT_KEY: Cell<u64> = const { Cell::new(0) };
    /// Whether the row stylesheet is installed on the display (once per process).
    static CSS_INSTALLED: Cell<bool> = const { Cell::new(false) };
    /// The window shell behind each host widget, keyed by the host's address.
    static SHELLS: RefCell<HashMap<usize, Rc<ShellState>>> = RefCell::new(HashMap::new());
    /// The shell by its node, for the piece side to find the dialog it realized into.
    static SHELL_BY_NODE: RefCell<HashMap<NodeId, Rc<ShellState>>> = RefCell::new(HashMap::new());
    /// The open preferences shell, for a sub-page push into its dialog.
    static OPEN_SHELL: RefCell<Option<OpenShell>> = const { RefCell::new(None) };
    /// Each shell slot's widget by key, while it is realized.
    static SLOT_WIDGETS: RefCell<HashMap<u64, gtk4::Widget>> = RefCell::new(HashMap::new());
    /// Each slot widget's key, for `release`.
    static SLOT_KEYS: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
    /// The width GTK last granted each filling slot, by slot key.
    static FILL_WIDTHS: RefCell<HashMap<u64, f64>> = RefCell::new(HashMap::new());
}

fn next_key() -> u64 {
    NEXT_KEY.with(|k| {
        let v = k.get() + 1;
        k.set(v);
        v
    })
}

/// The width of a row's suffix spacer: what the layout last reserved for the control.
fn reserve_width(st: &RowState) -> f64 {
    f64::from(st.reserve.width_request().max(0))
}

/// Reserve `w` beside the row's text for the control the layout places over its suffix.
fn set_reserve(key: u64, w: f64) {
    RESERVES.with(|m| {
        if let Some(b) = m.borrow().get(&key) {
            let w = w.round() as i32;
            if b.width_request() != w {
                b.set_size_request(w, -1);
            }
        }
    });
}

fn key(w: &gtk4::Widget) -> usize {
    w.as_ptr() as usize
}

/// The one-row list carries no background of its own (the Day section card paints it), and an
/// activatable row gets libadwaita's hover and press feedback inside rounded corners.
fn install_css() {
    if CSS_INSTALLED.with(|c| c.replace(true)) {
        return;
    }
    let provider = gtk4::CssProvider::new();
    provider.load_from_data(
        "list.day-settings-list { background: none; }\n\
         list.day-settings-list > row { border-radius: 6px; }\n\
         list.day-settings-list > row.activatable:hover { background-color: alpha(currentColor, 0.07); }\n\
         list.day-settings-list > row.activatable:active { background-color: alpha(currentColor, 0.16); }\n\
         list.day-settings-list > row.day-settings-caption-only .title { font-size: smaller; opacity: 0.55; }",
    );
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn make(_backend: &mut Gtk, p: &RowProps, id: NodeId) -> gtk4::Widget {
    install_css();
    let row = adw::ActionRow::new();
    // Titles are plain text from the app's catalog, never Pango markup.
    row.set_use_markup(false);
    row.set_title(&p.title);
    // A caption standing in for the title wraps like a subtitle; a real title stays one line.
    row.set_title_lines(if p.secondary_title { 2 } else { 1 });
    row.set_subtitle_lines(2);
    if !p.subtitle.is_empty() {
        row.set_subtitle(&p.subtitle);
    }
    row.set_activatable(p.activatable);
    row.set_sensitive(p.enabled);
    row.add_css_class("day-settings-row");
    if p.secondary_title {
        row.add_css_class("day-settings-caption-only");
    }
    if let Some(icon) = p.suffix_icon {
        let image = gtk4::Image::from_icon_name(icon);
        image.add_css_class("dim-label");
        row.add_suffix(&image);
    }
    // The last suffix: the room for the Day control, sized by the layout (0 until it places).
    let reserve = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    row.add_suffix(&reserve);
    RESERVES.with(|m| m.borrow_mut().insert(p.key, reserve.clone()));
    // A `GtkListBoxRow` only activates inside a `GtkListBox`, so each row gets its own list.
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.add_css_class("day-settings-list");
    list.append(&row);
    if p.activatable {
        list.connect_row_activated(move |_, _| {
            day_gtk::emit(id, Event::custom(ACTIVATE_TAG, ""));
        });
    }
    // The host is the `gtk4::Fixed` the backend's generic `insert` puts the Day control into.
    // Day sizes the host through `set_size_request` (`set_frame`); the bindings hand the same
    // request to the list, so the row fills the host while the control draws over its suffix.
    let host = gtk4::Fixed::new();
    host.put(&list, 0.0, 0.0);
    host.bind_property("width-request", &list, "width-request")
        .sync_create()
        .build();
    host.bind_property("height-request", &list, "height-request")
        .sync_create()
        .build();
    let w: gtk4::Widget = host.upcast();
    ROWS.with(|m| {
        m.borrow_mut().insert(
            key(&w),
            RowState {
                row,
                reserve,
                key: p.key,
            },
        )
    });
    w
}

fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &RowPatch) {
    ROWS.with(|m| {
        let m = m.borrow();
        let Some(st) = m.get(&key(h)) else {
            return;
        };
        match patch {
            RowPatch::Title(t) => st.row.set_title(t),
            RowPatch::Subtitle(s) => st.row.set_subtitle(s),
            RowPatch::Enabled(on) => st.row.set_sensitive(*on),
        }
    });
}

/// The row's own natural size, without the control's reserved room: its unwrapped width (title
/// column plus suffix glyph and padding) and its height when the text has the proposed width.
/// Measured on the `AdwActionRow`, not the host, whose size request Day set from the previous
/// layout and which would only echo it back. The proposal excludes the reserve the layout set
/// (the row is allocated the full width and the spacer takes that part), so the answer is the
/// same before and after the layout's first `place`.
fn measure(_backend: &mut Gtk, h: &gtk4::Widget, p: Proposal) -> Size {
    let Some((row, reserve)) = ROWS.with(|m| {
        m.borrow()
            .get(&key(h))
            .map(|s| (s.row.clone(), reserve_width(s)))
    }) else {
        return Size::new(p.width.unwrap_or(0.0), ROW_MIN_H);
    };
    let (min_w, nat_w, _, _) = row.measure(gtk4::Orientation::Horizontal, -1);
    let for_w = match p.width {
        Some(w) => ((w + reserve).round() as i32).max(min_w),
        None => -1,
    };
    let (_, nat_h, _, _) = row.measure(gtk4::Orientation::Vertical, for_w);
    Size::new(
        (f64::from(nat_w) - reserve).max(0.0),
        f64::from(nat_h).max(ROW_MIN_H),
    )
}

/// Drop the row entry when the host goes away: the map's key is the host's address, which the
/// allocator reuses.
fn release(_backend: &mut Gtk, h: &gtk4::Widget) {
    let taken = ROWS.with(|m| m.borrow_mut().remove(&key(h)));
    if let Some(st) = taken {
        RESERVES.with(|m| {
            m.borrow_mut().remove(&st.key);
        });
    }
}

day_pieces::renderer!(day_gtk::RENDERERS, Gtk,
    kind: ROW_KIND, props: RowProps, patch: RowPatch,
    make: make, update: update, measure: measure, release: release);

// ---------------------------------------------------------------------------
// The container piece and its layout
// ---------------------------------------------------------------------------

/// Places the single Day child (the control) at the row's suffix position, vertically
/// centered; the row's own text is the native leaf, measured at the width left beside the
/// control. A stretching control (a text field, a slider) takes the width left beside the
/// title column's natural width, never less than its minimum.
struct RowLayout {
    stretch: Option<f64>,
    key: u64,
}

impl RowLayout {
    /// The control's size at `width`: natural, or the stretch share beside the text column.
    fn control_size(&self, cx: &mut dyn LayoutOps, c: RNode, width: Option<f64>) -> Size {
        let proposal = match self.stretch {
            Some(min) => {
                let text_w = cx.measure_leaf(Proposal::new(None, None)).width;
                let w = width
                    .map(|w| (w - text_w - GAP - TRAILING_INSET).max(min))
                    .unwrap_or(min);
                Proposal::new(Some(w), None)
            }
            None => Proposal::new(None, None),
        };
        cx.measure_child(c, proposal)
    }

    /// The width the row's text may take beside a control of `ctl_w`.
    fn text_proposal(width: Option<f64>, ctl_w: f64) -> Proposal {
        Proposal::new(width.map(|w| (w - ctl_w - GAP).max(0.0)), None)
    }
}

impl Layout for RowLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let ctl = children
            .first()
            .map(|&c| self.control_size(cx, c, p.width))
            .unwrap_or(Size::ZERO);
        let leaf = cx.measure_leaf(Self::text_proposal(p.width, ctl.width));
        let width = p
            .width
            .unwrap_or(leaf.width + GAP + ctl.width + TRAILING_INSET);
        let height = leaf.height.max(ctl.height + 2.0 * CONTROL_PAD);
        Size::new(width, height)
    }

    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        let Some(&c) = children.first() else {
            return;
        };
        let width = bounds.size.width;
        let s = self.control_size(cx, c, Some(width));
        // The row's text column ends where the control begins.
        set_reserve(self.key, s.width + GAP);
        let x = (width - TRAILING_INSET - s.width).max(0.0);
        let y = ((bounds.size.height - s.height) / 2.0).max(0.0);
        cx.place_child(c, Rect::new(x, y, s.width, s.height));
    }
}

/// One native row: the `AdwActionRow` with the Day control as its child.
struct NativeRow {
    props: RowProps,
    title: TextSource,
    caption: Option<TextSource>,
    enabled: Option<Reactive<bool>>,
    control: Option<AnyPiece>,
    stretch: Option<f64>,
    on_activate: Option<Rc<dyn Fn()>>,
}

fn patch(node: RNode, p: RowPatch, affects_size: bool) {
    with_tree(|t| t.patch(node, Box::new(p), affects_size));
}

impl Piece for NativeRow {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let NativeRow {
            mut props,
            title,
            caption,
            enabled,
            control,
            stretch,
            on_activate,
        } = self;
        let key = next_key();
        props.key = key;
        let node = cx.native(
            ROW_KIND,
            &props,
            Rc::new(RowLayout { stretch, key }),
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
        // A dynamic title or caption (a localized string) re-resolves on locale change.
        if let TextSource::Dyn(f) = title {
            watch(
                move || f(),
                move |t, _| patch(node, RowPatch::Title(t.clone()), true),
            );
        }
        if let Some(TextSource::Dyn(f)) = caption {
            watch(
                move || f(),
                move |t, _| patch(node, RowPatch::Subtitle(t.clone()), true),
            );
        }
        if let Some(enabled) = enabled {
            watch(
                move || enabled.get(),
                move |on, _| patch(node, RowPatch::Enabled(*on), false),
            );
        }
        if let Some(on) = on_activate {
            // The native activation, and dayscript's `tap:` on the row's id.
            cx.on(node, move |ev| match ev {
                Event::Custom { tag, .. } if *tag == ACTIVATE_TAG => on(),
                Event::Tap(_) => on(),
                _ => {}
            });
        }
        node
    }
}

/// The GTK row for `r`, or `None` for the kinds that are not a titled row (a note, a custom
/// piece), which keep the composed shape.
pub(super) fn row_piece(inst: Rc<Installed>, r: &Row, _desktop: bool) -> Option<AnyPiece> {
    let id = r.id.clone();
    let enabled = r.enabled.clone();
    let with_id = |p: AnyPiece| match &id {
        Some(id) => p.id(id.clone()).any(),
        None => p,
    };
    let enabled_fn = move || enabled.as_ref().is_none_or(|e| e.get());
    // What the row starts as, for the native row\'s first draw; the control tracks the rest.
    let enabled_now = enabled_fn();
    // `key` is assigned at build (`NativeRow::build`).
    let props = |activatable: bool, suffix_icon: Option<&'static str>| RowProps {
        key: 0,
        title: r.title.initial(),
        subtitle: r
            .caption
            .as_ref()
            .map(TextSource::initial)
            .unwrap_or_default(),
        activatable,
        suffix_icon,
        secondary_title: false,
        enabled: enabled_now,
    };
    // A row whose suffix control (a button, a link) already shows the title: the title column
    // shows the caption alone, in the subtitle's style, never the title a second time.
    let caption_only_props = |c: &TextSource| RowProps {
        key: 0,
        title: c.initial(),
        subtitle: String::new(),
        activatable: false,
        suffix_icon: None,
        secondary_title: true,
        enabled: enabled_now,
    };
    let native = |props: RowProps,
                  control: Option<AnyPiece>,
                  stretch: Option<f64>,
                  on_activate: Option<Rc<dyn Fn()>>| {
        AnyPiece::new(NativeRow {
            props,
            title: r.title.clone(),
            caption: r.caption.clone(),
            enabled: r.enabled.clone(),
            control,
            stretch,
            on_activate,
        })
    };
    let native_caption_only = |c: &TextSource, control: AnyPiece| {
        AnyPiece::new(NativeRow {
            props: caption_only_props(c),
            title: c.clone(),
            caption: None,
            enabled: r.enabled.clone(),
            control: Some(control),
            stretch: None,
            on_activate: None,
        })
    };
    let piece = match &r.kind {
        RowKind::Toggle(sig) => {
            let ctl = with_id(toggle(*sig).enabled(enabled_fn.clone()).any());
            // The AdwSwitchRow contract: the row itself flips the switch.
            let sig = *sig;
            let flip = move || {
                if enabled_fn() {
                    sig.set(!sig.get_untracked());
                }
            };
            native(props(true, None), Some(ctl), None, Some(Rc::new(flip)))
        }
        RowKind::Choice {
            labels,
            index,
            style,
        } => {
            let p = choice_picker(labels, *index, *style == ChoiceStyle::Segmented, enabled_fn);
            native(props(false, None), Some(with_id(p.any())), None, None)
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
            native(props(false, None), Some(ctl), Some(160.0), None)
        }
        RowKind::Stepper { value, range, step } => {
            let v = *value;
            let (dec, inc) = stepper_buttons(v, range, *step, enabled_fn);
            let ctl = stepper_control(v, dec, inc, id.as_deref());
            native(props(false, None), Some(ctl), None, None)
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
                    .min_width(160.0)
                    .any(),
                None => f.enabled(enabled_fn).grow_w().min_width(160.0).any(),
            };
            native(props(false, None), Some(ctl), Some(160.0), None)
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
            let b = button(text_source_fn(&r.title))
                .action(run)
                .enabled(move || enabled_fn() && !busy_gate.as_ref().is_some_and(|b| b.get()));
            // libadwaita's red button: the destructive-action style class.
            let destructive = *destructive;
            let b = b.gtk(move |w, _| {
                if destructive {
                    w.add_css_class("destructive-action");
                }
            });
            let ctl = with_id(b.any());
            let ctl = match busy {
                Some(b) => row((when(move || b.get(), spinner), ctl))
                    .spacing(8.0)
                    .align(VAlign::Center)
                    .any(),
                None => ctl,
            };
            match &r.caption {
                Some(c) => native_caption_only(c, ctl),
                None => native(props(false, None), Some(ctl), None, None),
            }
        }
        RowKind::Link(url) => {
            let url = url.clone();
            match &r.caption {
                // The Day link carries the title; the row shows the caption beside it.
                Some(c) => {
                    let l = with_id(link(text_source_fn(&r.title), url).any());
                    native_caption_only(c, l)
                }
                None => {
                    let open: Rc<dyn Fn()> = Rc::new(move || day_core::open_url(&url));
                    with_id(native(
                        props(true, Some("adw-external-link-symbolic")),
                        None,
                        None,
                        Some(open),
                    ))
                }
            }
        }
        RowKind::Navigate(build) => {
            let build = build.clone();
            let title = text_source_fn(&r.title);
            let open: Rc<dyn Fn()> = Rc::new(move || {
                push_subpage(inst.clone(), title(), build.clone());
            });
            with_id(native(
                props(true, Some("go-next-symbolic")),
                None,
                None,
                Some(open),
            ))
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
            native(props(false, None), Some(with_id(v)), None, None)
        }
        RowKind::Note | RowKind::Custom(_) => return None,
    };
    Some(piece)
}

// ---------------------------------------------------------------------------
// About: AdwAboutDialog
// ---------------------------------------------------------------------------

/// Register `AdwAboutDialog` as the About action. `false` when the About carries a custom
/// piece, which only the composed Day window can show.
pub(super) fn install_about(inst: Rc<Installed>) -> bool {
    let Some(about) = inst.about.as_ref() else {
        return false;
    };
    if about.custom.is_some() {
        return false;
    }
    day_core::register_about(move || present_about(&inst));
    true
}

/// The window the dialog attaches to: the active Day window, else any visible one. `None`
/// presents the dialog as a window of its own.
fn parent_window() -> Option<gtk4::Window> {
    let windows: Vec<gtk4::Window> = gtk4::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<gtk4::Window>().ok())
        .filter(|w| w.is_visible())
        .collect();
    windows
        .iter()
        .find(|w| w.is_active())
        .or(windows.first())
        .cloned()
}

fn present_about(inst: &Installed) {
    let Some(about) = inst.about.as_ref() else {
        return;
    };
    let dialog = adw::AboutDialog::new();
    dialog.set_application_name(&about.display_name());
    let version = about
        .version_value()
        .map(str::to_owned)
        .or_else(|| day_core::launch_options().and_then(|o| o.version.clone()));
    if let Some(v) = version {
        let v = match about.build_value() {
            Some(b) if !b.is_empty() => format!("{v} ({b})"),
            _ => v,
        };
        dialog.set_version(&v);
    }
    if let Some(t) = about.tagline_text() {
        dialog.set_comments(&t);
    }
    if let Some(c) = about.copyright_text() {
        dialog.set_copyright(&c);
    }
    // `day launch`/the Linux package export the app icon as a themed icon the backend already
    // put on the icon theme's search path (the window icon uses the same name).
    if let Ok(icon) = std::env::var("DAY_ICON_NAME")
        && !icon.is_empty()
    {
        dialog.set_application_icon(&icon);
    }
    // The link titled with the catalog's "Website" is the dialog's own website slot; the
    // rest are its titled links.
    let website = day_l10n::t("day-about-website");
    for (title, url) in about.links() {
        let title = title.initial();
        if title == website {
            dialog.set_website(url);
        } else {
            dialog.add_link(&title, url);
        }
    }
    if let Some(p) = about.privacy_url() {
        dialog.add_link(&day_l10n::t("day-about-privacy"), p);
    }
    if let Some(credits) = about.credits_text() {
        let people: Vec<&str> = credits
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if !people.is_empty() {
            dialog.add_acknowledgement_section(
                Some(&day_l10n::t("day-about-acknowledgements")),
                &people,
            );
        }
    }
    if let Some(licenses) = about.licenses_text() {
        dialog.add_legal_section(
            &day_l10n::t("day-about-licenses"),
            None,
            gtk4::License::Custom,
            Some(&licenses),
        );
    }
    dialog.present(parent_window().as_ref());
}

// ---------------------------------------------------------------------------
// The window shell: the preferences window's own structure, from its real parts
// ---------------------------------------------------------------------------
//
// Day mounts a piece's children into the renderer's `GtkFixed` and positions them from its own
// layout, which cannot know where libadwaita puts a row's suffix. So every control is wrapped in
// a SLOT, a one-child `GtkFixed` node of its own, and the slot is moved out of the host into the
// native row (`AdwActionRow::add_suffix` for a control; the child of an `AdwPreferencesRow` for
// a custom row; a group child for a group's custom piece). GTK then lays the slot out like any
// suffix widget, while Day keeps measuring the control and sizing the slot: `set_frame` on a
// child outside a `GtkFixed` sets its size request and nothing else, so the control stays a Day
// node with its element id, its enabled binding and its scripting surface. A width-filling slot
// sits in a `SlotHost`, a plain widget whose `size_allocate` reports the width GTK granted, and
// Day re-lays the piece at that width. A row with no control but an element id (navigate, link,
// action) keeps the id on a 1×1 marker slot that never leaves the host; dayscript's `tap:` on
// the id is a Day event, so the marker runs the row's action like the native activation does.

/// The shell's kind under `day_gtk::RENDERERS`.
const SHELL_KIND: &str = "day-piece-settings.panel";
/// A slot's kind.
const SLOT_KIND: &str = "day-piece-settings.slot";
/// The switcher picked a page: `text` is the group id.
const SELECT_TAG: &str = "day-piece-settings:select";
/// A native row was activated: `text` is the row key.
const ROW_TAG: &str = "day-piece-settings:row";
/// A filling slot learned its width from GTK: re-lay its piece.
const FILL_TAG: &str = "day-piece-settings:fill";

/// The width of a slider or a text field at the suffix position, the same on every row so a
/// page of them lines up.
const SUFFIX_CONTROL_W: f64 = 180.0;

/// How a slot is hosted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotKind {
    /// A control at the row's suffix, at its natural size.
    Suffix,
    /// A piece that takes the row's width (a custom row, a group's custom piece).
    Fill,
    /// No widget: a 1×1 node carrying a row's element id.
    Marker,
}

/// What the Day layouts and the GTK side share about one slot.
struct SlotShared {
    key: u64,
    kind: SlotKind,
    /// The width GTK last granted a filling slot's host.
    width: Cell<Option<f64>>,
    /// The slot's node, for a relayout when that width changes.
    node: Cell<Option<RNode>>,
}

#[derive(Clone, Debug, PartialEq)]
struct SlotProps {
    key: u64,
    kind: SlotKind,
}

/// One row as the shell realizes it.
#[derive(Clone, Debug, PartialEq)]
struct ShellRow {
    key: u64,
    title: String,
    subtitle: String,
    activatable: bool,
    suffix_icon: Option<&'static str>,
    /// A destructive command: the title draws in the error color.
    destructive: bool,
    enabled: bool,
    kind: ShellRowKind,
}

#[derive(Clone, Debug, PartialEq)]
enum ShellRowKind {
    /// An `AdwActionRow`, with a control slot at its suffix.
    Row { slot: Option<u64> },
    /// An `AdwPreferencesRow` whose child is a filling slot.
    Custom { slot: u64 },
    /// A footnote under the group's list.
    Note,
}

#[derive(Clone, Debug, PartialEq)]
struct ShellGroup {
    key: u64,
    title: String,
    description: String,
    rows: Vec<ShellRow>,
}

#[derive(Clone, Debug, PartialEq)]
struct ShellPage {
    id: String,
    title: String,
    icon: Option<String>,
    groups: Vec<ShellGroup>,
    /// The group's custom piece, as a filling slot in a trailing untitled group.
    custom: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
struct ShellProps {
    /// The window root the shell builds under: what names its dialog to day-gtk.
    root: NodeId,
    pages: Vec<ShellPage>,
    selected: String,
}

#[derive(Clone, Debug, PartialEq)]
enum ShellPatch {
    /// Show this group's page (a programmatic selection: `open_at`, a route).
    Select(String),
    PageTitle(String, String),
    GroupTitle(u64, String),
    GroupDescription(u64, String),
    RowTitle(u64, String),
    RowSubtitle(u64, String),
    RowEnabled(u64, bool),
    Note(u64, String),
}

/// Where a slot goes once Day has mounted it into the host.
enum SlotTarget {
    Suffix(adw::ActionRow),
    Fill(SlotHost),
}

/// Where the pages went.
enum ShellHost {
    /// The `AdwPreferencesDialog` day-gtk presents for the panel: the pages are its own.
    Dialog(adw::PreferencesDialog),
    /// No app window to present over: the pages in a stack in the window's content.
    Stack(adw::ViewStack),
}

impl ShellHost {
    fn visible_page_name(&self) -> Option<String> {
        match self {
            ShellHost::Dialog(d) => d.visible_page_name().map(|n| n.to_string()),
            ShellHost::Stack(s) => s.visible_child_name().map(|n| n.to_string()),
        }
    }
    fn set_visible_page_name(&self, name: &str) {
        match self {
            ShellHost::Dialog(d) => d.set_visible_page_name(name),
            ShellHost::Stack(s) => s.set_visible_child_name(name),
        }
    }
}

struct ShellState {
    node: NodeId,
    host: ShellHost,
    pages: HashMap<String, adw::PreferencesPage>,
    groups: HashMap<u64, adw::PreferencesGroup>,
    rows: HashMap<u64, adw::ActionRow>,
    /// The custom rows, for their sensitivity.
    custom_rows: HashMap<u64, adw::PreferencesRow>,
    notes: HashMap<u64, gtk4::Label>,
    /// Slots still to move into their rows, by key.
    pending: RefCell<Vec<(u64, SlotTarget)>>,
    /// A programmatic page change in flight: no echo as a selection.
    suppress: Cell<bool>,
}

/// The open preferences shell, as the piece side registered it (`Shell::build`).
struct OpenShell {
    /// The shell's node: a sub-page's slot builds under it.
    node: RNode,
    /// The shell's build scope: a sub-page's reactive graph hangs off a child of it.
    owner: day_reactive::Scope,
    /// The filling slots the shell re-lays when their hosts report a width.
    fills: Rc<RefCell<Vec<Rc<SlotShared>>>>,
    state: Rc<ShellState>,
}

mod slot_host {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use gtk4::glib;
    use gtk4::prelude::*;
    use gtk4::subclass::prelude::*;

    /// A container that takes part in GTK's size negotiation the way a custom container in a
    /// C GTK app does: it asks for no width and gives its one child the whole allocation.
    /// Around the fallback's page stack it keeps the stack exactly the host's size (a `GtkFixed`
    /// allocates a child its natural size, and a page's natural size is its content's). Around
    /// a filling slot it also reports the width GTK granted, which is the one thing Day cannot
    /// measure itself.
    /// A host's allocation observer: the width granted.
    type WidthHook = Rc<dyn Fn(i32)>;

    #[derive(Default)]
    pub struct SlotHostImp {
        /// Report the child's natural height (a filling slot); else ask for nothing at all.
        pub natural_height: Cell<bool>,
        /// Runs after every allocation with the width granted.
        pub on_width: RefCell<Option<WidthHook>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SlotHostImp {
        const NAME: &'static str = "DaySettingsSlotHost";
        type Type = super::SlotHost;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for SlotHostImp {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for SlotHostImp {
        fn measure(&self, orientation: gtk4::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk4::Orientation::Vertical
                && self.natural_height.get()
                && let Some(child) = self.obj().first_child()
            {
                let (min, nat, _, _) = child.measure(orientation, for_size);
                return (min, nat, -1, -1);
            }
            (0, 0, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.obj().first_child() {
                // GTK insists a child gets at least its minimum.
                let (min_w, ..) = child.measure(gtk4::Orientation::Horizontal, -1);
                let (min_h, ..) = child.measure(gtk4::Orientation::Vertical, -1);
                child.allocate(width.max(min_w), height.max(min_h), baseline, None);
            }
            let hook = self.on_width.borrow().clone();
            if let Some(hook) = hook {
                day_spec::ffi_guard::contain((), || hook(width));
            }
        }
    }
}

glib::wrapper! {
    pub struct SlotHost(ObjectSubclass<slot_host::SlotHostImp>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl SlotHost {
    fn new(natural_height: bool) -> Self {
        use gtk4::subclass::prelude::ObjectSubclassIsExt;
        let host: Self = glib::Object::new();
        host.imp().natural_height.set(natural_height);
        host
    }
    fn set_child(&self, child: &gtk4::Widget) {
        child.set_parent(self);
    }
    fn on_width(&self, f: impl Fn(i32) + 'static) {
        use gtk4::subclass::prelude::ObjectSubclassIsExt;
        *self.imp().on_width.borrow_mut() = Some(Rc::new(f));
    }
}

// --- the slot: a one-child Fixed, laid out by GTK once it is in its row ---------------------

/// Places the slot's one child at the origin: at its natural size, or at the width the host
/// granted a filling slot.
struct SlotLayout {
    shared: Rc<SlotShared>,
}

impl Layout for SlotLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let Some(&c) = children.first() else {
            // A marker: a node with a frame, so a script finds it visible.
            return Size::new(1.0, 1.0);
        };
        let width = match self.shared.kind {
            SlotKind::Fill => self.shared.width.get().or(p.width),
            _ => None,
        };
        let s = cx.measure_child(c, Proposal::new(width, None));
        Size::new(width.unwrap_or(s.width), s.height)
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

struct Slot {
    shared: Rc<SlotShared>,
    child: Option<AnyPiece>,
    /// The action a marker runs on a (scripted) tap.
    on_tap: Option<Rc<dyn Fn()>>,
    /// The row's element id, when the slot node carries it (a marker, a busy action).
    id: Option<String>,
}

impl Piece for Slot {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Slot {
            shared,
            child,
            on_tap,
            id,
        } = self;
        let node = cx.native(
            SLOT_KIND,
            &SlotProps {
                key: shared.key,
                kind: shared.kind,
            },
            Rc::new(SlotLayout {
                shared: shared.clone(),
            }),
            Flex::default(),
            Boundary::No,
        );
        shared.node.set(Some(node));
        if let Some(id) = id {
            with_tree(|t| t.set_id(node, id));
        }
        if let Some(child) = child {
            cx.under(node, |cx| {
                let _ = child.build(cx);
            });
        }
        if let Some(on) = on_tap {
            cx.on(node, move |ev| {
                if let Event::Tap(_) = ev {
                    on();
                }
            });
        }
        node
    }
}

fn slot_make(_backend: &mut Gtk, p: &SlotProps, _id: NodeId) -> gtk4::Widget {
    let fixed = gtk4::Fixed::new();
    if p.kind == SlotKind::Marker {
        // Never drawn, never a pointer target: the row underneath is the real control.
        fixed.set_visible(false);
        fixed.set_can_target(false);
        fixed.set_can_focus(false);
    }
    let w: gtk4::Widget = fixed.upcast();
    SLOT_WIDGETS.with(|m| m.borrow_mut().insert(p.key, w.clone()));
    SLOT_KEYS.with(|m| m.borrow_mut().insert(key(&w), p.key));
    w
}

fn slot_update(_backend: &mut Gtk, _h: &gtk4::Widget, _patch: &()) {}

fn slot_release(_backend: &mut Gtk, h: &gtk4::Widget) {
    if let Some(k) = SLOT_KEYS.with(|m| m.borrow_mut().remove(&key(h))) {
        SLOT_WIDGETS.with(|m| m.borrow_mut().remove(&k));
    }
}

// One `renderer!` per module: each shell kind registers from a module of its own.
mod slot_renderer {
    use super::*;
    day_pieces::renderer!(day_gtk::RENDERERS, Gtk,
        kind: SLOT_KIND, props: SlotProps, patch: (),
        make: slot_make, update: slot_update, measure: day_pieces::fill_measure, release: slot_release);
}

// --- the shell: pages, groups, rows ----------------------------------------------------------

/// A page's scrolled window (the first one under it).
fn find_scrolled(w: &gtk4::Widget) -> Option<gtk4::ScrolledWindow> {
    if let Some(sw) = w.downcast_ref::<gtk4::ScrolledWindow>() {
        return Some(sw.clone());
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        if let Some(sw) = find_scrolled(&c) {
            return Some(sw);
        }
        child = c.next_sibling();
    }
    None
}

/// The natural height of a page's content at `for_w`, measured on the scrolled window's
/// viewport so the scrolled window itself stays as libadwaita made it: it fills the page, and
/// a propagated natural height would shrink it to its content instead.
fn page_content_height(page: &gtk4::Widget, for_w: i32) -> i32 {
    let Some(sw) = find_scrolled(page) else {
        return 0;
    };
    let Some(viewport) = sw.child() else {
        return 0;
    };
    let (_, nat, _, _) = viewport.measure(gtk4::Orientation::Vertical, for_w);
    nat
}

fn make_action_row(r: &ShellRow, node: NodeId) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    // Titles are plain text from the app's catalog, never Pango markup.
    row.set_use_markup(false);
    row.set_title(&r.title);
    row.set_title_lines(1);
    row.set_subtitle_lines(2);
    if !r.subtitle.is_empty() {
        row.set_subtitle(&r.subtitle);
    }
    row.set_activatable(r.activatable);
    row.set_sensitive(r.enabled);
    if r.destructive {
        row.add_css_class("error");
    }
    if let Some(icon) = r.suffix_icon {
        let image = gtk4::Image::from_icon_name(icon);
        image.add_css_class("dim-label");
        row.add_suffix(&image);
    }
    if r.activatable {
        let k = r.key;
        row.connect_activated(move |_| {
            day_gtk::emit(node, Event::custom(ROW_TAG, k.to_string()));
        });
    }
    row
}

fn shell_make(_backend: &mut Gtk, p: &ShellProps, id: NodeId) -> gtk4::Widget {
    let mut pages = HashMap::new();
    let mut groups = HashMap::new();
    let mut rows = HashMap::new();
    let mut custom_rows = HashMap::new();
    let mut notes = HashMap::new();
    let mut pending = Vec::new();
    let mut pending_fill: Vec<(u64, SlotTarget)> = Vec::new();
    for page in &p.pages {
        let pp = adw::PreferencesPage::new();
        pp.set_title(&page.title);
        pp.set_name(Some(&page.id));
        if let Some(icon) = &page.icon {
            pp.set_icon_name(Some(icon));
        }
        for g in &page.groups {
            let group = adw::PreferencesGroup::new();
            if !g.title.is_empty() {
                group.set_title(&g.title);
            }
            if !g.description.is_empty() {
                group.set_description(Some(&g.description));
            }
            for r in &g.rows {
                match &r.kind {
                    ShellRowKind::Row { slot } => {
                        let row = make_action_row(r, id);
                        if let Some(slot) = slot {
                            pending.push((*slot, SlotTarget::Suffix(row.clone())));
                        }
                        group.add(&row);
                        rows.insert(r.key, row);
                    }
                    ShellRowKind::Custom { slot } => {
                        let row = adw::PreferencesRow::new();
                        row.set_activatable(false);
                        row.set_sensitive(r.enabled);
                        let host = SlotHost::new(true);
                        host.set_hexpand(true);
                        // The room an `AdwActionRow` keeps around its content.
                        host.set_margin_start(12);
                        host.set_margin_end(12);
                        host.set_margin_top(6);
                        host.set_margin_bottom(6);
                        row.set_child(Some(&host));
                        pending_fill.push((*slot, SlotTarget::Fill(host)));
                        group.add(&row);
                        custom_rows.insert(r.key, row);
                    }
                    ShellRowKind::Note => {
                        let label = gtk4::Label::new(Some(&r.title));
                        label.set_wrap(true);
                        label.set_xalign(0.0);
                        label.add_css_class("dim-label");
                        label.add_css_class("caption");
                        group.add(&label);
                        notes.insert(r.key, label);
                    }
                }
            }
            pp.add(&group);
            groups.insert(g.key, group);
        }
        if let Some(slot) = page.custom {
            let group = adw::PreferencesGroup::new();
            let host = SlotHost::new(true);
            host.set_hexpand(true);
            group.add(&host);
            pending_fill.push((slot, SlotTarget::Fill(host)));
            pp.add(&group);
        }
        // The page's scrolled window propagates its natural height (so a dialog can size to
        // its content) and does not expand; in a stock preferences window its viewport is
        // invisible below the content, here it would paint the view color over the bottom of
        // a short page. Let it take the page.
        if let Some(sw) = find_scrolled(pp.upcast_ref::<gtk4::Widget>()) {
            sw.set_vexpand(true);
        }
        pages.insert(page.id.clone(), pp);
    }
    pending.extend(pending_fill);
    // The host: where Day mounts the slots before `settle` moves them into their rows. With the
    // dialog it shows nothing itself; the stack fallback fills it with the pages.
    let host = gtk4::Fixed::new();
    host.set_overflow(gtk4::Overflow::Hidden);
    let shell_host = match day_gtk::preferences_dialog_of(p.root) {
        Some(dialog) => {
            for page in &p.pages {
                if let Some(pp) = pages.get(&page.id) {
                    dialog.add(pp);
                }
            }
            // The HIG: search over the rows once there is more than one page to search.
            dialog.set_search_enabled(pages.len() > 1);
            if pages.contains_key(&p.selected) {
                dialog.set_visible_page_name(&p.selected);
            }
            ShellHost::Dialog(dialog)
        }
        None => {
            let stack = adw::ViewStack::new();
            for page in &p.pages {
                if let Some(pp) = pages.get(&page.id) {
                    let sp = stack.add_titled(pp, Some(&page.id), &page.title);
                    if let Some(icon) = &page.icon {
                        sp.set_icon_name(Some(icon));
                    }
                }
            }
            if pages.contains_key(&p.selected) {
                stack.set_visible_child_name(&p.selected);
            }
            let column = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            if pages.len() > 1 {
                let switcher = adw::ViewSwitcher::new();
                switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
                switcher.set_stack(Some(&stack));
                switcher.set_margin_top(6);
                switcher.set_margin_bottom(6);
                column.append(&switcher);
            }
            stack.set_vexpand(true);
            column.append(&stack);
            // The column fills the host exactly (a `GtkFixed` would allocate it its natural
            // size).
            let wrapper = SlotHost::new(false);
            wrapper.set_child(column.upcast_ref());
            host.put(&wrapper, 0.0, 0.0);
            host.bind_property("width-request", &wrapper, "width-request")
                .sync_create()
                .build();
            host.bind_property("height-request", &wrapper, "height-request")
                .sync_create()
                .build();
            ShellHost::Stack(stack)
        }
    };
    let w: gtk4::Widget = host.upcast();
    let state = Rc::new(ShellState {
        node: id,
        host: shell_host,
        pages,
        groups,
        rows,
        custom_rows,
        notes,
        pending: RefCell::new(pending),
        suppress: Cell::new(false),
    });
    let select = {
        let st = state.clone();
        move || {
            if st.suppress.get() {
                return;
            }
            if let Some(name) = st.host.visible_page_name() {
                day_gtk::emit(id, Event::custom(SELECT_TAG, name));
            }
        }
    };
    match &state.host {
        ShellHost::Dialog(d) => {
            d.connect_visible_page_notify(move |_| select());
        }
        ShellHost::Stack(s) => {
            s.connect_visible_child_name_notify(move |_| select());
        }
    }
    SHELLS.with(|m| m.borrow_mut().insert(key(&w), state.clone()));
    SHELL_BY_NODE.with(|m| m.borrow_mut().insert(id, state));
    w
}

/// Move the slots Day mounted into the host into their rows. Runs at every measure and update,
/// so a slot mounted later (a sub-page's) still finds its row.
fn settle(st: &ShellState, host: &gtk4::Widget) {
    let Some(fixed) = host.downcast_ref::<gtk4::Fixed>() else {
        return;
    };
    let mut pending = st.pending.borrow_mut();
    pending.retain(|(k, target)| {
        let Some(slot) = SLOT_WIDGETS.with(|m| m.borrow().get(k).cloned()) else {
            return true;
        };
        if slot.parent().as_ref() != Some(host) {
            return true;
        }
        fixed.remove(&slot);
        match target {
            SlotTarget::Suffix(row) => {
                // Centered on the row like any suffix widget, whatever the row's text needs.
                slot.set_valign(gtk4::Align::Center);
                row.add_suffix(&slot);
            }
            SlotTarget::Fill(fill) => {
                let shared_key = *k;
                let node = st.node;
                fill.set_child(&slot);
                fill.on_width(move |w| {
                    let changed = FILL_WIDTHS.with(|m| {
                        let mut m = m.borrow_mut();
                        let w = f64::from(w);
                        if m.get(&shared_key) == Some(&w) {
                            false
                        } else {
                            m.insert(shared_key, w);
                            true
                        }
                    });
                    if changed {
                        // Outside the allocation GTK is running: a Day relayout sets size
                        // requests, which must not land mid-pass.
                        glib::idle_add_local_once(move || {
                            ffi_guard::contain((), || {
                                day_gtk::emit(node, Event::custom(FILL_TAG, shared_key.to_string()))
                            })
                        });
                    }
                });
            }
        }
        false
    });
}

fn shell_update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &ShellPatch) {
    let Some(st) = SHELLS.with(|m| m.borrow().get(&key(h)).cloned()) else {
        return;
    };
    settle(&st, h);
    match patch {
        ShellPatch::Select(id) => {
            if st.pages.contains_key(id) && st.host.visible_page_name().as_deref() != Some(id) {
                st.suppress.set(true);
                st.host.set_visible_page_name(id);
                st.suppress.set(false);
            }
        }
        ShellPatch::PageTitle(id, t) => {
            if let Some(pp) = st.pages.get(id) {
                pp.set_title(t);
                if let ShellHost::Stack(s) = &st.host {
                    s.page(pp).set_title(Some(t));
                }
            }
        }
        ShellPatch::GroupTitle(k, t) => {
            if let Some(g) = st.groups.get(k) {
                g.set_title(t);
            }
        }
        ShellPatch::GroupDescription(k, t) => {
            if let Some(g) = st.groups.get(k) {
                g.set_description((!t.is_empty()).then_some(t.as_str()));
            }
        }
        ShellPatch::RowTitle(k, t) => {
            if let Some(r) = st.rows.get(k) {
                r.set_title(t);
            }
        }
        ShellPatch::RowSubtitle(k, t) => {
            if let Some(r) = st.rows.get(k) {
                r.set_subtitle(t);
            }
        }
        ShellPatch::RowEnabled(k, on) => {
            if let Some(r) = st.rows.get(k) {
                r.set_sensitive(*on);
            }
            if let Some(r) = st.custom_rows.get(k) {
                r.set_sensitive(*on);
            }
        }
        ShellPatch::Note(k, t) => {
            if let Some(l) = st.notes.get(k) {
                l.set_text(t);
            }
        }
    }
}

/// The shell's natural size: the widest page, and the tallest page's content at the proposed
/// width. The dialog fits that at open (`size_to_fit`, up to the declared ceiling), so every
/// page fits without scrolling; a page taller than the ceiling scrolls inside its own
/// `AdwPreferencesPage`.
fn shell_measure(_backend: &mut Gtk, h: &gtk4::Widget, p: Proposal) -> Size {
    let Some(st) = SHELLS.with(|m| m.borrow().get(&key(h)).cloned()) else {
        return Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0));
    };
    settle(&st, h);
    let nat_w = st
        .pages
        .values()
        .map(|pp| pp.measure(gtk4::Orientation::Horizontal, -1).1)
        .max()
        .unwrap_or(0);
    let for_w = p.width.map(|w| w.round() as i32).unwrap_or(-1);
    let nat_h = st
        .pages
        .values()
        .map(|pp| page_content_height(pp.upcast_ref(), for_w))
        .max()
        .unwrap_or(0);
    Size::new(p.width.unwrap_or(f64::from(nat_w)), f64::from(nat_h))
}

fn shell_release(_backend: &mut Gtk, h: &gtk4::Widget) {
    let Some(st) = SHELLS.with(|m| m.borrow_mut().remove(&key(h))) else {
        return;
    };
    SHELL_BY_NODE.with(|m| m.borrow_mut().remove(&st.node));
    OPEN_SHELL.with(|o| {
        let mut o = o.borrow_mut();
        if o.as_ref().is_some_and(|s| Rc::ptr_eq(&s.state, &st)) {
            *o = None;
        }
    });
}

mod shell_renderer {
    use super::*;
    day_pieces::renderer!(day_gtk::RENDERERS, Gtk,
        kind: SHELL_KIND, props: ShellProps, patch: ShellPatch,
        make: shell_make, update: shell_update, measure: shell_measure, release: shell_release);
}

/// The host fills the window; its height is the pages' natural height, clamped to the
/// proposal so a tall page scrolls inside the window instead of running past it.
struct ShellLayout;

impl Layout for ShellLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        // The slots first: their sizes become the rows' suffix requests, which the pages' own
        // measure then includes.
        for &c in children {
            let _ = cx.measure_child(c, Proposal::new(None, None));
        }
        let leaf = cx.measure_leaf(p);
        let height = match p.height {
            Some(h) => h.min(leaf.height),
            None => leaf.height,
        };
        Size::new(p.width.unwrap_or(leaf.width), height)
    }

    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], _bounds: Rect) {
        // Each slot's on-screen frame is GTK's (it sits in its row); Day records the size.
        for &c in children {
            let s = cx.measure_child(c, Proposal::new(None, None));
            cx.place_child_native(c, Rect::new(0.0, 0.0, s.width, s.height));
        }
    }
}

// --- the piece: the declaration as pages, groups, rows and slots ---------------------------

/// A localized text the shell re-reads on locale change, and where it goes.
enum DynText {
    PageTitle(String),
    GroupTitle(u64),
    GroupDescription(u64),
    RowTitle(u64),
    RowSubtitle(u64),
    Note(u64),
}

impl DynText {
    fn patch(&self, t: String) -> ShellPatch {
        match self {
            DynText::PageTitle(id) => ShellPatch::PageTitle(id.clone(), t),
            DynText::GroupTitle(k) => ShellPatch::GroupTitle(*k, t),
            DynText::GroupDescription(k) => ShellPatch::GroupDescription(*k, t),
            DynText::RowTitle(k) => ShellPatch::RowTitle(*k, t),
            DynText::RowSubtitle(k) => ShellPatch::RowSubtitle(*k, t),
            DynText::Note(k) => ShellPatch::Note(*k, t),
        }
    }
}

/// Whether `page` is still on its `AdwNavigationView`'s stack: a page another sub-page was
/// pushed over, as opposed to one that was popped (and unparented).
fn on_navigation_stack(page: &adw::NavigationPage) -> bool {
    let Some(view) = page
        .parent()
        .and_then(|p| p.downcast::<adw::NavigationView>().ok())
    else {
        return false;
    };
    let stack = view.navigation_stack();
    (0..stack.n_items()).any(|i| {
        stack
            .item(i)
            .is_some_and(|o| o.downcast_ref::<adw::NavigationPage>() == Some(page))
    })
}

/// What `Shell::build` accumulates while it walks the declaration.
#[derive(Default)]
struct ShellBuild {
    slots: Vec<Slot>,
    actions: HashMap<u64, Rc<dyn Fn()>>,
    dyns: Vec<(TextSource, DynText)>,
    enabled: Vec<(u64, Reactive<bool>)>,
}

impl ShellBuild {
    fn slot(
        &mut self,
        kind: SlotKind,
        child: Option<AnyPiece>,
        on_tap: Option<Rc<dyn Fn()>>,
        id: Option<String>,
    ) -> u64 {
        let key = next_key();
        self.slots.push(Slot {
            shared: Rc::new(SlotShared {
                key,
                kind,
                width: Cell::new(None),
                node: Cell::new(None),
            }),
            child,
            on_tap,
            id,
        });
        key
    }
    fn text(&mut self, src: &TextSource, target: DynText) {
        if matches!(src, TextSource::Dyn(_)) {
            self.dyns.push((src.clone(), target));
        }
    }
}

/// One row of the declaration as a shell row, with its control (if any) as a slot.
fn shell_row(inst: Rc<Installed>, r: &Row, b: &mut ShellBuild) -> ShellRow {
    let key = next_key();
    let id = r.id.clone();
    let enabled = r.enabled.clone();
    let with_id = |p: AnyPiece| match &id {
        Some(id) => p.id(id.clone()).any(),
        None => p,
    };
    let enabled_fn = move || enabled.as_ref().is_none_or(|e| e.get());
    // What the row starts as, for the native row\'s first draw; the control tracks the rest.
    let enabled_now = enabled_fn();
    if let Some(sig) = r.enabled.clone() {
        b.enabled.push((key, sig));
    }
    b.text(&r.title, DynText::RowTitle(key));
    if let Some(c) = &r.caption {
        b.text(c, DynText::RowSubtitle(key));
    }
    let mk_row =
        |activatable: bool, suffix_icon: Option<&'static str>, kind: ShellRowKind| ShellRow {
            key,
            title: r.title.initial(),
            subtitle: r
                .caption
                .as_ref()
                .map(TextSource::initial)
                .unwrap_or_default(),
            activatable,
            suffix_icon,
            destructive: false,
            enabled: enabled_now,
            kind,
        };
    match &r.kind {
        RowKind::Toggle(sig) => {
            let ctl = with_id(toggle(*sig).enabled(enabled_fn.clone()).any());
            let slot = b.slot(SlotKind::Suffix, Some(ctl), None, None);
            // The `AdwSwitchRow` contract: the row itself flips the switch.
            let sig = *sig;
            b.actions.insert(
                key,
                Rc::new(move || {
                    if enabled_fn() {
                        sig.set(!sig.get_untracked());
                    }
                }),
            );
            mk_row(true, None, ShellRowKind::Row { slot: Some(slot) })
        }
        RowKind::Choice {
            labels,
            index,
            style,
        } => {
            let p = choice_picker(labels, *index, *style == ChoiceStyle::Segmented, enabled_fn);
            let slot = b.slot(SlotKind::Suffix, Some(with_id(p.any())), None, None);
            mk_row(false, None, ShellRowKind::Row { slot: Some(slot) })
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
                Some(i) => s
                    .id(i.clone())
                    .min_width(SUFFIX_CONTROL_W)
                    .max_width(SUFFIX_CONTROL_W)
                    .any(),
                None => s
                    .min_width(SUFFIX_CONTROL_W)
                    .max_width(SUFFIX_CONTROL_W)
                    .any(),
            };
            let ctl = match value_text {
                Some(f) => {
                    let f = f.clone();
                    let v = *value;
                    let value_label = label(move || f(v.get()))
                        .tabular()
                        .secondary()
                        .single_line()
                        .align(day_spec::props::TextAlign::Trailing);
                    let value_label = match &id {
                        Some(i) => value_label
                            .id(format!("{i}-value"))
                            .min_width(VALUE_WIDTH)
                            .max_width(VALUE_WIDTH)
                            .any(),
                        None => value_label
                            .min_width(VALUE_WIDTH)
                            .max_width(VALUE_WIDTH)
                            .any(),
                    };
                    row((ctl, value_label))
                        .spacing(8.0)
                        .align(VAlign::Center)
                        .any()
                }
                None => ctl,
            };
            let slot = b.slot(SlotKind::Suffix, Some(ctl), None, None);
            mk_row(false, None, ShellRowKind::Row { slot: Some(slot) })
        }
        RowKind::Stepper { value, range, step } => {
            let v = *value;
            let (dec, inc) = stepper_buttons(v, range, *step, enabled_fn);
            let ctl = stepper_control(v, dec, inc, id.as_deref());
            let slot = b.slot(SlotKind::Suffix, Some(ctl), None, None);
            mk_row(false, None, ShellRowKind::Row { slot: Some(slot) })
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
            let f = f.enabled(enabled_fn);
            let ctl = match &id {
                Some(i) => f
                    .id(i.clone())
                    .min_width(SUFFIX_CONTROL_W)
                    .max_width(SUFFIX_CONTROL_W)
                    .any(),
                None => f
                    .min_width(SUFFIX_CONTROL_W)
                    .max_width(SUFFIX_CONTROL_W)
                    .any(),
            };
            let slot = b.slot(SlotKind::Suffix, Some(ctl), None, None);
            mk_row(false, None, ShellRowKind::Row { slot: Some(slot) })
        }
        RowKind::Action {
            on,
            destructive,
            confirm,
            confirm_label,
            busy,
        } => {
            // The command row GNOME apps use: an activatable row titled with the command
            // (Clear Personal Data…, Manage Passwords), its caption as the subtitle; a
            // destructive one draws in the error color. The confirm dialog and the busy
            // state are the composed row's.
            let on = on.clone();
            let title = text_source_fn(&r.title);
            let confirm = confirm.clone();
            let confirm_label = confirm_label.clone();
            let busy = busy.clone();
            let (enabled_run, busy_run) = (enabled_fn.clone(), busy.clone());
            let run: Rc<dyn Fn()> = Rc::new(move || {
                if !enabled_run() || busy_run.as_ref().is_some_and(|b| b.get_untracked()) {
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
            });
            b.actions.insert(key, run.clone());
            // The id goes on a slot node that runs the command on a scripted tap: a marker,
            // or the spinner's slot while the row can be busy. A busy row is insensitive.
            let slot = match (&id, busy.clone()) {
                (None, None) => None,
                (_, Some(b_sig)) => Some(b.slot(
                    SlotKind::Suffix,
                    Some(when(move || b_sig.get(), spinner).any()),
                    Some(run.clone()),
                    id.clone(),
                )),
                (Some(_), None) => {
                    Some(b.slot(SlotKind::Marker, None, Some(run.clone()), id.clone()))
                }
            };
            if let Some(b_sig) = busy {
                let sig = Signal::global(enabled_fn() && !b_sig.get_untracked());
                day_reactive::Scope::root().enter(|| {
                    watch(
                        move || enabled_fn() && !b_sig.get(),
                        move |v, _| sig.set_if_changed(*v),
                    );
                });
                b.enabled.retain(|(k, _)| *k != key);
                b.enabled.push((key, sig.into_reactive()));
            }
            let mut sr = mk_row(true, None, ShellRowKind::Row { slot });
            sr.destructive = *destructive;
            sr
        }
        RowKind::Link(url) => {
            let url = url.clone();
            let open: Rc<dyn Fn()> = Rc::new(move || day_core::open_url(&url));
            b.actions.insert(key, open.clone());
            let slot = id
                .as_ref()
                .map(|_| b.slot(SlotKind::Marker, None, Some(open.clone()), id.clone()));
            mk_row(
                true,
                Some("adw-external-link-symbolic"),
                ShellRowKind::Row { slot },
            )
        }
        RowKind::Navigate(build) => {
            let build = build.clone();
            let title = text_source_fn(&r.title);
            let open: Rc<dyn Fn()> = Rc::new(move || {
                push_subpage(inst.clone(), title(), build.clone());
            });
            b.actions.insert(key, open.clone());
            let slot = id
                .as_ref()
                .map(|_| b.slot(SlotKind::Marker, None, Some(open.clone()), id.clone()));
            mk_row(true, Some("go-next-symbolic"), ShellRowKind::Row { slot })
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
            let slot = b.slot(SlotKind::Suffix, Some(with_id(v)), None, None);
            mk_row(false, None, ShellRowKind::Row { slot: Some(slot) })
        }
        RowKind::Note => {
            b.text(&r.title, DynText::Note(key));
            mk_row(false, None, ShellRowKind::Note)
        }
        RowKind::Custom(build) => {
            let slot = b.slot(SlotKind::Fill, Some(with_id(build())), None, None);
            mk_row(false, None, ShellRowKind::Custom { slot })
        }
    }
}

struct Shell {
    inst: Rc<Installed>,
}

impl Piece for Shell {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let inst = self.inst;
        let mut b = ShellBuild::default();
        let mut pages = Vec::new();
        for g in inst.visible_groups() {
            let mut groups = Vec::new();
            for s in &g.sections {
                let rows: Vec<&Row> = s.rows.iter().filter(|r| r.is_available()).collect();
                if rows.is_empty() {
                    continue;
                }
                let gkey = next_key();
                if let Some(t) = &s.title {
                    b.text(t, DynText::GroupTitle(gkey));
                }
                if let Some(f) = &s.footer {
                    b.text(f, DynText::GroupDescription(gkey));
                }
                let rows = rows
                    .into_iter()
                    .map(|r| shell_row(inst.clone(), r, &mut b))
                    .collect();
                groups.push(ShellGroup {
                    key: gkey,
                    title: s
                        .title
                        .as_ref()
                        .map(TextSource::initial)
                        .unwrap_or_default(),
                    description: s
                        .footer
                        .as_ref()
                        .map(TextSource::initial)
                        .unwrap_or_default(),
                    rows,
                });
            }
            let custom = g
                .custom
                .as_ref()
                .map(|build| b.slot(SlotKind::Fill, Some(build()), None, None));
            let icon = match &g.icon {
                Some(GroupIcon::Symbol(s)) => day_gtk::symbol_icon_name(*s),
                Some(GroupIcon::Image(n)) => day_gtk::vector_icon_name(n.as_str()),
                None => None,
            };
            b.text(&g.title, DynText::PageTitle(g.id.to_string()));
            pages.push(ShellPage {
                id: g.id.to_string(),
                title: g.title.initial(),
                icon,
                groups,
                custom,
            });
        }
        let ShellBuild {
            slots,
            actions,
            dyns,
            enabled,
        } = b;
        let fills: Rc<RefCell<Vec<Rc<SlotShared>>>> = Rc::new(RefCell::new(
            slots
                .iter()
                .filter(|s| s.shared.kind == SlotKind::Fill)
                .map(|s| s.shared.clone())
                .collect(),
        ));
        let node = cx.native(
            SHELL_KIND,
            &ShellProps {
                root: day_core::rnode_to_id(cx.parent()),
                pages,
                selected: inst.selected.get_untracked(),
            },
            Rc::new(ShellLayout),
            Flex {
                grow_w: true,
                grow_h: true,
                ..Default::default()
            },
            Boundary::Yes,
        );
        cx.under(node, |cx| {
            for s in slots {
                let _ = s.build(cx);
            }
        });
        // The realized shell, for a navigate row's sub-page push (`push_subpage`).
        if let Some(state) =
            SHELL_BY_NODE.with(|m| m.borrow().get(&day_core::rnode_to_id(node)).cloned())
        {
            OPEN_SHELL.with(|o| {
                *o.borrow_mut() = Some(OpenShell {
                    node,
                    owner: day_reactive::Scope::current(),
                    fills: fills.clone(),
                    state,
                })
            });
        }
        let i = inst.clone();
        let fill_slots = fills.clone();
        cx.on(node, move |ev| {
            if let Event::Custom { tag, text, .. } = ev {
                match *tag {
                    SELECT_TAG => i.selected.set_if_changed(text.clone()),
                    ROW_TAG => {
                        if let Some(run) = text.parse::<u64>().ok().and_then(|k| actions.get(&k)) {
                            run();
                        }
                    }
                    FILL_TAG => {
                        let k = text.parse::<u64>().ok();
                        for s in fill_slots.borrow().iter() {
                            if k.is_some_and(|k| k != s.key) {
                                continue;
                            }
                            let w = FILL_WIDTHS.with(|m| m.borrow().get(&s.key).copied());
                            s.width.set(w);
                            if let Some(n) = s.node.get() {
                                day_core::invalidate_size(n);
                            }
                        }
                    }
                    _ => {}
                }
            }
        });
        // A selection made elsewhere (`open_at`, a route) shows that page too.
        let i = inst.clone();
        watch(
            move || i.selected.get(),
            move |id, _| {
                let id = id.clone();
                with_tree(|t| t.patch(node, Box::new(ShellPatch::Select(id)), false));
            },
        );
        // Localized titles, captions, section texts and notes follow the locale.
        for (src, target) in dyns {
            if let TextSource::Dyn(f) = src {
                watch(
                    move || f(),
                    move |t, prev| {
                        if prev.is_some() {
                            let affects_size = true;
                            with_tree(|tree| {
                                tree.patch(node, Box::new(target.patch(t.clone())), affects_size)
                            });
                        }
                    },
                );
            }
        }
        for (k, sig) in enabled {
            watch(
                move || sig.get(),
                move |on, _| {
                    with_tree(|t| t.patch(node, Box::new(ShellPatch::RowEnabled(k, *on)), false));
                },
            );
        }
        node
    }
}

/// The preferences window's body: the real preferences-window structure around the visible
/// groups. `None` with nothing to show.
pub(super) fn window_shell(inst: Rc<Installed>) -> Option<AnyPiece> {
    let groups = inst.visible_groups();
    if groups.is_empty() {
        return None;
    }
    let selected = inst.selected.get_untracked();
    if !groups.iter().any(|g| g.id == selected) {
        inst.selected.set(groups[0].id.to_string());
    }
    Some(AnyPiece::new(Shell { inst }))
}

// ---------------------------------------------------------------------------
// Sub-pages: an AdwNavigationPage pushed into the dialog
// ---------------------------------------------------------------------------

/// Push a navigate row's sub-page into the open preferences dialog as an `AdwNavigationPage`
/// (the dialog's navigation view adds the back button to the page's header bar). The Day
/// sub-page is a filling slot built under the shell in a scope of its own and moved into the
/// page by `settle`, so it lays out at the page's width like a custom row does. Popping the
/// page disposes the slot and its scope. `false` when no dialog shell is open (the row is in a
/// page shown on its own, or the shell went into a window), which keeps the caller's own path.
pub(super) fn push_subpage(inst: Rc<Installed>, title: String, build: Build) -> bool {
    if inst.presentation != Presentation::Window {
        return false;
    }
    let Some((node, owner, fills, state)) = OPEN_SHELL.with(|o| {
        o.borrow()
            .as_ref()
            .map(|s| (s.node, s.owner, s.fills.clone(), s.state.clone()))
    }) else {
        return false;
    };
    let ShellHost::Dialog(dialog) = &state.host else {
        return false;
    };
    if !with_tree(|t| t.node_exists(node)) {
        return false;
    }
    // The page: a header bar over a scrolling, clamped column, the shape of an
    // `AdwPreferencesPage`, holding the Day sub-page at the column's width.
    let fill = SlotHost::new(true);
    fill.set_hexpand(true);
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(600);
    clamp.set_tightening_threshold(400);
    clamp.set_margin_start(12);
    clamp.set_margin_end(12);
    clamp.set_margin_top(24);
    clamp.set_margin_bottom(24);
    clamp.set_child(Some(&fill));
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&clamp));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scroll));
    let page = adw::NavigationPage::new(&toolbar, &title);
    // The slot, pending its move into the page.
    let key = next_key();
    let shared = Rc::new(SlotShared {
        key,
        kind: SlotKind::Fill,
        width: Cell::new(None),
        node: Cell::new(None),
    });
    fills.borrow_mut().push(shared.clone());
    state
        .pending
        .borrow_mut()
        .push((key, SlotTarget::Fill(fill)));
    let scope = owner.enter(day_reactive::Scope::child);
    scope.enter(|| {
        let mut cx = BuildCx::new(node);
        let _ = Slot {
            shared: shared.clone(),
            child: Some(build()),
            on_tap: None,
            id: None,
        }
        .build(&mut cx);
    });
    // The shell's next measure settles the slot into the page.
    with_tree(|t| {
        t.mark_layout_dirty();
        t.layout_if_needed();
    });
    dialog.push_subpage(&page);
    // Popped (the back button, a swipe, Escape): the sub-page's tree and scope go with it.
    // `hidden` also fires when a further sub-page is pushed over this one, so it is only the
    // cue to look: a page that is still on the dialog's navigation stack stays alive, and
    // its `hidden` after the pop (no longer on the stack, unparented) is the one that cleans
    // up.
    let popped = Cell::new(false);
    page.connect_hidden(move |page| {
        ffi_guard::contain((), || {
            if popped.get() || on_navigation_stack(page) {
                return;
            }
            popped.set(true);
            scope.dispose();
            fills.borrow_mut().retain(|s| s.key != key);
            FILL_WIDTHS.with(|m| m.borrow_mut().remove(&key));
            if let Some(n) = shared.node.get()
                && with_tree(|t| t.node_exists(n))
            {
                with_tree(|t| {
                    t.remove_subtree(n);
                    t.mark_layout_dirty();
                });
            }
        })
    });
    true
}
