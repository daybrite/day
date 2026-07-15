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
    // `item_icon` attaches a bundled template image per tab (docs/tabs.md). Backends whose tab
    // widget shows icons (iOS UITabBar, the Android tab strip) render them; text-only tab widgets
    // (the desktop NSTabView/GtkNotebook/QTabWidget) ignore the icon and just show the label.
    selector(tab)
        .style(SelectorStyle::Tabs)
        .item_icon(Tab::One, crate::res::str::tab_one(), crate::res::images::tab_one, || {
            pane(crate::res::str::tab_one(), crate::res::str::tab_one_body(), "tab-one-content")
        })
        .item_icon(Tab::Two, crate::res::str::tab_two(), crate::res::images::tab_two, || {
            pane(crate::res::str::tab_two(), crate::res::str::tab_two_body(), "tab-two-content")
        })
        .item_icon(Tab::Three, crate::res::str::tab_three(), crate::res::images::tab_three, || {
            pane(crate::res::str::tab_three(), crate::res::str::tab_three_body(), "tab-three-content")
        })
        .id("demo-tabs")
}
