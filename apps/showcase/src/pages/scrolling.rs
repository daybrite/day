use day::prelude::*;

use crate::widgets::heading;

/// Programmatic scrolling (docs/scroll.md): a long list inside a `scroll` driven by a
/// `Signal<Option<ScrollTarget>>` — buttons jump to the top, the bottom, or reveal one item by
/// its dayscript id. The buttons stay pinned outside the demo scroll so they remain tappable at
/// any offset; dayscript's `scroll_to:` step drives the same rail.
pub(crate) fn scrolling_page() -> AnyPiece {
    let jump: Signal<Option<ScrollTarget>> = Signal::new(None);
    let rows = PieceVec(
        (1..=150)
            .map(|i| {
                label(crate::res::str::scrolling_item(format!("{i}")))
                    .id(format!("scroll-item-{i}"))
                    .any()
            })
            .collect(),
    );
    column((
        heading(
            crate::res::str::nav_scrolling(),
            "scrolling-title",
            Some(crate::res::str::scrolling_caption()),
        ),
        row((
            button(crate::res::str::scroll_to_top())
                .bordered()
                .action(move || jump.set(Some(ScrollTarget::Top)))
                .id("scroll-top"),
            button(crate::res::str::scroll_to_bottom())
                .bordered()
                .action(move || jump.set(Some(ScrollTarget::Bottom)))
                .id("scroll-bottom"),
            button(crate::res::str::scroll_to_item())
                .bordered()
                .action(move || jump.set(Some(ScrollTarget::Id("scroll-item-100".into()))))
                .id("scroll-item"),
        ))
        .spacing(8.0),
        scroll(column(rows).spacing(8.0).align(HAlign::Leading))
            .scroll_target(jump)
            .id("scroll-demo"),
    ))
    .spacing(16.0)
    .align(HAlign::Leading)
    .padding(20.0)
    .any()
}
