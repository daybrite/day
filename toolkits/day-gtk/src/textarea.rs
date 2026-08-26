// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// GTK: a GtkTextView inside a GtkScrolledWindow (vertical scrolling only), wrapped in a GtkOverlay that
// carries a dim placeholder GtkLabel pinned top-left (GtkTextView has no native placeholder). The
// buffer's "changed" signal fires on user input AND on programmatic set_text, so a per-node `suppress`
// cell guards the programmatic sync in `update` from echoing back as an Event::TextChanged. `measure`
// grows the editor's height with its content between `min_lines` and `max_lines`, then the scrolled
// window scrolls.
// ---------------------------------------------------------------------------

use day_spec::Event;
use day_spec::props::{TextAreaPatch as TextPatch, TextAreaProps as TextProps};
use std::cell::Cell;
use std::rc::Rc;

use crate::Gtk;
use day_spec::sidetable::SideTable;
use day_spec::{NodeId, Proposal, Size, ffi_guard};
use gtk4::prelude::*;

// Text view margins (px) — a little breathing room around the text; `PAD` is their vertical sum, used
// so the line-based min/max height band lines up with the measured content height.
const MARGIN_V: i32 = 4;
const MARGIN_H: i32 = 6;
const PAD: f64 = (2 * MARGIN_V) as f64;

struct TAState {
    textview: gtk4::TextView,
    buffer: gtk4::TextBuffer,
    suppress: Rc<Cell<bool>>,
    line_h: f64,
    min_lines: u32,
    max_lines: u32,
}

day_core::tls_group! {
    /// Overlay widget ptr → text-area state. A [`SideTable`]: no local release path exists
    /// here, so the backend's release sweep is what drops a dead editor's entry.
    static STATE: SideTable<TAState> = SideTable::new();

}

fn key(w: &gtk4::Widget) -> usize {
    w.as_ptr() as usize
}

fn buffer_text(b: &gtk4::TextBuffer) -> String {
    b.text(&b.start_iter(), &b.end_iter(), false).to_string()
}

fn make(_backend: &mut Gtk, p: &TextProps, id: NodeId) -> gtk4::Widget {
    let textview = gtk4::TextView::new();
    textview.set_wrap_mode(gtk4::WrapMode::WordChar);
    // GtkTextView honors editability; it's always selectable and has no built-in spell-check
    // (Cap::TextSelectable / Cap::TextSpellCheck are Unsupported here), so those props are no-ops.
    textview.set_editable(p.editable);
    textview.set_top_margin(MARGIN_V);
    textview.set_bottom_margin(MARGIN_V);
    textview.set_left_margin(MARGIN_H);
    textview.set_right_margin(MARGIN_H);
    let buffer = textview.buffer();
    if !p.text.is_empty() {
        buffer.set_text(&p.text);
    }

    // One-line pixel height for this view's font, for the line-based min/max band.
    let ctx = textview.pango_context();
    let layout = gtk4::pango::Layout::new(&ctx);
    layout.set_text("Ag");
    let line_h = layout.pixel_size().1 as f64;

    // Submit-on-enter: claim a plain Return/Enter press as `Event::Submitted`; Shift+Return
    // falls through to the buffer as a line break. CAPTURE phase, so the press never reaches
    // the GtkTextView's own key handling.
    if p.submit_on_enter {
        let keys = gtk4::EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, keyval, _, state| {
            // Contained with Proceed: a panic must not swallow the key press.
            ffi_guard::contain(gtk4::glib::Propagation::Proceed, || {
                let enter = matches!(keyval, gtk4::gdk::Key::Return | gtk4::gdk::Key::KP_Enter);
                if enter && !state.contains(gtk4::gdk::ModifierType::SHIFT_MASK) {
                    crate::emit(id, Event::Submitted);
                    return gtk4::glib::Propagation::Stop;
                }
                gtk4::glib::Propagation::Proceed
            })
        });
        textview.add_controller(keys);
    }

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_hexpand(true);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&textview));

    let overlay = gtk4::Overlay::new();
    overlay.set_child(Some(&scroll));

    // Placeholder overlay: a dim label at the text origin, non-interactive, shown only when empty.
    let placeholder = gtk4::Label::new(Some(&p.placeholder));
    placeholder.add_css_class("dim-label");
    placeholder.set_halign(gtk4::Align::Start);
    placeholder.set_valign(gtk4::Align::Start);
    placeholder.set_margin_start(MARGIN_H);
    placeholder.set_margin_top(MARGIN_V);
    placeholder.set_can_target(false);
    placeholder.set_visible(p.text.is_empty());
    overlay.add_overlay(&placeholder);

    let suppress = Rc::new(Cell::new(false));
    let sup = suppress.clone();
    let ph = placeholder.clone();
    buffer.connect_changed(move |b| {
        ffi_guard::contain((), || {
            let text = buffer_text(b);
            ph.set_visible(text.is_empty());
            if sup.get() {
                return;
            }
            crate::emit(id, Event::TextChanged(text));
        });
    });

    let w: gtk4::Widget = overlay.upcast();
    STATE.with(|m| {
        m.insert(
            key(&w),
            TAState {
                textview,
                buffer,
                suppress,
                line_h,
                min_lines: p.min_lines,
                max_lines: p.max_lines,
            },
        )
    });
    w
}

fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &TextPatch) {
    STATE.with(|m| {
        m.with(key(h), |st| match patch {
            TextPatch::SetText(t) => {
                if buffer_text(&st.buffer) != *t {
                    st.suppress.set(true);
                    st.buffer.set_text(t); // fires "changed" → placeholder updates; emit suppressed
                    st.suppress.set(false);
                }
            }
            TextPatch::SetEditable(v) => st.textview.set_editable(*v),
            // GtkTextView has no selectable/spell-check toggle (see the make() note).
            TextPatch::SetSelectable(_) | TextPatch::SetSpellCheck(_) => {}
        });
    });
}

fn measure(_backend: &mut Gtk, h: &gtk4::Widget, p: Proposal) -> Size {
    STATE
        .with(|m| {
            m.with(key(h), |st| {
                let (_, nat_w, _, _) = st.textview.measure(gtk4::Orientation::Horizontal, -1);
                let avail_w = p.width.unwrap_or(nat_w as f64).max(120.0);
                // Natural content height at the proposed width (includes the text view margins).
                let (_, nat_h, _, _) = st
                    .textview
                    .measure(gtk4::Orientation::Vertical, avail_w as i32);
                let min_h = (st.min_lines as f64) * st.line_h + PAD;
                let max_h = if st.max_lines > 0 {
                    (st.max_lines as f64) * st.line_h + PAD
                } else {
                    f64::MAX
                };
                let hgt = (nat_h as f64).clamp(min_h, max_h);
                Size::new(avail_w, hgt)
            })
        })
        .unwrap_or_else(|| {
            let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
            Size::new(p.width.unwrap_or(nat_w as f64).max(120.0), 44.0)
        })
}

// Built-in dispatch adapters: the backend's realize/update matches call these (the downcasts
// the satellite-era `renderer!` macro used to generate).
pub(crate) fn realize_any(
    b: &mut crate::Gtk,
    props: &dyn std::any::Any,
    id: day_spec::NodeId,
) -> crate::Handle {
    // A wrong payload degrades to the placeholder (props_of reports it) — never a panic.
    match day_spec::props_of::<TextProps>(day_spec::kinds::TEXT_AREA, "gtk", props) {
        Some(p) => make(b, p, id),
        None => crate::placeholder_label(day_spec::kinds::TEXT_AREA),
    }
}

pub(crate) fn update_any(b: &mut crate::Gtk, h: &crate::Handle, patch: &dyn std::any::Any) {
    if let Some(p) = patch.downcast_ref::<TextPatch>() {
        update(b, h, p);
    }
}

pub(crate) fn measure_any(
    b: &mut crate::Gtk,
    h: &crate::Handle,
    p: day_spec::Proposal,
) -> day_spec::Size {
    measure(b, h, p)
}
