// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Store listings (docs/store.md): one canonical source per app, generated into the layouts App Store
//! Connect and Google Play expect.
//!
//! An app's listing text is localized user-facing copy, so it lives beside the app's other
//! localized copy, as plain text a translator can edit, under `store/<locale>/`, keyed by the same
//! locale tags `resource/locales/` uses. The stores disagree about almost everything else: what the
//! fields are called, how long they may be, and how a locale is spelled (`zh-CN` here is `zh-Hans`
//! to Apple and `zh-CN` to Google; Hebrew is `he` to Apple and the legacy `iw-IL` to Google). All of
//! that divergence is handled here, at generation time, rather than by asking the author to keep two
//! parallel trees in step, the same reason `resource/` fans out to per-platform resources rather
//! than being authored per platform.
//!
//! ```text
//! store/app.toml            # not localized: category, copyright, contacts, review notes
//! store/en/name.txt         # ≤30   App Store name / Play title
//! store/en/subtitle.txt     # ≤30   App Store only
//! store/en/short.txt        # ≤80   Play short description
//! store/en/description.txt  # ≤4000 both
//! store/en/keywords.txt     # ≤100  App Store only, comma-separated
//! store/en/release-notes.txt        # ≤4000 App Store, ≤500 Play (the stricter one binds)
//! store/en/promo.txt        # ≤170  App Store promotional text (optional)
//! store/en/marketing-url.txt, support-url.txt, privacy-url.txt
//! ```
//!
//! `day store stage -p <target>` (and every `day pack` of a store target) writes
//! `build/day/fastlane/<target>/`, a tree `fastlane deliver` / `fastlane supply` accept as-is.
//! Generated, never checked in: the pristine-checkout rule (§20.3) means a build must not write
//! into tracked directories.

use std::collections::BTreeMap;
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
    /// The file under `store/<locale>/` this field is read from.
    pub fn file(self) -> &'static str {
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

    /// `deliver`'s file name and the App Store limit, when the App Store has this field.
    pub fn apple(self) -> Option<(&'static str, usize)> {
        match self {
            Field::Name => Some(("name.txt", 30)),
            Field::Subtitle => Some(("subtitle.txt", 30)),
            Field::Description => Some(("description.txt", 4000)),
            Field::Keywords => Some(("keywords.txt", 100)),
            Field::ReleaseNotes => Some(("release_notes.txt", 4000)),
            Field::Promo => Some(("promotional_text.txt", 170)),
            Field::MarketingUrl => Some(("marketing_url.txt", 255)),
            Field::SupportUrl => Some(("support_url.txt", 255)),
            Field::PrivacyUrl => Some(("privacy_url.txt", 255)),
            // Play's short description has no App Store counterpart.
            Field::Short => None,
        }
    }

    /// `supply`'s file name and the Play limit, when Google Play has this field.
    pub fn play(self) -> Option<(&'static str, usize)> {
        match self {
            Field::Name => Some(("title.txt", 30)),
            Field::Short => Some(("short_description.txt", 80)),
            Field::Description => Some(("full_description.txt", 4000)),
            // Play's changelog is far shorter than the App Store's release notes, and it is the
            // limit that binds for an app shipping to both.
            Field::ReleaseNotes => Some(("changelog", 500)),
            // Play's `video.txt` is a YouTube promo video, not a website: supply sends it as
            // the listing's video and Google refuses any other URL ("Invalid YouTube URL",
            // the Day Showcase's first dry run, 2026-09-11). Play takes no marketing URL
            // through the API; the website lives in the Play Console's store settings.
            Field::MarketingUrl => None,
            Field::Subtitle | Field::Keywords | Field::Promo => None,
            Field::SupportUrl | Field::PrivacyUrl => None,
        }
    }

    /// Fields without which a store rejects the listing.
    pub fn required(self) -> bool {
        matches!(self, Field::Name | Field::Description)
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

/// Day locale tag → (App Store locale, Play locale).
///
/// Neither store accepts a bare BCP-47 tag for every language: Apple wants `zh-Hans` where Google
/// wants `zh-CN`, Google still spells Hebrew with the pre-1989 ISO code `iw`, and both prefer
/// region-qualified English. A tag missing from this table is a lint error rather than a guess,
/// because uploading a listing under a locale the store does not know silently drops it.
pub const LOCALES: &[(&str, Option<&str>, Option<&str>)] = &[
    ("en", Some("en-US"), Some("en-US")),
    ("en-GB", Some("en-GB"), Some("en-GB")),
    ("fr", Some("fr-FR"), Some("fr-FR")),
    ("fr-CA", Some("fr-CA"), Some("fr-CA")),
    ("de", Some("de-DE"), Some("de-DE")),
    ("es", Some("es-ES"), Some("es-ES")),
    ("es-MX", Some("es-MX"), Some("es-419")),
    ("it", Some("it"), Some("it-IT")),
    ("nl", Some("nl-NL"), Some("nl-NL")),
    ("pt-BR", Some("pt-BR"), Some("pt-BR")),
    ("pt-PT", Some("pt-PT"), Some("pt-PT")),
    ("ru", Some("ru"), Some("ru-RU")),
    ("pl", Some("pl"), Some("pl-PL")),
    ("tr", Some("tr"), Some("tr-TR")),
    ("ar", Some("ar-SA"), Some("ar")),
    ("he", Some("he"), Some("iw-IL")),
    ("ja", Some("ja"), Some("ja-JP")),
    ("ko", Some("ko"), Some("ko-KR")),
    ("zh-CN", Some("zh-Hans"), Some("zh-CN")),
    ("zh-TW", Some("zh-Hant"), Some("zh-TW")),
    ("hi", Some("hi"), Some("hi-IN")),
    ("id", Some("id"), Some("id")),
    ("th", Some("th"), Some("th")),
    ("vi", Some("vi"), Some("vi")),
    ("uk", Some("uk"), Some("uk")),
    ("cs", Some("cs"), Some("cs-CZ")),
    ("sv", Some("sv"), Some("sv-SE")),
    ("da", Some("da"), Some("da-DK")),
    ("fi", Some("fi"), Some("fi-FI")),
    ("no", Some("no"), Some("no-NO")),
    ("el", Some("el"), Some("el-GR")),
    ("hu", Some("hu"), Some("hu-HU")),
    ("ro", Some("ro"), Some("ro")),
    ("sk", Some("sk"), Some("sk")),
    ("ms", Some("ms"), Some("ms")),
    ("ca", Some("ca"), Some("ca")),
    ("hr", Some("hr"), Some("hr")),
    // Region-qualified spellings of the locales `day new app` scaffolds. Apple takes a
    // script subtag for Chinese and a bare language for most others; Play prefers the
    // region-qualified form. Kept alongside the short tags rather than replacing them, so
    // a project that spells its locales either way still stages.
    ("en-US", Some("en-US"), Some("en-US")),
    ("es-ES", Some("es-ES"), Some("es-ES")),
    ("fr-FR", Some("fr-FR"), Some("fr-FR")),
    ("de-DE", Some("de-DE"), Some("de-DE")),
    ("it-IT", Some("it"), Some("it-IT")),
    ("nl-NL", Some("nl-NL"), Some("nl-NL")),
    ("pl-PL", Some("pl"), Some("pl-PL")),
    ("ru-RU", Some("ru"), Some("ru-RU")),
    ("tr-TR", Some("tr"), Some("tr-TR")),
    ("cs-CZ", Some("cs"), Some("cs-CZ")),
    ("uk-UA", Some("uk"), Some("uk")),
    ("ar-SA", Some("ar-SA"), Some("ar")),
    ("ja-JP", Some("ja"), Some("ja-JP")),
    ("ko-KR", Some("ko"), Some("ko-KR")),
    ("id-ID", Some("id"), Some("id")),
    ("ms-MY", Some("ms"), Some("ms")),
    ("th-TH", Some("th"), Some("th")),
    ("vi-VN", Some("vi"), Some("vi")),
    ("zh-Hans-CN", Some("zh-Hans"), Some("zh-CN")),
    ("zh-Hant-TW", Some("zh-Hant"), Some("zh-TW")),
];

/// The store locale for a Day tag, or `None` when that store has no such locale.
pub fn store_locale(day_tag: &str, apple: bool) -> Option<&'static str> {
    LOCALES
        .iter()
        .find(|(d, _, _)| *d == day_tag)
        .and_then(|(_, a, p)| if apple { *a } else { *p })
}

/// Whether a Day tag is mappable to either store at all.
pub fn mappable(day_tag: &str) -> bool {
    LOCALES.iter().any(|(d, _, _)| *d == day_tag)
}

/// `store/app.toml`: the parts of a listing that are not localized.
#[derive(Debug, Clone, Default)]
pub struct AppMeta {
    /// Play package name / App Store bundle id. Defaults to `[app] id`.
    pub bundle_id: Option<String>,
    /// App Store primary category, e.g. `DEVELOPER_TOOLS` (deliver's `primary_category`).
    ///
    /// There is no Play counterpart: Google Play's category is set in the Play
    /// Console and `supply` cannot write it, so recording one here would be a value that silently
    /// never reached the store.
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
    /// Which theme's captures the listing's screenshots come from (`light` when unset), for a
    /// walkthrough that captures both. A capture with no theme is taken either way.
    pub screenshot_theme: Option<String>,
}

/// A listing: the non-localized metadata plus one map of fields per locale.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    pub app: AppMeta,
    /// Day locale tag → field → text (trimmed of the trailing newline an editor adds).
    pub locales: BTreeMap<String, BTreeMap<Field, String>>,
}

impl Listing {
    pub fn is_empty(&self) -> bool {
        self.locales.is_empty()
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
    let mut listing = Listing::default();
    if !root.is_dir() {
        return Ok(listing);
    }
    listing.app = read_app_meta(&root.join("app.toml"))?;

    let entries = std::fs::read_dir(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let Some(tag) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let mut fields = BTreeMap::new();
        for f in FIELDS {
            let file = p.join(f.file());
            if !file.is_file() {
                continue;
            }
            let text = std::fs::read_to_string(&file)
                .map_err(|e| format!("{}: {e}", file.display()))?
                .trim_end_matches(['\n', '\r'])
                .to_string();
            if !text.trim().is_empty() {
                fields.insert(*f, text);
            }
        }
        if !fields.is_empty() {
            listing.locales.insert(tag.to_string(), fields);
        }
    }
    Ok(listing)
}

fn read_app_meta(path: &Path) -> Result<AppMeta, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(AppMeta::default());
    };
    let doc: toml::Value = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let get = |k: &str| {
        doc.get(k)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Ok(AppMeta {
        bundle_id: get("bundle-id"),
        apple_category: get("apple-category"),
        copyright: get("copyright"),
        contact_email: get("contact-email"),
        contact_first_name: get("contact-first-name"),
        contact_last_name: get("contact-last-name"),
        contact_phone: get("contact-phone"),
        review_notes: get("review-notes"),
        screenshot_theme: get("screenshot-theme"),
    })
}

/// Locales the app itself ships (`resource/locales/<tag>/`), which the listing must match.
pub fn app_locales(project: &Project) -> Vec<String> {
    let dir = project.root.join("resource/locales");
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
/// iOS gets `metadata/<apple-locale>/…` (deliver); Android gets `metadata/android/<play-locale>/…`
/// plus `changelogs/<versionCode>.txt` (supply). Both get an `Appfile` and a `Fastfile` with the
/// two lanes a release needs: a dry run that validates against the store without publishing, and
/// the upload itself.
pub fn stage(
    project: &Project,
    target: &'static crate::targets::Target,
    listing: &Listing,
    out: &Path,
    screenshots: Option<&ScreenshotSource>,
) -> Result<Vec<PathBuf>, String> {
    let apple = match target.toolkit {
        "uikit" => true,
        "mdc" => false,
        other => return Err(format!("{other} has no store listing format")),
    };
    let _ = std::fs::remove_dir_all(out);
    let mut written = Vec::new();
    // The screenshots first, so a listing whose index cannot be read fails before anything is
    // written: a tree with the copy and no images would upload a listing with none.
    let with_screenshots = match screenshots {
        Some(source) => {
            let placed = stage_screenshots(source, target, listing, out)?;
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

    for (tag, fields) in &listing.locales {
        let Some(loc) = store_locale(tag, apple) else {
            continue; // reported by lint; generation skips rather than inventing a locale
        };
        for (field, text) in fields {
            let Some((name, _)) = (if apple { field.apple() } else { field.play() }) else {
                continue;
            };
            let rel = if apple {
                format!("fastlane/metadata/{loc}/{name}")
            } else if name == "changelog" {
                // supply keys the changelog by versionCode, which is Day.toml's `[app] build`.
                format!(
                    "fastlane/metadata/android/{loc}/changelogs/{}.txt",
                    project.manifest.app.build
                )
            } else {
                format!("fastlane/metadata/android/{loc}/{name}")
            };
            write(&rel, &format!("{}\n", text.trim_end()))?;
        }
    }

    // The id this target publishes under, not the app's top-level one: an `[app.android]` id is
    // how a bundle id with a hyphen becomes a legal Play package name, and `supply` refuses the
    // unresolved one ("Invalid package name"). A listing that names its own `bundle-id` still
    // wins, since it describes a record that already exists.
    let resolved = project.manifest.resolve(target.name);
    let id = listing
        .app
        .bundle_id
        .clone()
        .unwrap_or_else(|| resolved.id.clone());
    if apple {
        if let Some(c) = &listing.app.copyright {
            write("fastlane/metadata/copyright.txt", &format!("{c}\n"))?;
        }
        if let Some(c) = &listing.app.apple_category {
            write("fastlane/metadata/primary_category.txt", &format!("{c}\n"))?;
        }
        // The review contact only as a complete record (`AppMeta::contact_first_name`).
        if let (Some(first), Some(last), Some(phone), Some(email)) = (
            &listing.app.contact_first_name,
            &listing.app.contact_last_name,
            &listing.app.contact_phone,
            &listing.app.contact_email,
        ) {
            let ri = "fastlane/metadata/review_information";
            write(&format!("{ri}/first_name.txt"), &format!("{first}\n"))?;
            write(&format!("{ri}/last_name.txt"), &format!("{last}\n"))?;
            write(&format!("{ri}/phone_number.txt"), &format!("{phone}\n"))?;
            write(&format!("{ri}/email_address.txt"), &format!("{email}\n"))?;
            if let Some(n) = &listing.app.review_notes {
                write(&format!("{ri}/notes.txt"), &format!("{n}\n"))?;
            }
        }
        write("fastlane/Appfile", &apple_appfile(&id, &listing.app))?;
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
    write("fastlane/.env.default", FASTLANE_ENV)?;
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
    width: u32,
    height: u32,
    entry: serde_json::Value,
}

/// The captures an index marks for `target`'s store, in listing order: `store: N` entries of
/// that OS, in the theme the listing asked for (a capture with no theme is taken as it is), with
/// a locale.
fn listing_shots(
    index: &serde_json::Value,
    target: &'static crate::targets::Target,
    theme: &str,
) -> Result<Vec<StoreShot>, String> {
    let entries = index["screenshots"]
        .as_array()
        .ok_or("the gallery index has no `screenshots` list")?;
    let mut chosen: Vec<StoreShot> = Vec::new();
    for e in entries {
        let Some(position) = e["store"].as_u64() else {
            continue;
        };
        if e["os"].as_str() != Some(target.os) {
            continue;
        }
        if let Some(t) = e["theme"].as_str()
            && t != theme
        {
            continue;
        }
        let Some(locale) = e["locale"].as_str() else {
            continue;
        };
        chosen.push(StoreShot {
            position: position as u32,
            device: e["device"].as_str().unwrap_or("phone").to_string(),
            shot: e["shot"].as_str().unwrap_or("shot").to_string(),
            locale: locale.to_string(),
            width: e["width"].as_u64().unwrap_or(0) as u32,
            height: e["height"].as_u64().unwrap_or(0) as u32,
            entry: e.clone(),
        });
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

/// What a store takes for one device kind of screenshot (docs/store.md).
struct ShotRule {
    /// The capture's device slug, or its prefix for `tablet*`.
    kind: &'static str,
    /// Whether every locale needs at least one: App Store Connect refuses a version whose
    /// localization has none, and Play refuses a listing without phone screenshots.
    required: bool,
    /// The store's ceiling per locale.
    max: usize,
    /// Apple: the exact sizes it takes for the kind.
    sizes: &'static [(u32, u32)],
    /// Google: the short side at least, the long side at most, and the long side at most this
    /// many times the short.
    range: Option<(u32, u32, f64)>,
}

/// The App Store's 6.9" iPhone and 13" iPad sizes: what the default CI device profiles
/// (`iPhone * Pro Max`, `iPad Pro 13-inch`) produce.
const APPLE_RULES: &[ShotRule] = &[
    ShotRule {
        kind: "iphone",
        required: true,
        max: 10,
        sizes: &[
            (1320, 2868),
            (2868, 1320),
            (1290, 2796),
            (2796, 1290),
            (1260, 2736),
            (2736, 1260),
        ],
        range: None,
    },
    ShotRule {
        kind: "ipad",
        required: true,
        max: 10,
        sizes: &[(2064, 2752), (2752, 2064), (2048, 2732), (2732, 2048)],
        range: None,
    },
];

/// Google Play's rules: any size from 320 to 3840 px a side whose long side is at most twice the
/// short, which the 20:9 `medium_phone` profile exceeds and a 9:16 one such as `pixel` meets.
const PLAY_RULES: &[ShotRule] = &[
    ShotRule {
        kind: "phone",
        required: true,
        max: 8,
        sizes: &[],
        range: Some((320, 3840, 2.0)),
    },
    ShotRule {
        kind: "tablet",
        required: false,
        max: 8,
        sizes: &[],
        range: Some((320, 3840, 2.0)),
    },
];

fn rule_for<'a>(rules: &'a [ShotRule], device: &str) -> Option<&'a ShotRule> {
    rules
        .iter()
        .find(|r| device == r.kind || (r.kind == "tablet" && device.starts_with("tablet")))
}

/// What `target`'s store would refuse about the listing's screenshots in `index`, read from the
/// index alone (its `width`/`height`): each capture's size, the ceiling per locale, and a
/// screenshot for every locale the index carries that the store knows, on each required kind.
/// Empty when the listing would go through.
pub fn screenshot_problems(
    index: &serde_json::Value,
    target: &'static crate::targets::Target,
    theme: &str,
) -> Vec<String> {
    let apple = target.toolkit == "uikit";
    let store = if apple {
        "the App Store"
    } else {
        "Google Play"
    };
    let rules = if apple { APPLE_RULES } else { PLAY_RULES };
    let chosen = match listing_shots(index, target, theme) {
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
    locales.retain(|l| store_locale(l, apple).is_some());
    for s in &chosen {
        if rule_for(rules, &s.device).is_none() {
            problems.push(format!(
                "{store}: the screenshot {:?} ({}) was captured on device {:?}, which is not a \
                 kind the store lists; the CI device profile's `slug=` names it: {}",
                s.shot,
                s.locale,
                s.device,
                rules.iter().map(|r| r.kind).collect::<Vec<_>>().join(", ")
            ));
        }
    }
    for rule in rules {
        let mine: Vec<&StoreShot> = chosen
            .iter()
            .filter(|s| rule_for(rules, &s.device).is_some_and(|r| r.kind == rule.kind))
            .collect();
        for s in &mine {
            let (w, h) = (s.width, s.height);
            let (lo, hi) = (w.min(h), w.max(h));
            if !rule.sizes.is_empty() && !rule.sizes.contains(&(w, h)) {
                problems.push(format!(
                    "{store}: the {} screenshot {:?} ({}) is {w}×{h}, which the store does not \
                     take for {}; it takes {}. Capture on a device profile that produces one of \
                     those",
                    rule.kind,
                    s.shot,
                    s.locale,
                    rule.kind,
                    rule.sizes
                        .iter()
                        .map(|(a, b)| format!("{a}×{b}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                continue;
            }
            if let Some((min_side, max_side, ratio)) = rule.range {
                if lo < min_side {
                    problems.push(format!(
                        "{store}: the {} screenshot {:?} ({}) is {w}×{h}; the short side has to \
                         be at least {min_side} px",
                        rule.kind, s.shot, s.locale
                    ));
                }
                if hi > max_side {
                    problems.push(format!(
                        "{store}: the {} screenshot {:?} ({}) is {w}×{h}; the long side has to \
                         be at most {max_side} px",
                        rule.kind, s.shot, s.locale
                    ));
                }
                if lo > 0 && f64::from(hi) / f64::from(lo) > ratio + 1e-9 {
                    problems.push(format!(
                        "{store}: the {} screenshot {:?} ({}) is {w}×{h}, {:.2}:1; the store \
                         takes at most {ratio}:1 (the long side no more than {ratio}× the \
                         short). Capture on a {} profile with a shorter screen, such as a 9:16 \
                         one",
                        rule.kind,
                        s.shot,
                        s.locale,
                        f64::from(hi) / f64::from(lo),
                        rule.kind
                    ));
                }
            }
        }
        let mut per_locale: BTreeMap<&str, usize> = BTreeMap::new();
        for s in &mine {
            *per_locale.entry(s.locale.as_str()).or_default() += 1;
        }
        for (locale, n) in &per_locale {
            if *n > rule.max {
                problems.push(format!(
                    "{store}: {n} {} screenshots for {locale}, and the store takes at most {}; \
                     lower the `store:` marks in the walkthrough to {}",
                    rule.kind, rule.max, rule.max
                ));
            }
        }
        if rule.required {
            if mine.is_empty() {
                problems.push(format!(
                    "{store} needs {} screenshots and the index marks none. Add `store: N` to \
                     the `screenshot:` steps the listing should show, and capture on a {} \
                     profile",
                    rule.kind, rule.kind
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
                        rule.kind,
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
/// The index marks a capture for the listing with `store: N` (the dayscript step's `store:`,
/// §14.7). Of the theme variants, the one `store/app.toml`'s `screenshot-theme` names is taken,
/// `light` by default; a capture with no theme is taken as it is. Every locale in the index that
/// the store knows gets its own set, so the listing is as localized as the walkthrough. A set the
/// store would refuse ([`screenshot_problems`]) is refused here, before anything is uploaded.
///
/// Apple: `fastlane/screenshots/<locale>/<NN>-<device>-<shot>.png`; deliver reads the device
/// from the image's size and orders by name. Play: `fastlane/metadata/android/<locale>/images/
/// <phoneScreenshots|sevenInchScreenshots|tenInchScreenshots>/<NN>-<shot>.png`, the folder
/// chosen by the capture's device slug.
fn stage_screenshots(
    source: &ScreenshotSource,
    target: &'static crate::targets::Target,
    listing: &Listing,
    out: &Path,
) -> Result<Vec<PathBuf>, String> {
    let apple = target.toolkit == "uikit";
    let index = source.index()?;
    let theme = listing_theme(listing);
    let problems = screenshot_problems(&index, target, &theme);
    if !problems.is_empty() {
        return Err(format!(
            "the listing's screenshots would be refused:\n  {}",
            problems.join("\n  ")
        ));
    }
    let mut written = Vec::new();
    for shot in listing_shots(&index, target, &theme)? {
        let Some(loc) = store_locale(&shot.locale, apple) else {
            continue; // a locale the store does not know; lint reports it
        };
        let rel = if apple {
            format!(
                "fastlane/screenshots/{loc}/{:02}-{}-{}.png",
                shot.position, shot.device, shot.shot
            )
        } else {
            let folder = match shot.device.as_str() {
                "tablet-7" | "tablet7" => "sevenInchScreenshots",
                d if d.starts_with("tablet") => "tenInchScreenshots",
                _ => "phoneScreenshots",
            };
            format!(
                "fastlane/metadata/android/{loc}/images/{folder}/{:02}-{}.png",
                shot.position, shot.shot
            )
        };
        let bytes = source.fetch(&shot.entry)?;
        let path = out.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}

/// The theme the listing's screenshots come from: `screenshot-theme` in `store/app.toml`,
/// `light` by default.
fn listing_theme(listing: &Listing) -> String {
    listing
        .app
        .screenshot_theme
        .clone()
        .unwrap_or_else(|| "light".to_string())
}

/// fastlane opens its analytics session before it parses the Fastfile, so `opt_out_usage` there
/// cannot stop the launch event — it is already out. fastlane reads `fastlane/.env.default`
/// first, which is early enough, and it travels with the staged tree: a lane run by hand on a
/// laptop is as quiet as one run in CI.
const FASTLANE_ENV: &str = "\
# Generated by `day store stage` — edit store/ in the project, not this file.\n\
# No metrics: a lane that publishes an app sends nothing anywhere but the store.\n\
FASTLANE_OPT_OUT_USAGE=1\n";

fn apple_appfile(id: &str, app: &AppMeta) -> String {
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
  # twice is refused, so this submits what is there rather than sending it again.
  lane :submit do
    deliver(
      api_key: day_asc_key,
      skip_binary_upload: true,
      metadata_path: File.expand_path("metadata", __dir__),
      submit_for_review: true,
      automatic_release: false,
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
  # Release stays manual: an approved version waits for the Release button in App Store
  # Connect. Export compliance is answered as exempt (the app uses only the platform's
  # HTTPS), and the build carries no advertising identifier.
  lane :release do
    deliver(
      api_key: day_asc_key,
      ipa: day_ipa,
      metadata_path: File.expand_path("metadata", __dir__),
      submit_for_review: true,
      automatic_release: false,
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
    matches!(target.toolkit, "uikit" | "mdc")
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

/// A keyword list with no space around any comma, which is the only form that does not waste
/// Apple's 100-character budget.
///
/// Splitting and rejoining rather than replacing `", "` with `","`, so that two spaces, or a space
/// Before a comma, reach the same normal form: `day lint --fix` re-checks after writing, and a
/// repair that left work behind would report the same finding forever.
fn tidy_keywords(text: &str) -> String {
    format!(
        "{}\n",
        text.split(',').map(str::trim).collect::<Vec<_>>().join(",")
    )
}

/// Check a listing against the stores the app targets.
///
/// Silent when the app ships to neither store, and when it has no `store/` at all; an app that
/// never leaves a developer's machine should not be nagged about App Store copy. Once `store/`
/// exists, it is held to the stores' rules, because the alternative is finding out at upload time.
pub fn lint(project: &Project, listing: &Listing) -> Vec<Problem> {
    let mut out = Vec::new();
    // The listing directory this run read: `store/`, or the flavor's (DESIGN.md §16.6). Findings
    // name paths relative to the project, and `--fix` writes to the path a finding names, so a
    // flavored run has to point at the files it actually read.
    let store = dir(project)
        .strip_prefix(&project.root)
        .map(|rel| rel.display().to_string())
        .unwrap_or_else(|_| "store".to_string());
    let targets = &project.manifest.app.targets;
    let to_apple = targets.iter().any(|t| t == "ios-uikit");
    let to_play = targets.iter().any(|t| t == "android-mdc");
    if !to_apple && !to_play {
        return out;
    }
    let app_locales = app_locales(project);
    if listing.is_empty() {
        if !app_locales.is_empty() || !targets.is_empty() {
            out.push(Problem {
                code: "day::lint::store-missing",
                message: format!(
                    "this app ships to {} but has no {store}/ listing — run `day store init`",
                    if to_apple && to_play {
                        "the App Store and Google Play"
                    } else if to_apple {
                        "the App Store"
                    } else {
                        "Google Play"
                    }
                ),
                ..Default::default()
            });
        }
        return out;
    }

    // --- locale parity with the app's own translations ---
    for tag in &app_locales {
        if !listing.locales.contains_key(tag) {
            out.push(Problem {
                code: "day::lint::store-missing-locale",
                message: format!(
                    "the app is translated into {tag} but {store}/{tag}/ has no listing — the store \
                     shows those users the default language"
                ),
                ..Default::default()
            });
        }
    }
    for tag in listing.locales.keys() {
        if !app_locales.contains(tag) && !app_locales.is_empty() {
            out.push(Problem {
                code: "day::lint::store-orphan-locale",
                message: format!(
                    "{store}/{tag}/ has a listing for a locale the app is not translated into \
                     (resource/locales/{tag}/ does not exist)"
                ),
                ..Default::default()
            });
        }
        if !mappable(tag) {
            out.push(Problem {
                code: "day::lint::store-unmapped-locale",
                message: format!(
                    "{store}/{tag}/: no App Store or Play locale is known for {tag:?} — a listing \
                     uploaded under an unknown locale is dropped without an error"
                ),
                ..Default::default()
            });
        }
    }
    if let Some(def) = default_locale(&app_locales)
        && !listing.locales.contains_key(&def)
    {
        out.push(Problem {
            code: "day::lint::store-default-locale",
            message: format!(
                "store/{def}/ is missing, and {def} is the app's default locale — both stores \
                 require a complete listing in the primary language"
            ),
            ..Default::default()
        });
    }

    // --- per-locale field rules ---
    for (tag, fields) in &listing.locales {
        for f in FIELDS {
            let Some(text) = fields.get(f) else {
                if f.required()
                    && ((to_apple && f.apple().is_some()) || (to_play && f.play().is_some()))
                {
                    out.push(Problem {
                        code: "day::lint::store-missing-field",
                        message: format!("{store}/{tag}/{} is required", f.file()),
                        ..Default::default()
                    });
                }
                continue;
            };
            let chars = text.chars().count();
            for (store, spec, targeted) in [
                ("App Store", f.apple(), to_apple),
                ("Google Play", f.play(), to_play),
            ] {
                let Some((_, limit)) = spec else { continue };
                if targeted && chars > limit {
                    out.push(Problem {
                        code: "day::lint::store-too-long",
                        message: format!(
                            "{store}/{tag}/{}: {chars} characters, {store} allows {limit}",
                            f.file()
                        ),
                        file: Some(format!("{store}/{tag}/{}", f.file())),
                        ..Default::default()
                    });
                }
            }
            if matches!(
                f,
                Field::MarketingUrl | Field::SupportUrl | Field::PrivacyUrl
            ) && !text.starts_with("https://")
            {
                out.push(Problem {
                    code: "day::lint::store-bad-url",
                    message: format!("{store}/{tag}/{}: must be an https:// URL", f.file()),
                    file: Some(format!("{store}/{tag}/{}", f.file())),
                    ..Default::default()
                });
            }
            if *f == Field::Keywords && to_apple {
                // Apple counts the whole string including separators, and a space after a comma is
                // a wasted character rather than a formatting nicety.
                if text.contains(", ") {
                    let file = format!("{store}/{tag}/keywords.txt");
                    out.push(Problem {
                        code: "day::lint::store-bad-keywords",
                        message: format!(
                            "{store}/{tag}/keywords.txt: drop the spaces after commas — the App \
                             Store counts them against the 100-character budget"
                        ),
                        file: Some(file.clone()),
                        fix: Some(crate::lint::Fix {
                            title: "Remove the spaces around commas".into(),
                            contents: tidy_keywords(text),
                            file,
                        }),
                    });
                }
            }
            if text.contains("TODO") {
                out.push(Problem {
                    code: "day::lint::store-placeholder",
                    message: format!(
                        "{store}/{tag}/{}: still the scaffold's TODO — it would upload verbatim",
                        f.file()
                    ),
                    file: Some(format!("{store}/{tag}/{}", f.file())),
                    ..Default::default()
                });
            }
            if text.trim() != text {
                let file = format!("{store}/{tag}/{}", f.file());
                out.push(Problem {
                    code: "day::lint::store-whitespace",
                    message: format!("{store}/{tag}/{}: leading or trailing whitespace", f.file()),
                    file: Some(file.clone()),
                    fix: Some(crate::lint::Fix {
                        title: "Trim the surrounding whitespace".into(),
                        contents: format!("{}\n", text.trim()),
                        file,
                    }),
                });
            }
        }
        // Play requires a support email on the listing; Apple requires a privacy policy URL for
        // every app, and both are easy to forget until the submission is rejected.
        if to_play && !fields.contains_key(&Field::Short) {
            out.push(Problem {
                code: "day::lint::store-missing-field",
                message: format!(
                    "{store}/{tag}/short.txt is required by Google Play (short description)"
                ),
                ..Default::default()
            });
        }
        if to_apple && !fields.contains_key(&Field::PrivacyUrl) {
            out.push(Problem {
                code: "day::lint::store-missing-field",
                message: format!(
                    "{store}/{tag}/privacy-url.txt is required — the App Store rejects an app \
                     without a privacy policy URL"
                ),
                ..Default::default()
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// `day store …`
// ---------------------------------------------------------------------------

/// Skeleton text for a field: what it is for, and the budget it has to fit.
fn skeleton(field: Field, project: &Project, tag: &str) -> Option<String> {
    let title = project
        .manifest
        .app
        .title
        .clone()
        .unwrap_or_else(|| project.manifest.app.name.clone());
    Some(match field {
        Field::Name if tag == "en" => title,
        Field::Name => title,
        Field::Subtitle => "TODO: 30 characters, App Store".into(),
        Field::Short => {
            "TODO: one sentence, up to 80 characters, shown in Google Play search results.".into()
        }
        Field::Description => "TODO: what the app does, who it is for, what it does not do.\n\n\
             Both stores allow 4000 characters and show the first few lines before a fold."
            .into(),
        Field::Keywords => "todo,comma,separated,no,spaces".into(),
        Field::ReleaseNotes => "TODO: what changed in this version (Google Play allows 500 \
                                characters, and that is the limit that binds)"
            .into(),
        // Optional fields are left absent rather than filled with a placeholder that would upload.
        Field::Promo | Field::MarketingUrl => return None,
        Field::SupportUrl | Field::PrivacyUrl => return None,
    })
}

fn init(project: &Project) {
    let locales = app_locales(project);
    let locales = if locales.is_empty() {
        vec!["en".to_string()]
    } else {
        locales
    };
    let root = dir(project);
    let mut written = 0usize;
    let mut skipped = 0usize;
    for tag in &locales {
        for f in FIELDS {
            let Some(body) = skeleton(*f, project, tag) else {
                continue;
            };
            let path = root.join(tag).join(f.file());
            if path.exists() {
                skipped += 1;
                continue;
            }
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::write(&path, format!("{body}\n")).is_ok() {
                written += 1;
            }
        }
    }
    let app_toml = root.join("app.toml");
    if !app_toml.exists() {
        let body = format!(
            "# Store listing metadata that is NOT localized. The localized text lives in\n\
             # store/<locale>/ — one directory per locale in resource/locales/.\n\
             bundle-id = {:?}\n\
             # apple-category = \"DEVELOPER_TOOLS\"   # App Store primary category\n\
             # play-category = \"TOOLS\"              # Google Play category\n\
             # copyright = \"2026 Example\"\n\
             # contact-email = \"support@example.com\"\n\
             # review-notes = \"How to exercise the app, for the store reviewer.\"\n",
            project.manifest.app.id
        );
        if std::fs::write(&app_toml, body).is_ok() {
            written += 1;
        }
    } else {
        skipped += 1;
    }
    crate::ops::status(
        "Listing",
        &format!(
            "{} file(s) written under {}, {skipped} left alone ({} locale(s))",
            written,
            root.display(),
            locales.len()
        ),
    );
    crate::ops::status("Next", "fill in every TODO, then `day lint`");
}

/// The store targets a command acts on: the one named, or every App Store / Google Play target
/// the app declares.
fn store_targets(
    project: &Project,
    want: Option<&str>,
) -> Result<Vec<&'static crate::targets::Target>, CliError> {
    let targets: Vec<&'static crate::targets::Target> = match want {
        Some(name) => match crate::targets::find(name) {
            Some(t) if is_store_target(t) => vec![t],
            Some(t) => {
                return Err(CliError::failure(format!(
                    "{} has no store listing format",
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
            .filter(|t| is_store_target(t))
            .collect(),
    };
    if targets.is_empty() {
        return Err(CliError::failure(
            "this app declares no App Store or Google Play target",
        ));
    }
    Ok(targets)
}

fn stage_cmd(
    project: &Project,
    want: Option<&str>,
    screenshots: Option<&ScreenshotSource>,
) -> Result<(), CliError> {
    let listing = read(project).map_err(CliError::failure)?;
    if listing.is_empty() {
        return Err(CliError::failure(
            "no store/ listing in this project — run `day store init` first",
        ));
    }
    for t in store_targets(project, want)? {
        let out = stage_dir(project, t);
        let files = stage(project, t, &listing, &out, screenshots)
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
) -> Result<(), CliError> {
    let listing = read(project).map_err(CliError::failure)?;
    let theme = listing_theme(&listing);
    let index = source.index().map_err(CliError::failure)?;
    let mut refused = 0;
    for t in store_targets(project, want)? {
        let shots = listing_shots(&index, t, &theme).map_err(CliError::failure)?;
        // One line per device kind: how many captures each locale contributes.
        let mut per_device: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
        for s in &shots {
            *per_device
                .entry(s.device.as_str())
                .or_default()
                .entry(s.locale.as_str())
                .or_default() += 1;
        }
        let summary = if per_device.is_empty() {
            "no capture marked `store: N`".to_string()
        } else {
            per_device
                .iter()
                .map(|(device, locales)| {
                    let counts: Vec<String> =
                        locales.iter().map(|(l, n)| format!("{l} {n}")).collect();
                    format!("{device}: {}", counts.join(", "))
                })
                .collect::<Vec<_>>()
                .join("; ")
        };
        crate::ops::status("Listing", &format!("{} ({theme}): {summary}", t.name));
        for p in screenshot_problems(&index, t, &theme) {
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

/// `day store <init|stage|screenshots>`.
pub fn run(project: &Project, cmd: &crate::cli::StoreCmd) -> Result<(), CliError> {
    match cmd {
        crate::cli::StoreCmd::Init => {
            init(project);
            Ok(())
        }
        crate::cli::StoreCmd::Stage {
            target,
            screenshots,
        } => {
            let source = screenshots.as_deref().map(ScreenshotSource::parse);
            stage_cmd(project, target.as_deref(), source.as_ref())
        }
        crate::cli::StoreCmd::Screenshots { index, target } => {
            screenshots_cmd(project, target.as_deref(), &ScreenshotSource::parse(index))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tidying_keywords_reaches_a_fixed_point() {
        use super::tidy_keywords;
        assert_eq!(tidy_keywords("a, b, c"), "a,b,c\n");
        // Whatever the spacing, one pass is enough: `day lint --fix` re-checks after writing,
        // and a repair that left work behind would report its finding on every run.
        for messy in ["a,  b ,c", "a , b,  c", "a,b,c"] {
            assert_eq!(tidy_keywords(messy), "a,b,c\n", "{messy:?}");
            assert_eq!(tidy_keywords(tidy_keywords(messy).trim()), "a,b,c\n");
        }
    }

    use super::*;

    #[test]
    fn locale_mapping_covers_both_stores_and_their_disagreements() {
        assert_eq!(store_locale("zh-CN", true), Some("zh-Hans"));
        assert_eq!(store_locale("zh-CN", false), Some("zh-CN"));
        // Google Play still spells Hebrew with the pre-1989 code.
        assert_eq!(store_locale("he", false), Some("iw-IL"));
        assert_eq!(store_locale("he", true), Some("he"));
        assert_eq!(store_locale("ar", true), Some("ar-SA"));
        assert_eq!(store_locale("ar", false), Some("ar"));
        assert_eq!(store_locale("kl", true), None, "unknown tag maps nowhere");
        assert!(mappable("fr") && !mappable("kl"));
    }

    #[test]
    fn the_binding_limit_is_the_stricter_store() {
        // Release notes: 4000 on the App Store, 500 on Play. An app shipping to both must fit 500.
        assert_eq!(Field::ReleaseNotes.apple().map(|(_, n)| n), Some(4000));
        assert_eq!(Field::ReleaseNotes.play().map(|(_, n)| n), Some(500));
        // Fields only one store has.
        assert!(Field::Keywords.play().is_none());
        assert!(Field::Short.apple().is_none());
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

        let mut fields = BTreeMap::new();
        fields.insert(Field::Name, "Example".to_string());
        fields.insert(Field::Description, "What it does.".to_string());
        fields.insert(Field::Short, "One line.".to_string());
        fields.insert(Field::ReleaseNotes, "First release.".to_string());
        let mut listing = Listing::default();
        listing.locales.insert("zh-CN".to_string(), fields);

        let ios = crate::targets::find("ios-uikit").expect("ios");
        let out = tmp.join("out-ios");
        stage(&project, ios, &listing, &out, None).expect("stage ios");
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
        stage(&project, android, &listing, &out, None).expect("stage android");
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
        let mut fields = BTreeMap::new();
        fields.insert(Field::Name, "Example".to_string());
        let mut listing = Listing::default();
        listing.locales.insert("en".to_string(), fields);

        let android = crate::targets::find("android-mdc").expect("android");
        stage(&project, android, &listing, &tmp.join("out-android"), None).expect("stage android");
        let play = std::fs::read_to_string(tmp.join("out-android/fastlane/Appfile")).expect("read");
        assert!(
            play.contains("package_name(\"dev.example.app_x\")"),
            "Play takes the resolved android id: {play}"
        );

        let ios = crate::targets::find("ios-uikit").expect("ios");
        stage(&project, ios, &listing, &tmp.join("out-ios"), None).expect("stage ios");
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
        let mut listing = Listing::default();
        for loc in ["en", "fr"] {
            let mut fields = BTreeMap::new();
            fields.insert(Field::Name, "Example".to_string());
            listing.locales.insert(loc.to_string(), fields);
        }

        // A capture tree as the runner leaves it, and the index over it.
        let tree = tmp.join("screenshots");
        let mut entries = Vec::new();
        let captures = [
            ("ios-uikit", "iphone", "light", "en", "home", Some(1)),
            ("ios-uikit", "iphone", "light", "en", "play", Some(2)),
            ("ios-uikit", "iphone", "light", "en", "debug", None),
            ("ios-uikit", "iphone", "dark", "en", "home", Some(1)),
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
                "shot": shot, "device": device, "os": target.split('-').next(),
                "theme": theme, "locale": locale, "store": store, "width": w, "height": h,
            }));
        }
        let index = tree.join("gallery.json");
        std::fs::write(
            &index,
            serde_json::json!({ "locales": ["en", "fr"], "screenshots": entries }).to_string(),
        )
        .expect("index");
        let source = ScreenshotSource::parse(index.to_str().expect("utf-8"));

        let ios = crate::targets::find("ios-uikit").expect("ios");
        let out = tmp.join("out-ios");
        stage(&project, ios, &listing, &out, Some(&source)).expect("stage ios");
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
        stage(&project, android, &listing, &out, Some(&source)).expect("stage android");
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
        stage(&project, android, &listing, &out, None).expect("stage android");
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
        serde_json::json!({
            "shot": shot, "device": device, "os": os, "theme": "light", "locale": locale,
            "store": 1, "width": w, "height": h, "path": format!("gallery/x/{shot}.png"),
        })
    }

    fn index(locales: &[&str], entries: Vec<serde_json::Value>) -> serde_json::Value {
        serde_json::json!({ "locales": locales, "screenshots": entries })
    }

    /// The rules are the stores': Apple takes exact sizes for its two device kinds, Google a
    /// range whose long side is at most twice the short, and each required kind needs a capture
    /// in every locale the index carries.
    #[test]
    fn the_stores_rules_refuse_what_the_stores_refuse() {
        let ios = crate::targets::find("ios-uikit").expect("ios");
        let android = crate::targets::find("android-mdc").expect("android");
        let ok = |t, i: &serde_json::Value| screenshot_problems(i, t, "light");

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

        // The default 20:9 phone profile, refused by the ratio rule and by nothing else.
        let tall = index(
            &["en"],
            vec![marked("android", "phone", "en", "home", 1080, 2400)],
        );
        let problems = ok(android, &tall);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("2.22:1") && problems[0].contains("9:16"),
            "{problems:?}"
        );

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

        // Nothing marked at all, and a device slug the store has no kind for.
        let none = index(&["en"], vec![]);
        assert_eq!(ok(android, &none).len(), 1);
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

        // Only the listing's theme counts: a dark capture at a refused size is not looked at.
        let mut dark = marked("android", "phone", "en", "home", 1080, 2400);
        dark["theme"] = serde_json::json!("dark");
        let themed = index(
            &["en"],
            vec![marked("android", "phone", "en", "home", 1080, 1920), dark],
        );
        assert_eq!(ok(android, &themed), Vec::<String>::new());
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
}
