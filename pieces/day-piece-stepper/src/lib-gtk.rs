// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// GTK: `GtkSpinButton` — the field-with-arrows widget GNOME ships for exactly this. One
// widget carries the whole contract: range, increments, display digits, keyboard entry.
//
// The echo guard is a flag rather than a value comparison for the colorpicker's reason:
// the widget quantizes to its digit count, so the f64 the binding holds and the f64 the
// widget reads back are not bit-identical, and comparing them would re-set the widget on
// every flush.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use day_gtk::Gtk;
use day_spec::{NodeId, Proposal, Size};
use gtk4::prelude::*;

thread_local! {
    /// Per-widget echo guard: `true` while day is writing the value in, so the resulting
    /// `value-changed` notification is not reported back as a user step.
    static SUPPRESS: RefCell<HashMap<usize, Rc<Cell<bool>>>> = RefCell::new(HashMap::new());
}

fn key(w: &gtk4::Widget) -> usize {
    w.as_ptr() as usize
}

fn make(_backend: &mut Gtk, p: &StepperProps, id: NodeId) -> gtk4::Widget {
    let spin = gtk4::SpinButton::with_range(p.min, p.max, p.step);
    spin.set_digits(p.decimals);
    let suppress = Rc::new(Cell::new(false));
    suppress.set(true);
    spin.set_value(p.value);
    suppress.set(false);
    {
        let suppress = suppress.clone();
        spin.connect_value_changed(move |s| {
            if suppress.get() {
                return;
            }
            day_gtk::emit(id, Event::custom(VALUE_TAG, s.value().to_string()));
        });
    }
    let root = spin.upcast::<gtk4::Widget>();
    SUPPRESS.with(|m| m.borrow_mut().insert(key(&root), suppress));
    root
}

fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &StepperPatch) {
    let StepperPatch::SetValue(v) = patch;
    let Some(spin) = h.downcast_ref::<gtk4::SpinButton>() else {
        return;
    };
    let Some(suppress) = SUPPRESS.with(|m| m.borrow().get(&key(h)).cloned()) else {
        return;
    };
    suppress.set(true);
    spin.set_value(*v);
    suppress.set(false);
}

fn measure(_backend: &mut Gtk, h: &gtk4::Widget, _p: Proposal) -> Size {
    let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
    let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
    Size::new((nat_w as f64).max(96.0), (nat_h as f64).max(24.0))
}

/// Drop the echo guard when the widget goes away — the map's key is the widget's ADDRESS,
/// which the allocator reuses; a stale entry would hand its flag to the next widget there.
fn release(_backend: &mut Gtk, h: &gtk4::Widget) {
    SUPPRESS.with(|m| {
        m.borrow_mut().remove(&key(h));
    });
}

day_pieces::renderer!(day_gtk::RENDERERS, Gtk,
    kind: KIND, props: StepperProps, patch: StepperPatch,
    make: make, update: update, measure: measure, release: release);
