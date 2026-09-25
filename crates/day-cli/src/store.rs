// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Store listings (docs/store.md): one canonical source per app, generated into the layouts App Store
//! Connect and Google Play expect.
//!
//! Everything a listing is made of sits in one file, `store/storefront.toml` (or `store/storefront.yaml`, the
//! same tree in YAML): the submission metadata, the screenshots each listing shows, and the
//! localized text, each declared once and specialized per target, per store and per locale with
//! the nearest declaration winning. The stores disagree about almost everything else: what the
//! fields are called, how long they may be, and how a locale is spelled (`zh-CN` here is `zh-Hans`
//! to Apple and `zh-CN` to Google; Hebrew is `he` to Apple and the legacy `iw-IL` to Google). All of
//! that divergence is handled here, at generation time, rather than by asking the author to keep two
//! parallel trees in step, the same reason `resource/` fans out to per-platform resources rather
//! than being authored per platform.
//!
//! ```toml
//! [storefront.metadata]              # the default locale's text (en when the app has it)
//! name = "Example"                   # ≤30   App Store name / Play title
//! subtitle = "One line"              # ≤30   App Store only
//! short = "One sentence."            # ≤80   Play short description
//! description = """…"""              # ≤4000 both
//! keywords = ["a", "b"]              # ≤100  App Store only, counted joined by commas
//! release-notes = "…"                # ≤4000 App Store, ≤500 Play (the stricter one binds)
//! promo = "…"                        # ≤170  App Store promotional text (optional)
//! marketing-url = "https://…"        # and support-url, privacy-url
//! [storefront.metadata.fr]           # a locale's text, over the default's key by key
//! [storefront.ios-uikit.metadata]    # a target's, over the shared; and per store:
//! [storefront.ios-uikit.apple-app-store.metadata.fr]
//! ```
//!
//! Any field may instead be `<field>-ref = "path"`, a project-relative file holding the text (a
//! long description a translation tool manages); a field set both ways is refused. A locale's
//! text may also sit in `store/app.<tag>.toml` beside the main file, the same tree with only
//! `metadata` tables in it, each read as that locale's.
//!
//! `day store stage -p <target>` (and every `day pack` of a store target) writes
//! `build/day/store/<target>/`, a tree `fastlane deliver` / `fastlane supply` accept as-is.
//! Generated, never checked in: the pristine-checkout rule (§20.3) means a build must not write
//! into tracked directories. `day store export` writes the whole listing, resolved, as one JSON
//! document for the app's website and for anything else that publishes it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::cli::CliError;
use crate::meta::Project;

/// A field of a store listing. The variants are Day's vocabulary; each store's own name for the
/// field (and whether it has one at all) lives in `Field::apple` / `Field::play`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    Name,
    Subtitle,
    Short,
    Description,
    Keywords,
    ReleaseNotes,
    Promo,
    MarketingUrl,
    SupportUrl,
    PrivacyUrl,
}

impl Field {
    /// The key this field takes in a `metadata` table.
    pub fn key(self) -> &'static str {
        match self {
            Field::Name => "name",
            Field::Subtitle => "subtitle",
            Field::Short => "short",
            Field::Description => "description",
            Field::Keywords => "keywords",
            Field::ReleaseNotes => "release-notes",
            Field::Promo => "promo",
            Field::MarketingUrl => "marketing-url",
            Field::SupportUrl => "support-url",
            Field::PrivacyUrl => "privacy-url",
        }
    }

    /// The field a `metadata` key names.
    pub fn from_key(key: &str) -> Option<Field> {
        FIELDS.iter().copied().find(|f| f.key() == key)
    }

    /// The file the legacy `store/<locale>/` layout kept this field in (`day store migrate`).
    pub fn legacy_file(self) -> &'static str {
        match self {
            Field::Name => "name.txt",
            Field::Subtitle => "subtitle.txt",
            Field::Short => "short.txt",
            Field::Description => "description.txt",
            Field::Keywords => "keywords.txt",
            Field::ReleaseNotes => "release-notes.txt",
            Field::Promo => "promo.txt",
            Field::MarketingUrl => "marketing-url.txt",
            Field::SupportUrl => "support-url.txt",
            Field::PrivacyUrl => "privacy-url.txt",
        }
    }

    /// Whether the field is a URL, held to `https://`.
    pub fn is_url(self) -> bool {
        matches!(
            self,
            Field::MarketingUrl | Field::SupportUrl | Field::PrivacyUrl
        )
    }
}

/// Every field, in file order.
pub const FIELDS: &[Field] = &[
    Field::Name,
    Field::Subtitle,
    Field::Short,
    Field::Description,
    Field::Keywords,
    Field::ReleaseNotes,
    Field::Promo,
    Field::MarketingUrl,
    Field::SupportUrl,
    Field::PrivacyUrl,
];

/// Whether a Day tag is spelled by any store the CLI stages, per the rules the CLI ships.
pub fn mappable(day_tag: &str) -> bool {
    StoreRules::builtin().mappable(day_tag)
}

/// The non-localized submission metadata at one level of `store/storefront.toml`'s `[storefront]`
/// table: shared, per target, or per store. Every key is optional at every level; a store's
/// value wins over its target's, which wins over the shared one, key by key.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct SubmissionInfo {
    /// Play package name / App Store bundle id. Defaults to the id the target builds with.
    pub bundle_id: Option<String>,
    /// App Store primary category, e.g. `DEVELOPER_TOOLS` (deliver's `primary_category`).
    ///
    /// There is no Play counterpart: Google Play's category is set in the Play Console and
    /// `supply` cannot write it, so recording one here would be a value that silently never
    /// reached the store.
    pub apple_category: Option<String>,
    pub copyright: Option<String>,
    /// Where the store writes back to a human: review contact, support email.
    pub contact_email: Option<String>,
    /// The App Review contact, which App Store Connect refuses without a first name, a last
    /// name and a phone number written with its country code (`+1 555 555 5555`). All three
    /// together with the email make the `review_information/` tree; short of the set, staging
    /// leaves the tree out, and the contact already entered in App Store Connect stands.
    pub contact_first_name: Option<String>,
    pub contact_last_name: Option<String>,
    pub contact_phone: Option<String>,
    /// Free-form notes for the reviewer (deliver's `review_information/notes.txt`).
    pub review_notes: Option<String>,
    /// What happens to an App Store version once App Review approves it: `automatic` (the
    /// default) releases it to the store on its own; `manual` leaves it in Pending Developer
    /// Release until someone presses Release in App Store Connect. Staged as
    /// `DAY_ASC_MANUAL_RELEASE` in the fastlane tree's `.env.default`, which the generated
    /// `submit` and `release` lanes read for deliver's `automatic_release`. Google Play has no
    /// such gate: a completed production release rolls out when its review passes.
    pub apple_release: Option<String>,
}

impl SubmissionInfo {
    const KEYS: [&'static str; 9] = [
        "bundle-id",
        "apple-category",
        "copyright",
        "contact-email",
        "contact-first-name",
        "contact-last-name",
        "contact-phone",
        "review-notes",
        "apple-release",
    ];

    fn parse(
        table: &serde_json::Value,
        at: &str,
        problems: &mut Vec<String>,
    ) -> Result<SubmissionInfo, String> {
        let table = table
            .as_object()
            .ok_or_else(|| format!("{at}: a table of keys, `[{at}]`"))?;
        for extra in table.keys().filter(|k| !Self::KEYS.contains(&k.as_str())) {
            // Read on without it: a typo in one key is a finding, not a listing nobody can stage.
            problems.push(format!(
                "[{at}] {extra}: unknown key; the keys are {}",
                Self::KEYS.join(", ")
            ));
        }
        // Every value is text. YAML types a bare `2026` or `1.2` as a number, which is the one
        // way the two formats diverge, so a non-string is refused by name rather than dropped.
        let get = |k: &str| -> Result<Option<String>, String> {
            match table.get(k) {
                None | Some(serde_json::Value::Null) => Ok(None),
                Some(serde_json::Value::String(s)) => {
                    Ok(Some(s.trim().to_string()).filter(|s| !s.is_empty()))
                }
                Some(other) => Err(format!(
                    "{at}.{k}: a string, not {other}; quote it (\"{other}\") in YAML",
                )),
            }
        };
        Ok(SubmissionInfo {
            bundle_id: get("bundle-id")?,
            apple_category: get("apple-category")?,
            copyright: get("copyright")?,
            contact_email: get("contact-email")?,
            contact_first_name: get("contact-first-name")?,
            contact_last_name: get("contact-last-name")?,
            contact_phone: get("contact-phone")?,
            review_notes: get("review-notes")?,
            apple_release: match get("apple-release")? {
                Some(v) if v == "automatic" || v == "manual" => Some(v),
                Some(other) => {
                    return Err(format!(
                        "{at}.apple-release: \"automatic\" or \"manual\", not {other:?}"
                    ));
                }
                None => None,
            },
        })
    }

    /// This level's values over `base`'s, key by key.
    fn over(&self, base: &SubmissionInfo) -> SubmissionInfo {
        let pick = |mine: &Option<String>, theirs: &Option<String>| {
            mine.clone().or_else(|| theirs.clone())
        };
        SubmissionInfo {
            bundle_id: pick(&self.bundle_id, &base.bundle_id),
            apple_category: pick(&self.apple_category, &base.apple_category),
            copyright: pick(&self.copyright, &base.copyright),
            contact_email: pick(&self.contact_email, &base.contact_email),
            contact_first_name: pick(&self.contact_first_name, &base.contact_first_name),
            contact_last_name: pick(&self.contact_last_name, &base.contact_last_name),
            contact_phone: pick(&self.contact_phone, &base.contact_phone),
            review_notes: pick(&self.review_notes, &base.review_notes),
            apple_release: pick(&self.apple_release, &base.apple_release),
        }
    }

    /// Whether an approved App Store version waits for the Release button (`apple-release =
    /// "manual"`); unset and `automatic` release on approval.
    pub fn manual_release(&self) -> bool {
        self.apple_release.as_deref() == Some("manual")
    }
}

/// Where a listing text came from, for lint to name it and `day lint --fix` to rewrite it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// `key = "…"` in a storefront file: the file's name under `store/`, the table and the key.
    Inline {
        file: String,
        table: String,
        key: &'static str,
    },
    /// `key-ref = "path"`: the project-relative file the text was read from, and the table
    /// and key that named it.
    File {
        file: String,
        table: String,
        key: &'static str,
    },
}

impl Origin {
    /// The place as a finding names it: `store/storefront.toml [storefront.metadata.fr] description`,
    /// or the referenced file followed by the key that named it.
    pub fn describe(&self, store: &str) -> String {
        match self {
            Origin::Inline { file, table, key } => format!("{store}/{file} [{table}] {key}"),
            Origin::File { file, table, key } => format!("{file} ([{table}] {key}-ref)"),
        }
    }

    /// The project-relative file a finding points an editor at.
    pub fn path(&self, store: &str) -> String {
        match self {
            Origin::Inline { file, .. } => format!("{store}/{file}"),
            Origin::File { file, .. } => file.clone(),
        }
    }
}

/// One listing text and where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text {
    pub value: String,
    pub origin: Origin,
}

/// A `metadata` table: the default locale's fields, and a table per other locale over them.
///
/// ```toml
/// [storefront.metadata]
/// name = "Example"
/// keywords = ["rust", "native"]
/// description-ref = "store/metadata/description.txt"
/// [storefront.metadata.fr]
/// name = "Exemple"
/// ```
///
/// A scalar (or a list, for `keywords`) is a field; a table is a locale. A locale's table
/// carries what differs from the default locale's, so URLs and names an app keeps the same
/// everywhere are written once.
#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub base: BTreeMap<Field, Text>,
    pub locales: BTreeMap<String, BTreeMap<Field, Text>>,
}

/// The file a storefront document is read from, and what it is read against.
pub struct Source<'a> {
    /// Its name under `store/`, for messages: `storefront.toml`, `storefront.fr.yaml`.
    pub file: &'a str,
    /// The locale a sidecar file carries; `None` for the main file.
    pub sidecar: Option<&'a str>,
    /// The app's default locale, whose text is the base table itself.
    pub default_locale: &'a str,
    /// Reads a `-ref` path (project-relative) to its text.
    pub read_ref: &'a dyn Fn(&str) -> Result<String, String>,
}

impl<'a> Source<'a> {
    /// A `-ref` path is project-relative and stays inside the project: a test resource lives in
    /// the repository, never at a path one machine happens to have.
    pub fn check_ref(path: &str) -> Result<(), String> {
        // Judged as text rather than through `Path`, so the answer is the same on every host:
        // `Path::is_absolute` says no to `/etc/passwd` on Windows (rooted, no drive) and to
        // `C:\\Users\\…` on Unix, and a listing is read on both.
        let bytes = path.as_bytes();
        let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
        if path.starts_with(['/', '\\', '~']) || drive {
            return Err(format!(
                "{path:?} is not project-relative; a `-ref` names a file inside the project"
            ));
        }
        if path.split(['/', '\\']).any(|c| c == "..") {
            return Err(format!(
                "{path:?} leaves the project; a `-ref` names a file inside it"
            ));
        }
        Ok(())
    }
}

impl Metadata {
    /// Read a `metadata` table at `at` from `src` into this one (a sidecar adds to what the
    /// main file declared; a field set in both is refused).
    fn parse(
        &mut self,
        value: &serde_json::Value,
        at: &str,
        src: &Source,
        problems: &mut Vec<String>,
    ) -> Result<(), String> {
        let table = value.as_object().ok_or_else(|| {
            format!(
                "{}: {at}: a table of fields (name, description, …), with a table per locale",
                src.file
            )
        })?;
        match src.sidecar {
            None => {
                for (key, value) in table {
                    if let serde_json::Value::Object(_) = value {
                        if key == src.default_locale {
                            return Err(format!(
                                "{}: {at}.{key}: {key} is the app's default locale; its text is \
                                 [{at}] itself",
                                src.file
                            ));
                        }
                        let fields = self.locales.entry(key.clone()).or_default();
                        Self::parse_fields(
                            fields,
                            value,
                            &format!("{at}.{key}"),
                            src,
                            true,
                            problems,
                        )?;
                    }
                }
                let plain: serde_json::Map<String, serde_json::Value> = table
                    .iter()
                    .filter(|(_, v)| !v.is_object())
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                Self::parse_fields(
                    &mut self.base,
                    &serde_json::Value::Object(plain),
                    at,
                    src,
                    false,
                    problems,
                )
            }
            Some(tag) => {
                if let Some((key, _)) = table.iter().find(|(_, v)| v.is_object()) {
                    return Err(format!(
                        "{}: {at}.{key}: a locale file's metadata table holds {tag}'s fields; \
                         no locale tables inside it",
                        src.file
                    ));
                }
                if tag == src.default_locale {
                    Self::parse_fields(&mut self.base, value, at, src, false, problems)
                } else {
                    let fields = self.locales.entry(tag.to_string()).or_default();
                    Self::parse_fields(fields, value, at, src, false, problems)
                }
            }
        }
    }

    /// The fields of one table (a locale's, or the base), each inline or by `-ref`.
    fn parse_fields(
        into: &mut BTreeMap<Field, Text>,
        value: &serde_json::Value,
        at: &str,
        src: &Source,
        tables_are_errors: bool,
        problems: &mut Vec<String>,
    ) -> Result<(), String> {
        let table = value
            .as_object()
            .ok_or_else(|| format!("{}: {at}: a table of fields", src.file))?;
        for (key, value) in table {
            if value.is_object() {
                if tables_are_errors {
                    return Err(format!(
                        "{}: {at}.{key}: a locale table holds fields, not further tables",
                        src.file
                    ));
                }
                continue;
            }
            let (name, is_ref) = match key.strip_suffix("-ref") {
                Some(name) => (name, true),
                None => (key.as_str(), false),
            };
            let Some(field) = Field::from_key(name) else {
                problems.push(format!(
                    "{}: [{at}] {key}: unknown key; the fields are {}, each also as `<field>-ref`",
                    src.file,
                    FIELDS
                        .iter()
                        .map(|f| f.key())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                continue;
            };
            let twin = if is_ref {
                name.to_string()
            } else {
                format!("{name}-ref")
            };
            if table.contains_key(&twin) {
                return Err(format!(
                    "{}: {at}: both `{name}` and `{name}-ref` are set; keep one",
                    src.file
                ));
            }
            if let Some(prior) = into.get(&field) {
                return Err(format!(
                    "{}: {at}.{key} is also set by {}; keep one",
                    src.file,
                    prior.origin.describe("store")
                ));
            }
            let raw = if is_ref {
                let path = match value {
                    serde_json::Value::String(s) => s.trim().to_string(),
                    other => {
                        return Err(format!(
                            "{}: {at}.{key}: a project-relative path, not {other}",
                            src.file
                        ));
                    }
                };
                Source::check_ref(&path).map_err(|e| format!("{}: {at}.{key}: {e}", src.file))?;
                let text = (src.read_ref)(&path)
                    .map_err(|e| format!("{}: {at}.{key}: {path}: {e}", src.file))?;
                let text = if field == Field::Keywords {
                    join_keywords(text.split([',', '\n']))
                } else {
                    text
                };
                (
                    text,
                    Origin::File {
                        file: path,
                        table: at.to_string(),
                        key: field.key(),
                    },
                )
            } else {
                let text = match (field, value) {
                    (Field::Keywords, serde_json::Value::Array(items)) => {
                        let mut words = Vec::new();
                        for item in items {
                            let serde_json::Value::String(word) = item else {
                                return Err(format!(
                                    "{}: {at}.{key}: a list of strings, not {item}; quote it \
                                     (\"{item}\") in YAML",
                                    src.file
                                ));
                            };
                            if word.contains(',') {
                                return Err(format!(
                                    "{}: {at}.{key}: {word:?} holds a comma; one keyword per \
                                     item",
                                    src.file
                                ));
                            }
                            words.push(word.as_str());
                        }
                        join_keywords(words.into_iter())
                    }
                    (Field::Keywords, other) => {
                        return Err(format!(
                            "{}: {at}.{key}: a list of keywords, [\"a\", \"b\"], not {other}",
                            src.file
                        ));
                    }
                    (_, serde_json::Value::String(s)) => s.clone(),
                    (_, serde_json::Value::Null) => continue,
                    (_, other) => {
                        return Err(format!(
                            "{}: {at}.{key}: a string, not {other}; quote it (\"{other}\") in YAML",
                            src.file
                        ));
                    }
                };
                (
                    text,
                    Origin::Inline {
                        file: src.file.to_string(),
                        table: at.to_string(),
                        key: field.key(),
                    },
                )
            };
            // The newline an editor ends a file with, or the one before a closing `"""`, is
            // not part of the text; other surrounding whitespace is, and lint reports it.
            let value = raw.0.trim_end_matches(['\n', '\r']).to_string();
            if value.trim().is_empty() {
                continue;
            }
            into.insert(
                field,
                Text {
                    value,
                    origin: raw.1,
                },
            );
        }
        Ok(())
    }

    /// The table for `locale`: its own by exact tag, else one of the same primary language
    /// (`fr` covers `fr-CA`).
    fn table(&self, locale: &str) -> Option<&BTreeMap<Field, Text>> {
        self.locales.get(locale).or_else(|| {
            let lang = crate::screenshot::primary_language(locale);
            self.locales
                .iter()
                .find(|(k, _)| crate::screenshot::primary_language(k) == lang)
                .map(|(_, v)| v)
        })
    }

    pub fn is_empty(&self) -> bool {
        self.base.is_empty() && self.locales.values().all(|l| l.is_empty())
    }
}

/// Keywords as the App Store counts them: joined by commas with no space around any, which is
/// the only form that does not waste the 100-character budget. Empty items are dropped.
fn join_keywords<'a>(words: impl Iterator<Item = &'a str>) -> String {
    words
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(",")
}

/// One declared text, with the level and locale it was declared at, for lint.
pub struct Declared<'a> {
    pub target: Option<&'a str>,
    pub store: Option<&'a str>,
    pub locale: Option<&'a str>,
    pub field: Field,
    pub text: &'a Text,
}

/// One screenshot of a listing: a `screenshot:` step's name and the theme variant to take.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct ListingShot {
    pub shot: String,
    /// `light` unless the declaration says otherwise. A capture with no theme matches any.
    pub theme: String,
}

/// The lists of one `screenshots` table at one locale level: per device kind, and `default`
/// for the kinds it does not name.
#[derive(Clone, Debug, Default)]
pub struct ShotLists {
    pub default: Option<Vec<ListingShot>>,
    pub kinds: BTreeMap<String, Vec<ListingShot>>,
}

impl ShotLists {
    fn get(&self, kind: &str) -> Option<&Vec<ListingShot>> {
        self.kinds.get(kind).or(self.default.as_ref())
    }

    fn lists(&self) -> impl Iterator<Item = &Vec<ListingShot>> {
        self.default.iter().chain(self.kinds.values())
    }
}

/// A `screenshots` table: lists per device kind (and `default`) that apply to every locale,
/// plus the same shape under a locale key for captures of that locale:
///
/// ```toml
/// [storefront.ios-uikit.screenshots]
/// default = ["home", "canvas"]
/// ipad = ["home", "canvas", "grid"]
/// [storefront.ios-uikit.screenshots.fr]      # the French listing leads with localization
/// default = ["localization", "home", "canvas"]
/// ```
///
/// A list is a device kind; a table is a locale. Within one table the locale's lists win
/// over the general ones, kind before `default`; a locale matches by exact tag, then by
/// primary language (`fr` covers `fr-CA`).
#[derive(Clone, Debug, Default)]
pub struct Screenshots {
    pub all: ShotLists,
    pub locales: BTreeMap<String, ShotLists>,
}

impl Screenshots {
    fn parse(value: &serde_json::Value, at: &str) -> Result<Screenshots, String> {
        let table = value.as_object().ok_or_else(|| {
            format!("{at}: a table with `default` and device kinds (`iphone`, `ipad`, `phone`, `tablet`) as lists, and locales as tables")
        })?;
        let mut out = Screenshots::default();
        for (key, value) in table {
            match value {
                serde_json::Value::Object(_) => {
                    let inner = value.as_object().unwrap_or(&serde_json::Map::new()).clone();
                    let mut lists = ShotLists::default();
                    for (kind, value) in &inner {
                        if value.is_object() {
                            return Err(format!(
                                "{at}.{key}.{kind}: a locale table holds lists per device kind, not further tables"
                            ));
                        }
                        let shots = parse_shots(value, &format!("{at}.{key}.{kind}"))?;
                        if kind == "default" {
                            lists.default = Some(shots);
                        } else {
                            lists.kinds.insert(kind.clone(), shots);
                        }
                    }
                    out.locales.insert(key.clone(), lists);
                }
                _ => {
                    let shots = parse_shots(value, &format!("{at}.{key}"))?;
                    if key == "default" {
                        out.all.default = Some(shots);
                    } else {
                        out.all.kinds.insert(key.clone(), shots);
                    }
                }
            }
        }
        Ok(out)
    }

    /// The list for `kind` in `locale`: the locale's (exact tag, then primary language), then
    /// the general one; kind before `default` at each step.
    fn get(&self, kind: &str, locale: &str) -> Option<&Vec<ListingShot>> {
        self.get_localized(kind, locale)
            .or_else(|| self.all.get(kind))
    }

    /// The locale's own list for `kind` (exact tag, then primary language; kind, then
    /// `default`), without the general lists.
    fn get_localized(&self, kind: &str, locale: &str) -> Option<&Vec<ListingShot>> {
        let by_locale = self.locales.get(locale).or_else(|| {
            let lang = crate::screenshot::primary_language(locale);
            self.locales
                .iter()
                .find(|(k, _)| crate::screenshot::primary_language(k) == lang)
                .map(|(_, v)| v)
        });
        by_locale.and_then(|l| l.get(kind))
    }

    fn is_empty(&self) -> bool {
        self.all.default.is_none() && self.all.kinds.is_empty() && self.locales.is_empty()
    }

    fn lists(&self) -> impl Iterator<Item = &Vec<ListingShot>> {
        self.all
            .lists()
            .chain(self.locales.values().flat_map(|l| l.lists()))
    }

    fn kinds(&self) -> impl Iterator<Item = &String> {
        self.all
            .kinds
            .keys()
            .chain(self.locales.values().flat_map(|l| l.kinds.keys()))
    }
}

/// `[storefront.<target>.<store>]`: a store's own submission info, text and screenshots.
#[derive(Clone, Debug, Default)]
pub struct StoreEntry {
    pub info: SubmissionInfo,
    pub metadata: Metadata,
    pub screenshots: Screenshots,
}

/// `[storefront.<target>]`: the target's submission info and text, its own screenshots (what
/// its website page shows, per device kind, and what its stores fall back to), and its stores.
#[derive(Clone, Debug, Default)]
pub struct TargetEntry {
    pub info: SubmissionInfo,
    pub metadata: Metadata,
    pub screenshots: Screenshots,
    pub stores: BTreeMap<String, StoreEntry>,
}

/// `store/storefront.toml`'s (or `store/storefront.yaml`'s) `storefront`: submission metadata, listing text
/// and listing screenshots, declared once for every storefront, per target, or per store, with
/// the nearest level winning. The two formats carry the same tree, parsed into one value first,
/// so a file written in either resolves identically; in YAML the same declaration reads
///
/// ```yaml
/// storefront:
///   submission-info: { copyright: "2026 Example" }
///   metadata:
///     name: Example
///     keywords: [rust, native]
///     fr: { name: Exemple }
///   ios-uikit:
///     screenshots: { default: [home, canvas], ipad: [home, canvas, grid] }
///     apple-app-store:
///       submission-info: { apple-category: DEVELOPER_TOOLS }
///       screenshots: { iphone: [{ name: controls, theme: dark }, layout] }
/// ```
///
/// ```toml
/// [storefront.submission-info]                        # shared by every store
/// copyright = "2026 Example"
/// contact-email = "support@example.com"
/// review-notes = "How to exercise the app."
///
/// [storefront.metadata]                               # the default locale's text
/// name = "Example"
/// keywords = ["rust", "native"]
/// description-ref = "store/metadata/description.txt"  # or inline
///
/// [storefront.metadata.fr]                            # French, over the default key by key
/// name = "Exemple"
///
/// [storefront.ios-uikit.screenshots]                  # the website's ios-uikit page; the stores' fallback
/// default = ["home", "canvas"]
/// ipad = ["home", "canvas", "grid"]
///
/// [storefront.ios-uikit.apple-app-store.submission-info]
/// apple-category = "DEVELOPER_TOOLS"
///
/// [storefront.ios-uikit.apple-app-store.metadata]     # the App Store's own wording
/// subtitle = "Native UI from Rust"
///
/// [storefront.ios-uikit.apple-app-store.screenshots]
/// iphone = [{ name = "controls", theme = "dark" }, "layout"]   # a bare name is the light theme
/// # ipad is not named: the target's ipad list above
///
/// [storefront.android-mdc.google-play-store.screenshots]
/// default = ["canvas", "list"]
/// ```
///
/// Store keys are free: `mac-app-store`, `altstore`, `f-droid` ride through the index for
/// whatever reads them; the CLI itself stages `apple-app-store` and `google-play-store`. A key
/// outside `[storefront]` is not read: `day lint` names it and where it belongs.
#[derive(Clone, Debug, Default)]
pub struct Storefront {
    pub info: SubmissionInfo,
    pub metadata: Metadata,
    pub targets: BTreeMap<String, TargetEntry>,
    /// Top-level keys of a file from before the `[storefront]` layout, ignored here.
    pub legacy_keys: Vec<String>,
    /// Keys the parser did not know and read past, one message each, for `day lint`
    /// (`store-unknown-key`): a typo is a finding, not a listing nobody can stage.
    pub problems: Vec<String>,
}

impl Storefront {
    /// Read a parsed `storefront.toml` or `storefront.yaml`, as the one tree both formats produce
    /// ([`parse_document`]), against `src`.
    pub fn parse(doc: &serde_json::Value, src: &Source) -> Result<Storefront, String> {
        let mut out = Storefront::default();
        let Some(top) = doc.as_object() else {
            return Ok(out);
        };
        for key in top.keys() {
            if key != "storefront" {
                out.legacy_keys.push(key.clone());
            }
        }
        let Some(root) = top.get("storefront") else {
            return Ok(out);
        };
        let root = root.as_object().ok_or(
            "storefront: a table, `[storefront.submission-info]`, `[storefront.<target>]`",
        )?;
        for (key, value) in root {
            match key.as_str() {
                "submission-info" => {
                    out.info = SubmissionInfo::parse(
                        value,
                        "storefront.submission-info",
                        &mut out.problems,
                    )?
                }
                "metadata" => {
                    out.metadata
                        .parse(value, "storefront.metadata", src, &mut out.problems)?
                }
                "screenshots" => {
                    return Err("storefront.screenshots: screenshots are declared per target, `[storefront.<target>.screenshots]`".into());
                }
                target => {
                    let at = format!("storefront.{target}");
                    let table = value
                        .as_object()
                        .ok_or_else(|| format!("{at}: a table, `[{at}]`"))?;
                    let mut te = TargetEntry::default();
                    for (key, value) in table {
                        match key.as_str() {
                            "submission-info" => {
                                te.info = SubmissionInfo::parse(
                                    value,
                                    &format!("{at}.submission-info"),
                                    &mut out.problems,
                                )?
                            }
                            "metadata" => te.metadata.parse(
                                value,
                                &format!("{at}.metadata"),
                                src,
                                &mut out.problems,
                            )?,
                            "screenshots" => {
                                te.screenshots =
                                    Screenshots::parse(value, &format!("{at}.screenshots"))?
                            }
                            store => {
                                let at = format!("{at}.{store}");
                                let table = value.as_object().ok_or_else(|| {
                                    format!("{at}: a store is a table with `submission-info`, `metadata` and `screenshots`, `[{at}.screenshots]`")
                                })?;
                                let mut se = StoreEntry::default();
                                for (key, value) in table {
                                    match key.as_str() {
                                        "submission-info" => {
                                            se.info = SubmissionInfo::parse(
                                                value,
                                                &format!("{at}.submission-info"),
                                                &mut out.problems,
                                            )?
                                        }
                                        "metadata" => se.metadata.parse(
                                            value,
                                            &format!("{at}.metadata"),
                                            src,
                                            &mut out.problems,
                                        )?,
                                        "screenshots" => {
                                            se.screenshots = Screenshots::parse(
                                                value,
                                                &format!("{at}.screenshots"),
                                            )?
                                        }
                                        other => out.problems.push(format!(
                                            "[{at}] {other}: unknown key; a store takes \
                                             `submission-info`, `metadata` and `screenshots`"
                                        )),
                                    }
                                }
                                te.stores.insert(store.to_string(), se);
                            }
                        }
                    }
                    out.targets.insert(target.to_string(), te);
                }
            }
        }
        Ok(out)
    }

    /// Read a locale file (`store/app.<tag>.toml`) into this storefront: the same tree, with
    /// only `metadata` tables in it, each the text of the locale `src.sidecar` names, at the
    /// shared level or under a target or store the main file declares.
    pub fn merge_sidecar(&mut self, doc: &serde_json::Value, src: &Source) -> Result<(), String> {
        let tag = src
            .sidecar
            .ok_or("a locale file is read with the locale it carries")?;
        let Some(top) = doc.as_object() else {
            return Ok(());
        };
        let only = |at: String| -> String {
            format!(
                "{}: {at}: a locale file carries {tag}'s text alone: `metadata` tables at the \
                 shared level, under a target, or under a target's store",
                src.file
            )
        };
        for key in top.keys() {
            if key != "storefront" {
                return Err(only(key.clone()));
            }
        }
        let Some(root) = top.get("storefront").and_then(|v| v.as_object()) else {
            return Ok(());
        };
        for (key, value) in root {
            match key.as_str() {
                "metadata" => {
                    let mut problems = Vec::new();
                    self.metadata
                        .parse(value, "storefront.metadata", src, &mut problems)?;
                    self.problems.extend(problems);
                }
                "submission-info" | "screenshots" => {
                    return Err(only(format!("storefront.{key}")));
                }
                target => {
                    let at = format!("storefront.{target}");
                    let Some(te) = self.targets.get_mut(target) else {
                        return Err(format!(
                            "{}: {at}: not a target the main storefront file declares",
                            src.file
                        ));
                    };
                    let table = value
                        .as_object()
                        .ok_or_else(|| format!("{}: {at}: a table", src.file))?;
                    for (key, value) in table {
                        match key.as_str() {
                            "metadata" => {
                                let mut problems = Vec::new();
                                te.metadata.parse(
                                    value,
                                    &format!("{at}.metadata"),
                                    src,
                                    &mut problems,
                                )?;
                                self.problems.extend(problems);
                            }
                            "submission-info" | "screenshots" => {
                                return Err(only(format!("{at}.{key}")));
                            }
                            store => {
                                let at = format!("{at}.{store}");
                                let Some(se) = te.stores.get_mut(store) else {
                                    return Err(format!(
                                        "{}: {at}: not a store the main storefront file declares",
                                        src.file
                                    ));
                                };
                                let table = value
                                    .as_object()
                                    .ok_or_else(|| format!("{}: {at}: a table", src.file))?;
                                for (key, value) in table {
                                    match key.as_str() {
                                        "metadata" => {
                                            let mut problems = Vec::new();
                                            se.metadata.parse(
                                                value,
                                                &format!("{at}.metadata"),
                                                src,
                                                &mut problems,
                                            )?;
                                            self.problems.extend(problems);
                                        }
                                        other => return Err(only(format!("{at}.{other}"))),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// The submission metadata `store` on `target` publishes with: the store's over the
    /// target's over the shared, key by key.
    pub fn submission(&self, target: &str, store: &str) -> SubmissionInfo {
        let Some(te) = self.targets.get(target) else {
            return self.info.clone();
        };
        let at_target = te.info.over(&self.info);
        match te.stores.get(store) {
            Some(se) => se.info.over(&at_target),
            None => at_target,
        }
    }

    /// The metadata tables that apply to `store` on `target`, nearest first: the store's, the
    /// target's, the shared.
    fn levels(&self, target: &str, store: &str) -> Vec<&Metadata> {
        let mut levels = Vec::new();
        if let Some(te) = self.targets.get(target) {
            if let Some(se) = te.stores.get(store) {
                levels.push(&se.metadata);
            }
            levels.push(&te.metadata);
        }
        levels.push(&self.metadata);
        levels
    }

    /// The listing text `store` on `target` publishes in `locale`, field by field: the
    /// locale's own tables at every level, nearest first, then the default locale's the same
    /// way. A locale's wording outranks a store's, so an app that specializes the App Store's
    /// subtitle in the default locale alone still shows French users French.
    pub fn metadata(&self, target: &str, store: &str, locale: &str) -> BTreeMap<Field, Text> {
        let levels = self.levels(target, store);
        let mut out: BTreeMap<Field, Text> = BTreeMap::new();
        for m in &levels {
            if let Some(table) = m.table(locale) {
                for (f, text) in table {
                    out.entry(*f).or_insert_with(|| text.clone());
                }
            }
        }
        for m in &levels {
            for (f, text) in &m.base {
                out.entry(*f).or_insert_with(|| text.clone());
            }
        }
        out
    }

    /// Every locale some metadata table specializes, at any level.
    pub fn metadata_locales(&self) -> BTreeSet<String> {
        let mut out: BTreeSet<String> = self.metadata.locales.keys().cloned().collect();
        for te in self.targets.values() {
            out.extend(te.metadata.locales.keys().cloned());
            for se in te.stores.values() {
                out.extend(se.metadata.locales.keys().cloned());
            }
        }
        out
    }

    /// Whether any listing text is declared at all.
    pub fn has_metadata(&self) -> bool {
        !self.metadata.is_empty()
            || self.targets.values().any(|te| {
                !te.metadata.is_empty() || te.stores.values().any(|se| !se.metadata.is_empty())
            })
    }

    /// Every declared text, with the level and locale it was declared at.
    pub fn texts(&self) -> Vec<Declared<'_>> {
        let mut out = Vec::new();
        fn collect<'a>(
            out: &mut Vec<Declared<'a>>,
            target: Option<&'a str>,
            store: Option<&'a str>,
            m: &'a Metadata,
        ) {
            for (field, text) in &m.base {
                out.push(Declared {
                    target,
                    store,
                    locale: None,
                    field: *field,
                    text,
                });
            }
            for (locale, fields) in &m.locales {
                for (field, text) in fields {
                    out.push(Declared {
                        target,
                        store,
                        locale: Some(locale.as_str()),
                        field: *field,
                        text,
                    });
                }
            }
        }
        collect(&mut out, None, None, &self.metadata);
        for (target, te) in &self.targets {
            collect(&mut out, Some(target.as_str()), None, &te.metadata);
            for (store, se) in &te.stores {
                collect(
                    &mut out,
                    Some(target.as_str()),
                    Some(store.as_str()),
                    &se.metadata,
                );
            }
        }
        out
    }

    /// Whether `target` declares any screenshot list, its own or a store's. A target table
    /// that holds submission info or text alone (the scaffold's commented store stub, say)
    /// declares nothing about screenshots, and a listing built from it would be empty lists
    /// that hide every capture the walkthrough took.
    pub fn declares_screenshots(&self, target: &str) -> bool {
        self.targets.get(target).is_some_and(|te| {
            !te.screenshots.is_empty() || te.stores.values().any(|se| !se.screenshots.is_empty())
        })
    }

    /// The list a target's website page shows on device `kind` in `locale`: the target's
    /// `screenshots`, the locale's lists first (kind, then `default`), then the general ones.
    pub fn website(&self, target: &str, kind: &str, locale: &str) -> Vec<ListingShot> {
        self.targets
            .get(target)
            .and_then(|te| te.screenshots.get(kind, locale))
            .cloned()
            .unwrap_or_default()
    }

    /// What `store`'s listing shows on device `kind` in `locale`: the locale's lists first,
    /// the store's then the target's, then the general lists the same way (kind before
    /// `default` at each step). The same order the listing text resolves in: a locale's
    /// choices outrank a store's, so a French listing leads with what French was given.
    pub fn store_kind(
        &self,
        target: &str,
        store: &str,
        kind: &str,
        locale: &str,
    ) -> Vec<ListingShot> {
        let Some(te) = self.targets.get(target) else {
            return Vec::new();
        };
        let se = te.stores.get(store);
        se.and_then(|se| se.screenshots.get_localized(kind, locale))
            .or_else(|| te.screenshots.get_localized(kind, locale))
            .or_else(|| se.and_then(|se| se.screenshots.all.get(kind)))
            .or_else(|| te.screenshots.all.get(kind))
            .cloned()
            .unwrap_or_default()
    }

    /// The stores a target declares, in name order.
    pub fn stores(&self, target: &str) -> Vec<String> {
        self.targets
            .get(target)
            .map(|te| te.stores.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// The device kinds a target's own or a store's screenshots name explicitly.
    pub fn declared_kinds(&self, target: &str, store: &str) -> Vec<String> {
        let Some(te) = self.targets.get(target) else {
            return Vec::new();
        };
        let mut kinds: BTreeSet<String> = te.screenshots.kinds().cloned().collect();
        if let Some(se) = te.stores.get(store) {
            kinds.extend(se.screenshots.kinds().cloned());
        }
        kinds.into_iter().collect()
    }

    /// The locales a target's own or a store's screenshots specialize.
    pub fn declared_locales(&self, target: &str, store: &str) -> Vec<String> {
        let Some(te) = self.targets.get(target) else {
            return Vec::new();
        };
        let mut locales: BTreeSet<String> = te.screenshots.locales.keys().cloned().collect();
        if let Some(se) = te.stores.get(store) {
            locales.extend(se.screenshots.locales.keys().cloned());
        }
        locales.into_iter().collect()
    }

    /// Whether any screenshots are declared at all.
    pub fn has_screenshots(&self) -> bool {
        self.targets.values().any(|te| {
            !te.screenshots.is_empty() || te.stores.values().any(|se| !se.screenshots.is_empty())
        })
    }

    /// Every shot name the declaration mentions, for lint to hold against the dayscripts.
    pub fn shot_names(&self) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for te in self.targets.values() {
            for list in te
                .screenshots
                .lists()
                .chain(te.stores.values().flat_map(|se| se.screenshots.lists()))
            {
                names.extend(list.iter().map(|s| s.shot.clone()));
            }
        }
        names
    }
}

/// A list of shots: `[ "home", { name = "controls", theme = "dark" } ]`.
fn parse_shots(value: &serde_json::Value, at: &str) -> Result<Vec<ListingShot>, String> {
    let items = value.as_array().ok_or_else(|| {
        format!("{at}: a list of screenshots, `[ \"home\", {{ name = \"controls\", theme = \"dark\" }} ]`")
    })?;
    let mut out = Vec::new();
    for item in items {
        let shot = match item {
            serde_json::Value::String(name) => ListingShot {
                shot: name.trim().to_string(),
                theme: "light".to_string(),
            },
            serde_json::Value::Object(t) => {
                let name = match t.get("name") {
                    Some(serde_json::Value::String(s)) if !s.trim().is_empty() => s.trim(),
                    _ => {
                        return Err(format!(
                            "{at}: each screenshot names a `screenshot:` step, `{{ name = \"home\" }}`"
                        ));
                    }
                };
                let theme = match t.get("theme") {
                    None | Some(serde_json::Value::Null) => "light".to_string(),
                    Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {
                        s.trim().to_string()
                    }
                    Some(_) => {
                        return Err(format!(
                            "{at}: `theme` is a string, such as \"light\" or \"dark\""
                        ));
                    }
                };
                if let Some(extra) = t.keys().find(|k| *k != "name" && *k != "theme") {
                    return Err(format!(
                        "{at}: unknown key {extra:?}; a screenshot takes `name` and `theme`"
                    ));
                }
                ListingShot {
                    shot: name.to_string(),
                    theme,
                }
            }
            _ => {
                return Err(format!(
                    "{at}: a screenshot is a name or `{{ name = ..., theme = ... }}`"
                ));
            }
        };
        if shot.shot.is_empty() {
            return Err(format!("{at}: an empty screenshot name"));
        }
        out.push(shot);
    }
    Ok(out)
}

/// The names the storefront file may have, in the order one is looked for. `storefront.toml`
/// is what `day store init` writes; YAML is accepted so a project can keep its metadata in
/// either, and the names from before 2026-09-25 (`app.*`) still read, with a lint finding that
/// says to rename.
pub const STOREFRONT_FILES: [&str; 6] = [
    "storefront.toml",
    "storefront.yaml",
    "storefront.yml",
    "app.toml",
    "app.yaml",
    "app.yml",
];

/// The stem a storefront file was written under: `storefront`, or the older `app`.
fn storefront_stem(name: &str) -> &str {
    name.split('.').next().unwrap_or("storefront")
}

/// Whether a storefront or locale file carries the name from before the rename.
pub fn is_legacy_name(name: &str) -> bool {
    storefront_stem(name) == "app"
}

/// Parse a storefront document in either format into the one tree [`Storefront::parse`] reads.
/// TOML goes through the `toml` crate, YAML through `serde_norway`; both land in
/// `serde_json::Value`, so the declaration means the same whichever file carried it.
pub fn parse_document(text: &str, yaml: bool) -> Result<serde_json::Value, String> {
    if yaml {
        serde_norway::from_str::<serde_json::Value>(text).map_err(|e| e.to_string())
    } else {
        let doc: toml::Value = toml::from_str(text).map_err(|e| e.to_string())?;
        serde_json::to_value(doc).map_err(|e| e.to_string())
    }
}

/// Whether a storefront file name is the YAML form.
fn is_yaml(name: &str) -> bool {
    name.ends_with(".yaml") || name.ends_with(".yml")
}

/// The locale a locale file name carries: `storefront.fr.toml` (or the older `app.fr.toml`)
/// → `fr`. The main file and anything else is `None`.
fn sidecar_locale(name: &str) -> Option<&str> {
    let stem = name
        .strip_suffix(".toml")
        .or_else(|| name.strip_suffix(".yaml"))
        .or_else(|| name.strip_suffix(".yml"))?;
    let tag = stem
        .strip_prefix("storefront.")
        .or_else(|| stem.strip_prefix("app."))?;
    (!tag.is_empty() && !tag.contains('.')).then_some(tag)
}

/// What a storefront directory holds: the main file's name, every file read (the main file and
/// the locale files, in name order), and the declaration they make together.
#[derive(Debug)]
pub struct Loaded {
    pub file: Option<String>,
    pub files: Vec<String>,
    pub storefront: Storefront,
}

/// The storefront in `dir`: `storefront.toml` or `storefront.yaml` (two of them, or one beside
/// an older `app.toml`, is an error, not a preference: a project must not carry two copies
/// that drift apart), plus every locale file `storefront.<tag>.toml` / `.yaml` beside it. `-ref` paths are read relative to `project_root`;
/// `default_locale` is the locale the base tables are the text of.
pub fn load_storefront(
    project_root: &Path,
    dir: &Path,
    default_locale: &str,
) -> Result<Loaded, String> {
    let read_ref = |rel: &str| -> Result<String, String> {
        let path = project_root.join(rel);
        std::fs::read_to_string(&path).map_err(|e| e.to_string())
    };
    let present: Vec<&str> = STOREFRONT_FILES
        .iter()
        .copied()
        .filter(|name| dir.join(name).is_file())
        .collect();
    if present.len() > 1 {
        return Err(format!(
            "{}: {} both declare the storefront; keep one",
            dir.display(),
            present.join(" and ")
        ));
    }
    let mut loaded = Loaded {
        file: None,
        files: Vec::new(),
        storefront: Storefront::default(),
    };
    if let Some(name) = present.first() {
        let path = dir.join(name);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let doc =
            parse_document(&text, is_yaml(name)).map_err(|e| format!("{}: {e}", path.display()))?;
        let src = Source {
            file: name,
            sidecar: None,
            default_locale,
            read_ref: &read_ref,
        };
        loaded.storefront =
            Storefront::parse(&doc, &src).map_err(|e| format!("{}: {e}", dir.display()))?;
        loaded.file = Some(name.to_string());
        loaded.files.push(name.to_string());
    }
    // The locale files, in name order, so a conflict between two is reported the same way
    // on every machine.
    let mut sidecars: Vec<String> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .filter(|e| e.path().is_file())
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|n| sidecar_locale(n).is_some())
            .collect(),
        Err(_) => Vec::new(),
    };
    sidecars.sort();
    for name in &sidecars {
        let tag = sidecar_locale(name).unwrap_or_default();
        if let Some(twin) = sidecars
            .iter()
            .find(|n| *n != name && sidecar_locale(n) == Some(tag))
        {
            return Err(format!(
                "{}: {name} and {twin} both carry {tag}; keep one",
                dir.display()
            ));
        }
        let path = dir.join(name);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let doc =
            parse_document(&text, is_yaml(name)).map_err(|e| format!("{}: {e}", path.display()))?;
        let src = Source {
            file: name,
            sidecar: Some(tag),
            default_locale,
            read_ref: &read_ref,
        };
        loaded
            .storefront
            .merge_sidecar(&doc, &src)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        loaded.files.push(name.clone());
    }
    Ok(loaded)
}

/// The device kind a capture belongs to, for the listing: its device slug, or the kind a
/// profile-less capture on the platform is (the store's `default-kind` in the rules the CLI
/// ships: `iphone`, `phone`), else `default`.
pub fn capture_kind(target: &crate::targets::Target, device: Option<&str>) -> String {
    StoreRules::builtin().capture_kind(target.name, device)
}

/// A listing: the storefront as read, and the files it came from.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    pub app: Storefront,
    /// The main file (`storefront.toml` or `storefront.yaml`), for messages; `None` when the directory has
    /// none.
    pub app_file: Option<String>,
    /// Every file read, main and locale files, in the order they were read.
    pub files: Vec<String>,
    /// The app's default locale: the base tables' text.
    pub default_locale: String,
}

impl Listing {
    /// Whether the listing has no text at all.
    pub fn is_empty(&self) -> bool {
        !self.app.has_metadata()
    }

    /// The locales the listing carries: the default, then every locale some table
    /// specializes.
    pub fn locales(&self) -> Vec<String> {
        let mut out = vec![self.default_locale.clone()];
        out.extend(
            self.app
                .metadata_locales()
                .into_iter()
                .filter(|l| *l != self.default_locale),
        );
        out
    }
}

/// The project's `store/` directory — the active flavor's when it declares one, since a flavor
/// is usually its own store record with its own name, description and screenshots
/// (DESIGN.md §16.6).
pub fn dir(project: &Project) -> PathBuf {
    crate::flavor::store_overlay(&project.root).unwrap_or_else(|| project.root.join("store"))
}

/// Read `store/`. A project without one gets an empty listing rather than an error: store metadata
/// is only required of an app that actually ships to a store.
pub fn read(project: &Project) -> Result<Listing, String> {
    let root = dir(project);
    let default_locale = default_locale(&app_locales(project)).unwrap_or_else(|| "en".into());
    let mut listing = Listing {
        default_locale,
        ..Listing::default()
    };
    if !root.is_dir() {
        return Ok(listing);
    }
    let loaded = load_storefront(&project.root, &root, &listing.default_locale)?;
    listing.app = loaded.storefront;
    listing.app_file = loaded.file;
    listing.files = loaded.files;
    Ok(listing)
}

/// The locales a project's listing carries, from its root alone (no `Project`): `None` when
/// there is no `store/` or it cannot be read, for `day localize`'s survey.
pub fn listing_locales(project_root: &Path) -> Option<Vec<String>> {
    let dir = project_root.join("store");
    if !dir.is_dir() {
        return None;
    }
    let locales = app_locales_in(project_root);
    let default = default_locale(&locales).unwrap_or_else(|| "en".into());
    let loaded = load_storefront(project_root, &dir, &default).ok()?;
    let listing = Listing {
        app: loaded.storefront,
        app_file: loaded.file,
        files: loaded.files,
        default_locale: default,
    };
    if listing.is_empty() {
        return Some(Vec::new());
    }
    Some(listing.locales())
}

/// Locales the app itself ships (`resource/locales/<tag>/`), which the listing must match.
pub fn app_locales(project: &Project) -> Vec<String> {
    app_locales_in(&project.root)
}

/// [`app_locales`] from a project root.
fn app_locales_in(root: &Path) -> Vec<String> {
    let dir = root.join("resource/locales");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        // A pseudolocale (`-XA`) is a development aid, never a store listing.
        .filter(|t| !t.ends_with("-XA"))
        .collect();
    out.sort();
    out
}

/// The default locale: `en` when present, else the first, the same rule day-build applies to
/// `res::locales::DEFAULT`, so the listing's primary language matches the app's.
pub fn default_locale(locales: &[String]) -> Option<String> {
    if locales.iter().any(|l| l == "en") {
        return Some("en".into());
    }
    locales.first().cloned()
}

// ---------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------

/// Write the fastlane tree for a target under `out`, returning the files written.
///
/// The store the rules name for the target decides the layout: `deliver` (App Store Connect)
/// gets `metadata/<store-locale>/…`, `supply` (Google Play) `metadata/android/<store-locale>/…`
/// plus `changelogs/<versionCode>.txt`. Both get an `Appfile` and a `Fastfile` with the lanes a
/// release needs: a dry run that validates against the store without publishing, the upload
/// itself, and the submission.
pub fn stage(
    project: &Project,
    target: &'static crate::targets::Target,
    listing: &Listing,
    out: &Path,
    screenshots: Option<&ScreenshotSource>,
    rules: &StoreRules,
) -> Result<Vec<PathBuf>, String> {
    let Some((store_key, rule)) = rules.store_for(target.name) else {
        return Err(format!(
            "{}: no store the rules stage publishes this target",
            target.name
        ));
    };
    let deliver = rule.layout.as_deref() == Some("deliver");
    let _ = std::fs::remove_dir_all(out);
    let mut written = Vec::new();
    // The screenshots first, so a listing whose index cannot be read fails before anything is
    // written: a tree with the copy and no images would upload a listing with none.
    let with_screenshots = match screenshots {
        Some(source) => {
            let placed = stage_screenshots(source, rules, target, out)?;
            written.extend(placed.iter().cloned());
            !placed.is_empty()
        }
        None => false,
    };
    let mut write = |rel: &str, body: &str| -> Result<(), String> {
        let path = out.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(path);
        Ok(())
    };

    // One set of files per locale the listing carries, each field resolved for this store:
    // the locale's tables at every level, then the default locale's (`Storefront::metadata`),
    // under the store's own spelling of the locale and its own name for the field.
    for tag in listing.locales() {
        let Some(loc) = rule.locale(&tag) else {
            continue; // reported by lint; generation skips rather than inventing a locale
        };
        let fields = listing.app.metadata(target.name, store_key, &tag);
        for (field, text) in &fields {
            let Some(fr) = rule.field(*field) else {
                continue;
            };
            let rel = if deliver {
                format!("fastlane/metadata/{loc}/{}", fr.file)
            } else if fr.file == "changelog" {
                // supply keys the changelog by versionCode, which is Day.toml's `[app] build`.
                format!(
                    "fastlane/metadata/android/{loc}/changelogs/{}.txt",
                    project.manifest.app.build
                )
            } else {
                format!("fastlane/metadata/android/{loc}/{}", fr.file)
            };
            write(&rel, &format!("{}\n", text.value.trim_end()))?;
        }
    }

    // The id this target publishes under, not the app's top-level one: an `[app.android]` id is
    // how a bundle id with a hyphen becomes a legal Play package name, and `supply` refuses the
    // unresolved one ("Invalid package name"). A listing that names its own `bundle-id` still
    // wins, since it describes a record that already exists.
    let resolved = project.manifest.resolve(target.name);
    // The submission info this store's record takes: the store's over the target's over the
    // shared (`Storefront::submission`).
    let info = listing.app.submission(target.name, store_key);
    let id = info
        .bundle_id
        .clone()
        .unwrap_or_else(|| resolved.id.clone());
    if deliver {
        if let Some(c) = &info.copyright {
            write("fastlane/metadata/copyright.txt", &format!("{c}\n"))?;
        }
        if let Some(c) = &info.apple_category {
            write("fastlane/metadata/primary_category.txt", &format!("{c}\n"))?;
        }
        // The review contact only as a complete record (`SubmissionInfo::contact_first_name`).
        if let (Some(first), Some(last), Some(phone), Some(email)) = (
            &info.contact_first_name,
            &info.contact_last_name,
            &info.contact_phone,
            &info.contact_email,
        ) {
            let ri = "fastlane/metadata/review_information";
            write(&format!("{ri}/first_name.txt"), &format!("{first}\n"))?;
            write(&format!("{ri}/last_name.txt"), &format!("{last}\n"))?;
            write(&format!("{ri}/phone_number.txt"), &format!("{phone}\n"))?;
            write(&format!("{ri}/email_address.txt"), &format!("{email}\n"))?;
            if let Some(n) = &info.review_notes {
                write(&format!("{ri}/notes.txt"), &format!("{n}\n"))?;
            }
        }
        write("fastlane/Appfile", &apple_appfile(&id, &info))?;
        write(
            "fastlane/Fastfile",
            &apple_fastfile(project, with_screenshots),
        )?;
    } else {
        write("fastlane/Appfile", &play_appfile(&id))?;
        write(
            "fastlane/Fastfile",
            &play_fastfile(project, with_screenshots),
        )?;
    }
    // The lanes read their one policy switch from the env file, so the Fastfile stays the
    // same text for every app and the listing's choice travels with the staged tree.
    let mut env = FASTLANE_ENV.to_string();
    if deliver && info.manual_release() {
        env.push_str(
            "# apple-release = \"manual\" in the listing: an approved version waits for the Release\n\
             # button in App Store Connect instead of going live on its own.\n\
             DAY_ASC_MANUAL_RELEASE=1\n",
        );
    }
    write("fastlane/.env.default", &env)?;
    written.sort();
    Ok(written)
}

/// Where `day store stage --screenshots` reads the gallery index (§14.7) from: the published
/// `gallery.json` of the app's site, or a local one beside its capture tree.
#[derive(Debug, Clone)]
pub enum ScreenshotSource {
    Url(String),
    File(PathBuf),
}

impl ScreenshotSource {
    pub fn parse(text: &str) -> ScreenshotSource {
        if text.starts_with("http://") || text.starts_with("https://") {
            ScreenshotSource::Url(text.to_string())
        } else {
            ScreenshotSource::File(PathBuf::from(text))
        }
    }

    fn index(&self) -> Result<serde_json::Value, String> {
        match self {
            ScreenshotSource::Url(url) => {
                let text = ureq::get(url)
                    .call()
                    .map_err(|e| format!("{url}: {e}"))?
                    .into_body()
                    .with_config()
                    // An index of a few thousand captures runs to a few megabytes.
                    .limit(64 * 1024 * 1024)
                    .read_to_string()
                    .map_err(|e| format!("{url}: {e}"))?;
                serde_json::from_str(&text).map_err(|e| format!("{url}: {e}"))
            }
            ScreenshotSource::File(path) => {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
            }
        }
    }

    /// One capture's bytes: over HTTP from the entry's `url`, or from the file its `path`
    /// names. A path is `gallery/<target>/…`, relative to the site root a published index sits
    /// under (`<root>/gallery/gallery.json`); an index `day screenshot index` wrote sits in the
    /// capture tree itself (`<tree>/gallery.json` beside `<tree>/<target>/…`), so the same path
    /// is tried there without its first segment.
    fn fetch(&self, entry: &serde_json::Value) -> Result<Vec<u8>, String> {
        match self {
            ScreenshotSource::Url(_) => {
                let url = entry["url"].as_str().ok_or("an index entry with no url")?;
                ureq::get(url)
                    .call()
                    .map_err(|e| format!("{url}: {e}"))?
                    .into_body()
                    .with_config()
                    .limit(64 * 1024 * 1024)
                    .read_to_vec()
                    .map_err(|e| format!("{url}: {e}"))
            }
            ScreenshotSource::File(index) => {
                let rel = entry["path"]
                    .as_str()
                    .ok_or("an index entry with no path")?;
                let dir = index.parent().unwrap_or(Path::new("."));
                let site = dir.parent().unwrap_or(Path::new(".")).join(rel);
                let tree = dir.join(rel.split_once('/').map(|(_, r)| r).unwrap_or(rel));
                let path = if site.exists() { site } else { tree };
                std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
            }
        }
    }
}

/// A screenshot the listing takes, chosen from the index.
struct StoreShot {
    position: u32,
    device: String,
    shot: String,
    locale: String,
    /// The size the store receives: the capture's, times `scale`.
    width: u32,
    height: u32,
    /// The whole factor the capture is scaled up by before placing (`ShotRule::scale_for`); 1
    /// places it as captured.
    scale: u32,
    entry: serde_json::Value,
}

/// The captures `target`'s store listing shows, from the index's `listings` (what `day
/// screenshot index` resolved from `store/storefront.toml`): per device kind and locale, the declared
/// shots in order, in the declared theme (a capture with no theme is taken).
fn listing_shots(
    index: &serde_json::Value,
    target: &'static crate::targets::Target,
    rules: &StoreRules,
) -> Result<Vec<StoreShot>, String> {
    let entries = index["screenshots"]
        .as_array()
        .ok_or("the gallery index has no `screenshots` list")?;
    let mut chosen: Vec<StoreShot> = Vec::new();
    let Some((store, rule)) = rules.store_for(target.name) else {
        return Ok(chosen);
    };
    let rule_kinds = rule.kinds();
    let Some(kinds) = index["listings"][target.name]["stores"][store].as_object() else {
        return Ok(chosen);
    };
    for (kind, by_locale) in kinds {
        let Some(by_locale) = by_locale.as_object() else {
            continue;
        };
        for (locale, list) in by_locale {
            let Some(list) = list.as_array() else {
                continue;
            };
            for (i, wanted) in list.iter().enumerate() {
                let (Some(shot), Some(theme)) = (wanted["shot"].as_str(), wanted["theme"].as_str())
                else {
                    continue;
                };
                for e in entries {
                    if e["platform"].as_str() != Some(target.name)
                        || e["shot"].as_str() != Some(shot)
                        || capture_kind(target, e["device"].as_str()) != *kind
                        || e["locale"].as_str() != Some(locale.as_str())
                    {
                        continue;
                    }
                    if let Some(t) = e["theme"].as_str()
                        && t != theme
                    {
                        continue;
                    }
                    let (w, h) = (
                        e["width"].as_u64().unwrap_or(0) as u32,
                        e["height"].as_u64().unwrap_or(0) as u32,
                    );
                    let scale = rule_for(&rule_kinds, kind)
                        .map(|(_, r)| r.scale_for(w, h))
                        .unwrap_or(1);
                    chosen.push(StoreShot {
                        position: i as u32 + 1,
                        device: kind.clone(),
                        shot: shot.to_string(),
                        locale: locale.clone(),
                        width: w * scale,
                        height: h * scale,
                        scale,
                        entry: e.clone(),
                    });
                }
            }
        }
    }
    chosen.sort_by(|a, b| {
        (a.locale.as_str(), a.device.as_str(), a.position).cmp(&(
            b.locale.as_str(),
            b.device.as_str(),
            b.position,
        ))
    });
    Ok(chosen)
}

/// One field of a store's listing: the file fastlane reads it from, and the store's limit in
/// characters.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct FieldRule {
    pub file: String,
    pub limit: usize,
}

/// What a store takes for one device kind of screenshot: one table of the rules file.
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ShotRule {
    /// How the kind is called to a reader (`iPhone`, `Handset`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// supply's folder for the kind's images (`phoneScreenshots`); deliver reads the device
    /// from each image's size and takes none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// Whether every locale needs at least one: App Store Connect refuses a version whose
    /// localization has none, and Play refuses a listing without phone screenshots.
    #[serde(default)]
    pub required: bool,
    /// The store's ceiling per locale; zero is no ceiling.
    #[serde(default)]
    pub max: usize,
    /// Apple: the exact sizes it takes for the kind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sizes: Vec<(u32, u32)>,
    /// Google: the short side at least.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_side: Option<u32>,
    /// Google: the long side at most.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_side: Option<u32>,
    /// Google: the long side at most this many times the short.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ratio: Option<f64>,
    /// Whether a capture under `min-side` is scaled up to clear it (by the smallest whole
    /// factor) rather than refused: a headless CI tablet is captured halved, and the store
    /// checks the pixels' count, not their provenance.
    #[serde(default)]
    pub upscale: bool,
}

impl ShotRule {
    /// The whole factor a `w`×`h` capture is scaled by to clear `min-side`, or 1: none when the
    /// rule does not allow scaling, the capture already clears the floor, or the scaled long
    /// side would pass `max-side`.
    pub fn scale_for(&self, w: u32, h: u32) -> u32 {
        let (lo, hi) = (w.min(h), w.max(h));
        let Some(min) = self.min_side.filter(|_| self.upscale) else {
            return 1;
        };
        if lo == 0 || lo >= min {
            return 1;
        }
        let k = min.div_ceil(lo);
        match self.max_side {
            Some(max) if hi * k > max => 1,
            _ => k,
        }
    }
}

/// One store's rules: what it is called, which targets publish to it, the fastlane layout its
/// listing takes (a store without one is known by name and not staged), its fields with their
/// names and limits, how it spells each locale, and what it takes for a screenshot per device
/// kind. One table of `store-rules.toml`.
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct StoreRule {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub targets: Vec<String>,
    /// `deliver` (App Store Connect) or `supply` (Google Play).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    /// The device kind a capture from a profile-less run counts as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_kind: Option<String>,
    /// Fields the store's record cannot do without.
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub fields: BTreeMap<String, FieldRule>,
    /// Day locale tag → the store's spelling.
    #[serde(default)]
    pub locales: BTreeMap<String, String>,
    #[serde(default)]
    pub screenshots: BTreeMap<String, ShotRule>,
}

impl StoreRule {
    /// The store's rule for a listing field, when it has the field at all.
    pub fn field(&self, f: Field) -> Option<&FieldRule> {
        self.fields.get(f.key())
    }

    /// The store's spelling of a Day locale tag, when it knows the locale.
    pub fn locale(&self, tag: &str) -> Option<&str> {
        self.locales.get(tag).map(String::as_str)
    }

    /// Whether the CLI stages a listing for this store.
    pub fn stages(&self) -> bool {
        self.layout.is_some()
    }

    /// The store's name in a sentence: its label, else its key.
    pub fn name<'a>(&'a self, key: &'a str) -> &'a str {
        if self.label.is_empty() {
            key
        } else {
            &self.label
        }
    }

    pub(crate) fn kinds(&self) -> Vec<(&str, &ShotRule)> {
        self.screenshots
            .iter()
            .map(|(k, r)| (k.as_str(), r))
            .collect()
    }
}

/// The stores' rules (`store-rules.toml`, docs/store.md "The stores' rules"), keyed by store.
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct StoreRules(pub BTreeMap<String, StoreRule>);

/// The rules the CLI ships: the stores as they were last measured.
pub const DEFAULT_RULES: &str = include_str!("store-rules.toml");

impl StoreRules {
    pub fn parse(text: &str) -> Result<StoreRules, String> {
        let rules: StoreRules = toml::from_str(text).map_err(|e| e.to_string())?;
        for (key, rule) in &rules.0 {
            if let Some(layout) = &rule.layout
                && layout != "deliver"
                && layout != "supply"
            {
                return Err(format!(
                    "[{key}] layout = {layout:?}: the layouts are `deliver` and `supply`"
                ));
            }
            for field in rule.fields.keys() {
                if Field::from_key(field).is_none() {
                    return Err(format!(
                        "[{key}.fields] {field}: not a listing field; the fields are {}",
                        FIELDS
                            .iter()
                            .map(|f| f.key())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            for field in &rule.required {
                if !rule.fields.contains_key(field) {
                    return Err(format!(
                        "[{key}] required names {field:?}, which [{key}.fields] does not have"
                    ));
                }
            }
        }
        Ok(rules)
    }

    /// The rules the CLI ships, parsed once.
    pub fn builtin() -> &'static StoreRules {
        static BUILTIN: std::sync::OnceLock<StoreRules> = std::sync::OnceLock::new();
        // The embedded file is held to `parse` by a unit test, so a failure here is a build of
        // the CLI that could not have passed its own tests.
        BUILTIN.get_or_init(|| {
            StoreRules::parse(DEFAULT_RULES).unwrap_or_else(|e| panic!("store-rules.toml: {e}"))
        })
    }

    /// The rules in force: `explicit` (the `--rules` flag), else the file DAY_STORE_RULES names
    /// (how a CI workflow hands the CLI its own copy), else the project's `store/rules.toml`,
    /// else the CLI's own.
    pub fn load(project: &Project, explicit: Option<&Path>) -> Result<StoreRules, String> {
        let from_env = std::env::var_os("DAY_STORE_RULES")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        let own = project.root.join("store/rules.toml");
        let path = match explicit.map(Path::to_path_buf).or(from_env) {
            Some(p) => Some(p),
            None => own.is_file().then_some(own),
        };
        match path {
            Some(p) => {
                let text =
                    std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                StoreRules::parse(&text).map_err(|e| format!("{}: {e}", p.display()))
            }
            None => Ok(StoreRules::builtin().clone()),
        }
    }

    /// The store the CLI stages a target's listing for: the first store with a layout whose
    /// `targets` name it.
    pub fn store_for(&self, target: &str) -> Option<(&str, &StoreRule)> {
        self.0
            .iter()
            .find(|(_, r)| r.stages() && r.targets.iter().any(|t| t == target))
            .map(|(k, r)| (k.as_str(), r))
    }

    /// Whether a store key is one the rules know, staged or not.
    pub fn known(&self, store: &str) -> bool {
        self.0.contains_key(store)
    }

    /// Whether some store the CLI stages publishes the target.
    pub fn is_store_target(&self, target: &str) -> bool {
        self.store_for(target).is_some()
    }

    /// The device kind a capture belongs to: its device slug, else the target's store's
    /// `default-kind`, else `default`.
    pub fn capture_kind(&self, target: &str, device: Option<&str>) -> String {
        device
            .map(str::to_string)
            .or_else(|| {
                self.store_for(target)
                    .and_then(|(_, r)| r.default_kind.clone())
            })
            .unwrap_or_else(|| "default".to_string())
    }

    /// Whether some store the CLI stages spells the locale.
    pub fn mappable(&self, tag: &str) -> bool {
        self.0
            .values()
            .any(|r| r.stages() && r.locales.contains_key(tag))
    }

    /// The stores the CLI stages that the app's targets publish to: the store's key, its rule,
    /// and the target that publishes there.
    pub fn lanes(&self, targets: &[String]) -> Vec<(&str, &StoreRule, String)> {
        let mut out = Vec::new();
        for (key, rule) in &self.0 {
            if !rule.stages() {
                continue;
            }
            if let Some(t) = rule.targets.iter().find(|t| targets.contains(t)) {
                out.push((key.as_str(), rule, t.clone()));
            }
        }
        out
    }
}

/// The rule for a capture's device slug: its own kind, or `tablet` for any `tablet*`.
fn rule_for<'a>(
    kinds: &'a [(&'a str, &'a ShotRule)],
    device: &str,
) -> Option<&'a (&'a str, &'a ShotRule)> {
    kinds
        .iter()
        .find(|(kind, _)| device == *kind || (*kind == "tablet" && device.starts_with("tablet")))
}

/// What `target`'s store would refuse about the listing's screenshots in `index`, read from the
/// index alone (its `width`/`height`): each capture's size, the ceiling per locale, and a
/// screenshot for every locale the index carries that the store knows, on each required kind.
/// Empty when the listing would go through.
pub fn screenshot_problems(
    index: &serde_json::Value,
    target: &'static crate::targets::Target,
    rules: &StoreRules,
) -> Vec<String> {
    let Some((store_key, rule)) = rules.store_for(target.name) else {
        return vec![format!(
            "no store the rules stage publishes {}",
            target.name
        )];
    };
    let store = rule.name(store_key);
    let kinds = rule.kinds();
    if kinds.is_empty() {
        return vec![format!(
            "the store rules name no screenshot kinds for {store_key}; the CLI's own \
             (`day store screenshots --rules`) do"
        )];
    }
    let chosen = match listing_shots(index, target, rules) {
        Ok(c) => c,
        Err(e) => return vec![e],
    };
    let mut problems = Vec::new();
    // Every locale the index carries that the store knows; a capture's own locale when the
    // index lists none.
    let mut locales: Vec<String> = index["locales"]
        .as_array()
        .map(|l| {
            l.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if locales.is_empty() {
        for s in &chosen {
            if !locales.contains(&s.locale) {
                locales.push(s.locale.clone());
            }
        }
    }
    locales.retain(|l| rule.locale(l).is_some());
    for s in &chosen {
        if rule_for(&kinds, &s.device).is_none() {
            problems.push(format!(
                "{store}: the screenshot {:?} ({}) was captured on device {:?}, which is not a \
                 kind the store lists; the CI device profile's `slug=` names it: {}",
                s.shot,
                s.locale,
                s.device,
                kinds.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")
            ));
        }
    }
    for (kind, rule) in &kinds {
        let kind = *kind;
        let mine: Vec<&StoreShot> = chosen
            .iter()
            .filter(|s| rule_for(&kinds, &s.device).is_some_and(|(k, _)| *k == kind))
            .collect();
        for s in &mine {
            let (w, h) = (s.width, s.height);
            let (lo, hi) = (w.min(h), w.max(h));
            if !rule.sizes.is_empty() && !rule.sizes.contains(&(w, h)) {
                problems.push(format!(
                    "{store}: the {} screenshot {:?} ({}) is {w}×{h}, which the store does not \
                     take for {}; it takes {}. Capture on a device profile that produces one of \
                     those",
                    kind,
                    s.shot,
                    s.locale,
                    kind,
                    rule.sizes
                        .iter()
                        .map(|(a, b)| format!("{a}×{b}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                continue;
            }
            if let Some(min_side) = rule.min_side
                && lo < min_side
            {
                problems.push(format!(
                    "{store}: the {} screenshot {:?} ({}) is {w}×{h}; the short side has to \
                         be at least {min_side} px. A CI tablet past three million pixels is \
                         captured halved (`medium_tablet` at 1280×800); such a capture is \
                         scaled up to the floor only where the store rules say `upscale = true` \
                         for the kind",
                    kind, s.shot, s.locale
                ));
            }
            if let Some(max_side) = rule.max_side
                && hi > max_side
            {
                problems.push(format!(
                    "{store}: the {} screenshot {:?} ({}) is {w}×{h}; the long side has to \
                         be at most {max_side} px",
                    kind, s.shot, s.locale
                ));
            }
            if let Some(ratio) = rule.max_ratio
                && lo > 0
                && f64::from(hi) / f64::from(lo) > ratio + 1e-9
            {
                problems.push(format!(
                    "{store}: the {} screenshot {:?} ({}) is {w}×{h}, {:.2}:1; the store \
                         takes at most {ratio}:1 (the long side no more than {ratio}× the \
                         short). Capture on a {} profile with a shorter screen",
                    kind,
                    s.shot,
                    s.locale,
                    f64::from(hi) / f64::from(lo),
                    kind
                ));
            }
        }
        let mut per_locale: BTreeMap<&str, usize> = BTreeMap::new();
        for s in &mine {
            *per_locale.entry(s.locale.as_str()).or_default() += 1;
        }
        for (locale, n) in &per_locale {
            if rule.max > 0 && *n > rule.max {
                problems.push(format!(
                    "{store}: {n} {} screenshots for {locale}, and the store takes at most {}; \
                     shorten the list in [storefront.{}.{store_key}.screenshots] to {}",
                    kind, rule.max, target.name, rule.max
                ));
            }
        }
        if rule.required {
            if mine.is_empty() {
                problems.push(format!(
                    "{store} needs {kind} screenshots and the listing declares none for it, or \
                     the walkthrough captured none on a {kind} profile. Declare them in \
                     store/storefront.toml, [storefront.{}.{store_key}.screenshots] with a \
                     `{kind}` or `default` list, and capture on a {kind} profile",
                    target.name
                ));
            } else {
                let missing: Vec<&str> = locales
                    .iter()
                    .map(String::as_str)
                    .filter(|l| !per_locale.contains_key(l))
                    .collect();
                if !missing.is_empty() {
                    problems.push(format!(
                        "{store}: no {} screenshots for {}; the walkthrough captured other \
                         locales, so run it for these too",
                        kind,
                        missing.join(", ")
                    ));
                }
            }
        }
    }
    problems
}

/// The listing's screenshots for `target`, placed where fastlane reads them.
///
/// The index's `listings` name the captures per device kind and locale (what `day screenshot
/// index` resolved from the storefront's `screenshots` tables), in the declared theme. Every
/// locale in the index that the store knows gets its own set, so the listing is as localized as
/// the walkthrough. A set the store would refuse ([`screenshot_problems`]) is refused here,
/// before anything is uploaded.
///
/// `deliver`: `fastlane/screenshots/<locale>/<NN>-<device>-<shot>.png`; deliver reads the
/// device from the image's size and orders by name. `supply`:
/// `fastlane/metadata/android/<locale>/images/<folder>/<NN>-<shot>.png`, the folder the kind's
/// rule names (`phoneScreenshots`, `tenInchScreenshots`, …).
fn stage_screenshots(
    source: &ScreenshotSource,
    rules: &StoreRules,
    target: &'static crate::targets::Target,
    out: &Path,
) -> Result<Vec<PathBuf>, String> {
    let Some((store_key, rule)) = rules.store_for(target.name) else {
        return Err(format!(
            "{}: no store the rules stage publishes this target",
            target.name
        ));
    };
    let store = rule.name(store_key);
    let deliver = rule.layout.as_deref() == Some("deliver");
    let kinds = rule.kinds();
    let index = source.index()?;
    let problems = screenshot_problems(&index, target, rules);
    if !problems.is_empty() {
        return Err(format!(
            "the listing's screenshots would be refused:\n  {}",
            problems.join("\n  ")
        ));
    }
    let mut written = Vec::new();
    for shot in listing_shots(&index, target, rules)? {
        let Some(loc) = rule.locale(&shot.locale) else {
            continue; // a locale the store does not know; lint reports it
        };
        let rel = if deliver {
            format!(
                "fastlane/screenshots/{loc}/{:02}-{}-{}.png",
                shot.position, shot.device, shot.shot
            )
        } else {
            let folder = rule_for(&kinds, &shot.device)
                .and_then(|(_, r)| r.folder.as_deref())
                .unwrap_or("phoneScreenshots");
            format!(
                "fastlane/metadata/android/{loc}/images/{folder}/{:02}-{}.png",
                shot.position, shot.shot
            )
        };
        let mut bytes = source.fetch(&shot.entry)?;
        if shot.scale > 1 {
            // A halved CI capture, brought up to the floor the store checks. The layout and
            // the content are the device's own; only the raster is coarser than a native one.
            bytes = upscale_png(&bytes, shot.scale)
                .map_err(|e| format!("{}: scaling ×{}: {e}", shot.shot, shot.scale))?;
            crate::ops::status(
                "Scaled",
                &format!(
                    "{} ({}, {}) ×{} to {}×{} for {}",
                    shot.shot, shot.device, shot.locale, shot.scale, shot.width, shot.height, store
                ),
            );
        }
        let path = out.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}

/// A PNG scaled up by a whole factor, resampled bicubically: a capture the store's floor is
/// reached by scaling, not a native one, and the caller says so.
fn upscale_png(png: &[u8], factor: u32) -> Result<Vec<u8>, String> {
    use day_vector::tiny_skia;
    let src = tiny_skia::Pixmap::decode_png(png).map_err(|e| e.to_string())?;
    let mut out = tiny_skia::Pixmap::new(src.width() * factor, src.height() * factor)
        .ok_or("an empty capture")?;
    let paint = tiny_skia::PixmapPaint {
        quality: tiny_skia::FilterQuality::Bicubic,
        ..Default::default()
    };
    out.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &paint,
        tiny_skia::Transform::from_scale(factor as f32, factor as f32),
        None,
    );
    out.encode_png().map_err(|e| e.to_string())
}

/// fastlane opens its analytics session before it parses the Fastfile, so `opt_out_usage` there
/// cannot stop the launch event — it is already out. fastlane reads `fastlane/.env.default`
/// first, which is early enough, and it travels with the staged tree: a lane run by hand on a
/// laptop is as quiet as one run in CI.
const FASTLANE_ENV: &str = "\
# Generated by `day store stage` — edit store/ in the project, not this file.\n\
# No metrics: a lane that publishes an app sends nothing anywhere but the store.\n\
FASTLANE_OPT_OUT_USAGE=1\n";

fn apple_appfile(id: &str, app: &SubmissionInfo) -> String {
    let mut s = String::from(
        "# Generated by `day store stage` — edit store/ in the project, not this file.\n",
    );
    s.push_str(&format!("app_identifier({id:?})\n"));
    if let Some(e) = &app.contact_email {
        s.push_str(&format!("apple_id({e:?})\n"));
    }
    s.push_str(
        "# Credentials come from the environment (App Store Connect API key):\n\
         #   DAY_ASC_KEY_ID, DAY_ASC_ISSUER, and DAY_ASC_KEY (path to the .p8) or\n\
         #   DAY_ASC_KEY_CONTENT (its contents)\n",
    );
    s
}

fn play_appfile(id: &str) -> String {
    format!(
        "# Generated by `day store stage` — edit store/ in the project, not this file.\n\
         package_name({id:?})\n\
         # Credentials come from the environment: SUPPLY_JSON_KEY (service-account JSON path).\n"
    )
}

fn apple_fastfile(_project: &Project, with_screenshots: bool) -> String {
    // `deliver` uploads the screenshot tree beside the metadata when told to; without any staged
    // there is nothing to send, and skipping keeps what the record already shows.
    let screenshots = if with_screenshots {
        "skip_screenshots: false,\n      overwrite_screenshots: true,"
    } else {
        "skip_screenshots: true,"
    };
    apple_fastfile_text().replace("skip_screenshots: true,", screenshots)
}

fn apple_fastfile_text() -> String {
    // The artifact is found by glob, not by name: `day pack` names an unsigned device build
    // `<stem>-ios-uikit-unsigned.ipa` and a signed one `<stem>-ios-uikit.ipa` (pack/naming.rs),
    // and a lane that hardcoded one of those would break on exactly the day signing was
    // configured. The stem is the app's, which is the other reason not to write it here.
    r##"# Generated by `day store stage` — edit store/ in the project, not this file.
#
# No metrics: fastlane reports usage to its own servers unless a Fastfile opts out, and a lane
# that publishes an app is not a place to send anything anywhere but the store.
opt_out_usage
# The .ipa comes from `day pack -p ios-uikit`; this only uploads what that produced.
default_platform(:ios)

# DAY_IPA names the artifact outright (the release workflow points it at the packed
# artifact it downloaded). Otherwise __dir__ is <project>/build/day/store/<target>/fastlane,
# so ../../../dist is build/day/dist — where `day pack` puts its output.
def day_ipa
  return ENV["DAY_IPA"] unless ENV["DAY_IPA"].to_s.empty?
  Dir[File.expand_path("../../../dist/*.ipa", __dir__)].first ||
    UI.user_error!("no .ipa in build/day/dist — run `day pack -p ios-uikit` first")
end

# App Store Connect API key from the environment, so no Apple ID password is ever needed.
def day_env!(name, what)
  ENV[name].to_s.empty? ? UI.user_error!("#{name} is not set — #{what}") : ENV[name]
end

def day_asc_key
  app_store_connect_api_key(
    key_id: day_env!("DAY_ASC_KEY_ID", "the App Store Connect API key id"),
    issuer_id: day_env!("DAY_ASC_ISSUER", "the App Store Connect issuer id"),
    key_filepath: ENV["DAY_ASC_KEY"],
    key_content: ENV["DAY_ASC_KEY_CONTENT"],
    in_house: false,
  )
end

platform :ios do
  desc "Validate the listing and the .ipa against App Store Connect WITHOUT uploading."
  lane :validate do
    deliver(
      api_key: day_asc_key,
      ipa: day_ipa,
      metadata_path: File.expand_path("metadata", __dir__),
      verify_only: true,
      force: true,
      skip_screenshots: true,
      precheck_include_in_app_purchases: false,
    )
  end

  desc "Upload the build + listing to App Store Connect. Does NOT submit for review."
  # The version is left in Prepare for Submission with no build attached: App Store Connect
  # attaches one as part of a submission, so an upload alone reads as an empty version in the
  # web UI even though the binary is there and processing. `release` is the lane that finishes
  # the job; `submit` finishes one this lane started earlier.
  lane :upload do
    deliver(
      api_key: day_asc_key,
      ipa: day_ipa,
      metadata_path: File.expand_path("metadata", __dir__),
      submit_for_review: false,
      automatic_release: false,
      force: true,
      skip_screenshots: true,
      precheck_include_in_app_purchases: false,
    )
  end

  desc "Attach the build already uploaded for this version and SUBMIT it for review."
  # For a version whose binary is in App Store Connect already: uploading the same build number
  # twice is refused, so this submits what is there rather than sending it again. Once approved
  # the version goes live on its own, unless the listing said `apple-release = "manual"`, which
  # `day store stage` writes into .env.default as DAY_ASC_MANUAL_RELEASE.
  lane :submit do
    deliver(
      api_key: day_asc_key,
      skip_binary_upload: true,
      metadata_path: File.expand_path("metadata", __dir__),
      submit_for_review: true,
      automatic_release: ENV["DAY_ASC_MANUAL_RELEASE"].to_s.empty?,
      force: true,
      skip_screenshots: true,
      submission_information: {
        export_compliance_uses_encryption: false,
        add_id_info_uses_idfa: false,
      },
      precheck_include_in_app_purchases: false,
    )
  end

  desc "Upload the build + listing, wait for processing, and SUBMIT the version for review."
  # An approved version is released to the store on its own; `apple-release = "manual"` in the
  # listing (DAY_ASC_MANUAL_RELEASE in .env.default) keeps it waiting for the Release button in
  # App Store Connect instead. Export compliance is answered as exempt (the app uses only the
  # platform's HTTPS), and the build carries no advertising identifier.
  lane :release do
    deliver(
      api_key: day_asc_key,
      ipa: day_ipa,
      metadata_path: File.expand_path("metadata", __dir__),
      submit_for_review: true,
      automatic_release: ENV["DAY_ASC_MANUAL_RELEASE"].to_s.empty?,
      submission_information: {
        export_compliance_uses_encryption: false,
        add_id_info_uses_idfa: false,
      },
      force: true,
      skip_screenshots: true,
      precheck_include_in_app_purchases: false,
    )
  end
end
"##
    .to_string()
}

fn play_fastfile(_project: &Project, with_screenshots: bool) -> String {
    // `supply` sends every image directory it finds under the listing. The feature graphic and
    // icon are never staged, and the screenshots only when some were.
    let images = format!(
        "skip_upload_apk: true,\n      skip_upload_images: true,\n      skip_upload_screenshots: {},",
        !with_screenshots
    );
    play_fastfile_text().replace("skip_upload_apk: true,", &images)
}

fn play_fastfile_text() -> String {
    r##"# Generated by `day store stage` — edit store/ in the project, not this file.
#
# No metrics: fastlane reports usage to its own servers unless a Fastfile opts out, and a lane
# that publishes an app is not a place to send anything anywhere but the store.
opt_out_usage
# The .aab comes from `day pack -p android-mdc`; this only uploads what that produced.
default_platform(:android)

# __dir__ is <project>/build/day/store/<target>/fastlane, so ../../../dist is build/day/dist.
def day_json_key
  key = ENV["SUPPLY_JSON_KEY"].to_s
  UI.user_error!("SUPPLY_JSON_KEY is not set — the path to the Play service-account JSON") if key.empty?
  key
end

# DAY_AAB names the artifact outright (the release workflow points it at the packed
# artifact it downloaded), the way DAY_IPA does for iOS. Otherwise the glob under
# build/day/dist — where `day pack` puts its output.
def day_aab
  return ENV["DAY_AAB"] unless ENV["DAY_AAB"].to_s.empty?
  Dir[File.expand_path("../../../dist/*.aab", __dir__)].first ||
    UI.user_error!("no .aab in build/day/dist — run `day pack -p android-mdc` first")
end

platform :android do
  desc "Validate the listing and the .aab against Google Play WITHOUT publishing."
  lane :validate do
    supply(
      aab: day_aab,
      json_key: day_json_key,
      metadata_path: File.expand_path("metadata/android", __dir__),
      track: "internal",
      release_status: "draft",
      validate_only: true,
      skip_upload_apk: true,
    )
  end

  desc "Upload the .aab + listing to the internal track as an unreleased draft."
  lane :upload do
    supply(
      aab: day_aab,
      json_key: day_json_key,
      metadata_path: File.expand_path("metadata/android", __dir__),
      track: "internal",
      release_status: "draft",
      skip_upload_apk: true,
    )
  end

  desc "Upload the .aab + listing to the production track and submit it: Google reviews it, then rolls it out."
  # Play has no separate submit step: a completed production release is the submission, and
  # the rollout starts when Google's review passes. The first bundle of a new app still has to
  # be uploaded through the Play Console by hand before the API will take one.
  lane :release do
    supply(
      aab: day_aab,
      json_key: day_json_key,
      metadata_path: File.expand_path("metadata/android", __dir__),
      track: "production",
      release_status: "completed",
      skip_upload_apk: true,
    )
  end
end
"##
    .to_string()
}

/// Where a target's fastlane project is written: `build/day/store/<target>/`, holding the
/// `fastlane/` folder the tool insists on finding (it locates its config by folder, not by file;
/// a Fastfile sitting loose in the working directory is not seen).
pub fn stage_dir(project: &Project, target: &'static crate::targets::Target) -> PathBuf {
    project.root.join("build/day/store").join(target.name)
}

/// Targets that have a store listing at all.
pub fn is_store_target(target: &'static crate::targets::Target) -> bool {
    StoreRules::builtin().is_store_target(target.name)
}

// ---------------------------------------------------------------------------
// Lint (docs/store.md)
// ---------------------------------------------------------------------------

/// One problem with a listing. `code` matches `day lint`'s vocabulary.
#[derive(Default)]
pub struct Problem {
    pub code: &'static str,
    pub message: String,
    /// Project-relative file the problem is in. `None` when the problem is that a file, a locale
    /// or the whole listing is missing, so there is nothing to point an editor at.
    pub file: Option<String>,
    /// A safe, unambiguous repair. Only two listing rules have one: whitespace around a field, and
    /// spaces in a keyword list. Everything else needs a human to write words.
    pub fix: Option<crate::lint::Fix>,
}

/// Check a listing against the stores the app targets.
///
/// Silent when the app ships to neither store, and when it has no `store/` at all; an app that
/// never leaves a developer's machine should not be nagged about App Store copy. Once `store/`
/// exists, it is held to the stores' rules, because the alternative is finding out at upload time.
pub fn lint(project: &Project, listing: &Listing, rules: &StoreRules) -> Vec<Problem> {
    let mut out = Vec::new();
    // The listing directory this run read: `store/`, or the flavor's (DESIGN.md §16.6). Findings
    // name paths relative to the project, and `--fix` writes to the path a finding names, so a
    // flavored run has to point at the files it actually read.
    let store = dir(project)
        .strip_prefix(&project.root)
        .map(|rel| rel.display().to_string())
        .unwrap_or_else(|_| "store".to_string());
    let targets = &project.manifest.app.targets;
    let declared = &listing.app;
    let app_file = format!(
        "{store}/{}",
        listing.app_file.as_deref().unwrap_or("storefront.toml")
    );
    // The names from before the rename still read; the finding says what to call them now.
    for file in &listing.files {
        if is_legacy_name(file) {
            let now = format!("storefront{}", &file["app".len()..]);
            out.push(Problem {
                code: "day::lint::store-legacy-name",
                message: format!(
                    "{store}/{file}: the name from before the storefront file was renamed \
                     (2026-09-25); rename it to {store}/{now}"
                ),
                file: Some(format!("{store}/{file}")),
                fix: None,
            });
        }
    }
    for key in &declared.legacy_keys {
        out.push(Problem {
            code: "day::lint::store-top-level",
            message: format!(
                "{app_file}: {key:?} at the top level is not read. Submission metadata \
                 lives under [storefront.submission-info] (shared), \
                 [storefront.<target>.submission-info] or \
                 [storefront.<target>.<store>.submission-info]; the listing text under \
                 [storefront.metadata] and its locale tables; screenshots under \
                 [storefront.<target>.screenshots] and [storefront.<target>.<store>.screenshots] \
                 (docs/store.md)"
            ),
            file: Some(app_file.clone()),
            fix: None,
        });
    }
    // Keys the parser read past: each names the table, the key, and the keys it takes.
    for message in &declared.problems {
        let message = match listing
            .files
            .iter()
            .find(|f| message.starts_with(&format!("{f}: ")))
        {
            Some(_) => format!("{store}/{message}"),
            None => format!("{app_file}: {message}"),
        };
        out.push(Problem {
            code: "day::lint::store-unknown-key",
            message,
            file: Some(app_file.clone()),
            fix: None,
        });
    }
    // The listings (§14.7) name targets the app builds and shots a dayscript captures; a typo
    // in either is a listing that silently shows nothing, so both are checked here, for every
    // target, since a desktop target's list drives its website page. A store key the rules do
    // not know rides through for whatever publishes there, which is right for a storefront
    // the CLI has not heard of and wrong for a typo, so it is named either way.
    if !declared.targets.is_empty() {
        let file = Some(app_file.clone());
        let known: Vec<&str> = rules.0.keys().map(String::as_str).collect();
        for (target, te) in &declared.targets {
            if !targets.iter().any(|t| t == target) {
                out.push(Problem {
                    code: "day::lint::store-unknown-target",
                    message: format!(
                        "{app_file} declares [storefront.{target}], which is not one \
                         of this app's targets ({})",
                        targets.join(", ")
                    ),
                    file: file.clone(),
                    fix: None,
                });
            }
            for key in te.stores.keys() {
                if !rules.known(key) {
                    out.push(Problem {
                        code: "day::lint::store-unknown-store",
                        message: format!(
                            "{app_file} [storefront.{target}.{key}]: not a storefront the store \
                             rules know ({}); its declarations ride through gallery.json and \
                             the export for whatever publishes there, and the CLI stages none \
                             of it",
                            known.join(", ")
                        ),
                        file: file.clone(),
                        fix: None,
                    });
                }
            }
        }
        let mut captured: BTreeSet<String> = BTreeSet::new();
        if let Ok(scripts) = std::fs::read_dir(project.root.join("dayscript")) {
            for script in scripts.flatten() {
                let path = script.path();
                if path.extension().is_some_and(|x| x == "yaml" || x == "yml") {
                    captured.extend(
                        crate::screenshot::script_screenshot_meta(&path)
                            .into_iter()
                            .map(|(name, _)| name),
                    );
                }
            }
        }
        if !captured.is_empty() && declared.has_screenshots() {
            for shot in declared.shot_names() {
                if !captured.contains(&shot) {
                    out.push(Problem {
                        code: "day::lint::store-unknown-shot",
                        message: format!(
                            "{app_file} lists the screenshot {shot:?}, which no dayscript \
                             captures (no `screenshot:` step of that name under dayscript/)"
                        ),
                        file: file.clone(),
                        fix: None,
                    });
                }
            }
        }
    }
    // The stores the CLI stages that this app publishes to, per the rules: none, and the
    // listing text is nobody's business.
    let lanes = rules.lanes(targets);
    if lanes.is_empty() {
        return out;
    }
    let store_names: Vec<&str> = lanes.iter().map(|(k, r, _)| r.name(k)).collect();
    let app_locales = app_locales(project);
    let default = listing.default_locale.as_str();
    if listing.is_empty() {
        if !app_locales.is_empty() || !targets.is_empty() {
            let legacy = dir(project).join(default).is_dir();
            out.push(Problem {
                code: "day::lint::store-missing",
                message: format!(
                    "this app ships to {} but {store}/ has no listing text — {}",
                    store_names.join(" and "),
                    if legacy {
                        format!(
                            "{store}/{default}/ is the layout from before [storefront.metadata]; \
                             run `day store migrate` to fold it into {app_file}"
                        )
                    } else {
                        "run `day store init`".to_string()
                    }
                ),
                ..Default::default()
            });
        }
        return out;
    }

    // --- locale parity with the app's own translations ---
    let declared_locales = declared.metadata_locales();
    for tag in &app_locales {
        if tag != default && !declared_locales.contains(tag) {
            out.push(Problem {
                code: "day::lint::store-missing-locale",
                message: format!(
                    "the app is translated into {tag} but no [storefront.metadata.{tag}] table \
                     (or {store}/storefront.{tag}.toml) carries its listing — the store shows \
                     those users the default language"
                ),
                ..Default::default()
            });
        }
    }
    for tag in &declared_locales {
        if !app_locales.contains(tag) && !app_locales.is_empty() {
            out.push(Problem {
                code: "day::lint::store-orphan-locale",
                message: format!(
                    "[storefront.metadata.{tag}] is a listing for a locale the app is not \
                     translated into (resource/locales/{tag}/ does not exist)"
                ),
                ..Default::default()
            });
        }
        if !rules.mappable(tag) {
            out.push(Problem {
                code: "day::lint::store-unmapped-locale",
                message: format!(
                    "[storefront.metadata.{tag}]: no store spells {tag:?} (store-rules.toml \
                     lists each store's locales) — a listing uploaded under an unknown locale \
                     is dropped without an error"
                ),
                ..Default::default()
            });
        }
    }
    if declared.metadata("", "", default).is_empty()
        && declared.targets.values().all(|te| {
            te.metadata.base.is_empty() && te.stores.values().all(|se| se.metadata.base.is_empty())
        })
    {
        out.push(Problem {
            code: "day::lint::store-default-locale",
            message: format!(
                "{app_file} has no [storefront.metadata] table, and {default} is the app's \
                 default locale — both stores require a complete listing in the primary language"
            ),
            file: Some(app_file.clone()),
            ..Default::default()
        });
    }

    // --- each declared text, held to the stores it can reach ---
    for d in declared.texts() {
        let reach: Vec<&(&str, &StoreRule, String)> = lanes
            .iter()
            .filter(|(key, _, target)| match (d.target, d.store) {
                (None, _) => true,
                (Some(t), None) => t == target,
                (Some(t), Some(s)) => t == target && s == *key,
            })
            .collect();
        let place = d.text.origin.describe(&store);
        let file = Some(d.text.origin.path(&store));
        let text = d.text.value.as_str();
        let chars = text.chars().count();
        for (key, rule, _) in &reach {
            let Some(fr) = rule.field(d.field) else {
                continue;
            };
            if chars > fr.limit {
                out.push(Problem {
                    code: "day::lint::store-too-long",
                    message: format!(
                        "{place}: {chars} characters, {} allows {}",
                        rule.name(key),
                        fr.limit
                    ),
                    file: file.clone(),
                    ..Default::default()
                });
            }
        }
        if d.field.is_url() && !text.starts_with("https://") {
            out.push(Problem {
                code: "day::lint::store-bad-url",
                message: format!("{place}: must be an https:// URL"),
                file: file.clone(),
                ..Default::default()
            });
        }
        if text.contains("TODO") {
            let locale = d.locale.unwrap_or(default);
            out.push(Problem {
                code: "day::lint::store-placeholder",
                message: format!(
                    "{place}: still the scaffold's TODO — the {locale} listing would upload it \
                     verbatim"
                ),
                file: file.clone(),
                ..Default::default()
            });
        }
        if text.trim() != text {
            // A referenced file is rewritten whole; an inline value sits in a file the fix
            // machinery does not edit by key, so that one is reported alone.
            let fix = match &d.text.origin {
                Origin::File { file, .. } => Some(crate::lint::Fix {
                    title: "Trim the surrounding whitespace".into(),
                    contents: format!("{}\n", text.trim()),
                    file: file.clone(),
                }),
                Origin::Inline { .. } => None,
            };
            out.push(Problem {
                code: "day::lint::store-whitespace",
                message: format!("{place}: leading or trailing whitespace"),
                file: file.clone(),
                fix,
            });
        }
    }

    // --- what each store's record resolves to, per locale ---
    // Each store's `required` fields (Play a short description, Apple a privacy policy URL)
    // are easy to forget until the submission is rejected.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (store_key, rule, target) in &lanes {
        for locale in listing.locales() {
            let fields = declared.metadata(target, store_key, &locale);
            for key in &rule.required {
                let Some(f) = Field::from_key(key) else {
                    continue;
                };
                if fields.contains_key(&f) {
                    continue;
                }
                let table = if locale == default {
                    "[storefront.metadata]".to_string()
                } else {
                    format!("[storefront.metadata.{locale}]")
                };
                let message = format!(
                    "{} requires `{key}` and the {locale} listing has none — set it in \
                     {app_file} {table} (or a target's or store's metadata table)",
                    rule.name(store_key)
                );
                if seen.insert(message.clone()) {
                    out.push(Problem {
                        code: "day::lint::store-missing-field",
                        message,
                        file: Some(app_file.clone()),
                        ..Default::default()
                    });
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// `day store …`
// ---------------------------------------------------------------------------

/// Skeleton text for a field: what it is for, and the budget it has to fit.
fn skeleton(field: Field, project: &Project) -> Option<String> {
    let title = project
        .manifest
        .app
        .title
        .clone()
        .unwrap_or_else(|| project.manifest.app.name.clone());
    Some(match field {
        Field::Name => title,
        Field::Subtitle => "TODO: 30 characters, App Store".into(),
        Field::Short => {
            "TODO: one sentence, up to 80 characters, shown in Google Play search results.".into()
        }
        Field::Description => "TODO: what the app does, who it is for, what it does not do.\n\n\
             Both stores allow 4000 characters and show the first few lines before a fold."
            .into(),
        Field::Keywords => "todo,comma,separated".into(),
        Field::ReleaseNotes => "TODO: what changed in this version (Google Play allows 500 \
                                characters, and that is the limit that binds)"
            .into(),
        Field::PrivacyUrl => "https://example.com/privacy".into(),
        // Optional fields are left absent rather than filled with a placeholder that would upload.
        Field::Promo | Field::MarketingUrl | Field::SupportUrl => return None,
    })
}

/// A field's value as TOML: a list for keywords, a multi-line basic string for text with
/// newlines, a plain string otherwise.
fn toml_value(field: Field, value: &str) -> String {
    if field == Field::Keywords {
        let words: Vec<String> = value
            .split(',')
            .map(|w| toml::Value::String(w.trim().to_string()).to_string())
            .collect();
        return format!("[{}]", words.join(", "));
    }
    if value.contains('\n') {
        // A newline right after the opening delimiter is not part of the string, so the text
        // starts on its own line; the closing delimiter takes a line of its own the same way.
        let body = value.replace('\\', "\\\\").replace("\"\"\"", "\"\"\\\"");
        return format!("\"\"\"\n{body}\n\"\"\"");
    }
    toml::Value::String(value.to_string()).to_string()
}

/// One `metadata` table as TOML text, headed `[name]`.
fn toml_table(name: &str, fields: &BTreeMap<Field, String>) -> String {
    let mut s = format!("\n[{name}]\n");
    for f in FIELDS {
        if let Some(v) = fields.get(f) {
            s.push_str(&format!("{} = {}\n", f.key(), toml_value(*f, v)));
        }
    }
    s
}

/// A locale file's whole text in YAML: `storefront: { metadata: { … } }` (the same shape under
/// `<target>` and `<store>` is not written by the CLI; an author adds those by hand).
fn yaml_sidecar(fields: &BTreeMap<Field, String>) -> Result<String, String> {
    let mut table = serde_json::Map::new();
    for f in FIELDS {
        if let Some(v) = fields.get(f) {
            let value = if *f == Field::Keywords {
                serde_json::Value::Array(
                    v.split(',')
                        .map(|w| serde_json::Value::String(w.trim().to_string()))
                        .collect(),
                )
            } else {
                serde_json::Value::String(v.clone())
            };
            table.insert(f.key().to_string(), value);
        }
    }
    let doc = serde_json::json!({ "storefront": { "metadata": table } });
    serde_norway::to_string(&doc).map_err(|e| e.to_string())
}

/// A locale's text as one table, keyed by field: what the writers below take.
type LocaleTable = (String, BTreeMap<Field, String>);

/// The table a locale's text is written under: the base table for the default locale, a
/// locale table otherwise.
fn table_name(locale: &str, default: &str) -> String {
    if locale == default {
        "storefront.metadata".to_string()
    } else {
        format!("storefront.metadata.{locale}")
    }
}

/// Write listing text for locales into the storefront: appended to a TOML main file as
/// `[storefront.metadata]` / `[storefront.metadata.<tag>]` tables (created, with a header,
/// when there is no main file), or, beside a YAML main file, as one `app.<tag>.yaml` locale
/// file each, since a YAML document cannot take a table appended at its end. Returns one line
/// per file touched.
fn write_locale_tables(
    root: &Path,
    main: Option<&str>,
    default: &str,
    tables: &[LocaleTable],
    id: &str,
) -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    if tables.is_empty() {
        return Ok(done);
    }
    std::fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    match main {
        Some(name) if is_yaml(name) => {
            let stem = storefront_stem(name);
            for (locale, fields) in tables {
                let file = format!("{stem}.{locale}.yaml");
                let path = root.join(&file);
                if path.exists() {
                    return Err(format!("{}: already exists", path.display()));
                }
                let body = format!(
                    "# The {locale} listing text, over the default locale's ({}'s [storefront.metadata]) key\n\
                     # by key. A locale file carries `metadata` tables alone.\n{}",
                    default,
                    yaml_sidecar(fields)?
                );
                std::fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))?;
                done.push(format!("created {file} ({} field(s))", fields.len()));
            }
        }
        _ => {
            let name = main.unwrap_or("storefront.toml");
            let path = root.join(name);
            let mut text = if path.is_file() {
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?
            } else {
                storefront_header(id)
            };
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            for (locale, fields) in tables {
                text.push_str(&toml_table(&table_name(locale, default), fields));
                done.push(format!(
                    "wrote [{}] to {name} ({} field(s))",
                    table_name(locale, default),
                    fields.len()
                ));
            }
            std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(done)
}

/// The opening of a fresh `store/storefront.toml`: what the file is, and the submission info to fill.
fn storefront_header(id: &str) -> String {
    format!(
        "# The store listing (https://daybrite.dev/docs/store): everything the App Store, Google\n\
         # Play and the app's website show, in one file. [storefront.submission-info] is shared\n\
         # by every store; [storefront.<target>.submission-info] and\n\
         # [storefront.<target>.<store>.submission-info] override it key by key. The listing\n\
         # text is [storefront.metadata] (the default locale) with a table per other locale,\n\
         # specialized the same way per target and per store; `<field>-ref = \"path\"` reads a\n\
         # field from a project file instead. Screenshots: [storefront.<target>.screenshots]\n\
         # (the website's page for the target, and the stores' fallback) and\n\
         # [storefront.<target>.<store>.screenshots], per device kind or `default`, naming\n\
         # dayscript `screenshot:` steps. The same tree may be written as store/app.yaml.\n\
         [storefront.submission-info]\n\
         bundle-id = {id:?}\n\
         # copyright = \"2026 Example\"\n\
         # contact-email = \"support@example.com\"\n\
         # review-notes = \"How to exercise the app, for the store reviewer.\"\n\
         # apple-release = \"manual\"   # hold an approved App Store version for the Release button; unset releases it\n\
         \n\
         # [storefront.ios-uikit.apple-app-store.submission-info]\n\
         # apple-category = \"DEVELOPER_TOOLS\"   # App Store primary category\n"
    )
}

/// `day store init`: the listing text for every locale the app ships, as tables in
/// `store/storefront.toml` (created if absent), leaving whatever is already declared alone.
fn init(project: &Project) -> Result<(), CliError> {
    let listing = read(project).map_err(CliError::failure)?;
    let root = dir(project);
    let default = listing.default_locale.clone();
    let mut locales = app_locales(project);
    if locales.is_empty() {
        locales.push(default.clone());
    }
    let present = listing.app.metadata_locales();
    let has_base = !listing.app.metadata("", "", &default).is_empty();
    let mut tables = Vec::new();
    for tag in &locales {
        if *tag == default {
            if has_base {
                continue;
            }
            let mut fields = BTreeMap::new();
            for f in FIELDS {
                if let Some(body) = skeleton(*f, project) {
                    fields.insert(*f, body);
                }
            }
            tables.push((tag.clone(), fields));
        } else if !present.contains(tag) {
            // A locale's table starts as the default's text: a translator replaces it, and
            // until then the store shows those users the default language on purpose.
            let base: BTreeMap<Field, String> = listing
                .app
                .metadata("", "", &default)
                .into_iter()
                .map(|(f, t)| (f, t.value))
                .collect();
            let fields = if base.is_empty() {
                FIELDS
                    .iter()
                    .filter_map(|f| skeleton(*f, project).map(|b| (*f, b)))
                    .collect()
            } else {
                base
            };
            tables.push((tag.clone(), fields));
        }
    }
    let lines = write_locale_tables(
        &root,
        listing.app_file.as_deref(),
        &default,
        &tables,
        &project.manifest.app.id,
    )
    .map_err(CliError::failure)?;
    for line in &lines {
        crate::ops::status("Listing", line);
    }
    if lines.is_empty() {
        crate::ops::status(
            "Listing",
            &format!("{} already carries every locale's text", root.display()),
        );
    } else {
        crate::ops::status("Next", "fill in every TODO, then `day lint`");
    }
    Ok(())
}

/// The text a legacy `store/<locale>/*.txt` directory holds, one map per locale directory.
fn legacy_locale_dirs(root: &Path) -> Result<Vec<LocaleTable>, String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return Ok(out);
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for p in dirs {
        let Some(tag) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let mut fields = BTreeMap::new();
        for f in FIELDS {
            let file = p.join(f.legacy_file());
            if !file.is_file() {
                continue;
            }
            let text = std::fs::read_to_string(&file)
                .map_err(|e| format!("{}: {e}", file.display()))?
                .trim_end_matches(['\n', '\r'])
                .to_string();
            if !text.trim().is_empty() {
                let text = if *f == Field::Keywords {
                    join_keywords(text.split([',', '\n']))
                } else {
                    text
                };
                fields.insert(*f, text);
            }
        }
        if !fields.is_empty() {
            out.push((tag.to_string(), fields));
        }
    }
    Ok(out)
}

/// `day store migrate`: fold the `store/<locale>/*.txt` layout into the storefront file's
/// `metadata` tables, and remove the directories (unless `--keep`).
fn migrate(project: &Project, keep: bool) -> Result<(), CliError> {
    let listing = read(project).map_err(CliError::failure)?;
    let root = dir(project);
    let tables = legacy_locale_dirs(&root).map_err(CliError::failure)?;
    if tables.is_empty() {
        return Err(CliError::failure(format!(
            "{}: no store/<locale>/ directories with listing text to migrate",
            root.display()
        )));
    }
    if listing.app.has_metadata() {
        return Err(CliError::failure(format!(
            "{} already carries listing text under [storefront.metadata]; fold the remaining \
             store/<locale>/ files in by hand and delete them",
            root.display()
        )));
    }
    let lines = write_locale_tables(
        &root,
        listing.app_file.as_deref(),
        &listing.default_locale,
        &tables,
        &project.manifest.app.id,
    )
    .map_err(CliError::failure)?;
    for line in &lines {
        crate::ops::status("Migrated", line);
    }
    if !keep {
        for (tag, _) in &tables {
            let dir = root.join(tag);
            std::fs::remove_dir_all(&dir)
                .map_err(|e| CliError::failure(format!("{}: {e}", dir.display())))?;
            crate::ops::status("Removed", &format!("{}/", dir.display()));
        }
    }
    crate::ops::status(
        "Next",
        "move values every locale shares (URLs, the name) up into [storefront.metadata] and \
         drop them from the locale tables, then `day lint`",
    );
    Ok(())
}

/// A locale's text as JSON, keyed by field (`keywords` as a list).
fn fields_json(fields: &BTreeMap<Field, Text>) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for (f, t) in fields {
        let value = if *f == Field::Keywords {
            serde_json::Value::Array(
                t.value
                    .split(',')
                    .filter(|w| !w.is_empty())
                    .map(|w| serde_json::Value::String(w.to_string()))
                    .collect(),
            )
        } else {
            serde_json::Value::String(t.value.clone())
        };
        out.insert(f.key().to_string(), value);
    }
    serde_json::Value::Object(out)
}

/// A `screenshots` declaration as JSON, as written: lists per device kind and `default`, and
/// a table per locale of the same.
fn screenshots_json(s: &Screenshots) -> serde_json::Value {
    let lists = |l: &ShotLists| -> serde_json::Map<String, serde_json::Value> {
        let mut out = serde_json::Map::new();
        if let Some(d) = &l.default {
            out.insert(
                "default".into(),
                serde_json::to_value(d).unwrap_or_default(),
            );
        }
        for (k, v) in &l.kinds {
            out.insert(k.clone(), serde_json::to_value(v).unwrap_or_default());
        }
        out
    };
    let mut out = lists(&s.all);
    for (locale, l) in &s.locales {
        out.insert(locale.clone(), serde_json::Value::Object(lists(l)));
    }
    serde_json::Value::Object(out)
}

/// The storefront, resolved: the shared submission info and text per locale, and per target
/// (every target the app builds or the file names) the same plus its screenshots and its
/// stores, each store with its own resolved info, text and screenshots. What `day metadata
/// --json` carries under `storefront` and `day store export` writes.
pub fn storefront_json(project: &Project, listing: &Listing) -> serde_json::Value {
    let app = &listing.app;
    let locales = listing.locales();
    let per_locale = |target: &str, store: &str| -> serde_json::Value {
        let mut out = serde_json::Map::new();
        for locale in &locales {
            out.insert(
                locale.clone(),
                fields_json(&app.metadata(target, store, locale)),
            );
        }
        serde_json::Value::Object(out)
    };
    let mut names: Vec<String> = project.manifest.app.targets.clone();
    for declared in app.targets.keys() {
        if !names.contains(declared) {
            names.push(declared.clone());
        }
    }
    let mut targets = serde_json::Map::new();
    for target in names {
        let mut stores = serde_json::Map::new();
        for store in app.stores(&target) {
            let se = &app.targets[&target].stores[&store];
            stores.insert(
                store.clone(),
                serde_json::json!({
                    "submission-info": app.submission(&target, &store),
                    "metadata": per_locale(&target, &store),
                    "screenshots": screenshots_json(&se.screenshots),
                }),
            );
        }
        let screenshots = app
            .targets
            .get(&target)
            .map(|te| screenshots_json(&te.screenshots))
            .unwrap_or_else(|| serde_json::json!({}));
        targets.insert(
            target.clone(),
            serde_json::json!({
                "submission-info": app.submission(&target, ""),
                "metadata": per_locale(&target, ""),
                "screenshots": screenshots,
                "stores": stores,
            }),
        );
    }
    serde_json::json!({
        "file": listing.app_file.as_ref().map(|f| format!("store/{f}")),
        "files": listing.files.iter().map(|f| format!("store/{f}")).collect::<Vec<_>>(),
        "default-locale": listing.default_locale,
        "locales": locales,
        "submission-info": app.info,
        "metadata": per_locale("", ""),
        "targets": targets,
    })
}

/// `day store export`: the whole listing as one JSON document — the project's identity, its
/// locales, the storefront resolved per target, store and locale, the live store records, the
/// declared permissions, and the stores' rules in force — for the app's website build and for
/// anything else that publishes the app (docs/store.md "Exporting").
pub fn export_document(project: &Project) -> Result<serde_json::Value, String> {
    let listing = read(project)?;
    let rules = StoreRules::load(project, None)?;
    let m = &project.manifest;
    Ok(serde_json::json!({
        "schema": 1,
        "generator": format!("day {}", env!("CARGO_PKG_VERSION")),
        "project": {
            "name": m.app.name,
            "id": m.app.id,
            "title": m.app.title,
            "version": m.app.version,
            "build": m.app.build,
            "artifact": crate::meta::slug(m.app.artifact.as_deref().unwrap_or_else(|| {
                m.app.title.as_deref().unwrap_or(&m.app.name)
            })),
            "targets": m.app.targets,
            "store": {
                "apple-app-id": m.store.apple_app_id,
                "google-play-id": m.store.google_play_id,
                "apple-url": m.store.apple_url(),
                "google-url": m.store.google_url(),
            },
        },
        "default-locale": listing.default_locale,
        "locales": listing.locales(),
        "storefront": storefront_json(project, &listing),
        "permissions": crate::metadata::declared_permissions(project),
        "rawPermissions": crate::metadata::raw_permissions(project),
        // The stores' rules in force for this project, so a consumer holds captures and text
        // to the same limits without a copy of its own.
        "rules": rules,
    }))
}

fn export_cmd(project: &Project, out: Option<&Path>) -> Result<(), CliError> {
    let doc = export_document(project).map_err(CliError::failure)?;
    let text = serde_json::to_string_pretty(&doc).map_err(|e| CliError::failure(e.to_string()))?;
    match out {
        Some(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| CliError::failure(format!("{}: {e}", parent.display())))?;
            }
            std::fs::write(path, format!("{text}\n"))
                .map_err(|e| CliError::failure(format!("{}: {e}", path.display())))?;
            crate::ops::status("Exported", &path.display().to_string());
        }
        None => println!("{text}"),
    }
    Ok(())
}

/// `day localize add`: a locale's table, started as the default locale's text (the shared
/// level's), unless the listing already carries the locale. One line per file touched.
pub fn add_locale(project_root: &Path, tag: &str) -> Result<Vec<String>, String> {
    let root = project_root.join("store");
    let locales = app_locales_in(project_root);
    let default = default_locale(&locales).unwrap_or_else(|| "en".into());
    let loaded = load_storefront(project_root, &root, &default)?;
    if tag == default || loaded.storefront.metadata_locales().contains(tag) {
        return Ok(Vec::new());
    }
    let base: BTreeMap<Field, String> = loaded
        .storefront
        .metadata("", "", &default)
        .into_iter()
        .map(|(f, t)| (f, t.value))
        .collect();
    if base.is_empty() {
        return Ok(Vec::new());
    }
    let id = String::new();
    write_locale_tables(
        &root,
        loaded.file.as_deref(),
        &default,
        &[(tag.to_string(), base)],
        &id,
    )
}

/// `day localize remove`: drop a locale's text everywhere it sits — its locale files, and its
/// tables at every level of a TOML main file (a YAML main file is edited by hand: no
/// comment-preserving YAML editor is at hand). One line per file touched.
pub fn remove_locale(project_root: &Path, tag: &str) -> Result<Vec<String>, String> {
    let root = project_root.join("store");
    let mut done = Vec::new();
    if !root.is_dir() {
        return Ok(done);
    }
    for stem in ["storefront", "app"] {
        for ext in ["toml", "yaml", "yml"] {
            let file = format!("{stem}.{tag}.{ext}");
            let path = root.join(&file);
            if path.is_file() {
                std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                done.push(format!("removed store/{file}"));
            }
        }
    }
    let Some(main) = STOREFRONT_FILES.iter().find(|n| root.join(n).is_file()) else {
        return Ok(done);
    };
    let path = root.join(main);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if is_yaml(main) {
        let doc = parse_document(&text, true)?;
        let mentions = |v: &serde_json::Value| -> bool {
            v.get("metadata")
                .and_then(|m| m.get(tag))
                .is_some_and(|t| t.is_object())
        };
        let root_v = doc.get("storefront").cloned().unwrap_or_default();
        let inline = mentions(&root_v)
            || root_v.as_object().is_some_and(|targets| {
                targets.values().any(|te| {
                    mentions(te)
                        || te
                            .as_object()
                            .is_some_and(|stores| stores.values().any(mentions))
                })
            });
        if inline {
            return Err(format!(
                "store/{main} carries a `metadata.{tag}` table; remove it by hand (a YAML file \
                 is not rewritten, so its comments survive)"
            ));
        }
        return Ok(done);
    }
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("store/{main}: {e}"))?;
    let removed = doc
        .get_mut("storefront")
        .map(|sf| drop_metadata_locale(sf, tag, 0))
        .unwrap_or(0);
    if removed > 0 {
        std::fs::write(&path, doc.to_string()).map_err(|e| format!("{}: {e}", path.display()))?;
        done.push(format!(
            "removed {removed} metadata.{tag} table(s) from store/{main}"
        ));
    }
    Ok(done)
}

/// Remove `metadata.<tag>` under `item` and, down to the store level, under each table in it.
fn drop_metadata_locale(item: &mut toml_edit::Item, tag: &str, depth: usize) -> usize {
    let Some(table) = item.as_table_like_mut() else {
        return 0;
    };
    let mut removed = 0;
    if let Some(m) = table
        .get_mut("metadata")
        .and_then(|m| m.as_table_like_mut())
        && m.remove(tag).is_some()
    {
        removed += 1;
    }
    if depth < 2 {
        let keys: Vec<String> = table
            .iter()
            .map(|(k, _)| k.to_string())
            .filter(|k| k != "metadata" && k != "submission-info" && k != "screenshots")
            .collect();
        for key in keys {
            if let Some(child) = table.get_mut(&key) {
                removed += drop_metadata_locale(child, tag, depth + 1);
            }
        }
    }
    removed
}

/// The store targets a command acts on: the one named, or every target of the app that a store
/// in the rules stages.
fn store_targets(
    project: &Project,
    want: Option<&str>,
    rules: &StoreRules,
) -> Result<Vec<&'static crate::targets::Target>, CliError> {
    let targets: Vec<&'static crate::targets::Target> = match want {
        Some(name) => match crate::targets::find(name) {
            Some(t) if rules.is_store_target(t.name) => vec![t],
            Some(t) => {
                return Err(CliError::failure(format!(
                    "{}: no store the rules stage publishes this target",
                    t.name
                )));
            }
            None => {
                return Err(CliError::failure(format!("unknown target {name:?}")));
            }
        },
        None => project
            .manifest
            .app
            .targets
            .iter()
            .filter_map(|t| crate::targets::find(t))
            .filter(|t| rules.is_store_target(t.name))
            .collect(),
    };
    if targets.is_empty() {
        return Err(CliError::failure(
            "this app declares no target a store publishes",
        ));
    }
    Ok(targets)
}

/// `day store stage`: the listing, held to `day lint`'s store rules first. What a store would
/// refuse (lint's errors) is refused here; placeholder text is too, unless
/// `--allow-placeholders` says a scaffold's TODO goes up on purpose.
fn stage_cmd(
    project: &Project,
    want: Option<&str>,
    screenshots: Option<&ScreenshotSource>,
    rules: Option<&Path>,
    allow_placeholders: bool,
) -> Result<(), CliError> {
    let rules = StoreRules::load(project, rules).map_err(CliError::failure)?;
    let listing = read(project).map_err(CliError::failure)?;
    if listing.is_empty() {
        return Err(CliError::failure(
            "no listing text in store/ — run `day store init` first (or `day store migrate` \
             for a store/<locale>/ layout)",
        ));
    }
    let blocking: Vec<String> = lint(project, &listing, &rules)
        .into_iter()
        .filter(|p| {
            crate::lint::severity_of(p.code) == crate::lint::Severity::Error
                || (p.code == "day::lint::store-placeholder" && !allow_placeholders)
        })
        .map(|p| {
            format!(
                "{}: {}",
                p.code.trim_start_matches("day::lint::"),
                p.message
            )
        })
        .collect();
    if !blocking.is_empty() {
        return Err(CliError::failure(format!(
            "the stores would refuse this listing:\n  {}\n(`day lint` shows every finding; \
             --allow-placeholders stages TODO text on purpose)",
            blocking.join("\n  ")
        )));
    }
    for t in store_targets(project, want, &rules)? {
        let out = stage_dir(project, t);
        let files = stage(project, t, &listing, &out, screenshots, &rules)
            .map_err(|e| CliError::failure(format!("{}: {e}", t.name)))?;
        crate::ops::status(
            "Staged",
            &format!("{} ({} file(s)) for {}", out.display(), files.len(), t.name),
        );
    }
    Ok(())
}

/// `day store screenshots <index>`: what each store's listing would take from the index, and
/// what it would refuse. Fails on any refusal, so a release pipeline can check the published
/// gallery before it signs anything.
fn screenshots_cmd(
    project: &Project,
    want: Option<&str>,
    source: &ScreenshotSource,
    rules: Option<&Path>,
) -> Result<(), CliError> {
    let rules = StoreRules::load(project, rules).map_err(CliError::failure)?;
    let index = source.index().map_err(CliError::failure)?;
    let mut refused = 0;
    for t in store_targets(project, want, &rules)? {
        let shots = listing_shots(&index, t, &rules).map_err(CliError::failure)?;
        // One line per device kind: how many captures each locale contributes.
        let mut per_device: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
        let mut scaled: BTreeMap<&str, (u32, u32, u32)> = BTreeMap::new();
        for s in &shots {
            *per_device
                .entry(s.device.as_str())
                .or_default()
                .entry(s.locale.as_str())
                .or_default() += 1;
            if s.scale > 1 {
                scaled.insert(s.device.as_str(), (s.scale, s.width, s.height));
            }
        }
        let summary = if per_device.is_empty() {
            "nothing declared for it in store/storefront.toml [storefront…screenshots], or \
             nothing captured"
                .to_string()
        } else {
            per_device
                .iter()
                .map(|(device, locales)| {
                    let counts: Vec<String> =
                        locales.iter().map(|(l, n)| format!("{l} {n}")).collect();
                    match scaled.get(device) {
                        Some((k, w, h)) => {
                            format!("{device}: {} (scaled ×{k} to {w}×{h})", counts.join(", "))
                        }
                        None => format!("{device}: {}", counts.join(", ")),
                    }
                })
                .collect::<Vec<_>>()
                .join("; ")
        };
        crate::ops::status(
            "Listing",
            &format!(
                "{} ({}): {summary}",
                t.name,
                rules.store_for(t.name).map(|(k, _)| k).unwrap_or("?")
            ),
        );
        for p in screenshot_problems(&index, t, &rules) {
            eprintln!("error: {}: {p}", t.name);
            refused += 1;
        }
    }
    if refused > 0 {
        return Err(CliError::failure(format!(
            "{refused} screenshot problem(s); the stores would refuse the listing"
        )));
    }
    Ok(())
}

/// `day store <init|migrate|stage|screenshots|export>`.
pub fn run(project: &Project, cmd: &crate::cli::StoreCmd) -> Result<(), CliError> {
    match cmd {
        crate::cli::StoreCmd::Init => init(project),
        crate::cli::StoreCmd::Migrate { keep } => migrate(project, *keep),
        crate::cli::StoreCmd::Export { out } => export_cmd(project, out.as_deref()),
        crate::cli::StoreCmd::Stage {
            target,
            screenshots,
            rules,
            allow_placeholders,
        } => {
            let source = screenshots.as_deref().map(ScreenshotSource::parse);
            stage_cmd(
                project,
                target.as_deref(),
                source.as_ref(),
                rules.as_deref(),
                *allow_placeholders,
            )
        }
        crate::cli::StoreCmd::Screenshots {
            index,
            target,
            rules,
        } => screenshots_cmd(
            project,
            target.as_deref(),
            &ScreenshotSource::parse(index),
            rules.as_deref(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_refs(_: &str) -> Result<String, String> {
        Err("no such file".into())
    }

    /// A main-file source with no `-ref` files behind it.
    fn source() -> Source<'static> {
        Source {
            file: "storefront.toml",
            sidecar: None,
            default_locale: "en",
            read_ref: &no_refs,
        }
    }

    fn parse_src(doc: &serde_json::Value) -> Result<Storefront, String> {
        Storefront::parse(doc, &source())
    }

    /// A listing built in memory: the default locale's fields are the shared base table, every
    /// other locale a table over it.
    fn listing_of(default: &str, locales: &[(&str, &[(Field, &str)])]) -> Listing {
        let mut listing = Listing {
            default_locale: default.into(),
            ..Listing::default()
        };
        for (tag, fields) in locales {
            let map: BTreeMap<Field, Text> = fields
                .iter()
                .map(|(f, v)| {
                    (
                        *f,
                        Text {
                            value: v.to_string(),
                            origin: Origin::Inline {
                                file: "app.toml".into(),
                                table: "storefront.metadata".into(),
                                key: f.key(),
                            },
                        },
                    )
                })
                .collect();
            if *tag == default {
                listing.app.metadata.base = map;
            } else {
                listing.app.metadata.locales.insert(tag.to_string(), map);
            }
        }
        listing
    }

    /// The resolved text as `field: value` pairs, for terse assertions.
    fn values(fields: &BTreeMap<Field, Text>) -> Vec<(Field, String)> {
        fields.iter().map(|(f, t)| (*f, t.value.clone())).collect()
    }

    fn get(fields: &BTreeMap<Field, Text>, f: Field) -> Option<String> {
        fields.get(&f).map(|t| t.value.clone())
    }

    /// Keywords are held joined by commas with no spaces, the form the App Store counts;
    /// spacing and empty items in the source do not survive.
    #[test]
    fn keywords_join_without_spaces() {
        assert_eq!(join_keywords("a, b ,c".split(',')), "a,b,c");
        assert_eq!(join_keywords("a\n\nb\n".split('\n')), "a,b");
    }

    /// The rules the CLI ships: each store's spelling of a locale, its fields and their limits,
    /// which targets it stages, and the storefronts it knows by name alone.
    #[test]
    fn the_builtin_rules_describe_both_stores_and_their_disagreements() {
        let rules = StoreRules::builtin();
        let apple = &rules.0["apple-app-store"];
        let play = &rules.0["google-play-store"];
        assert_eq!(apple.locale("zh-CN"), Some("zh-Hans"));
        assert_eq!(play.locale("zh-CN"), Some("zh-CN"));
        // Google Play still spells Hebrew with the pre-1989 code.
        assert_eq!(play.locale("he"), Some("iw-IL"));
        assert_eq!(apple.locale("he"), Some("he"));
        assert_eq!(apple.locale("ar"), Some("ar-SA"));
        assert_eq!(play.locale("ar"), Some("ar"));
        assert_eq!(apple.locale("kl"), None, "unknown tag maps nowhere");
        assert!(mappable("fr") && !mappable("kl"));
        // Release notes: 4000 on the App Store, 500 on Play. An app shipping to both must fit 500.
        assert_eq!(
            apple.field(Field::ReleaseNotes).map(|f| f.limit),
            Some(4000)
        );
        assert_eq!(play.field(Field::ReleaseNotes).map(|f| f.limit), Some(500));
        // Fields only one store has.
        assert!(play.field(Field::Keywords).is_none());
        assert!(apple.field(Field::Short).is_none());
        assert_eq!(apple.name("apple-app-store"), "the App Store");
        assert_eq!(
            rules.store_for("ios-uikit").map(|(k, _)| k),
            Some("apple-app-store")
        );
        assert_eq!(
            rules.store_for("android-mdc").map(|(k, _)| k),
            Some("google-play-store")
        );
        assert!(
            rules.store_for("macos-appkit").is_none(),
            "known, not staged"
        );
        assert!(rules.known("mac-app-store") && !rules.known("apple-appstore"));
        assert_eq!(rules.capture_kind("ios-uikit", None), "iphone");
        assert_eq!(rules.capture_kind("android-mdc", Some("tablet")), "tablet");
        assert_eq!(rules.capture_kind("web-dom", None), "default");
        let lanes = rules.lanes(&["android-mdc".into(), "web-dom".into()]);
        assert_eq!(lanes.len(), 1);
        assert_eq!(
            (lanes[0].0, lanes[0].2.as_str()),
            ("google-play-store", "android-mdc")
        );
        assert_eq!(
            play.screenshots["tablet-7"].folder.as_deref(),
            Some("sevenInchScreenshots")
        );
        // What parse refuses: a layout nobody stages, a field no store has, a required field
        // the store does not list.
        for (src, expected) in [
            ("[x]\nlayout = \"ftp\"\n", "the layouts are"),
            (
                "[x]\n[x.fields]\ntitle = { file = \"t\", limit = 1 }\n",
                "not a listing field",
            ),
            ("[x]\nrequired = [\"name\"]\n", "required names"),
            ("[x]\n[x.screenshots.phone]\nheight = 3\n", "unknown field"),
        ] {
            let err = StoreRules::parse(src).expect_err(src);
            assert!(err.contains(expected), "{src}: {err}");
        }
    }

    /// An approved App Store version goes live on its own unless the listing says
    /// `apple-release = "manual"`, which staging hands the lanes as DAY_ASC_MANUAL_RELEASE in the
    /// tree's env file; the Fastfile itself reads that switch and is the same text either way.
    /// Any other value in the key is refused by name.
    #[test]
    fn apple_release_is_automatic_unless_the_listing_holds_it() {
        let tmp = std::env::temp_dir().join(format!("day-store-release-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).expect("mkdir");
        std::fs::write(
            tmp.join("Day.toml"),
            "schema = 1\n[app]\nid = \"dev.example.app\"\nbuild = 7\ntargets = [\"ios-uikit\"]\n",
        )
        .expect("Day.toml");
        std::fs::write(
            tmp.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"1.0.0\"\n",
        )
        .expect("Cargo.toml");
        let project = crate::meta::find_project(Some(&tmp)).expect("project");
        let rules = StoreRules::builtin();
        let ios = crate::targets::find("ios-uikit").expect("ios");
        let base = listing_of(
            "en",
            &[(
                "en",
                &[
                    (Field::Name, "Example"),
                    (Field::Description, "What it does."),
                    (Field::ReleaseNotes, "First release."),
                ],
            )],
        );
        for (src, held) in [
            (
                "[storefront.ios-uikit.apple-app-store.submission-info]\napple-release = \"manual\"\n",
                true,
            ),
            (
                "[storefront.submission-info]\napple-release = \"automatic\"\n",
                false,
            ),
            (
                "[storefront.submission-info]\ncopyright = \"2026 Example\"\n",
                false,
            ),
        ] {
            let mut listing = base.clone();
            listing.app = parse_src(&parse_document(src, false).expect("toml")).expect("parse");
            let out = tmp.join(if held { "held" } else { "auto" });
            let _ = std::fs::remove_dir_all(&out);
            stage(&project, ios, &listing, &out, None, rules).expect("stage ios");
            let env = std::fs::read_to_string(out.join("fastlane/.env.default")).expect("env");
            assert_eq!(env.contains("DAY_ASC_MANUAL_RELEASE=1"), held, "{src}");
            let fastfile =
                std::fs::read_to_string(out.join("fastlane/Fastfile")).expect("Fastfile");
            assert!(
                fastfile.contains("automatic_release: ENV[\"DAY_ASC_MANUAL_RELEASE\"].to_s.empty?"),
                "the lanes read the switch"
            );
            assert!(!fastfile.contains("automatic_release: false,\n      submission_information"));
        }
        let err = parse_src(
            &parse_document(
                "[storefront.submission-info]\napple-release = \"later\"\n",
                false,
            )
            .expect("toml"),
        )
        .expect_err("a value that is neither");
        assert!(err.contains("apple-release"), "{err}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The generated tree has to be one fastlane accepts unchanged: the tool locates its config by
    /// finding a `fastlane` folder, each store's own file names differ from Day's, and Play keys the
    /// changelog by versionCode.
    #[test]
    fn staging_writes_each_store_its_own_layout() {
        let tmp = std::env::temp_dir().join(format!("day-store-stage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).expect("mkdir");
        std::fs::write(
            tmp.join("Day.toml"),
            "schema = 1\n[app]\nid = \"dev.example.app\"\nbuild = 7\ntargets = [\"ios-uikit\"]\n",
        )
        .expect("Day.toml");
        std::fs::write(
            tmp.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"1.0.0\"\n",
        )
        .expect("Cargo.toml");
        let project = crate::meta::find_project(Some(&tmp)).expect("project");
        let rules = StoreRules::builtin();

        let listing = listing_of(
            "en",
            &[(
                "zh-CN",
                &[
                    (Field::Name, "Example"),
                    (Field::Description, "What it does."),
                    (Field::Short, "One line."),
                    (Field::ReleaseNotes, "First release."),
                ],
            )],
        );

        let ios = crate::targets::find("ios-uikit").expect("ios");
        let out = tmp.join("out-ios");
        stage(&project, ios, &listing, &out, None, rules).expect("stage ios");
        // Apple: `zh-Hans`, deliver's file names, and no short description (it has no such field).
        assert!(out.join("fastlane/metadata/zh-Hans/name.txt").is_file());
        assert!(
            out.join("fastlane/metadata/zh-Hans/description.txt")
                .is_file()
        );
        assert!(
            out.join("fastlane/metadata/zh-Hans/release_notes.txt")
                .is_file()
        );
        assert!(
            !out.join("fastlane/metadata/zh-Hans/short_description.txt")
                .exists()
        );
        assert!(
            out.join("fastlane/Fastfile").is_file(),
            "fastlane finds config by folder"
        );
        // Read before fastlane starts its analytics session, which is the only moment early
        // enough to stop one.
        assert!(
            std::fs::read_to_string(out.join("fastlane/.env.default"))
                .expect("dotenv")
                .contains("FASTLANE_OPT_OUT_USAGE=1")
        );

        let android = crate::targets::find("android-mdc").expect("android");
        let out = tmp.join("out-android");
        stage(&project, android, &listing, &out, None, rules).expect("stage android");
        // Google: `zh-CN`, supply's names, and the changelog keyed by versionCode (= [app] build).
        assert!(
            out.join("fastlane/metadata/android/zh-CN/title.txt")
                .is_file()
        );
        assert!(
            out.join("fastlane/metadata/android/zh-CN/full_description.txt")
                .is_file()
        );
        assert!(
            out.join("fastlane/metadata/android/zh-CN/short_description.txt")
                .is_file()
        );
        assert!(
            out.join("fastlane/metadata/android/zh-CN/changelogs/7.txt")
                .is_file(),
            "the changelog is keyed by versionCode"
        );
        assert!(
            !out.join("fastlane/metadata/android/zh-CN/keywords.txt")
                .exists()
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Play refuses a package name with a hyphen, which is why an app whose bundle id has one
    /// states an `[app.android] id`. The staged Appfile has to carry the id the store it is for
    /// knows, or `supply` stops with "Invalid package name".
    #[test]
    fn the_staged_id_is_the_one_each_store_knows() {
        let tmp = std::env::temp_dir().join(format!("day-store-id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).expect("mkdir");
        std::fs::write(
            tmp.join("Day.toml"),
            "schema = 1\n[app]\nid = \"dev.example.app-x\"\nbuild = 7\n\
             targets = [\"ios-uikit\", \"android-mdc\"]\n\
             [app.android]\nid = \"dev.example.app_x\"\n",
        )
        .expect("Day.toml");
        std::fs::write(
            tmp.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"1.0.0\"\n",
        )
        .expect("Cargo.toml");
        let project = crate::meta::find_project(Some(&tmp)).expect("project");
        let rules = StoreRules::builtin();
        let listing = listing_of("en", &[("en", &[(Field::Name, "Example")])]);

        let android = crate::targets::find("android-mdc").expect("android");
        stage(
            &project,
            android,
            &listing,
            &tmp.join("out-android"),
            None,
            rules,
        )
        .expect("stage android");
        let play = std::fs::read_to_string(tmp.join("out-android/fastlane/Appfile")).expect("read");
        assert!(
            play.contains("package_name(\"dev.example.app_x\")"),
            "Play takes the resolved android id: {play}"
        );

        let ios = crate::targets::find("ios-uikit").expect("ios");
        stage(&project, ios, &listing, &tmp.join("out-ios"), None, rules).expect("stage ios");
        let apple = std::fs::read_to_string(tmp.join("out-ios/fastlane/Appfile")).expect("read");
        assert!(
            apple.contains("app_identifier(\"dev.example.app-x\")"),
            "the App Store keeps the bundle id: {apple}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The listing's screenshots land where each store's fastlane tool reads them, in listing
    /// order, one set per locale, the light theme only, from an index `day screenshot index`
    /// wrote beside its capture tree.
    #[test]
    fn staging_places_the_listing_screenshots_for_each_store() {
        let tmp = std::env::temp_dir().join(format!("day-store-shots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).expect("mkdir");
        std::fs::write(
            tmp.join("Day.toml"),
            "schema = 1\n[app]\nid = \"dev.example.app\"\nbuild = 7\n\
             targets = [\"ios-uikit\", \"android-mdc\"]\n",
        )
        .expect("Day.toml");
        std::fs::write(
            tmp.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"1.0.0\"\n",
        )
        .expect("Cargo.toml");
        let project = crate::meta::find_project(Some(&tmp)).expect("project");
        let listing = listing_of(
            "en",
            &[
                ("en", &[(Field::Name, "Example")]),
                ("fr", &[(Field::Name, "Exemple")]),
            ],
        );

        // A capture tree as the runner leaves it, and the index over it.
        let tree = tmp.join("screenshots");
        let mut entries = Vec::new();
        let captures = [
            ("ios-uikit", "iphone", "light", "en", "home", Some(1)),
            ("ios-uikit", "iphone", "light", "en", "play", Some(2)),
            ("ios-uikit", "iphone", "light", "en", "debug", None),
            ("ios-uikit", "iphone", "dark", "en", "home", None),
            ("ios-uikit", "iphone", "light", "fr", "home", Some(1)),
            ("ios-uikit", "ipad", "light", "en", "home", Some(1)),
            ("ios-uikit", "ipad", "light", "fr", "home", Some(1)),
            ("android-mdc", "phone", "light", "en", "home", Some(1)),
            ("android-mdc", "phone", "light", "fr", "home", Some(1)),
            ("android-mdc", "tablet", "light", "en", "home", Some(1)),
        ];
        for (target, device, theme, locale, shot, store) in captures {
            let variant = if locale == "en" {
                theme.to_string()
            } else {
                format!("{theme}-{locale}")
            };
            let rel = format!("{target}/{device}/{variant}/{shot}.png");
            let path = tree.join(&rel);
            std::fs::create_dir_all(path.parent().expect("dir")).expect("mkdir");
            std::fs::write(&path, rel.as_bytes()).expect("capture");
            // A store size for each kind, so the rules let the set through.
            let (w, h) = match device {
                "iphone" => (1320, 2868),
                "ipad" => (2752, 2064),
                "tablet" => (2560, 1600),
                _ => (1080, 1920),
            };
            entries.push(serde_json::json!({
                "path": format!("gallery/{rel}"),
                "shot": shot, "device": device, "os": target.split('-').next(), "platform": target,
                "theme": theme, "locale": locale, "store": store, "width": w, "height": h,
            }));
        }
        let index_path = tree.join("gallery.json");
        std::fs::write(&index_path, index(&["en", "fr"], entries).to_string()).expect("index");
        let index = index_path;
        let source = ScreenshotSource::parse(index.to_str().expect("utf-8"));

        let ios = crate::targets::find("ios-uikit").expect("ios");
        let out = tmp.join("out-ios");
        let rules = StoreRules::parse(DEFAULT_RULES).expect("rules");
        stage(&project, ios, &listing, &out, Some(&source), &rules).expect("stage ios");
        let placed = |rel: &str| std::fs::read_to_string(out.join(rel)).ok();
        assert_eq!(
            placed("fastlane/screenshots/en-US/01-iphone-home.png").as_deref(),
            Some("ios-uikit/iphone/light/home.png"),
            "the capture's own bytes, from the tree beside the index"
        );
        assert!(placed("fastlane/screenshots/en-US/02-iphone-play.png").is_some());
        assert!(placed("fastlane/screenshots/en-US/01-ipad-home.png").is_some());
        assert!(placed("fastlane/screenshots/fr-FR/01-iphone-home.png").is_some());
        let apple: Vec<String> = walk(&out.join("fastlane/screenshots"));
        assert_eq!(
            apple.len(),
            5,
            "no unmarked step, no dark theme, no Android: {apple:?}"
        );
        let fastfile = std::fs::read_to_string(out.join("fastlane/Fastfile")).expect("Fastfile");
        assert!(
            fastfile.contains("skip_screenshots: false")
                && fastfile.contains("overwrite_screenshots: true"),
            "deliver uploads what was staged: {fastfile}"
        );

        let android = crate::targets::find("android-mdc").expect("android");
        let out = tmp.join("out-android");
        stage(&project, android, &listing, &out, Some(&source), &rules).expect("stage android");
        let placed = |rel: &str| out.join(rel).is_file();
        assert!(placed(
            "fastlane/metadata/android/en-US/images/phoneScreenshots/01-home.png"
        ));
        assert!(placed(
            "fastlane/metadata/android/en-US/images/tenInchScreenshots/01-home.png"
        ));
        let fastfile = std::fs::read_to_string(out.join("fastlane/Fastfile")).expect("Fastfile");
        assert!(
            fastfile.contains("skip_upload_screenshots: false"),
            "{fastfile}"
        );

        // Without a source the Fastfiles keep the stores' current screenshots.
        let out = tmp.join("out-none");
        stage(&project, android, &listing, &out, None, &rules).expect("stage android");
        let fastfile = std::fs::read_to_string(out.join("fastlane/Fastfile")).expect("Fastfile");
        assert!(
            fastfile.contains("skip_upload_screenshots: true"),
            "{fastfile}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// An index entry as `day screenshot index` writes it, marked for the listing.
    fn marked(
        os: &str,
        device: &str,
        locale: &str,
        shot: &str,
        w: u32,
        h: u32,
    ) -> serde_json::Value {
        let platform = match os {
            "ios" => "ios-uikit",
            "android" => "android-mdc",
            other => other,
        };
        serde_json::json!({
            "shot": shot, "device": device, "os": os, "platform": platform, "theme": "light",
            "locale": locale, "store": 1, "width": w, "height": h,
            "path": format!("gallery/x/{shot}.png"),
        })
    }

    /// An index whose `listings` are derived from a test-only `store` position on each entry:
    /// per platform and device kind, the shots in position order, in the entry's theme. What
    /// `day screenshot index` writes from `store/storefront.toml`, built here from the captures.
    fn index(locales: &[&str], entries: Vec<serde_json::Value>) -> serde_json::Value {
        // (platform, kind) → (position, shot, theme), the shape the derived listing needs.
        #[allow(clippy::type_complexity)]
        let mut per: BTreeMap<(String, String), Vec<(u64, String, String)>> = BTreeMap::new();
        for e in &entries {
            let Some(pos) = e["store"].as_u64() else {
                continue;
            };
            let platform = e["platform"].as_str().unwrap_or_default().to_string();
            let kind = e["device"].as_str().unwrap_or("default").to_string();
            let list = per.entry((platform, kind)).or_default();
            let item = (
                pos,
                e["shot"].as_str().unwrap_or_default().to_string(),
                e["theme"].as_str().unwrap_or("light").to_string(),
            );
            if !list.contains(&item) {
                list.push(item);
            }
        }
        let mut listings = serde_json::Map::new();
        for ((platform, kind), mut list) in per {
            list.sort();
            let store = match platform.as_str() {
                "ios-uikit" => "apple-app-store",
                _ => "google-play-store",
            };
            let items: Vec<serde_json::Value> = list
                .into_iter()
                .map(|(_, shot, theme)| serde_json::json!({ "shot": shot, "theme": theme }))
                .collect();
            let target = listings
                .entry(platform)
                .or_insert_with(|| serde_json::json!({ "default": [], "stores": {} }));
            // The same list for every locale the index carries, as the real index resolves it.
            for locale in locales {
                target["stores"][store][&kind][*locale] = serde_json::Value::Array(items.clone());
            }
        }
        serde_json::json!({ "locales": locales, "screenshots": entries, "listings": listings })
    }

    /// The rules are the stores': Apple takes exact sizes for its two device kinds, Google a
    /// range whose long side is at most twice the short, and each required kind needs a capture
    /// in every locale the index carries.
    #[test]
    fn the_stores_rules_refuse_what_the_stores_refuse() {
        let ios = crate::targets::find("ios-uikit").expect("ios");
        let android = crate::targets::find("android-mdc").expect("android");
        let rules = StoreRules::parse(DEFAULT_RULES).expect("the CLI's own rules parse");
        let ok = |t, i: &serde_json::Value| screenshot_problems(i, t, &rules);

        let good = index(
            &["en", "fr"],
            vec![
                marked("ios", "iphone", "en", "home", 1320, 2868),
                marked("ios", "iphone", "fr", "home", 1320, 2868),
                marked("ios", "ipad", "en", "home", 2752, 2064),
                marked("ios", "ipad", "fr", "home", 2752, 2064),
                marked("android", "phone", "en", "home", 1080, 1920),
                marked("android", "phone", "fr", "home", 1080, 1920),
                marked("android", "tablet", "en", "home", 2560, 1600),
            ],
        );
        assert_eq!(ok(ios, &good), Vec::<String>::new());
        assert_eq!(
            ok(android, &good),
            Vec::<String>::new(),
            "a tablet set is optional"
        );

        // The 20:9 `medium_phone` profile is within the API's 2.3:1; a taller one is not.
        let tall = index(
            &["en"],
            vec![marked("android", "phone", "en", "home", 1080, 2400)],
        );
        assert_eq!(ok(android, &tall), Vec::<String>::new());
        let taller = index(
            &["en"],
            vec![marked("android", "phone", "en", "home", 1080, 2500)],
        );
        let problems = ok(android, &taller);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("2.31:1"), "{problems:?}");
        // The halved CI tablet: 800 px on the short side, under the API's 1080 floor. The
        // shipped rules let `day store stage` scale it up (×2, to 2560×1600), so it passes;
        // rules without `upscale` refuse it and say why.
        let halved = index(
            &["en"],
            vec![
                marked("android", "phone", "en", "home", 1080, 1920),
                marked("android", "tablet", "en", "home", 1280, 800),
            ],
        );
        assert_eq!(ok(android, &halved), Vec::<String>::new());
        let mut strict = rules.clone();
        strict
            .0
            .get_mut("google-play-store")
            .expect("play")
            .screenshots
            .get_mut("tablet")
            .expect("tablet")
            .upscale = false;
        let problems = screenshot_problems(&halved, android, &strict);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("1280×800") && problems[0].contains("upscale = true"),
            "{problems:?}"
        );
        // The factor is the smallest whole one that clears the floor, and never past the
        // long side's ceiling.
        let play = &rules.0["google-play-store"].screenshots["tablet"];
        assert_eq!(play.scale_for(1280, 800), 2);
        assert_eq!(play.scale_for(1920, 1200), 1);
        assert_eq!(play.scale_for(700, 500), 3);
        assert_eq!(
            play.scale_for(4000, 500),
            1,
            "×3 would pass 7680 on the long side"
        );
        // What the runner tablet captures instead.
        let seven = index(
            &["en"],
            vec![
                marked("android", "phone", "en", "home", 1080, 1920),
                marked("android", "tablet", "en", "home", 1920, 1200),
            ],
        );
        assert_eq!(ok(android, &seven), Vec::<String>::new());

        // An iPhone capture from a profile that is not a 6.9" one.
        let small = index(
            &["en"],
            vec![
                marked("ios", "iphone", "en", "home", 1179, 2556),
                marked("ios", "ipad", "en", "home", 2752, 2064),
            ],
        );
        let problems = ok(ios, &small);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("1179×2556") && problems[0].contains("1320×2868"));

        // Marks on the iPhone only: the iPad is required too.
        let no_ipad = index(
            &["en"],
            vec![marked("ios", "iphone", "en", "home", 1320, 2868)],
        );
        let problems = ok(ios, &no_ipad);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("needs ipad screenshots"),
            "{problems:?}"
        );

        // A locale the index carries with no capture on a required kind; one the store does
        // not know is not asked for.
        let partial = index(
            &["en", "fr", "kl"],
            vec![
                marked("ios", "iphone", "en", "home", 1320, 2868),
                marked("ios", "ipad", "en", "home", 2752, 2064),
                marked("ios", "ipad", "fr", "home", 2752, 2064),
            ],
        );
        let problems = ok(ios, &partial);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("no iphone screenshots for fr"),
            "{problems:?}"
        );

        // Over the ceiling.
        let mut many: Vec<serde_json::Value> = (0..9)
            .map(|n| {
                let mut e = marked("android", "phone", "en", &format!("s{n}"), 1080, 1920);
                e["store"] = serde_json::json!(n + 1);
                e
            })
            .collect();
        many.push(marked("android", "tablet", "en", "home", 2560, 1600));
        let problems = ok(android, &index(&["en"], many));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("9 phone screenshots") && problems[0].contains("at most 8"));

        // Nothing declared at all, and a device slug the store has no kind for.
        let none = index(&["en"], vec![]);
        let problems = ok(android, &none);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("[storefront.android-mdc.google-play-store.screenshots]"),
            "{problems:?}"
        );
        let odd = index(
            &["en"],
            vec![
                marked("android", "phone", "en", "home", 1080, 1920),
                marked("android", "watch", "en", "home", 400, 400),
            ],
        );
        let problems = ok(android, &odd);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("\"watch\""), "{problems:?}");

        // Only the declared theme counts: a dark capture at a refused size is not looked at
        // when the listing takes the light one, and a listing that takes the dark one sees it.
        let mut dark = marked("android", "phone", "en", "home", 1080, 2500);
        dark["theme"] = serde_json::json!("dark");
        dark.as_object_mut().expect("entry").remove("store");
        let themed = index(
            &["en"],
            vec![
                marked("android", "phone", "en", "home", 1080, 1920),
                dark.clone(),
            ],
        );
        assert_eq!(ok(android, &themed), Vec::<String>::new());
        dark["store"] = serde_json::json!(1);
        let mut light = marked("android", "phone", "en", "home", 1080, 1920);
        light.as_object_mut().expect("entry").remove("store");
        let themed_dark = index(&["en"], vec![light, dark]);
        assert_eq!(
            ok(android, &themed_dark).len(),
            1,
            "the dark capture is the listing's now"
        );
    }

    /// Every file under `dir`, as paths relative to it.
    fn walk(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = e.file_name().to_string_lossy().into_owned();
                out.extend(walk(&p).into_iter().map(|f| format!("{name}/{f}")));
            } else {
                out.push(e.file_name().to_string_lossy().into_owned());
            }
        }
        out
    }

    /// The same declaration in both formats, as one parsed storefront each.
    fn both(toml_src: &str, yaml_src: &str) -> (Storefront, Storefront) {
        let from_toml =
            parse_src(&parse_document(toml_src, false).expect("toml")).expect("toml storefront");
        let from_yaml =
            parse_src(&parse_document(yaml_src, true).expect("yaml")).expect("yaml storefront");
        (from_toml, from_yaml)
    }

    /// Everything a storefront resolves to, as one comparable value: the submission info for
    /// every target and store named (plus a store nobody declared, for the fallbacks) and the
    /// website and store lists for every device kind.
    fn resolved(s: &Storefront) -> serde_json::Value {
        let kinds = ["iphone", "ipad", "phone", "tablet", "default"];
        let locales = ["en", "fr", "fr-CA", "zh-CN", "ar"];
        let text = |target: &str, store: &str| -> serde_json::Value {
            let mut out = serde_json::Map::new();
            for locale in locales {
                out.insert(
                    locale.to_string(),
                    fields_json(&s.metadata(target, store, locale)),
                );
            }
            serde_json::Value::Object(out)
        };
        let mut targets = serde_json::Map::new();
        for (target, te) in &s.targets {
            let mut stores = serde_json::Map::new();
            let mut names: Vec<String> = te.stores.keys().cloned().collect();
            names.push("nobody".to_string());
            for store in names {
                let mut per_kind = serde_json::Map::new();
                for kind in kinds {
                    for locale in locales {
                        per_kind.insert(
                            format!("{kind}/{locale}"),
                            serde_json::to_value(s.store_kind(target, &store, kind, locale))
                                .unwrap(),
                        );
                    }
                }
                stores.insert(
                    store.clone(),
                    serde_json::json!({
                        "submission-info": s.submission(target, &store),
                        "metadata": text(target, &store),
                        "screenshots": per_kind,
                    }),
                );
            }
            let mut website = serde_json::Map::new();
            for kind in kinds {
                for locale in locales {
                    website.insert(
                        format!("{kind}/{locale}"),
                        serde_json::to_value(s.website(target, kind, locale)).unwrap(),
                    );
                }
            }
            targets.insert(
                target.clone(),
                serde_json::json!({
                    "website": website,
                    "metadata": text(target, ""),
                    "stores": stores,
                }),
            );
        }
        serde_json::json!({
            "shared": s.info,
            "text": text("", ""),
            "text-elsewhere": text("web-dom", "none"),
            "legacy": s.legacy_keys,
            "targets": targets,
            "shots": s.shot_names(),
        })
    }

    fn names(v: Vec<ListingShot>) -> Vec<String> {
        v.into_iter()
            .map(|s| format!("{}:{}", s.shot, s.theme))
            .collect()
    }

    const FULL_TOML: &str = r#"
[storefront.submission-info]
copyright = "2026 Example"
contact-email = "support@example.com"
review-notes = """
Line one.

Line three, after a blank.
"""

[storefront.metadata]
name = "Example"
short = "One line."
description = """
What it does.

In two paragraphs.
"""
keywords = ["rust", "native"]
privacy-url = "https://example.com/privacy"

[storefront.metadata.fr]
name = "Exemple"
short = "Une ligne."

[storefront.ios-uikit.submission-info]
contact-email = "ios@example.com"

[storefront.ios-uikit.metadata]
subtitle = "Native UI"

[storefront.ios-uikit.screenshots]
default = ["home", { name = "canvas" }]
ipad = ["home", "canvas", "grid"]

[storefront.ios-uikit.screenshots.fr]
default = ["localization", "home"]

[storefront.ios-uikit.apple-app-store.submission-info]
apple-category = "DEVELOPER_TOOLS"
bundle-id = "com.example.store"

[storefront.ios-uikit.apple-app-store.metadata]
subtitle = "App Store subtitle"
promo = "Promo"

[storefront.ios-uikit.apple-app-store.metadata.fr]
promo = "Promo FR"

[storefront.ios-uikit.apple-app-store.screenshots]
iphone = [{ name = "controls", theme = "dark" }, "layout"]

[storefront.ios-uikit.apple-app-store.screenshots.zh-CN]
iphone = ["text", "layout"]

[storefront.ios-uikit.altstore.screenshots]
default = ["map"]

[storefront.android-mdc.google-play-store.submission-info]
review-notes = "Play only."

[storefront.android-mdc.google-play-store.metadata.zh-CN]
short = "一行。"

[storefront.android-mdc.google-play-store.screenshots]
tablet = ["grid"]

[storefront.macos-appkit.screenshots]
default = ["home"]
"#;

    const FULL_YAML: &str = r#"
storefront:
  submission-info:
    copyright: "2026 Example"
    contact-email: support@example.com
    review-notes: |
      Line one.

      Line three, after a blank.
  metadata:
    name: Example
    short: One line.
    description: |-
      What it does.

      In two paragraphs.
    keywords: [rust, native]
    privacy-url: https://example.com/privacy
    fr:
      name: Exemple
      short: Une ligne.
  ios-uikit:
    submission-info:
      contact-email: ios@example.com
    metadata:
      subtitle: Native UI
    screenshots:
      default: [home, { name: canvas }]
      ipad: [home, canvas, grid]
      fr:
        default: [localization, home]
    apple-app-store:
      submission-info:
        apple-category: DEVELOPER_TOOLS
        bundle-id: com.example.store
      metadata:
        subtitle: App Store subtitle
        promo: Promo
        fr: { promo: Promo FR }
      screenshots:
        iphone:
          - { name: controls, theme: dark }
          - layout
        zh-CN:
          iphone: [text, layout]
    altstore:
      screenshots:
        default: [map]
  android-mdc:
    google-play-store:
      submission-info:
        review-notes: Play only.
      metadata:
        zh-CN: { short: 一行。 }
      screenshots:
        tablet: [grid]
  macos-appkit:
    screenshots:
      default: [home]
"#;

    /// One declaration, two files: every resolution comes out the same, and each one is what
    /// the fallback rules say. The block scalar and the multi-line string agree byte for byte.
    #[test]
    fn toml_and_yaml_resolve_identically() {
        let (from_toml, from_yaml) = both(FULL_TOML, FULL_YAML);
        assert_eq!(resolved(&from_toml), resolved(&from_yaml));
        for s in [&from_toml, &from_yaml] {
            assert!(s.legacy_keys.is_empty());
            // Submission info: store over target over shared, key by key.
            let apple = s.submission("ios-uikit", "apple-app-store");
            assert_eq!(apple.apple_category.as_deref(), Some("DEVELOPER_TOOLS"));
            assert_eq!(apple.bundle_id.as_deref(), Some("com.example.store"));
            assert_eq!(apple.contact_email.as_deref(), Some("ios@example.com"));
            assert_eq!(apple.copyright.as_deref(), Some("2026 Example"));
            assert_eq!(
                apple.review_notes.as_deref(),
                Some("Line one.\n\nLine three, after a blank.")
            );
            let alt = s.submission("ios-uikit", "altstore");
            assert_eq!(
                alt.apple_category, None,
                "another store of the target sees no Apple category"
            );
            assert_eq!(alt.contact_email.as_deref(), Some("ios@example.com"));
            let play = s.submission("android-mdc", "google-play-store");
            assert_eq!(play.review_notes.as_deref(), Some("Play only."));
            assert_eq!(play.contact_email.as_deref(), Some("support@example.com"));
            assert_eq!(play.bundle_id, None);
            let mac = s.submission("macos-appkit", "mac-app-store");
            assert_eq!(
                mac, s.info,
                "a target with no info of its own is the shared info"
            );
            assert_eq!(s.submission("web-dom", "none"), s.info);
            // Text: the locale's tables at every level, nearest first, then the default
            // locale's the same way.
            let apple_en = s.metadata("ios-uikit", "apple-app-store", "en");
            assert_eq!(get(&apple_en, Field::Name).as_deref(), Some("Example"));
            assert_eq!(
                get(&apple_en, Field::Subtitle).as_deref(),
                Some("App Store subtitle"),
                "the store's wording beats the target's"
            );
            assert_eq!(get(&apple_en, Field::Promo).as_deref(), Some("Promo"));
            assert_eq!(
                get(&apple_en, Field::Description).as_deref(),
                Some("What it does.\n\nIn two paragraphs."),
                "a multi-line string and a block scalar agree"
            );
            assert_eq!(
                get(&apple_en, Field::Keywords).as_deref(),
                Some("rust,native")
            );
            let apple_fr = s.metadata("ios-uikit", "apple-app-store", "fr");
            assert_eq!(get(&apple_fr, Field::Name).as_deref(), Some("Exemple"));
            assert_eq!(get(&apple_fr, Field::Short).as_deref(), Some("Une ligne."));
            assert_eq!(
                get(&apple_fr, Field::Promo).as_deref(),
                Some("Promo FR"),
                "the store's French table"
            );
            assert_eq!(
                get(&apple_fr, Field::Subtitle).as_deref(),
                Some("App Store subtitle"),
                "no French subtitle anywhere: the store's default-locale one"
            );
            assert_eq!(
                get(&apple_fr, Field::Description).as_deref(),
                Some("What it does.\n\nIn two paragraphs."),
                "French inherits the shared description"
            );
            let alt_fr = s.metadata("ios-uikit", "altstore", "fr");
            assert_eq!(
                get(&alt_fr, Field::Subtitle).as_deref(),
                Some("Native UI"),
                "another store of the target: the target's subtitle"
            );
            assert_eq!(get(&alt_fr, Field::Promo), None);
            let play_zh = s.metadata("android-mdc", "google-play-store", "zh-CN");
            assert_eq!(get(&play_zh, Field::Short).as_deref(), Some("一行。"));
            assert_eq!(get(&play_zh, Field::Name).as_deref(), Some("Example"));
            let play_frca = s.metadata("android-mdc", "google-play-store", "fr-CA");
            assert_eq!(
                get(&play_frca, Field::Short).as_deref(),
                Some("Une ligne."),
                "fr covers fr-CA"
            );
            assert_eq!(
                get(&s.metadata("web-dom", "none", "en"), Field::Subtitle),
                None,
                "a target with no text of its own sees the shared text alone"
            );
            assert_eq!(
                s.metadata_locales().into_iter().collect::<Vec<_>>(),
                ["fr", "zh-CN"]
            );
            assert!(s.has_metadata());
            assert_eq!(s.texts().len(), 12);
            // Screenshots: store kind → store default → target kind → target default → none,
            // the locale's lists before the general ones at each level.
            assert_eq!(
                names(s.store_kind("ios-uikit", "apple-app-store", "iphone", "en")),
                ["controls:dark", "layout:light"]
            );
            assert_eq!(
                names(s.store_kind("ios-uikit", "apple-app-store", "ipad", "en")),
                ["home:light", "canvas:light", "grid:light"]
            );
            assert_eq!(
                names(s.store_kind("ios-uikit", "apple-app-store", "iphone", "zh-CN")),
                ["text:light", "layout:light"]
            );
            assert_eq!(
                names(s.store_kind("ios-uikit", "apple-app-store", "ipad", "zh-CN")),
                ["home:light", "canvas:light", "grid:light"],
                "the locale table names iphone alone"
            );
            assert_eq!(
                names(s.store_kind("ios-uikit", "apple-app-store", "iphone", "fr")),
                ["localization:light", "home:light"],
                "the target's French list beats the store's general one: locale first"
            );
            assert_eq!(
                names(s.store_kind("ios-uikit", "apple-app-store", "ipad", "fr")),
                ["localization:light", "home:light"],
                "no store list for ipad: the target's French default"
            );
            assert_eq!(
                names(s.store_kind("ios-uikit", "altstore", "ipad", "en")),
                ["map:light"]
            );
            assert_eq!(
                names(s.store_kind("ios-uikit", "altstore", "iphone", "fr-CA")),
                ["localization:light", "home:light"],
                "fr covers fr-CA, and the target's French list beats the store's general one"
            );
            assert_eq!(
                names(s.store_kind("ios-uikit", "nobody", "iphone", "en")),
                ["home:light", "canvas:light"]
            );
            assert_eq!(
                names(s.store_kind("ios-uikit", "nobody", "iphone", "fr-CA")),
                ["localization:light", "home:light"],
                "fr covers fr-CA"
            );
            assert_eq!(
                names(s.store_kind("ios-uikit", "nobody", "ipad", "en")),
                ["home:light", "canvas:light", "grid:light"]
            );
            assert_eq!(
                names(s.store_kind("android-mdc", "google-play-store", "tablet", "en")),
                ["grid:light"]
            );
            assert_eq!(
                names(s.store_kind("android-mdc", "google-play-store", "phone", "en")),
                Vec::<String>::new()
            );
            assert_eq!(
                names(s.store_kind("macos-appkit", "mac-app-store", "default", "ar")),
                ["home:light"]
            );
            assert_eq!(
                names(s.store_kind("web-dom", "none", "default", "en")),
                Vec::<String>::new()
            );
            // The website's rows.
            assert_eq!(
                names(s.website("ios-uikit", "iphone", "en")),
                ["home:light", "canvas:light"]
            );
            assert_eq!(
                names(s.website("ios-uikit", "ipad", "en")),
                ["home:light", "canvas:light", "grid:light"]
            );
            assert_eq!(
                names(s.website("ios-uikit", "iphone", "fr")),
                ["localization:light", "home:light"]
            );
            assert_eq!(
                names(s.website("ios-uikit", "ipad", "fr")),
                ["localization:light", "home:light"],
                "a locale's default covers its unnamed kinds too"
            );
            assert_eq!(
                names(s.website("android-mdc", "phone", "en")),
                Vec::<String>::new()
            );
            assert_eq!(s.stores("ios-uikit"), ["altstore", "apple-app-store"]);
            assert_eq!(
                s.declared_kinds("ios-uikit", "apple-app-store"),
                ["ipad", "iphone"]
            );
            assert_eq!(
                s.declared_locales("ios-uikit", "apple-app-store"),
                ["fr", "zh-CN"]
            );
            assert_eq!(s.declared_locales("ios-uikit", "altstore"), ["fr"]);
            assert_eq!(
                s.declared_kinds("android-mdc", "google-play-store"),
                ["tablet"]
            );
            assert_eq!(s.shot_names().len(), 8);
            assert!(s.has_screenshots());
        }
    }

    /// Each fallback step on its own, in both formats: a store list beats a store default beats
    /// the target's kind beats the target's default.
    #[test]
    fn each_screenshot_fallback_step_holds_in_both_formats() {
        let cases: [(&str, &str, &[&str]); 5] = [
            (
                "[storefront.t.screenshots]\ndefault = [\"a\"]\niphone = [\"b\"]\n[storefront.t.s.screenshots]\ndefault = [\"c\"]\niphone = [\"d\"]\n",
                "storefront:\n  t:\n    screenshots: { default: [a], iphone: [b] }\n    s:\n      screenshots: { default: [c], iphone: [d] }\n",
                &["d:light"],
            ),
            (
                "[storefront.t.screenshots]\ndefault = [\"a\"]\niphone = [\"b\"]\n[storefront.t.s.screenshots]\ndefault = [\"c\"]\n",
                "storefront:\n  t:\n    screenshots: { default: [a], iphone: [b] }\n    s:\n      screenshots: { default: [c] }\n",
                &["c:light"],
            ),
            (
                "[storefront.t.screenshots]\ndefault = [\"a\"]\niphone = [\"b\"]\n[storefront.t.s.submission-info]\ncopyright = \"x\"\n",
                "storefront:\n  t:\n    screenshots: { default: [a], iphone: [b] }\n    s:\n      submission-info: { copyright: x }\n",
                &["b:light"],
            ),
            (
                "[storefront.t.screenshots]\ndefault = [\"a\"]\n",
                "storefront:\n  t:\n    screenshots: { default: [a] }\n",
                &["a:light"],
            ),
            (
                "[storefront.t.submission-info]\ncopyright = \"x\"\n",
                "storefront:\n  t:\n    submission-info: { copyright: x }\n",
                &[],
            ),
        ];
        for (toml_src, yaml_src, want) in cases {
            let (a, b) = both(toml_src, yaml_src);
            assert_eq!(resolved(&a), resolved(&b), "{toml_src}");
            assert_eq!(
                names(a.store_kind("t", "s", "iphone", "en")),
                want,
                "{toml_src}"
            );
            assert_eq!(
                names(b.store_kind("t", "s", "iphone", "en")),
                want,
                "{yaml_src}"
            );
        }
    }

    /// The locale axis on its own, in both formats: at one level a locale's kind beats its
    /// default beats the general kind beats the general default; across levels the locale's
    /// lists come first (the store's, then the target's) and the general lists after; a locale
    /// matches by tag, then by language.
    #[test]
    fn locale_specializations_resolve_the_same_in_both_formats() {
        let toml_src = r#"
[storefront.t.screenshots]
default = ["a"]
iphone = ["b"]
[storefront.t.screenshots.fr]
default = ["c"]
iphone = ["d"]
[storefront.t.screenshots.zh-CN]
default = ["e"]
[storefront.t.s.screenshots]
ipad = ["f"]
[storefront.t.s.screenshots.fr]
ipad = ["g"]
[storefront.t.s.screenshots.ar]
default = ["h"]
"#;
        let yaml_src = r#"
storefront:
  t:
    screenshots:
      default: [a]
      iphone: [b]
      fr: { default: [c], iphone: [d] }
      zh-CN: { default: [e] }
    s:
      screenshots:
        ipad: [f]
        fr: { ipad: [g] }
        ar: { default: [h] }
"#;
        let (a, b) = both(toml_src, yaml_src);
        assert_eq!(resolved(&a), resolved(&b));
        for s in [&a, &b] {
            // The target's own table.
            assert_eq!(names(s.website("t", "iphone", "fr")), ["d:light"]);
            assert_eq!(names(s.website("t", "ipad", "fr")), ["c:light"]);
            assert_eq!(names(s.website("t", "iphone", "fr-CA")), ["d:light"]);
            assert_eq!(
                names(s.website("t", "iphone", "zh-CN")),
                ["e:light"],
                "a locale default beats the general kind"
            );
            assert_eq!(
                names(s.website("t", "iphone", "zh-TW")),
                ["e:light"],
                "zh matches zh"
            );
            assert_eq!(names(s.website("t", "iphone", "en")), ["b:light"]);
            assert_eq!(names(s.website("t", "ipad", "en")), ["a:light"]);
            // The store's table over it.
            assert_eq!(names(s.store_kind("t", "s", "ipad", "fr")), ["g:light"]);
            assert_eq!(names(s.store_kind("t", "s", "ipad", "en")), ["f:light"]);
            assert_eq!(
                names(s.store_kind("t", "s", "ipad", "ar")),
                ["h:light"],
                "the store's locale default beats its general kind"
            );
            assert_eq!(names(s.store_kind("t", "s", "iphone", "ar")), ["h:light"]);
            assert_eq!(
                names(s.store_kind("t", "s", "iphone", "fr")),
                ["d:light"],
                "nothing at the store for iphone: the target's French kind"
            );
            assert_eq!(names(s.store_kind("t", "s", "iphone", "en")), ["b:light"]);
            assert_eq!(s.declared_locales("t", "s"), ["ar", "fr", "zh-CN"]);
            assert_eq!(s.declared_kinds("t", "s"), ["ipad", "iphone"]);
            assert_eq!(s.shot_names().len(), 8);
        }
        // A locale table holds lists, not a deeper table.
        for (src, yaml) in [
            ("[storefront.t.screenshots.fr.iphone]\nx = 1\n", false),
            (
                "storefront:\n  t:\n    screenshots:\n      fr:\n        iphone: { x: 1 }\n",
                true,
            ),
        ] {
            let err = parse_src(&parse_document(src, yaml).expect("parses")).expect_err(src);
            assert!(err.contains("not further tables"), "{err}");
        }
    }

    /// Each submission key falls back on its own, in both formats.
    #[test]
    fn each_submission_key_falls_back_on_its_own_in_both_formats() {
        let toml_src = r#"
[storefront.submission-info]
copyright = "shared"
contact-email = "shared@x"
review-notes = "shared"
[storefront.t.submission-info]
contact-email = "target@x"
review-notes = "target"
[storefront.t.s.submission-info]
review-notes = "store"
"#;
        let yaml_src = r#"
storefront:
  submission-info: { copyright: shared, contact-email: shared@x, review-notes: shared }
  t:
    submission-info: { contact-email: target@x, review-notes: target }
    s:
      submission-info: { review-notes: store }
"#;
        let (a, b) = both(toml_src, yaml_src);
        assert_eq!(resolved(&a), resolved(&b));
        for s in [&a, &b] {
            let at_store = s.submission("t", "s");
            assert_eq!(
                (
                    at_store.copyright.as_deref(),
                    at_store.contact_email.as_deref(),
                    at_store.review_notes.as_deref()
                ),
                (Some("shared"), Some("target@x"), Some("store"))
            );
            let at_target = s.submission("t", "other");
            assert_eq!(
                (
                    at_target.copyright.as_deref(),
                    at_target.contact_email.as_deref(),
                    at_target.review_notes.as_deref()
                ),
                (Some("shared"), Some("target@x"), Some("target"))
            );
            let elsewhere = s.submission("u", "s");
            assert_eq!(
                (
                    elsewhere.copyright.as_deref(),
                    elsewhere.contact_email.as_deref(),
                    elsewhere.review_notes.as_deref()
                ),
                (Some("shared"), Some("shared@x"), Some("shared"))
            );
            assert_eq!(at_store.bundle_id, None, "a key nobody sets stays unset");
        }
    }

    /// The forms an item takes, an empty declaration, and a file from before the layout,
    /// in both formats.
    #[test]
    fn item_forms_empty_files_and_legacy_keys_read_the_same_in_both_formats() {
        let (a, b) = both(
            "[storefront.t.screenshots]\ndefault = [\"one\", { name = \"two\" }, { name = \"three\", theme = \"dark\" }, { name = \" four \", theme = \"light\" }]\n",
            "storefront:\n  t:\n    screenshots:\n      default:\n        - one\n        - { name: two }\n        - name: three\n          theme: dark\n        - { name: ' four ', theme: light }\n",
        );
        assert_eq!(resolved(&a), resolved(&b));
        assert_eq!(
            names(a.website("t", "default", "en")),
            ["one:light", "two:light", "three:dark", "four:light"]
        );
        for (toml_src, yaml_src) in [
            ("", ""),
            ("# nothing\n", "# nothing\n"),
            ("[storefront]\n", "storefront: {}\n"),
        ] {
            let (a, b) = both(toml_src, yaml_src);
            assert_eq!(resolved(&a), resolved(&b), "{toml_src:?}");
            assert!(
                !a.has_screenshots()
                    && !a.has_metadata()
                    && a.targets.is_empty()
                    && a.info == SubmissionInfo::default()
            );
        }
        let (a, b) = both(
            "apple-category = \"GAMES\"\ncopyright = \"x\"\n[storefront.submission-info]\ncontact-email = \"s@x\"\n",
            "apple-category: GAMES\ncopyright: x\nstorefront:\n  submission-info: { contact-email: s@x }\n",
        );
        assert_eq!(resolved(&a), resolved(&b));
        assert_eq!(a.legacy_keys, ["apple-category", "copyright"]);
        assert_eq!(
            a.submission("ios-uikit", "apple-app-store").apple_category,
            None,
            "a top-level key is not read"
        );
        assert_eq!(a.info.contact_email.as_deref(), Some("s@x"));
    }

    /// What is refused, in both formats, with the same message; and YAML's implicit typing,
    /// the one place the formats differ, is refused by name.
    #[test]
    fn the_same_mistakes_are_refused_in_both_formats() {
        let cases: [(&str, &str, &str); 11] = [
            (
                "[storefront.metadata]\nkeywords = \"a,b\"\n",
                "storefront:\n  metadata: { keywords: \"a,b\" }\n",
                "a list of keywords",
            ),
            (
                "[storefront.metadata]\nkeywords = [\"a,b\"]\n",
                "storefront:\n  metadata: { keywords: [\"a,b\"] }\n",
                "holds a comma",
            ),
            (
                "[storefront.metadata]\nname = \"x\"\nname-ref = \"store/name.txt\"\n",
                "storefront:\n  metadata: { name: x, name-ref: store/name.txt }\n",
                "both `name` and `name-ref` are set",
            ),
            (
                "[storefront.metadata.en]\nname = \"x\"\n",
                "storefront:\n  metadata: { en: { name: x } }\n",
                "en is the app's default locale",
            ),
            (
                "[storefront.metadata.fr.name]\nx = 1\n",
                "storefront:\n  metadata: { fr: { name: { x: 1 } } }\n",
                "a locale table holds fields",
            ),
            (
                "[storefront]\nios-uikit = 3\n",
                "storefront:\n  ios-uikit: 3\n",
                "storefront.ios-uikit: a table",
            ),
            (
                "[storefront.ios-uikit.screenshots]\ndefault = 3\n",
                "storefront:\n  ios-uikit:\n    screenshots: { default: 3 }\n",
                "a list of screenshots",
            ),
            (
                "[storefront.ios-uikit.apple-app-store.screenshots]\niphone = [{ theme = \"dark\" }]\n",
                "storefront:\n  ios-uikit:\n    apple-app-store:\n      screenshots: { iphone: [{ theme: dark }] }\n",
                "names a `screenshot:` step",
            ),
            (
                "[storefront.ios-uikit.apple-app-store.screenshots]\niphone = [{ name = \"x\", mode = \"dark\" }]\n",
                "storefront:\n  ios-uikit:\n    apple-app-store:\n      screenshots: { iphone: [{ name: x, mode: dark }] }\n",
                "unknown key \"mode\"",
            ),
            (
                "[storefront.screenshots]\ndefault = [\"home\"]\n",
                "storefront:\n  screenshots: { default: [home] }\n",
                "declared per target",
            ),
            (
                "[storefront.submission-info]\ncopyright = 2026\n",
                "storefront:\n  submission-info: { copyright: 2026 }\n",
                "storefront.submission-info.copyright: a string, not 2026",
            ),
        ];
        for (toml_src, yaml_src, expected) in cases {
            for (src, yaml) in [(toml_src, false), (yaml_src, true)] {
                let doc = parse_document(src, yaml).unwrap_or_else(|e| panic!("{src}: {e}"));
                let err = parse_src(&doc).expect_err(src);
                assert!(err.contains(expected), "{src}: {err}");
            }
        }
        // YAML alone types these; quoting them is the fix the message names.
        for src in [
            "storefront:\n  submission-info: { bundle-id: 1.2 }\n",
            "storefront:\n  submission-info: { copyright: true }\n",
        ] {
            let err = parse_src(&parse_document(src, true).expect("yaml")).expect_err(src);
            assert!(err.contains("quote it"), "{err}");
        }
        let err = parse_src(
            &parse_document("storefront:\n  metadata: { name: 2026 }\n", true).expect("yaml"),
        )
        .expect_err("a number for a name");
        assert!(err.contains("quote it"), "{err}");
        assert!(
            parse_src(
                &parse_document(
                    "storefront:\n  submission-info: { copyright: '2026' }\n",
                    true
                )
                .expect("yaml")
            )
            .is_ok()
        );
        // A document that is not a table at all.
        assert!(
            parse_src(&parse_document("- a\n- b\n", true).expect("yaml"))
                .expect("a list is nothing")
                .targets
                .is_empty()
        );
        assert!(parse_document("[storefront\n", false).is_err());
        assert!(parse_document("storefront: [unclosed\n", true).is_err());
    }

    /// The file is found by name in either format, `.yml` included; two of them is refused.
    #[test]
    fn the_storefront_file_is_toml_or_yaml_but_not_both() {
        let dir = std::env::temp_dir().join(format!("day-store-dual-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let load = || load_storefront(&dir, &dir, "en");
        assert_eq!(load().expect("none").file, None);
        std::fs::write(dir.join("app.toml"), FULL_TOML).expect("toml");
        let from_toml = load().expect("toml");
        assert_eq!(from_toml.file.as_deref(), Some("app.toml"));
        assert_eq!(from_toml.files, ["app.toml"]);
        std::fs::write(dir.join("app.yaml"), FULL_YAML).expect("yaml");
        let err = load().expect_err("both");
        assert!(err.contains("app.toml and app.yaml both declare"), "{err}");
        std::fs::remove_file(dir.join("app.toml")).expect("rm");
        let from_yaml = load().expect("yaml");
        assert_eq!(from_yaml.file.as_deref(), Some("app.yaml"));
        assert_eq!(
            resolved(&from_toml.storefront),
            resolved(&from_yaml.storefront)
        );
        std::fs::rename(dir.join("app.yaml"), dir.join("app.yml")).expect("mv");
        let from_yml = load().expect("yml");
        assert_eq!(from_yml.file.as_deref(), Some("app.yml"));
        assert_eq!(
            resolved(&from_toml.storefront),
            resolved(&from_yml.storefront)
        );
        std::fs::write(dir.join("app.yml"), "storefront: [unclosed\n").expect("bad yaml");
        assert!(load().expect_err("bad").contains("app.yml"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The rules are data: the CLI's own copy, a file the caller names, or the project's.
    #[test]
    fn the_rules_file_is_read_in_order_of_precedence() {
        let own = StoreRules::parse(DEFAULT_RULES).expect("the CLI's own rules parse");
        let android = crate::targets::find("android-mdc").expect("android");
        assert_eq!(
            own.0["apple-app-store"]
                .kinds()
                .iter()
                .map(|(k, _)| *k)
                .collect::<Vec<_>>(),
            ["ipad", "iphone"]
        );
        let phone = &own.0["google-play-store"].screenshots["phone"];
        assert_eq!(
            (phone.min_side, phone.max_side, phone.max_ratio),
            (Some(1080), Some(7680), Some(2.3))
        );
        assert!(
            StoreRules::parse("[google-play-store.screenshots.phone]\nmax-ratio = 'tall'\n")
                .is_err()
        );

        let (tmp, project) = temp_project("rules", "\"android-mdc\"", &["en"]);
        assert_eq!(
            StoreRules::load(&project, None)
                .expect("own")
                .store_for(android.name)
                .map(|(_, r)| r.kinds().len()),
            Some(3)
        );
        // A project's file replaces the CLI's whole, so it has to say which store stages what.
        std::fs::write(
            tmp.join("store/rules.toml"),
            "[google-play-store]\nlayout = \"supply\"\ntargets = [\"android-mdc\"]\n\
             [google-play-store.screenshots.phone]\nmax = 3\n",
        )
        .expect("project rules");
        assert_eq!(
            StoreRules::load(&project, None)
                .expect("project")
                .store_for(android.name)
                .map(|(_, r)| r.kinds().len()),
            Some(1),
            "the project's file replaces the CLI's"
        );
        let named = tmp.join("named.toml");
        std::fs::write(
            &named,
            "[google-play-store]\nlayout = \"supply\"\ntargets = [\"android-mdc\"]\n\
             [google-play-store.screenshots.phone]\nmax = 4\n[google-play-store.screenshots.tablet]\n\
             [google-play-store.screenshots.tv]\n",
        )
        .expect("named rules");
        assert_eq!(
            StoreRules::load(&project, Some(&named))
                .expect("named")
                .store_for(android.name)
                .map(|(_, r)| r.kinds().len()),
            Some(3)
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn default_locale_prefers_en_then_the_first() {
        assert_eq!(
            default_locale(&["ar".into(), "en".into(), "fr".into()]),
            Some("en".into())
        );
        assert_eq!(
            default_locale(&["ar".into(), "fr".into()]),
            Some("ar".into())
        );
        assert_eq!(default_locale(&[]), None);
    }

    /// A `-ref` reads a project file; the same field set both ways, a file that is not there,
    /// and a path that leaves the project are each refused by name. Keywords in a file are
    /// one per line or comma-separated.
    #[test]
    fn refs_read_project_files_and_are_held_inside_the_project() {
        let files: BTreeMap<&str, &str> = [
            ("store/text/description.txt", "Long text.\n\nMore.\n"),
            ("store/text/keywords.txt", "rust\nnative, ui\n\n"),
        ]
        .into_iter()
        .collect();
        let read_ref = move |p: &str| -> Result<String, String> {
            files
                .get(p)
                .map(|s| s.to_string())
                .ok_or_else(|| "No such file or directory".to_string())
        };
        let src = Source {
            file: "storefront.toml",
            sidecar: None,
            default_locale: "en",
            read_ref: &read_ref,
        };
        let doc = parse_document(
            "[storefront.metadata]\nname = \"x\"\ndescription-ref = \"store/text/description.txt\"\nkeywords-ref = \"store/text/keywords.txt\"\n",
            false,
        )
        .expect("toml");
        let s = Storefront::parse(&doc, &src).expect("refs read");
        let en = s.metadata("", "", "en");
        assert_eq!(
            get(&en, Field::Description).as_deref(),
            Some("Long text.\n\nMore."),
            "the file's trailing newline is not part of the text"
        );
        assert_eq!(get(&en, Field::Keywords).as_deref(), Some("rust,native,ui"));
        assert_eq!(
            en[&Field::Description].origin,
            Origin::File {
                file: "store/text/description.txt".into(),
                table: "storefront.metadata".into(),
                key: "description",
            }
        );
        assert_eq!(
            en[&Field::Description].origin.describe("store"),
            "store/text/description.txt ([storefront.metadata] description-ref)"
        );
        assert_eq!(
            en[&Field::Name].origin.describe("store"),
            "store/storefront.toml [storefront.metadata] name"
        );
        for (toml_src, expected) in [
            (
                "[storefront.metadata]\ndescription-ref = \"store/text/missing.txt\"\n",
                "store/text/missing.txt: No such file",
            ),
            (
                "[storefront.metadata]\ndescription-ref = \"../secret.txt\"\n",
                "leaves the project",
            ),
            (
                "[storefront.metadata]\ndescription-ref = \"/etc/passwd\"\n",
                "not project-relative",
            ),
            (
                "[storefront.metadata]\ndescription-ref = \"C:\\\\Users\\\\me\\\\description.txt\"\n",
                "not project-relative",
            ),
            (
                "[storefront.metadata]\ndescription-ref = \"~/description.txt\"\n",
                "not project-relative",
            ),
            (
                "[storefront.metadata]\ndescription-ref = 3\n",
                "a project-relative path, not 3",
            ),
        ] {
            let err = Storefront::parse(&parse_document(toml_src, false).expect("toml"), &src)
                .expect_err(toml_src);
            assert!(err.contains(expected), "{toml_src}: {err}");
        }
    }

    /// Locale files beside the main one carry a locale's text at any level; what the main file
    /// already sets for that locale, a second locale file for the same tag, and anything but
    /// `metadata` tables in one are refused.
    #[test]
    fn locale_files_merge_into_the_main_declaration() {
        let dir = std::env::temp_dir().join(format!("day-store-sidecar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let load = || load_storefront(&dir, &dir, "en");
        std::fs::write(
            dir.join("app.toml"),
            "[storefront.metadata]\nname = \"Example\"\nshort = \"One line.\"\n\
             [storefront.metadata.fr]\nname = \"Exemple\"\n\
             [storefront.ios-uikit.apple-app-store.submission-info]\napple-category = \"GAMES\"\n",
        )
        .expect("main");
        std::fs::write(
            dir.join("app.zh-CN.yaml"),
            "storefront:\n  metadata:\n    name: 示例\n  ios-uikit:\n    apple-app-store:\n      metadata:\n        subtitle: 副标题\n",
        )
        .expect("zh");
        std::fs::write(
            dir.join("app.fr.toml"),
            "[storefront.metadata]\nshort = \"Une ligne.\"\n",
        )
        .expect("fr");
        let l = load().expect("merged");
        assert_eq!(l.files, ["app.toml", "app.fr.toml", "app.zh-CN.yaml"]);
        let s = &l.storefront;
        let fr = s.metadata("", "", "fr");
        assert_eq!(get(&fr, Field::Name).as_deref(), Some("Exemple"));
        assert_eq!(get(&fr, Field::Short).as_deref(), Some("Une ligne."));
        assert_eq!(
            fr[&Field::Short].origin,
            Origin::Inline {
                file: "app.fr.toml".into(),
                table: "storefront.metadata".into(),
                key: "short",
            }
        );
        let zh = s.metadata("ios-uikit", "apple-app-store", "zh-CN");
        assert_eq!(get(&zh, Field::Name).as_deref(), Some("示例"));
        assert_eq!(get(&zh, Field::Subtitle).as_deref(), Some("副标题"));
        assert_eq!(get(&zh, Field::Short).as_deref(), Some("One line."));
        assert_eq!(
            s.metadata_locales().into_iter().collect::<Vec<_>>(),
            ["fr", "zh-CN"]
        );

        // The default locale's file is the base table.
        std::fs::write(
            dir.join("app.en.toml"),
            "[storefront.metadata]\ndescription = \"From the en file.\"\n",
        )
        .expect("en");
        let l = load().expect("en file");
        assert_eq!(
            get(&l.storefront.metadata("", "", "en"), Field::Description).as_deref(),
            Some("From the en file.")
        );
        std::fs::remove_file(dir.join("app.en.toml")).expect("rm");

        // Refusals.
        let refused = |name: &str, body: &str, expected: &str| {
            std::fs::write(dir.join(name), body).expect("write");
            let err = load().expect_err(name);
            assert!(err.contains(expected), "{name}: {err}");
            std::fs::remove_file(dir.join(name)).expect("rm");
        };
        refused(
            "app.fr.yaml",
            "storefront:\n  metadata: { name: x }\n",
            "app.fr.toml and app.fr.yaml both carry fr",
        );
        refused(
            "app.de.toml",
            "[storefront.submission-info]\ncopyright = \"x\"\n",
            "a locale file carries de's text alone",
        );
        refused(
            "app.de.toml",
            "[storefront.android-mdc.metadata]\nname = \"x\"\n",
            "not a target the main storefront file declares",
        );
        refused(
            "app.de.toml",
            "[storefront.ios-uikit.altstore.metadata]\nname = \"x\"\n",
            "not a store the main storefront file declares",
        );
        refused(
            "app.de.toml",
            "[storefront.metadata.de]\nname = \"x\"\n",
            "no locale tables inside it",
        );
        std::fs::remove_file(dir.join("app.fr.toml")).expect("rm");
        refused(
            "app.fr.toml",
            "[storefront.metadata]\nname = \"Twice\"\n",
            "is also set by store/app.toml [storefront.metadata.fr] name",
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The tables the CLI writes read back as they were written: a multi-line description with
    /// the delimiter and backslashes in it, a keyword list, and every other field.
    #[test]
    fn written_tables_read_back_verbatim() {
        let mut fields = BTreeMap::new();
        fields.insert(
            Field::Description,
            "Line \"one\".\n\nA \\ backslash and a \"\"\" delimiter.\n  indented".to_string(),
        );
        fields.insert(Field::Keywords, "a,b c,d".to_string());
        fields.insert(Field::Name, "Ex \"ample\"".to_string());
        fields.insert(Field::PrivacyUrl, "https://x/p".to_string());
        let text = toml_table("storefront.metadata.fr", &fields);
        let s = parse_src(&parse_document(&text, false).expect("parses")).expect("reads");
        let fr = s.metadata("", "", "fr");
        assert_eq!(values(&fr).len(), 4);
        for (f, v) in &fields {
            assert_eq!(get(&fr, *f).as_deref(), Some(v.as_str()), "{f:?}");
        }
        let yaml = yaml_sidecar(&fields).expect("yaml");
        let doc = parse_document(&yaml, true).expect("yaml parses");
        let mut s = Storefront::default();
        s.merge_sidecar(
            &doc,
            &Source {
                file: "app.fr.yaml",
                sidecar: Some("fr"),
                default_locale: "en",
                read_ref: &no_refs,
            },
        )
        .expect("merges");
        let fr = s.metadata("", "", "fr");
        for (f, v) in &fields {
            assert_eq!(get(&fr, *f).as_deref(), Some(v.as_str()), "yaml {f:?}");
        }
    }

    /// A temp project with the given locales and targets.
    fn temp_project(name: &str, targets: &str, locales: &[&str]) -> (PathBuf, Project) {
        let tmp = std::env::temp_dir().join(format!("day-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("store")).expect("mkdir");
        std::fs::write(
            tmp.join("Day.toml"),
            format!("schema = 1\n[app]\nid = \"dev.example.app\"\ntitle = \"Example\"\nbuild = 7\ntargets = [{targets}]\n"),
        )
        .expect("Day.toml");
        std::fs::write(
            tmp.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"1.0.0\"\n",
        )
        .expect("Cargo.toml");
        for l in locales {
            std::fs::create_dir_all(tmp.join("resource/locales").join(l)).expect("locale");
        }
        let project = crate::meta::find_project(Some(&tmp)).expect("project");
        (tmp, project)
    }

    /// `day store migrate` folds the old per-locale files into tables, `init` starts the
    /// locales the tables lack, and `day localize add/remove` write and drop a locale's
    /// table; each reads back through the same loader.
    #[test]
    fn migrate_init_and_localize_edit_the_tables() {
        let (tmp, project) =
            temp_project("migrate", "\"ios-uikit\", \"android-mdc\"", &["en", "fr"]);
        for (tag, name) in [("en", "Example"), ("fr", "Exemple")] {
            let dir = tmp.join("store").join(tag);
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(dir.join("name.txt"), format!("{name}\n")).expect("name");
            std::fs::write(dir.join("keywords.txt"), "a, b,c\n").expect("kw");
            std::fs::write(dir.join("description.txt"), "Para.\n\nTwo.\n").expect("desc");
        }
        migrate(&project, false).expect("migrate");
        assert!(!tmp.join("store/en").exists() && !tmp.join("store/fr").exists());
        let text =
            std::fs::read_to_string(tmp.join("store/storefront.toml")).expect("storefront.toml");
        assert!(text.contains("[storefront.metadata]\n"), "{text}");
        assert!(text.contains("[storefront.metadata.fr]\n"), "{text}");
        assert!(text.contains("keywords = [\"a\", \"b\", \"c\"]"), "{text}");
        let listing = read(&project).expect("reads");
        assert_eq!(listing.locales(), ["en", "fr"]);
        assert_eq!(
            get(&listing.app.metadata("", "", "fr"), Field::Name).as_deref(),
            Some("Exemple")
        );
        assert_eq!(
            get(&listing.app.metadata("", "", "fr"), Field::Description).as_deref(),
            Some("Para.\n\nTwo.")
        );
        assert!(migrate(&project, false).is_err(), "nothing left to migrate");

        // A third locale through `day localize add`, then removed again.
        std::fs::create_dir_all(tmp.join("resource/locales/de")).expect("de");
        let lines = add_locale(&tmp, "de").expect("add");
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(add_locale(&tmp, "de").expect("again").is_empty());
        let listing = read(&project).expect("reads");
        assert_eq!(listing.locales(), ["en", "de", "fr"]);
        assert_eq!(
            get(&listing.app.metadata("", "", "de"), Field::Name).as_deref(),
            Some("Example"),
            "starts as the default locale's text"
        );
        let lines = remove_locale(&tmp, "de").expect("remove");
        assert_eq!(lines.len(), 1, "{lines:?}");
        let text =
            std::fs::read_to_string(tmp.join("store/storefront.toml")).expect("storefront.toml");
        assert!(!text.contains("metadata.de"), "{text}");
        assert!(
            text.contains("[storefront.metadata.fr]"),
            "the rest survives: {text}"
        );
        assert_eq!(read(&project).expect("reads").locales(), ["en", "fr"]);

        // `init` on a listing that lacks a locale adds that one alone.
        std::fs::create_dir_all(tmp.join("resource/locales/es")).expect("es");
        init(&project).expect("init");
        let listing = read(&project).expect("reads");
        assert_eq!(
            listing.locales(),
            ["en", "de", "es", "fr"],
            "de is a locale the app has again, so init starts it too"
        );
        assert!(init(&project).is_ok(), "nothing to add is not an error");

        // A YAML main file takes locale files instead of appended tables.
        let (tmp, project) = temp_project("migrate-yaml", "\"ios-uikit\"", &["en", "fr"]);
        std::fs::write(
            tmp.join("store/app.yaml"),
            "storefront:\n  submission-info: { copyright: x }\n",
        )
        .expect("yaml");
        for (tag, name) in [("en", "Example"), ("fr", "Exemple")] {
            let dir = tmp.join("store").join(tag);
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(dir.join("name.txt"), format!("{name}\n")).expect("name");
        }
        migrate(&project, true).expect("migrate");
        assert!(tmp.join("store/en").is_dir(), "--keep");
        assert!(tmp.join("store/app.en.yaml").is_file());
        assert!(tmp.join("store/app.fr.yaml").is_file());
        let listing = read(&project).expect("reads");
        assert_eq!(listing.files, ["app.yaml", "app.en.yaml", "app.fr.yaml"]);
        assert_eq!(
            get(&listing.app.metadata("", "", "fr"), Field::Name).as_deref(),
            Some("Exemple")
        );
        assert_eq!(
            remove_locale(&tmp, "fr").expect("rm"),
            ["removed store/app.fr.yaml"]
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Lint names the table or the referenced file a text sits in, repairs whitespace in a
    /// referenced file, and reports what each store's record lacks per locale.
    #[test]
    fn lint_names_each_place_and_what_the_stores_lack() {
        let (tmp, project) = temp_project("lint", "\"ios-uikit\", \"android-mdc\"", &["en", "fr"]);
        std::fs::create_dir_all(tmp.join("store/text")).expect("mkdir");
        std::fs::write(tmp.join("store/text/description.txt"), "  Padded.  \n").expect("desc");
        std::fs::write(
            tmp.join("store/storefront.toml"),
            "[storefront.metadata]\nname = \"Example\"\nsubtitle = \"TODO: 30 characters\"\n\
             description-ref = \"store/text/description.txt\"\nkeywords = [\"a\"]\n\
             support-url = \"http://x\"\n\
             [storefront.android-mdc.google-play-store.metadata]\nshort = \"Play only.\"\n",
        )
        .expect("storefront.toml");
        let listing = read(&project).expect("reads");
        let problems = lint(&project, &listing, StoreRules::builtin());
        let find =
            |code: &str| -> Vec<&Problem> { problems.iter().filter(|p| p.code == code).collect() };
        let missing_locale = find("day::lint::store-missing-locale");
        assert_eq!(missing_locale.len(), 1, "fr has no table");
        assert!(
            missing_locale[0]
                .message
                .contains("[storefront.metadata.fr]")
        );
        let ws = find("day::lint::store-whitespace");
        assert_eq!(ws.len(), 1);
        assert!(
            ws[0]
                .message
                .starts_with("store/text/description.txt ([storefront.metadata] description-ref)"),
            "{}",
            ws[0].message
        );
        let fix = ws[0].fix.as_ref().expect("a referenced file is repaired");
        assert_eq!(
            (fix.file.as_str(), fix.contents.as_str()),
            ("store/text/description.txt", "Padded.\n")
        );
        let todo = find("day::lint::store-placeholder");
        assert_eq!(todo.len(), 1);
        assert!(
            todo[0]
                .message
                .starts_with("store/storefront.toml [storefront.metadata] subtitle"),
            "{}",
            todo[0].message
        );
        assert_eq!(todo[0].file.as_deref(), Some("store/storefront.toml"));
        let url = find("day::lint::store-bad-url");
        assert_eq!(url.len(), 1);
        assert!(url[0].message.contains("support-url"), "{}", url[0].message);
        let missing: Vec<&str> = find("day::lint::store-missing-field")
            .iter()
            .map(|p| p.message.as_str())
            .collect();
        assert_eq!(missing.len(), 1, "{missing:?}");
        assert!(
            missing[0].contains("`privacy-url`") && missing[0].contains("the App Store"),
            "Play has its short description at the store level; Apple lacks its URL: {missing:?}"
        );
        assert!(find("day::lint::store-too-long").is_empty());

        // Too long, per the store the text can reach: a Play-only field at the App Store's
        // level is not measured by Play's rule.
        let long = "x".repeat(600);
        std::fs::write(
            tmp.join("store/storefront.toml"),
            format!(
                "[storefront.metadata]\nname = \"Example\"\nrelease-notes = \"{long}\"\n\
                 [storefront.ios-uikit.apple-app-store.metadata]\nrelease-notes = \"{long}\"\n"
            ),
        )
        .expect("storefront.toml");
        let listing = read(&project).expect("reads");
        let problems = lint(&project, &listing, StoreRules::builtin());
        let long: Vec<&str> = problems
            .iter()
            .filter(|p| p.code == "day::lint::store-too-long")
            .map(|p| p.message.as_str())
            .collect();
        assert_eq!(long.len(), 1, "{long:?}");
        assert!(
            long[0].starts_with("store/storefront.toml [storefront.metadata] release-notes")
                && long[0].contains("Google Play allows 500"),
            "{long:?}"
        );

        // No text at all, with the old layout still there: the finding says how to migrate.
        std::fs::write(
            tmp.join("store/storefront.toml"),
            "[storefront.submission-info]\n",
        )
        .expect("empty");
        std::fs::create_dir_all(tmp.join("store/en")).expect("legacy");
        let listing = read(&project).expect("reads");
        let problems = lint(&project, &listing, StoreRules::builtin());
        assert_eq!(problems.len(), 1);
        assert!(
            problems[0].code == "day::lint::store-missing"
                && problems[0].message.contains("day store migrate"),
            "{}",
            problems[0].message
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The export carries the listing resolved per target, store and locale, keywords as a
    /// list, with the project's identity and locales beside it.
    #[test]
    fn export_carries_the_resolved_listing() {
        let (tmp, project) = temp_project("export", "\"ios-uikit\"", &["en", "fr"]);
        std::fs::write(tmp.join("store/storefront.toml"), FULL_TOML).expect("storefront.toml");
        let doc = export_document(&project).expect("export");
        assert_eq!(doc["schema"], 1);
        assert_eq!(doc["project"]["id"], "dev.example.app");
        assert_eq!(doc["project"]["build"], 7);
        assert_eq!(doc["default-locale"], "en");
        assert_eq!(doc["locales"], serde_json::json!(["en", "fr", "zh-CN"]));
        let sf = &doc["storefront"];
        assert_eq!(sf["file"], "store/storefront.toml");
        assert_eq!(
            sf["metadata"]["en"]["keywords"],
            serde_json::json!(["rust", "native"])
        );
        assert_eq!(sf["metadata"]["fr"]["name"], "Exemple");
        assert_eq!(
            sf["metadata"]["fr"]["description"], "What it does.\n\nIn two paragraphs.",
            "inherited from the default locale"
        );
        assert_eq!(
            sf["targets"]["ios-uikit"]["metadata"]["en"]["subtitle"],
            "Native UI"
        );
        let apple = &sf["targets"]["ios-uikit"]["stores"]["apple-app-store"];
        assert_eq!(apple["metadata"]["fr"]["promo"], "Promo FR");
        assert_eq!(apple["metadata"]["en"]["subtitle"], "App Store subtitle");
        assert_eq!(
            apple["submission-info"]["apple-category"],
            "DEVELOPER_TOOLS"
        );
        assert_eq!(apple["screenshots"]["iphone"][0]["shot"], "controls");
        assert_eq!(apple["screenshots"]["zh-CN"]["iphone"][1]["shot"], "layout");
        assert_eq!(
            sf["targets"]["ios-uikit"]["screenshots"]["fr"]["default"][0]["shot"],
            "localization"
        );
        assert!(
            sf["targets"]["android-mdc"].is_object(),
            "a target the file names and the app does not build is still exported"
        );
        assert!(doc["permissions"].is_array());
        assert_eq!(
            doc["rules"]["apple-app-store"]["fields"]["name"]["limit"], 30,
            "the rules in force travel with the export"
        );
        assert_eq!(doc["rules"]["google-play-store"]["locales"]["he"], "iw-IL");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// `storefront.toml` is the name; `app.toml` still reads, with a finding that says to
    /// rename, and a directory carrying both is refused like two formats are.
    #[test]
    fn the_storefront_file_is_named_storefront_and_the_old_name_still_reads() {
        let (tmp, project) = temp_project("names", "\"ios-uikit\"", &["en", "fr"]);
        std::fs::write(
            tmp.join("store/app.toml"),
            "[storefront.metadata]\nname = \"Example\"\ndescription = \"x\"\nprivacy-url = \"https://x/p\"\n",
        )
        .expect("app.toml");
        std::fs::write(
            tmp.join("store/app.fr.toml"),
            "[storefront.metadata]\nname = \"Exemple\"\n",
        )
        .expect("app.fr.toml");
        let listing = read(&project).expect("reads the old names");
        assert_eq!(listing.files, ["app.toml", "app.fr.toml"]);
        let problems = lint(&project, &listing, StoreRules::builtin());
        let legacy: Vec<&str> = problems
            .iter()
            .filter(|p| p.code == "day::lint::store-legacy-name")
            .map(|p| p.message.as_str())
            .collect();
        assert_eq!(legacy.len(), 2, "{legacy:?}");
        assert!(
            legacy[0].contains("rename it to store/storefront.toml")
                && legacy[1].contains("store/storefront.fr.toml"),
            "{legacy:?}"
        );
        std::fs::rename(
            tmp.join("store/app.toml"),
            tmp.join("store/storefront.toml"),
        )
        .expect("mv");
        std::fs::rename(
            tmp.join("store/app.fr.toml"),
            tmp.join("store/storefront.fr.toml"),
        )
        .expect("mv");
        let listing = read(&project).expect("reads the new names");
        assert_eq!(listing.files, ["storefront.toml", "storefront.fr.toml"]);
        assert!(
            lint(&project, &listing, StoreRules::builtin())
                .iter()
                .all(|p| p.code != "day::lint::store-legacy-name")
        );
        // A new locale file is named after the main file's stem.
        std::fs::create_dir_all(tmp.join("resource/locales/de")).expect("de");
        add_locale(&tmp, "de").expect("add");
        assert!(
            std::fs::read_to_string(tmp.join("store/storefront.toml"))
                .expect("main")
                .contains("[storefront.metadata.de]")
        );
        std::fs::write(tmp.join("store/app.toml"), "[storefront]\n").expect("a second file");
        let err = read(&project).expect_err("two main files");
        assert!(
            err.contains("storefront.toml and app.toml both declare"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A key the parser does not know is read past and reported by name, with the keys the
    /// table takes; the listing still stages around it. A store key the rules do not know is
    /// named too, since it is either a storefront the CLI has not heard of or a typo.
    #[test]
    fn unknown_keys_and_stores_are_findings_not_failures() {
        let (tmp, project) = temp_project("unknown", "\"ios-uikit\"", &["en"]);
        std::fs::write(
            tmp.join("store/storefront.toml"),
            "[storefront.submission-info]\ncopyrite = \"x\"\n\
             [storefront.metadata]\nname = \"Example\"\ntitle = \"x\"\ndescription = \"d\"\nprivacy-url = \"https://x/p\"\n\
             [storefront.metadata.fr]\nkeyword = [\"a\"]\n\
             [storefront.ios-uikit.apple-appstore.submission-info]\napple-category = \"GAMES\"\n\
             [storefront.ios-uikit.apple-app-store]\nnotes = \"x\"\n",
        )
        .expect("storefront.toml");
        let listing = read(&project).expect("reads past the typos");
        assert_eq!(
            get(&listing.app.metadata("", "", "en"), Field::Name).as_deref(),
            Some("Example")
        );
        let problems = lint(&project, &listing, StoreRules::builtin());
        let unknown: Vec<&str> = problems
            .iter()
            .filter(|p| p.code == "day::lint::store-unknown-key")
            .map(|p| p.message.as_str())
            .collect();
        assert_eq!(unknown.len(), 4, "{unknown:?}");
        assert!(
            unknown
                .iter()
                .any(|m| m.contains("[storefront.submission-info] copyrite")
                    && m.contains("bundle-id")),
            "{unknown:?}"
        );
        assert!(
            unknown
                .iter()
                .any(|m| m.contains("[storefront.metadata] title") && m.contains("subtitle")),
            "{unknown:?}"
        );
        assert!(
            unknown
                .iter()
                .any(|m| m.contains("[storefront.metadata.fr] keyword")),
            "{unknown:?}"
        );
        assert!(
            unknown
                .iter()
                .any(|m| m.contains("[storefront.ios-uikit.apple-app-store] notes")),
            "{unknown:?}"
        );
        let stores: Vec<&str> = problems
            .iter()
            .filter(|p| p.code == "day::lint::store-unknown-store")
            .map(|p| p.message.as_str())
            .collect();
        assert_eq!(stores.len(), 1, "{stores:?}");
        assert!(
            stores[0].contains("apple-appstore") && stores[0].contains("apple-app-store"),
            "{stores:?}"
        );
        assert_eq!(
            crate::lint::severity_of("day::lint::store-unknown-key"),
            crate::lint::Severity::Error
        );
        assert_eq!(
            crate::lint::severity_of("day::lint::store-unknown-store"),
            crate::lint::Severity::Warning
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A halved CI tablet capture is placed scaled up to the store's floor, a real PNG twice
    /// the size; a capture already at a store size is placed as it is.
    #[test]
    fn staging_scales_a_halved_tablet_capture_up_for_play() {
        use day_vector::tiny_skia;
        let (tmp, project) = temp_project("upscale", "\"android-mdc\"", &["en"]);
        let listing = listing_of("en", &[("en", &[(Field::Name, "Example")])]);
        let tree = tmp.join("screenshots");
        let mut entries = Vec::new();
        for (device, w, h) in [("phone", 1080u32, 1920u32), ("tablet", 1280, 800)] {
            let rel = format!("android-mdc/{device}/light/home.png");
            let path = tree.join(&rel);
            std::fs::create_dir_all(path.parent().expect("dir")).expect("mkdir");
            let mut pm = tiny_skia::Pixmap::new(w, h).expect("pixmap");
            pm.fill(tiny_skia::Color::from_rgba8(30, 60, 90, 255));
            std::fs::write(&path, pm.encode_png().expect("png")).expect("capture");
            entries.push(serde_json::json!({
                "path": format!("gallery/{rel}"), "shot": "home", "device": device,
                "os": "android", "platform": "android-mdc", "theme": "light", "locale": "en",
                "store": 1, "width": w, "height": h,
            }));
        }
        let index_path = tree.join("gallery.json");
        std::fs::write(&index_path, index(&["en"], entries).to_string()).expect("index");
        let source = ScreenshotSource::parse(index_path.to_str().expect("utf-8"));
        let rules = StoreRules::parse(DEFAULT_RULES).expect("rules");
        let android = crate::targets::find("android-mdc").expect("android");
        let out = tmp.join("out-android");
        stage(&project, android, &listing, &out, Some(&source), &rules).expect("stage");
        let dims =
            |rel: &str| crate::screenshot::png_dims(&std::fs::read(out.join(rel)).expect("placed"));
        assert_eq!(
            dims("fastlane/metadata/android/en-US/images/tenInchScreenshots/01-home.png"),
            Some((2560, 1600)),
            "the halved capture goes up ×2"
        );
        assert_eq!(
            dims("fastlane/metadata/android/en-US/images/phoneScreenshots/01-home.png"),
            Some((1080, 1920)),
            "a capture at a store size is placed as captured"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
