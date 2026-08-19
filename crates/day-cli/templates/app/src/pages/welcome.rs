use day::prelude::*;

/// The opening screen: a mark, a greeting, and a short piece of prose.
///
/// Three things worth copying from it. The art is a VECTOR
/// (https://daybrite.dev/docs/vectors) — `resource/icons/icon.svg`, the same file `day icon`
/// renders every platform's launcher icon from — so it is crisp at any size on any DPI instead of
/// a bitmap picked from a ladder. The prose is one `label(...).markdown()`
/// (https://daybrite.dev/docs/markdown): the emphasis and the LINK live in the translation rather
/// than in the layout, so a language that stresses a different word simply says so in its own
/// `.ftl` and no Rust changes. A tapped link with no `.on_link()` handler opens in the platform's
/// browser, which is what a link in a paragraph is normally expected to do.
///
/// The spacers above and below are what center the column vertically; `.align(TextAlign::Center)`
/// is what centers the wrapped prose within its own width, which the column alone cannot do —
/// a container can center a label's BOX without centering the lines inside it.
pub(crate) fn welcome_page() -> AnyPiece {
    column((
        spacer(),
        // `corner_radius` clips, which is what softens the mark's square corners — the same
        // rounding every platform gives an app icon.
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
    // `.grow()` is what makes the centering VISIBLE: a column sized to its widest child is
    // already "centered" within itself, and sits wherever the pane puts it — at the leading
    // edge. Filling the pane first is what gives `HAlign::Center` room to mean anything.
    .grow()
    .padding(24.0)
    .any()
}
