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
    // `props` is one string: the initial URL, or the inline-site marker the ArkTS side parses
    // (`day-inline<US><root><US><start>`, 0x1F-separated like the eval replies). The rawfile
    // URL itself is composed over there, next to the code that polices it.
    let props = if p.inline_root.is_empty() {
        p.url.clone()
    } else {
        format!(
            "day-inline{SEP}{}{SEP}{}",
            p.inline_root, p.inline_start,
            SEP = super::SEP
        )
    };
    piece::make(KIND, id, &props)
}

fn update(_backend: &mut ArkUi, h: &AHandle, patch: &WebPatch) {
    // Evaluation (docs/webview-eval.md): `req` rides in front of the script, 0x1F-separated —
    // the ArkTS side runs `runJavaScript`, normalizes the reply, and answers on `pieceEvent`
    // with `req` as the num. Its try/catch guarantees exactly one reply even when the
    // controller is not yet attached (error 17100001).
    if let WebPatch::Eval { req, script } = patch {
        piece::update(h, "eval", &format!("{req}{}{script}", super::SEP));
        return;
    }
    let (cmd, arg) = match patch {
        WebPatch::Load(u) => ("load", u.as_str()),
        WebPatch::Back => ("back", ""),
        WebPatch::Forward => ("forward", ""),
        WebPatch::Stop => ("stop", ""),
        WebPatch::Reload => ("reload", ""),
        WebPatch::Eval { .. } => return, // handled above; keeps the match exhaustive
    };
    piece::update(h, cmd, arg);
}

day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
    kind: KIND, props: WebProps, patch: WebPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
