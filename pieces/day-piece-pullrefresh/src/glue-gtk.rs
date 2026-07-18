// ---------------------------------------------------------------------------
// GTK emulated-gesture glue: GtkScrolledWindow has a purpose-built `edge-overshot` signal — emitted
// when user-initiated (kinetic/gesture) scrolling firmly surpasses a content edge. A Top overshoot
// IS the pull gesture. Applied as a `Decorate::tweak` on the wrapped scrollable; inert when the
// child's realized widget is not a GtkScrolledWindow.
//
// Safety: the handler must NOT run app logic synchronously inside GTK's scroll dispatch (a signal
// trampoline aborts the process if a panic unwinds through it, and mutating the widget tree
// mid-dispatch is reentrancy-hazardous). So the overshoot only posts an idle that routes a
// `pullrefresh:begin` Custom event through the backend's sink — queued, pumped at a safe point,
// and panic-contained, exactly like a built-in control's event. The piece's `cx.on` wire on the
// host node turns it into the begin. Repeats during one overshoot are harmless: the begin path is
// idempotent while a refresh is in flight.
// ---------------------------------------------------------------------------

use day_core::RNode;
use day_spec::{Event, NodeId};
use gtk4::prelude::*;

pub(crate) fn attach(node: RNode, host: NodeId) {
    let _ = day_gtk::with_native(node, |w, _class| {
        let Some(sw) = w.downcast_ref::<gtk4::ScrolledWindow>() else {
            return; // not a scroll-backed child — gesture inert (spinner overlay still works)
        };
        sw.connect_edge_overshot(move |_, pos| {
            if pos == gtk4::PositionType::Top {
                // Hop out of the scroll dispatch, then enter Day through the event sink.
                gtk4::glib::idle_add_local_once(move || {
                    day_gtk::emit(host, Event::custom("pullrefresh:begin", ""));
                });
            }
        });
    });
}
