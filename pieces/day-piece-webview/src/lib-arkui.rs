// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// HarmonyOS: the ArkTS `Web` component. Unlike every other backend here, there is no native widget
// to construct — the ArkUI C node API has no Web node kind — so this crate ships its OWN ArkTS
// (ohos/ets/Index.ets) that `day build` stages into the app's hvigor project via
// `[package.metadata.day.ohos]`, the HarmonyOS counterpart of the android `java` contribution.
// day-arkui's generic piece bridge builds it and returns its FrameNode as an ordinary handle
// (docs/extending.md); commands cross as this piece's own (cmd, arg) strings, and each committed
// navigation comes back through the shim's `pieceEvent` as the Custom event kind (12) — §8.2's
// piece-defined channel, the same one the Android renderer uses.
// ---------------------------------------------------------------------------

use super::*;
use day_arkui::{AHandle, ArkUi, piece};
use day_spec::NodeId;

fn make(_backend: &mut ArkUi, p: &WebProps, id: NodeId) -> AHandle {
    // `props` is the initial URL — this piece's whole realize-time state.
    piece::make(KIND, id, &p.url)
}

fn update(_backend: &mut ArkUi, h: &AHandle, patch: &WebPatch) {
    let (cmd, arg) = match patch {
        WebPatch::Load(u) => ("load", u.as_str()),
        WebPatch::Back => ("back", ""),
        WebPatch::Forward => ("forward", ""),
        WebPatch::Stop => ("stop", ""),
        WebPatch::Reload => ("reload", ""),
        // Not implemented on this backend yet (docs/webview-eval.md). `eval_support()`
        // reports Unsupported, so the front-end resolves the future without dispatching
        // and this arm is unreachable — it exists to keep the match exhaustive.
        WebPatch::Eval { .. } => return,
    };
    piece::update(h, cmd, arg);
}

day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
    kind: KIND, props: WebProps, patch: WebPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
