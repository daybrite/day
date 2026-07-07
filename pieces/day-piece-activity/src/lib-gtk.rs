// ---------------------------------------------------------------------------
// GTK: gtk4::Spinner — the native GTK activity indicator. `start()`/`stop()` run/stop the spin.
// GtkSpinner scales to its allocation and its natural size is small, so a `set_size_request` gives
// it a stable, visible square (larger for `.large`). A stopped spinner stays on screen (drawn
// static), matching the other backends.
// ---------------------------------------------------------------------------

use super::*;
use day_gtk::Gtk;
use day_spec::NodeId;
use gtk4::prelude::*;

fn set_animating(spinner: &gtk4::Spinner, on: bool) {
    if on {
        spinner.start();
    } else {
        spinner.stop();
    }
}

fn make(_backend: &mut Gtk, p: &ActivityProps, _id: NodeId) -> gtk4::Widget {
    let spinner = gtk4::Spinner::new();
    let side = if p.large { 48 } else { 24 };
    spinner.set_size_request(side, side);
    set_animating(&spinner, p.animating);
    spinner.upcast()
}

fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &ActivityPatch) {
    let Some(spinner) = h.downcast_ref::<gtk4::Spinner>() else {
        return;
    };
    match patch {
        ActivityPatch::Animating(on) => set_animating(spinner, *on),
    }
}

day_pieces::renderer!(day_gtk::RENDERERS, Gtk,
    kind: KIND, props: ActivityProps, patch: ActivityPatch,
    make: make, update: update);
