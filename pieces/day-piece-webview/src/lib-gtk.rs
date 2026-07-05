// ---------------------------------------------------------------------------
// GTK: WebKitGTK 6.0 via the `webkit6` crate — a `WebView` widget (a `gtk4::Widget`). Written blind
// (WebKitGTK isn't installed on the reference host); it builds+runs where `webkitgtk-6.0` is present
// (the CI gtk jobs install it). The `uri` property notify reports navigation back via
// `Event::custom("webview:url", …)`, matching the AppKit/Qt renderers.
// ---------------------------------------------------------------------------

use super::*;
use day_gtk::Gtk;
use day_spec::NodeId;
use gtk4::prelude::*;
use webkit6::prelude::*;

fn make(_backend: &mut Gtk, p: &WebProps, id: NodeId) -> gtk4::Widget {
    let wv = webkit6::WebView::new();
    // Report the current URL back on every navigation so a bound text field follows.
    wv.connect_uri_notify(move |wv| {
        if let Some(uri) = wv.uri() {
            day_gtk::emit(id, Event::custom("webview:url", uri.to_string()));
        }
    });
    if !p.url.is_empty() {
        wv.load_uri(&p.url);
    }
    wv.upcast()
}

fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &WebPatch) {
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

day_pieces::renderer!(day_gtk::RENDERERS, Gtk,
    kind: KIND, props: WebProps, patch: WebPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
