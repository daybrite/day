//! day-l10n — the core localization engine (DESIGN.md §12), low enough in the crate graph that even
//! the central crates (day-pieces' dialogs, menu-role labels) can localize their own UI strings.
//!
//! Two tiers of Fluent bundles:
//! - **core** — a built-in catalog (`catalog/*.ftl`) of the standard strings the framework itself
//!   emits (dialog OK/Cancel, standard menu commands), shipped in several languages. Always present.
//! - **app** — the locales an app registers with [`install`]. These take precedence over core, so an
//!   app can override any `day-*` string, and core is the fallback for keys the app didn't define.
//!
//! The current locale is a [`Signal`], so every `format`/binding re-runs on a locale switch. The
//! piece-facing reactive `tr()` wrapper lives one layer up in `day-fluent` (which re-exports this).

use std::cell::RefCell;
use std::collections::HashMap;

use day_reactive::Signal;
use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

mod collate;
mod intl;

pub use collate::{compare, compare_in, sort_localized};

/// The built-in core catalog: standard UI strings the framework needs, per language. Apps override
/// individual keys by defining them in their own catalog (see [`install`]).
const CORE_CATALOG: &[(&str, &str)] = &[
    ("en", include_str!("../catalog/en.ftl")),
    ("fr", include_str!("../catalog/fr.ftl")),
    ("es", include_str!("../catalog/es.ftl")),
    ("de", include_str!("../catalog/de.ftl")),
    ("ja", include_str!("../catalog/ja.ftl")),
    ("zh", include_str!("../catalog/zh.ftl")),
];

struct State {
    /// App-registered bundles (from `install`), keyed by locale — take precedence over `core`.
    app: HashMap<String, FluentBundle<FluentResource>>,
    /// Built-in core catalog bundles, keyed by locale — the fallback for `day-*` keys.
    core: HashMap<String, FluentBundle<FluentResource>>,
    default: String,
    locale: Signal<String>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn build_bundles(locales: &[(&str, &str)]) -> HashMap<String, FluentBundle<FluentResource>> {
    let mut bundles = HashMap::new();
    for (name, src) in locales {
        let langid: LanguageIdentifier = name.parse().unwrap_or_else(|_| "en".parse().unwrap());
        let mut bundle = FluentBundle::new(vec![langid]);
        // icu4x-backed NUMBER()/DATETIME() + the bundle-wide number formatter (src/intl.rs).
        intl::register(&mut bundle);
        // Overriding the same key across resources (app > core when merged) is expected — Fluent
        // errors on duplicate adds, so we keep app and core in SEPARATE bundles and pick at lookup.
        match FluentResource::try_new((*src).to_string()) {
            Ok(res) => {
                let _ = bundle.add_resource(res);
            }
            Err((res, errs)) => {
                eprintln!("day-l10n: {name}: {} syntax error(s)", errs.len());
                let _ = bundle.add_resource(res);
            }
        }
        bundles.insert((*name).to_string(), bundle);
    }
    bundles
}

/// Lazily create the state with the core catalog loaded, so core strings (and `format_in`) work even
/// before an app calls [`install`]. The initial locale honors `DAY_LOCALE`.
fn ensure_state() {
    STATE.with(|s| {
        if s.borrow().is_some() {
            return;
        }
        let initial = std::env::var("DAY_LOCALE")
            .ok()
            .map(normalize)
            .unwrap_or_else(|| "en".to_string());
        *s.borrow_mut() = Some(State {
            app: HashMap::new(),
            core: build_bundles(CORE_CATALOG),
            default: "en".to_string(),
            locale: Signal::new(initial),
        });
    });
}

/// Register an app's locales from `.ftl` sources and set the current locale from (1) the `DAY_LOCALE`
/// launch override, (2) `default`. Call once, before building the root piece. The built-in core
/// catalog is preserved (and remains the fallback); the app's strings take precedence over it, and
/// re-registering reuses the existing locale [`Signal`] so bindings created earlier keep working.
pub fn install(default: &str, locales: &[(&str, &str)]) {
    ensure_state();
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let st = st.as_mut().unwrap();
        st.app = build_bundles(locales);
        st.default = default.to_string();
        let initial = std::env::var("DAY_LOCALE")
            .ok()
            .map(normalize)
            .filter(|l| {
                // Accept anything that RESOLVES — exactly, sans `-u-…` extension, or by language
                // half (`fr-FR` → `fr`) — mirroring `message_from`'s lookup, so a regional or
                // extension-carrying launch locale isn't silently dropped to the default.
                let lang = l.split('-').next().unwrap_or(l);
                [l.as_str(), base_locale(l), lang]
                    .iter()
                    .any(|k| st.app.contains_key(*k) || st.core.contains_key(*k))
                    || l == "en-XA"
            })
            .unwrap_or_else(|| default.to_string());
        st.locale.set(initial);
    });
}

fn normalize(l: String) -> String {
    // fr_FR / fr-FR → try exact, else the language half.
    l.replace('_', "-")
}

/// The locale string without any `-u-…` Unicode extension: `"zh-u-co-stroke"` → `"zh"`. Bundle
/// lookup uses the base (translations don't vary by collation/numbering preferences); the
/// collator ([`compare_in`]) keeps the full string so extensions still select tailorings.
fn base_locale(l: &str) -> &str {
    l.split("-u-").next().unwrap_or(l)
}

/// The current locale (a tracked read inside bindings).
pub fn locale() -> Signal<String> {
    ensure_state();
    STATE.with(|s| s.borrow().as_ref().unwrap().locale)
}

/// Switch the locale at runtime — every `tr`/binding re-runs.
pub fn set_locale(l: &str) {
    locale().set(normalize(l.to_string()));
}

#[derive(Clone)]
pub enum FArg {
    Str(String),
    Num(f64),
    SigStr(Signal<String>),
    SigI64(Signal<i64>),
    SigF64(Signal<f64>),
}

impl FArg {
    /// Tracked resolution (signal args subscribe the binding).
    fn resolve(&self) -> FluentValue<'static> {
        match self {
            FArg::Str(s) => FluentValue::from(s.clone()),
            FArg::Num(n) => FluentValue::from(*n),
            FArg::SigStr(s) => FluentValue::from(s.get()),
            FArg::SigI64(s) => FluentValue::from(s.get()),
            FArg::SigF64(s) => FluentValue::from(s.get()),
        }
    }
}

/// Disjoint-marker conversion for `.arg` values (the same E0119 dodge as `IntoText`).
pub trait IntoFArg<M> {
    fn into_farg(self) -> FArg;
}
pub struct ValM;
pub struct SigM;
impl IntoFArg<ValM> for &str {
    fn into_farg(self) -> FArg {
        FArg::Str(self.to_owned())
    }
}
impl IntoFArg<ValM> for String {
    fn into_farg(self) -> FArg {
        FArg::Str(self)
    }
}
impl IntoFArg<ValM> for i64 {
    fn into_farg(self) -> FArg {
        FArg::Num(self as f64)
    }
}
impl IntoFArg<ValM> for f64 {
    fn into_farg(self) -> FArg {
        FArg::Num(self)
    }
}
impl IntoFArg<SigM> for Signal<String> {
    fn into_farg(self) -> FArg {
        FArg::SigStr(self)
    }
}
impl IntoFArg<SigM> for Signal<i64> {
    fn into_farg(self) -> FArg {
        FArg::SigI64(self)
    }
}
impl IntoFArg<SigM> for Signal<f64> {
    fn into_farg(self) -> FArg {
        FArg::SigF64(self)
    }
}

/// Marker for [`IntoFArg`] values that carry a **number** — required for a Fluent variable used as a
/// plural / `select` selector, where CLDR plural rules select on a number (so a string can't be
/// passed there by mistake). Implemented for the numeric `IntoFArg` types only (`i64`, `f64`, and
/// their `Signal`s); the generated `res::str::<key>(…)` functions (§18.5) type such parameters as
/// `impl IntoNumberFArg` instead of `impl IntoFArg`.
pub trait IntoNumberFArg<M>: IntoFArg<M> {}
impl IntoNumberFArg<ValM> for i64 {}
impl IntoNumberFArg<ValM> for f64 {}
impl IntoNumberFArg<SigM> for Signal<i64> {}
impl IntoNumberFArg<SigM> for Signal<f64> {}

/// Resolve `key` in the given locale from one bundle map (exact locale, then the language half).
fn message_from(
    map: &HashMap<String, FluentBundle<FluentResource>>,
    locale_name: &str,
    key: &str,
    args: &FluentArgs,
) -> Option<String> {
    let bundle = map
        .get(locale_name)
        .or_else(|| map.get(base_locale(locale_name)))
        .or_else(|| {
            let lang = locale_name.split('-').next().unwrap_or(locale_name);
            map.get(lang)
        })?;
    let msg = bundle.get_message(key)?;
    let pattern = msg.value()?;
    let mut errs = Vec::new();
    Some(
        bundle
            .format_pattern(pattern, Some(args), &mut errs)
            .into_owned(),
    )
}

/// Resolve `key`: app bundle for the locale, then app-default, then the built-in core catalog for the
/// locale, then core English. `en-XA` resolves against the default and accents the result.
/// Signal args are tracked reads.
pub fn format_in(locale_name: &str, key: &str, args: &[(String, FArg)]) -> String {
    ensure_state();
    STATE.with(|s| {
        let st = s.borrow();
        let st = st.as_ref().unwrap();
        let mut fargs = FluentArgs::new();
        for (k, v) in args {
            fargs.set(k.clone(), v.resolve());
        }
        let pseudo = locale_name == "en-XA";
        let lookup = if pseudo {
            st.default.as_str()
        } else {
            locale_name
        };
        let out = message_from(&st.app, lookup, key, &fargs)
            .or_else(|| message_from(&st.app, &st.default, key, &fargs))
            .or_else(|| message_from(&st.core, lookup, key, &fargs))
            .or_else(|| message_from(&st.core, "en", key, &fargs));
        match out {
            Some(out) => {
                if pseudo {
                    pseudolocalize(&out)
                } else {
                    out
                }
            }
            None => format!("⟨{key}⟩"),
        }
    })
}

/// Resolve `key` in the CURRENT locale (no args) — the one-shot form the framework's own strings use
/// (dialog buttons, menu-role labels), which are resolved once at present/build time.
pub fn t(key: &str) -> String {
    format_in(&locale().get(), key, &[])
}

/// Accent + expansion transform for layout-stress testing (`en-XA`).
fn pseudolocalize(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| match c {
            'a' => 'á',
            'e' => 'é',
            'i' => 'í',
            'o' => 'ó',
            'u' => 'ú',
            'y' => 'ý',
            'A' => 'Á',
            'E' => 'É',
            'I' => 'Í',
            'O' => 'Ó',
            'U' => 'Ú',
            'Y' => 'Ý',
            other => other,
        })
        .collect();
    out.push_str(" ・ロング");
    out
}

/// Strip Fluent's FSI/PDI isolation marks (dayscript text comparison, §14.3).
pub fn strip_isolates(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_catalog_resolves_without_install_and_across_locales() {
        // Works before any `install` — the core catalog self-initializes.
        assert_eq!(
            strip_isolates(&format_in("en", "day-cancel", &[])),
            "Cancel"
        );
        assert_eq!(
            strip_isolates(&format_in("fr", "day-cancel", &[])),
            "Annuler"
        );
        assert_eq!(
            strip_isolates(&format_in("de", "day-cancel", &[])),
            "Abbrechen"
        );
        assert_eq!(strip_isolates(&format_in("ja", "day-ok", &[])), "OK");
        // Unknown locale → English core fallback.
        assert_eq!(strip_isolates(&format_in("xx", "day-copy", &[])), "Copy");
        // Unknown key → visible marker, never a panic.
        assert_eq!(format_in("en", "nope", &[]), "⟨nope⟩");
    }

    #[test]
    fn menu_role_labels_and_interpolated_app_commands_localize() {
        let app = |lang, key, val: &str| {
            let out = format_in(lang, key, &[("app".to_string(), FArg::Str("Day".into()))]);
            assert_eq!(strip_isolates(&out), val, "{lang}/{key}");
        };
        // Standard menu-command labels (used by day-pieces `lower_menu` for role items).
        assert_eq!(strip_isolates(&format_in("fr", "day-cut", &[])), "Couper");
        assert_eq!(
            strip_isolates(&format_in("de", "day-paste", &[])),
            "Einfügen"
        );
        assert_eq!(
            strip_isolates(&format_in("es", "day-select-all", &[])),
            "Seleccionar todo"
        );
        // Interpolated App-menu commands with per-language word order (AppKit About/Quit).
        app("en", "day-quit-app", "Quit Day");
        app("fr", "day-quit-app", "Quitter Day");
        app("de", "day-quit-app", "Day beenden");
        app("ja", "day-quit-app", "Dayを終了");
        app("fr", "day-about-app", "À propos de Day");
    }

    #[test]
    fn app_bundle_overrides_core_but_core_is_the_fallback() {
        install("en", &[("en", "day-cancel = Dismiss\ngreeting = Hi")]);
        // App key wins for its own strings.
        assert_eq!(strip_isolates(&format_in("en", "greeting", &[])), "Hi");
        // App override of a core key wins.
        assert_eq!(
            strip_isolates(&format_in("en", "day-cancel", &[])),
            "Dismiss"
        );
        // A core key the app didn't define still resolves from the built-in catalog.
        assert_eq!(strip_isolates(&format_in("en", "day-ok", &[])), "OK");
    }
}
