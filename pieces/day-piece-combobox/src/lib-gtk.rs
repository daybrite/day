// ---------------------------------------------------------------------------
// GTK: GtkComboBoxText WITH an entry — GTK's real combo box (free text + a dropdown of items).
// The internal GtkEntry's "changed" signal is the single change path: it fires on typing AND
// when picking a dropdown item (the pick writes the entry), so both report as TextChanged. It
// also fires on programmatic set_text, so a per-node `suppress` cell guards the sync in
// `update` from echoing back.
//
// GTK 4.10 deprecated GtkComboBoxText without shipping an editable replacement (GtkDropDown
// has no entry), so this renderer keeps it deliberately — hence the file-wide
// allow(deprecated). Revisit if GTK grows an editable dropdown.
// ---------------------------------------------------------------------------
#![allow(deprecated)]

use super::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use day_gtk::Gtk;
use day_spec::{NodeId, Proposal, Size};
use gtk4::prelude::*;

struct ComboState {
    combo: gtk4::ComboBoxText,
    entry: gtk4::Entry,
    suppress: Rc<Cell<bool>>,
}

thread_local! {
    static STATE: RefCell<HashMap<usize, ComboState>> = RefCell::new(HashMap::new());
}

fn key(w: &gtk4::Widget) -> usize {
    w.as_ptr() as usize
}

fn make(_backend: &mut Gtk, p: &ComboProps, id: NodeId) -> gtk4::Widget {
    let combo = gtk4::ComboBoxText::with_entry();
    for item in &p.items {
        combo.append_text(item);
    }
    let w: gtk4::Widget = combo.clone().upcast();
    // with_entry() always builds a GtkEntry child; if that ever stops holding, render the bare
    // combo unbound rather than crash.
    let Some(entry) = combo.child().and_then(|c| c.downcast::<gtk4::Entry>().ok()) else {
        return w;
    };
    if !p.placeholder.is_empty() {
        entry.set_placeholder_text(Some(&p.placeholder));
    }
    if !p.text.is_empty() {
        entry.set_text(&p.text);
    }
    let suppress = Rc::new(Cell::new(false));
    let sup = suppress.clone();
    entry.connect_changed(move |e| {
        if sup.get() {
            return;
        }
        day_gtk::emit(id, Event::TextChanged(e.text().to_string()));
    });
    STATE.with(|m| {
        m.borrow_mut().insert(
            key(&w),
            ComboState {
                combo,
                entry,
                suppress,
            },
        )
    });
    w
}

fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &ComboPatch) {
    STATE.with(|m| {
        let m = m.borrow();
        let Some(st) = m.get(&key(h)) else {
            return;
        };
        match patch {
            ComboPatch::Items(items) => {
                // The text is the value and must survive the list swap (remove_all resets the
                // active item, which can rewrite the entry).
                let keep = st.entry.text().to_string();
                st.suppress.set(true);
                st.combo.remove_all();
                for item in items {
                    st.combo.append_text(item);
                }
                if st.entry.text().as_str() != keep {
                    st.entry.set_text(&keep);
                }
                st.suppress.set(false);
            }
            ComboPatch::SetText(t) => {
                if st.entry.text().as_str() != t {
                    st.suppress.set(true);
                    st.entry.set_text(t);
                    st.suppress.set(false);
                }
            }
        }
    });
}

fn measure(_backend: &mut Gtk, h: &gtk4::Widget, p: Proposal) -> Size {
    // Grow to the proposed width; natural (single-line) height.
    let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
    let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
    let w = p.width.unwrap_or(nat_w as f64).max(120.0);
    Size::new(w, (nat_h as f64).max(24.0))
}

day_pieces::renderer!(day_gtk::RENDERERS, Gtk,
    kind: KIND, props: ComboProps, patch: ComboPatch,
    make: make, update: update, measure: measure);
