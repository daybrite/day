//! Localized search matching (docs/localization.md "Searching"): does a piece of text match what
//! the user typed into a search field?
//!
//! The rule is **case-insensitive prefix of any word**. Typing `s` in an English UI matches
//! "Canvas & shapes", "Device & sensors", "Platform services" and "Stack" — every title with a
//! word beginning in `s` — and not "Toolbars", whose only `s` is inside a word.
//!
//! Two pieces of icu4x do the work that makes this correct outside English:
//!
//! - **Word segmentation** finds where words begin. Splitting on spaces would be wrong for the
//!   scripts that do not write them: `日本語入力` is two words to a reader and one to
//!   `split_whitespace`. The segmenter carries dictionaries and LSTM models for Chinese,
//!   Japanese, Thai, Khmer, Lao and Burmese, so word starts are real there too.
//! - **Case folding**, not `to_lowercase`: folding is the operation Unicode defines for
//!   caseless matching, so `ß` matches `SS` and `Σ`/`σ`/`ς` all match each other. Turkish and
//!   Azerbaijani fold the dotted and dotless I apart, and get the Turkic variant.
//!
//! Matching is on case only. `é` does not match `e` — see the accent-insensitivity note in
//! docs/localization.md.

use icu_casemap::CaseMapper;
use icu_segmenter::WordSegmenter;
use icu_segmenter::options::WordBreakInvariantOptions;

/// Case-fold for caseless comparison. Turkic locales need the variant that keeps `i`/`ı` and
/// `İ`/`I` apart; every other language uses Unicode's language-independent full folding.
fn fold(locale: &str, s: &str) -> String {
    let mapper = CaseMapper::new();
    let lang = locale.split(['-', '_']).next().unwrap_or("");
    match lang {
        "tr" | "az" => mapper.fold_turkic_string(s).into_owned(),
        _ => mapper.fold_string(s).into_owned(),
    }
}

/// The byte offset at which each word of `text` begins.
///
/// The segmenter reports boundaries with the type of the segment BEFORE each one, so a word's
/// start is the previous boundary of any segment that is word-like. Segments that are only
/// spaces or punctuation are skipped, so `&` in "Canvas & shapes" is not a place a search can
/// start.
fn word_starts(text: &str) -> Vec<usize> {
    let segmenter = WordSegmenter::new_auto(WordBreakInvariantOptions::default());
    let mut starts = Vec::new();
    let mut prev = 0usize;
    for (boundary, word_type) in segmenter.segment_str(text).iter_with_word_type() {
        if boundary > prev && word_type.is_word_like() {
            starts.push(prev);
        }
        prev = boundary;
    }
    starts
}

/// Does `text` match `query` under `locale`'s rules? (untracked — pass the locale explicitly).
///
/// An empty or whitespace-only `query` matches everything, so an empty search box filters
/// nothing.
///
/// ```
/// assert!(day_l10n::matches_search_in("en", "Canvas & shapes", "s"));
/// assert!(day_l10n::matches_search_in("en", "Canvas & shapes", "canvas &"));
/// assert!(!day_l10n::matches_search_in("en", "Toolbars", "s"));
/// ```
pub fn matches_search_in(locale: &str, text: &str, query: &str) -> bool {
    let needle = fold(locale, query.trim());
    if needle.is_empty() {
        return true;
    }
    // Fold each candidate SUFFIX rather than the whole string once: full case folding can change
    // a string's length (`ß` folds to `ss`), so byte offsets taken from the original text would
    // not survive folding it. Folding a suffix is sound because full folding is
    // context-independent — unlike lowercasing, which is not (Greek final sigma).
    //
    // A multi-word query works without a separate rule: the word start at 0 lets it match the
    // whole title, so "canvas &" matches "Canvas & shapes".
    word_starts(text)
        .into_iter()
        .any(|start| fold(locale, &text[start..]).starts_with(&needle))
}

/// [`matches_search_in`] against the CURRENT locale. Reads the locale signal (tracked), so a
/// filtered list inside a reactive closure re-filters when the language changes — the same
/// contract [`crate::compare`] has.
pub fn matches_search(text: &str, query: &str) -> bool {
    let locale = crate::locale().get();
    matches_search_in(&locale, text, query)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The documented example, exactly: `s` finds every English title with a word starting in
    /// `s`, and no others.
    #[test]
    fn a_query_matches_the_start_of_any_word() {
        let titles = [
            "About",
            "Canvas & shapes",
            "Controls",
            "Device & sensors",
            "Platform services",
            "Stack",
            "Toolbars",
        ];
        let hit: Vec<&str> = titles
            .iter()
            .copied()
            .filter(|t| matches_search_in("en", t, "s"))
            .collect();
        assert_eq!(
            hit,
            [
                "Canvas & shapes",
                "Device & sensors",
                "Platform services",
                "Stack"
            ]
        );
        // "Toolbars" and "Controls" both contain an `s`, but not at a word start.
        assert!(!matches_search_in("en", "Toolbars", "s"));
        assert!(!matches_search_in("en", "Controls", "s"));
    }

    #[test]
    fn matching_ignores_case_in_both_directions() {
        assert!(matches_search_in("en", "About", "abo"));
        assert!(matches_search_in("en", "about", "ABO"));
        assert!(matches_search_in("en", "ABOUT", "aBo"));
    }

    /// An empty search box filters nothing.
    #[test]
    fn an_empty_query_matches_everything() {
        assert!(matches_search_in("en", "Anything", ""));
        assert!(matches_search_in("en", "Anything", "   "));
        assert!(matches_search_in("en", "", ""));
        // An empty title, though, has no word to start.
        assert!(!matches_search_in("en", "", "a"));
    }

    /// The word start at offset 0 lets a query span words without a separate rule.
    #[test]
    fn a_multi_word_query_matches_from_a_word_start() {
        assert!(matches_search_in("en", "Canvas & shapes", "canvas &"));
        assert!(matches_search_in("en", "Platform services", "platform ser"));
        assert!(!matches_search_in(
            "en",
            "Platform services",
            "services platform"
        ));
    }

    /// Only prefixes match — a substring in the middle of a word does not.
    #[test]
    fn a_query_inside_a_word_does_not_match() {
        assert!(!matches_search_in("en", "Localization", "cal"));
        assert!(matches_search_in("en", "Localization", "loc"));
    }

    /// Folding, not lowercasing: `ß` and `SS` are the same string for caseless matching, and a
    /// Greek final sigma matches its non-final form.
    #[test]
    fn folding_handles_the_cases_lowercasing_gets_wrong() {
        assert!(matches_search_in("de", "Straße", "STRASSE"));
        assert!(matches_search_in("de", "STRASSE", "straße"));
        assert!(matches_search_in("el", "Οδός", "οδός"));
    }

    /// Turkish keeps the dotted and dotless I apart, so `i` must not match `I`'s word there —
    /// and must still match `İ`'s. Every other locale folds them together.
    #[test]
    fn turkish_folds_the_two_letter_is_apart() {
        // In Turkish, uppercase `I` lowercases to dotless `ı`, so a dotted `i` is a different
        // letter and must not match it.
        assert!(!matches_search_in("tr", "Irmak", "i"));
        assert!(matches_search_in("tr", "Irmak", "ı"));
        // The same word in English folds the ordinary way.
        assert!(matches_search_in("en", "Irmak", "i"));
    }

    /// The scripts that do not write spaces: word starts still have to be found, or search is
    /// useless in half the world's UIs. `日本語入力` is not one word.
    #[test]
    fn word_starts_are_found_in_scripts_without_spaces() {
        // Segmented as 日本語 / 入力, so the second word's start is matchable on its own.
        assert!(matches_search_in("ja", "日本語入力", "入力"));
        assert!(matches_search_in("ja", "日本語入力", "日本"));
        // Mid-word, not a word start.
        assert!(!matches_search_in("ja", "日本語入力", "語"));
    }

    /// Arabic: right-to-left text matches on its leading characters like any other script.
    #[test]
    fn arabic_matches_by_word_prefix() {
        // "شريط الأدوات" — "toolbar". Both words are matchable by their own prefix.
        assert!(matches_search_in("ar", "شريط الأدوات", "شريط"));
        assert!(matches_search_in("ar", "شريط الأدوات", "الأ"));
        assert!(!matches_search_in("ar", "شريط الأدوات", "دوات"));
    }

    /// Numbers are word-like to the segmenter, so a version or a count is searchable.
    #[test]
    fn digits_count_as_a_word() {
        assert!(matches_search_in("en", "Day 2026 release", "2026"));
        assert!(matches_search_in("en", "Day 2026 release", "20"));
    }
}
