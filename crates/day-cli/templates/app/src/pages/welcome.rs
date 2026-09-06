use day::prelude::*;

/// The opening screen: vector art, a greeting, and markdown prose that lives in the
/// translation (https://daybrite.dev/docs/resources).
pub(crate) fn welcome_page() -> impl Piece {
    column((
        spacer(),
        vector(crate::res::vectors::app_mark)
            .frame(132.0, 132.0)
            .corner_radius(30.0)
            .id("welcome-mark"),
        label(crate::res::str::welcome_title())
            .font(Font::LargeTitle)
            .align(TextAlign::Center)
            .id("welcome-title"),
        label(crate::res::str::welcome_body())
            .markdown()
            .align(TextAlign::Center)
            .max_width(440.0)
            .id("welcome-body"),
        spacer(),
    ))
    .spacing(20.0)
    .align(HAlign::Center)
    // Fill the pane first, so centering has room to work.
    .grow()
    .padding(24.0)
}
