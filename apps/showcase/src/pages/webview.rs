use day::prelude::*;
use day_piece_webview::web_view;

use crate::widgets::heading;

/// A native web view (day-piece-webview, an EXTERNAL standalone piece): WKWebView / QWebEngineView /
/// android.webkit.WebView. The URL bar is bound two-way to the view — type + Go loads it, and
/// navigation reports the URL back so the field follows. Back/Forward/Stop/Reload drive history via
/// `Trigger`s the piece watches. The view fills the remaining space (a growing leaf).
pub(crate) fn webview_page() -> AnyPiece {
    let url = Signal::new("https://daybrite.dev".to_string());
    let go = Trigger::new();
    let back = Trigger::new();
    let forward = Trigger::new();
    let stop = Trigger::new();
    let reload = Trigger::new();
    column((
        heading(crate::res::str::nav_webview(), "webview-title", None),
        // URL bar: the field is bound to the view's URL; Go loads whatever it holds.
        row((
            text_field(url)
                .placeholder(crate::res::str::webview_url_hint())
                .id("webview-url"),
            button(crate::res::str::webview_go())
                .prominent()
                .action(move || go.notify())
                .id("webview-go"),
        ))
        .spacing(8.0),
        // History controls. "Stop" is the demo's cancel.
        row((
            button(crate::res::str::webview_back())
                .bordered()
                .action(move || back.notify())
                .id("webview-back"),
            button(crate::res::str::webview_forward())
                .bordered()
                .action(move || forward.notify())
                .id("webview-forward"),
            button(crate::res::str::webview_stop())
                .bordered()
                .action(move || stop.notify())
                .id("webview-stop"),
            button(crate::res::str::webview_reload())
                .bordered()
                .action(move || reload.notify())
                .id("webview-reload"),
        ))
        .spacing(8.0),
        web_view(url)
            .go(go)
            .back(back)
            .forward(forward)
            .stop(stop)
            .reload(reload)
            .id("webview"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
