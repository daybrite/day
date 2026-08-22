// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// GTK: GtkDropDown (menu) / `.linked` grouped ToggleButtons (segmented) / grouped
// CheckButton radios (inline). Echo-guarded so programmatic selection doesn't loop.
// ---------------------------------------------------------------------------

use day_spec::Event;
use day_spec::props::{PickerPatch, PickerProps, PickerStyle};
use std::cell::Cell;
use std::rc::Rc;

use crate::Gtk;
use day_spec::sidetable::SideTable;
use day_spec::{NodeId, Proposal, Size, ffi_guard};
use gtk4::prelude::*;

struct PickerState {
    dropdown: Option<gtk4::DropDown>,
    toggles: Vec<gtk4::ToggleButton>, // segmented
    checks: Vec<gtk4::CheckButton>,   // inline (radio)
    suppress: Rc<Cell<bool>>,
}

thread_local! {
    /// Root widget ptr → picker state. A [`SideTable`]: no local release path exists here, so
    /// the backend's release sweep is what drops a dead picker's entry.
    static STATE: SideTable<PickerState> = SideTable::new();
    /// Root widget ptr → the picker's node, so an option patch can wire the handlers of the
    /// buttons it adds (they emit for this node).
    static NODES: SideTable<NodeId> = SideTable::new();
}

fn key(w: &gtk4::Widget) -> usize {
    w.as_ptr() as usize
}

fn make_menu(p: &PickerProps, id: NodeId, suppress: Rc<Cell<bool>>) -> gtk4::DropDown {
    let refs: Vec<&str> = p.options.iter().map(|s| s.as_str()).collect();
    let dd = gtk4::DropDown::new(Some(gtk4::StringList::new(&refs)), gtk4::Expression::NONE);
    dd.set_selected(p.selected as u32);
    dd.connect_selected_notify(move |d| {
        ffi_guard::contain((), || {
            if suppress.get() {
                return;
            }
            let sel = d.selected();
            if sel != gtk4::INVALID_LIST_POSITION {
                crate::emit(id, Event::SelectionChanged(sel as i64));
            }
        });
    });
    dd
}

/// One segmented button's "I am the selection now" handler. Factored so [`set_options`] can
/// wire a button it adds later — the index is captured, so it must be per button.
fn wire_toggle(t: &gtk4::ToggleButton, id: NodeId, i: usize, suppress: Rc<Cell<bool>>) {
    t.connect_toggled(move |t| {
        ffi_guard::contain((), || {
            if suppress.get() || !t.is_active() {
                return;
            }
            crate::emit(id, Event::SelectionChanged(i as i64));
        });
    });
}

fn wire_check(c: &gtk4::CheckButton, id: NodeId, i: usize, suppress: Rc<Cell<bool>>) {
    c.connect_toggled(move |c| {
        ffi_guard::contain((), || {
            if suppress.get() || !c.is_active() {
                return;
            }
            crate::emit(id, Event::SelectionChanged(i as i64));
        });
    });
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
                wire_toggle(&t, id, i, suppress.clone());
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
                wire_check(&c, id, i, suppress.clone());
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
    STATE.with(|m| m.insert(key(&root), state));
    NODES.with(|m| m.insert(key(&root), id));
    root
}

fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &PickerPatch) {
    let i = match patch {
        PickerPatch::Selected(i) => *i,
        PickerPatch::Options(opts) => return set_options(h, opts),
    };
    STATE.with(|m| {
        m.with(key(h), |st| {
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
    });
}

/// New option labels, in place — the selected index survives where it still exists.
///
/// The dropdown swaps its model; the button styles relabel what they have and add or drop
/// the tail, because each button's handler captures its OWN index (a rebuilt button needs a
/// fresh handler, a relabeled one does not).
fn set_options(h: &gtk4::Widget, opts: &[String]) {
    let Some(node) = NODES.with(|m| m.get(key(h))) else {
        return;
    };
    STATE.with(|m| {
        m.with(key(h), |st| {
            st.suppress.set(true);
            if let Some(dd) = &st.dropdown {
                let want = dd.selected();
                let refs: Vec<&str> = opts.iter().map(|s| s.as_str()).collect();
                dd.set_model(Some(&gtk4::StringList::new(&refs)));
                if !opts.is_empty() {
                    dd.set_selected(want.min(opts.len() as u32 - 1));
                }
            } else if !st.toggles.is_empty() || !st.checks.is_empty() {
                let segmented = !st.toggles.is_empty();
                let parent = h.clone().downcast::<gtk4::Box>().ok();
                let active = if segmented {
                    st.toggles.iter().position(|t| t.is_active())
                } else {
                    st.checks.iter().position(|c| c.is_active())
                }
                .unwrap_or(0);
                for (i, o) in opts.iter().enumerate() {
                    if segmented {
                        match st.toggles.get(i) {
                            Some(t) => t.set_label(o),
                            None => {
                                let t = gtk4::ToggleButton::with_label(o);
                                if let Some(first) = st.toggles.first() {
                                    t.set_group(Some(first));
                                }
                                wire_toggle(&t, node, i, st.suppress.clone());
                                if let Some(bx) = &parent {
                                    bx.append(&t);
                                }
                                st.toggles.push(t);
                            }
                        }
                    } else {
                        match st.checks.get(i) {
                            Some(c) => c.set_label(Some(o)),
                            None => {
                                let c = gtk4::CheckButton::with_label(o);
                                if let Some(first) = st.checks.first() {
                                    c.set_group(Some(first));
                                }
                                wire_check(&c, node, i, st.suppress.clone());
                                if let Some(bx) = &parent {
                                    bx.append(&c);
                                }
                                st.checks.push(c);
                            }
                        }
                    }
                }
                if let Some(bx) = &parent {
                    for t in st.toggles.drain(opts.len().min(st.toggles.len())..) {
                        bx.remove(&t);
                    }
                    for c in st.checks.drain(opts.len().min(st.checks.len())..) {
                        bx.remove(&c);
                    }
                }
                let keep = active.min(opts.len().saturating_sub(1));
                if let Some(t) = st.toggles.get(keep) {
                    t.set_active(true);
                }
                if let Some(c) = st.checks.get(keep) {
                    c.set_active(true);
                }
            }
            st.suppress.set(false);
        })
    });
}

fn measure(_backend: &mut Gtk, h: &gtk4::Widget, _p: Proposal) -> Size {
    let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
    let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
    Size::new((nat_w as f64).max(60.0), (nat_h as f64).max(22.0))
}

// Built-in dispatch adapters: the backend's realize/update matches call these (the downcasts
// the satellite-era `renderer!` macro used to generate).
pub(crate) fn realize_any(
    b: &mut crate::Gtk,
    props: &dyn std::any::Any,
    id: day_spec::NodeId,
) -> crate::Handle {
    // A wrong payload degrades to the placeholder (props_of reports it) — never a panic.
    match day_spec::props_of::<PickerProps>(day_spec::kinds::PICKER, "gtk", props) {
        Some(p) => make(b, p, id),
        None => crate::placeholder_label(day_spec::kinds::PICKER),
    }
}

pub(crate) fn update_any(b: &mut crate::Gtk, h: &crate::Handle, patch: &dyn std::any::Any) {
    if let Some(p) = patch.downcast_ref::<PickerPatch>() {
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
