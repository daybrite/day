//! Build-time resource staging (DESIGN §18.3).
//!
//! Two declared buckets in a project:
//!   * `images/` — processed images, routed into each platform's native image pipeline so
//!     `image("name")` resolves by name (SwiftPM `.process` → `Assets.car`, Android `res/drawable`
//!     → `R`, GResource, `.qrc`, ArkUI rawfile, …). We never touch the pixels ourselves; the native
//!     build system optionally optimizes.
//!   * `assets/` — arbitrary raw data, staged **uncompressed** into each platform's native data
//!     store so `day::resource("name")` hands back a zero-copy random-access view (Apple bundle
//!     file, Android `AAssetManager`, GTK GResource, Qt QResource, ArkUI rawfile).
//!
//! `stage()` runs before the platform build and dispatches to the per-toolkit stager.

use std::path::PathBuf;

use crate::meta::Project;
use crate::targets::Target;

mod android;
pub mod apple; // write_media_xcassets is called from pieces::write_ios_pieces
mod arkui;
pub mod gtk; // gresource_path is read by ops::launch
pub mod qt; // qresource_path is read by ops::launch
mod winui;

/// A single declared resource file: its lookup `name` and on-disk source `path`.
// Fields are consumed by the per-toolkit stagers (some still landing).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ResourceFile {
    /// Lookup name. For images this is the file stem (no extension, `@2x`/`@3x` stripped); for data
    /// it is the full file name (e.g. `stations.json`).
    pub name: String,
    /// The source file on disk under the project.
    pub path: PathBuf,
    /// HiDPI scale parsed from an `@Nx` suffix (images only); `1` when absent.
    pub scale: u32,
}

/// Everything a project declares to bundle.
#[derive(Debug, Default, Clone)]
pub struct ResourceSet {
    /// Files under `images/` — routed to the native image pipeline.
    pub images: Vec<ResourceFile>,
    /// Files under `assets/` — routed to the native uncompressed data store.
    pub data: Vec<ResourceFile>,
}

impl ResourceSet {
    /// Scan a project's `images/` and `assets/` directories.
    pub fn scan(project: &Project) -> ResourceSet {
        ResourceSet {
            images: scan_dir(&project.root.join("resource/images"), true),
            data: scan_dir(&project.root.join("resource/assets"), false),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.images.is_empty() && self.data.is_empty()
    }
}

/// Collect top-level files under `dir`. When `image`, the lookup name is the file stem with any
/// `@Nx` HiDPI suffix parsed off; otherwise the name is the full file name.
fn scan_dir(dir: &std::path::Path, image: bool) -> Vec<ResourceFile> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_file() {
            continue;
        }
        let fname = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if fname.starts_with('.') {
            continue;
        }
        let (name, scale) = if image {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&fname)
                .to_string();
            parse_scale(&stem)
        } else {
            (fname.clone(), 1)
        };
        out.push(ResourceFile { name, path, scale });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.scale.cmp(&b.scale)));
    out
}

/// Split a `foo@2x` stem into (`"foo"`, 2); a bare `foo` yields (`"foo"`, 1).
fn parse_scale(stem: &str) -> (String, u32) {
    if let Some((base, tail)) = stem.rsplit_once('@')
        && let Some(digits) = tail.strip_suffix('x')
        && let Ok(scale) = digits.parse::<u32>()
        && scale >= 1
    {
        return (base.to_string(), scale);
    }
    (stem.to_string(), 1)
}

/// A bundled font file (§18.4): its source path, the family name parsed from the font's `name`
/// table (what `Font::Custom` matches on), and the Android/ArkUI resource identifier derived
/// from that family (the same rule the runtimes re-derive — `day_fonts::font_ident`).
#[derive(Debug, Clone)]
pub struct FontFile {
    pub path: PathBuf,
    pub family: String,
    pub ident: String,
}

impl FontFile {
    /// The staged file name on identifier-based platforms: `<ident>.<ext>`.
    pub fn staged_name(&self) -> String {
        let ext = self
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("ttf")
            .to_ascii_lowercase();
        format!("{}.{ext}", self.ident)
    }
}

/// Scan and validate the project's `fonts/` directory (§18.4). Every problem is a hard error —
/// each would otherwise surface only at runtime on some platform: a non-`.ttf`/`.otf` file
/// (Android font resources accept nothing else), an unparseable font (no family name to resolve
/// by), or two families that collide after identifier sanitization (they'd overwrite each other
/// in `res/font/`).
pub fn scan_fonts(project: &Project) -> Result<Vec<FontFile>, String> {
    let dir = project.root.join("resource/fonts");
    let mut out: Vec<FontFile> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(out);
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .starts_with('.')
        })
        .collect();
    files.sort();
    for path in files {
        let fname = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if !matches!(ext.as_str(), "ttf" | "otf") {
            return Err(format!(
                "fonts/{fname}: only .ttf and .otf files can be bundled (Android's res/font/ \
                 accepts nothing else — convert collections/other formats to single faces)"
            ));
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("fonts/{fname}: {e}"))?;
        let names = day_fonts::parse_font_names(&bytes).ok_or_else(|| {
            format!("fonts/{fname}: not a recognizable font file (no readable name table)")
        })?;
        let ident = day_fonts::font_ident(&names.family);
        if let Some(prev) = out.iter().find(|f| f.ident == ident) {
            return Err(format!(
                "fonts/{fname}: family {:?} collides with {}'s family {:?} on the sanitized \
                 resource name `{ident}` — bundle one face per family, or rename a family",
                names.family,
                prev.path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?"),
                prev.family,
            ));
        }
        out.push(FontFile {
            path,
            family: names.family,
            ident,
        });
    }
    Ok(out)
}

/// Sanitize a name to the strictest platform identifier rules (Android `R` / ArkUI): lowercase, and
/// only `[a-z0-9_]`, leading letter. Re-exported from `day-build` — the single source of truth — so
/// the identifier a stager writes into a backend's native store is exactly the one the generated
/// `res::…` constants (produced by the same crate) resolve by (§18.5). Used by the android/arkui
/// stagers that need identifier-safe names.
pub use day_build::sanitize_ident;

/// Resolve the platform-appropriate app icon from the project's `icons/` directory (§18.2): the
/// LARGEST file of the wanted type in the first candidate subdirectory that has one. The convention
/// matches a per-platform icon export set — `icons/{macos,linux,windows,png}/…` — falling back to
/// any icon at the `icons/` root.
pub fn app_icon(project: &Project, toolkit: &'static str) -> Option<PathBuf> {
    let icons = project.root.join("resource/icons");
    // Windows taskbar icons are .ico; everything else takes a PNG (dock, icon theme, dialogs).
    let (subdirs, ext): (&[&str], &str) = match toolkit {
        "winui" => (&["windows", ""], "ico"),
        _ if cfg!(target_os = "macos") => (&["macos", "png", ""], "png"),
        _ => (&["linux", "png", ""], "png"),
    };
    for sub in subdirs {
        let dir = if sub.is_empty() {
            icons.clone()
        } else {
            icons.join(sub)
        };
        let mut best: Option<(u64, PathBuf)> = None;
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some(ext) {
                continue;
            }
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            if best.as_ref().is_none_or(|(s, _)| size > *s) {
                best = Some((size, p));
            }
        }
        if let Some((_, p)) = best {
            return Some(p);
        }
    }
    None
}

/// The built-in fallback icon (the Day logo) at the appstream-compose icon-policy sizes, for
/// packagers whose format REQUIRES an icon when the project ships none: flatpak's appstream
/// catalog and the MSIX logo slots. The sizes are load-bearing — compose only probes its policy
/// sizes (48/64/128) plus the standard upscale candidates, so e.g. a lone 192×192 icon fails
/// `appstreamcli compose` with `icon-not-found` (verified against appstream 1.0.2 on
/// ubuntu-24.04, the flatpak-builder CI environment).
pub const DEFAULT_ICONS: [(u32, &[u8]); 3] = [
    (48, include_bytes!("../../resources/icons/day-icon-48.png")),
    (64, include_bytes!("../../resources/icons/day-icon-64.png")),
    (
        128,
        include_bytes!("../../resources/icons/day-icon-128.png"),
    ),
];

/// Stage a project's declared resources into the native locations for `target`, before its platform
/// build runs. Desktop toolkits (appkit/gtk/qt on a cargo binary) load data via the mmap file opener
/// and images via the bundle file, so they need no pre-build staging here (handled at pack/launch).
pub fn stage(project: &Project, target: &Target) -> Result<(), String> {
    let set = ResourceSet::scan(project);
    let fonts = scan_fonts(project)?;
    if set.is_empty() && fonts.is_empty() {
        return Ok(());
    }
    match target.toolkit {
        // iOS images are staged into the DayPieces `.process` catalog by pieces::write_ios_pieces
        // (during build_ios), fonts as its `.copy("fonts")` bundle dir + the app's UIAppFonts;
        // data rides the existing bundle copy phase + default file opener.
        "uikit" => Ok(()),
        "widget" => android::stage(project, &set, &fonts),
        "arkui" => arkui::stage(project, &set, &fonts),
        // Desktop toolkits load fonts as loose files: DAY_FONT_ROOT under `day launch`, a
        // `fonts/` dir next to the binary / in Resources when packed (§18.4).
        "gtk" => gtk::stage(project, &set),
        "qt" => qt::stage(project, &set),
        "winui" => winui::stage(project, &set),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_ICONS;

    /// Guards the embedded files: each entry must be a real PNG whose IHDR pixel size matches
    /// its declared hicolor size (a mismatched size directory breaks the icon-theme lookup).
    #[test]
    fn default_icons_are_pngs_at_their_declared_sizes() {
        for (size, bytes) in DEFAULT_ICONS {
            assert!(
                bytes.starts_with(&[0x89, b'P', b'N', b'G']),
                "{size}: not a PNG"
            );
            // IHDR: width at bytes 16..20, height at 20..24, big-endian.
            let dim = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
            assert_eq!(dim(16), size, "{size}: IHDR width mismatch");
            assert_eq!(dim(20), size, "{size}: IHDR height mismatch");
        }
    }
}
