// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-fluent: the piece-facing localization layer over the [`day_l10n`] engine (DESIGN.md §12).
//!
//! The engine (bundles, the current-locale [`Signal`], `format_in`, and the built-in core catalog of
//! standard UI strings) now lives in `day-l10n`, low enough that the central crates localize their
//! own strings. This crate re-exports that engine and adds the reactive `tr()` **text source**:
//! `label(tr("greeting").arg("name", name))`, whose value re-reads the locale signal plus any signal
//! args, so a locale switch updates every visible string fine-grained, followed by one relayout.

use day_pieces::{IntoText, TextSource};

// Re-export the engine so the app-facing API (`install_locales`, `set_locale`, …) is unchanged.
pub use day_l10n::{
    Catalog, FArg, IntoFArg, IntoNumberFArg, SigM, ValM, add_launch_locales, compare, compare_in,
    format_catalog, format_decimal, format_decimal_in, format_in, locale, matches_search,
    matches_search_in, register_catalog, set_launch_locale, set_launch_locales, set_locale,
    sort_localized, strip_isolates, t,
};

/// Register the app's locales (see [`day_l10n::install`]) and fix the layout direction from the
/// locale that actually resolved (docs/localization): an RTL locale (Arabic, Hebrew, …) mirrors
/// every horizontal placement and flips the native toolkit's direction. Direction is resolved
/// once, before the first layout; runtime `set_locale` switches strings but not direction.
pub fn install(default: &str, locales: &[(&str, &str)]) {
    day_l10n::install(default, locales);
    day_core::set_layout_direction(day_core::direction_of_locale(
        &day_l10n::locale().get_untracked(),
    ));
}

/// A localized text source: `label(tr("greeting").arg("name", name))` (§12.2).
#[derive(Clone)]
pub struct LocalizedText {
    key: String,
    catalog: Option<&'static Catalog>,
    args: Vec<(String, FArg)>,
}

pub fn tr(key: &str) -> LocalizedText {
    LocalizedText {
        key: key.to_owned(),
        catalog: None,
        args: Vec::new(),
    }
}

/// Bind a key to a private crate/file catalog. Prefer generated typed accessors.
pub fn tr_in(catalog: &'static Catalog, key: &str) -> LocalizedText {
    register_catalog(catalog);
    LocalizedText {
        key: key.to_owned(),
        catalog: Some(catalog),
        args: Vec::new(),
    }
}

/// Include the private localization modules written by `day_build::generate_locales()`.
/// Usable by reusable pieces without depending on the `day` umbrella crate.
#[macro_export]
macro_rules! locales {
    () => {
        pub mod res {
            include!(concat!(env!("OUT_DIR"), "/day_locales.rs"));
        }
    };
}

impl LocalizedText {
    pub fn arg<M>(mut self, name: &str, value: impl IntoFArg<M>) -> Self {
        self.args.push((name.to_owned(), value.into_farg()));
        self
    }

    /// Tracked format: reads the locale signal + any signal args.
    pub fn format(&self) -> String {
        match self.catalog {
            Some(catalog) => format_catalog(catalog, &locale().get(), &self.key, &self.args),
            None => format_in(&locale().get(), &self.key, &self.args),
        }
    }
}

pub struct LocalizedMark;

impl IntoText<LocalizedMark> for LocalizedText {
    fn into_text(self) -> TextSource {
        TextSource::Dyn(std::rc::Rc::new(move || self.format()))
    }
}
