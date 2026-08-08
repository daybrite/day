// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Web (web-dom): a plain `<video>` element.
//
// The one backend where a media player is LESS work than the native ones: the browser supplies the
// transport chrome, buffering, scrubbing, fullscreen, captions and picture-in-picture, and every
// MediaProps field maps onto an attribute of the same meaning.
//
// Two web-only truths, both documented in docs/media.md:
//
// - **A file PATH will not load.** The other backends accept `/Users/me/clip.mp4`; a page can only
//   fetch a URL, and a cross-origin one needs CORS. Use a same-origin file under the app's `dist/`
//   or a permissive remote.
// - **Autoplay with sound is blocked** by every modern browser unless the user has interacted with
//   the page. `.muted(true)` is what makes autoplay actually start — otherwise the element loads
//   and waits, which looks like a broken player but is the browser's policy, not day's.
// ---------------------------------------------------------------------------

use super::*;
use day_dom::{Dom, DomHandle};
use day_spec::{NodeId, Proposal, Size};

/// Boolean DOM attributes use day-dom's marker convention: `"-"` sets, `""` removes.
fn flag(on: bool) -> &'static str {
    if on { "-" } else { "" }
}

fn make(backend: &mut Dom, p: &MediaProps, _id: NodeId) -> DomHandle {
    let h = backend.element("video");
    if !p.url.is_empty() {
        backend.set_attr(&h, "src", &p.url);
    }
    backend.set_attr(&h, "controls", flag(p.controls));
    backend.set_attr(&h, "autoplay", flag(p.autoplay));
    backend.set_attr(&h, "loop", flag(p.looping));
    backend.set_attr(&h, "muted", flag(p.muted));
    // `playsinline` keeps iOS Safari from hijacking the whole screen for playback — without it a
    // small inline player becomes a fullscreen takeover on iPhone.
    backend.set_attr(&h, "playsinline", "-");
    // Fill the frame day's layout assigns, the same growing-leaf contract as the native arms.
    backend.set_attr(&h, "style", "width:100%;height:100%;object-fit:contain");
    h
}

fn update(backend: &mut Dom, h: &DomHandle, patch: &MediaPatch) {
    match patch {
        MediaPatch::Load(url) => {
            backend.set_attr(h, "src", url);
            // `load()` re-reads the new src; `play()` then starts it, matching `.load()`'s
            // documented "reload and play" behavior on the native arms.
            backend.call(h, "load");
            backend.call(h, "play");
        }
        MediaPatch::Play => backend.call(h, "play"),
        MediaPatch::Pause => backend.call(h, "pause"),
    }
}

/// A growing leaf: take whatever the layout proposes, with a modest default when it proposes
/// nothing (the same posture as the other arms, which report the native view's intrinsic size).
fn measure(_backend: &mut Dom, _h: &DomHandle, p: Proposal) -> Size {
    Size::new(p.width.unwrap_or(320.0), p.height.unwrap_or(180.0))
}

// Defines `register()`, which `media()` calls — web-dom's registry is populated at runtime, unlike
// the link-time `renderer!` the other eight arms use.
day_pieces::dom_renderer!(day_dom::register_renderer, Dom,
    kind: KIND, props: MediaProps, patch: MediaPatch,
    make: make, update: update, measure: measure);
