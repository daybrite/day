// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// GTK: gtk4::Spinner, the native GTK activity indicator. `start()`/`stop()` run/stop the spin.
// GtkSpinner scales to its allocation and its natural size is small, so a `set_size_request` gives
// it a stable, visible square (larger for `.large`). A stopped spinner stays on screen (drawn
// static), matching the other backends.
// ---------------------------------------------------------------------------

use crate::Gtk;
use day_spec::NodeId;
use day_spec::props::{ProgressPatch as ActivityPatch, ProgressProps as ActivityProps};
use gtk4::prelude::*;

std::thread_local! {
    // Entries are reclaimed by the toolkit's release sweep.
    static SIDES: day_spec::sidetable::SideTable<f64> = day_spec::sidetable::SideTable::new();
}

pub(crate) fn side(h: &gtk4::Widget) -> Option<f64> {
    SIDES.with(|sizes| sizes.get(h.as_ptr() as usize))
}

fn set_animating(spinner: &gtk4::Spinner, on: bool) {
    if on {
        spinner.start();
    } else {
        spinner.stop();
    }
}

pub(crate) fn make(_backend: &mut Gtk, p: &ActivityProps, _id: NodeId) -> gtk4::Widget {
    let spinner = gtk4::Spinner::new();
    let side = if p.large { 48 } else { 24 };
    spinner.set_size_request(side, side);
    SIDES.with(|sizes| sizes.insert(spinner.as_ptr() as usize, f64::from(side)));
    set_animating(&spinner, p.animating);
    spinner.upcast()
}

pub(crate) fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &ActivityPatch) {
    let Some(spinner) = h.downcast_ref::<gtk4::Spinner>() else {
        return;
    };
    match patch {
        ActivityPatch::Value(_) => {}
        ActivityPatch::Animating(on) => set_animating(spinner, *on),
    }
}
