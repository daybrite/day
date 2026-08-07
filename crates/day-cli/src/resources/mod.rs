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
use crate::ops::status;
use crate::targets::Target;

mod android;
pub mod apple; // write_media_xcassets is called from pieces::write_ios_pieces
mod arkui;
pub mod gtk; // gresource_path is read by ops::launch
pub mod qt; // qresource_path is read by ops::launch
mod xaml;

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
    /// Scan a project's `images/` and `assets/` directories — plus `toolkit`'s vector raster
    /// FALLBACKS (docs/vectors.md), appended as ordinary images so the stager ships them through
    /// its native image pipeline under the same name. On a toolkit that draws vectors this adds
    /// only the glyphs its vector pipeline could not express, which on most projects is none;
    /// on gtk/qt, which have no vector arm, it is every glyph.
    pub fn scan(project: &Project, toolkit: &str) -> ResourceSet {
        let mut images = scan_dir(&project.root.join("resource/images"), true);
        images.extend(scan_dir(&vector_fallback_dir(project, toolkit), true));
        ResourceSet {
            images,
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
        "xaml" => (&["windows", ""], "ico"),
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
    // Vectors first (docs/vectors.md): every glyph gets its raster-cache PNG, then the cache is
    // filtered to what THIS toolkit actually needs one for — which `ResourceSet::scan` below
    // picks up for its image pipeline, and which `day launch`/`day pack` ship.
    let vectors = prepare_vectors(project)?;
    write_vector_fallbacks(project, target.toolkit, &vectors)?;
    let set = ResourceSet::scan(project, target.toolkit);
    let fonts = scan_fonts(project)?;
    if set.is_empty() && fonts.is_empty() {
        return Ok(());
    }
    match target.toolkit {
        // iOS images are staged into the DayPieces `.process` catalog by pieces::write_ios_pieces
        // (during build_ios), fonts as its `.copy("fonts")` bundle dir + the app's UIAppFonts;
        // data rides the existing bundle copy phase + default file opener.
        "uikit" => Ok(()),
        "mdc" => android::stage(project, &set, &fonts, &vectors),
        "arkui" => arkui::stage(project, &set, &fonts),
        // Desktop toolkits load fonts as loose files: DAY_FONT_ROOT under `day launch`, a
        // `fonts/` dir next to the binary / in Resources when packed (§18.4).
        "gtk" => gtk::stage(project, &set),
        "qt" => qt::stage(project, &set),
        "xaml" => xaml::stage(project, &set),
        _ => Ok(()),
    }
}

/// A prepared `resource/vectors/` glyph: its resolution name and standalone SVG text (an SF
/// Symbol template already reduced to its canonical Regular variant, a `.symbolset` bundle to
/// its inner art).
pub struct VectorAsset {
    pub name: String,
    pub glyph: String,
    /// XAML geometry was emitted for this glyph — windows-xaml draws it without a raster.
    pub xaml: bool,
    /// The glyph converts to an Android VectorDrawable — android-mdc needs no raster for it.
    pub vd: bool,
}

/// Where the build-time vector rasters live: EVERY glyph, always, as the build's own cache. This
/// directory is an input, not a shipping form — what a target actually carries is
/// [`vector_fallback_dir`], which holds only the glyphs that target cannot draw as a vector.
pub fn vector_raster_dir(project: &Project) -> PathBuf {
    project.root.join("build/day/vectors/raster")
}

/// Toolkits that draw `resource/vectors/` glyphs from a REAL vector form (docs/vectors.md).
///
/// On these the raster is not a shipping asset at all — bundling it would add a second copy of
/// every glyph, and, worse, stand in silently when the vector path fails, so a broken renderer
/// still looks right. Both XAML bugs found while building that backend (quadratics rejected by
/// the parser, then XamlReader returning nothing under the island's metadata provider) were
/// invisible for exactly that reason. What ships instead is per-GLYPH: art the vector pipeline
/// could not express still needs its raster, and only that art gets one.
fn toolkit_draws_vectors(toolkit: &str) -> bool {
    matches!(
        toolkit,
        "appkit" | "uikit" | "mdc" | "arkui" | "dom" | "xaml"
    )
}

/// The glyphs `toolkit` must ship a raster for — everything, where it has no vector arm at all
/// (gtk, qt); otherwise only the art its vector pipeline could not express.
fn vector_fallback_names(toolkit: &str, vectors: &[VectorAsset]) -> Vec<String> {
    vectors
        .iter()
        .filter(|v| match toolkit {
            // Staged as SVG, which these render natively for every glyph — nothing falls back.
            "appkit" | "uikit" | "arkui" | "dom" => false,
            "xaml" => !v.xaml,
            "mdc" => !v.vd,
            // No vector arm: the raster IS the glyph here.
            _ => true,
        })
        .map(|v| v.name.clone())
        .collect()
}

/// Where `toolkit`'s raster fallbacks are staged — a per-toolkit directory, so building two
/// targets never races over one shared tree the way filtering the cache in place would.
pub fn vector_fallback_dir(project: &Project, toolkit: &str) -> PathBuf {
    project
        .root
        .join("build/day/vectors/fallback")
        .join(toolkit)
}

/// Materialize [`vector_fallback_dir`]: the raster cache filtered to what this toolkit actually
/// needs. Rewritten from empty each time, so a glyph that starts converting stops shipping a
/// raster on the very next build.
pub fn write_vector_fallbacks(
    project: &Project,
    toolkit: &str,
    vectors: &[VectorAsset],
) -> Result<PathBuf, String> {
    let dir = vector_fallback_dir(project, toolkit);
    let _ = std::fs::remove_dir_all(&dir);
    // Always created, even when nothing falls back: the roots that point here (the launch env,
    // the packed trees) are then an EMPTY directory rather than a missing one, which is the
    // difference between "this target ships no rasters" and "the path is wrong".
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir vector fallbacks: {e}"))?;
    let names = vector_fallback_names(toolkit, vectors);
    let cache = vector_raster_dir(project);
    for name in &names {
        let from = cache.join(format!("{name}.png"));
        if from.is_file() {
            std::fs::copy(&from, dir.join(format!("{name}.png")))
                .map_err(|e| format!("vector fallback {name}: {e}"))?;
        }
    }
    // Reported on the toolkits that draw vectors, including when the answer is none: a raster
    // here is the coverage-honest degradation, so it should be visible in the build rather than
    // discovered later as a bundle that is bigger than it should be.
    if toolkit_draws_vectors(toolkit) && !vectors.is_empty() {
        let detail = if names.is_empty() {
            "every glyph draws as a vector".to_string()
        } else {
            format!("raster fallback: {}", names.join(", "))
        };
        status(
            "Vectors",
            &format!(
                "{toolkit}: {}/{} glyph(s) vector — {detail}",
                vectors.len() - names.len(),
                vectors.len()
            ),
        );
    }
    Ok(dir)
}

/// Where the prepared glyph SVGs live (docs/vectors.md): the Apple catalogs copy these in as
/// preserve-vector imagesets, and day-appkit's `DAY_VECTOR_SVG_ROOT` probe loads them directly
/// (NSImage renders SVG at display size on macOS 11+).
pub fn vector_svg_dir(project: &Project) -> PathBuf {
    project.root.join("build/day/vectors/svg")
}

/// Where the prepared XAML geometry lives (docs/vectors.md): day-xaml loads these as real
/// `Path`/`PathIcon` geometry, which is what lets a Windows glyph stay vector at any size AND
/// take its tint as a brush at runtime. Converted here, in the CLI, so the backend needs no SVG
/// parser — the same split Android's VectorDrawable emission uses. A glyph outside the
/// convertible subset simply has no file here and falls back to the raster cache.
pub fn vector_xaml_dir(project: &Project) -> PathBuf {
    project.root.join("build/day/vectors/xaml")
}

/// The raster edge for cached vector PNGs: sized for icon duty (nav rows, grids) at high-dpi.
const VECTOR_RASTER_PX: u32 = 256;

/// Scan `resource/vectors/` (plain `.svg`, SF-template `.svg`, `.symbolset/` bundles), reduce
/// each to a standalone glyph, and (re)write the raster cache. Returns the prepared glyphs.
pub fn prepare_vectors(project: &Project) -> Result<Vec<VectorAsset>, String> {
    let src = project.root.join("resource/vectors");
    let cache = vector_raster_dir(project);
    let svgs = vector_svg_dir(project);
    let geom = vector_xaml_dir(project);
    // Regenerate fresh so removed/renamed vectors don't linger in any pipeline.
    let _ = std::fs::remove_dir_all(&cache);
    let _ = std::fs::remove_dir_all(&svgs);
    let _ = std::fs::remove_dir_all(&geom);
    if !src.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&src)
        .map_err(|e| format!("resource/vectors: {e}"))?
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    let mut out = Vec::new();
    for path in entries {
        let fname = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if fname.starts_with('.') {
            continue;
        }
        let (name, svg_path) = if path.is_file() && fname.to_ascii_lowercase().ends_with(".svg") {
            (fname[..fname.len() - 4].to_string(), path.clone())
        } else if path.is_dir() && fname.to_ascii_lowercase().ends_with(".symbolset") {
            // The bundle's inner template SVG (first .svg member).
            let inner = std::fs::read_dir(&path)
                .ok()
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .find(|p| p.extension().and_then(|x| x.to_str()) == Some("svg"))
                .ok_or_else(|| format!("{fname}: .symbolset bundle has no inner .svg"))?;
            (fname[..fname.len() - ".symbolset".len()].to_string(), inner)
        } else {
            continue;
        };
        let text = std::fs::read_to_string(&svg_path).map_err(|e| format!("vector {name}: {e}"))?;
        // SF Symbol templates reduce to the canonical Regular variant (the plan's
        // "Regular only" policy) — and the Light/Bold variants ALSO stage, under
        // `__light`/`__bold` suffixed names, which is what the piece's `.weight(…)` resolves
        // (docs/vectors.md). Plain SVGs are the glyph as-is; their weight names alias the same
        // art, so `.weight(…)` degrades to Regular rather than to a missing asset, everywhere.
        let template = day_vector::classify(&text) == day_vector::SourceKind::SfTemplate;
        std::fs::create_dir_all(&cache).map_err(|e| format!("mkdir vectors cache: {e}"))?;
        for (suffix, weight) in [("", "Regular"), ("__light", "Light"), ("__bold", "Bold")] {
            let glyph = if template {
                day_vector::extract_variant(&text, weight, "M")
                    .map_err(|e| format!("vector {name}: {e}"))?
            } else {
                text.clone()
            };
            let staged = format!("{name}{suffix}");
            // Post-extraction check: a template's Notes/Guides carry documentation <text> that
            // never ships; only text in the GLYPH itself is unrenderable (shaping is not
            // compiled in — outline it).
            if glyph.contains("<text") {
                return Err(format!(
                    "vector {staged}: contains <text> — outline text in your editor (docs/vectors.md)"
                ));
            }
            let tree =
                day_vector::parse(glyph.as_bytes()).map_err(|e| format!("vector {staged}: {e}"))?;
            let png = day_vector::render_png(&tree, VECTOR_RASTER_PX)
                .map_err(|e| format!("vector {staged}: {e}"))?;
            std::fs::write(cache.join(format!("{staged}.png")), png)
                .map_err(|e| format!("vector cache {staged}: {e}"))?;
            std::fs::create_dir_all(&svgs).map_err(|e| format!("mkdir vectors svg: {e}"))?;
            std::fs::write(svgs.join(format!("{staged}.svg")), glyph.as_bytes())
                .map_err(|e| format!("vector svg {staged}: {e}"))?;
            // XAML geometry, when the art converts. Unlike the Android emission this is not a
            // declared-target concern — every project stages it, and a glyph that cannot convert
            // just leaves day-xaml on the raster, so there is nothing to warn about here.
            let xaml_ok = match day_vector::to_xaml_geometry(&tree) {
                Ok(g) => {
                    std::fs::create_dir_all(&geom)
                        .map_err(|e| format!("mkdir vectors xaml: {e}"))?;
                    std::fs::write(geom.join(format!("{staged}.xamlgeom")), g.to_spec())
                        .map_err(|e| format!("vector xaml {staged}: {e}"))?;
                    true
                }
                Err(_) => false,
            };
            // Recorded, not emitted, here: android::stage does the conversion it ships (it needs
            // the XML and the reason to report). This only answers "does this glyph need a
            // raster on android", which decides whether one is staged at all.
            let vd_ok = day_vector::to_vector_drawable(&tree).is_ok();
            out.push(VectorAsset {
                name: staged,
                glyph,
                xaml: xaml_ok,
                vd: vd_ok,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod vector_tests {
    use super::*;

    fn glyphs() -> Vec<VectorAsset> {
        vec![
            // Ordinary art: converts everywhere.
            VectorAsset {
                name: "plain".into(),
                glyph: String::new(),
                xaml: true,
                vd: true,
            },
            // A gradient, say: no geometry on either converting backend.
            VectorAsset {
                name: "fancy".into(),
                glyph: String::new(),
                xaml: false,
                vd: false,
            },
        ]
    }

    /// The rule the bundle size depends on: a toolkit that draws vectors ships a raster ONLY
    /// for art its vector pipeline could not express. Shipping the rest would double every
    /// glyph and let a broken vector path hide behind a raster that still looks right.
    #[test]
    fn drawing_toolkits_ship_rasters_only_for_unconvertible_art() {
        let g = glyphs();
        assert_eq!(vector_fallback_names("xaml", &g), ["fancy"]);
        assert_eq!(vector_fallback_names("mdc", &g), ["fancy"]);
        // These stage an SVG for every glyph, so nothing falls back at all.
        for toolkit in ["appkit", "uikit", "arkui", "dom"] {
            assert!(
                vector_fallback_names(toolkit, &g).is_empty(),
                "{toolkit} should ship no rasters"
            );
        }
    }

    /// The other half: a toolkit with no vector arm still needs every glyph as a raster, or its
    /// icons simply vanish.
    #[test]
    fn toolkits_without_a_vector_arm_ship_every_raster() {
        let g = glyphs();
        for toolkit in ["gtk", "qt"] {
            assert_eq!(
                vector_fallback_names(toolkit, &g),
                ["plain", "fancy"],
                "{toolkit} has no vector arm and needs both"
            );
            assert!(!toolkit_draws_vectors(toolkit));
        }
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
