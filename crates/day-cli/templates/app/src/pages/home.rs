use day::prelude::*;

/// Welcome + the classic reactive counter: one `Signal`, two buttons, a live label
/// (https://daybrite.dev/docs/state). No invalidation bookkeeping — the closure label
/// re-renders because it reads `count`.
pub(crate) fn home_page() -> AnyPiece {
    let count = Signal::new(0i64);
    column((
        label(tr("home-welcome")).font(Font::Title).id("home-title"),
        label(tr("home-blurb")),
        row((
            button("−")
                .action(move || count.update(|c| *c -= 1))
                .id("dec"),
            label(move || format!("{}", count.get()))
                .font(Font::Headline)
                .id("count"),
            button("+")
                .action(move || count.update(|c| *c += 1))
                .id("inc"),
        ))
        .spacing(12.0),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
