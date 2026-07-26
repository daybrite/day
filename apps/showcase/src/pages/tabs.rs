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
    selector(tab)
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
                    details_extra(),
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
