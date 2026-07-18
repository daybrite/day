// ---------------------------------------------------------------------------
// ArkUI (HarmonyOS): the real thing — ARKUI_NODE_REFRESH, created by this crate's OWN NDK shim
// (src/refresh-arkui.cpp, compiled by build.rs against OHOS_NDK_HOME — the tickmarks pattern).
// The realized node IS the Refresh node: day-core's generic day_ark_insert_child mounts the
// wrapped scrollable into it (Refresh hosts exactly one child). Pull-begins route from
// NODE_REFRESH_ON_REFRESH through the shim's callback into `day_arkui::emit`; `RefreshPatch`
// drives NODE_REFRESH_REFRESHING both ways. This is the first external ArkUI piece renderer —
// registration is the same `renderer!` slice as every other backend.
// ---------------------------------------------------------------------------

use super::*;
use day_arkui::{AHandle, ArkUi};
use day_spec::NodeId;
use std::os::raw::{c_int, c_longlong, c_void};

unsafe extern "C" {
    fn day_prf_node_new(
        id: c_longlong,
        refreshing: c_int,
        cb: extern "C" fn(c_longlong),
    ) -> *mut c_void;
    fn day_prf_set_refreshing(node: *mut c_void, on: c_int);
}

/// Shim → Day: a user pull began on the Refresh node carrying this day NodeId.
extern "C" fn on_pull(id: c_longlong) {
    day_arkui::emit(NodeId(id as u64), Event::custom("pullrefresh:begin", ""));
}

fn make(_backend: &mut ArkUi, p: &RefreshProps, id: NodeId) -> AHandle {
    AHandle(unsafe { day_prf_node_new(id.0 as c_longlong, p.refreshing as c_int, on_pull) })
}

fn update(_backend: &mut ArkUi, h: &AHandle, patch: &RefreshPatch) {
    let RefreshPatch::SetRefreshing(on) = patch;
    unsafe { day_prf_set_refreshing(h.0, *on as c_int) };
}

day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
    kind: KIND, props: RefreshProps, patch: RefreshPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
