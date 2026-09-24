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

use std::collections::BTreeMap;
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
fn primary_language(tag: &str) -> &str {
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
fn png_dims(bytes: &[u8]) -> Option<(u32, u32)> {
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
    /// The run's actual `--locale`, when one was passed: ground truth the variant name only
    /// approximates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<Text>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<Text>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The capture's position in the store listing, when the step declared `store:` (§14.7).
    /// Absent for the captures a walkthrough takes as evidence rather than for the listing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<u32>,
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
        title: meta.and_then(|m| m.title.clone()),
        caption: meta.and_then(|m| m.caption.clone()),
        source: meta.and_then(|m| m.source.clone()),
        store: meta.and_then(|m| m.store),
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
    /// `store: N`: this capture is the Nth screenshot of the store listing, in every locale and
    /// on every device the walkthrough runs on. `store: true` is position 1.
    pub store: Option<u32>,
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
        store: step.remove("store").and_then(|v| match v {
            serde_json::Value::Bool(true) => Some(1),
            serde_json::Value::Number(n) => n.as_u64().filter(|n| *n > 0).map(|n| n as u32),
            _ => None,
        }),
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
            if m.store.is_none() {
                m.store = e.store;
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
            let (theme, vlocale) = parse_variant(&e.variant);
            let locale = e
                .locale
                .clone()
                .or_else(|| vlocale.map(str::to_string))
                .or_else(|| theme.is_some().then(|| fallback_locale.clone()));
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
                // The listing position, when the step declared one: what `day store stage
                // --screenshots` and a catalog's review select on.
                "store": e.store.or_else(|| meta.and_then(|m| m.store)),
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
                "store": m.store,
            })
        })
        .collect();

    let doc = serde_json::json!({
        "generator": "day screenshot index",
        "generated": iso_utc_now(),
        "site": host.as_ref().map(|(o, b)| format!("{o}{b}")),
        "themes": themes,
        "locales": locales,
        "platforms": platforms,
        "shots": shots,
        "screenshots": screenshots,
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
        assert_eq!(meta.store, Some(2));
        // `store: true` is the first position; zero and a string are no position at all.
        for (raw, want) in [("true", Some(1)), ("0", None), ("\"first\"", None)] {
            let mut step = serde_json::from_str::<serde_json::Map<_, _>>(&format!(
                r#"{{"op":"screenshot","name":"x","store":{raw}}}"#
            ))
            .unwrap();
            assert_eq!(extract_meta(&mut step).store, want, "store: {raw}");
        }
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
            device: None,
            shot: "gone".into(),
            locale: None,
            title: None,
            caption: None,
            source: None,
            store: None,
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
        let project = crate::meta::find_project(Some(&project_dir)).unwrap();
        let marked = |store: u32| ShotMeta {
            title: Some(Text::Plain("Home".into())),
            store: Some(store),
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
                .filter(|e| e["store"].as_u64() == Some(1) && e["title"].is_string())
                .map(|e| e["device"].as_str().unwrap().to_string())
                .collect();
            v.sort();
            v
        };
        assert_eq!(marked_devices(&doc), ["ipad", "iphone"]);

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
        assert_eq!(doc["screenshots"][0]["store"].as_u64(), Some(3));
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
}
