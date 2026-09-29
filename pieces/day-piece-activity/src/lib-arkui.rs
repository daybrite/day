// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// HarmonyOS: ARKUI_NODE_LOADING_PROGRESS, the platform's indeterminate spinner, straight from the
// C node API. `large` doubles the default 32 vp to the 64 vp HarmonyOS uses for a page-level
// loader; a stopped spinner is hidden but keeps its box, as on Android and iOS.
// ---------------------------------------------------------------------------

use super::*;
use day_arkui::{AHandle, ArkUi};
use day_arkui_sys as ffi;
use day_spec::{NodeId, Proposal, Size};

/// `day_ark_node_new`'s LOADING_PROGRESS kind.
const K_LOADING: std::os::raw::c_int = 8;

std::thread_local! {
    /// The spinners built large, by handle, for `measure`.
    static LARGE: std::cell::RefCell<std::collections::HashSet<usize>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

fn side(h: &AHandle) -> f64 {
    if LARGE.with(|l| l.borrow().contains(&(h.0 as usize))) {
        64.0
    } else {
        32.0
    }
}

fn make(_backend: &mut ArkUi, p: &ActivityProps, _id: NodeId) -> AHandle {
    let n = unsafe { ffi::day_ark_node_new(K_LOADING) };
    if p.large {
        LARGE.with(|l| l.borrow_mut().insert(n as usize));
    }
    unsafe { ffi::day_ark_set_loading(n, std::os::raw::c_int::from(p.animating)) };
    AHandle(n)
}

fn update(_backend: &mut ArkUi, h: &AHandle, patch: &ActivityPatch) {
    match patch {
        ActivityPatch::Animating(on) => unsafe {
            ffi::day_ark_set_loading(h.0, std::os::raw::c_int::from(*on))
        },
    }
}

fn measure(_backend: &mut ArkUi, h: &AHandle, _p: Proposal) -> Size {
    let s = side(h);
    Size::new(s, s)
}

fn release(_backend: &mut ArkUi, h: &AHandle) {
    LARGE.with(|l| l.borrow_mut().remove(&(h.0 as usize)));
}

day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
    kind: KIND, props: ActivityProps, patch: ActivityPatch,
    make: make, update: update, measure: measure, release: release);
