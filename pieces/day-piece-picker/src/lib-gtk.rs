// ---------------------------------------------------------------------------
// GTK: GtkDropDown (menu) / `.linked` grouped ToggleButtons (segmented) / grouped
// CheckButton radios (inline). Echo-guarded so programmatic selection doesn't loop.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use day_gtk::Gtk;
use day_spec::{NodeId, Proposal, Size};
use gtk4::prelude::*;

struct PickerState {
    dropdown: Option<gtk4::DropDown>,
    toggles: Vec<gtk4::ToggleButton>, // segmented
    checks: Vec<gtk4::CheckButton>,   // inline (radio)
    suppress: Rc<Cell<bool>>,
}

thread_local! {
    static STATE: RefCell<HashMap<usize, PickerState>> = RefCell::new(HashMap::new());
}

fn key(w: &gtk4::Widget) -> usize {
    w.as_ptr() as usize
}

fn make_menu(p: &PickerProps, id: NodeId, suppress: Rc<Cell<bool>>) -> gtk4::DropDown {
    let refs: Vec<&str> = p.options.iter().map(|s| s.as_str()).collect();
    let dd = gtk4::DropDown::new(Some(gtk4::StringList::new(&refs)), gtk4::Expression::NONE);
    dd.set_selected(p.selected as u32);
    dd.connect_selected_notify(move |d| {
        if suppress.get() {
            return;
        }
        let sel = d.selected();
        if sel != gtk4::INVALID_LIST_POSITION {
            day_gtk::emit(id, Event::SelectionChanged(sel as i64));
        }
    });
    dd
}

fn make(_backend: &mut Gtk, p: &PickerProps, id: NodeId) -> gtk4::Widget {
    let suppress = Rc::new(Cell::new(false));
    let (root, state): (gtk4::Widget, PickerState) = match p.style {
        PickerStyle::Menu => {
            let dd = make_menu(p, id, suppress.clone());
            (
                dd.clone().upcast(),
                PickerState {
                    dropdown: Some(dd),
                    toggles: vec![],
                    checks: vec![],
                    suppress,
                },
            )
        }
        PickerStyle::Segmented => {
            let bx = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            bx.add_css_class("linked"); // segmented appearance
            bx.set_halign(gtk4::Align::Start);
            let mut toggles = Vec::new();
            for (i, opt) in p.options.iter().enumerate() {
                let t = gtk4::ToggleButton::with_label(opt);
                if let Some(first) = toggles.first() {
                    t.set_group(Some(first)); // mutually exclusive
                }
                let suppress = suppress.clone();
                t.connect_toggled(move |t| {
                    if suppress.get() || !t.is_active() {
                        return;
                    }
                    day_gtk::emit(id, Event::SelectionChanged(i as i64));
                });
                bx.append(&t);
                toggles.push(t);
            }
            if let Some(t) = toggles.get(p.selected) {
                suppress.set(true);
                t.set_active(true);
                suppress.set(false);
            }
            (
                bx.upcast(),
                PickerState {
                    dropdown: None,
                    toggles,
                    checks: vec![],
                    suppress,
                },
            )
        }
        PickerStyle::Inline => {
            let bx = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            bx.set_halign(gtk4::Align::Start);
            let mut checks = Vec::new();
            for (i, opt) in p.options.iter().enumerate() {
                let c = gtk4::CheckButton::with_label(opt); // grouped ⇒ radio
                if let Some(first) = checks.first() {
                    c.set_group(Some(first));
                }
                let suppress = suppress.clone();
                c.connect_toggled(move |c| {
                    if suppress.get() || !c.is_active() {
                        return;
                    }
                    day_gtk::emit(id, Event::SelectionChanged(i as i64));
                });
                bx.append(&c);
                checks.push(c);
            }
            if let Some(c) = checks.get(p.selected) {
                suppress.set(true);
                c.set_active(true);
                suppress.set(false);
            }
            (
                bx.upcast(),
                PickerState {
                    dropdown: None,
                    toggles: vec![],
                    checks,
                    suppress,
                },
            )
        }
    };
    STATE.with(|m| m.borrow_mut().insert(key(&root), state));
    root
}

fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &PickerPatch) {
    let PickerPatch::Selected(i) = patch;
    let i = *i;
    STATE.with(|m| {
        let m = m.borrow();
        let Some(st) = m.get(&key(h)) else {
            return;
        };
        st.suppress.set(true);
        if let Some(dd) = &st.dropdown {
            if dd.selected() as usize != i {
                dd.set_selected(i as u32);
            }
        } else if let Some(t) = st.toggles.get(i) {
            t.set_active(true);
        } else if let Some(c) = st.checks.get(i) {
            c.set_active(true);
        }
        st.suppress.set(false);
    });
}

fn measure(_backend: &mut Gtk, h: &gtk4::Widget, _p: Proposal) -> Size {
    let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
    let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
    Size::new((nat_w as f64).max(60.0), (nat_h as f64).max(22.0))
}

day_pieces::renderer!(day_gtk::RENDERERS, Gtk,
    kind: KIND, props: PickerProps, patch: PickerPatch,
    make: make, update: update, measure: measure);
