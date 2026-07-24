//! Per-miniapp Fluent localization (docs/lite.md §7): each package ships
//! `i18n/<locale>.ftl` files; `day.i18n.t(key, args?)` formats through a per-app
//! `FluentBundle`. Locale resolution: `DAY_LOCALE` env (what `day launch --locale` and the
//! test runner deliver), else day's live locale signal, with a `<lang>` then `en` fallback
//! chain. Bidi isolates are stripped from output the same way day-l10n does for its own
//! catalogs, so scripted `assert_text` matches what authors wrote.

use std::cell::RefCell;

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};

use crate::store::Store;

pub struct I18n {
    /// The loaded bundles, primary first (resolved locale, then fallbacks). Rebuilt when
    /// the locale changes.
    state: RefCell<Option<Loaded>>,
}

struct Loaded {
    locale: String,
    bundles: Vec<FluentBundle<FluentResource>>,
}

impl Default for I18n {
    fn default() -> Self {
        I18n {
            state: RefCell::new(None),
        }
    }
}

fn current_locale() -> String {
    if let Ok(l) = std::env::var("DAY_LOCALE")
        && !l.is_empty()
    {
        return l;
    }
    day_l10n::locale().get()
}

/// The candidate file stems for a locale, most specific first: `zh-CN` → ["zh-CN", "zh",
/// "en"], deduplicated.
fn chain(locale: &str) -> Vec<String> {
    let mut out = vec![locale.to_string()];
    if let Some((lang, _)) = locale.split_once('-')
        && !out.contains(&lang.to_string())
    {
        out.push(lang.to_string());
    }
    if !out.contains(&"en".to_string()) {
        out.push("en".to_string());
    }
    out
}

fn load_bundle(store: &Store, app_id: &str, stem: &str) -> Option<FluentBundle<FluentResource>> {
    let bytes = store.read_file(app_id, &format!("i18n/{stem}.ftl")).ok()?;
    let source = String::from_utf8_lossy(&bytes).into_owned();
    let resource = FluentResource::try_new(source).ok()?;
    let lang = stem.parse().unwrap_or_else(|_| "en".parse().expect("en"));
    let mut bundle = FluentBundle::new(vec![lang]);
    bundle.add_resource(resource).ok()?;
    Some(bundle)
}

impl I18n {
    /// Format `key` with `args`. Missing keys and apps that ship no `i18n/` fall back to
    /// the key itself, so unlocalized apps keep working.
    pub fn t(&self, store: &Store, app_id: &str, key: &str, args: &[(String, String)]) -> String {
        let locale = current_locale();
        {
            let state = self.state.borrow();
            if state.as_ref().is_none_or(|s| s.locale != locale) {
                drop(state);
                let bundles = chain(&locale)
                    .iter()
                    .filter_map(|stem| load_bundle(store, app_id, stem))
                    .collect();
                *self.state.borrow_mut() = Some(Loaded {
                    locale: locale.clone(),
                    bundles,
                });
            }
        }
        let state = self.state.borrow();
        let Some(loaded) = state.as_ref() else {
            return key.to_string();
        };
        let mut fargs = FluentArgs::new();
        for (k, v) in args {
            // Numeric strings format as numbers so `{ $n }` renders per-locale.
            if let Ok(n) = v.parse::<f64>() {
                fargs.set(k.clone(), FluentValue::from(n));
            } else {
                fargs.set(k.clone(), FluentValue::from(v.clone()));
            }
        }
        for bundle in &loaded.bundles {
            if let Some(msg) = bundle.get_message(key)
                && let Some(pattern) = msg.value()
            {
                let mut errs = Vec::new();
                let out = bundle.format_pattern(pattern, Some(&fargs), &mut errs);
                return day_l10n::strip_isolates(&out);
            }
        }
        key.to_string()
    }
}
