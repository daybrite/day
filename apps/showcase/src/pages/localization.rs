use day::prelude::*;

use crate::widgets::page;

/// Localization playground (docs/localization.md): the locale is a live Signal, and one set of
/// Fluent translations renders ICU-correctly per locale — number formatting (grouping, digit
/// systems), date/time formatting (CLDR patterns via `DATETIME()`), CLDR plural categories, and
/// collation-aware sorting (zh = pinyin). Stable ids for the walkthrough (§14); the key-based
/// asserts resolve through the same bundles as the labels, so they are locale-correct by
/// construction.
pub(crate) fn localization_page() -> AnyPiece {
    page(
        crate::res::str::nav_localization(),
        "localization-title",
        Some(crate::res::str::fmt_caption()),
        form((
            locale_section(),
            numbers_section(),
            datetimes_section(),
            plurals_section(),
            sorting_section(),
        ))
        .any(),
    )
}

/// The live-locale demo: switching re-runs every `tr()`/`res::str` binding on the spot. Button
/// labels are the languages' own names (autonyms — deliberately not localized). Reset restores
/// the locale the run started in, so the walkthrough (and the rest of the app) continues in the
/// launch locale after the demo.
fn locale_section() -> impl Piece {
    let initial = day::locale().get_untracked();
    section((
        label(crate::res::str::loc_live_note()).font(Font::Footnote),
        row((
            button("English")
                .action(|| set_locale("en"))
                .id("locale-en"),
            button("Français")
                .action(|| set_locale("fr"))
                .id("locale-fr"),
            button("العربية")
                .action(|| set_locale("ar"))
                .id("locale-ar"),
            button("中文")
                .action(|| set_locale("zh-CN"))
                .id("locale-zh"),
        ))
        .spacing(8.0),
        labeled(
            crate::res::str::loc_current_label(),
            // The raw locale tag — a locale-independent value the walkthrough asserts literally.
            label(move || day::locale().get()).id("loc-current"),
        ),
        row((
            button(crate::res::str::loc_reset())
                .bordered()
                .action(move || set_locale(&initial))
                .id("locale-reset"),
            // `en-XA` accents + expands every string — the layout stress-test pseudolocale.
            button("Ⓔⓝ-ⓍⒶ")
                .action(|| set_locale("en-XA"))
                .id("locale-xa"),
        ))
        .spacing(8.0),
    ))
    .title(crate::res::str::loc_locale_section())
}

/// `NUMBER()` and the bundle-wide formatter: the SAME translation renders `1,234,567.891` in en,
/// `1.234.567,891` in de-style locales, narrow-NBSP groups in fr, Arabic-Indic digits where CLDR
/// says so.
fn numbers_section() -> impl Piece {
    section((
        labeled(
            crate::res::str::fmt_number_label(),
            label(crate::res::str::fmt_number(1234567.891)).id("fmt-number"),
        ),
        labeled(
            crate::res::str::fmt_fraction_label(),
            label(crate::res::str::fmt_fraction(1234.5)).id("fmt-fraction"),
        ),
        labeled(
            crate::res::str::fmt_percent_label(),
            label(crate::res::str::fmt_percent(0.72)).id("fmt-percent"),
        ),
    ))
    .title(crate::res::str::loc_numbers_section())
}

/// `DATETIME()` with civil ISO inputs: CLDR date/time patterns per locale (month names, order,
/// 12/24-hour convention) from one translation.
fn datetimes_section() -> impl Piece {
    section((
        labeled(
            crate::res::str::fmt_date_label(),
            label(crate::res::str::fmt_date("2026-07-18")).id("fmt-date"),
        ),
        labeled(
            crate::res::str::fmt_time_label(),
            label(crate::res::str::fmt_time("14:45")).id("fmt-time"),
        ),
        labeled(
            crate::res::str::fmt_datetime_label(),
            label(crate::res::str::fmt_datetime("2026-07-18T14:45")).id("fmt-datetime"),
        ),
    ))
    .title(crate::res::str::loc_dates_section())
}

/// CLDR plural categories, live: Arabic exercises zero/one/two/few/many; French counts 0 as
/// "one"; Chinese has no plural at all — the same key, the right grammar everywhere.
fn plurals_section() -> impl Piece {
    let count = Signal::new(0i64);
    section((row((
        button(crate::res::str::decrement())
            .bordered()
            .action(move || count.update(|c| *c = (*c - 1).max(0)))
            .id("plural-remove"),
        label(crate::res::str::plural_items(count)).id("plural-label"),
        button(crate::res::str::increment())
            .prominent()
            .action(move || count.update(|c| *c += 1))
            .id("plural-add"),
    ))
    .spacing(8.0),))
    .title(crate::res::str::loc_plurals_section())
}

/// Collation-aware sorting: the localized fruit names re-sort by the locale's rules (zh by
/// pinyin, not code points) whenever the locale switches.
fn sorting_section() -> impl Piece {
    section((labeled(
        crate::res::str::fmt_sorted_label(),
        label(move || {
            let mut fruits = vec![
                crate::res::str::fruit_banana().format(),
                crate::res::str::fruit_cherry().format(),
                crate::res::str::fruit_apple().format(),
                crate::res::str::fruit_elderberry().format(),
                crate::res::str::fruit_date().format(),
            ];
            sort_localized(&mut fruits);
            fruits.join(" · ")
        })
        .id("fmt-sorted"),
    ),))
    .title(crate::res::str::loc_sorting_section())
}
