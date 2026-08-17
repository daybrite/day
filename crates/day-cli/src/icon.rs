// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `day icon` (docs/icons.md, DESIGN.md §16.5) — every platform's app-icon set from ONE
//! master, kept in sync.
//!
//! Master discovery (first hit wins): an explicit path argument, else
//! `resource/icons/icon.svg`, `resource/icons/day-icon.svg`, `resource/icons/icon.png`.
//!
//! An SVG master may mark top-level groups as semantic layers by id:
//! `day:background`, `day:foreground` (any number), `day:monochrome`, `day:dark`.
//! The composite (background+foregrounds) feeds every full-bleed output; the split layers feed
//! Android's adaptive icon. An unlayered SVG (or a PNG master) still produces the full legacy
//! set — the adaptive foreground is then the whole art in the safe zone over a derived
//! background color. `day:monochrome`/`day:dark` are reserved for the modern formats
//! (Icon Composer, themed icons) and are excluded from every composite today.
//!
//! Everything renders in memory first; `--check` compares those bytes against the working tree
//! and exits 5 on drift (the duty-matrix pattern — CI's gate), a plain run writes them plus
//! `resource/icons/icons.lock.json` recording the master and generator. Byte-stable renders
//! hold per generator version; the lock's `generator` field is how `--check` tells "you edited
//! an output" from "regenerate with this day version".

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use day_vector::tiny_skia;

use crate::meta::Project;
use crate::ops::status;

/// Lock path, relative to the project root.
const LOCK: &str = "resource/icons/icons.lock.json";
/// The macOS margin convention: 824 pt of art on the 1024 canvas, radius 184.
const MAC_INSET: f32 = 100.0;
const MAC_ART: f32 = 824.0;
const MAC_RADIUS: f32 = 184.0;
/// Android adaptive canvas and safe zone (108 dp canvas, 66 dp safe → 432/264 px at xxxhdpi).
const ADAPTIVE_PX: u32 = 432;
const SAFE_PX: f32 = 264.0;

pub struct IconOptions {
    pub master: Option<PathBuf>,
    pub check: bool,
    pub platforms: Vec<String>,
}

/// Resolve a `--seed` / `--icon-seed` spec: a bare integer is used as-is, anything else is
/// hashed ([`day_vector::icongen::seed_from_str`] — how `day new` seeds from the app id),
/// and `None` draws fresh entropy. Always tell the user the number (via the return), so a
/// liked random icon can be reproduced.
pub fn resolve_seed(spec: Option<&str>) -> u64 {
    match spec {
        Some(s) => s
            .parse::<u64>()
            .unwrap_or_else(|_| day_vector::icongen::seed_from_str(s)),
        None => {
            use std::hash::{BuildHasher as _, Hasher as _};
            let clock = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
                .unwrap_or(0);
            let keyed = std::collections::hash_map::RandomState::new()
                .build_hasher()
                .finish();
            clock ^ keyed
        }
    }
}

/// `day icon --generate`: write the seeded master to `resource/icons/icon.svg`. Refuses to
/// clobber an existing master (any discovery candidate) unless `overwrite` — a hand-drawn
/// icon is unrecoverable. The caller then runs [`run`] to regenerate every output.
pub fn generate_master(project: &Project, seed: u64, overwrite: bool) -> Result<PathBuf, String> {
    if !overwrite {
        for candidate in [
            "resource/icons/icon.svg",
            "resource/icons/day-icon.svg",
            "resource/icons/icon.png",
        ] {
            let p = project.root.join(candidate);
            if p.exists() {
                return Err(format!(
                    "a master icon already exists ({candidate}) — pass --overwrite to replace it"
                ));
            }
        }
    }
    let dest = project.root.join("resource/icons/icon.svg");
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    }
    std::fs::write(&dest, day_vector::icongen::generate(seed))
        .map_err(|e| format!("write {}: {e}", dest.display()))?;
    Ok(dest)
}

/// `day icon --generate --out <file.svg>`: preview mode — write the seeded master to an
/// arbitrary path (no project needed, nothing else touched) plus a 512 px PNG render beside
/// it, so seeds can be browsed before committing to one.
pub fn generate_preview(path: &Path, seed: u64) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    }
    let svg = day_vector::icongen::generate(seed);
    std::fs::write(path, &svg).map_err(|e| format!("write {}: {e}", path.display()))?;
    let tree = day_vector::parse(svg.as_bytes())?;
    let png = day_vector::render_png(&tree, 512)?;
    let png_path = path.with_extension("png");
    std::fs::write(&png_path, png).map_err(|e| format!("write {}: {e}", png_path.display()))?;
    Ok(())
}

/// Drift lines (for exit code 5), or a hard error.
pub enum IconError {
    Drift(Vec<String>),
    Other(String),
}

impl From<String> for IconError {
    fn from(s: String) -> Self {
        IconError::Other(s)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Png,
    Ios,
    Macos,
    Linux,
    Windows,
    Android,
    Ohos,
}

fn family_of_target(t: &str) -> Vec<Family> {
    match t {
        "ios-uikit" => vec![Family::Ios],
        "android-mdc" => vec![Family::Android],
        "harmony-arkui" => vec![Family::Ohos],
        "windows-xaml" | "windows-gtk" | "windows-qt" => vec![Family::Windows],
        "macos-appkit" | "macos-gtk" | "macos-qt" => vec![Family::Macos],
        "linux-gtk" | "linux-qt" => vec![Family::Linux],
        "web-dom" => vec![Family::Png],
        _ => vec![],
    }
}

pub fn run(project: &Project, opts: &IconOptions) -> Result<usize, IconError> {
    let master = discover(project, opts.master.as_deref())?;
    let families: Vec<Family> = if opts.platforms.is_empty() {
        vec![
            Family::Png,
            Family::Ios,
            Family::Macos,
            Family::Linux,
            Family::Windows,
            Family::Android,
            Family::Ohos,
        ]
    } else {
        let mut fams: Vec<Family> = opts
            .platforms
            .iter()
            .flat_map(|p| family_of_target(p))
            .collect();
        fams.push(Family::Png); // the shared exports underpin every family
        fams.dedup();
        fams
    };

    status(
        if opts.check { "Checking" } else { "Rendering" },
        &format!("app icons from {}", master.display()),
    );
    let outputs = generate(project, &master, &families)?;

    if opts.check {
        let mut drift = Vec::new();
        for (rel, bytes) in &outputs {
            match std::fs::read(project.root.join(rel)) {
                Ok(on_disk) if &on_disk == bytes => {}
                Ok(_) => drift.push(format!("{rel}: differs from the master's render")),
                Err(_) => drift.push(format!("{rel}: missing (never generated?)")),
            }
        }
        // Same-version guard: a lock from another generator makes byte comparison unfair.
        if let Ok(lock) = std::fs::read_to_string(project.root.join(LOCK))
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(&lock)
            && let Some(generator) = v.get("generator").and_then(|g| g.as_str())
            && generator != generator_id()
        {
            drift.push(format!(
                "icons.lock.json was written by {generator}; this is {} — regenerate with `day icon`",
                generator_id()
            ));
        }
        if drift.is_empty() {
            status("Verified", &format!("{} icon outputs match", outputs.len()));
            Ok(outputs.len())
        } else {
            Err(IconError::Drift(drift))
        }
    } else {
        let mut lock_outputs = BTreeMap::new();
        for (rel, bytes) in &outputs {
            let path = project.root.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
            }
            std::fs::write(&path, bytes).map_err(|e| format!("write {rel}: {e}"))?;
            lock_outputs.insert(rel.clone(), sha256_hex(bytes));
        }
        let master_bytes = std::fs::read(&master).map_err(|e| e.to_string())?;
        let lock = serde_json::json!({
            "generator": generator_id(),
            "master": {
                "path": master.strip_prefix(&project.root).unwrap_or(&master).to_string_lossy(),
                "sha256": sha256_hex(&master_bytes),
            },
            "outputs": lock_outputs,
        });
        std::fs::write(
            project.root.join(LOCK),
            serde_json::to_string_pretty(&lock).unwrap_or_default() + "\n",
        )
        .map_err(|e| format!("write lock: {e}"))?;
        status(
            "Generated",
            &format!("{} icon outputs + lock", outputs.len()),
        );
        Ok(outputs.len())
    }
}

fn generator_id() -> String {
    format!(
        "day-cli {} ({})",
        env!("CARGO_PKG_VERSION"),
        day_vector::ENGINE
    )
}

fn discover(project: &Project, explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        let p = if p.is_absolute() {
            p.to_path_buf()
        } else {
            project.root.join(p)
        };
        return if p.exists() {
            Ok(p)
        } else {
            Err(format!("master {} does not exist", p.display()))
        };
    }
    for candidate in [
        "resource/icons/icon.svg",
        "resource/icons/day-icon.svg",
        "resource/icons/icon.png",
    ] {
        let p = project.root.join(candidate);
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(
        "no icon master — add resource/icons/icon.svg (layered via day: group ids, see \
         docs/icons.md), or icon.png for a raster-only set"
            .into(),
    )
}

// ---------------------------------------------------------------------------
// Generation: master → Vec<(project-relative path, bytes)>
// ---------------------------------------------------------------------------

fn generate(
    project: &Project,
    master: &Path,
    families: &[Family],
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let is_svg = master
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    let art: Art = if is_svg {
        let text = std::fs::read_to_string(master).map_err(|e| e.to_string())?;
        if text.contains("<text") {
            return Err(
                "the master contains <text> — outline text in your editor (text shaping is \
                 deliberately not compiled into day; see docs/icons.md)"
                    .into(),
            );
        }
        Art::from_svg(&text)?
    } else {
        let bytes = std::fs::read(master).map_err(|e| e.to_string())?;
        Art::from_png(&bytes)?
    };

    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let has = |f: Family| families.contains(&f);

    if has(Family::Png) {
        for px in [16u32, 32, 64, 128, 256, 512, 1024] {
            out.push((
                format!("resource/icons/png/day-icon-{px}.png"),
                art.composite(px)?,
            ));
        }
    }
    if has(Family::Linux) {
        for px in [48u32, 128, 256, 512] {
            out.push((
                format!("resource/icons/linux/day-icon-{px}.png"),
                art.composite(px)?,
            ));
        }
    }
    if has(Family::Windows) {
        out.push((
            "resource/icons/windows/day-icon-256.png".into(),
            art.composite(256)?,
        ));
        let entries: Vec<(u32, Vec<u8>)> = [16u32, 32, 48, 256]
            .iter()
            .map(|&px| Ok((px, art.composite(px)?)))
            .collect::<Result<_, String>>()?;
        out.push((
            "resource/icons/windows/day.ico".into(),
            day_vector::pack_ico(&entries),
        ));
    }
    if has(Family::Macos) {
        let mut icns_entries = Vec::new();
        for px in [16u32, 32, 64, 128, 256, 512, 1024] {
            let png = art.squircle(px)?;
            if px != 64 {
                // 64 is rendered only for the icns ladder — the export set never carried it.
                out.push((
                    format!("resource/icons/macos/day-icon-macos-{px}.png"),
                    png.clone(),
                ));
            }
            icns_entries.push((px, png));
        }
        out.push((
            "resource/icons/macos/day-icon.icns".into(),
            day_vector::pack_icns(&icns_entries)?,
        ));
    }
    if has(Family::Ios) {
        let flat = art.flat_composite(1024)?;
        out.push(("resource/icons/ios/AppIcon-1024.png".into(), flat.clone()));
        let appiconset = "platform/ios/Assets.xcassets/AppIcon.appiconset";
        if project.root.join(appiconset).is_dir() {
            out.push((format!("{appiconset}/AppIcon-1024.png"), flat));
        }
        // Icon Composer package (Xcode 26 Liquid Glass, docs/icons.md): SVG layers + icon.json.
        // Emitted for layered SVG masters; open in Icon Composer to tune materials, and point
        // Xcode 26's app-icon build setting at it (the appiconset stays the pre-26 fallback).
        if let Some(files) = art.icon_composer_package()? {
            for (name, bytes) in &files {
                out.push((
                    format!("resource/icons/ios/AppIcon.icon/{name}"),
                    bytes.clone(),
                ));
            }
            if project.root.join("platform/ios").is_dir() {
                for (name, bytes) in files {
                    out.push((format!("platform/ios/AppIcon.icon/{name}"), bytes));
                }
            }
        }
    }
    if has(Family::Android) {
        let fg = art.adaptive_foreground()?;
        let bg = art.adaptive_background()?;
        let legacy = art.flat_composite(192)?;
        let play = art.flat_composite(512)?;
        out.push((
            "resource/icons/android/ic_launcher_foreground.png".into(),
            fg.clone(),
        ));
        out.push((
            "resource/icons/android/ic_launcher_background.png".into(),
            bg.clone(),
        ));
        out.push((
            "resource/icons/android/ic_launcher-legacy-192.png".into(),
            legacy.clone(),
        ));
        out.push(("resource/icons/android/play-store-512.png".into(), play));
        let mipmap = "platform/android/app/src/main/res/mipmap-xxxhdpi";
        if project.root.join(mipmap).is_dir() {
            out.push((format!("{mipmap}/ic_launcher.png"), legacy));
            out.push((format!("{mipmap}/ic_launcher_foreground.png"), fg));
            out.push((format!("{mipmap}/ic_launcher_background.png"), bg));
        }
        // Themed icon (Android 13, docs/icons.md): a monochrome layer the system tints. A
        // `day:monochrome` layer becomes a VectorDrawable when it fits the subset; otherwise
        // (and for unlayered/raster masters) the adaptive foreground's alpha serves as the
        // mask, as a bitmap drawable. The committed adaptive XML gains `<monochrome>`.
        let mono = art.monochrome_drawable()?;
        let (rel, sync_rel) = match &mono {
            MonoDrawable::Vector(_) => (
                "resource/icons/android/ic_launcher_monochrome.xml".to_string(),
                "platform/android/app/src/main/res/drawable/ic_launcher_monochrome.xml".to_string(),
            ),
            MonoDrawable::Bitmap(_) => (
                "resource/icons/android/ic_launcher_monochrome.png".to_string(),
                "platform/android/app/src/main/res/drawable-xxxhdpi/ic_launcher_monochrome.png"
                    .to_string(),
            ),
        };
        let bytes = match mono {
            MonoDrawable::Vector(xml) => xml.into_bytes(),
            MonoDrawable::Bitmap(png) => png,
        };
        out.push((rel, bytes.clone()));
        let anydpi = "platform/android/app/src/main/res/mipmap-anydpi-v26/ic_launcher.xml";
        if project.root.join(anydpi).is_file() {
            out.push((sync_rel, bytes));
            let xml =
                std::fs::read_to_string(project.root.join(anydpi)).map_err(|e| e.to_string())?;
            if !xml.contains("<monochrome") {
                let with_mono = xml.replace(
                    "</adaptive-icon>",
                    "    <monochrome android:drawable=\"@drawable/ic_launcher_monochrome\" />\n</adaptive-icon>",
                );
                out.push((anydpi.to_string(), with_mono.into_bytes()));
            }
        }
    }
    if has(Family::Ohos) {
        let start = art.flat_composite(512)?;
        // Layered icon (docs/icons.md): fg/bg media + the layered_image.json descriptor, wired
        // into app.json5/module.json5's icon slots (startWindowIcon keeps the flat startIcon).
        let l_fg = art.ohos_foreground()?;
        let l_bg = art.ohos_background()?;
        const LAYERED: &str = "{\n  \"layered-image\": {\n    \"background\": \"$media:background\",\n    \"foreground\": \"$media:foreground\"\n  }\n}\n";
        // `platform/harmony`, or the pre-rename `platform/ohos` where that's what the project
        // has — these relative strings are the lock's output keys, so they follow the layout.
        let hroot = crate::ohos::harmony_dir(project);
        let hrel = hroot
            .strip_prefix(&project.root)
            .unwrap_or(&hroot)
            .to_string_lossy()
            .into_owned();
        for dir in [
            format!("{hrel}/entry/src/main/resources/base/media"),
            format!("{hrel}/AppScope/resources/base/media"),
        ] {
            if project.root.join(&dir).is_dir() {
                out.push((format!("{dir}/startIcon.png"), start.clone()));
                out.push((format!("{dir}/foreground.png"), l_fg.clone()));
                out.push((format!("{dir}/background.png"), l_bg.clone()));
                out.push((
                    format!("{dir}/layered_image.json"),
                    LAYERED.as_bytes().to_vec(),
                ));
            }
        }
        for manifest in [
            format!("{hrel}/AppScope/app.json5"),
            format!("{hrel}/entry/src/main/module.json5"),
        ] {
            let path = project.root.join(&manifest);
            if let Ok(text) = std::fs::read_to_string(&path) {
                let updated = text.replace(
                    "\"icon\": \"$media:startIcon\"",
                    "\"icon\": \"$media:layered_image\"",
                );
                if updated != text {
                    out.push((manifest, updated.into_bytes()));
                }
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// The master's renderable forms
// ---------------------------------------------------------------------------

/// `.icon` package members: (bundle-relative name, bytes).
type PackageFiles = Vec<(String, Vec<u8>)>;

/// A monochrome themed-icon drawable: VectorDrawable XML, or a bitmap alpha mask.
enum MonoDrawable {
    Vector(String),
    Bitmap(Vec<u8>),
}

/// A prepared master: SVG documents per role, or a decoded raster.
enum Art {
    Svg {
        /// Whole art minus the reserved layers — every full-bleed output.
        composite: String,
        /// Foreground-only document (layered masters), pre-tightened to its content box.
        foreground: Option<String>,
        /// Background-only document (layered masters).
        background: Option<String>,
        /// Monochrome-only document (`day:monochrome`) — Android themed icons, `.icon` Tinted.
        monochrome: Option<String>,
    },
    Raster(tiny_skia::Pixmap),
}

impl Art {
    fn from_svg(text: &str) -> Result<Art, String> {
        let layers = day_layers(text)?;
        let composite = splice_out(text, &[&layers.monochrome, &layers.dark]);
        // Sanity-parse now so errors carry the master's name, not an output's.
        day_vector::parse(composite.as_bytes())?;
        let (foreground, background) =
            if !layers.background.is_empty() || !layers.foreground.is_empty() {
                let fg = splice_out(
                    text,
                    &[&layers.background, &layers.monochrome, &layers.dark],
                );
                let bg_ranges: Vec<Range<usize>> = layers.foreground.clone();
                let bg = splice_out(text, &[&bg_ranges, &layers.monochrome, &layers.dark]);
                (Some(fg), Some(bg))
            } else {
                (None, None)
            };
        let monochrome = if layers.monochrome.is_empty() {
            None
        } else {
            let mut keep = vec![&layers.background, &layers.foreground, &layers.dark];
            let bg_fg_dark: Vec<Range<usize>> =
                keep.drain(..).flat_map(|v| v.iter().cloned()).collect();
            Some(unhide_layer(
                splice_out(text, &[&bg_fg_dark]),
                "day:monochrome",
            ))
        };
        Ok(Art::Svg {
            composite,
            foreground,
            background,
            monochrome,
        })
    }

    fn from_png(bytes: &[u8]) -> Result<Art, String> {
        let pm = tiny_skia::Pixmap::decode_png(bytes).map_err(|e| format!("png master: {e}"))?;
        if pm.width() < 1024 || pm.height() < 1024 {
            status(
                "Warning",
                &format!(
                    "png master is {}×{} — 1024×1024 or larger avoids upscaled large slots",
                    pm.width(),
                    pm.height()
                ),
            );
        }
        Ok(Art::Raster(pm))
    }

    /// Full-bleed square render (transparent background preserved).
    fn composite(&self, px: u32) -> Result<Vec<u8>, String> {
        match self {
            Art::Svg { composite, .. } => {
                let tree = day_vector::parse(composite.as_bytes())?;
                day_vector::render_png(&tree, px)
            }
            Art::Raster(pm) => scale_png(pm, px),
        }
    }

    /// Composite flattened over the derived background color (opaque — iOS/store slots).
    fn flat_composite(&self, px: u32) -> Result<Vec<u8>, String> {
        let png = self.composite(px)?;
        flatten(&png, self.backdrop()?)
    }

    /// The macOS shape: art inset 824/1024 with the 184-radius rounded clip, transparent margin.
    fn squircle(&self, px: u32) -> Result<Vec<u8>, String> {
        match self {
            Art::Svg { composite, .. } => {
                let vb = view_box(composite)?;
                let inner = inner_markup(composite)?;
                let doc = format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1024 1024\">\
                     <defs><clipPath id=\"day-squircle\"><rect x=\"{MAC_INSET}\" y=\"{MAC_INSET}\" width=\"{MAC_ART}\" height=\"{MAC_ART}\" rx=\"{MAC_RADIUS}\"/></clipPath></defs>\
                     <g clip-path=\"url(#day-squircle)\"><svg x=\"{MAC_INSET}\" y=\"{MAC_INSET}\" width=\"{MAC_ART}\" height=\"{MAC_ART}\" viewBox=\"{vb}\" preserveAspectRatio=\"xMidYMid slice\">{inner}</svg></g></svg>"
                );
                let tree = day_vector::parse(doc.as_bytes())?;
                day_vector::render_png(&tree, px)
            }
            Art::Raster(pm) => {
                let art_px = (px as f32 * MAC_ART / 1024.0).round() as u32;
                let inset = ((px as f32 - art_px as f32) / 2.0).round() as i32;
                let radius = px as f32 * MAC_RADIUS / 1024.0;
                let scaled = tiny_skia::Pixmap::decode_png(&scale_png(pm, art_px)?)
                    .map_err(|e| e.to_string())?;
                let mut canvas = tiny_skia::Pixmap::new(px, px).ok_or("pixmap")?;
                canvas.draw_pixmap(
                    inset,
                    inset,
                    scaled.as_ref(),
                    &tiny_skia::PixmapPaint::default(),
                    tiny_skia::Transform::identity(),
                    None,
                );
                apply_round_mask(&mut canvas, inset as f32, art_px as f32, radius);
                canvas.encode_png().map_err(|e| e.to_string())
            }
        }
    }

    /// The adaptive foreground: content tightened, centered in the 66/108 safe zone, transparent.
    fn adaptive_foreground(&self) -> Result<Vec<u8>, String> {
        let inset = (ADAPTIVE_PX as f32 - SAFE_PX) / 2.0;
        match self {
            Art::Svg {
                foreground: Some(fg),
                ..
            } => {
                let tree = day_vector::parse(fg.as_bytes())?;
                let b = day_vector::content_bbox(&tree)
                    .ok_or("the day:foreground layers render no content")?;
                let (bx, by, bw, bh) = bbox_in_viewbox_units(fg, &tree, b)?;
                let inner = inner_markup(fg)?;
                let doc = format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {ADAPTIVE_PX} {ADAPTIVE_PX}\">\
                     <svg x=\"{inset}\" y=\"{inset}\" width=\"{SAFE_PX}\" height=\"{SAFE_PX}\" viewBox=\"{bx} {by} {bw} {bh}\" preserveAspectRatio=\"xMidYMid meet\">{inner}</svg></svg>",
                );
                let tree = day_vector::parse(doc.as_bytes())?;
                day_vector::render_png(&tree, ADAPTIVE_PX)
            }
            // Unlayered/raster: the whole art in the safe zone (over the derived background).
            _ => {
                let art = tiny_skia::Pixmap::decode_png(&self.composite(SAFE_PX as u32)?)
                    .map_err(|e| e.to_string())?;
                let mut canvas =
                    tiny_skia::Pixmap::new(ADAPTIVE_PX, ADAPTIVE_PX).ok_or("pixmap")?;
                canvas.draw_pixmap(
                    inset as i32,
                    inset as i32,
                    art.as_ref(),
                    &tiny_skia::PixmapPaint::default(),
                    tiny_skia::Transform::identity(),
                    None,
                );
                canvas.encode_png().map_err(|e| e.to_string())
            }
        }
    }

    /// The adaptive background: the day:background layer full-bleed, else the derived color.
    fn adaptive_background(&self) -> Result<Vec<u8>, String> {
        if let Art::Svg {
            background: Some(bg),
            ..
        } = self
        {
            let vb = view_box(bg)?;
            let inner = inner_markup(bg)?;
            let doc = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {ADAPTIVE_PX} {ADAPTIVE_PX}\">\
                 <svg width=\"{ADAPTIVE_PX}\" height=\"{ADAPTIVE_PX}\" viewBox=\"{vb}\" preserveAspectRatio=\"xMidYMid slice\">{inner}</svg></svg>"
            );
            let tree = day_vector::parse(doc.as_bytes())?;
            return day_vector::render_png(&tree, ADAPTIVE_PX);
        }
        let mut pm = tiny_skia::Pixmap::new(ADAPTIVE_PX, ADAPTIVE_PX).ok_or("pixmap")?;
        let c = self.backdrop()?;
        pm.fill(c);
        pm.encode_png().map_err(|e| e.to_string())
    }

    /// The Android themed-icon monochrome drawable: the `day:monochrome` layer as a
    /// VectorDrawable when it fits the subset, else (and without the layer) the adaptive
    /// foreground's alpha as a bitmap mask — the system tints either.
    fn monochrome_drawable(&self) -> Result<MonoDrawable, String> {
        if let Art::Svg {
            monochrome: Some(mono),
            ..
        } = self
        {
            let tree = day_vector::parse(mono.as_bytes())?;
            if let Some(b) = day_vector::content_bbox(&tree) {
                let (bx, by, bw, bh) = bbox_in_viewbox_units(mono, &tree, b)?;
                let inset = (ADAPTIVE_PX as f32 - SAFE_PX) / 2.0;
                let inner = inner_markup(mono)?;
                // Safe-zone fit as an EXPLICIT transform, not a nested <svg> viewport: usvg
                // models a nested svg as a clipped group, which is outside the
                // VectorDrawable subset — the very conversion this document exists for. The
                // math is `xMidYMid meet` by hand: uniform scale to the safe square,
                // centered, then offset by the zone inset.
                let s = SAFE_PX / bw.max(bh).max(1e-3);
                let tx = inset + (SAFE_PX - s * bw) / 2.0 - s * bx;
                let ty = inset + (SAFE_PX - s * bh) / 2.0 - s * by;
                let doc = format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {ADAPTIVE_PX} {ADAPTIVE_PX}\">\
                     <g transform=\"translate({tx} {ty}) scale({s})\">{inner}</g></svg>"
                );
                let boxed = day_vector::parse(doc.as_bytes())?;
                match day_vector::to_vector_drawable(&boxed) {
                    Ok(xml) => return Ok(MonoDrawable::Vector(xml)),
                    Err(why) => status(
                        "Warning",
                        &format!(
                            "day:monochrome → bitmap mask ({why} is outside the VectorDrawable subset)"
                        ),
                    ),
                }
                return Ok(MonoDrawable::Bitmap(day_vector::render_png(
                    &boxed,
                    ADAPTIVE_PX,
                )?));
            }
        }
        Ok(MonoDrawable::Bitmap(self.adaptive_foreground()?))
    }

    /// The HarmonyOS layered-icon foreground: the motif centered in a safe zone on a 216 canvas.
    fn ohos_foreground(&self) -> Result<Vec<u8>, String> {
        // Reuse the Android adaptive geometry, downscaled to the OHOS 216 canvas.
        let png = self.adaptive_foreground()?;
        let pm = tiny_skia::Pixmap::decode_png(&png).map_err(|e| e.to_string())?;
        scale_png(&pm, 216)
    }

    /// The HarmonyOS layered-icon background: full-bleed at 216.
    fn ohos_background(&self) -> Result<Vec<u8>, String> {
        let png = self.adaptive_background()?;
        let pm = tiny_skia::Pixmap::decode_png(&png).map_err(|e| e.to_string())?;
        scale_png(&pm, 216)
    }

    /// The Icon Composer `.icon` package files (Xcode 26): SVG layer assets + `icon.json`.
    /// Only for layered SVG masters — the package's value IS the layer split; a flat master
    /// has nothing to feed the Liquid Glass modes.
    fn icon_composer_package(&self) -> Result<Option<PackageFiles>, String> {
        let Art::Svg {
            foreground: Some(fg),
            background,
            monochrome,
            ..
        } = self
        else {
            return Ok(None);
        };
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        // Layer assets, top-first in the group below.
        files.push(("Assets/foreground.svg".into(), fg.clone().into_bytes()));
        let mut layers = vec![serde_json::json!({
            "image-name": "foreground.svg",
            "name": "foreground",
        })];
        if let Some(bg) = background {
            files.push(("Assets/background.svg".into(), bg.clone().into_bytes()));
            layers.push(serde_json::json!({
                "image-name": "background.svg",
                "name": "background",
            }));
        }
        if let Some(mono) = monochrome {
            // Not referenced by the default group: Icon Composer assigns it to the Tinted
            // appearance when the designer opts in — shipping the asset makes that a drag.
            files.push(("Assets/monochrome.svg".into(), mono.clone().into_bytes()));
        }
        let json = serde_json::json!({
            "fill": "automatic",
            "groups": [ { "layers": layers } ],
            "supported-platforms": { "squares": "shared" },
        });
        files.push((
            "icon.json".into(),
            (serde_json::to_string_pretty(&json).unwrap_or_default() + "\n").into_bytes(),
        ));
        Ok(Some(files))
    }

    /// The flattening/back-plate color: the composite's corner pixel (a full-bleed master's own
    /// background), or white when the corner is transparent.
    fn backdrop(&self) -> Result<tiny_skia::Color, String> {
        let pm = tiny_skia::Pixmap::decode_png(&self.composite(64)?).map_err(|e| e.to_string())?;
        let px = pm.pixel(1, 1).ok_or("empty pixmap")?;
        if px.alpha() == 255 {
            Ok(tiny_skia::Color::from_rgba8(
                px.red(),
                px.green(),
                px.blue(),
                255,
            ))
        } else {
            Ok(tiny_skia::Color::WHITE)
        }
    }
}

// ---------------------------------------------------------------------------
// SVG text helpers (layer slicing is textual — see the module docs)
// ---------------------------------------------------------------------------

struct Layers {
    background: Vec<Range<usize>>,
    foreground: Vec<Range<usize>>,
    monochrome: Vec<Range<usize>>,
    dark: Vec<Range<usize>>,
}

fn day_layers(xml: &str) -> Result<Layers, String> {
    let doc = day_vector::roxmltree::Document::parse(xml).map_err(|e| format!("master: {e}"))?;
    let mut layers = Layers {
        background: Vec::new(),
        foreground: Vec::new(),
        monochrome: Vec::new(),
        dark: Vec::new(),
    };
    for child in doc.root_element().children() {
        let Some(id) = child.attribute("id") else {
            continue;
        };
        match id {
            "day:background" => layers.background.push(child.range()),
            "day:monochrome" => layers.monochrome.push(child.range()),
            "day:dark" => layers.dark.push(child.range()),
            _ if id.starts_with("day:foreground") => layers.foreground.push(child.range()),
            _ => {}
        }
    }
    Ok(layers)
}

/// The document with the given ranges removed (spliced back-to-front so offsets stay valid).
fn splice_out(xml: &str, removals: &[&Vec<Range<usize>>]) -> String {
    let mut ranges: Vec<Range<usize>> = removals.iter().flat_map(|v| v.iter().cloned()).collect();
    ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
    let mut out = xml.to_string();
    for r in ranges {
        out.replace_range(r, "");
    }
    out
}

/// Drop a `display="none"` from the named layer's OPEN TAG. A master may hide a reserved
/// layer (`day:monochrome`, `day:dark`) so plain SVG viewers show the icon as shipped — the
/// generated masters do — and the layer-only documents re-enable it here.
fn unhide_layer(doc: String, id: &str) -> String {
    let Some(at) = doc.find(&format!("id=\"{id}\"")) else {
        return doc;
    };
    let Some(end) = doc[at..].find('>').map(|e| at + e) else {
        return doc;
    };
    match doc[at..end].find(" display=\"none\"") {
        Some(rel) => {
            let mut out = doc;
            out.replace_range(at + rel..at + rel + " display=\"none\"".len(), "");
            out
        }
        None => doc,
    }
}

/// The root `<svg>`'s viewBox, or one derived from width/height.
/// A usvg content box mapped back into the document's own viewBox units.
///
/// usvg normalizes a parsed tree to the svg's width/height, so when a master declares e.g.
/// `viewBox="0 0 120 120" width="1024"`, [`day_vector::content_bbox`] answers in 1024-space —
/// while the raw inner markup the safe-zone wrappers re-parse is still in 120-space. Windowing
/// the markup with unconverted bounds selects a region outside the art entirely (an empty
/// adaptive foreground). Identity when the viewBox and the tree size already agree.
fn bbox_in_viewbox_units(
    doc: &str,
    tree: &day_vector::usvg::Tree,
    b: day_vector::usvg::Rect,
) -> Result<(f32, f32, f32, f32), String> {
    let vb = view_box(doc)?;
    let parts: Vec<f32> = vb
        .split_whitespace()
        .filter_map(|p| p.parse::<f32>().ok())
        .collect();
    let [vx, vy, vw, vh] = parts.as_slice() else {
        return Err(format!("unparseable viewBox {vb:?}"));
    };
    let size = tree.size();
    let sx = vw / size.width().max(1e-6);
    let sy = vh / size.height().max(1e-6);
    Ok((
        vx + b.x() * sx,
        vy + b.y() * sy,
        b.width() * sx,
        b.height() * sy,
    ))
}

fn view_box(xml: &str) -> Result<String, String> {
    let doc = day_vector::roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if let Some(vb) = root.attribute("viewBox") {
        return Ok(vb.to_string());
    }
    let dim = |a: &str| {
        root.attribute(a)
            .and_then(|s| s.trim_end_matches("px").parse::<f32>().ok())
    };
    match (dim("width"), dim("height")) {
        (Some(w), Some(h)) => Ok(format!("0 0 {w} {h}")),
        _ => Err("the master SVG has neither viewBox nor width/height".into()),
    }
}

/// Everything between the root `<svg …>` tag and its `</svg>` close.
fn inner_markup(xml: &str) -> Result<&str, String> {
    let open_end = xml
        .find("<svg")
        .and_then(|at| xml[at..].find('>').map(|o| at + o + 1))
        .ok_or("no <svg> root")?;
    let close = xml.rfind("</svg>").ok_or("no </svg>")?;
    Ok(&xml[open_end..close])
}

// ---------------------------------------------------------------------------
// Raster helpers
// ---------------------------------------------------------------------------

fn scale_png(pm: &tiny_skia::Pixmap, px: u32) -> Result<Vec<u8>, String> {
    let mut out = tiny_skia::Pixmap::new(px, px).ok_or("pixmap")?;
    let scale = px as f32 / pm.width().max(pm.height()) as f32;
    let paint = tiny_skia::PixmapPaint {
        quality: tiny_skia::FilterQuality::Bilinear,
        ..Default::default()
    };
    out.draw_pixmap(
        0,
        0,
        pm.as_ref(),
        &paint,
        tiny_skia::Transform::from_scale(scale, scale),
        None,
    );
    out.encode_png().map_err(|e| e.to_string())
}

fn flatten(png: &[u8], color: tiny_skia::Color) -> Result<Vec<u8>, String> {
    let img = tiny_skia::Pixmap::decode_png(png).map_err(|e| e.to_string())?;
    let mut base = tiny_skia::Pixmap::new(img.width(), img.height()).ok_or("pixmap")?;
    base.fill(color);
    base.draw_pixmap(
        0,
        0,
        img.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        tiny_skia::Transform::identity(),
        None,
    );
    base.encode_png().map_err(|e| e.to_string())
}

/// Intersect the canvas alpha with a rounded rect (the raster-master squircle mask).
fn apply_round_mask(canvas: &mut tiny_skia::Pixmap, inset: f32, edge: f32, radius: f32) {
    let mut mask = match tiny_skia::Pixmap::new(canvas.width(), canvas.height()) {
        Some(m) => m,
        None => return,
    };
    let mut pb = tiny_skia::PathBuilder::new();
    // Rounded rect from four lines + four cubic corners (kappa circle approximation).
    let k = 0.5523 * radius;
    let (x0, y0, x1, y1) = (inset, inset, inset + edge, inset + edge);
    pb.move_to(x0 + radius, y0);
    pb.line_to(x1 - radius, y0);
    pb.cubic_to(x1 - radius + k, y0, x1, y0 + radius - k, x1, y0 + radius);
    pb.line_to(x1, y1 - radius);
    pb.cubic_to(x1, y1 - radius + k, x1 - radius + k, y1, x1 - radius, y1);
    pb.line_to(x0 + radius, y1);
    pb.cubic_to(x0 + radius - k, y1, x0, y1 - radius + k, x0, y1 - radius);
    pb.line_to(x0, y0 + radius);
    pb.cubic_to(x0, y0 + radius - k, x0 + radius - k, y0, x0 + radius, y0);
    pb.close();
    let Some(path) = pb.finish() else { return };
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(tiny_skia::Color::WHITE);
    paint.anti_alias = true;
    mask.fill_path(
        &path,
        &paint,
        tiny_skia::FillRule::Winding,
        tiny_skia::Transform::identity(),
        None,
    );
    // canvas.alpha *= mask.alpha, per pixel.
    let mask_px: Vec<u8> = mask.pixels().iter().map(|p| p.alpha()).collect();
    for (px, m) in canvas.pixels_mut().iter_mut().zip(mask_px) {
        let a = (px.alpha() as u16 * m as u16 / 255) as u8;
        let scale = if px.alpha() == 0 {
            0.0
        } else {
            a as f32 / px.alpha() as f32
        };
        *px = tiny_skia::PremultipliedColorU8::from_rgba(
            (px.red() as f32 * scale) as u8,
            (px.green() as f32 * scale) as u8,
            (px.blue() as f32 * scale) as u8,
            a,
        )
        .unwrap_or(*px);
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYERED: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\">\
        <defs><linearGradient id=\"g\"/></defs>\
        <rect id=\"day:background\" width=\"100\" height=\"100\" fill=\"#123456\"/>\
        <g id=\"day:foreground\"><circle cx=\"50\" cy=\"50\" r=\"20\" fill=\"#fff\"/></g>\
        <g id=\"day:monochrome\"><circle cx=\"50\" cy=\"50\" r=\"20\"/></g></svg>";

    #[test]
    fn layers_split_and_splice() {
        let l = day_layers(LAYERED).unwrap();
        assert_eq!(l.background.len(), 1);
        assert_eq!(l.foreground.len(), 1);
        assert_eq!(l.monochrome.len(), 1);
        // Composite drops the monochrome layer but keeps bg+fg+defs.
        let composite = splice_out(LAYERED, &[&l.monochrome, &l.dark]);
        assert!(composite.contains("day:background"));
        assert!(composite.contains("day:foreground"));
        assert!(!composite.contains("day:monochrome"));
        day_vector::parse(composite.as_bytes()).unwrap();
        // Foreground-only drops the background.
        let fg = splice_out(LAYERED, &[&l.background, &l.monochrome, &l.dark]);
        assert!(!fg.contains("day:background"));
        assert!(fg.contains("day:foreground"));
    }

    #[test]
    fn adaptive_foreground_survives_viewbox_size_mismatch() {
        // A master may declare `viewBox="0 0 120 120" width="1024"`. usvg reports content
        // bounds in 1024-space while the raw markup the safe-zone wrapper re-parses is in
        // 120-space; unconverted bounds window a region outside the art and the adaptive
        // foreground renders EMPTY (the Day-Showcase sunrise master, 2026-08-07).
        let master = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 120 120\" \
             width=\"1024\" height=\"1024\">\
             <rect id=\"day:background\" width=\"120\" height=\"120\" fill=\"#123456\"/>\
             <g id=\"day:foreground\"><circle cx=\"60\" cy=\"60\" r=\"30\" fill=\"#fff\"/></g>\
             </svg>";
        let art = Art::from_svg(master).unwrap();
        let png = art.adaptive_foreground().unwrap();
        let pm = tiny_skia::Pixmap::decode_png(&png).unwrap();
        let visible = pm.pixels().iter().filter(|p| p.alpha() > 0).count();
        assert!(
            visible > 1000,
            "adaptive foreground is (nearly) empty: {visible} visible px"
        );
    }

    #[test]
    fn view_box_falls_back_to_width_height() {
        assert_eq!(view_box(LAYERED).unwrap(), "0 0 100 100");
        let wh = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64px\" height=\"32\"></svg>";
        assert_eq!(view_box(wh).unwrap(), "0 0 64 32");
    }

    #[test]
    fn generated_families_cover_the_legacy_set() {
        // A pure-function sanity: family mapping is total over the shipping targets.
        for t in [
            "ios-uikit",
            "android-mdc",
            "harmony-arkui",
            "windows-xaml",
            "macos-appkit",
            "linux-gtk",
            "linux-qt",
            "web-dom",
        ] {
            assert!(!family_of_target(t).is_empty(), "{t} maps to no family");
        }
    }
}
