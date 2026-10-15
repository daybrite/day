use crate::res;
use day::prelude::*;

/// The opening screen: vector art, a greeting, and markdown prose that lives in the
/// translation (https://daybrite.dev/docs/resources).
pub(crate) fn welcome_page() -> impl Piece {
    column((
        spacer(),
        // Hide decorative art from screen readers; name meaningful images instead.
        // Accessibility semantics: https://daybrite.dev/docs/accessibility/
        vector(res::vectors::app_mark)
            .decorative()
            .frame(132.0, 132.0)
            .corner_radius(30.0)
            .id("welcome-mark"),
        label(res::str::welcome_title())
            .font(Font::LargeTitle)
            .a11y(|a| a.role(Role::Heading(1)))
            .align(TextAlign::Center)
            .id("welcome-title"),
        label(res::str::welcome_body())
            .markdown()
            .on_link(|target| {
                // Settings opens its window on desktop and selects the section on a phone.
                if target == "#settings" {
                    day_piece_settings::open();
                } else {
                    open_link(target);
                }
            })
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
