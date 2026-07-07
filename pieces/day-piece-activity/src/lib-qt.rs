// ---------------------------------------------------------------------------
// Qt: Qt has no native spinner widget, so this crate's OWN shim (src/lib-qt-shim.cpp) wraps a
// QProgressBar in **busy mode** (range 0..0), the idiomatic Qt indeterminate indicator — the same
// technique day-qt uses for `spinner()`. build.rs compiles the shim against Qt6Widgets (already
// linked by day-qt-sys, so no extra link flags). Animating toggles between busy (range 0..0) and a
// frozen static bar (range 0..1, value 0); `.large` gives it a bigger minimum size.
// ---------------------------------------------------------------------------

use super::*;
use std::os::raw::{c_int, c_void};

use day_qt::{Qt, QtHandle};
use day_spec::NodeId;

unsafe extern "C" {
    fn day_activity_qt_new(large: c_int) -> *mut c_void;
    fn day_activity_qt_set_animating(w: *mut c_void, on: c_int);
}

fn make(_backend: &mut Qt, p: &ActivityProps, _id: NodeId) -> QtHandle {
    let w = QtHandle(unsafe { day_activity_qt_new(p.large as c_int) });
    unsafe { day_activity_qt_set_animating(w.0, p.animating as c_int) };
    w
}

fn update(_backend: &mut Qt, h: &QtHandle, patch: &ActivityPatch) {
    match patch {
        ActivityPatch::Animating(on) => unsafe { day_activity_qt_set_animating(h.0, *on as c_int) },
    }
}

day_pieces::renderer!(day_qt::RENDERERS, Qt,
    kind: KIND, props: ActivityProps, patch: ActivityPatch,
    make: make, update: update);
