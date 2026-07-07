// ---------------------------------------------------------------------------
// GTK: GtkSearchEntry — a native search entry (magnifier + clear icon). Its "search-changed" signal
// fires on user input AND on programmatic set_text, so a per-node `suppress` cell guards the
// programmatic sync in `update` from echoing back as an Event::TextChanged.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use day_gtk::Gtk;
use day_spec::{NodeId, Proposal, Size};
use gtk4::prelude::*;

struct SearchState {
    entry: gtk4::SearchEntry,
    suppress: Rc<Cell<bool>>,
}

thread_local! {
    static STATE: RefCell<HashMap<usize, SearchState>> = RefCell::new(HashMap::new());
}

fn key(w: &gtk4::Widget) -> usize {
    w.as_ptr() as usize
}

fn make(_backend: &mut Gtk, p: &SearchProps, id: NodeId) -> gtk4::Widget {
    let entry = gtk4::SearchEntry::new();
    if !p.placeholder.is_empty() {
        entry.set_placeholder_text(Some(&p.placeholder));
    }
    if !p.text.is_empty() {
        entry.set_text(&p.text);
    }
    let suppress = Rc::new(Cell::new(false));
    let sup = suppress.clone();
    entry.connect_search_changed(move |e| {
        if sup.get() {
            return;
        }
        day_gtk::emit(id, Event::TextChanged(e.text().to_string()));
    });
    let w: gtk4::Widget = entry.clone().upcast();
    STATE.with(|m| {
        m.borrow_mut()
            .insert(key(&w), SearchState { entry, suppress })
    });
    w
}

fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &SearchPatch) {
    let SearchPatch::SetText(t) = patch;
    STATE.with(|m| {
        let m = m.borrow();
        let Some(st) = m.get(&key(h)) else {
            return;
        };
        if st.entry.text().as_str() != t {
            st.suppress.set(true);
            st.entry.set_text(t);
            st.suppress.set(false);
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
    kind: KIND, props: SearchProps, patch: SearchPatch,
    make: make, update: update, measure: measure);
