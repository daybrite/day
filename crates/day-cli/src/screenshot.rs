// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Screenshot gallery indexing (`day screenshot index`, DESIGN.md §16.5; dayscript metadata,
//! §14.7).
//!
//! Two layers produce the published index:
//!
//! - The dayscript runner (script.rs) records every capture it saves into
//!   `build/day/screenshots/<target>/gallery.json` (one file per target, upserted across
//!   runs and variants), carrying the `screenshot:` step's localized `title:` / `caption:`
//!   metadata plus the file facts (dimensions, byte size, sha-256). The metadata lives on the
//!   step because that is where the capture is declared; the runner strips it before the step
//!   reaches the engine, so apps need nothing new.
//!
//! - `day screenshot index` merges the per-target files (backfilling bare entries for files no
//!   index describes), resolves the published host from `website/site.toml`, and writes the
//!   unified `gallery.json` that site builds parse and app sites publish at
//!   `<host>/gallery/gallery.json`.
//!
//! A shot with a `title:` is gallery-curated; untitled captures stay in the index for
//! machines but a gallery page shows the curated set when one exists. `day lint`
//! cross-references the metadata's locale keys against the app's translation locales.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::Digest as _;

use crate::meta::Project;
use crate::term::BOLD;

/// Capture directories that are development stand-ins (macos-gtk exercises linux-gtk's
/// toolkit) or tool droppings, never publication targets.
const SKIP_DIRS: &[&str] = &[
    "_drive",
    "macos-gtk",
    "macos-qt",
    "windows-gtk",
    "windows-qt",
    "android-widget",
];

// ---------------------------------------------------------------------------
// Localized text
// ---------------------------------------------------------------------------

/// A localized text from dayscript metadata: a plain string, or a locale-keyed map
/// (`title: { en: "Home", fr: "Accueil" }`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Text {
    Plain(String),
    ByLocale(BTreeMap<String, String>),
}

impl Text {
    /// Resolve for `locale`: the exact tag, then any tag with the same primary language
    /// (`fr` ↔ `fr-FR`), then English (`en` or any `en-*`), then any value at all.
    pub fn resolve(&self, locale: &str) -> Option<&str> {
        match self {
            Text::Plain(s) => Some(s),
            Text::ByLocale(map) => {
                if let Some(s) = map.get(locale) {
                    return Some(s);
                }
                let lang = primary_language(locale);
                if let Some((_, s)) = map.iter().find(|(k, _)| primary_language(k) == lang) {
                    return Some(s);
                }
                if let Some((_, s)) = map.iter().find(|(k, _)| primary_language(k) == "en") {
                    return Some(s);
                }
                map.values().next().map(String::as_str)
            }
        }
    }

    /// The locale tags this text is authored in (empty for a plain string).
    pub fn locales(&self) -> Vec<&str> {
        match self {
            Text::Plain(_) => Vec::new(),
            Text::ByLocale(map) => map.keys().map(String::as_str).collect(),
        }
    }

    /// Normalize to a locale-keyed map: a plain string becomes `{ default_locale: s }`.
    fn to_map(&self, default_locale: &str) -> BTreeMap<String, String> {
        match self {
            Text::Plain(s) => BTreeMap::from([(default_locale.to_string(), s.clone())]),
            Text::ByLocale(map) => map.clone(),
        }
    }
}

/// The primary language subtag: `zh-CN` → `zh`.
pub(crate) fn primary_language(tag: &str) -> &str {
    tag.split(['-', '_']).next().unwrap_or(tag)
}

/// True when a variant segment reads as a language tag (`fr`, `zh-CN`). Variant names are
/// data and anything may appear (a local run leaves `ipad` or `uicheck` behind); the index
/// only claims a locale for one shaped like a locale.
fn is_locale_like(s: &str) -> bool {
    let mut parts = s.split('-');
    let Some(lang) = parts.next() else {
        return false;
    };
    ((2..=3).contains(&lang.len()) && lang.chars().all(|c| c.is_ascii_lowercase()))
        && parts.all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// The locale an index entry is filed under: the one recorded at capture, else the one its
/// variant names, else the project's default when the variant names a theme (a themed capture
/// is a real run in the default locale); a bare capture has none.
fn entry_locale(e: &TargetEntry, fallback_locale: &str) -> Option<String> {
    let (theme, vlocale) = parse_variant(&e.variant);
    e.locale
        .clone()
        .or_else(|| vlocale.map(str::to_string))
        .or_else(|| theme.is_some().then(|| fallback_locale.to_string()))
}

/// `light-fr` → (Some("light"), Some("fr")); `fr` → (None, Some("fr")); `default` → (None,
/// None). Mirrors the capture matrix's `--variant` naming (script.rs).
fn parse_variant(name: &str) -> (Option<&str>, Option<&str>) {
    if name == "default" {
        return (None, None);
    }
    for theme in ["light", "dark"] {
        if name == theme {
            return (Some(theme), None);
        }
        if let Some(rest) = name.strip_prefix(theme).and_then(|r| r.strip_prefix('-')) {
            return (Some(theme), is_locale_like(rest).then_some(rest));
        }
    }
    (None, is_locale_like(name).then_some(name))
}

// ---------------------------------------------------------------------------
// File facts
// ---------------------------------------------------------------------------

/// Width/height straight out of the PNG IHDR; no image library for 8 fixed bytes.
pub(crate) fn png_dims(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let be = |o: usize| u32::from_be_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    Some((be(16), be(20)))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = sha2::Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// `home` → `Home`, `list-item-100` → `List Item 100`: the label for a shot no metadata
/// titles (mirrors daysite's `shotLabel`).
fn derived_label(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    let mut start = true;
    for c in id.chars() {
        if c == '-' || c == '_' {
            out.push(' ');
            start = true;
        } else if start {
            out.extend(c.to_uppercase());
            start = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Now as `2026-08-13T21:04:05Z`. Hand-rolled (day-cli carries no time-format dependency);
/// the civil-from-days algorithm is Howard Hinnant's.
fn iso_utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days as i64 + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

// ---------------------------------------------------------------------------
// The per-target index (written by the dayscript runner)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Capture size (website docs "dayscript", "Capture size")
// ---------------------------------------------------------------------------

/// What a scripted run captures a desktop-class target at: a pixel size and the scale it is
/// rendered at. The window that produces it is `width / scale` by `height / scale` points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptureSize {
    /// Capture width in pixels.
    pub width: u32,
    /// Capture height in pixels.
    pub height: u32,
    /// Pixels per point.
    pub scale: f64,
}

impl CaptureSize {
    /// The window (capture region) size in points.
    pub fn points(&self) -> (u32, u32) {
        (
            (f64::from(self.width) / self.scale).round() as u32,
            (f64::from(self.height) / self.scale).round() as u32,
        )
    }
}

/// Parse `"<width>x<height>"` or `"<width>x<height>@<scale>"` (pixels). `"window"` is `None`:
/// capture at the app's own `[window]` size and the host display's scale.
pub fn parse_capture_size(text: &str, default_scale: f64) -> Result<Option<CaptureSize>, String> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("window") {
        return Ok(None);
    }
    let bad = || {
        format!(
            "capture size {text:?}: expected <width>x<height> in pixels, optionally @<scale> \
             (2560x1600, 2880x1800@2), or \"window\""
        )
    };
    let (dims, scale) = match text.split_once('@') {
        Some((d, s)) => (d, s.trim().parse::<f64>().map_err(|_| bad())?),
        None => (text, default_scale),
    };
    let (w, h) = dims.split_once(['x', 'X', '×']).ok_or_else(bad)?;
    let width = w.trim().parse::<u32>().map_err(|_| bad())?;
    let height = h.trim().parse::<u32>().map_err(|_| bad())?;
    if width == 0 || height == 0 || !(scale.is_finite() && (0.5..=4.0).contains(&scale)) {
        return Err(bad());
    }
    // A window is a whole number of points; a size the scale does not divide would come back a
    // pixel off, which is exactly what a store's size check refuses.
    let whole = |px: u32| (f64::from(px) / scale).fract().abs() < 1e-9;
    if !whole(width) || !whole(height) {
        return Err(format!(
            "capture size {text:?}: {width}x{height} pixels is not a whole number of points at \
             scale {scale}"
        ));
    }
    Ok(Some(CaptureSize {
        width,
        height,
        scale,
    }))
}

/// The capture size a scripted run uses: `--capture-size`, else the `DAY_CAPTURE_SIZE`
/// environment variable (what a CI workflow sets without touching the command line), else
/// Day.toml `[screenshots]`, whose default is 2560x1600 at 2x.
pub fn capture_size(project: &Project, flag: Option<&str>) -> Result<Option<CaptureSize>, String> {
    let shots = &project.manifest.screenshots;
    let from_env = std::env::var("DAY_CAPTURE_SIZE")
        .ok()
        .filter(|v| !v.trim().is_empty());
    match flag.map(str::to_string).or(from_env) {
        Some(text) => parse_capture_size(&text, shots.desktop_scale),
        None => parse_capture_size(&shots.desktop_size, shots.desktop_scale)
            .map_err(|e| format!("Day.toml [screenshots]: {e}")),
    }
}

/// Whether a target's captures follow the capture size: the desktop toolkits and the web build.
/// A phone or tablet captures its device's own panel.
pub fn is_desktop_class(target: &crate::targets::Target) -> bool {
    matches!(
        target.kind,
        crate::targets::TargetKind::Desktop | crate::targets::TargetKind::Web
    )
}

/// The variables that carry a capture size to the app: `DAY_WINDOW` (the window, in points) and
/// `DAY_CAPTURE_SCALE` (what a backend that renders its own snapshot renders it at). A variable
/// the caller already set, through `--env` or the environment `day` runs in, is left alone, so
/// a responsive-layout run with `DAY_WINDOW=500x640` keeps its narrow window.
pub fn capture_envs(size: CaptureSize, given: &[(String, String)]) -> Vec<(String, String)> {
    let (w, h) = size.points();
    let mut out = Vec::new();
    let mut put = |key: &str, value: String| {
        let taken = given.iter().any(|(k, _)| k == key) || std::env::var_os(key).is_some();
        if !taken {
            out.push((key.to_string(), value));
        }
    };
    put("DAY_WINDOW", format!("{w}x{h}"));
    put("DAY_CAPTURE_SCALE", format!("{}", size.scale));
    out
}

/// The capture-display helper (resources/macos/capture-display.m, which says why it exists).
#[cfg(target_os = "macos")]
const CAPTURE_DISPLAY_M: &str = include_str!("../resources/macos/capture-display.m");

/// The running helper and the display id it answered with. Held for the life of this process:
/// the helper exits, and its display with it, when the pipe in here closes.
#[cfg(target_os = "macos")]
static CAPTURE_DISPLAY: std::sync::Mutex<Option<(std::process::Child, Option<String>)>> =
    std::sync::Mutex::new(None);

/// A display that composites at the capture's scale, for a macos-appkit scripted run: the
/// `CGDirectDisplayID` to hand the app as `DAY_WINDOW_SCREEN`, or `None` when a display the
/// host already has will do (or no virtual one could be made, which is reported and leaves the
/// run capturing at the display's own scale). Started once per `day` process and shared by
/// every run of a capture matrix.
///
/// `DAY_CAPTURE_DISPLAY=native` turns this off; `=virtual` makes the display even on a host
/// that does not need one, which is how the path is exercised on a HiDPI Mac.
#[cfg(target_os = "macos")]
pub fn capture_display(size: CaptureSize) -> Option<String> {
    use std::io::BufRead as _;
    let mode = std::env::var("DAY_CAPTURE_DISPLAY").unwrap_or_default();
    if mode == "native" {
        return None;
    }
    let mut slot = CAPTURE_DISPLAY.lock().ok()?;
    if let Some((_, id)) = slot.as_ref() {
        return id.clone();
    }
    let note = |why: &str| {
        let (w, h) = size.points();
        crate::ops::status(
            "Warning",
            &format!(
                "no {}x capture display ({why}); macos-appkit captures at this display's own \
                 scale, {w}x{h} pixels on a 1x display",
                size.scale
            ),
        );
    };
    let helper = match materialize_capture_display() {
        Ok(path) => path,
        Err(e) => {
            note(&e);
            return None;
        }
    };
    let (w, h) = size.points();
    let mut command = std::process::Command::new(&helper);
    command
        .args([w.to_string(), h.to_string(), size.scale.to_string()])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    if mode == "virtual" {
        command.arg("force");
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            note(&format!("{}: {e}", helper.display()));
            return None;
        }
    };
    let mut line = String::new();
    if let Some(out) = child.stdout.take() {
        let _ = std::io::BufReader::new(out).read_line(&mut line);
    }
    let id = match line.trim().split_once(' ') {
        Some(("display", id)) => {
            crate::ops::status(
                "Capture",
                &format!(
                    "virtual {}x display {id} ({})",
                    size.scale,
                    if mode == "virtual" {
                        "DAY_CAPTURE_DISPLAY=virtual"
                    } else {
                        "no attached display has the scale"
                    }
                ),
            );
            crate::signals::register_child(child.id());
            Some(id.to_string())
        }
        Some(("unavailable", why)) => {
            note(why);
            None
        }
        _ => None, // "native": an attached display already has the scale
    };
    *slot = Some((child, id.clone()));
    id
}

/// Compile the helper with the host's clang into a version-stamped temp location, once. Xcode's
/// command-line tools are already a macos-appkit prerequisite, so this adds none.
#[cfg(target_os = "macos")]
fn materialize_capture_display() -> Result<PathBuf, String> {
    let digest = &sha256_hex(CAPTURE_DISPLAY_M.as_bytes())[..12];
    let dir = std::env::temp_dir().join(format!(
        "day-capture-display-{}-{digest}",
        env!("CARGO_PKG_VERSION")
    ));
    let bin = dir.join("day-capture-display");
    if bin.exists() {
        return Ok(bin);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let src = dir.join("capture-display.m");
    std::fs::write(&src, CAPTURE_DISPLAY_M).map_err(|e| format!("{}: {e}", src.display()))?;
    // Built beside its final name and renamed, so a concurrent `day` never runs half a binary.
    let staged = dir.join(format!("day-capture-display.{}", std::process::id()));
    let out = std::process::Command::new("xcrun")
        .args(["clang", "-fobjc-arc", "-O1", "-framework", "Cocoa"])
        .arg(&src)
        .arg("-o")
        .arg(&staged)
        .output()
        .map_err(|e| format!("xcrun clang: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "compiling the capture-display helper failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    std::fs::rename(&staged, &bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    Ok(bin)
}

/// No capture display off macOS: the other desktop backends render their own snapshot at the
/// stated scale, or (XAML) read back a display this tool cannot stand in for.
#[cfg(not(target_os = "macos"))]
pub fn capture_display(_size: CaptureSize) -> Option<String> {
    None
}

/// One capture in a target's `gallery.json` (`build/day/screenshots/<target>/gallery.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TargetEntry {
    pub file: String,
    pub variant: String,
    /// The `--device` slug this capture was taken on, when the run named one: the extra path
    /// level under the target (docs/screenshots.md). `None` for a single-device project, which
    /// keeps the index of every app that does not use device profiles byte-identical.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    pub shot: String,
    /// The locale the capture was taken in: the run's `--locale`, else the app's default
    /// locale, stamped by the runner so no reader has to decode the variant name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// The theme the capture was taken in (`light`, `dark`), when the run set one; stamped by
    /// the runner for the same reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<Text>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<Text>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Default, Serialize, Deserialize)]
struct TargetIndex {
    generator: String,
    target: String,
    screenshots: Vec<TargetEntry>,
}

/// Build a [`TargetEntry`] for a capture the runner just saved.
pub fn target_entry(
    path: &Path,
    variant: &str,
    device: Option<&str>,
    shot: &str,
    locale: Option<&str>,
    meta: Option<&ShotMeta>,
) -> Option<TargetEntry> {
    let bytes = std::fs::read(path).ok()?;
    let dims = png_dims(&bytes);
    Some(TargetEntry {
        file: path.file_name()?.to_string_lossy().into_owned(),
        variant: variant.to_string(),
        device: device.map(str::to_string),
        shot: shot.to_string(),
        locale: locale.map(str::to_string),
        theme: parse_variant(variant).0.map(str::to_string),
        title: meta.and_then(|m| m.title.clone()),
        caption: meta.and_then(|m| m.caption.clone()),
        source: meta.and_then(|m| m.source.clone()),
        width: dims.map(|d| d.0),
        height: dims.map(|d| d.1),
        bytes: bytes.len() as u64,
        sha256: sha256_hex(&bytes),
    })
}

/// Upsert `entries` into `<screenshots_root>/<target>/gallery.json`, keyed by
/// (device, variant, file). Entries whose files no longer exist are dropped, so a trimmed
/// walkthrough trims the index.
///
/// The index stays one file per target even when captures come from several devices: a device is
/// a dimension of a capture, like its theme and its locale, not a separate target.
pub fn record_target_entries(screenshots_root: &Path, target: &str, entries: Vec<TargetEntry>) {
    if entries.is_empty() {
        return;
    }
    let target_dir = screenshots_root.join(target);
    // A device profile's captures get their own index, `<target>/<device>/gallery.json`. Two
    // profiles of one target run on two CI runners and upload two artifacts, and when both
    // carried `<target>/gallery.json` the two files collided on merge: `download-artifact`
    // extracts artifacts concurrently, so the survivor was garbled, and every capture of the
    // target lost its metadata (Day-Showcase, 2026-09-24). `index` reads both layouts.
    let device = entries
        .iter()
        .map(|e| e.device.clone())
        .reduce(|a, b| if a == b { a } else { None })
        .flatten();
    let path = match &device {
        Some(d) => target_dir.join(d).join("gallery.json"),
        None => target_dir.join("gallery.json"),
    };
    let mut index: TargetIndex = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    index.generator = "day launch --script".into();
    index.target = target.into();
    // A target-level index from before the per-device layout may still describe this device;
    // its entries would shadow the fresh ones, so they leave that file now.
    if let Some(d) = &device {
        let shared = target_dir.join("gallery.json");
        if let Some(mut old) = std::fs::read_to_string(&shared)
            .ok()
            .and_then(|t| serde_json::from_str::<TargetIndex>(&t).ok())
            && old
                .screenshots
                .iter()
                .any(|e| e.device.as_deref() == Some(d))
        {
            old.screenshots.retain(|e| e.device.as_deref() != Some(d));
            if let Ok(json) = serde_json::to_string_pretty(&old) {
                let _ = std::fs::write(&shared, json + "\n");
            }
        }
    }
    for e in entries {
        if let Some(slot) = index
            .screenshots
            .iter_mut()
            .find(|s| s.device == e.device && s.variant == e.variant && s.file == e.file)
        {
            *slot = e;
        } else {
            index.screenshots.push(e);
        }
    }
    index.screenshots.retain(|e| {
        let mut p = target_dir.clone();
        if let Some(d) = &e.device {
            p = p.join(d);
        }
        p.join(&e.variant).join(&e.file).exists()
    });
    if let Ok(json) = serde_json::to_string_pretty(&index) {
        let _ = std::fs::write(&path, json + "\n");
    }
}

// ---------------------------------------------------------------------------
// dayscript metadata (shared with script.rs and lint.rs)
// ---------------------------------------------------------------------------

/// The gallery metadata a `screenshot:` step may carry (§14.7). Runner-side only: the runner
/// strips these keys before the step reaches the engine, so they are invisible to apps.
#[derive(Clone, Debug, Default)]
pub struct ShotMeta {
    pub title: Option<Text>,
    pub caption: Option<Text>,
    pub source: Option<String>,
    /// The step carried `store:`, the key that once marked a capture for the store listing. The
    /// listing is declared in `store/storefront.toml` `[screenshots]` now (§14.7); the key is stripped
    /// and the runner says so once.
    pub legacy_store: bool,
}

/// Take the metadata out of a runner step object (leaving the step engine-clean).
pub fn extract_meta(step: &mut serde_json::Map<String, serde_json::Value>) -> ShotMeta {
    let text = |v: serde_json::Value| serde_json::from_value::<Text>(v).ok();
    ShotMeta {
        title: step.remove("title").and_then(text),
        caption: step.remove("caption").and_then(text),
        source: step
            .remove("source")
            .and_then(|v| v.as_str().map(str::to_string)),
        legacy_store: step.remove("store").is_some(),
    }
}

/// Every `screenshot:` step's (name, metadata) in a dayscript file: `day lint`'s view.
pub fn script_screenshot_meta(path: &Path) -> Vec<(String, ShotMeta)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(doc) = serde_norway::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(flow) = doc.get("flow").and_then(|f| f.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in flow {
        let Some(obj) = entry.as_object() else {
            continue;
        };
        let Some(params) = obj.get("screenshot") else {
            continue;
        };
        let Some(params) = params.as_object() else {
            continue;
        };
        let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let mut params = params.clone();
        out.push((name.to_string(), extract_meta(&mut params)));
    }
    out
}

// ---------------------------------------------------------------------------
// `day screenshot index`: the merger
// ---------------------------------------------------------------------------

/// `website/site.toml`, for the published host and base path. Absent is fine; the index
/// carries paths only.
fn site_host(project_root: &Path) -> Option<(String, String)> {
    #[derive(Deserialize)]
    struct SiteToml {
        host: Option<String>,
    }
    let text = std::fs::read_to_string(project_root.join("website/site.toml")).ok()?;
    let site: SiteToml = toml::from_str(&text).ok()?;
    let host = site.host?;
    // `host` may carry a base path (a github.io project page); split origin from base so
    // published URLs come out right either way.
    let rest = host.split_once("://")?;
    let (origin, base) = match rest.1.split_once('/') {
        Some((h, path)) => (
            format!("{}://{h}", rest.0),
            format!("/{}", path.trim_end_matches('/')),
        ),
        None => (host.clone(), String::new()),
    };
    Some((origin, base))
}

/// The app's default locale for normalizing plain-string titles: `en` when the app has it,
/// else its first translation locale, else `en`.
fn default_locale(project_root: &Path) -> String {
    let fluent = crate::localize::survey(project_root).fluent;
    if fluent.iter().any(|l| l == "en") || fluent.is_empty() {
        "en".into()
    } else {
        fluent[0].clone()
    }
}

pub struct IndexOptions {
    /// Capture trees (`<target>/<variant>/<shot>.png`). Empty = `build/day/screenshots`.
    pub screenshot_paths: Vec<PathBuf>,
    /// Output file. Default: `gallery.json` in the first tree.
    pub out: Option<PathBuf>,
}

/// A capture's path under its target directory, with the device level when it has one.
fn capture_path(tdir: &Path, device: Option<&str>, variant: &str, file: &str) -> PathBuf {
    let mut p = tdir.to_path_buf();
    if let Some(d) = device {
        p = p.join(d);
    }
    p.join(variant).join(file)
}

/// Every `(device, variant, dir)` under a target directory, sorted.
///
/// Distinguishes a device level from a variant level by content: a directory whose children are
/// all directories is a device holding variants; one that holds files is a variant holding
/// captures. A device's own `gallery.json` (the index the runner writes beside its variants)
/// does not make it a variant. An empty directory counts as a variant, which contributes
/// nothing either way.
fn variant_dirs(tdir: &Path) -> Vec<(Option<String>, String, PathBuf)> {
    fn subdirs(p: &Path) -> Vec<(String, PathBuf)> {
        let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(p)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| e.file_name().to_str().map(|n| (n.to_string(), e.path())))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
    fn holds_only_dirs(p: &Path) -> bool {
        let mut any = false;
        let Ok(rd) = std::fs::read_dir(p) else {
            return false;
        };
        for e in rd.flatten() {
            if e.file_name() == "gallery.json" {
                continue;
            }
            any = true;
            if !e.path().is_dir() {
                return false;
            }
        }
        any
    }
    let mut out = Vec::new();
    for (name, dir) in subdirs(tdir) {
        if holds_only_dirs(&dir) {
            for (vname, vdir) in subdirs(&dir) {
                out.push((Some(name.clone()), vname, vdir));
            }
        } else {
            out.push((None, name, dir));
        }
    }
    out
}

/// Merge capture trees into the unified `gallery.json` (the file app sites publish at
/// `<host>/gallery/gallery.json` and site builds parse). Returns the path written.
pub fn index(project: &Project, opts: &IndexOptions) -> Result<PathBuf, String> {
    let roots = if opts.screenshot_paths.is_empty() {
        vec![crate::ops::staged_root(project).join("screenshots")]
    } else {
        opts.screenshot_paths.clone()
    };
    let out = opts
        .out
        .clone()
        .unwrap_or_else(|| roots[0].join("gallery.json"));
    let host = site_host(&project.root);
    let fallback_locale = default_locale(&project.root);

    // Collect per target, first tree wins on a (target, variant, file) collision.
    let mut by_target: BTreeMap<String, Vec<TargetEntry>> = BTreeMap::new();
    // Targets whose entries came from a per-target index, which preserves the dayscript's
    // declaration order; bare directory scans only offer alphabetical order.
    let mut script_ordered: Vec<String> = Vec::new();
    for root in &roots {
        let Ok(targets) = std::fs::read_dir(root) else {
            continue;
        };
        for t in targets.flatten() {
            let tdir = t.path();
            let Some(target) = t.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if !tdir.is_dir() || SKIP_DIRS.contains(&target.as_str()) {
                continue;
            }
            // The target's index, and each device profile's own (`<target>/<device>/
            // gallery.json`, which is how the runner writes a device's captures so two
            // profiles' artifacts never collide on one file).
            let mut known: TargetIndex = std::fs::read_to_string(tdir.join("gallery.json"))
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            for (device, _, _) in variant_dirs(&tdir) {
                let Some(device) = device else {
                    continue;
                };
                let per_device = tdir.join(&device).join("gallery.json");
                if known
                    .screenshots
                    .iter()
                    .any(|e| e.device.as_deref() == Some(device.as_str()))
                    && !per_device.exists()
                {
                    continue;
                }
                if let Some(more) = std::fs::read_to_string(&per_device)
                    .ok()
                    .and_then(|s| serde_json::from_str::<TargetIndex>(&s).ok())
                {
                    known.screenshots.extend(more.screenshots);
                }
            }
            if !known.screenshots.is_empty() && !script_ordered.contains(&target) {
                script_ordered.push(target.clone());
            }
            let list = by_target.entry(target).or_default();
            // The per-target index leads, in its order, which is the dayscript's declaration
            // order. Files stay the truth: an entry whose file is gone contributes nothing.
            for e in &known.screenshots {
                if capture_path(&tdir, e.device.as_deref(), &e.variant, &e.file).exists()
                    && !list
                        .iter()
                        .any(|x| x.device == e.device && x.variant == e.variant && x.file == e.file)
                {
                    list.push(e.clone());
                }
            }
            // Then the tree walk backfills captures no index describes (bare, derived facts).
            //
            // A target's children are either variant directories (`dark-fr/`) or device
            // directories that each hold variants (`ipad/dark-fr/`, docs/screenshots.md). The
            // two are told apart by what is inside: a directory holding only directories is a
            // device level. Guessing from the name would be worse: a device slug and a variant
            // name are both free-form, and `ipad` reads exactly like a variant.
            for (device, vname, vdir) in variant_dirs(&tdir) {
                let Ok(files) = std::fs::read_dir(&vdir) else {
                    continue;
                };
                let mut fnames: Vec<String> = files
                    .flatten()
                    .filter_map(|f| f.file_name().to_str().map(str::to_string))
                    .filter(|f| f.to_lowercase().ends_with(".png"))
                    .collect();
                fnames.sort();
                for fname in fnames {
                    if list.iter().any(|e| {
                        e.device.as_deref() == device.as_deref()
                            && e.variant == vname
                            && e.file == fname
                    }) {
                        continue; // an earlier tree already provided it
                    }
                    let shot = fname.trim_end_matches(".png").to_string();
                    if let Some(e) = target_entry(
                        &vdir.join(&fname),
                        &vname,
                        device.as_deref(),
                        &shot,
                        None,
                        None,
                    ) {
                        list.push(e);
                    }
                }
            }
        }
    }

    // Platform order: the target vocabulary's presentation order, unknowns appended.
    let mut platforms: Vec<String> = crate::targets::TARGETS
        .iter()
        .map(|t| t.name.to_string())
        .filter(|n| by_target.contains_key(n))
        .collect();
    for t in by_target.keys() {
        if !platforms.contains(t) {
            platforms.push(t.clone());
        }
    }

    // Shot order: first appearance, from the script-ordered targets first (their per-target
    // index preserves the dayscript's declaration order), then any bare-scanned stragglers
    // (alphabetical is all a directory walk can offer).
    let mut order_walk: Vec<&String> = platforms
        .iter()
        .filter(|p| script_ordered.contains(p))
        .collect();
    order_walk.extend(platforms.iter().filter(|p| !script_ordered.contains(p)));
    let mut shot_order: Vec<String> = Vec::new();
    let mut shot_meta: BTreeMap<String, ShotMeta> = BTreeMap::new();
    for platform in order_walk {
        for e in &by_target[platform] {
            if !shot_order.contains(&e.shot) {
                shot_order.push(e.shot.clone());
            }
            let m = shot_meta.entry(e.shot.clone()).or_default();
            if m.title.is_none() {
                m.title = e.title.clone();
            }
            if m.caption.is_none() {
                m.caption = e.caption.clone();
            }
            if m.source.is_none() {
                m.source = e.source.clone();
            }
        }
    }

    let mut themes: Vec<String> = Vec::new();
    let mut locales: Vec<String> = Vec::new();
    let mut screenshots = Vec::new();
    for platform in &platforms {
        let (os, toolkit) = crate::targets::TARGETS
            .iter()
            .find(|t| t.name == *platform)
            .map(|t| (t.os.to_string(), t.toolkit.to_string()))
            .unwrap_or_else(|| {
                let (os, tk) = platform.split_once('-').unwrap_or((platform, ""));
                (os.to_string(), tk.to_string())
            });
        let mut entries: Vec<&TargetEntry> = by_target[platform].iter().collect();
        entries.sort_by(|a, b| {
            let ra = shot_order.iter().position(|s| *s == a.shot);
            let rb = shot_order.iter().position(|s| *s == b.shot);
            ra.cmp(&rb).then_with(|| a.variant.cmp(&b.variant))
        });
        for e in entries {
            let theme = e.theme.as_deref().or_else(|| parse_variant(&e.variant).0);
            let locale = entry_locale(e, &fallback_locale);
            if let Some(t) = theme
                && !themes.iter().any(|x| x == t)
            {
                themes.push(t.to_string());
            }
            if let Some(l) = &locale
                && !locales.contains(l)
            {
                locales.push(l.clone());
            }
            let meta = shot_meta.get(&e.shot);
            let for_locale = locale.as_deref().unwrap_or(&fallback_locale);
            let title = meta
                .and_then(|m| m.title.as_ref())
                .and_then(|t| t.resolve(for_locale))
                .map(str::to_string)
                .unwrap_or_else(|| derived_label(&e.shot));
            let caption = meta
                .and_then(|m| m.caption.as_ref())
                .and_then(|c| c.resolve(for_locale))
                .map(str::to_string);
            // The device level, where the capture has one, is part of the published path as well
            // as its own field: a consumer that only reads `path` still resolves the right image,
            // and one that groups by device does not have to parse it back out
            // (docs/screenshots.md).
            let path = match &e.device {
                Some(d) => format!("gallery/{platform}/{d}/{}/{}", e.variant, e.file),
                None => format!("gallery/{platform}/{}/{}", e.variant, e.file),
            };
            screenshots.push(serde_json::json!({
                "file": e.file,
                "path": path,
                "url": host.as_ref().map(|(o, b)| format!("{o}{b}/{path}")),
                "shot": e.shot,
                "title": title,
                "caption": caption,
                "platform": platform,
                "device": e.device,
                "os": os,
                "toolkit": toolkit,
                "variant": e.variant,
                "theme": theme,
                "locale": locale,
                "width": e.width,
                "height": e.height,
                "bytes": e.bytes,
                "sha256": e.sha256,
            }));
        }
    }

    let shots: Vec<serde_json::Value> = shot_order
        .iter()
        .map(|id| {
            let m = shot_meta.get(id).cloned().unwrap_or_default();
            serde_json::json!({
                "id": id,
                "title": m.title.map(|t| t.to_map(&fallback_locale)),
                "caption": m.caption.map(|c| c.to_map(&fallback_locale)),
                "source": m.source,
            })
        })
        .collect();

    // The listings (§14.7): `store/storefront.toml` `[storefront.<target>…screenshots]` resolved for
    // every captured target, so a consumer selects on the index alone. Per target: `website`,
    // one list per device kind and locale the target captured, for its page's rows, and
    // `stores`, the same per store it declares. Each item is a shot id and the theme to take.
    let declared = crate::store::read(project)
        .map(|l| l.app)
        .unwrap_or_default();
    // The stores' rules, for the store each target publishes through: its list is written
    // whether or not the declaration names the store.
    let loaded_rules = crate::store::StoreRules::load(project, None).ok();
    let store_rules: &crate::store::StoreRules = match &loaded_rules {
        Some(r) => r,
        None => crate::store::StoreRules::builtin(),
    };
    let mut listings = serde_json::Map::new();
    for platform in &platforms {
        // Only a target with screenshot lists gets a listing: a target table holding submission
        // info alone would resolve to empty lists, and a consumer reads an empty list as "show
        // nothing" (Day-Rise's iOS page showed no captures while its gallery had 16, 2026-09-24).
        if !declared.declares_screenshots(platform) {
            continue;
        }
        let target = crate::targets::find(platform);
        let mut kinds: BTreeSet<String> = by_target[platform]
            .iter()
            .map(|e| match target {
                Some(t) => crate::store::capture_kind(t, e.device.as_deref()),
                None => e.device.clone().unwrap_or_else(|| "default".to_string()),
            })
            .collect();
        // Every list is resolved per locale the target captured, keyed as the entries above
        // are (a capture with no locale at all files under `default`), so a consumer matching
        // a capture's kind and locale finds its list without knowing the fallback rules.
        let mut per_locale: BTreeSet<String> = by_target[platform]
            .iter()
            .map(|e| entry_locale(e, &fallback_locale).unwrap_or_else(|| "default".to_string()))
            .collect();
        let mut stores = serde_json::Map::new();
        // Every store the declaration names, and the store the rules stage for the target
        // whether the declaration names it or not: a target-level list serves the store's
        // listing as well as the website's (docs/store.md), and the stager and the App Fair's
        // queue read a store's captures from this block alone. Games-Fair declared
        // `[storefront.android-mdc.screenshots]` and nothing per store, its release's index
        // carried an empty `stores`, and the queue refused the submission as "declares none"
        // (v2.1.9, 2026-09-25).
        let mut store_names: BTreeSet<String> = declared.stores(platform).into_iter().collect();
        if let Some((key, _)) = store_rules.store_for(platform) {
            store_names.insert(key.to_string());
        }
        for store in store_names {
            kinds.extend(declared.declared_kinds(platform, &store));
            per_locale.extend(declared.declared_locales(platform, &store));
            // A store's block carries the kinds its rules list (phone and tablet for Play,
            // iPhone and iPad for the App Store) and any the declaration names for it; a
            // capture on some other profile — the optional API-floor row a CI matrix adds,
            // say — belongs to the website's rows, not to a listing the store would refuse it
            // from. A store the rules know by name only keeps every captured kind.
            let rule_kinds: Vec<String> = store_rules
                .store_for(platform)
                .filter(|(key, _)| *key == store)
                .map(|(_, r)| r.kinds().into_iter().map(|(k, _)| k.to_string()).collect())
                .unwrap_or_default();
            let named: BTreeSet<String> = declared
                .declared_kinds(platform, &store)
                .into_iter()
                .collect();
            let store_kinds: Vec<&String> = kinds
                .iter()
                .filter(|k| rule_kinds.is_empty() || rule_kinds.contains(k) || named.contains(*k))
                .collect();
            let mut per_kind = serde_json::Map::new();
            for kind in store_kinds {
                let mut lists = serde_json::Map::new();
                for locale in &per_locale {
                    lists.insert(
                        locale.clone(),
                        serde_json::to_value(declared.store_kind(platform, &store, kind, locale))
                            .unwrap_or_default(),
                    );
                }
                per_kind.insert(kind.clone(), serde_json::Value::Object(lists));
            }
            stores.insert(store, serde_json::Value::Object(per_kind));
        }
        // The website's rows: one list per device kind the target captured, per locale.
        kinds.extend(declared.declared_kinds(platform, ""));
        per_locale.extend(declared.declared_locales(platform, ""));
        let mut website = serde_json::Map::new();
        for kind in &kinds {
            let mut lists = serde_json::Map::new();
            for locale in &per_locale {
                lists.insert(
                    locale.clone(),
                    serde_json::to_value(declared.website(platform, kind, locale))
                        .unwrap_or_default(),
                );
            }
            website.insert(kind.clone(), serde_json::Value::Object(lists));
        }
        listings.insert(
            platform.clone(),
            serde_json::json!({
                "website": website,
                "stores": stores,
            }),
        );
    }

    let doc = serde_json::json!({
        "generator": "day screenshot index",
        "generated": iso_utc_now(),
        "site": host.as_ref().map(|(o, b)| format!("{o}{b}")),
        "themes": themes,
        "locales": locales,
        "platforms": platforms,
        "shots": shots,
        "screenshots": screenshots,
        "listings": listings,
    });
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(
        &out,
        serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())? + "\n",
    )
    .map_err(|e| e.to_string())?;
    eprintln!(
        "{BOLD}      Index{BOLD:#} {} shot(s), {} capture(s) on {} target(s) → {}",
        shot_order.len(),
        screenshots.len(),
        platforms.len(),
        out.display()
    );
    Ok(out)
}

// ---------------------------------------------------------------------------
// The frame archive (`day screenshot pack` / `unpack`)
// ---------------------------------------------------------------------------
//
// A run's captures are near-copies of each other: one page in eight variants, forty pages
// sharing a sidebar. PNG compresses each file alone and sees none of that. The archive stores
// the decoded pixels of every capture, back to back, in a Zstandard stream whose window reaches
// across the variants of a shot, and the index says where each capture's pixels sit and what
// they hash to. Measured on Day-Showcase v0.4.25's iPad captures (392 files, 2752×2064): 191 MB
// of PNG, 57 MB as lossless H.264, 23.7 MB here.
//
// The layout, which docs/screenshot-archive.md states for other tools:
//
// - One file. Each (platform, device) group is its own Zstandard frame, and the frames are
//   concatenated, so `zstd -d --long=30` on the whole file yields every group's pixels in order
//   and a reader that wants one group reads `bytes` from `offset`.
// - Inside a group the captures follow the index's order (shot, then variant), which keeps a
//   shot's variants adjacent. Groups are independent because two targets share no pixels worth
//   a window: iPhone and iPad packed together came to the sum of the two apart.
// - A capture is its rows top to bottom with no padding, `rgb24` (R, G, B) when every pixel is
//   opaque and `rgba` (straight alpha) otherwise.

/// The `archive.format` this CLI writes and reads.
pub const ARCHIVE_FORMAT: &str = "day-frames-1";
/// The archive's default file name, beside the index.
pub const ARCHIVE_FILE: &str = "screenshots.frames.zst";
/// Level 19 took 37 s for the 392 iPad captures on four threads; level 15 took 15 s and wrote a
/// file 21% larger. A release packs once and is downloaded for years.
const ARCHIVE_LEVEL: i32 = 19;
/// The largest window a group asks for: 1 GiB, which is also what a reader must allocate.
/// 2 GiB wrote a file 2% smaller for twice the memory at both ends.
const ARCHIVE_MAX_WINDOW_LOG: u32 = 30;

/// Options for [`pack`].
pub struct PackOptions {
    /// The merged index (`day screenshot index` output) naming the captures.
    pub index: PathBuf,
    /// The capture tree the index's paths resolve in. Default: the index's directory.
    pub root: Option<PathBuf>,
    /// The archive to write. Default: [`ARCHIVE_FILE`] beside the index.
    pub out: Option<PathBuf>,
    /// Where the index, now describing the archive, is written. Default: over the input.
    pub index_out: Option<PathBuf>,
}

/// Options for [`unpack`].
pub struct UnpackOptions {
    /// An index whose `archive` block describes the file.
    pub index: PathBuf,
    /// The archive. Default: the index's `archive.file`, beside the index.
    pub archive: Option<PathBuf>,
    /// The capture tree to write. `None` checks the archive and writes nothing.
    pub out: Option<PathBuf>,
}

/// One capture's place in the archive: the index entry's `frame`.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct FrameRef {
    /// Index into `archive.groups`.
    group: usize,
    /// Byte offset of the pixels in the group's decompressed stream.
    offset: u64,
    /// Byte length of the pixels: width × height × 3 (`rgb24`) or × 4 (`rgba`).
    bytes: u64,
    /// `rgb24` or `rgba`.
    pixels: String,
    /// sha-256 of those bytes.
    sha256: String,
}

/// One (platform, device) run of captures: an independent Zstandard frame of the archive.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ArchiveGroup {
    platform: String,
    #[serde(default)]
    device: Option<String>,
    /// Byte offset of the group's Zstandard frame in the archive file.
    offset: u64,
    /// Compressed length of that frame.
    bytes: u64,
    /// Decompressed length: the sum of its captures' `frame.bytes`.
    raw_bytes: u64,
    /// The number of captures in the group.
    frames: u64,
}

/// The index's `archive` block.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ArchiveInfo {
    format: String,
    /// The archive's file name, relative to the index.
    file: String,
    bytes: u64,
    sha256: String,
    compression: String,
    /// The largest window any group uses, as a power of two: what `zstd --long=<n>` and a
    /// decoder's window limit must allow.
    window_log: u32,
    groups: Vec<ArchiveGroup>,
}

/// Counts and hashes everything written through it, so the archive's size and sha-256 come
/// from the bytes as they are written.
struct HashingWriter<W: std::io::Write> {
    inner: W,
    hash: sha2::Sha256,
    written: u64,
}

impl<W: std::io::Write> std::io::Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hash.update(&buf[..n]);
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn hex(digest: impl AsRef<[u8]>) -> String {
    digest.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// The file an index entry's `path` names under a capture tree. The published path leads with
/// `gallery/`; the tree does not. A path that would leave the tree is refused: an index can
/// come from a download.
fn tree_path(root: &Path, entry: &serde_json::Value) -> Result<PathBuf, String> {
    let path = entry["path"]
        .as_str()
        .ok_or("an index entry has no `path`")?;
    let rel = Path::new(path.strip_prefix("gallery/").unwrap_or(path));
    if rel.as_os_str().is_empty()
        || !rel
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
    {
        return Err(format!("the index path `{path}` leaves the capture tree"));
    }
    Ok(root.join(rel))
}

/// Decode a PNG to the archive's pixels: `(width, height, "rgb24" | "rgba", bytes)`.
fn decode_png(path: &Path) -> Result<(u32, u32, &'static str, Vec<u8>), String> {
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let file = std::fs::File::open(path).map_err(|e| at(&e))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    // Palettes, low bit depths and tRNS become plain 8-bit samples.
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| at(&e))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| at(&"the image is too large to decode"))?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(|e| at(&e))?;
    buf.truncate(info.buffer_size());
    if info.bit_depth != png::BitDepth::Eight {
        return Err(at(&"16-bit captures are not supported"));
    }
    let (pixels, bytes): (&'static str, Vec<u8>) = match info.color_type {
        png::ColorType::Rgb => ("rgb24", buf),
        png::ColorType::Grayscale => ("rgb24", buf.iter().flat_map(|&g| [g, g, g]).collect()),
        // An opaque capture drops its alpha: the pixels are the same and a quarter smaller.
        png::ColorType::Rgba => {
            let (px, _) = buf.as_chunks::<4>();
            if px.iter().all(|p| p[3] == 255) {
                (
                    "rgb24",
                    px.iter().flat_map(|p| [p[0], p[1], p[2]]).collect(),
                )
            } else {
                ("rgba", buf)
            }
        }
        png::ColorType::GrayscaleAlpha => {
            let (px, _) = buf.as_chunks::<2>();
            if px.iter().all(|p| p[1] == 255) {
                (
                    "rgb24",
                    px.iter().flat_map(|p| [p[0], p[0], p[0]]).collect(),
                )
            } else {
                (
                    "rgba",
                    px.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
                )
            }
        }
        png::ColorType::Indexed => return Err(at(&"the palette did not expand")),
    };
    Ok((info.width, info.height, pixels, bytes))
}

/// Rewrite a capture the runner just saved as a normalized PNG, so every target's files have
/// one shape whatever tool took them: 8-bit samples, RGB when every pixel is opaque and RGBA
/// otherwise, an `sRGB` chunk and no other ancillary chunk, one encoder setting.
///
/// The targets hand back different files for the same kind of image (Day-Showcase v0.4.25):
/// RGBA with an opaque alpha channel everywhere but web-dom, `eXIf` on Apple's, `sBIT` on
/// Android's, `gAMA` and `pHYs` on Windows', three zlib settings. The pixels are kept exactly.
///
/// A capture is left as saved, with `Ok(false)`, when rewriting would change what it shows: a
/// 16-bit image, or one embedding an ICC profile other than sRGB, whose samples are not sRGB.
pub fn normalize_capture(path: &Path) -> Result<bool, String> {
    let profile = std::fs::File::open(path)
        .ok()
        .and_then(|f| {
            png::Decoder::new(std::io::BufReader::new(f))
                .read_info()
                .ok()
        })
        .map(|r| {
            let info = r.info();
            (
                info.bit_depth == png::BitDepth::Sixteen,
                info.icc_profile.as_ref().map(|p| is_srgb_profile(p)),
            )
        });
    match profile {
        None => return Err(format!("{} is not a PNG", path.display())),
        Some((true, _)) | Some((_, Some(false))) => return Ok(false),
        Some(_) => {}
    }
    let (width, height, pixels, bytes) = decode_png(path)?;
    let png = encode_png(width, height, pixels, &bytes)?;
    std::fs::write(path, png).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

/// Whether an ICC profile is an sRGB one, by its description: `sRGB IEC61966-2.1` is what
/// AppKit and ImageIO embed for the sRGB color space.
fn is_srgb_profile(profile: &[u8]) -> bool {
    // The tag table: a count at byte 128, then (signature, offset, size) triples.
    let u32_at = |o: usize| {
        profile
            .get(o..o + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let Some(count) = u32_at(128) else {
        return false;
    };
    (0..count.min(256)).any(|i| {
        let entry = 132 + 12 * i;
        if profile.get(entry..entry + 4) != Some(b"desc".as_slice()) {
            return false;
        }
        let (Some(offset), Some(size)) = (u32_at(entry + 4), u32_at(entry + 8)) else {
            return false;
        };
        let Some(tag) = profile.get(offset..offset.saturating_add(size)) else {
            return false;
        };
        // `desc` holds ASCII and `mluc` UTF-16BE; dropping the zero bytes reads both.
        let text: Vec<u8> = tag.iter().copied().filter(|&b| b != 0).collect();
        text.windows(4).any(|w| w == b"sRGB")
    })
}

/// Encode archive pixels as a PNG file's bytes: the normalized form [`normalize_capture`]
/// describes.
fn encode_png(width: u32, height: u32, pixels: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    encoder.set_color(if pixels == "rgba" {
        png::ColorType::Rgba
    } else {
        png::ColorType::Rgb
    });
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(bytes).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

fn read_index(path: &Path) -> Result<serde_json::Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if !doc["screenshots"].is_array() {
        return Err(format!(
            "{} is not a gallery index: it has no `screenshots` list (`day screenshot index` writes one)",
            path.display()
        ));
    }
    Ok(doc)
}

fn write_index(path: &Path, doc: &serde_json::Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(doc).map_err(|e| e.to_string())?;
    std::fs::write(path, json + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

fn worker_count() -> usize {
    std::thread::available_parallelism().map_or(2, |n| n.get())
}

/// Pack the captures an index names into one frame archive, and write the index with an
/// `archive` block and a `frame` per capture. Returns the archive's path.
pub fn pack(opts: &PackOptions) -> Result<PathBuf, String> {
    use std::io::Write as _;

    let mut doc = read_index(&opts.index)?;
    let index_dir = opts.index.parent().unwrap_or(Path::new(".")).to_path_buf();
    let root = opts.root.clone().unwrap_or_else(|| index_dir.clone());
    let out = opts
        .out
        .clone()
        .unwrap_or_else(|| index_dir.join(ARCHIVE_FILE));
    let entries = doc["screenshots"].as_array().cloned().unwrap_or_default();
    if entries.is_empty() {
        return Err(format!(
            "{} lists no captures to pack",
            opts.index.display()
        ));
    }

    // Groups in first-appearance order, each keeping the index's order: shot, then variant.
    type GroupKey = (String, Option<String>);
    let mut groups: Vec<(GroupKey, Vec<usize>)> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let key = (
            e["platform"].as_str().unwrap_or_default().to_string(),
            e["device"].as_str().map(str::to_string),
        );
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(i),
            None => groups.push((key, vec![i])),
        }
    }

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let io = |e: std::io::Error| format!("{}: {e}", out.display());
    let mut file = HashingWriter {
        inner: std::io::BufWriter::new(std::fs::File::create(&out).map_err(io)?),
        hash: sha2::Sha256::new(),
        written: 0,
    };
    let threads = worker_count().min(4) as u32;
    let mut infos: Vec<ArchiveGroup> = Vec::new();
    let mut frames: Vec<Option<FrameRef>> = vec![None; entries.len()];
    let mut png_total = 0u64;
    let mut window_log = 0u32;
    // Captures per platform that embed an ICC profile, for the warning below.
    let mut profiled: BTreeMap<String, usize> = BTreeMap::new();
    for (g, ((platform, device), members)) in groups.iter().enumerate() {
        // The window covers the group when it can, so a shot's last variant still sees its
        // first. The estimate is the opaque size; a group that turns out to carry alpha gets a
        // window a little short of it.
        let estimate: u64 = members
            .iter()
            .map(|&i| {
                entries[i]["width"].as_u64().unwrap_or(0)
                    * entries[i]["height"].as_u64().unwrap_or(0)
                    * 3
            })
            .sum();
        let wlog = (u64::BITS - estimate.max(1).leading_zeros()).clamp(20, ARCHIVE_MAX_WINDOW_LOG);
        window_log = window_log.max(wlog);
        let start = file.written;
        let mut encoder =
            zstd::stream::write::Encoder::new(&mut file, ARCHIVE_LEVEL).map_err(io)?;
        encoder.long_distance_matching(true).map_err(io)?;
        encoder.window_log(wlog).map_err(io)?;
        encoder.include_checksum(true).map_err(io)?;
        encoder.multithread(threads).map_err(io)?;
        let mut raw = 0u64;
        // Decoding runs ahead of the compressor on other threads, in order.
        let decoded = decode_ahead(members.iter().map(|&i| tree_path(&root, &entries[i])));
        for (&i, item) in members.iter().zip(decoded) {
            let (width, height, pixels, bytes, png_bytes, icc) = item?;
            if icc {
                *profiled.entry(platform.clone()).or_default() += 1;
            }
            let e = &entries[i];
            if let (Some(w), Some(h)) = (e["width"].as_u64(), e["height"].as_u64())
                && (w, h) != (u64::from(width), u64::from(height))
            {
                return Err(format!(
                    "{} is {width}×{height}, and the index says {w}×{h}: rebuild the index with `day screenshot index`",
                    e["path"].as_str().unwrap_or_default()
                ));
            }
            encoder.write_all(&bytes).map_err(io)?;
            frames[i] = Some(FrameRef {
                group: g,
                offset: raw,
                bytes: bytes.len() as u64,
                pixels: pixels.to_string(),
                sha256: sha256_hex(&bytes),
            });
            raw += bytes.len() as u64;
            png_total += png_bytes;
        }
        encoder.finish().map_err(io)?;
        infos.push(ArchiveGroup {
            platform: platform.clone(),
            device: device.clone(),
            offset: start,
            bytes: file.written - start,
            raw_bytes: raw,
            frames: members.len() as u64,
        });
    }
    file.flush().map_err(io)?;
    let info = ArchiveInfo {
        format: ARCHIVE_FORMAT.into(),
        file: out
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| ARCHIVE_FILE.into()),
        bytes: file.written,
        sha256: hex(file.hash.finalize()),
        compression: "zstd".into(),
        window_log,
        groups: infos,
    };

    if let Some(list) = doc["screenshots"].as_array_mut() {
        for (e, frame) in list.iter_mut().zip(&frames) {
            e["frame"] = serde_json::to_value(frame).map_err(|e| e.to_string())?;
        }
    }
    doc["archive"] = serde_json::to_value(&info).map_err(|e| e.to_string())?;
    let index_out = opts.index_out.clone().unwrap_or_else(|| opts.index.clone());
    write_index(&index_out, &doc)?;
    for (platform, count) in &profiled {
        eprintln!(
            "warning: {count} {platform} capture(s) embed an ICC color profile other than sRGB, which the archive does not carry: they unpack with the same samples tagged sRGB, so their colors shift"
        );
    }
    eprintln!(
        "{BOLD}       Pack{BOLD:#} {} capture(s) in {} group(s), {} of PNG → {} ({})",
        entries.len(),
        info.groups.len(),
        human_bytes(png_total),
        out.display(),
        human_bytes(info.bytes)
    );
    Ok(out)
}

type Decoded = Result<(u32, u32, &'static str, Vec<u8>, u64, bool), String>;

/// Whether a PNG embeds an ICC profile (`iCCP`) other than sRGB: its samples are in that
/// profile's color space, which the archive does not carry.
fn embeds_icc_profile(path: &Path) -> bool {
    std::fs::File::open(path)
        .ok()
        .and_then(|f| {
            png::Decoder::new(std::io::BufReader::new(f))
                .read_info()
                .ok()
        })
        .is_some_and(|r| {
            r.info()
                .icc_profile
                .as_ref()
                .is_some_and(|p| !is_srgb_profile(p))
        })
}

/// Decode captures on worker threads and hand them back in the order given, a few ahead of
/// the consumer at most: a decoded iPad capture is 17 MB.
fn decode_ahead(
    paths: impl Iterator<Item = Result<PathBuf, String>>,
) -> impl Iterator<Item = Decoded> {
    let ahead = worker_count().clamp(1, 6);
    let mut pending: std::collections::VecDeque<std::thread::JoinHandle<Decoded>> =
        std::collections::VecDeque::new();
    let mut paths = paths.fuse();
    std::iter::from_fn(move || {
        while pending.len() < ahead {
            let Some(path) = paths.next() else { break };
            pending.push_back(std::thread::spawn(move || {
                let path = path?;
                let png_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let (w, h, pixels, bytes) = decode_png(&path)?;
                Ok((w, h, pixels, bytes, png_bytes, embeds_icc_profile(&path)))
            }));
        }
        let handle = pending.pop_front()?;
        Some(
            handle
                .join()
                .unwrap_or_else(|_| Err("a PNG decoder thread panicked".into())),
        )
    })
}

fn human_bytes(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1} MB", n as f64 / 1e6)
    } else {
        format!("{:.1} kB", n as f64 / 1e3)
    }
}

/// Check an archive against its index and, with `out`, write the capture tree back:
/// `<out>/<platform>/[<device>/]<variant>/<shot>.png` and `<out>/gallery.json`.
///
/// Every capture's pixels are held to the sha-256 the index records, so the tree holds exactly
/// the pixels that were packed. The PNG files are new encodings of them, so the written index
/// carries each file's new `bytes` and `sha256` and drops the archive fields.
pub fn unpack(opts: &UnpackOptions) -> Result<(), String> {
    use std::io::{Read as _, Seek as _};

    let mut doc = read_index(&opts.index)?;
    let info: ArchiveInfo = match doc.get("archive") {
        Some(a) if !a.is_null() => serde_json::from_value(a.clone())
            .map_err(|e| format!("{}: `archive`: {e}", opts.index.display()))?,
        _ => {
            return Err(format!(
                "{} describes no archive (`day screenshot pack` writes one)",
                opts.index.display()
            ));
        }
    };
    if info.format != ARCHIVE_FORMAT || info.compression != "zstd" {
        return Err(format!(
            "the archive is `{}` ({}), and this CLI reads `{ARCHIVE_FORMAT}` (zstd): update day-cli",
            info.format, info.compression
        ));
    }
    let archive = match &opts.archive {
        Some(p) => p.clone(),
        None => {
            let name = Path::new(&info.file);
            if name.components().count() != 1 || name.file_name().is_none() {
                return Err(format!(
                    "the archive name `{}` is not a file name",
                    info.file
                ));
            }
            opts.index.parent().unwrap_or(Path::new(".")).join(name)
        }
    };
    let io = |e: std::io::Error| format!("{}: {e}", archive.display());

    // The file first: its size and sha-256 as the index states them.
    let mut file = std::fs::File::open(&archive).map_err(io)?;
    let mut hash = sha2::Sha256::new();
    let mut size = 0u64;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).map_err(io)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        size += n as u64;
    }
    let digest = hex(hash.finalize());
    if size != info.bytes || digest != info.sha256 {
        return Err(format!(
            "{} is not the archive the index describes: {size} bytes, sha-256 {digest}; expected {} bytes, sha-256 {}",
            archive.display(),
            info.bytes,
            info.sha256
        ));
    }

    // Each capture's frame, by group, in stream order.
    let entries = doc["screenshots"].as_array().cloned().unwrap_or_default();
    let mut by_group: Vec<Vec<(usize, FrameRef, PathBuf)>> = vec![Vec::new(); info.groups.len()];
    for (i, e) in entries.iter().enumerate() {
        let name = e["path"].as_str().unwrap_or_default();
        let frame: FrameRef = serde_json::from_value(e["frame"].clone())
            .map_err(|err| format!("{name}: `frame`: {err}"))?;
        let rel = tree_path(Path::new(""), e)?;
        let channels = match frame.pixels.as_str() {
            "rgb24" => 3,
            "rgba" => 4,
            other => return Err(format!("{name}: unknown pixel format `{other}`")),
        };
        let expected = e["width"].as_u64().unwrap_or(0) * e["height"].as_u64().unwrap_or(0);
        if expected == 0 || frame.bytes != expected * channels {
            return Err(format!(
                "{name}: {} bytes of {} do not make the {}×{} image the index states",
                frame.bytes, frame.pixels, e["width"], e["height"]
            ));
        }
        by_group
            .get_mut(frame.group)
            .ok_or_else(|| format!("{name}: no archive group {}", frame.group))?
            .push((i, frame, rel));
    }

    // Encoding a PNG costs far more than decompressing its pixels, so the frames fan out to
    // worker threads; the bounded channel keeps a few captures in memory, not a group.
    let workers = worker_count().clamp(1, 8);
    type Job = (usize, FrameRef, PathBuf, u32, u32, Vec<u8>);
    let (job_tx, job_rx) = std::sync::mpsc::sync_channel::<Job>(workers);
    let job_rx = std::sync::Mutex::new(job_rx);
    let (done_tx, done_rx) = std::sync::mpsc::channel::<Result<(usize, u64, String), String>>();
    let out_dir = opts.out.as_deref();
    let total = entries.len();
    let outcome: Result<Vec<(usize, u64, String)>, String> = std::thread::scope(|scope| {
        for _ in 0..workers {
            let (job_rx, done_tx) = (&job_rx, done_tx.clone());
            scope.spawn(move || {
                loop {
                    // The lock is held for the receive alone, never across an encode.
                    let job = match job_rx.lock() {
                        Ok(rx) => rx.recv(),
                        Err(_) => return,
                    };
                    let Ok((i, frame, rel, width, height, bytes)) = job else {
                        return;
                    };
                    let result = (|| {
                        let Some(out_dir) = out_dir else {
                            return Ok((i, 0, String::new()));
                        };
                        let png = encode_png(width, height, &frame.pixels, &bytes)?;
                        let path = out_dir.join(&rel);
                        if let Some(parent) = path.parent() {
                            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                        }
                        std::fs::write(&path, &png)
                            .map_err(|e| format!("{}: {e}", path.display()))?;
                        Ok((i, png.len() as u64, sha256_hex(&png)))
                    })();
                    if done_tx.send(result).is_err() {
                        return;
                    }
                }
            });
        }
        drop(done_tx);

        let feed = (|| -> Result<(), String> {
            for (g, group) in info.groups.iter().enumerate() {
                let mut members = std::mem::take(&mut by_group[g]);
                members.sort_by_key(|(_, f, _)| f.offset);
                if members.len() as u64 != group.frames {
                    return Err(format!(
                        "group {g} ({}) holds {} capture(s), and the index names {}",
                        group.platform,
                        group.frames,
                        members.len()
                    ));
                }
                file.seek(std::io::SeekFrom::Start(group.offset))
                    .map_err(io)?;
                let mut decoder =
                    zstd::stream::read::Decoder::new((&mut file).take(group.bytes)).map_err(io)?;
                decoder.window_log_max(31).map_err(io)?;
                let mut at = 0u64;
                for (i, frame, rel) in members {
                    if frame.offset != at {
                        return Err(format!(
                            "{}: its pixels start at {}, and the previous capture ended at {at}",
                            rel.display(),
                            frame.offset
                        ));
                    }
                    let len = usize::try_from(frame.bytes).map_err(|e| e.to_string())?;
                    let mut bytes = vec![0u8; len];
                    decoder
                        .read_exact(&mut bytes)
                        .map_err(|e| format!("{}: {e}", rel.display()))?;
                    at += frame.bytes;
                    let digest = sha256_hex(&bytes);
                    if digest != frame.sha256 {
                        return Err(format!(
                            "{}: the pixels hash to {digest}, and the index expects {}",
                            rel.display(),
                            frame.sha256
                        ));
                    }
                    let e = &entries[i];
                    // Checked non-zero and within the frame's byte count above.
                    let width = e["width"].as_u64().unwrap_or(0) as u32;
                    let height = e["height"].as_u64().unwrap_or(0) as u32;
                    job_tx
                        .send((i, frame, rel, width, height, bytes))
                        .map_err(|_| "the PNG writers stopped".to_string())?;
                }
                let mut rest = [0u8; 1];
                if at != group.raw_bytes || decoder.read(&mut rest).map_err(io)? != 0 {
                    return Err(format!(
                        "group {g} ({}) holds more pixels than its captures account for",
                        group.platform
                    ));
                }
            }
            Ok(())
        })();
        drop(job_tx);
        let mut written = Vec::with_capacity(total);
        let mut failure = feed.err();
        for result in done_rx {
            match result {
                Ok(w) => written.push(w),
                Err(e) => failure = failure.or(Some(e)),
            }
        }
        match failure {
            Some(e) => Err(e),
            None => Ok(written),
        }
    });
    let written = outcome?;

    let Some(out_dir) = out_dir else {
        eprintln!(
            "{BOLD}    Checked{BOLD:#} {total} capture(s) in {}: every pixel checksum matches",
            archive.display()
        );
        return Ok(());
    };
    if let Some(list) = doc["screenshots"].as_array_mut() {
        for (i, bytes, sha256) in written {
            if let Some(e) = list[i].as_object_mut() {
                e.remove("frame");
                e.insert("bytes".into(), bytes.into());
                e.insert("sha256".into(), sha256.into());
            }
        }
    }
    if let Some(d) = doc.as_object_mut() {
        d.remove("archive");
    }
    write_index(&out_dir.join("gallery.json"), &doc)?;
    eprintln!(
        "{BOLD}     Unpack{BOLD:#} {total} capture(s) from {} → {}",
        archive.display(),
        out_dir.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_size_parses_pixels_scale_and_the_window_opt_out() {
        let size = parse_capture_size("2560x1600", 2.0).unwrap().unwrap();
        assert_eq!((size.width, size.height, size.scale), (2560, 1600, 2.0));
        assert_eq!(size.points(), (1280, 800));
        // `@` names the scale, over the table's.
        let one = parse_capture_size(" 1440X900@1 ", 2.0).unwrap().unwrap();
        assert_eq!((one.points(), one.scale), ((1440, 900), 1.0));
        assert_eq!(parse_capture_size("window", 2.0).unwrap(), None);
        assert_eq!(parse_capture_size("Window", 2.0).unwrap(), None);
    }

    #[test]
    fn capture_size_refuses_what_no_window_can_produce() {
        for bad in [
            "",
            "2560",
            "2560x",
            "0x1600",
            "2560x1600@0",
            "2560x1600@9",
            "axb",
        ] {
            assert!(parse_capture_size(bad, 2.0).is_err(), "{bad:?} parsed");
        }
        // 2561 pixels at 2x is half a point: the capture would come back a pixel off.
        let e = parse_capture_size("2561x1600", 2.0).unwrap_err();
        assert!(e.contains("whole number of points"), "{e}");
    }

    #[test]
    fn the_default_capture_is_a_mac_app_store_size() {
        let shots = crate::meta::Screenshots::default();
        let size = parse_capture_size(&shots.desktop_size, shots.desktop_scale)
            .unwrap()
            .unwrap();
        // The four sizes App Store Connect takes for a Mac screenshot.
        let accepted = [(1280, 800), (1440, 900), (2560, 1600), (2880, 1800)];
        assert!(accepted.contains(&(size.width, size.height)));
        assert_eq!(size.points(), (1280, 800));
    }

    #[test]
    fn capture_envs_leave_a_variable_the_caller_set() {
        let size = parse_capture_size("2560x1600", 2.0).unwrap().unwrap();
        let given = vec![("DAY_WINDOW".to_string(), "500x640".to_string())];
        let envs = capture_envs(size, &given);
        assert!(!envs.iter().any(|(k, _)| k == "DAY_WINDOW"));
        // Only asserted when the test environment itself does not set it.
        if std::env::var_os("DAY_CAPTURE_SCALE").is_none() {
            assert!(envs.contains(&("DAY_CAPTURE_SCALE".to_string(), "2".to_string())));
        }
    }

    #[test]
    fn text_resolution_falls_back_language_then_english() {
        let map = Text::ByLocale(BTreeMap::from([
            ("en".into(), "Home".into()),
            ("fr-FR".into(), "Accueil".into()),
        ]));
        assert_eq!(map.resolve("fr-FR"), Some("Accueil"));
        assert_eq!(map.resolve("fr"), Some("Accueil")); // language match
        assert_eq!(map.resolve("zh-CN"), Some("Home")); // English fallback
        let no_en = Text::ByLocale(BTreeMap::from([("fr".into(), "Accueil".into())]));
        assert_eq!(no_en.resolve("de"), Some("Accueil")); // any value beats none
        assert_eq!(Text::Plain("X".into()).resolve("anything"), Some("X"));
    }

    #[test]
    fn variant_names_parse_to_theme_and_locale() {
        assert_eq!(parse_variant("default"), (None, None));
        assert_eq!(parse_variant("light"), (Some("light"), None));
        assert_eq!(parse_variant("dark-fr"), (Some("dark"), Some("fr")));
        assert_eq!(parse_variant("light-zh-CN"), (Some("light"), Some("zh-CN")));
        assert_eq!(parse_variant("fr"), (None, Some("fr")));
        // Not locale-shaped: a local capture's ad-hoc variant claims no locale.
        assert_eq!(parse_variant("uicheck"), (None, None));
        assert_eq!(parse_variant("light-uicheck"), (Some("light"), None));
    }

    #[test]
    fn derived_labels_title_case_ids() {
        assert_eq!(derived_label("home"), "Home");
        assert_eq!(derived_label("list-item-100"), "List Item 100");
    }

    #[test]
    fn iso_utc_now_is_shaped_like_iso8601() {
        let s = iso_utc_now();
        assert_eq!(s.len(), 20, "{s}");
        assert_eq!(&s[4..5], "-");
        assert!(s.ends_with('Z'));
        // The year is sane (catches an off-by-era in the civil arithmetic).
        let year: i32 = s[..4].parse().unwrap();
        assert!((2024..2100).contains(&year), "{s}");
    }

    #[test]
    fn extract_meta_strips_the_step() {
        let mut step = serde_json::from_str::<serde_json::Map<_, _>>(
            r#"{"op":"screenshot","name":"home","title":{"en":"Home","fr":"Accueil"},"caption":"The hub","source":"src/lib.rs","store":2}"#,
        )
        .unwrap();
        let meta = extract_meta(&mut step);
        assert!(
            !step.contains_key("title")
                && !step.contains_key("caption")
                && !step.contains_key("source")
                && !step.contains_key("store")
        );
        assert_eq!(meta.title.unwrap().resolve("fr"), Some("Accueil"));
        assert_eq!(meta.caption.unwrap().resolve("en"), Some("The hub"));
        assert_eq!(meta.source.as_deref(), Some("src/lib.rs"));
        // The retired `store:` key is stripped like the others and remembered, so the runner
        // can say once that the listing lives in store/storefront.toml now.
        assert!(meta.legacy_store);
        let mut step =
            serde_json::from_str::<serde_json::Map<_, _>>(r#"{"op":"screenshot","name":"x"}"#)
                .unwrap();
        assert!(!extract_meta(&mut step).legacy_store);
    }

    #[test]
    fn record_upserts_and_prunes() {
        let dir = std::env::temp_dir().join(format!("day-shot-index-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let vdir = dir.join("macos-appkit").join("light");
        std::fs::create_dir_all(&vdir).unwrap();
        // A tiny valid-enough PNG header (8 magic + IHDR chunk).
        let mut png = vec![
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d,
        ];
        png.extend(b"IHDR");
        png.extend(2u32.to_be_bytes());
        png.extend(3u32.to_be_bytes());
        std::fs::write(vdir.join("home.png"), &png).unwrap();
        let entry = target_entry(
            &vdir.join("home.png"),
            "light",
            None,
            "home",
            Some("en"),
            None,
        )
        .unwrap();
        assert_eq!((entry.width, entry.height), (Some(2), Some(3)));
        record_target_entries(&dir, "macos-appkit", vec![entry.clone()]);
        // Upsert replaces rather than duplicates; a vanished file is pruned.
        record_target_entries(&dir, "macos-appkit", vec![entry]);
        let idx: TargetIndex = serde_json::from_str(
            &std::fs::read_to_string(dir.join("macos-appkit/gallery.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(idx.screenshots.len(), 1);
        std::fs::remove_file(vdir.join("home.png")).unwrap();
        let ghost = TargetEntry {
            file: "gone.png".into(),
            variant: "light".into(),
            theme: Some("light".into()),
            device: None,
            shot: "gone".into(),
            locale: None,
            title: None,
            caption: None,
            source: None,
            width: None,
            height: None,
            bytes: 0,
            sha256: String::new(),
        };
        record_target_entries(&dir, "macos-appkit", vec![ghost]);
        let idx: TargetIndex = serde_json::from_str(
            &std::fs::read_to_string(dir.join("macos-appkit/gallery.json")).unwrap(),
        )
        .unwrap();
        assert!(idx.screenshots.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A list declared on the target alone serves the store the rules stage for it: the index's
    /// `stores` block carries the store's resolved list, which is the only place the stager and
    /// the App Fair's queue read a store's captures from (Games-Fair v2.1.9 shipped an index
    /// whose `stores` was empty, and the queue refused the submission as "declares none").
    #[test]
    fn a_target_level_list_serves_the_store_the_rules_stage_for_it() {
        let dir = std::env::temp_dir().join(format!("day-shots-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        png.extend(b"IHDR");
        png.extend(2u32.to_be_bytes());
        png.extend(3u32.to_be_bytes());
        let project_dir = dir.join("project");
        std::fs::create_dir_all(project_dir.join("store")).unwrap();
        std::fs::write(
            project_dir.join("Day.toml"),
            "schema = 1\n[app]\nid = \"dev.example.app\"\ntargets = [\"android-mdc\"]\n",
        )
        .unwrap();
        std::fs::write(
            project_dir.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        std::fs::write(
            project_dir.join("store/storefront.toml"),
            "[storefront.android-mdc.screenshots]\ndefault = [\"home\"]\n",
        )
        .unwrap();
        let project = crate::meta::find_project(Some(&project_dir)).unwrap();
        // Two profiles: the phone, and the optional API-floor row a CI matrix adds under its own
        // slug, which Google Play has no slot for.
        let tree = dir.join("shots");
        let mut entries = Vec::new();
        for device in ["phone", "medium-phone-24"] {
            let vdir = tree.join("android-mdc").join(device).join("light");
            std::fs::create_dir_all(&vdir).unwrap();
            std::fs::write(vdir.join("home.png"), &png).unwrap();
            entries.push(
                target_entry(
                    &vdir.join("home.png"),
                    "light",
                    Some(device),
                    "home",
                    Some("en"),
                    None,
                )
                .unwrap(),
            );
        }
        record_target_entries(&tree, "android-mdc", entries);
        let out = dir.join("gallery.json");
        index(
            &project,
            &IndexOptions {
                screenshot_paths: vec![tree.clone()],
                out: Some(out.clone()),
            },
        )
        .unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        let listings = &doc["listings"]["android-mdc"];
        let home = serde_json::json!([{ "shot": "home", "theme": "light" }]);
        assert_eq!(listings["website"]["phone"]["en"], home);
        assert_eq!(listings["website"]["medium-phone-24"]["en"], home);
        let play = &listings["stores"]["google-play-store"];
        assert_eq!(
            play["phone"]["en"], home,
            "the store the rules stage for android-mdc takes the target's list"
        );
        assert!(
            play.get("medium-phone-24").is_none(),
            "a profile the store has no slot for stays out of its block: {play}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Each device profile's captures go to their own index, `<target>/<device>/gallery.json`,
    /// and `index` reads those beside the target's own, so two profiles that ran on two
    /// machines never wrote the same file. Two artifacts from an older runner, each with its
    /// device in `<target>/gallery.json`, still merge whole when handed over as two roots.
    #[test]
    fn device_indexes_live_apart_and_merge_whole() {
        let dir = std::env::temp_dir().join(format!("day-shots-split-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        png.extend(b"IHDR");
        png.extend(2u32.to_be_bytes());
        png.extend(3u32.to_be_bytes());
        // A project, for `index` to read its locale and site from.
        let project_dir = dir.join("project");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(
            project_dir.join("Day.toml"),
            "schema = 1\n[app]\nid = \"dev.example.app\"\ntargets = [\"ios-uikit\"]\n",
        )
        .unwrap();
        std::fs::write(
            project_dir.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        // The listings: the website's list, an iPhone list of its own, and the iPad falling
        // back to the target's default; a store the CLI does not know rides through.
        std::fs::create_dir_all(project_dir.join("store")).unwrap();
        std::fs::write(
            project_dir.join("store/storefront.toml"),
            "[storefront.ios-uikit.screenshots]\ndefault = [\"home\"]\n\
             [storefront.ios-uikit.screenshots.fr]\ndefault = [\"home\", \"home\", \"home\"]\n\
             [storefront.ios-uikit.apple-app-store.screenshots]\niphone = [{ name = \"home\", theme = \"dark\" }]\n\
             [storefront.ios-uikit.altstore.screenshots]\ndefault = [\"home\", \"home\"]\n",
        )
        .unwrap();
        let project = crate::meta::find_project(Some(&project_dir)).unwrap();
        let marked = |n: u32| ShotMeta {
            title: Some(Text::Plain("Home".into())),
            caption: Some(Text::Plain(format!("mark {n}"))),
            ..ShotMeta::default()
        };

        // One tree, two device profiles recorded one after the other, as a local run does.
        let tree = dir.join("one");
        for device in ["iphone", "ipad"] {
            let vdir = tree.join("ios-uikit").join(device).join("light");
            std::fs::create_dir_all(&vdir).unwrap();
            std::fs::write(vdir.join("home.png"), &png).unwrap();
            let entry = target_entry(
                &vdir.join("home.png"),
                "light",
                Some(device),
                "home",
                Some("en"),
                Some(&marked(1)),
            )
            .unwrap();
            record_target_entries(&tree, "ios-uikit", vec![entry]);
        }
        assert!(tree.join("ios-uikit/iphone/gallery.json").is_file());
        assert!(tree.join("ios-uikit/ipad/gallery.json").is_file());
        assert!(
            !tree.join("ios-uikit/gallery.json").exists(),
            "no shared file to collide on"
        );
        let out = dir.join("one.json");
        index(
            &project,
            &IndexOptions {
                screenshot_paths: vec![tree.clone()],
                out: Some(out.clone()),
            },
        )
        .unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        let marked_devices = |doc: &serde_json::Value| -> Vec<String> {
            let mut v: Vec<String> = doc["screenshots"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["caption"].as_str() == Some("mark 1") && e["title"].is_string())
                .map(|e| e["device"].as_str().unwrap().to_string())
                .collect();
            v.sort();
            v
        };
        assert_eq!(marked_devices(&doc), ["ipad", "iphone"]);
        let listings = &doc["listings"]["ios-uikit"];
        assert_eq!(
            listings["website"]["iphone"]["en"],
            serde_json::json!([{ "shot": "home", "theme": "light" }])
        );
        assert_eq!(
            listings["website"]["ipad"]["en"],
            serde_json::json!([{ "shot": "home", "theme": "light" }])
        );
        assert_eq!(
            listings["website"]["ipad"]["fr"].as_array().map(Vec::len),
            Some(3),
            "a declared locale is resolved even when nothing was captured in it"
        );
        assert_eq!(
            listings["stores"]["apple-app-store"]["iphone"]["en"],
            serde_json::json!([{ "shot": "home", "theme": "dark" }])
        );
        assert_eq!(
            listings["stores"]["apple-app-store"]["ipad"]["en"],
            serde_json::json!([{ "shot": "home", "theme": "light" }]),
            "the iPad takes the target's default"
        );
        assert_eq!(
            listings["stores"]["apple-app-store"]["ipad"]["fr"]
                .as_array()
                .map(Vec::len),
            Some(3),
            "the store falls through to the target's French list"
        );
        assert_eq!(
            listings["stores"]["altstore"]["ipad"]["en"]
                .as_array()
                .map(Vec::len),
            Some(2)
        );
        assert!(
            doc["screenshots"][0].get("store").is_none(),
            "the per-capture mark is gone"
        );
        // A target table with submission info and no screenshot lists declares nothing about
        // screenshots: no listing, so a site falls back to showing every capture rather than
        // obeying an empty list.
        std::fs::write(
            project_dir.join("store/storefront.toml"),
            "[storefront.ios-uikit.apple-app-store.submission-info]\napple-category = \"GAMES\"\n",
        )
        .unwrap();
        std::fs::remove_file(project_dir.join("store/app.toml")).ok();
        let project = crate::meta::find_project(Some(&project_dir)).unwrap();
        let out = dir.join("stub.json");
        index(
            &project,
            &IndexOptions {
                screenshot_paths: vec![tree.clone()],
                out: Some(out.clone()),
            },
        )
        .unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert!(
            doc["listings"].get("ios-uikit").is_none(),
            "no screenshot lists, no listing: {}",
            doc["listings"]
        );

        // Two artifacts of an older runner: each root has its device in
        // `ios-uikit/gallery.json`. Handed over as two roots, both keep their metadata.
        for device in ["iphone", "ipad"] {
            let root = dir.join("artifacts").join(format!("screenshots-{device}"));
            let vdir = root.join("ios-uikit").join(device).join("light");
            std::fs::create_dir_all(&vdir).unwrap();
            std::fs::write(vdir.join("home.png"), &png).unwrap();
            let entry = target_entry(
                &vdir.join("home.png"),
                "light",
                Some(device),
                "home",
                Some("en"),
                Some(&marked(1)),
            )
            .unwrap();
            let idx = TargetIndex {
                generator: "test".into(),
                target: "ios-uikit".into(),
                screenshots: vec![entry],
            };
            std::fs::write(
                root.join("ios-uikit/gallery.json"),
                serde_json::to_string(&idx).unwrap(),
            )
            .unwrap();
        }
        let out = dir.join("two.json");
        index(
            &project,
            &IndexOptions {
                screenshot_paths: vec![
                    dir.join("artifacts/screenshots-iphone"),
                    dir.join("artifacts/screenshots-ipad"),
                ],
                out: Some(out.clone()),
            },
        )
        .unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(marked_devices(&doc), ["ipad", "iphone"]);

        // A stale shared index from before the split gives up the device it re-records.
        let tree = dir.join("stale");
        let vdir = tree.join("ios-uikit/iphone/light");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("home.png"), &png).unwrap();
        let old = target_entry(
            &vdir.join("home.png"),
            "light",
            Some("iphone"),
            "home",
            Some("en"),
            None,
        )
        .unwrap();
        std::fs::write(
            tree.join("ios-uikit/gallery.json"),
            serde_json::to_string(&TargetIndex {
                generator: "test".into(),
                target: "ios-uikit".into(),
                screenshots: vec![old],
            })
            .unwrap(),
        )
        .unwrap();
        let fresh = target_entry(
            &vdir.join("home.png"),
            "light",
            Some("iphone"),
            "home",
            Some("en"),
            Some(&marked(3)),
        )
        .unwrap();
        record_target_entries(&tree, "ios-uikit", vec![fresh]);
        let shared: TargetIndex = serde_json::from_str(
            &std::fs::read_to_string(tree.join("ios-uikit/gallery.json")).unwrap(),
        )
        .unwrap();
        assert!(
            shared.screenshots.is_empty(),
            "the shared file no longer describes the iphone"
        );
        let out = dir.join("stale.json");
        index(
            &project,
            &IndexOptions {
                screenshot_paths: vec![tree.clone()],
                out: Some(out.clone()),
            },
        )
        .unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(doc["screenshots"][0]["caption"].as_str(), Some("mark 3"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A device level and a device-less level coexist in one target's tree and one index.
    ///
    /// The tree walk tells them apart by content, not by name: `ipad/` holds directories, so it
    /// is a device; `light/` holds captures, so it is a variant. Getting that wrong either hides
    /// every device capture or invents a device called "light", and both look like an empty
    /// gallery column rather than an error.
    #[test]
    fn a_device_level_and_a_plain_variant_coexist() {
        let dir = std::env::temp_dir().join(format!("day-shots-dev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let plain = dir.join("ios-uikit/light");
        let ipad = dir.join("ios-uikit/ipad/dark");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::create_dir_all(&ipad).unwrap();
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        png.extend(b"IHDR");
        png.extend(2u32.to_be_bytes());
        png.extend(3u32.to_be_bytes());
        std::fs::write(plain.join("home.png"), &png).unwrap();
        std::fs::write(ipad.join("home.png"), &png).unwrap();

        let found = variant_dirs(&dir.join("ios-uikit"));
        assert!(
            found.contains(&(None, "light".to_string(), plain.clone())),
            "the plain variant was not seen as one: {found:?}"
        );
        assert!(
            found.contains(&(Some("ipad".to_string()), "dark".to_string(), ipad.clone())),
            "the device level was not seen as one: {found:?}"
        );

        // Both survive an upsert, because the key includes the device.
        let a = target_entry(&plain.join("home.png"), "light", None, "home", None, None).unwrap();
        let b = target_entry(
            &ipad.join("home.png"),
            "dark",
            Some("ipad"),
            "home",
            None,
            None,
        )
        .unwrap();
        record_target_entries(&dir, "ios-uikit", vec![a, b]);
        let idx: TargetIndex = serde_json::from_str(
            &std::fs::read_to_string(dir.join("ios-uikit/gallery.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            idx.screenshots.len(),
            2,
            "same shot, same file, different device — one must not evict the other"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A fixture capture: its path under the tree, its size, and its archive pixels.
    type ExpectedCapture = (String, u32, u32, Vec<u8>);

    /// A capture tree of generated PNGs and the merged-index shape `pack` reads: two groups,
    /// an opaque RGBA capture, a translucent one, and a grayscale one.
    fn archive_fixture(name: &str) -> (PathBuf, Vec<ExpectedCapture>) {
        let dir = std::env::temp_dir().join(format!("day-shots-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let shade = |n: usize, seed: u8| -> Vec<u8> {
            (0..n).map(|i| (i as u8).wrapping_mul(31) ^ seed).collect()
        };
        let rgb = shade(6 * 4 * 3, 7);
        let opaque: Vec<u8> = rgb
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect();
        // (path under the tree, width, height, color, png samples, expected archive pixels)
        let shots = [
            (
                "ios-uikit/ipad/light/home.png",
                6u32,
                4u32,
                png::ColorType::Rgba,
                opaque,
                rgb,
            ),
            (
                "ios-uikit/ipad/dark/home.png",
                6,
                4,
                png::ColorType::Rgba,
                shade(6 * 4 * 4, 3),
                shade(6 * 4 * 4, 3),
            ),
            (
                "macos-appkit/light/home.png",
                5,
                3,
                png::ColorType::Grayscale,
                shade(15, 9),
                shade(15, 9).iter().flat_map(|&g| [g, g, g]).collect(),
            ),
        ];
        let mut entries = Vec::new();
        let mut expected = Vec::new();
        for (path, w, h, color, samples, pixels) in shots {
            let file = dir.join("tree").join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            let mut bytes = Vec::new();
            let mut enc = png::Encoder::new(&mut bytes, w, h);
            enc.set_color(color);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&samples).unwrap();
            wr.finish().unwrap();
            std::fs::write(&file, &bytes).unwrap();
            let parts: Vec<&str> = path.split('/').collect();
            entries.push(serde_json::json!({
                "file": "home.png",
                "path": format!("gallery/{path}"),
                "shot": "home",
                "platform": parts[0],
                "device": (parts.len() == 4).then(|| parts[1]),
                "variant": parts[parts.len() - 2],
                "width": w,
                "height": h,
                "bytes": bytes.len(),
                "sha256": sha256_hex(&bytes),
            }));
            expected.push((path.to_string(), w, h, pixels));
        }
        write_index(
            &dir.join("tree/gallery.json"),
            &serde_json::json!({ "generator": "day screenshot index", "screenshots": entries }),
        )
        .unwrap();
        (dir, expected)
    }

    fn pack_fixture(dir: &Path) -> serde_json::Value {
        pack(&PackOptions {
            index: dir.join("tree/gallery.json"),
            root: None,
            out: Some(dir.join("assets").join(ARCHIVE_FILE)),
            index_out: Some(dir.join("assets/gallery.json")),
        })
        .unwrap();
        read_index(&dir.join("assets/gallery.json")).unwrap()
    }

    #[test]
    fn an_archive_returns_the_pixels_it_was_given() {
        let (dir, expected) = archive_fixture("roundtrip");
        let packed = pack_fixture(&dir);
        // One group per (platform, device), each its own Zstandard frame, back to back.
        let groups = packed["archive"]["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0]["device"], "ipad");
        assert_eq!(groups[0]["raw_bytes"], 72 + 96);
        assert_eq!(groups[1]["offset"], groups[0]["bytes"]);
        assert_eq!(packed["archive"]["format"], ARCHIVE_FORMAT);
        let shots = packed["screenshots"].as_array().unwrap();
        assert_eq!(shots[0]["frame"]["pixels"], "rgb24"); // opaque RGBA drops its alpha
        assert_eq!(shots[1]["frame"]["pixels"], "rgba");
        assert_eq!(shots[1]["frame"]["offset"], 72);
        assert_eq!(shots[2]["frame"]["group"], 1);
        for (shot, (_, _, _, pixels)) in shots.iter().zip(&expected) {
            assert_eq!(shot["frame"]["sha256"], sha256_hex(pixels));
        }

        // The archive checks clean, then unpacks to the same tree with the same pixels.
        let opts = |out: Option<PathBuf>| UnpackOptions {
            index: dir.join("assets/gallery.json"),
            archive: None,
            out,
        };
        unpack(&opts(None)).unwrap();
        unpack(&opts(Some(dir.join("out")))).unwrap();
        for (path, w, h, pixels) in &expected {
            let (dw, dh, _, decoded) = decode_png(&dir.join("out").join(path)).unwrap();
            assert_eq!((dw, dh), (*w, *h));
            assert_eq!(&decoded, pixels, "{path}");
        }
        // The unpacked index describes the files written and no archive.
        let unpacked = read_index(&dir.join("out/gallery.json")).unwrap();
        assert!(unpacked.get("archive").is_none());
        for (shot, (path, ..)) in unpacked["screenshots"]
            .as_array()
            .unwrap()
            .iter()
            .zip(&expected)
        {
            let bytes = std::fs::read(dir.join("out").join(path)).unwrap();
            assert!(shot.get("frame").is_none());
            assert_eq!(shot["bytes"], bytes.len());
            assert_eq!(shot["sha256"], sha256_hex(&bytes));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unpack_refuses_what_the_index_does_not_describe() {
        let (dir, _) = archive_fixture("refuse");
        let packed = pack_fixture(&dir);
        let unpack_with = |name: &str, doc: &serde_json::Value| {
            let index = dir.join("assets").join(name);
            write_index(&index, doc).unwrap();
            unpack(&UnpackOptions {
                index,
                archive: Some(dir.join("assets").join(ARCHIVE_FILE)),
                out: None,
            })
        };
        // A capture whose pixels hash to something else.
        let mut doc = packed.clone();
        doc["screenshots"][1]["frame"]["sha256"] = "00".repeat(32).into();
        let err = unpack_with("pixels.json", &doc).unwrap_err();
        assert!(err.contains("the pixels hash to"), "{err}");
        // A different archive file.
        let mut doc = packed.clone();
        doc["archive"]["sha256"] = "00".repeat(32).into();
        let err = unpack_with("file.json", &doc).unwrap_err();
        assert!(
            err.contains("is not the archive the index describes"),
            "{err}"
        );
        // A path that climbs out of the output directory.
        let mut doc = packed.clone();
        doc["screenshots"][0]["path"] = "gallery/../../escape.png".into();
        let err = unpack_with("path.json", &doc).unwrap_err();
        assert!(err.contains("leaves the capture tree"), "{err}");
        // An index nobody packed.
        let mut doc = packed;
        doc.as_object_mut().unwrap().remove("archive");
        let err = unpack_with("bare.json", &doc).unwrap_err();
        assert!(err.contains("describes no archive"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A minimal ICC profile whose only tag is a `desc` naming it.
    fn icc_named(name: &str) -> Vec<u8> {
        let mut tag = b"desc\0\0\0\0".to_vec();
        tag.extend((name.len() as u32 + 1).to_be_bytes());
        tag.extend(name.as_bytes());
        tag.push(0);
        let mut profile = vec![0u8; 128];
        profile.extend(1u32.to_be_bytes());
        profile.extend(b"desc");
        profile.extend(144u32.to_be_bytes());
        profile.extend((tag.len() as u32).to_be_bytes());
        profile.extend(tag);
        profile
    }

    /// The chunk names of a PNG file, `IDAT` once.
    fn chunk_names(bytes: &[u8]) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        let mut at = 8;
        while at + 8 <= bytes.len() {
            let len = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            let name = String::from_utf8_lossy(&bytes[at + 4..at + 8]).into_owned();
            if names.last() != Some(&name) {
                names.push(name);
            }
            at += 12 + len;
        }
        names
    }

    #[test]
    fn a_capture_is_normalized_to_one_file_shape() {
        let dir = std::env::temp_dir().join(format!("day-shots-normal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, samples: &[u8], icc: Option<Vec<u8>>| {
            let mut info = png::Info::with_size(4, 2);
            info.color_type = png::ColorType::Rgba;
            info.bit_depth = png::BitDepth::Eight;
            info.icc_profile = icc.map(Into::into);
            info.pixel_dims = Some(png::PixelDimensions {
                xppu: 3779,
                yppu: 3779,
                unit: png::Unit::Meter,
            });
            let mut bytes = Vec::new();
            let mut enc = png::Encoder::with_info(&mut bytes, info).unwrap();
            enc.add_text_chunk("Software".into(), "a capture tool".into())
                .unwrap();
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(samples).unwrap();
            wr.finish().unwrap();
            let path = dir.join(name);
            std::fs::write(&path, &bytes).unwrap();
            path
        };
        let opaque: Vec<u8> = (0..8u8).flat_map(|i| [i * 9, 200 - i, i, 255]).collect();
        let mut rounded = opaque.clone();
        rounded[3] = 0; // one transparent corner pixel

        // Opaque RGBA with stray chunks becomes RGB with an sRGB tag and nothing else.
        let path = write("opaque.png", &opaque, None);
        assert!(normalize_capture(&path).unwrap());
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(chunk_names(&bytes), ["IHDR", "sRGB", "IDAT", "IEND"]);
        assert_eq!(bytes[25], 2, "color type RGB");
        let (w, h, pixels, rgb) = decode_png(&path).unwrap();
        assert_eq!((w, h, pixels), (4, 2, "rgb24"));
        let (px, _) = opaque.as_chunks::<4>();
        assert_eq!(
            rgb,
            px.iter()
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect::<Vec<u8>>()
        );
        // A second pass writes the same bytes: the index's sha-256 is stable.
        assert!(normalize_capture(&path).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);

        // A translucent pixel keeps the alpha channel, sample for sample.
        let path = write("rounded.png", &rounded, None);
        assert!(normalize_capture(&path).unwrap());
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(chunk_names(&bytes), ["IHDR", "sRGB", "IDAT", "IEND"]);
        assert_eq!(decode_png(&path).unwrap(), (4, 2, "rgba", rounded));

        // An embedded sRGB profile is replaced by the tag; a display's profile stops the
        // rewrite, because its samples are not sRGB.
        let path = write("srgb.png", &opaque, Some(icc_named("sRGB IEC61966-2.1")));
        assert!(normalize_capture(&path).unwrap());
        assert_eq!(
            chunk_names(&std::fs::read(&path).unwrap()),
            ["IHDR", "sRGB", "IDAT", "IEND"]
        );
        let path = write("display.png", &opaque, Some(icc_named("Display")));
        let before = std::fs::read(&path).unwrap();
        assert!(!normalize_capture(&path).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(embeds_icc_profile(&path));

        std::fs::write(dir.join("text.png"), "not a png").unwrap();
        assert!(normalize_capture(&dir.join("text.png")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
