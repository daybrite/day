//! The Preferences window (docs/windows.md): the app-wide appearance + language settings,
//! persisted under `showcase.*` prefs keys and applied live everywhere — every open
//! window recolors on a theme change and re-strings on a language change. Opened via the
//! auto Settings…/⌘, menu item (`day::register_preferences_with` in `root()`); on
//! backends without secondary windows the same content presents as a fullscreen cover.

use day::prelude::*;

use crate::res;

pub(crate) fn preferences_window() -> AnyPiece {
    scroll(
        column((
            label(res::str::prefs_window_title())
                .font(Font::Title2)
                .id("prefs-title"),
            label(res::str::prefs_window_caption())
                .font(Font::Footnote)
                .color(Color::rgba(0.55, 0.57, 0.62, 1.0)),
            form((day_piece_settings::settings_sections(
                "showcase.theme",
                "showcase.locale",
                res::locales::ALL,
            ),)),
        ))
        .spacing(8.0)
        .align(HAlign::Leading)
        .padding(16.0),
    )
    .any()
}
