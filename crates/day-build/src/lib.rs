//! day-build — resource-constant codegen for a Day app's `build.rs` (DESIGN.md §18.5).
//!
//! An app's `build.rs` calls [`generate_resources`], which scans the project's
//! `resource/{images,assets,fonts}` directories and writes typed symbolic constants to
//! `$OUT_DIR/day_resources.rs`:
//!
//! ```text
//! pub mod images { use day::ImageName;
//!     pub const nav_system: ImageName = ImageName::from_static("nav_system"); }
//! pub mod assets { use day::AssetName;
//!     pub const numbers_bin: AssetName = AssetName::from_static("numbers.bin"); }
//! pub mod fonts  { use day::FontFamily;
//!     pub const pacifico: FontFamily = FontFamily::from_static("Pacifico"); }
//! ```
//!
//! The app surfaces it once (`pub mod res { include!(concat!(env!("OUT_DIR"), "/day_resources.rs")); }`)
//! and then writes `image(res::images::nav_system)` — a typo is a compile error and the resource is
//! guaranteed bundled. `cargo:rerun-if-changed` on each resource dir regenerates when a file is
//! added or removed.
//!
//! This crate is also the canonical source of the resource-name → identifier rules: the CLI stagers
//! (`day-cli/src/resources`) reuse [`sanitize_ident`] and the derivation helpers here so the string
//! baked into a constant is exactly the name staged into each backend's native store.

use std::path::{Path, PathBuf};

/// A single generated constant: its Rust `symbol`, the `value` string it wraps (the wire name the
/// backend resolves by), and the `source` file (for the doc comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub symbol: String,
    pub value: String,
    pub source: String,
}

/// The full set of constants to emit, grouped by bucket.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ResourcePlan {
    pub images: Vec<Entry>,
    pub assets: Vec<Entry>,
    pub fonts: Vec<Entry>,
}

/// The build-script entry point: scan `resource/{images,assets,fonts}` under `CARGO_MANIFEST_DIR`,
/// emit `$OUT_DIR/day_resources.rs`, and register the resource dirs for `cargo:rerun-if-changed`.
/// Returns `Err` (with a fix hint) on a name that is not portable or a symbol collision — the app
/// `build.rs` should `.expect(...)` this so the problem fails the build loudly.
pub fn generate_resources() -> Result<(), String> {
    let root = PathBuf::from(env("CARGO_MANIFEST_DIR")?);
    let out = PathBuf::from(env("OUT_DIR")?);
    let plan = plan_resources(&root)?;
    let code = render(&plan);
    std::fs::write(out.join("day_resources.rs"), code)
        .map_err(|e| format!("day-build: writing day_resources.rs: {e}"))?;
    // Regenerate when a resource is added/removed/renamed (a proc-macro could not do this reliably).
    for bucket in ["images", "assets", "fonts"] {
        println!("cargo:rerun-if-changed=resource/{bucket}");
    }
    Ok(())
}

fn env(key: &str) -> Result<String, String> {
    std::env::var(key).map_err(|_| format!("day-build: ${key} is not set (call from a build.rs)"))
}

/// Scan and validate a project's resources into a [`ResourcePlan`] (the pure, testable core).
pub fn plan_resources(root: &Path) -> Result<ResourcePlan, String> {
    Ok(ResourcePlan {
        images: plan_images(&root.join("resource/images"))?,
        assets: plan_assets(&root.join("resource/assets"))?,
        fonts: plan_fonts(&root.join("resource/fonts"))?,
    })
}

/// Top-level, non-hidden files in `dir`, sorted by name for deterministic output.
fn list_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
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
    files
}

/// Images: the constant is keyed on the file **stem** (with any `@Nx` HiDPI suffix stripped), which
/// is the name `image("…")` resolves by. The stem must be *portable* — identical after
/// [`sanitize_ident`] — because Apple/GTK/Qt resolve it verbatim while Android/ArkUI re-sanitize it;
/// a non-portable stem would silently resolve to two different names across toolkits, so it is a hard
/// error with a rename hint. `foo.png` + `foo@2x.png` collapse to one constant; two *distinct* files
/// claiming the same stem at the same scale collide.
fn plan_images(dir: &Path) -> Result<Vec<Entry>, String> {
    // stem -> (scales seen, first source path)
    let mut seen: std::collections::BTreeMap<String, (Vec<u32>, String)> = Default::default();
    for path in list_files(dir) {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let (base, scale) = parse_scale(&stem);
        let src = display(&path);
        let sane = sanitize_ident(&base);
        if sane != base {
            return Err(format!(
                "day-build: image {base:?} ({src}) is not a portable resource name — it resolves \
                 to {sane:?} on Android/HarmonyOS but {base:?} on Apple/GTK/Qt. Rename the file so \
                 its stem is lowercase [a-z0-9_] (e.g. `{sane}`)."
            ));
        }
        let ent = seen.entry(base.clone()).or_insert_with(|| (Vec::new(), src.clone()));
        if ent.0.contains(&scale) {
            return Err(format!(
                "day-build: two files map to image {base:?} at the same scale ({}, {src}) — keep \
                 one file per image (HiDPI variants use an `@2x`/`@3x` suffix).",
                ent.1
            ));
        }
        ent.0.push(scale);
    }
    Ok(seen
        .into_iter()
        .map(|(base, (_, src))| Entry {
            symbol: base.clone(),
            value: base,
            source: src,
        })
        .collect())
}

/// Data assets: the constant wraps the **full file name** (extension included) — the exact string
/// `resource("…")` resolves by — with the symbol sanitized for Rust (`numbers.bin` → `numbers_bin`).
fn plan_assets(dir: &Path) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for path in list_files(dir) {
        let fname = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        entries.push(Entry {
            symbol: sanitize_ident(&fname),
            value: fname,
            source: display(&path),
        });
    }
    dedup_symbols(entries, "asset")
}

/// Fonts: the constant wraps the **family name** parsed from the sfnt `name` table (what
/// `Font::custom` resolves by, *not* the file name), with the symbol derived by the same
/// `font_ident` rule the runtimes use (`"Special Elite"` → `special_elite`).
fn plan_fonts(dir: &Path) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for path in list_files(dir) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if !matches!(ext.as_str(), "ttf" | "otf") {
            continue; // non-font files are ignored (matches scan_fonts, which errors at stage time)
        }
        let src = display(&path);
        let bytes =
            std::fs::read(&path).map_err(|e| format!("day-build: reading {src}: {e}"))?;
        let names = day_fonts::parse_font_names(&bytes)
            .ok_or_else(|| format!("day-build: {src}: not a recognizable font (no name table)"))?;
        entries.push(Entry {
            symbol: day_fonts::font_ident(&names.family),
            value: names.family,
            source: src,
        });
    }
    dedup_symbols(entries, "font")
}

/// Reject two entries whose symbols collide after sanitization (they would define the same constant).
fn dedup_symbols(entries: Vec<Entry>, kind: &str) -> Result<Vec<Entry>, String> {
    let mut seen: std::collections::BTreeMap<String, String> = Default::default();
    for e in &entries {
        if let Some(prev) = seen.insert(e.symbol.clone(), e.source.clone()) {
            return Err(format!(
                "day-build: {kind}s {} and {} both map to the symbol `{}` — rename one so they \
                 differ after sanitization to [a-z0-9_].",
                prev, e.source, e.symbol
            ));
        }
    }
    Ok(entries)
}

/// Render a plan to the `day_resources.rs` source text. This file is `include!`d inside the app's
/// `pub mod res { … }`, so the lint waivers are **outer** attributes on each bucket module (an inner
/// `#![…]` is not valid at an `include!` site) and cover a bucket with no constants (unused `use`).
pub fn render(plan: &ResourcePlan) -> String {
    let mut s = String::new();
    s.push_str("// @generated by day-build — do not edit.\n");
    s.push_str("// Regenerated on every build from resource/{images,assets,fonts}.\n\n");
    render_bucket(&mut s, "images", "ImageName", &plan.images);
    render_bucket(&mut s, "assets", "AssetName", &plan.assets);
    render_bucket(&mut s, "fonts", "FontFamily", &plan.fonts);
    s
}

fn render_bucket(s: &mut String, module: &str, ty: &str, entries: &[Entry]) {
    s.push_str("#[allow(non_upper_case_globals, dead_code, unused_imports)]\n");
    s.push_str(&format!("pub mod {module} {{\n    use day::{ty};\n"));
    for e in entries {
        s.push_str(&format!(
            "    /// `{}`\n    pub const {}: {ty} = {ty}::from_static({:?});\n",
            e.source,
            ident_token(&e.symbol),
            e.value,
        ));
    }
    s.push_str("}\n\n");
}

/// Wrap a Rust keyword symbol as a raw identifier so a resource named e.g. `type` still compiles.
fn ident_token(sym: &str) -> String {
    const KEYWORDS: &[&str] = &[
        "as", "break", "const", "continue", "dyn", "else", "enum", "extern", "false", "fn", "for",
        "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
        "static", "struct", "trait", "true", "type", "union", "unsafe", "use", "where", "while",
        "async", "await", "try",
    ];
    if KEYWORDS.contains(&sym) {
        format!("r#{sym}")
    } else {
        sym.to_string()
    }
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

/// Sanitize a name to the strictest platform identifier rules (Android `R` / ArkUI): lowercase, only
/// `[a-z0-9_]`, forced leading letter. The canonical copy — the CLI stagers re-export this so the
/// staged native name and the generated constant string agree by construction.
pub fn sanitize_ident(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if !s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        s.insert(0, 'r');
    }
    s
}

/// A project-relative-ish display path for error messages / doc comments (`resource/images/x.png`).
fn display(path: &Path) -> String {
    // Keep the last three components (`resource/<bucket>/<file>`) when present — stable across
    // machines and enough to locate the file.
    let comps: Vec<_> = path.components().collect();
    let n = comps.len();
    let start = n.saturating_sub(3);
    comps[start..]
        .iter()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(label: &str) -> PathBuf {
        // Unique per test so the parallel test threads never clobber each other's dirs.
        let d = std::env::temp_dir().join(format!("day-build-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn touch(dir: &Path, name: &str, bytes: &[u8]) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), bytes).unwrap();
    }

    #[test]
    fn sanitize_matches_strictest_rules() {
        assert_eq!(sanitize_ident("nav_system"), "nav_system");
        assert_eq!(sanitize_ident("Nav-System"), "nav_system");
        assert_eq!(sanitize_ident("123"), "r123");
        assert_eq!(sanitize_ident("numbers.bin"), "numbers_bin");
    }

    #[test]
    fn images_dedup_scale_variants_and_key_on_stem() {
        let root = tmp("images-dedup");
        let img = root.join("resource/images");
        touch(&img, "nav_system.png", b"x");
        touch(&img, "day_logo.png", b"x");
        touch(&img, "day_logo@2x.png", b"x"); // HiDPI variant of the same logical image
        let plan = plan_resources(&root).unwrap();
        let syms: Vec<_> = plan.images.iter().map(|e| e.symbol.as_str()).collect();
        assert_eq!(syms, vec!["day_logo", "nav_system"]);
        assert_eq!(plan.images[0].value, "day_logo");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn non_portable_image_stem_is_rejected() {
        let root = tmp("non-portable");
        touch(&root.join("resource/images"), "Nav-System.png", b"x");
        let err = plan_resources(&root).unwrap_err();
        assert!(err.contains("portable"), "{err}");
        assert!(err.contains("nav_system"), "{err}"); // suggests the fix
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn same_stem_same_scale_collides() {
        let root = tmp("collide");
        let img = root.join("resource/images");
        touch(&img, "logo.png", b"x");
        touch(&img, "logo.jpg", b"x"); // two distinct files, both stem `logo`, scale 1
        let err = plan_resources(&root).unwrap_err();
        assert!(err.contains("same scale"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn asset_symbol_sanitized_value_verbatim() {
        let root = tmp("assets");
        touch(&root.join("resource/assets"), "numbers.bin", b"x");
        let plan = plan_resources(&root).unwrap();
        assert_eq!(plan.assets[0].symbol, "numbers_bin");
        assert_eq!(plan.assets[0].value, "numbers.bin");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn render_shape_is_typed_and_lowercase() {
        let plan = ResourcePlan {
            images: vec![Entry {
                symbol: "nav_system".into(),
                value: "nav_system".into(),
                source: "resource/images/nav_system.png".into(),
            }],
            assets: vec![],
            fonts: vec![],
        };
        let code = render(&plan);
        assert!(code.contains("#[allow(non_upper_case_globals, dead_code, unused_imports)]"));
        assert!(code.contains("pub mod images {"));
        assert!(code.contains("use day::ImageName;"));
        assert!(code.contains(
            "pub const nav_system: ImageName = ImageName::from_static(\"nav_system\");"
        ));
    }

    #[test]
    fn keyword_symbol_becomes_raw_ident() {
        let plan = ResourcePlan {
            images: vec![Entry {
                symbol: "type".into(),
                value: "type".into(),
                source: "resource/images/type.png".into(),
            }],
            ..Default::default()
        };
        assert!(render(&plan).contains("pub const r#type: ImageName"));
    }

    #[test]
    fn missing_dirs_yield_empty_plan() {
        let root = tmp("missing-dirs");
        std::fs::create_dir_all(&root).unwrap();
        let plan = plan_resources(&root).unwrap();
        assert!(plan.images.is_empty() && plan.assets.is_empty() && plan.fonts.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }
}
