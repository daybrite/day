use day::prelude::*;
use day_piece_rating::Card;

day::routes! {
    /// The tab keys, typed (docs/tabs.md): the `.item` declarations are compile-checked,
    /// while deep links and dayscript still address the tabs as "one" / "two" / "three".
    enum Tab { One => "one", Two => "two", Three => "three" }
}

/// Native tabbed container (docs/tabs.md): a `selector` with `SelectorStyle::Tabs`, bound to a
/// `Signal<Tab>` of the active tab (tabs always have a selection, so no `Option`). NSTabView /
/// UITabBarController / GtkNotebook / QTabWidget / Android tab strip. Each pane holds live
/// controls whose signals are owned by the PAGE, not the pane — switch away and back and the
/// state is still there, which is the point the panes make.
pub(crate) fn tabs_page() -> AnyPiece {
    fn pane(
        title: LocalizedText,
        body: LocalizedText,
        content_id: &'static str,
        extra: AnyPiece,
    ) -> AnyPiece {
        column((
            label(title).font(Font::Title),
            label(body).id(content_id),
            extra,
        ))
        .spacing(12.0)
        .align(HAlign::Leading)
        .padding(20.0)
        .any()
    }
    let tab = Signal::new(Tab::One);
    // Page-scope state, one signal set per pane: the panes read/write these across tab switches.
    let clicks = Signal::new(0i64);
    let badges = Signal::new(true);
    let sounds = Signal::new(false);
    // `item_icon` attaches a bundled template image per tab (docs/tabs.md). Backends whose tab
    // widget shows icons (iOS UITabBar, the Android tab strip) render them; text-only tab widgets
    // (the desktop NSTabView/GtkNotebook/QTabWidget) ignore the icon and just show the label.
    let main = selector(tab)
        .style(SelectorStyle::Tabs)
        .item_icon(
            Tab::One,
            crate::res::str::tab_one(),
            crate::res::images::tab_one,
            move || {
                pane(
                    crate::res::str::tab_one(),
                    crate::res::str::tab_one_body(),
                    "tab-one-content",
                    overview_extra(clicks),
                )
            },
        )
        .item_icon(
            Tab::Two,
            crate::res::str::tab_two(),
            crate::res::images::tab_two,
            || {
                pane(
                    crate::res::str::tab_two(),
                    crate::res::str::tab_two_body(),
                    "tab-two-content",
                    // The data-driven tab demo lives INSIDE this pane — two nested tab views:
                    // the outer typed tabs, and a dynamic string-keyed set within Details.
                    column((details_extra(), dynamic_tabs_demo()))
                        .spacing(12.0)
                        .align(HAlign::Leading)
                        .any(),
                )
            },
        )
        .item_icon(
            Tab::Three,
            crate::res::str::tab_three(),
            crate::res::images::tab_three,
            move || {
                pane(
                    crate::res::str::tab_three(),
                    crate::res::str::tab_three_body(),
                    "tab-three-content",
                    settings_extra(badges, sounds),
                )
            },
        )
        .id("demo-tabs")
        .any();
    column((main,)).grow().any()
}

/// Data-driven tabs (docs/navigation.md): the tab set comes from a signal, so the Add/Remove
/// buttons grow and shrink the native tab strip live. String keys, with `.destination` building
/// each dynamic tab's page.
fn dynamic_tabs_demo() -> AnyPiece {
    let tabs = Signal::new(vec!["alpha".to_string(), "beta".to_string()]);
    let current = Signal::new("alpha".to_string());
    let (add, remove) = (tabs, tabs);
    column((
        label(crate::res::str::tab_dynamic_title())
            .font(Font::Headline)
            .id("dyn-tabs-title"),
        row((
            button(crate::res::str::tab_dynamic_add())
                .action(move || {
                    add.update(|v| {
                        let n = v.len() + 1;
                        v.push(format!("tab-{n}"));
                    })
                })
                .id("dyn-tab-add"),
            button(crate::res::str::tab_dynamic_remove())
                .bordered()
                .action(move || {
                    remove.update(|v| {
                        v.pop();
                    })
                })
                .id("dyn-tab-remove"),
        ))
        .spacing(8.0),
        selector(current)
            .style(SelectorStyle::Tabs)
            .local()
            .items(move || tabs.get(), |k: &String| item(k.clone(), k.clone()))
            .destination(|k: &String| {
                label(format!("Tab: {k}"))
                    .id("dyn-tab-content")
                    .padding(16.0)
                    .any()
            })
            .id("dyn-tabs"),
    ))
    .spacing(8.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .frame(560.0, 220.0)
    .any()
}

/// Overview: a counter whose signal outlives the pane — count a few clicks, switch tabs, and
/// come back to the same number.
fn overview_extra(clicks: Signal<i64>) -> AnyPiece {
    column((
        row((
            button(crate::res::str::decrement())
                .bordered()
                .action(move || clicks.update(|c| *c -= 1))
                .id("tab-one-dec"),
            label(crate::res::str::counter_value(clicks)).id("tab-one-count"),
            button(crate::res::str::increment())
                .prominent()
                .action(move || clicks.update(|c| *c += 1))
                .id("tab-one-inc"),
        ))
        .spacing(8.0),
        label(crate::res::str::tab_state_note()).font(Font::Footnote),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .any()
}

/// Details: the addressing facts for this tab, as a quiet card. The values are route keys —
/// data, not prose — so they stay raw.
fn details_extra() -> AnyPiece {
    column((
        labeled(
            crate::res::str::tab_detail_route(),
            label("two").font(Font::Callout),
        ),
        labeled(
            crate::res::str::tab_detail_link(),
            label("tabs/two").font(Font::Callout),
        ),
    ))
    .spacing(8.0)
    .align(HAlign::Leading)
    .modifier(Card)
    .any()
}

/// Settings: two toggles bound to page-scope signals — flip one, tour the other tabs, return.
fn settings_extra(badges: Signal<bool>, sounds: Signal<bool>) -> AnyPiece {
    column((
        labeled(
            crate::res::str::tab_set_badges(),
            toggle(badges).id("tab-set-badges"),
        ),
        labeled(
            crate::res::str::tab_set_sounds(),
            toggle(sounds).id("tab-set-sounds"),
        ),
        label(crate::res::str::tab_state_note()).font(Font::Footnote),
    ))
    .spacing(8.0)
    .align(HAlign::Leading)
    .any()
}
