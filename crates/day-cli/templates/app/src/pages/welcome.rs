use day::prelude::*;

/// The opening screen: vector art, a greeting, and markdown prose whose emphasis and link live
/// in the translation (https://daybrite.dev/docs/vectors, https://daybrite.dev/docs/markdown).
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
    // Fill the pane first, so the centering has room to mean anything.
    .grow()
    .padding(24.0)
}
