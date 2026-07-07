// ---------------------------------------------------------------------------
// WinUI: this crate's OWN C++/WinRT shim (src/lib-winui-shim.cpp) wrapping a
// Windows.UI.Xaml.Controls.ProgressRing — the native UWP-XAML indeterminate spinner — boxed into a
// day handle via day-winui-sys's `day_winui_box`/`day_winui_unbox` seam (like the media/picker/
// webview winui pieces). `IsActive` maps to `.animating`; `.large` sets Width/Height. Windows-only;
// written blind and built in CI.
// ---------------------------------------------------------------------------

use super::*;
use std::os::raw::{c_int, c_void};

use day_spec::NodeId;
use day_winui::{WinHandle, WinUi};

unsafe extern "C" {
    fn day_activity_winui_new(large: c_int, animating: c_int) -> *mut c_void;
    fn day_activity_winui_set_animating(w: *mut c_void, on: c_int);
}

fn make(_backend: &mut WinUi, p: &ActivityProps, _id: NodeId) -> WinHandle {
    WinHandle(unsafe { day_activity_winui_new(p.large as c_int, p.animating as c_int) })
}

fn update(_backend: &mut WinUi, h: &WinHandle, patch: &ActivityPatch) {
    match patch {
        ActivityPatch::Animating(on) => unsafe {
            day_activity_winui_set_animating(h.0, *on as c_int)
        },
    }
}

day_pieces::renderer!(day_winui::RENDERERS, WinUi,
    kind: KIND, props: ActivityProps, patch: ActivityPatch,
    make: make, update: update);
