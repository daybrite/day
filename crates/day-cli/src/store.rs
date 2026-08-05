//! Store listings (docs/store.md): one canonical source per app, generated into the layouts App Store
//! Connect and Google Play expect.
//!
//! An app's listing text is localized user-facing copy, so it lives beside the app's other
//! localized copy — as plain text a translator can edit, under `store/<locale>/`, keyed by the SAME
//! locale tags `resource/locales/` uses. The stores disagree about almost everything else: what the
//! fields are called, how long they may be, and how a locale is spelled (`zh-CN` here is `zh-Hans`
//! to Apple and `zh-CN` to Google; Hebrew is `he` to Apple and the legacy `iw-IL` to Google). All of
//! that divergence is handled here, at generation time, rather than by asking the author to keep two
//! parallel trees in step — the same reason `resource/` fans out to per-platform resources rather
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
//! `build/day/fastlane/<target>/` — a tree `fastlane deliver` / `fastlane supply` accept as-is.
//! Generated, never checked in: the pristine-checkout rule (§20.3) means a build must not write
//! into tracked directories.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
            // Play's changelog is FAR shorter than the App Store's release notes, and it is the
            // limit that binds for an app shipping to both.
            Field::ReleaseNotes => Some(("changelog", 500)),
            Field::MarketingUrl => Some(("video.txt", 255)),
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
/// region-qualified English. A tag missing from this table is a lint error rather than a guess —
/// uploading a listing under a locale the store does not know silently drops it.
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

/// `store/app.toml` — the parts of a listing that are not localized.
#[derive(Debug, Clone, Default)]
pub struct AppMeta {
    /// Play package name / App Store bundle id. Defaults to `[app] id`.
    pub bundle_id: Option<String>,
    /// App Store primary category, e.g. `DEVELOPER_TOOLS` (deliver's `primary_category`).
    ///
    /// There is deliberately no Play counterpart: Google Play's category is set in the Play
    /// Console and `supply` cannot write it, so recording one here would be a value that silently
    /// never reached the store.
    pub apple_category: Option<String>,
    pub copyright: Option<String>,
    /// Where the store writes back to a human: review contact, support email.
    pub contact_email: Option<String>,
    /// Free-form notes for the reviewer (deliver's `review_information/notes.txt`).
    pub review_notes: Option<String>,
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

/// The project's `store/` directory.
pub fn dir(project: &Project) -> PathBuf {
    project.root.join("store")
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
        review_notes: get("review-notes"),
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
        // The pseudolocale is a development aid, never a store listing.
        .filter(|t| t != "en-XA")
        .collect();
    out.sort();
    out
}

/// The default locale: `en` when present, else the first — the same rule day-build applies to
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
/// two lanes a release needs — a dry run that validates against the store without publishing, and
/// the upload itself.
pub fn stage(
    project: &Project,
    target: &'static crate::targets::Target,
    listing: &Listing,
    out: &Path,
) -> Result<Vec<PathBuf>, String> {
    let apple = match target.toolkit {
        "uikit" => true,
        "mdc" => false,
        other => return Err(format!("{other} has no store listing format")),
    };
    let _ = std::fs::remove_dir_all(out);
    let mut written = Vec::new();
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

    let id = listing
        .app
        .bundle_id
        .clone()
        .unwrap_or_else(|| project.manifest.app.id.clone());
    if apple {
        if let Some(c) = &listing.app.copyright {
            write("fastlane/metadata/copyright.txt", &format!("{c}\n"))?;
        }
        if let Some(c) = &listing.app.apple_category {
            write("fastlane/metadata/primary_category.txt", &format!("{c}\n"))?;
        }
        if let Some(n) = &listing.app.review_notes {
            write(
                "fastlane/metadata/review_information/notes.txt",
                &format!("{n}\n"),
            )?;
        }
        if let Some(e) = &listing.app.contact_email {
            write(
                "fastlane/metadata/review_information/email_address.txt",
                &format!("{e}\n"),
            )?;
        }
        write("fastlane/Appfile", &apple_appfile(&id, &listing.app))?;
        write("fastlane/Fastfile", &apple_fastfile(project))?;
    } else {
        write("fastlane/Appfile", &play_appfile(&id))?;
        write("fastlane/Fastfile", &play_fastfile(project))?;
    }
    written.sort();
    Ok(written)
}

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
         #   ASC_KEY_ID, ASC_ISSUER_ID, ASC_KEY (the .p8 contents)\n",
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

fn apple_fastfile(_project: &Project) -> String {
    // The artifact is found by glob, not by name: `day pack` names an unsigned device build
    // `<app>-unsigned.ipa` and a signed one `<app>.ipa`, and a lane that hardcoded one of those
    // would break on exactly the day signing was configured.
    r##"# Generated by `day store stage` — edit store/ in the project, not this file.
# The .ipa comes from `day pack -p ios-uikit`; this only uploads what that produced.
default_platform(:ios)

# __dir__ is <project>/build/day/store/<target>/fastlane, so ../../../dist is build/day/dist —
# where `day pack` puts its output.
def day_ipa
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
end
"##
    .to_string()
}

fn play_fastfile(_project: &Project) -> String {
    r##"# Generated by `day store stage` — edit store/ in the project, not this file.
# The .aab comes from `day pack -p android-mdc`; this only uploads what that produced.
default_platform(:android)

# __dir__ is <project>/build/day/store/<target>/fastlane, so ../../../dist is build/day/dist.
def day_json_key
  key = ENV["SUPPLY_JSON_KEY"].to_s
  UI.user_error!("SUPPLY_JSON_KEY is not set — the path to the Play service-account JSON") if key.empty?
  key
end

def day_aab
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
end
"##
    .to_string()
}

/// Where a target's fastlane project is written: `build/day/store/<target>/`, holding the
/// `fastlane/` folder the tool insists on finding (it locates its config by folder, not by file —
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
pub struct Problem {
    pub code: &'static str,
    pub message: String,
}

/// Check a listing against the stores the app targets.
///
/// Silent when the app ships to neither store, and when it has no `store/` at all — an app that
/// never leaves a developer's machine should not be nagged about App Store copy. Once `store/`
/// exists, it is held to the stores' rules, because the alternative is finding out at upload time.
pub fn lint(project: &Project, listing: &Listing) -> Vec<Problem> {
    let mut out = Vec::new();
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
                    "this app ships to {} but has no store/ listing — run `day store init`",
                    if to_apple && to_play {
                        "the App Store and Google Play"
                    } else if to_apple {
                        "the App Store"
                    } else {
                        "Google Play"
                    }
                ),
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
                    "the app is translated into {tag} but store/{tag}/ has no listing — the store \
                     shows those users the default language"
                ),
            });
        }
    }
    for tag in listing.locales.keys() {
        if !app_locales.contains(tag) && !app_locales.is_empty() {
            out.push(Problem {
                code: "day::lint::store-orphan-locale",
                message: format!(
                    "store/{tag}/ has a listing for a locale the app is not translated into \
                     (resource/locales/{tag}/ does not exist)"
                ),
            });
        }
        if !mappable(tag) {
            out.push(Problem {
                code: "day::lint::store-unmapped-locale",
                message: format!(
                    "store/{tag}/: no App Store or Play locale is known for {tag:?} — a listing \
                     uploaded under an unknown locale is dropped without an error"
                ),
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
                        message: format!("store/{tag}/{} is required", f.file()),
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
                            "store/{tag}/{}: {chars} characters, {store} allows {limit}",
                            f.file()
                        ),
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
                    message: format!("store/{tag}/{}: must be an https:// URL", f.file()),
                });
            }
            if *f == Field::Keywords && to_apple {
                // Apple counts the whole string including separators, and a space after a comma is
                // a wasted character rather than a formatting nicety.
                if text.contains(", ") {
                    out.push(Problem {
                        code: "day::lint::store-bad-keywords",
                        message: format!(
                            "store/{tag}/keywords.txt: drop the spaces after commas — the App \
                             Store counts them against the 100-character budget"
                        ),
                    });
                }
            }
            if text.contains("TODO") {
                out.push(Problem {
                    code: "day::lint::store-placeholder",
                    message: format!(
                        "store/{tag}/{}: still the scaffold's TODO — it would upload verbatim",
                        f.file()
                    ),
                });
            }
            if text.trim() != text {
                out.push(Problem {
                    code: "day::lint::store-whitespace",
                    message: format!("store/{tag}/{}: leading or trailing whitespace", f.file()),
                });
            }
        }
        // Play requires a support email on the listing; Apple requires a privacy policy URL for
        // every app, and both are easy to forget until the submission is rejected.
        if to_play && !fields.contains_key(&Field::Short) {
            out.push(Problem {
                code: "day::lint::store-missing-field",
                message: format!(
                    "store/{tag}/short.txt is required by Google Play (short description)"
                ),
            });
        }
        if to_apple && !fields.contains_key(&Field::PrivacyUrl) {
            out.push(Problem {
                code: "day::lint::store-missing-field",
                message: format!(
                    "store/{tag}/privacy-url.txt is required — the App Store rejects an app \
                     without a privacy policy URL"
                ),
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

fn init(project: &Project) -> i32 {
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
    0
}

fn stage_cmd(project: &Project, want: Option<&str>) -> i32 {
    let listing = match read(project) {
        Ok(l) => l,
        Err(e) => {
            crate::ops::status("Error", &e);
            return 1;
        }
    };
    if listing.is_empty() {
        crate::ops::status(
            "Error",
            "no store/ listing in this project — run `day store init` first",
        );
        return 1;
    }
    let targets: Vec<&'static crate::targets::Target> = match want {
        Some(name) => match crate::targets::find(name) {
            Some(t) if is_store_target(t) => vec![t],
            Some(t) => {
                crate::ops::status("Error", &format!("{} has no store listing format", t.name));
                return 1;
            }
            None => {
                crate::ops::status("Error", &format!("unknown target {name:?}"));
                return 1;
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
        crate::ops::status(
            "Error",
            "this app declares no App Store or Google Play target",
        );
        return 1;
    }
    for t in targets {
        let out = stage_dir(project, t);
        match stage(project, t, &listing, &out) {
            Ok(files) => crate::ops::status(
                "Staged",
                &format!("{} ({} file(s)) for {}", out.display(), files.len(), t.name),
            ),
            Err(e) => {
                crate::ops::status("Error", &format!("{}: {e}", t.name));
                return 1;
            }
        }
    }
    0
}

/// `day store <init|stage>`.
pub fn run(project: &Project, cmd: &crate::cli::StoreCmd) -> i32 {
    match cmd {
        crate::cli::StoreCmd::Init => init(project),
        crate::cli::StoreCmd::Stage { target } => stage_cmd(project, target.as_deref()),
    }
}

#[cfg(test)]
mod tests {
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
    /// finding a `fastlane` FOLDER, each store's own file names differ from Day's, and Play keys the
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
        stage(&project, ios, &listing, &out).expect("stage ios");
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

        let android = crate::targets::find("android-mdc").expect("android");
        let out = tmp.join("out-android");
        stage(&project, android, &listing, &out).expect("stage android");
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
