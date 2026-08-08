// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Web (web-dom): an `<iframe>`.
//
// The one backend with no browser engine to embed, because the host page already is one. An iframe
// displays a URL faithfully; everything beyond that is governed by the same-origin policy, which
// forbids a parent document from reading or driving a cross-origin child. Three consequences, all
// documented in docs/webview.md:
//
// - **Session history is unreachable.** `contentWindow.history.back()` throws `SecurityError` on a
//   cross-origin frame, so `Back` and `Forward` are no-ops. Driving the TOP-level history instead
//   would be worse than doing nothing: day's web router owns that stack (`pushState` on hash
//   routes), so a "back" press would navigate the app off the page hosting the frame.
// - **A load cannot be cancelled.** `contentWindow.stop()` is blocked for the same reason.
// - **Navigation does not report back.** Reading `contentWindow.location.href` throws, so the URL
//   binding is one-way here: `.go()` loads the field's value, but the field does not follow the
//   user's clicks inside the frame. `Reload` therefore re-loads the last URL day itself set, not
//   wherever the frame has since navigated to.
//
// Same-origin content has none of these limits, but a piece cannot know the origin before loading
// and the failure is silent when it guesses wrong, so this arm reports `Support::Emulated` and
// behaves identically either way rather than working only sometimes. An app gates its history
// controls on `day_piece_webview::support()`.
//
// A cross-origin frame that refuses embedding (`X-Frame-Options`, CSP `frame-ancestors`) renders
// blank, and the parent cannot detect it — the load event fires either way. That is the browser's
// policy, not day's, and no arm of this piece can report it.
// ---------------------------------------------------------------------------

use super::*;
use day_dom::{Dom, DomHandle};
use day_spec::{NodeId, Proposal, Size};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// The last URL day assigned per frame, so `Reload` has something to re-assign. day-dom is
    /// write-only (`set_attr` with no getter) and `WebPatch::Reload` carries no URL, so without
    /// this the command would have nothing to act on. Keyed by the shim element id; web-dom is
    /// single-threaded and handles are never reused, so a plain map is enough.
    static LAST_SRC: RefCell<HashMap<DomHandle, String>> = RefCell::new(HashMap::new());
}

/// Point the frame at `url` and remember it for `Reload`. Assigning `src` navigates, and assigning
/// the value it already holds re-navigates — which is what lets one helper serve both commands.
fn load(backend: &mut Dom, h: &DomHandle, url: &str) {
    backend.set_attr(h, "src", url);
    LAST_SRC.with(|m| m.borrow_mut().insert(*h, url.to_string()));
}

fn make(backend: &mut Dom, p: &WebProps, _id: NodeId) -> DomHandle {
    let h = backend.element("iframe");
    if !p.url.is_empty() {
        load(backend, &h, &p.url);
    }
    // No `sandbox` attribute: present-but-empty is deny-everything, which breaks scripts, forms and
    // same-origin reads on essentially every real site. Absent is the permissive default we want.
    // The inline style fills the frame day's layout assigns — the growing-leaf contract the native
    // arms honor by sizing their native view to the same rect.
    backend.set_attr(&h, "style", "width:100%;height:100%;border:0");
    h
}

fn update(backend: &mut Dom, h: &DomHandle, patch: &WebPatch) {
    match patch {
        WebPatch::Load(url) => load(backend, h, url),
        WebPatch::Reload => {
            // Re-assign whatever we last set. Nothing to do before the first load.
            if let Some(url) = LAST_SRC.with(|m| m.borrow().get(h).cloned()) {
                backend.set_attr(h, "src", &url);
            }
        }
        // Unreachable across origins (see the header). Faking them through the top-level history
        // would drive day's router rather than the frame, so they stay no-ops and the app disables
        // the controls via `support()`.
        WebPatch::Back | WebPatch::Forward | WebPatch::Stop => {}
        // Evaluation is blocked by the same policy: `contentWindow.eval` throws cross-origin.
        // `eval_support()` reports Unsupported, so this never arrives.
        WebPatch::Eval { .. } => {}
    }
}

/// A growing leaf: take whatever the layout proposes, with a modest default when it proposes
/// nothing — the same posture as the native arms, which report their web view's intrinsic size.
fn measure(_backend: &mut Dom, _h: &DomHandle, p: Proposal) -> Size {
    Size::new(p.width.unwrap_or(320.0), p.height.unwrap_or(240.0))
}

// Defines `register()`, which `web_view()` calls — web-dom's registry is populated at runtime,
// unlike the link-time `renderer!` the native arms use (wasm has no `linkme` slice).
day_pieces::dom_renderer!(day_dom::register_renderer, Dom,
    kind: KIND, props: WebProps, patch: WebPatch,
    make: make, update: update, measure: measure);
