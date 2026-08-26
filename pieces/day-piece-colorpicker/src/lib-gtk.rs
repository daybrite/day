// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// GTK: `GtkColorDialogButton` driving a `GtkColorDialog` (GTK 4.10+, which day-gtk already
// requires) — the swatch button and the GNOME color chooser behind it, editor pane and custom
// colors included. The older `GtkColorButton`/`GtkColorChooserDialog` pair is deprecated in the
// same release and is not used here.
//
// `GdkRGBA` is f32 where Day's `Color` is f64, so a value written into the button and read back
// out is not bit-identical. That is why the echo guard is a flag rather than a value comparison:
// comparing the f64 the signal holds against the f32 the button rounded it to would never match,
// and the button would be re-set on every flush.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use day_gtk::Gtk;
use day_spec::{NodeId, Proposal, Size};
use gtk4::prelude::*;

day_core::tls_group! {
    /// Per-button echo guard: `true` while day is writing the value in, so the resulting
    /// `rgba` notification is not reported back as a user pick.
    static SUPPRESS: RefCell<HashMap<usize, Rc<Cell<bool>>>> = RefCell::new(HashMap::new());

}

fn key(w: &gtk4::Widget) -> usize {
    w.as_ptr() as usize
}

fn to_day_color(c: gtk4::gdk::RGBA) -> Color {
    Color::rgba(
        c.red() as f64,
        c.green() as f64,
        c.blue() as f64,
        c.alpha() as f64,
    )
}

fn to_rgba(c: Color) -> gtk4::gdk::RGBA {
    gtk4::gdk::RGBA::new(c.r as f32, c.g as f32, c.b as f32, c.a as f32)
}

fn make(_backend: &mut Gtk, p: &ColorProps, id: NodeId) -> gtk4::Widget {
    let dialog = gtk4::ColorDialog::new();
    dialog.set_with_alpha(p.alpha);
    if !p.title.is_empty() {
        dialog.set_title(&p.title);
    }
    let button = gtk4::ColorDialogButton::new(Some(dialog));
    let suppress = Rc::new(Cell::new(false));
    suppress.set(true);
    button.set_rgba(&to_rgba(p.color));
    suppress.set(false);
    {
        let suppress = suppress.clone();
        button.connect_rgba_notify(move |b| {
            if suppress.get() {
                return;
            }
            day_gtk::emit(
                id,
                Event::custom(PICK_TAG, to_day_color(b.rgba()).to_string()),
            );
        });
    }
    let root = button.upcast::<gtk4::Widget>();
    SUPPRESS.with(|m| m.borrow_mut().insert(key(&root), suppress));
    root
}

fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &ColorPatch) {
    let ColorPatch::SetColor(c) = patch;
    let Some(button) = h.downcast_ref::<gtk4::ColorDialogButton>() else {
        return;
    };
    let Some(suppress) = SUPPRESS.with(|m| m.borrow().get(&key(h)).cloned()) else {
        return;
    };
    suppress.set(true);
    button.set_rgba(&to_rgba(*c));
    suppress.set(false);
}

fn measure(_backend: &mut Gtk, h: &gtk4::Widget, _p: Proposal) -> Size {
    let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
    let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
    Size::new((nat_w as f64).max(44.0), (nat_h as f64).max(24.0))
}

/// Drop the echo guard when the button goes away.
///
/// Without this the map grows by one entry per realized button, and — worse — its key is the
/// widget's ADDRESS, which the allocator reuses: a later widget landing on a freed address would
/// inherit the dead entry's flag, and a pick made while that stale flag was set would be dropped.
fn release(_backend: &mut Gtk, h: &gtk4::Widget) {
    SUPPRESS.with(|m| {
        m.borrow_mut().remove(&key(h));
    });
}

day_pieces::renderer!(day_gtk::RENDERERS, Gtk,
    kind: KIND, props: ColorProps, patch: ColorPatch,
    make: make, update: update, measure: measure, release: release);
