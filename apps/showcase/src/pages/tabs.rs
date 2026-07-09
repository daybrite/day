use day::prelude::*;

/// Native tabbed container (docs/tabs.md): a `selector` with `SelectorStyle::Tabs`, bound to a
/// `Signal<String>` of the active tab key. NSTabView / UITabBarController / GtkNotebook /
/// QTabWidget / Android tab strip. Keys are routes, so deep links and dayscript select tabs.
pub(crate) fn tabs_page() -> AnyPiece {
    fn pane(title: LocalizedText, body: LocalizedText, content_id: &'static str) -> AnyPiece {
        column((label(title).font(Font::Title), label(body).id(content_id)))
            .spacing(10.0)
            .align(HAlign::Leading)
            .padding(16.0)
            .any()
    }
    let tab = Signal::new("one".to_string());
    selector(tab)
        .style(SelectorStyle::Tabs)
        .item("one", tr("tab-one"), || {
            pane(tr("tab-one"), tr("tab-one-body"), "tab-one-content")
        })
        .item("two", tr("tab-two"), || {
            pane(tr("tab-two"), tr("tab-two-body"), "tab-two-content")
        })
        .item("three", tr("tab-three"), || {
            pane(tr("tab-three"), tr("tab-three-body"), "tab-three-content")
        })
        .id("demo-tabs")
}
