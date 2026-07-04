// ---------------------------------------------------------------------------
// GTK: WebKitGTK 6.0 via the `webkit6` crate — a `WebView` widget (a `gtk4::Widget`). Written blind
// (WebKitGTK isn't installed on the reference host); it builds+runs where `webkitgtk-6.0` is present
// (the CI gtk jobs install it). The `uri` property notify reports navigation back via
// `Event::Custom("webview:url", …)`, matching the AppKit/Qt renderers.
// ---------------------------------------------------------------------------

use super::*;
use day_gtk::Gtk;
use day_spec::{NodeId, Proposal, Renderer, Size};
use gtk4::prelude::*;
use linkme::distributed_slice;
use webkit6::prelude::*;

fn make(_backend: &mut Gtk, props: &dyn std::any::Any, id: NodeId) -> gtk4::Widget {
    let p = props.downcast_ref::<WebProps>().unwrap();
    let wv = webkit6::WebView::new();
    // Report the current URL back on every navigation so a bound text field follows.
    wv.connect_uri_notify(move |wv| {
        if let Some(uri) = wv.uri() {
            day_gtk::emit(id, Event::Custom("webview:url", uri.to_string()));
        }
    });
    if !p.url.is_empty() {
        wv.load_uri(&p.url);
    }
    wv.upcast()
}

fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &dyn std::any::Any) {
    let Some(patch) = patch.downcast_ref::<WebPatch>() else {
        return;
    };
    let Some(wv) = h.downcast_ref::<webkit6::WebView>() else {
        return;
    };
    match patch {
        WebPatch::Load(url) => wv.load_uri(url),
        WebPatch::Back => {
            if wv.can_go_back() {
                wv.go_back();
            }
        }
        WebPatch::Forward => {
            if wv.can_go_forward() {
                wv.go_forward();
            }
        }
        WebPatch::Stop => wv.stop_loading(),
        WebPatch::Reload => wv.reload(),
    }
}

/// Fill the offered space (a web view has no intrinsic size).
fn measure(_backend: &mut Gtk, _h: &gtk4::Widget, p: Proposal) -> Size {
    Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0))
}

#[distributed_slice(day_gtk::RENDERERS)]
static WEBVIEW_GTK: fn() -> Renderer<Gtk> = || Renderer {
    kind: KIND,
    make,
    update,
    measure: Some(measure),
};
