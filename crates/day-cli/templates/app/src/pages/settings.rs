use crate::res;
use day::prelude::*;

/// Appearance and language.
///
/// Both rows come from `day-piece-settings`, which is the same pair every Day app needs: they
/// persist through `day::prefs`, apply live through day-core's appearance and locale seams, and
/// take their LABELS from Day's own catalog — so they are already translated into every language
/// the framework ships and cost this app no keys of its own
/// (https://daybrite.dev/docs/localization). Picking a language here retitles the navigation rows
/// in place, with nothing to restart.
pub(crate) fn settings_body() -> impl Piece {
    form((day_piece_settings::settings_sections(
        crate::THEME_KEY,
        crate::LOCALE_KEY,
        res::locales::ALL,
    ),))
}

/// The same body as a navigable SECTION — what a phone shows, where there is no separate
/// preferences window to open. On a desktop this page is never reached: `root()` drops the
/// Settings row and `register_preferences` puts it in the App menu instead.
pub(crate) fn settings_page() -> impl Piece {
    column((
        label(res::str::nav_settings())
            .font(Font::Title)
            .id("settings-title"),
        settings_body(),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
}
