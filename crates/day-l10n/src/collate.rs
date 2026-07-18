//! Locale-aware string comparison and sorting (docs/localization.md "Sorting"), backed by
//! icu4x's collator. `zh` sorts by pinyin (the CLDR default); `-u-co-` locale extensions select
//! tailorings explicitly (`"zh-u-co-stroke"`). Collators are cached per full locale string in a
//! thread-local (single-threaded, like the bundle state); on any failure the comparison degrades
//! to code-point order rather than erroring.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;

use icu_collator::options::CollatorOptions;
use icu_collator::{Collator, CollatorBorrowed};

thread_local! {
    /// `None` = construction failed for that locale (negative-cached to avoid re-trying per call).
    static COLLATORS: RefCell<HashMap<String, Option<CollatorBorrowed<'static>>>> =
        RefCell::new(HashMap::new());
}

fn with_collator<R>(locale: &str, f: impl FnOnce(Option<&CollatorBorrowed<'static>>) -> R) -> R {
    COLLATORS.with(|c| {
        let mut map = c.borrow_mut();
        let entry = map.entry(locale.to_string()).or_insert_with(|| {
            let parsed: icu_locale_core::Locale = locale.parse().ok()?;
            Collator::try_new((&parsed).into(), CollatorOptions::default()).ok()
        });
        f(entry.as_ref())
    })
}

/// Compare two strings in `locale`'s collation order (untracked; accepts `-u-co-` extensions,
/// e.g. `"zh-u-co-stroke"`). Falls back to code-point order if the locale has no collation data.
pub fn compare_in(locale: &str, a: &str, b: &str) -> Ordering {
    with_collator(locale, |c| match c {
        Some(c) => c.compare(a, b),
        None => a.cmp(b),
    })
}

/// Compare two strings in the CURRENT locale's collation order. Reads the locale signal
/// (tracked), so a sort inside a reactive closure re-runs when the locale switches.
pub fn compare(a: &str, b: &str) -> Ordering {
    let locale = crate::locale().get();
    compare_in(&locale, a, b)
}

/// Sort a slice in place in the CURRENT locale's collation order (tracked, like [`compare`]).
pub fn sort_localized<T: AsRef<str>>(items: &mut [T]) {
    let locale = crate::locale().get();
    with_collator(&locale, |c| match c {
        Some(c) => items.sort_by(|a, b| c.compare(a.as_ref(), b.as_ref())),
        None => items.sort_by(|a, b| a.as_ref().cmp(b.as_ref())),
    });
}
