use day::prelude::*;

day::routes! {
    /// The tab keys, typed (docs/tabs.md): the `.item` declarations are compile-checked,
    /// while deep links and dayscript still address the tabs as "one" / "two" / "three".
    enum Tab { One => "one", Two => "two", Three => "three" }
}

/// Native tabbed container (docs/tabs.md): a `selector` with `SelectorStyle::Tabs`, bound to a
/// `Signal<Tab>` of the active tab (tabs always have a selection, so no `Option`). NSTabView /
/// UITabBarController / GtkNotebook / QTabWidget / Android tab strip.
pub(crate) fn tabs_page() -> AnyPiece {
    fn pane(title: LocalizedText, body: LocalizedText, content_id: &'static str) -> AnyPiece {
        column((label(title).font(Font::Title), label(body).id(content_id)))
            .spacing(10.0)
            .align(HAlign::Leading)
            .padding(16.0)
            .any()
    }
    let tab = Signal::new(Tab::One);
    selector(tab)
        .style(SelectorStyle::Tabs)
        .item(Tab::One, tr("tab-one"), || {
            pane(tr("tab-one"), tr("tab-one-body"), "tab-one-content")
        })
        .item(Tab::Two, tr("tab-two"), || {
            pane(tr("tab-two"), tr("tab-two-body"), "tab-two-content")
        })
        .item(Tab::Three, tr("tab-three"), || {
            pane(tr("tab-three"), tr("tab-three-body"), "tab-three-content")
        })
        .id("demo-tabs")
}
