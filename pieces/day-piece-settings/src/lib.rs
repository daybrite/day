// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-piece-settings — the shared appearance + language settings rows every app's
//! preferences surface needs (docs/windows.md), persisted through `day-part-prefs` and
//! applied live through day-core's appearance/locale seams. A COMPOSE piece: pure
//! composition, no native code, works on every backend.
//!
//! The rows match the pattern Day-Skies/Day-Tradr/Day-Matrix each hand-rolled — one place
//! now — including the fixed element ids (`theme-picker`, `language-picker`) their
//! walkthroughs assert. Labels come from the core catalog (`day-settings-*`,
//! `day-theme-*`), so the rows localize with zero app keys.
//!
//! ```ignore
//! // root(), after res::locales::install():
//! day_piece_settings::apply_startup("myapp.theme", "myapp.locale");
//! day::register_preferences(|| {
//!     form((day_piece_settings::settings_sections(
//!         "myapp.theme",
//!         "myapp.locale",
//!         res::locales::ALL,
//!         res::locales::DEFAULT,
//!     ),))
//! });
//! ```

use std::cell::OnceCell;

use day_pieces::prelude::*;
use day_reactive::{Signal, watch};

/// A `(tag, autonym)` locale table — the shape of a generated `res::locales::ALL`.
pub type LocaleTable = &'static [(&'static str, &'static str)];

thread_local! {
    /// The locale the app STARTED in (system or `--locale`), captured by [`apply_startup`]
    /// before any stored override applies — what the language picker's "System" entry
    /// restores.
    static SYSTEM_LOCALE: OnceCell<String> = const { OnceCell::new() };
}

fn system_locale() -> String {
    SYSTEM_LOCALE
        .with(|c| c.get().cloned())
        .unwrap_or_else(|| day_fluent::locale().get_untracked())
}

/// Apply the persisted appearance + language overrides at boot — call once from `root()`,
/// right after the locale catalog installs and before the first page builds.
///
/// The launch environment WINS over persistence: a `DAY_THEME` run keeps its forced theme
/// and a `DAY_LOCALE`/`--locale` run keeps its locale, so CI variant loops and `day launch`
/// overrides stay deterministic no matter what an earlier run persisted. Live picker
/// changes still apply after boot — user intent beats the environment once the app runs.
pub fn apply_startup(theme_key: &'static str, locale_key: &'static str) {
    SYSTEM_LOCALE.with(|c| {
        let _ = c.set(day_fluent::locale().get_untracked());
    });
    if std::env::var("DAY_LOCALE").is_err()
        && let Some(tag) = day_part_prefs::get(locale_key)
        && !tag.is_empty()
    {
        day_fluent::set_locale(&tag);
    }
    if std::env::var("DAY_THEME").is_err()
        && day_core::capability(day_spec::Cap::Appearance) != day_spec::Support::Unsupported
    {
        match day_part_prefs::get(theme_key).as_deref() {
            Some("light") => day_core::set_appearance(Some(false)),
            Some("dark") => day_core::set_appearance(Some(true)),
            _ => {}
        }
    }
}

/// The Light / Dark / System appearance row (id `theme-picker`): a labeled segmented
/// picker, present only where the backend honors a runtime override (`Cap::Appearance`) —
/// an empty piece otherwise. Selection applies live (`set_appearance`) and persists under
/// `prefs_key` (`"light"` / `"dark"`; absent = system).
/// Erases because it BRANCHES between two piece types at build time (docs/api-style.md) —
/// the empty column or the row — and its sibling [`language_picker`] matches it so the two
/// stay interchangeable as section rows.
pub fn appearance_picker(prefs_key: &'static str) -> AnyPiece {
    if day_core::capability(day_spec::Cap::Appearance) == day_spec::Support::Unsupported {
        return column(()).any();
    }
    let ix = Signal::new(match day_part_prefs::get(prefs_key).as_deref() {
        Some("light") => 0,
        Some("dark") => 1,
        _ => 2,
    });
    watch(
        move || ix.get(),
        move |ix, _| match ix {
            0 => {
                day_part_prefs::set(prefs_key, "light");
                day_core::set_appearance(Some(false));
            }
            1 => {
                day_part_prefs::set(prefs_key, "dark");
                day_core::set_appearance(Some(true));
            }
            _ => {
                day_part_prefs::remove(prefs_key);
                day_core::set_appearance(None);
            }
        },
    );
    labeled(
        day_fluent::tr("day-settings-theme"),
        picker(
            [
                day_fluent::tr("day-theme-light").format(),
                day_fluent::tr("day-theme-dark").format(),
                day_fluent::tr("day-theme-system").format(),
            ],
            ix,
        )
        .segmented()
        .id("theme-picker"),
    )
    .any()
}

/// The language row (id `language-picker`): "System" plus every bundled locale under its
/// own name. Selection applies live (`set_locale` — strings re-resolve; layout direction
/// applies on relaunch, docs/localization.md) and persists the tag under `prefs_key`
/// (absent = system). `locales` is the app's generated `res::locales::ALL`.
pub fn language_picker(prefs_key: &'static str, locales: LocaleTable) -> AnyPiece {
    let stored_tag = day_part_prefs::get(prefs_key).unwrap_or_default();
    let ix = Signal::new(
        locales
            .iter()
            .position(|(tag, _)| *tag == stored_tag)
            .map(|i| i + 1)
            .unwrap_or(0),
    );
    watch(
        move || ix.get(),
        move |ix, _| match ix.checked_sub(1).and_then(|i| locales.get(i)) {
            Some((tag, _)) => {
                day_part_prefs::set(prefs_key, tag);
                day_fluent::set_locale(tag);
            }
            None => {
                day_part_prefs::remove(prefs_key);
                day_fluent::set_locale(&system_locale());
            }
        },
    );
    let mut options = vec![day_fluent::tr("day-theme-system").format()];
    options.extend(locales.iter().map(|(_, name)| (*name).to_string()));
    labeled(
        day_fluent::tr("day-settings-language"),
        picker(options, ix).id("language-picker"),
    )
    .any()
}

/// The whole minimal preferences body: one section carrying both labeled rows (each row
/// labels itself — a titled section per row would just repeat the words), ready to drop
/// into a `form((...,))`. Use the rows individually to compose your own sections.
pub fn settings_sections(
    theme_key: &'static str,
    locale_key: &'static str,
    locales: LocaleTable,
) -> AnyPiece {
    section((
        appearance_picker(theme_key),
        language_picker(locale_key, locales),
    ))
    .any()
}
