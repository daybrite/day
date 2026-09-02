// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// HarmonyOS: the ArkTS `Video` component for pictures, an `AVPlayer` (@kit.MediaKit) for sound
// only. Like the webview, there is no native node kind to construct — the ArkUI C node API has no
// video node — so this crate ships its OWN ArkTS (ohos/ets/Index.ets) that `day build` stages
// into the app's hvigor project via `[package.metadata.day.ohos]`. day-arkui's generic piece
// bridge builds it and returns its FrameNode as an ordinary handle (docs/extending.md); commands
// cross as this piece's own (cmd, arg) strings, and playback state comes back through the shim's
// `pieceEvent` as the Custom event kind — `num` = the piece's report code.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::RefCell;
use std::collections::HashSet;

use day_arkui::{AHandle, ArkUi, piece};
use day_spec::{NodeId, Proposal, Size};

day_core::tls_group! {
    // The sound-only nodes, which measure zero (keyed by the handle's pointer).
    static AUDIO_ONLY: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

/// The 0x1F separator the props string uses, the same one the webview's ArkTS side parses.
const SEP: char = '\u{1F}';

fn make(_backend: &mut ArkUi, p: &MediaProps, id: NodeId) -> AHandle {
    // `props` is one string, 0x1F-separated: url, autoplay, looping, muted, controls,
    // audio_only, volume — parsed in the same order on the ArkTS side.
    let props = format!(
        "{}{SEP}{}{SEP}{}{SEP}{}{SEP}{}{SEP}{}{SEP}{:.3}",
        p.url,
        p.autoplay as u8,
        p.looping as u8,
        p.muted as u8,
        p.controls as u8,
        p.audio_only as u8,
        p.volume.clamp(0.0, 1.0)
    );
    let h = piece::make(KIND, id, &props);
    if p.audio_only {
        AUDIO_ONLY.with(|s| s.borrow_mut().insert(h.0 as usize));
    }
    h
}

fn update(_backend: &mut ArkUi, h: &AHandle, patch: &MediaPatch) {
    let volume;
    let (cmd, arg) = match patch {
        MediaPatch::Load(u) => ("load", u.as_str()),
        MediaPatch::Play => ("play", ""),
        MediaPatch::Pause => ("pause", ""),
        MediaPatch::Stop => ("stop", ""),
        MediaPatch::Volume(v) => {
            volume = format!("{:.3}", v.clamp(0.0, 1.0));
            ("volume", volume.as_str())
        }
    };
    piece::update(h, cmd, arg);
}

/// A sound-only player takes no room; a video fills what it is offered.
fn measure(backend: &mut ArkUi, h: &AHandle, proposal: Proposal) -> Size {
    if AUDIO_ONLY.with(|s| s.borrow().contains(&(h.0 as usize))) {
        return Size::ZERO;
    }
    day_pieces::fill_measure(backend, h, proposal)
}

fn release(_backend: &mut ArkUi, h: &AHandle) {
    AUDIO_ONLY.with(|s| s.borrow_mut().remove(&(h.0 as usize)));
}

day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
    kind: KIND, props: MediaProps, patch: MediaPatch,
    make: make, update: update, measure: measure, release: release);
