//! day-fluent — Mozilla Fluent localization (DESIGN.md §12). The current locale is a Signal;
//! every `tr()` binding reads it (plus any signal args), so a locale switch updates every
//! visible string fine-grained, followed by one incremental relayout.
//!
//! MVP notes (design deltas, documented): bundles register from embedded `.ftl` sources via
//! [`install`] (the §18 packaged-resource loader arrives with the asset pipeline); the `en-XA`
//! pseudolocale is a post-format accent/expansion transform (argument values get accented too —
//! the §12.2 TextElement-only transform is a refinement); ICU4X NUMBER/DATETIME functions are
//! not yet registered.

use std::cell::RefCell;
use std::collections::HashMap;

use day_pieces::{IntoText, TextSource};
use day_reactive::Signal;
use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

struct State {
    bundles: HashMap<String, FluentBundle<FluentResource>>,
    default: String,
    locale: Signal<String>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Register locales from `.ftl` sources and initialize the locale signal from (1) the
/// `DAY_LOCALE` launch override, (2) the default. Call once, before building the root piece.
pub fn install(default: &str, locales: &[(&str, &str)]) {
    let mut bundles = HashMap::new();
    for (name, src) in locales {
        let langid: LanguageIdentifier =
            name.parse().unwrap_or_else(|_| "en".parse().unwrap());
        let mut bundle = FluentBundle::new(vec![langid]);
        match FluentResource::try_new((*src).to_string()) {
            Ok(res) => {
                let _ = bundle.add_resource(res);
            }
            Err((res, errs)) => {
                eprintln!("day-fluent: {name}: {} syntax error(s)", errs.len());
                let _ = bundle.add_resource(res);
            }
        }
        bundles.insert((*name).to_string(), bundle);
    }
    let initial = std::env::var("DAY_LOCALE")
        .ok()
        .map(normalize)
        .filter(|l| bundles.contains_key(l) || l == "en-XA")
        .unwrap_or_else(|| default.to_string());
    let locale = Signal::new(initial);
    STATE.with(|s| {
        *s.borrow_mut() = Some(State { bundles, default: default.to_string(), locale })
    });
}

fn normalize(l: String) -> String {
    // fr_FR / fr-FR → try exact, else the language half.
    l.replace('_', "-")
}

/// The current locale (tracked read inside bindings).
pub fn locale() -> Signal<String> {
    STATE.with(|s| s.borrow().as_ref().expect("day_fluent::install not called").locale)
}

/// Switch the locale at runtime — every `tr` binding re-runs.
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

/// A localized text source: `label(tr("greeting").arg("name", name))` (§12.2).
#[derive(Clone)]
pub struct LocalizedText {
    key: String,
    args: Vec<(String, FArg)>,
}

pub fn tr(key: &str) -> LocalizedText {
    LocalizedText { key: key.to_owned(), args: Vec::new() }
}

impl LocalizedText {
    pub fn arg<M>(mut self, name: &str, value: impl IntoFArg<M>) -> Self {
        self.args.push((name.to_owned(), value.into_farg()));
        self
    }

    /// Tracked format: reads the locale signal + any signal args.
    pub fn format(&self) -> String {
        let loc = locale().get();
        format_in(&loc, &self.key, &self.args)
    }
}

/// Resolve `key` in `locale` (per-message fallback to the default bundle; `en-XA` transforms
/// the default). Signal args are tracked reads.
pub fn format_in(locale_name: &str, key: &str, args: &[(String, FArg)]) -> String {
    STATE.with(|s| {
        let state = s.borrow();
        let Some(state) = state.as_ref() else {
            return format!("⟨{key}⟩");
        };
        let mut fargs = FluentArgs::new();
        for (k, v) in args {
            fargs.set(k.clone(), v.resolve());
        }
        let pseudo = locale_name == "en-XA";
        let lookup = if pseudo { state.default.as_str() } else { locale_name };
        let bundle = state
            .bundles
            .get(lookup)
            .or_else(|| state.bundles.get(lookup.split('-').next().unwrap_or(lookup)))
            .or_else(|| state.bundles.get(&state.default));
        let Some(bundle) = bundle else { return format!("⟨{key}⟩") };
        let msg = bundle
            .get_message(key)
            .or_else(|| state.bundles.get(&state.default).and_then(|b| b.get_message(key)));
        let Some(msg) = msg else { return format!("⟨{key}⟩") };
        let Some(pattern) = msg.value() else { return format!("⟨{key}⟩") };
        let mut errs = Vec::new();
        let out = bundle.format_pattern(pattern, Some(&fargs), &mut errs).into_owned();
        if pseudo { pseudolocalize(&out) } else { out }
    })
}

/// Accent + expansion transform for layout-stress testing (`en-XA`).
fn pseudolocalize(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| match c {
            'a' => 'á', 'e' => 'é', 'i' => 'í', 'o' => 'ó', 'u' => 'ú', 'y' => 'ý',
            'A' => 'Á', 'E' => 'É', 'I' => 'Í', 'O' => 'Ó', 'U' => 'Ú', 'Y' => 'Ý',
            other => other,
        })
        .collect();
    out.push_str(" ・ロング");
    out
}

/// Strip Fluent's FSI/PDI isolation marks (dayscript text comparison, §14.3).
pub fn strip_isolates(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, '\u{2068}' | '\u{2069}')).collect()
}

pub struct LocalizedMark;

impl IntoText<LocalizedMark> for LocalizedText {
    fn into_text(self) -> TextSource {
        TextSource::Dyn(std::rc::Rc::new(move || self.format()))
    }
}
