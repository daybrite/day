// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Day.toml — the project manifest (DESIGN.md §17.3).
//!
//! Follows the Tauri / Dioxus model: a dedicated manifest file that doubles as the project
//! marker (`find_project` walks up to the nearest `Day.toml`). Two rules keep it honest:
//!
//! * **Derive, don't restate**: `name` and `version` come from the sibling `Cargo.toml`'s
//!   `[package]` — they are never written in Day.toml, so app identity can't drift from the
//!   crate's.
//! * **Base + overrides**: `[app]` holds the base properties; any of them can be overridden
//!   per platform (`[app.ios]`), per toolkit (`[app.qt]`), or per full target
//!   (`[app.macos-appkit]`) — most specific wins (see [`Manifest::resolve`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Manifest schema version (currently 1).
    pub schema: u32,
    pub app: App,
    #[serde(default)]
    pub window: Window,
    /// Code-signing / notarization configuration (§16.5, §17.3). Values may reference environment
    /// variables as `${VAR}` — resolved at use time (see `pack::settings::interpolate`), never at
    /// parse time, so `day sign --check` can report missing variables without failing the parse.
    #[serde(default)]
    pub signing: Option<Signing>,
    /// OS permissions this app declares, and the user-facing reason for each (docs/permissions.md).
    /// `day build` turns these into `<uses-permission>` entries, `Info.plist` usage descriptions,
    /// and HarmonyOS `requestPermissions` — the declaration every mobile OS requires before the app
    /// may even ask. `#[serde(default)]`, so every Day.toml written before this existed still parses.
    #[serde(default)]
    pub permissions: Permissions,
    /// `[[shortcuts]]` — launcher shortcuts: labeled, persistent deep links shown on a
    /// long-press of the app's icon (docs/deep-links.md "Shortcuts are saved deep links").
    /// `day build` conveys these into each platform's native declaration — iOS
    /// `UIApplicationShortcutItems`, Android `res/xml` shortcuts, HarmonyOS
    /// `shortcuts_config.json` — with labels resolved per locale from `resource/locales/`.
    #[serde(default)]
    pub shortcuts: Vec<Shortcut>,
    /// `[sbom]` — whether to produce a software bill of materials, in which formats, and whether it
    /// ships inside the app or beside it (§20.4). Defaults to two sidecar documents: sidecars cost
    /// the artifact nothing, whereas embedding both formats adds roughly 400 KB.
    #[serde(default)]
    pub sbom: SbomConfig,
}

/// Where a generated SBOM goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SbomMode {
    /// Write the documents next to the artifact, like the `.buildinfo` sidecar.
    #[default]
    Sidecar,
    /// Stage the documents inside the app so it can read them at runtime — a license screen, say.
    Embed,
    /// Produce nothing.
    None,
}

/// One SBOM serialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SbomFormat {
    /// CycloneDX 1.5 JSON.
    #[serde(alias = "cdx")]
    Cyclonedx,
    /// SPDX 2.3 JSON.
    Spdx,
}

impl SbomFormat {
    /// The name the document carries INSIDE an app bundle (`sbom = "embed …"`), and in the
    /// generator's own staging directory. Fixed, because an embedded document is looked up by
    /// name at runtime and by `day rebuild` inside a downloaded container.
    pub fn file_name(self) -> &'static str {
        match self {
            SbomFormat::Cyclonedx => "day-sbom.cdx.json",
            SbomFormat::Spdx => "day-sbom.spdx.json",
        }
    }

    /// The suffix a SIDECAR copy carries, appended to the artifact's own file name
    /// (`day-showcase-macos-appkit.dmg.sbom-cdx.json`). Sidecars sit in one release directory
    /// alongside every other target's, so each has to say which artifact it describes (§20.4).
    pub fn sidecar_suffix(self) -> &'static str {
        match self {
            SbomFormat::Cyclonedx => "sbom-cdx.json",
            SbomFormat::Spdx => "sbom-spdx.json",
        }
    }
}

/// `[sbom]`, accepted either as a table or as the shorthand string `"<mode> <format>…"`:
///
/// ```toml
/// sbom = "embed spdx"                    # one embedded SPDX document
/// sbom = "sidecar spdx cyclonedx"        # both, beside the artifact
/// sbom = "none"                          # generate nothing
///
/// [sbom]                                 # the same thing, spelled out
/// mode = "embed"
/// formats = ["spdx"]
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SbomConfig {
    pub mode: SbomMode,
    pub formats: Vec<SbomFormat>,
}

impl Default for SbomConfig {
    fn default() -> Self {
        Self {
            mode: SbomMode::Sidecar,
            formats: vec![SbomFormat::Cyclonedx, SbomFormat::Spdx],
        }
    }
}

impl SbomConfig {
    /// True when nothing should be produced.
    pub fn is_off(&self) -> bool {
        self.mode == SbomMode::None || self.formats.is_empty()
    }
}

impl<'de> serde::Deserialize<'de> for SbomConfig {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Table {
            #[serde(default)]
            mode: SbomMode,
            #[serde(default)]
            formats: Option<Vec<SbomFormat>>,
        }
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Either {
            Shorthand(String),
            Table(Table),
        }
        match Either::deserialize(d)? {
            Either::Table(t) => Ok(SbomConfig {
                mode: t.mode,
                formats: t.formats.unwrap_or_else(|| Self::default().formats),
            }),
            Either::Shorthand(text) => {
                let mut words = text.split_whitespace();
                let mode = match words.next() {
                    Some("sidecar") => SbomMode::Sidecar,
                    Some("embed") => SbomMode::Embed,
                    Some("none") => SbomMode::None,
                    other => {
                        return Err(serde::de::Error::custom(format!(
                            "sbom: expected `sidecar`, `embed`, or `none`, got {other:?}"
                        )));
                    }
                };
                let mut formats = Vec::new();
                for w in words {
                    match w {
                        "cyclonedx" | "cdx" => formats.push(SbomFormat::Cyclonedx),
                        "spdx" => formats.push(SbomFormat::Spdx),
                        other => {
                            return Err(serde::de::Error::custom(format!(
                                "sbom: unknown format {other:?} (expected `spdx` or `cyclonedx`)"
                            )));
                        }
                    }
                }
                if formats.is_empty() {
                    formats = Self::default().formats;
                }
                Ok(SbomConfig { mode, formats })
            }
        }
    }
}

/// `[permissions]`. Every key is a portable permission name from `day_build::permissions` except the
/// reserved `raw`, which carries per-platform escape hatches.
///
/// This struct carries no `deny_unknown_fields` because the `flatten` map has to absorb the
/// permission keys (the same reason [`App`] doesn't) — [`parse_manifest`] validates the names
/// instead, and rejects a typo with the list of valid ones, which is the better error anyway.
#[derive(Debug, Default, Deserialize)]
pub struct Permissions {
    #[serde(default)]
    pub raw: RawPermissions,
    #[serde(flatten)]
    pub declared: BTreeMap<String, Declaration>,
}

/// How one permission is declared. The short forms cover the common cases:
/// `camera = "Attach photos to your notes."` and `notifications = true`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Declaration {
    /// `<name> = "<reason>"`
    Reason(String),
    /// `<name> = true` declares a permission that needs no reason on any platform (notifications).
    /// `<name> = false` is the opposite: an explicit "not this one", useful to hold the line against
    /// a permission a dependency might otherwise pull in.
    Enabled(bool),
    /// The long form, for per-platform reason overrides or a platform subset.
    Detailed(Box<DeclarationTable>),
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct DeclarationTable {
    pub reason: Option<String>,
    pub ios_reason: Option<String>,
    pub macos_reason: Option<String>,
    pub ohos_reason: Option<String>,
    /// Restrict the declaration to a subset of `ios` / `macos` / `android` / `ohos`. Default: every
    /// platform the permission maps to.
    pub platforms: Option<Vec<String>>,
}

impl Declaration {
    /// The reason to use for `platform`, most specific first.
    pub fn reason_for(&self, platform: &str) -> Option<&str> {
        match self {
            Declaration::Reason(r) => Some(r.as_str()),
            Declaration::Enabled(_) => None,
            Declaration::Detailed(t) => {
                let specific = match platform {
                    "ios" => t.ios_reason.as_deref(),
                    "macos" => t.macos_reason.as_deref(),
                    "ohos" => t.ohos_reason.as_deref(),
                    _ => None,
                };
                specific.or(t.reason.as_deref())
            }
        }
    }

    /// Whether the app actually wants this permission. `<name> = false` declares that it does not.
    pub fn enabled(&self) -> bool {
        !matches!(self, Declaration::Enabled(false))
    }

    /// Whether this declaration applies to `platform`.
    pub fn covers(&self, platform: &str) -> bool {
        match self {
            Declaration::Detailed(t) => match &t.platforms {
                Some(list) => list.iter().any(|p| p == platform),
                None => true,
            },
            _ => true,
        }
    }
}

/// `[permissions.raw]` — platform-native declarations for anything outside the portable set.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPermissions {
    /// Android permission ids, e.g. `"android.permission.READ_CONTACTS"`.
    #[serde(default)]
    pub android: Vec<String>,
    /// `Info.plist` key → usage description.
    #[serde(default)]
    pub ios: BTreeMap<String, String>,
    #[serde(default)]
    pub macos: BTreeMap<String, String>,
    /// HarmonyOS entries, each needing its own reason and scene.
    #[serde(default)]
    pub ohos: Vec<RawOhosPermission>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawOhosPermission {
    pub name: String,
    #[serde(default)]
    pub reason: Option<String>,
    /// `"inuse"` (default) or `"always"`.
    #[serde(default)]
    pub when: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signing {
    #[serde(default)]
    pub macos: Option<MacosSigning>,
    #[serde(default)]
    pub ios: Option<IosSigning>,
    #[serde(default)]
    pub android: Option<AndroidSigning>,
    #[serde(default)]
    pub windows: Option<WindowsSigning>,
    #[serde(default)]
    pub ohos: Option<OhosSigning>,
}

/// macOS Developer-ID signing + notarization (§16.5: codesign + notarytool + stapler).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct MacosSigning {
    /// Signing identity ("Developer ID Application: …"); "-" or absent = ad-hoc (dev tier).
    #[serde(default)]
    pub identity: Option<String>,
    /// Entitlements plist path, relative to the project root.
    #[serde(default)]
    pub entitlements: Option<String>,
    #[serde(default)]
    pub notarize: Option<Notarize>,
}

/// notarytool App Store Connect API-key auth (never interactive Apple-ID — §16.5).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Notarize {
    pub key_id: String,
    pub issuer: String,
    /// Path to the AuthKey_<id>.p8 file.
    pub key_path: String,
}

/// iOS App Store export signing: xcodebuild automatic signing with an ASC API key.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct IosSigning {
    /// Apple Developer team id (DEVELOPMENT_TEAM).
    pub team: String,
    /// ExportOptions method; default "app-store-connect".
    #[serde(default)]
    pub export_method: Option<String>,
    /// ASC API key for `-allowProvisioningUpdates` in CI (optional locally, where the
    /// Xcode-account session signs). All three fields travel together.
    #[serde(default)]
    pub key_id: Option<String>,
    #[serde(default)]
    pub issuer: Option<String>,
    #[serde(default)]
    pub key_path: Option<String>,
}

/// Android release keystore (Gradle signingConfig; .aab is jar-signed by Gradle — §16.5).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct AndroidSigning {
    pub keystore: String,
    pub key_alias: String,
    pub store_pass: String,
    pub key_pass: String,
}

/// Windows Authenticode: certs are HSM/service-held since 2023 — a provider enum, not a .pfx path.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct WindowsSigning {
    /// "self-signed-dev" | "signtool-cert-store" | "azure-artifact-signing"
    pub provider: String,
    /// Cert subject for the MSIX Identity Publisher (must byte-match the signing cert subject).
    #[serde(default)]
    pub publisher: Option<String>,
    /// signtool-cert-store: SHA-1 thumbprint of the installed certificate.
    #[serde(default)]
    pub thumbprint: Option<String>,
    /// azure-artifact-signing: endpoint / account / certificate-profile (+ dlib path).
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    /// Path to Azure.CodeSigning.Dlib.dll (azure-artifact-signing).
    #[serde(default)]
    pub dlib: Option<String>,
    /// RFC-3161 timestamp URL; defaults per provider.
    #[serde(default)]
    pub timestamp_url: Option<String>,
}

/// OpenHarmony release signing material (hap-sign-tool).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct OhosSigning {
    /// .p12 keystore path.
    pub keystore: String,
    pub key_alias: String,
    pub store_pass: String,
    pub key_pass: String,
    /// Release certificate (.cer) path.
    pub cert: String,
    /// Provisioning profile (.p7b) path.
    pub profile: String,
}

/// `[app]`: the Day-specific app identity. `name`/`version` are FILLED FROM Cargo.toml after
/// parsing (never written in Day.toml). Every other property can be overridden per platform /
/// toolkit / target via `[app.<key>]` tables collected in `overrides`.
#[derive(Debug, Deserialize)]
pub struct App {
    /// The crate name, from Cargo.toml `[package] name`.
    #[serde(skip)]
    pub name: String,
    /// The crate version, from Cargo.toml `[package] version`.
    #[serde(skip)]
    pub version: String,
    /// Application id / bundle id (reverse-DNS).
    pub id: String,
    /// Display title (window / app store); default: the crate name.
    #[serde(default)]
    pub title: Option<String>,
    /// The filename stem every packaged artifact shares, before the `-<target>` suffix
    /// (`day-showcase` → `day-showcase-macos-appkit.dmg`). Default: [`slug`] of `title`.
    /// Slugged on use, so a value with spaces or capitals still yields a safe filename.
    #[serde(default)]
    pub artifact: Option<String>,
    /// The deep-link URI scheme (`showcase://<route>`, docs/deep-links.md). Default: the last
    /// segment of [`id`](App::id), which is what a scaffold's is.
    ///
    /// Declarable because a scheme is a PUBLISHED contract: links already in the world stop
    /// resolving if it moves. Apps scaffolded before the default existed derived theirs from the
    /// crate name instead (`Day-Showcase` ⇒ `dayshowcase`, not `showcase`), so they name it here
    /// and keep the scheme they shipped with.
    #[serde(default)]
    pub scheme: Option<String>,
    /// Monotonic build number (versionCode / CFBundleVersion).
    #[serde(default = "default_build")]
    pub build: u64,
    /// The platform-toolkit combos this app ships on (`day app add-toolkit` appends here).
    #[serde(default)]
    pub targets: Vec<String>,
    /// `[app.<platform|toolkit|target>]` override tables — validated by `day lint`.
    /// (serde note: this flatten map is why App has no deny_unknown_fields — a typo'd scalar
    /// key still errors because it can't parse as an override TABLE.)
    #[serde(flatten)]
    pub overrides: BTreeMap<String, AppOverride>,
}

/// One `[app.<key>]` override table: any subset of the overridable `[app]` properties.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppOverride {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artifact: Option<String>,
    #[serde(default)]
    pub scheme: Option<String>,
    #[serde(default)]
    pub build: Option<u64>,
}

/// One `[[shortcuts]]` entry — a launcher shortcut. Declaration order is display order on
/// every platform that shows an order.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shortcut {
    /// The day route the shortcut opens — the part after `scheme://`, query params allowed
    /// (`mail/inbox?filter=unread`). Validated by `day lint`'s unknown-route check.
    pub route: String,
    /// The Fluent message id for the user-visible label. Must be a single-line message with
    /// no placeables, present in every locale under `resource/locales/`.
    pub label: String,
}

/// The app identity a specific target builds with, after applying `[app.<key>]` overrides.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResolvedApp {
    pub name: String,
    pub version: String,
    pub id: String,
    pub title: String,
    /// The packaged-artifact filename stem, already slugged (`day-showcase`). Release CI reads
    /// this out of `day metadata --json` to name the web-dom zip, which no `day pack` produces.
    pub artifact: String,
    pub build: u64,
    /// `Day.toml [app] scheme` as declared for this target, if it was. `None` means "derive it"
    /// — see [`ResolvedApp::scheme`].
    pub scheme: Option<String>,
}

impl ResolvedApp {
    /// The app's deep-link URI scheme: `Day.toml [app] scheme` where declared, else the last
    /// segment of the bundle id — lowercased and stripped to what a scheme may contain
    /// (ALPHA/DIGIT/`+`/`-`/`.`, RFC 3986).
    ///
    /// Every platform gets it through that platform's generated channel — `DAY_URL_SCHEME` in
    /// the xcconfig, `scheme` in day-app.properties, the `uris` entry in module.json5 — so no
    /// scaffolded file spells it out (docs/deep-links.md).
    ///
    /// The DEFAULT is derived, so a new app never states it twice; declaring it is how an app
    /// keeps a scheme it already published when that differs from its id (an app scaffolded
    /// before this derived theirs from the crate name).
    pub fn scheme(&self) -> String {
        if let Some(declared) = self.scheme.as_deref().map(str::trim)
            && !declared.is_empty()
        {
            return declared.to_string();
        }
        let last = self.id.rsplit('.').next().unwrap_or_default();
        let s: String = last
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            .collect::<String>()
            .to_lowercase();
        // A scheme must start with a letter; an id ending in digits (or nothing usable) falls
        // back to the same constant `day new` used when it could make no scheme from the name.
        if s.starts_with(|c: char| c.is_ascii_alphabetic()) {
            s
        } else {
            "dayapp".to_string()
        }
    }
}

impl Manifest {
    /// Resolve the app identity for `target` (e.g. `macos-appkit`). Override precedence, most
    /// specific wins: `[app.<target>]` > `[app.<platform>]` > `[app.<toolkit>]` > `[app]`.
    pub fn resolve(&self, target: &str) -> ResolvedApp {
        let mut out = ResolvedApp {
            name: self.app.name.clone(),
            version: self.app.version.clone(),
            id: self.app.id.clone(),
            title: self
                .app
                .title
                .clone()
                .unwrap_or_else(|| self.app.name.clone()),
            // Filled in after the override loop: its default is derived from `title`, which the
            // loop may itself override.
            artifact: String::new(),
            build: self.app.build,
            scheme: self.app.scheme.clone(),
        };
        let mut artifact = self.app.artifact.clone();
        // `[app.ohos]` is the platform table for harmony-arkui — for BUILTIN targets the key
        // comes from the catalog (`Target::os`), never from splitting the name. An externally
        // declared target (docs/extending.md) is the opposite by contract: its os IS the name
        // prefix (there is no override key), so splitting is exact there, and this method has no
        // project to resolve the external catalog through anyway.
        let platform = crate::targets::find(target)
            .map(|t| t.os)
            .unwrap_or_else(|| target.split_once('-').map(|(os, _)| os).unwrap_or_default());
        let toolkit = target.split_once('-').map(|(_, t)| t).unwrap_or_default();
        // `[app.ohos]` is the pre-rename spelling of the harmony platform key — still read,
        // below the modern key in precedence.
        let legacy = if platform == "harmony" { "ohos" } else { "" };
        // Increasing precedence: toolkit, then platform (legacy spelling first), then the
        // exact target.
        for key in [toolkit, legacy, platform, target] {
            if key.is_empty() {
                continue;
            }
            if let Some(o) = self.app.overrides.get(key) {
                if let Some(id) = &o.id {
                    out.id = id.clone();
                }
                if let Some(title) = &o.title {
                    out.title = title.clone();
                }
                if let Some(s) = &o.scheme {
                    out.scheme = Some(s.clone());
                }
                if let Some(a) = &o.artifact {
                    artifact = Some(a.clone());
                }
                if let Some(build) = o.build {
                    out.build = build;
                }
            }
        }
        out.artifact = slug(artifact.as_deref().unwrap_or(&out.title));
        out
    }
}

fn default_build() -> u64 {
    1
}

/// A filename-safe slug: lowercase ASCII alphanumerics, every other run folded to a single `-`,
/// with no leading or trailing `-` (`"Day Showcase"` → `day-showcase`).
///
/// Every packaged artifact's name goes through this. Release assets are served from URLs and
/// listed by shells, and GitHub rewrites a space in an uploaded asset name to a dot
/// (`Day Skies.dmg` → `Day.Skies.dmg`) — so the safe name is chosen here rather than left to
/// whatever a `title` happens to contain. A slug that folds away to nothing (a title of only
/// punctuation, say) yields `app`, because a file still needs a name.
pub fn slug(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "app".to_string()
    } else {
        trimmed.to_string()
    }
}

/// `[window]`: the app's window geometry, in points/dp.
///
/// One declaration, two layers (docs/size-classes.md "Declaring a minimum size"). The MINIMUM has
/// to reach the platform at two different moments: Android wants it in the manifest at BUILD time
/// (`<layout android:minWidth>`, which is what desktop windowing and split-screen honor), iOS
/// wants it at RUN time (`UIWindowScene.sizeRestrictions`). `day build` conveys these values into
/// both, so an app states them once here rather than in two platform files that drift.
#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    #[serde(default = "default_w")]
    pub width: f64,
    #[serde(default = "default_h")]
    pub height: f64,
    /// The narrowest the app can usefully be drawn. Below this a platform that honors it stops
    /// shrinking the window; where a platform treats it as a preference (iOS says so explicitly)
    /// the app still has to lay out sensibly at whatever it gets.
    #[serde(default = "default_min_w")]
    pub min_width: f64,
    #[serde(default = "default_min_h")]
    pub min_height: f64,
}

impl Default for Window {
    fn default() -> Self {
        Window {
            width: default_w(),
            height: default_h(),
            min_width: default_min_w(),
            min_height: default_min_h(),
        }
    }
}

fn default_w() -> f64 {
    480.0
}
fn default_h() -> f64 {
    640.0
}
/// The narrowest phone Day targets is 320dp (an iPhone SE is 320pt, a small Android phone the
/// same), so a window narrower than that is one no app in this family has ever been laid out for.
fn default_min_w() -> f64 {
    320.0
}
/// Android's own `<layout>` minimum floor for a freeform window is 220dp; 400 keeps a compact
/// window tall enough for a nav bar, one row of content and a keyboard.
fn default_min_h() -> f64 {
    400.0
}

pub struct Project {
    pub root: PathBuf,
    pub manifest: Manifest,
}

impl Project {
    /// The app's LIB TARGET name — what cargo names every artifact after: `libdayapp.a`,
    /// `libdayapp.so`, `dayapp.wasm`, and the `dayapp::` path `src/main.rs` imports.
    ///
    /// A scaffolded app pins this to the constant `dayapp` (`[lib] name`, DESIGN.md §17.5) so no
    /// artifact carries the package name and renaming the app moves nothing. An app from before
    /// that pin declares no `[lib] name`, and cargo falls back to the package's with `-` → `_`.
    ///
    /// Read here rather than guessed at each call site: deriving it from the package name is
    /// exactly the assumption that broke the web-dom build when the pin landed (it went looking
    /// for `day_showcase.wasm` beside the `dayapp.wasm` cargo had just written).
    pub fn lib_name(&self) -> String {
        let text = std::fs::read_to_string(self.root.join("Cargo.toml")).unwrap_or_default();
        if let Some(rest) = text.split("[lib]").nth(1) {
            // Only the `[lib]` table's own keys — stop at the next table header.
            let table = rest.split("\n[").next().unwrap_or(rest);
            for line in table.lines() {
                if let Some(v) = line.trim().strip_prefix("name")
                    && let Some(v) = v.trim_start().strip_prefix('=')
                {
                    return v.trim().trim_matches('"').replace('-', "_");
                }
            }
        }
        self.manifest.app.name.replace('-', "_")
    }
}

/// On Windows `std::fs::canonicalize` returns an extended-length `\\?\` (verbatim) path. That prefix
/// flows into `CARGO_TARGET_DIR` (ops.rs), and the windows-gnu toolchain's MinGW linker
/// (`ld`/`collect2`) can't parse `\\?\` object-file arguments — it drops the prefix and reports
/// `cannot find \\symbols.o`, failing the link (hit on windows-gtk / windows-qt; MSVC's link.exe
/// tolerates it, so xaml was unaffected). De-verbatim the path so every subtool gets a plain
/// absolute path — still absolute, so the xcodebuild-SYMROOT need in `find_project` holds. No-op off
/// Windows, where canonicalize never adds a verbatim prefix.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    #[cfg(windows)]
    if let Some(s) = p.to_str() {
        // `\\?\UNC\server\share` → `\\server\share`; `\\?\D:\path` → `D:\path`.
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    p
}

/// Check `[permissions]` against the declaration table, with messages a human can act on.
///
/// This runs over the raw TOML BEFORE the typed parse on purpose. `Declaration` is an untagged
/// enum, so serde reports any malformed entry as "data did not match any variant of untagged enum
/// Declaration" — which names neither the permission nor the key at fault. Both mistakes it catches
/// are the same class: a permission that silently fails to be declared is a crash on iOS.
fn validate_permissions(day_toml: &str) -> Result<(), String> {
    /// The long form's keys, in `DeclarationTable`'s kebab-case spelling.
    const LONG_FORM: &[&str] = &[
        "reason",
        "ios-reason",
        "macos-reason",
        "ohos-reason",
        "platforms",
    ];

    let raw: toml::Value = toml::from_str(day_toml).map_err(|e| format!("Day.toml: {e}"))?;
    let Some(perms) = raw.get("permissions").and_then(|v| v.as_table()) else {
        return Ok(());
    };
    for (key, value) in perms {
        if key == "raw" {
            continue; // the escape hatch, checked by its own deny_unknown_fields
        }
        if day_build::permissions::find(key).is_none() {
            return Err(format!(
                "Day.toml: [permissions] {key:?} is not a known permission (valid: {})",
                day_build::permissions::names().join(", ")
            ));
        }
        if let Some(table) = value.as_table() {
            for k in table.keys() {
                if !LONG_FORM.contains(&k.as_str()) {
                    return Err(format!(
                        "Day.toml: [permissions.{key}] has unknown key {k:?} (valid: {})",
                        LONG_FORM.join(", ")
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Parse Day.toml text + the sibling Cargo.toml's `[package]` into a Manifest.
pub fn parse_manifest(
    day_toml: &str,
    cargo_toml: &str,
    workspace_version: Option<&str>,
) -> Result<Manifest, String> {
    // Before the typed parse: serde's untagged `Declaration` turns any mistake in [permissions]
    // into an unactionable "data did not match any variant".
    validate_permissions(day_toml)?;
    let mut manifest: Manifest = toml::from_str(day_toml).map_err(|e| format!("Day.toml: {e}"))?;
    if manifest.schema != 1 {
        return Err(format!(
            "Day.toml: unsupported schema version {}",
            manifest.schema
        ));
    }
    // `name`/`version` are derived, never restated (a permissive parse: version may be
    // workspace-inherited in exotic layouts — fall back rather than fail).
    let cargo: toml::Value = toml::from_str(cargo_toml).map_err(|e| format!("Cargo.toml: {e}"))?;
    let package = cargo
        .get("package")
        .ok_or("Cargo.toml: no [package] table")?;
    manifest.app.name = package
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Cargo.toml: no package.name")?
        .to_string();
    // `version.workspace = true` is the common shape for an app inside a workspace — including
    // every app in this repo — so the inherited value is resolved, not defaulted. Silently
    // falling back to a literal is worse than it looks: `app.version` stamps CFBundleShort
    // VersionString, Android's versionName, the MSIX/NSIS version and the .hap's, so a wrong
    // one ships in the artifacts, not just in the debug title bar.
    manifest.app.version = match package.get("version") {
        Some(v) if v.as_str().is_some() => v.as_str().unwrap_or_default().to_string(),
        Some(v) if inherits_from_workspace(v) => workspace_version
            .ok_or(
                "Cargo.toml: version.workspace = true, but no [workspace.package] version \
                    was found in an ancestor Cargo.toml",
            )?
            .to_string(),
        _ => "0.1.0".to_string(),
    };
    Ok(manifest)
}

/// Is this a `<field>.workspace = true` inheritance marker?
fn inherits_from_workspace(value: &toml::Value) -> bool {
    value
        .get("workspace")
        .and_then(|w| w.as_bool())
        .unwrap_or(false)
}

/// The `[workspace.package] version` of the nearest ancestor that declares a `[workspace]` — the
/// same search cargo itself does when resolving inheritance.
pub fn workspace_package_version(start: &Path) -> Option<String> {
    for dir in start.ancestors() {
        let candidate = dir.join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&candidate) else {
            continue;
        };
        // `toml::from_str`, not `str::parse` — the latter parses a bare VALUE in this toml
        // version and fails on the first table header, which is every Cargo.toml.
        let Ok(value) = toml::from_str::<toml::Value>(&text) else {
            continue;
        };
        let Some(workspace) = value.get("workspace") else {
            continue;
        };
        return workspace
            .get("package")
            .and_then(|p| p.get("version"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
    }
    None
}

/// Find the nearest ancestor directory containing Day.toml (from `start` or cwd).
pub fn find_project(start: Option<&Path>) -> Result<Project, String> {
    let mut dir = match start {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().map_err(|e| e.to_string())?,
    };
    loop {
        let candidate = dir.join("Day.toml");
        if candidate.exists() {
            let day_toml = std::fs::read_to_string(&candidate).map_err(|e| e.to_string())?;
            let cargo_path = dir.join("Cargo.toml");
            let cargo_toml = std::fs::read_to_string(&cargo_path).map_err(|e| {
                format!(
                    "{}: {e} (Day.toml marks a Day project, which is also a cargo package)",
                    cargo_path.display()
                )
            })?;
            let inherited = workspace_package_version(&dir);
            let manifest = parse_manifest(&day_toml, &cargo_toml, inherited.as_deref())?;
            // Always hand back an ABSOLUTE root. A relative `--project` (e.g. `apps/example`) would
            // otherwise flow into build-tool arguments like xcodebuild's `SYMROOT` as a relative path;
            // xcodebuild resolves relative build paths against each target's own working directory, so
            // the app target and a SwiftPM package dependency scatter their products into different
            // trees (a missing `*_*.bundle` copy failure). Absolute paths resolve identically everywhere.
            let root = std::fs::canonicalize(&dir).unwrap_or_else(|_| {
                std::env::current_dir()
                    .map(|cwd| cwd.join(&dir))
                    .unwrap_or_else(|_| dir.clone())
            });
            return Ok(Project {
                root: strip_verbatim(root),
                manifest,
            });
        }
        if !dir.pop() {
            return Err("no Day.toml found in this directory or any ancestor".into());
        }
    }
}

/// The app id a `Day.toml` declares, or `None` if it is unreadable or declares none.
pub fn day_toml_app_id(day_toml: &Path) -> Option<String> {
    let text = std::fs::read_to_string(day_toml).ok()?;
    let doc: toml::Value = toml::from_str(&text).ok()?;
    doc.get("app")?.get("id")?.as_str().map(str::to_string)
}

/// Every Day project in a checkout, deepest-last and sorted, so the answer never depends on
/// filesystem order. A directory qualifies only with BOTH manifests: `crates/day-cli/templates/app`
/// has a `Day.toml` and no `Cargo.toml`, and packing it fails with a missing-manifest error that
/// says nothing about the real problem.
///
/// Used by `day rebuild` to locate the project inside a cloned commit, and by `day launch --git`
/// to locate it inside a cloned repository.
pub fn day_projects(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if dir.join("Day.toml").is_file() && dir.join("Cargo.toml").is_file() {
            found.push(dir.clone());
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut kids: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && !p.ends_with(".git") && !p.ends_with("target"))
            .collect();
        kids.sort();
        stack.extend(kids);
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARGO: &str = "[package]\nname = \"demo-app\"\nversion = \"1.2.3\"\n";

    #[test]
    fn identity_derives_from_cargo_toml() {
        let m = parse_manifest(
            "schema = 1\n[app]\nid = \"dev.example.demo\"\n",
            CARGO,
            None,
        )
        .unwrap();
        assert_eq!(m.app.name, "demo-app");
        assert_eq!(m.app.version, "1.2.3");
        let r = m.resolve("macos-appkit");
        assert_eq!(r.title, "demo-app"); // no title ⇒ crate name
        assert_eq!(r.build, 1);
    }

    #[test]
    fn overrides_resolve_most_specific_wins() {
        let m = parse_manifest(
            r#"
schema = 1

[app]
id = "dev.example.demo"
title = "Demo"
targets = ["ios-uikit", "macos-appkit", "macos-qt"]

# toolkit-wide override
[app.qt]
title = "Demo (Qt)"

# platform override beats toolkit
[app.macos]
id = "dev.example.demo.mac"

# exact target beats both
[app.macos-qt]
title = "Demo for macOS Qt"
build = 7
"#,
            CARGO,
            None,
        )
        .unwrap();
        assert_eq!(m.resolve("ios-uikit").title, "Demo");
        assert_eq!(m.resolve("macos-appkit").id, "dev.example.demo.mac");
        assert_eq!(m.resolve("macos-appkit").title, "Demo");
        let mq = m.resolve("macos-qt");
        assert_eq!(mq.id, "dev.example.demo.mac"); // platform
        assert_eq!(mq.title, "Demo for macOS Qt"); // exact target beats [app.qt]
        assert_eq!(mq.build, 7);
        assert_eq!(m.resolve("linux-qt").title, "Demo (Qt)"); // toolkit layer
    }

    #[test]
    fn schema_and_shape_are_validated() {
        assert!(parse_manifest("schema = 2\n[app]\nid = \"x\"\n", CARGO, None).is_err());
        assert!(parse_manifest("schema = 1\n", CARGO, None).is_err()); // no [app]
        // A typo'd scalar under [app] can't parse as an override table.
        assert!(
            parse_manifest(
                "schema = 1\n[app]\nid = \"x\"\ntitel = \"y\"\n",
                CARGO,
                None
            )
            .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn strip_verbatim_deverbatims_windows_paths() {
        // Drive + UNC verbatim prefixes are removed so the MinGW linker can read the paths.
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\D:\a\day\day\apps\showcase")),
            PathBuf::from(r"D:\a\day\day\apps\showcase")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\UNC\server\share\proj")),
            PathBuf::from(r"\\server\share\proj")
        );
        // A plain absolute path is already fine — leave it untouched.
        assert_eq!(
            strip_verbatim(PathBuf::from(r"D:\a\proj")),
            PathBuf::from(r"D:\a\proj")
        );
        // canonicalize() really does hand back a verbatim path here; the result must not.
        let canon = std::fs::canonicalize(".").unwrap();
        assert!(!strip_verbatim(canon).to_string_lossy().starts_with(r"\\?\"));
    }

    /// Adding `permissions` to a `deny_unknown_fields` struct must not break the manifests already
    /// checked into every app in the tree.
    #[test]
    fn manifest_without_permissions_still_parses() {
        let m =
            parse_manifest("schema = 1\n[app]\nid = \"dev.x.demo\"\n", CARGO, None).expect("parse");
        assert!(m.permissions.declared.is_empty());
        assert!(m.permissions.raw.android.is_empty());
    }

    #[test]
    fn permission_declaration_forms() {
        let toml = r#"
schema = 1
[app]
id = "dev.x.demo"

[permissions]
camera = "Scan a document."
notifications = true

[permissions.photos]
reason = "Attach a picture."
ios-reason = "Day attaches pictures from your library."
platforms = ["ios", "android"]

[permissions.raw]
android = ["android.permission.READ_CONTACTS"]
ios = { NSContactsUsageDescription = "Find friends." }
ohos = [{ name = "ohos.permission.READ_CONTACTS", reason = "Find friends.", when = "inuse" }]
"#;
        let m = parse_manifest(toml, CARGO, None).expect("parse");
        assert_eq!(m.permissions.declared.len(), 3);

        let camera = &m.permissions.declared["camera"];
        assert_eq!(camera.reason_for("ios"), Some("Scan a document."));
        assert!(camera.covers("ohos"));

        // `true` declares the permission without a reason — the notifications shape.
        assert_eq!(
            m.permissions.declared["notifications"].reason_for("ios"),
            None
        );

        // The per-platform override wins over the shared reason; other platforms fall back to it.
        let photos = &m.permissions.declared["photos"];
        assert_eq!(
            photos.reason_for("ios"),
            Some("Day attaches pictures from your library.")
        );
        assert_eq!(photos.reason_for("ohos"), Some("Attach a picture."));
        assert!(photos.covers("android"));
        assert!(
            !photos.covers("ohos"),
            "platforms = [...] must exclude ohos"
        );

        assert_eq!(
            m.permissions.raw.android,
            ["android.permission.READ_CONTACTS"]
        );
        assert_eq!(
            m.permissions
                .raw
                .ios
                .get("NSContactsUsageDescription")
                .map(String::as_str),
            Some("Find friends.")
        );
        assert_eq!(
            m.permissions.raw.ohos[0].name,
            "ohos.permission.READ_CONTACTS"
        );
    }

    /// A misspelled permission must fail the parse with the valid names, not be ignored — an
    /// undeclared permission is a runtime crash on iOS.
    #[test]
    fn unknown_permission_name_is_rejected() {
        let err = parse_manifest(
            "schema = 1\n[app]\nid = \"dev.x.demo\"\n[permissions]\ncammera = \"typo\"\n",
            CARGO,
            None,
        )
        .expect_err("should reject");
        assert!(err.contains("cammera"), "{err}");
        assert!(
            err.contains("camera"),
            "the error must list the valid names: {err}"
        );
    }

    /// An unknown key inside the long form is a typo too, and `deny_unknown_fields` catches it.
    #[test]
    fn unknown_key_inside_a_declaration_is_rejected() {
        let err = parse_manifest(
            "schema = 1\n[app]\nid = \"dev.x.demo\"\n[permissions.camera]\nresaon = \"typo\"\n",
            CARGO,
            None,
        )
        .expect_err("should reject");
        assert!(
            err.contains("resaon") || err.contains("unknown field"),
            "{err}"
        );
    }
}

#[cfg(test)]
mod workspace_inheritance {
    use super::*;

    /// `version.workspace = true` resolves from the nearest ancestor `[workspace.package]` — the
    /// shape every app in this repo uses. Before this, the parse fell back to a literal "0.1.0",
    /// which then stamped every packaged artifact.
    #[test]
    fn an_inherited_version_resolves_from_the_workspace() {
        const INHERITED: &str = "[package]\nname = \"demo\"\nversion.workspace = true\n";
        let m = parse_manifest(
            "schema = 1\n[app]\nid = \"dev.example.demo\"\n",
            INHERITED,
            Some("9.9.9"),
        )
        .expect("parse");
        assert_eq!(m.app.version, "9.9.9");
    }

    /// An inherited version with no workspace to inherit from is an error, not a quiet default:
    /// the wrong version reaches shipped artifacts.
    #[test]
    fn an_inherited_version_without_a_workspace_is_an_error() {
        const INHERITED: &str = "[package]\nname = \"demo\"\nversion.workspace = true\n";
        let err = parse_manifest("schema = 1\n[app]\nid = \"dev.x.demo\"\n", INHERITED, None)
            .expect_err("must not silently default");
        assert!(err.contains("workspace"), "{err}");
    }

    /// The repo's own workspace answers for the showcase — the case in the bug report.
    #[test]
    fn this_repo_resolves_its_own_workspace_version() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR")); // crates/day-cli
        let found = workspace_package_version(here);
        assert_eq!(found.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    }
}

#[cfg(test)]
mod lib_name_tests {
    use super::*;

    fn project_with_manifest(day_toml: &str) -> Project {
        let dir = std::env::temp_dir().join(format!(
            "day-scheme-{}-{:p}",
            std::process::id(),
            day_toml.as_ptr()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        )
        .expect("Cargo.toml");
        std::fs::write(dir.join("Day.toml"), day_toml).expect("Day.toml");
        find_project(Some(&dir)).expect("project")
    }

    fn project_with(manifest: &str) -> Project {
        let dir = std::env::temp_dir().join(format!(
            "day-libname-{}-{:p}",
            std::process::id(),
            manifest.as_ptr()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("Cargo.toml"), manifest).expect("Cargo.toml");
        std::fs::write(
            dir.join("Day.toml"),
            "schema = 1\n[app]\nid = \"dev.example.app\"\n",
        )
        .expect("Day.toml");
        find_project(Some(&dir)).expect("project")
    }

    /// Every artifact is named after the LIB target, so a scaffolded app's pin is what the
    /// staticlib, the cdylib and the wasm are all called — whatever the package is named.
    #[test]
    fn the_pinned_lib_name_wins_over_the_package() {
        let p = project_with(
            "[package]\nname = \"day-showcase\"\nversion = \"0.1.0\"\n\
             [lib]\nname = \"dayapp\"\ncrate-type = [\"rlib\"]\n",
        );
        assert_eq!(p.lib_name(), "dayapp");
    }

    /// An app from before the pin declares no `[lib] name`; cargo falls back to the package's.
    #[test]
    fn without_a_pin_the_package_name_is_the_fallback() {
        let p = project_with(
            "[package]\nname = \"day-showcase\"\nversion = \"0.1.0\"\n\
             [lib]\ncrate-type = [\"rlib\"]\n",
        );
        assert_eq!(p.lib_name(), "day_showcase");
        let none = project_with("[package]\nname = \"day-showcase\"\nversion = \"0.1.0\"\n");
        assert_eq!(none.lib_name(), "day_showcase");
    }

    /// A declared scheme wins over the derived default, because a scheme already in the world is
    /// a contract: apps scaffolded before the default existed derived theirs from the CRATE name
    /// (`Day-Showcase` ⇒ `dayshowcase`), and silently re-deriving it from the id would have
    /// changed `dayshowcase://` links to `showcase://` on every platform at once.
    #[test]
    fn a_declared_scheme_beats_the_derived_default() {
        let derived = project_with_manifest("schema = 1\n[app]\nid = \"dev.daybrite.showcase\"\n");
        assert_eq!(
            derived.manifest.resolve("macos-appkit").scheme(),
            "showcase"
        );

        let declared = project_with_manifest(
            "schema = 1\n[app]\nid = \"dev.daybrite.showcase\"\nscheme = \"dayshowcase\"\n",
        );
        assert_eq!(
            declared.manifest.resolve("macos-appkit").scheme(),
            "dayshowcase"
        );

        // An id whose last segment starts with a digit cannot be a scheme (RFC 3986), so the
        // derivation falls back rather than emitting something a platform would reject.
        let numeric = project_with_manifest("schema = 1\n[app]\nid = \"dev.example.2048\"\n");
        assert_eq!(numeric.manifest.resolve("macos-appkit").scheme(), "dayapp");
    }

    /// A `name` in a LATER table is not the lib's — the scan stops at the next header.
    #[test]
    fn a_name_in_a_following_table_is_not_read() {
        let p = project_with(
            "[package]\nname = \"day-showcase\"\nversion = \"0.1.0\"\n\
             [lib]\ncrate-type = [\"rlib\"]\n\n[[bin]]\nname = \"other\"\n",
        );
        assert_eq!(p.lib_name(), "day_showcase");
    }
}
