use day::prelude::*;

use crate::widgets::page;

/// About — the opening page (and the desktop split's default detail): an identity hero
/// (logo + name + blurb + site link) over one card of live facts about this build and the
/// platform it landed on — version, bundle id, the native toolkit compiled into the binary,
/// the OS and device it is running on, the active locale, and the most recent app-lifecycle
/// phase (docs/lifecycle.md).
pub(crate) fn about_page() -> AnyPiece {
    let hero = column((
        image(crate::res::images::day_logo).frame(96.0, 96.0),
        label(crate::res::str::app_title()).font(Font::Title2),
        label(crate::res::str::about_text())
            .font(Font::Footnote)
            .id("about-text"),
        // The URL is the label (a value, not prose) — it stays raw in every locale.
        link("daybrite.dev", "https://daybrite.dev")
            .font(Font::Footnote)
            .id("about-link"),
    ))
    .spacing(8.0)
    .align(HAlign::Center)
    // Centered within the page column (which is leading-aligned): grow to the full width so
    // the hero's own centering is visible.
    .grow_w();

    // The platform identity, read once at build (day-part-deviceinfo; values vary per host).
    let d = day_part_deviceinfo::get();
    let info = section((
        labeled(
            crate::res::str::about_version(),
            label(env!("CARGO_PKG_VERSION"))
                .selectable()
                .id("about-version"),
        ),
        labeled(
            crate::res::str::about_id(),
            // Baked from Day.toml's [app].id by build.rs (DAY_APP_ID, set by `day build`);
            // a bare `cargo` build has no identity to show.
            label(option_env!("DAY_SHOWCASE_APP_ID").unwrap_or("\u{2014}"))
                .selectable()
                .id("about-id"),
        ),
        labeled(
            crate::res::str::about_toolkit(),
            label(day::toolkit_name()).selectable().id("about-toolkit"),
        ),
        labeled(
            crate::res::str::about_os(),
            label(format!("{} {}", d.system_name, d.system_version))
                .selectable()
                .id("about-os"),
        ),
        labeled(
            crate::res::str::about_model(),
            label(d.model).selectable().id("about-model"),
        ),
        labeled(
            crate::res::str::about_locale(),
            // Live: switching on the Localization page re-renders this tag on the spot.
            label(move || day::locale().get())
                .selectable()
                .id("about-locale"),
        ),
        labeled(
            crate::res::str::menus_lifecycle(),
            label(move || crate::lifecycle_log().get()).id("about-lifecycle"),
        ),
    ))
    .title(crate::res::str::about_app_section());

    page(
        crate::res::str::nav_about(),
        "about-title",
        None,
        column((hero, form((info,))))
            .spacing(16.0)
            .align(HAlign::Leading)
            .any(),
    )
}
