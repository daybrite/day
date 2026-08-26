// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Self-contained localization for the consent surface (feature `ui`). The four catalogs are
//! embedded (a library crate must not touch the app-owned `day_l10n::install`), formatted through
//! a per-locale `FluentBundle`, and resolved off the live locale — the day-lite i18n pattern
//! (`crates/day-lite/src/i18n.rs`) minus the on-disk store.

use std::cell::RefCell;

use fluent_bundle::{FluentBundle, FluentResource};

const CATALOGS: &[(&str, &str)] = &[
    ("en", include_str!("i18n/en.ftl")),
    ("fr", include_str!("i18n/fr.ftl")),
    ("ar", include_str!("i18n/ar.ftl")),
    ("zh-CN", include_str!("i18n/zh-CN.ftl")),
];

day_reactive::tls_slots! {
    i18n;
    /// The bundle chain for the current locale (primary first, `en` last), rebuilt on locale change.
    static STATE: RefCell<Option<Loaded>> = const { RefCell::new(None) };

}

struct Loaded {
    locale: String,
    bundles: Vec<FluentBundle<FluentResource>>,
}

fn current_locale() -> String {
    if let Ok(l) = std::env::var("DAY_LOCALE")
        && !l.is_empty()
    {
        return l;
    }
    day_l10n::locale().get()
}

/// Candidate catalog stems, most specific first: `zh-CN` → [`zh-CN`, `zh`, `en`], deduped.
fn chain(locale: &str) -> Vec<String> {
    let mut out = vec![locale.to_string()];
    if let Some((lang, _)) = locale.split_once('-')
        && !out.iter().any(|s| s == lang)
    {
        out.push(lang.to_string());
    }
    if !out.iter().any(|s| s == "en") {
        out.push("en".to_string());
    }
    out
}

fn build_bundle(stem: &str) -> Option<FluentBundle<FluentResource>> {
    let source = CATALOGS.iter().find(|(l, _)| *l == stem).map(|(_, s)| *s)?;
    let resource = FluentResource::try_new(source.to_string()).ok()?;
    let lang = stem
        .parse()
        .unwrap_or_else(|_| "en".parse().expect("en is a valid langid"));
    let mut bundle = FluentBundle::new(vec![lang]);
    bundle.add_resource(resource).ok()?;
    Some(bundle)
}

/// Translate `key` for the current locale. Unknown keys fall back to the key itself (so a typo is
/// visible, never a panic). No message takes arguments in this catalog.
pub(crate) fn t(key: &str) -> String {
    let locale = current_locale();
    STATE.with(|cell| {
        {
            let state = cell.borrow();
            if state.as_ref().is_none_or(|s| s.locale != locale) {
                drop(state);
                let bundles = chain(&locale)
                    .iter()
                    .filter_map(|stem| build_bundle(stem))
                    .collect();
                *cell.borrow_mut() = Some(Loaded {
                    locale: locale.clone(),
                    bundles,
                });
            }
        }
        let state = cell.borrow();
        let Some(loaded) = state.as_ref() else {
            return key.to_string();
        };
        for bundle in &loaded.bundles {
            if let Some(msg) = bundle.get_message(key)
                && let Some(pattern) = msg.value()
            {
                let mut errs = Vec::new();
                let out = bundle.format_pattern(pattern, None, &mut errs);
                return day_l10n::strip_isolates(&out);
            }
        }
        key.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_locale_defines_every_key() {
        // en is the source of keys; each other catalog must define the same set.
        let keys = |src: &str| -> Vec<String> {
            src.lines()
                .filter(|l| !l.trim_start().starts_with('#') && l.contains('='))
                .filter_map(|l| l.split('=').next().map(|k| k.trim().to_string()))
                .filter(|k| !k.is_empty())
                .collect()
        };
        let en: Vec<String> = keys(CATALOGS[0].1);
        assert!(!en.is_empty());
        for (loc, src) in &CATALOGS[1..] {
            let ks = keys(src);
            for k in &en {
                assert!(ks.contains(k), "locale {loc} is missing key `{k}`");
            }
        }
    }

    #[test]
    fn all_catalogs_parse() {
        for (loc, _) in CATALOGS {
            assert!(build_bundle(loc).is_some(), "catalog {loc} failed to parse");
        }
    }
}
