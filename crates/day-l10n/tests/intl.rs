//! icu4x-backed formatting + collation (docs/localization.md "Formatted values"/"Sorting").
//! These run under bare `cargo` — full compiled data, no thinning pipeline involved — and pin the
//! locale per assertion via `format_in`, so they are independent of DAY_LOCALE and each other.

use day_l10n::{FArg, compare_in, format_in, install};

fn install_fixture() {
    install(
        "en",
        &[
            (
                "en",
                r#"
plain = { $n }
num = { NUMBER($n) }
frac = { NUMBER($n, minimumFractionDigits: 2) }
maxfrac = { NUMBER($n, maximumFractionDigits: 1) }
nogroup = { NUMBER($n, useGrouping: "false") }
pct = { NUMBER($n, style: "percent") }
sig = { NUMBER($n, maximumSignificantDigits: 3) }
count = { $count ->
    [one] one item
   *[other] { $count } items
}
when = { DATETIME($d) }
when_long = { DATETIME($d, dateStyle: "long") }
when_time = { DATETIME($d, timeStyle: "short") }
"#,
            ),
            (
                "de",
                "plain = { $n }\nnum = { NUMBER($n) }\nwhen_long = { DATETIME($d, dateStyle: \"long\") }\n",
            ),
            (
                "fr",
                "plain = { $n }\nwhen_long = { DATETIME($d, dateStyle: \"long\") }\n",
            ),
            ("ar-EG", "plain = { $n }\n"),
            ("zh-CN", "plain = { $n }\n"),
        ],
    );
}

fn fmt(locale: &str, key: &str, name: &str, arg: FArg) -> String {
    day_l10n::strip_isolates(&format_in(locale, key, &[(name.to_string(), arg)]))
}

fn num(locale: &str, key: &str, v: f64) -> String {
    fmt(locale, key, "n", FArg::Num(v))
}

fn dt(locale: &str, key: &str, v: &str) -> String {
    fmt(locale, key, "d", FArg::Str(v.to_string()))
}

// ---------------------------------------------------------------------------
// NUMBER + the bundle-wide formatter
// ---------------------------------------------------------------------------

#[test]
fn plain_interpolations_localize() {
    install_fixture();
    // The set_formatter hook covers plain `{ $n }` — not just explicit NUMBER() calls.
    assert_eq!(num("en", "plain", 1234567.891), "1,234,567.891");
    assert_eq!(num("de", "plain", 1234567.891), "1.234.567,891");
    // fr groups with narrow no-break space (U+202F).
    assert_eq!(
        num("fr", "plain", 1234567.891),
        "1\u{202f}234\u{202f}567,891"
    );
}

#[test]
fn number_function_formats() {
    install_fixture();
    assert_eq!(num("en", "num", 1234567.891), "1,234,567.891");
    assert_eq!(num("de", "num", 1234567.891), "1.234.567,891");
}

#[test]
fn ecma_default_max_three_fraction_digits() {
    install_fixture();
    // 0.1 + 0.2 style float noise must not leak into translations.
    assert_eq!(num("en", "plain", 0.1 + 0.2), "0.3");
}

#[test]
fn fraction_digit_options() {
    install_fixture();
    assert_eq!(num("en", "frac", 5.0), "5.00");
    assert_eq!(num("en", "maxfrac", 2.65437), "2.7");
}

#[test]
fn grouping_can_be_disabled() {
    install_fixture();
    assert_eq!(num("en", "nogroup", 1234567.0), "1234567");
}

#[test]
fn percent_style() {
    install_fixture();
    assert_eq!(num("en", "pct", 0.72), "72%");
}

#[test]
fn significant_digits() {
    install_fixture();
    assert_eq!(num("en", "sig", 1234.567), "1,230");
}

#[test]
fn plural_select_still_works_with_formatter() {
    install_fixture();
    assert_eq!(
        fmt("en", "count", "count", FArg::Num(1.0)),
        "one item",
        "plural category selection unaffected by the formatter"
    );
    assert_eq!(fmt("en", "count", "count", FArg::Num(3.0)), "3 items");
}

// ---------------------------------------------------------------------------
// DATETIME
// ---------------------------------------------------------------------------

#[test]
fn datetime_default_styles_by_input_shape() {
    install_fixture();
    assert_eq!(dt("en", "when", "2026-07-18"), "Jul 18, 2026");
    assert_eq!(dt("en", "when", "14:45"), "2:45\u{202f}PM");
    assert_eq!(
        dt("en", "when", "2026-07-18T14:45"),
        "Jul 18, 2026, 2:45\u{202f}PM"
    );
}

#[test]
fn datetime_styles_localize() {
    install_fixture();
    assert_eq!(dt("en", "when_long", "2026-07-18"), "July 18, 2026");
    assert_eq!(dt("de", "when_long", "2026-07-18"), "18. Juli 2026");
    assert_eq!(dt("fr", "when_long", "2026-07-18"), "18 juillet 2026");
}

#[test]
fn datetime_epoch_seconds_input() {
    install_fixture();
    // 2026-07-18 14:45:00 UTC = 1784385900 epoch seconds.
    let out = fmt("en", "when", "d", FArg::Num(1_784_385_900.0));
    assert_eq!(out, "Jul 18, 2026, 2:45\u{202f}PM");
}

#[test]
fn datetime_garbage_is_echoed_not_blank() {
    install_fixture();
    assert_eq!(dt("en", "when", "not-a-date"), "not-a-date");
}

// ---------------------------------------------------------------------------
// Collation
// ---------------------------------------------------------------------------

#[test]
fn french_accents_collate_correctly() {
    use std::cmp::Ordering;
    // Base letters compare before accents (UCA secondary level): unaccented < accented, and the
    // earlier accent position wins — coté (accent on é, position 4) < côte (accent on ô,
    // position 2). Naive byte order would put both after "cote" by code point instead.
    assert_eq!(compare_in("fr", "cote", "coté"), Ordering::Less);
    assert_eq!(compare_in("fr", "coté", "côte"), Ordering::Less);
    assert_eq!(
        compare_in("en", "a", "B"),
        Ordering::Less,
        "case-insensitive primary strength"
    );
}

#[test]
fn chinese_sorts_by_pinyin() {
    // 北京 běi (U+5317), 广州 guǎng (U+5E7F), 上海 shàng (U+4E0A): code-point order is
    // 上海 < 北京 < 广州, but pinyin order is běi < guǎng < shàng.
    let mut cities = ["上海", "广州", "北京"];
    cities.sort_by(|a, b| compare_in("zh", a, b));
    assert_eq!(
        cities,
        ["北京", "广州", "上海"],
        "pinyin, not code-point, order"
    );
}

#[test]
fn chinese_stroke_extension_differs() {
    // Stroke order: 上 (3 strokes) < 广 (3, later radical order) < 北 (5)… — the exact order is
    // data-defined; the invariant we pin is that the -u-co-stroke tailoring CHANGES the result
    // vs pinyin for at least one pair.
    let pinyin = compare_in("zh", "上海", "北京");
    let stroke = compare_in("zh-u-co-stroke", "上海", "北京");
    assert_ne!(
        (pinyin, stroke),
        (std::cmp::Ordering::Greater, std::cmp::Ordering::Greater),
        "stroke tailoring loads and differs from pinyin for 上海/北京"
    );
}

// ---------------------------------------------------------------------------
// Locale resolution (base_locale fix)
// ---------------------------------------------------------------------------

#[test]
fn extension_locales_resolve_bundles() {
    install_fixture();
    // A collation-extension locale still finds its translations (bundle lookup strips -u-…).
    assert_eq!(num("zh-CN-u-co-stroke", "plain", 5.0), "5");
    // Region fallback: no ar bundle is registered under "ar", but ar-EG is exact.
    assert!(!num("ar-EG", "plain", 5.0).is_empty());
}

#[test]
fn pseudolocale_unaffected() {
    install_fixture();
    let out = num("en-XA", "num", 12.0);
    assert!(
        !out.is_empty() && out != "⟨num⟩",
        "en-XA still resolves: {out}"
    );
}
