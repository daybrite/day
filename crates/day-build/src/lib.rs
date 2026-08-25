// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

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
//! pub mod locales { pub const DEFAULT: &str = "en";
//!     pub const CATALOG: &[(&str, &str)] = &[("en", include_str!("…/en/app.ftl")), …];
//!     pub const ALL: &[(&str, &str)] = &[("en", "English"), …];  // tag + self-name
//!     pub fn install() { day::install_locales(DEFAULT, CATALOG) } }
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
//!
//! For the same reason it owns [`permissions`]: the CLI generates each platform's permission
//! declarations from that table while `day-part-permissions` queries the same permissions at
//! runtime, and the two must never disagree (docs/permissions.md).

use std::path::{Path, PathBuf};

pub mod bridge;
pub mod permissions;
pub mod swiftui;

/// A single generated constant: its Rust `symbol`, the `value` string it wraps (the wire name the
/// backend resolves by), and the `source` file (for the doc comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub symbol: String,
    pub value: String,
    pub source: String,
}

/// A generated localization function: the Fluent message `key` (the Rust fn name), its sorted
/// `params` (each `$variable` the message references, agreed across all locales), and `doc` (the
/// reference-locale value text, for the generated doc comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrEntry {
    pub key: String,
    pub params: Vec<StrParam>,
    pub doc: String,
}

/// One generated function parameter: the Fluent `$variable` name and whether it is used as a
/// **number** (a plural/`select` selector or `NUMBER()` argument) — which types it as
/// `IntoNumberFArg` instead of `IntoFArg`, so a string can't be passed where a plural count is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrParam {
    pub name: String,
    pub numeric: bool,
}

/// One locale's catalog: the directory name under `resource/locales/` (the tag apps pass to
/// `set_locale`) and every `.ftl` beneath it, sorted. Multiple files concatenate into the single
/// source string the Fluent bundle is built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleEntry {
    pub locale: String,
    pub sources: Vec<PathBuf>,
}

/// The full set of constants to emit, grouped by bucket.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ResourcePlan {
    pub images: Vec<Entry>,
    /// `resource/vectors/` — SVG glyphs (and `.symbolset` bundles), typed as `res::vectors::…`
    /// `VectorName` constants (docs/vectors.md).
    pub vectors: Vec<Entry>,
    /// `resource/assets/` — a TREE (§18.5): directories nest, rendered as nested modules with an
    /// `AssetDir` const per folder and an `AssetName` const per file (values are `/`-relative
    /// paths). Top-level files keep the flat form older apps compiled against.
    pub assets: AssetNode,
    pub fonts: Vec<Entry>,
    /// Localization keys → `res::str::<key>(params…)` functions (§18.5).
    pub strings: Vec<StrEntry>,
    /// The locales themselves → the `res::locales` catalog (§18.5), so an app registers them
    /// with one call instead of a hand-maintained `include_str!` list.
    pub locales: Vec<LocaleEntry>,
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
    for bucket in ["images", "vectors", "assets", "fonts", "locales"] {
        println!("cargo:rerun-if-changed=resource/{bucket}");
    }
    // Typed constructors for the SwiftUI views exported by declared local SwiftPM packages
    // (docs/swiftui.md) — always written, surfaced by an app that wants them via
    // `pub mod swiftui { include!(concat!(env!("OUT_DIR"), "/day_swiftui.rs")); }`.
    swiftui::generate_bindings(&root, &out)?;
    println!("cargo:rerun-if-changed=Cargo.toml");
    Ok(())
}

fn env(key: &str) -> Result<String, String> {
    std::env::var(key).map_err(|_| format!("day-build: ${key} is not set (call from a build.rs)"))
}

/// Scan and validate a project's resources into a [`ResourcePlan`] (the pure, testable core).
pub fn plan_resources(root: &Path) -> Result<ResourcePlan, String> {
    Ok(ResourcePlan {
        images: plan_images(&root.join("resource/images"))?,
        vectors: plan_vectors(&root.join("resource/vectors"))?,
        assets: plan_assets(&root.join("resource/assets"))?,
        fonts: plan_fonts(&root.join("resource/fonts"))?,
        strings: plan_strings(&root.join("resource/locales"))?,
        locales: plan_locales(&root.join("resource/locales")),
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
/// Vectors (docs/vectors.md): `resource/vectors/*.svg` files plus `*.symbolset` bundle
/// directories, keyed on the stem. Same portability rule as images — every backend resolves the
/// stem, Android/HarmonyOS re-sanitize it.
fn plan_vectors(dir: &Path) -> Result<Vec<Entry>, String> {
    let mut out: Vec<Entry> = Vec::new();
    let mut names: std::collections::BTreeSet<String> = Default::default();
    let entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    let mut sorted = entries;
    sorted.sort();
    for path in sorted {
        let fname = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if fname.starts_with('.') {
            continue;
        }
        let stem = match (path.is_file(), path.is_dir()) {
            (true, _) if fname.to_ascii_lowercase().ends_with(".svg") => path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string(),
            (_, true) if fname.to_ascii_lowercase().ends_with(".symbolset") => {
                fname[..fname.len() - ".symbolset".len()].to_string()
            }
            _ => continue,
        };
        let sane = sanitize_ident(&stem);
        if sane != stem {
            return Err(format!(
                "day-build: vector {stem:?} ({}) is not a portable resource name — rename it so \
                 its stem is lowercase [a-z0-9_] (e.g. `{sane}`).",
                display(&path)
            ));
        }
        if !names.insert(stem.clone()) {
            return Err(format!(
                "day-build: two entries map to vector {stem:?} — keep one .svg or .symbolset per name."
            ));
        }
        out.push(Entry {
            symbol: stem.clone(),
            value: stem,
            source: display(&path),
        });
    }
    Ok(out)
}

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
        let ent = seen
            .entry(base.clone())
            .or_insert_with(|| (Vec::new(), src.clone()));
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

/// One directory level of the assets tree (§18.5). `path` is the folder's `/`-relative path under
/// `resource/assets/` (`""` at the root); each child directory renders as an `AssetDir` const AND
/// a nested module sharing its name, so `res::assets::web::minisite` names the folder and
/// `res::assets::web::minisite::index_html` a file within it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AssetNode {
    pub path: String,
    pub files: Vec<Entry>,
    /// `(module/const symbol, subtree)`, sorted by symbol.
    pub dirs: Vec<(String, AssetNode)>,
}

/// Data assets: a recursive tree. Each file constant wraps the `/`-relative path — the exact
/// string `resource("…")` resolves by — with the symbol sanitized from the file name alone
/// (`numbers.bin` → `numbers_bin`); each directory yields an `AssetDir` const plus a nested
/// module. File and directory symbols share one namespace per level (both are consts), so a
/// collision at any level is a build error naming both sources.
fn plan_assets(dir: &Path) -> Result<AssetNode, String> {
    plan_asset_dir(dir, "")
}

fn plan_asset_dir(dir: &Path, rel: &str) -> Result<AssetNode, String> {
    let mut files = Vec::new();
    for path in list_files(dir) {
        let fname = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let value = if rel.is_empty() {
            fname.clone()
        } else {
            format!("{rel}/{fname}")
        };
        files.push(Entry {
            symbol: sanitize_ident(&fname),
            value,
            source: display(&path),
        });
    }
    let mut dirs = Vec::new();
    let mut subdirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .starts_with('.')
        })
        .collect();
    subdirs.sort();
    for sub in subdirs {
        let dname = sub
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let sub_rel = if rel.is_empty() {
            dname.clone()
        } else {
            format!("{rel}/{dname}")
        };
        dirs.push((sanitize_ident(&dname), plan_asset_dir(&sub, &sub_rel)?));
    }
    // Files and directories land in one const namespace per module — validate them jointly.
    let mut probe = files.clone();
    for (sym, node) in &dirs {
        probe.push(Entry {
            symbol: sym.clone(),
            value: node.path.clone(),
            source: format!("resource/assets/{} (directory)", node.path),
        });
    }
    dedup_symbols(probe, "asset")?;
    Ok(AssetNode {
        path: rel.to_string(),
        files,
        dirs,
    })
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
        let bytes = std::fs::read(&path).map_err(|e| format!("day-build: reading {src}: {e}"))?;
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

/// Recursively collect every `*.ftl` under `dir` (sorted, for deterministic diagnostics/output).
fn ftl_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "ftl") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// The message keys defined in a Fluent source (terms/comments ignored — and ATTRIBUTES too:
/// a locale that omits `menu_group.key` deliberately inherits the default locale's shortcut,
/// so the coverage lint must not demand attributes everywhere). Public so the CLI lint
/// (`day lint` fluent coverage) shares this one `fluent-syntax` parser with the codegen and
/// the runtime resolver, instead of a hand-rolled line scanner.
pub fn message_keys(ftl_src: &str) -> Vec<String> {
    ftl_messages(ftl_src)
        .into_iter()
        .map(|m| m.key)
        .filter(|k| !k.contains('.'))
        .collect()
}

/// Localization keys → parameter-typed `res::str` functions. Parses each `.ftl` with `fluent-syntax`
/// (the same syntax `fluent-bundle` resolves at runtime), collects every message's `$variable` set
/// (and which vars are numeric — plural/`select` selectors), unions keys across locales, and enforces
/// two build-time rules: each key must be a valid Rust identifier (the kebab→snake forcing rule) and
/// all locales must agree on a key's parameter names. A param is typed numeric if *any* locale uses it
/// numerically; the generated doc shows the value from the reference locale (`en` if present).
fn plan_strings(dir: &Path) -> Result<Vec<StrEntry>, String> {
    // key -> (params: name -> numeric, the locale file that first defined it)
    let mut agreed: std::collections::BTreeMap<String, (Params, String)> = Default::default();
    // key -> (reference value text, whether it came from `en`)
    let mut docs: std::collections::BTreeMap<String, (String, bool)> = Default::default();
    for path in ftl_files(dir) {
        let src = std::fs::read_to_string(&path)
            .map_err(|e| format!("day-build: reading {}: {e}", display(&path)))?;
        let loc = display(&path);
        let is_en = locale_of(&path) == "en";
        for msg in ftl_messages(&src) {
            let ident_ok = match msg.key.split_once('.') {
                // `message.attr` (an attribute entry): both halves become one generated fn
                // name, `message_attr`, so both must be identifiers.
                Some((m, a)) => is_rust_ident(m) && is_rust_ident(a),
                None => is_rust_ident(&msg.key),
            };
            if !ident_ok {
                return Err(format!(
                    "day-build: localization key {:?} ({loc}) is not a valid Rust identifier — \
                     rename it to snake_case (e.g. `{}`) in every resource/locales/*/*.ftl (Fluent \
                     allows `-`, Rust identifiers do not).",
                    msg.key,
                    msg.key.replace(['-', '.'], "_")
                ));
            }
            // Doc: prefer the `en` value, else keep the first one seen.
            let have_en = matches!(docs.get(&msg.key), Some((_, true)));
            if !have_en && (is_en || !docs.contains_key(&msg.key)) {
                docs.insert(msg.key.clone(), (msg.value_text, is_en));
            }
            // Params: names must agree across locales; numeric is the OR across locales.
            use std::collections::btree_map::Entry;
            match agreed.entry(msg.key.clone()) {
                Entry::Vacant(v) => {
                    v.insert((msg.params, loc.clone()));
                }
                Entry::Occupied(mut o) => {
                    let (prev, prev_loc) = o.get_mut();
                    let prev_names: Vars = prev.keys().cloned().collect();
                    let this_names: Vars = msg.params.keys().cloned().collect();
                    if prev_names != this_names {
                        return Err(format!(
                            "day-build: localization key {:?} references different parameters across \
                             locales — {prev_loc} has {{{}}}, {loc} has {{{}}}. Every locale's \
                             message must use the same `$variables`.",
                            msg.key,
                            comma(&prev_names),
                            comma(&this_names)
                        ));
                    }
                    for (name, numeric) in msg.params {
                        if numeric && let Some(v) = prev.get_mut(&name) {
                            *v = true;
                        }
                    }
                }
            }
        }
    }
    // An attribute's generated fn is `message_attr` — it must not collide with a real
    // message of that name (or another attribute flattening to it).
    {
        let mut fn_names: std::collections::BTreeMap<String, &String> = Default::default();
        for key in agreed.keys() {
            let fn_name = key.replace('.', "_");
            if let Some(prev) = fn_names.insert(fn_name.clone(), key) {
                return Err(format!(
                    "day-build: localization keys {prev:?} and {key:?} both generate \
                     `res::str::{fn_name}()` — rename one (a `message.attr` attribute \
                     flattens to `message_attr`)."
                ));
            }
        }
    }
    Ok(agreed
        .into_iter()
        .map(|(key, (params, _))| {
            let doc = docs.remove(&key).map(|(t, _)| t).unwrap_or_default();
            StrEntry {
                key,
                params: params
                    .into_iter()
                    .map(|(name, numeric)| StrParam { name, numeric })
                    .collect(),
                doc,
            }
        })
        .collect())
}

fn comma(names: &Vars) -> String {
    names.iter().cloned().collect::<Vec<_>>().join(", ")
}

/// Group `resource/locales/<locale>/**/*.ftl` by locale directory — the catalog `res::locales`
/// renders. Discovery is the whole point: adding or deleting a locale directory is the entire
/// act of adding or dropping a language, with no source list to keep in step (the
/// `cargo:rerun-if-changed` on `resource/locales` in [`generate_resources`] is what makes the
/// directory itself a build input).
///
/// Not fallible: unlike [`plan_strings`] — which validates keys and parameters — a locale
/// directory carries no name rules of its own. A tag that Fluent can't parse degrades to `en`
/// at runtime (`day_l10n::build_bundles`), which is the engine's call, not the build's.
fn plan_locales(dir: &Path) -> Vec<LocaleEntry> {
    let mut by_locale: std::collections::BTreeMap<String, Vec<PathBuf>> = Default::default();
    for path in ftl_files(dir) {
        let locale = locale_of(&path);
        // A stray `.ftl` directly in `resource/locales/` has the bucket itself as its parent and
        // names no locale — skip it rather than inventing a `locales` language.
        if locale.is_empty() || path.parent() == Some(dir) {
            continue;
        }
        by_locale.entry(locale).or_default().push(path);
    }
    by_locale
        .into_iter()
        .map(|(locale, sources)| LocaleEntry { locale, sources })
        .collect()
}

/// The fallback locale for the generated `install()`: `en` when the app ships it, else the first
/// locale alphabetically (a single-locale app gets its own language; a multi-locale app without
/// English gets a deterministic pick). Apps needing another default call
/// `install_locales(other, res::locales::CATALOG)` — the catalog stays generated either way.
fn default_locale(locales: &[LocaleEntry]) -> String {
    if locales.iter().any(|l| l.locale == "en") {
        return "en".to_string();
    }
    locales
        .first()
        .map(|l| l.locale.clone())
        .unwrap_or_else(|| "en".to_string())
}

/// The locale directory name of a `resource/locales/<locale>/*.ftl` path (its parent dir name).
fn locale_of(path: &Path) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// One parsed Fluent message: its key, `$variables` (name → used-as-a-number), and value text.
struct FtlMessage {
    key: String,
    params: Params,
    value_text: String,
}

/// Parse a Fluent resource → one [`FtlMessage`] per message, PLUS one per message ATTRIBUTE
/// under the dotted key `message.attr` (how a localized keyboard-shortcut key rides beside
/// its command's label — docs/localization.md; terms/comments/junk ignored; a parse error on
/// an unrelated entry is tolerated — the partial resource is still walked).
fn ftl_messages(src: &str) -> Vec<FtlMessage> {
    use fluent_syntax::ast::Entry;
    let res = match fluent_syntax::parser::parse(src) {
        Ok(r) => r,
        Err((r, _errs)) => r,
    };
    let mut out = Vec::new();
    for entry in &res.body {
        if let Entry::Message(m) = entry {
            let mut params = Params::new();
            let value_text = match &m.value {
                Some(value) => {
                    collect_pattern_vars(value, &mut params, false);
                    pattern_text(value)
                }
                None => String::new(),
            };
            out.push(FtlMessage {
                key: m.id.name.to_string(),
                params,
                value_text,
            });
            for attr in &m.attributes {
                let mut params = Params::new();
                collect_pattern_vars(&attr.value, &mut params, false);
                out.push(FtlMessage {
                    key: format!("{}.{}", m.id.name, attr.id.name),
                    params,
                    value_text: pattern_text(&attr.value),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod span_tests {
    use super::*;

    /// Offsets have to be REAL positions in the source, not a text search: a key named in a
    /// comment above the message would make a search land a line early, and an editor would then
    /// squiggle the comment.
    #[test]
    fn key_offsets_point_at_the_message_not_a_mention_of_it() {
        let src = "# greeting is the one below\ngreeting = Hello\nfarewell = Bye\n";
        let offsets: std::collections::BTreeMap<String, usize> =
            ftl_key_offsets(src).into_iter().collect();

        let greeting = offsets["greeting"];
        assert_eq!(&src[greeting..greeting + "greeting".len()], "greeting");
        assert_eq!(
            line_col(src, greeting),
            (2, 1),
            "the message, not the comment"
        );
        let farewell = offsets["farewell"];
        assert_eq!(line_col(src, farewell), (3, 1));
    }

    /// Attributes carry their own position, so a shortcut label's finding lands on the attribute.
    #[test]
    fn attributes_get_their_own_offset() {
        let src = "open = Open\n    .key = o\n";
        let offsets: std::collections::BTreeMap<String, usize> =
            ftl_key_offsets(src).into_iter().collect();
        assert_eq!(line_col(src, offsets["open"]), (1, 1));
        assert_eq!(
            line_col(src, offsets["open.key"]),
            (2, 6),
            "on the attribute's own line"
        );
    }

    /// A function call is reported where it is written — the whole point of carrying an offset
    /// on `FtlCall` rather than anchoring every option finding to line 1.
    #[test]
    fn function_calls_carry_their_position() {
        let src = "count = You have { NUMBER($n, style: \"decimal\") } left\nother = plain\n";
        let calls = function_calls(src);
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert_eq!(&src[calls[0].offset..calls[0].offset + 6], "NUMBER");
        let (line, col) = line_col(src, calls[0].offset);
        assert_eq!(line, 1);
        assert_eq!(col, 20, "the column the call starts at");
    }

    /// Columns count characters, because that is what an editor means by a column.
    #[test]
    fn columns_count_characters_not_bytes() {
        let src = "gruss = Grüße\nzweite = x\n";
        let at = src.find("zweite").expect("key");
        assert_eq!(line_col(src, at), (2, 1));
        // A multi-byte char earlier on the SAME line must not inflate the column.
        let inner = src.find("ße").expect("inner");
        assert_eq!(line_col(src, inner).1, 12);
    }
}

type Vars = std::collections::BTreeSet<String>;
/// `$variable` name → whether it is used numerically (plural/`select` selector or `NUMBER()` arg).
type Params = std::collections::BTreeMap<String, bool>;

fn collect_pattern_vars(p: &fluent_syntax::ast::Pattern<&str>, out: &mut Params, numeric: bool) {
    use fluent_syntax::ast::PatternElement;
    for el in &p.elements {
        if let PatternElement::Placeable { expression } = el {
            collect_expr_vars(expression, out, numeric);
        }
    }
}

fn collect_expr_vars(e: &fluent_syntax::ast::Expression<&str>, out: &mut Params, numeric: bool) {
    use fluent_syntax::ast::Expression;
    match e {
        Expression::Inline(ie) => collect_inline_vars(ie, out, numeric),
        Expression::Select { selector, variants } => {
            // A plural/number select makes its selector numeric; a string select (`$gender ->
            // [male]…`) does not. Variant bodies are ordinary (non-numeric) context.
            collect_inline_vars(selector, out, is_number_select(variants));
            for v in variants {
                collect_pattern_vars(&v.value, out, false);
            }
        }
    }
}

fn collect_inline_vars(
    ie: &fluent_syntax::ast::InlineExpression<&str>,
    out: &mut Params,
    numeric: bool,
) {
    use fluent_syntax::ast::InlineExpression as X;
    match ie {
        X::VariableReference { id } => {
            *out.entry(id.name.to_string()).or_insert(false) |= numeric;
        }
        X::Placeable { expression } => collect_expr_vars(expression, out, numeric),
        X::FunctionReference { id, arguments } => {
            // The built-in `NUMBER(...)` forces its positional arg numeric; named options don't.
            // `DATETIME(...)` deliberately does NOT: its argument is an ISO-8601 string (or an
            // epoch number the app formats itself), so the generated `res::str` fn keeps the
            // general `IntoFArg` bound (docs/localization.md "Formatted values").
            let num = id.name.eq_ignore_ascii_case("NUMBER");
            for a in &arguments.positional {
                collect_inline_vars(a, out, num);
            }
            for n in &arguments.named {
                collect_inline_vars(&n.value, out, false);
            }
        }
        X::TermReference {
            arguments: Some(arguments),
            ..
        } => {
            for a in &arguments.positional {
                collect_inline_vars(a, out, false);
            }
            for n in &arguments.named {
                collect_inline_vars(&n.value, out, false);
            }
        }
        _ => {}
    }
}

/// One `FUNC(...)` call in a message value — `day lint` validates function names and option
/// values across every locale file with this (the shared fluent-syntax parse, like
/// [`message_keys`]).
/// Byte offset of `part` within `src`, when `part` is a SUBSLICE of it.
///
/// `fluent_syntax::parser::parse` is generic over the slice type and, given a `&str`, hands back
/// an AST whose identifiers and literals borrow straight from the source — so their addresses are
/// positions in it. That is the whole span story: the 0.12 AST carries no explicit spans, and
/// re-finding a key by text search would land on the first comment that mentions it.
/// The byte offset of `part` within `src`, when `part` is a SUBSLICE of it.
///
/// Parsers here hand back `&str` views into the source rather than spans, so the only way to say
/// where a fragment came from is to compare addresses. Returns `None` for a string that merely
/// looks alike but was allocated elsewhere, which is what makes it safe to call on anything.
pub fn offset_in(src: &str, part: &str) -> Option<usize> {
    let (base, at) = (src.as_ptr() as usize, part.as_ptr() as usize);
    (at >= base && at + part.len() <= base + src.len()).then_some(at - base)
}

/// The 1-based line and column of a byte offset, for callers that report positions to a human or
/// an editor. Columns count CHARACTERS rather than bytes, which is what an editor's column means.
pub fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let upto = &src[..offset.min(src.len())];
    let line = upto.matches('\n').count() + 1;
    let col = upto.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (line, col)
}

/// Every message key in a Fluent resource with the byte offset of its identifier — what turns a
/// coverage finding into a diagnostic on the right line rather than on line 1.
pub fn ftl_key_offsets(src: &str) -> Vec<(String, usize)> {
    use fluent_syntax::ast::Entry;
    let res = match fluent_syntax::parser::parse(src) {
        Ok(r) => r,
        Err((r, _errs)) => r,
    };
    let mut out = Vec::new();
    for entry in &res.body {
        if let Entry::Message(m) = entry {
            let at = offset_in(src, m.id.name).unwrap_or(0);
            out.push((m.id.name.to_string(), at));
            for attr in &m.attributes {
                out.push((
                    format!("{}.{}", m.id.name, attr.id.name),
                    offset_in(src, attr.id.name).unwrap_or(at),
                ));
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct FtlCall {
    /// The message key the call appears under.
    pub key: String,
    /// The function name as written (`NUMBER`, `DATETIME`, …).
    pub name: String,
    /// Named options with their literal values (`style: "percent"` → `("style", "percent")`;
    /// non-literal option values are omitted).
    pub named: Vec<(String, String)>,
    /// Byte offset of the function name in the source, so a bad option can be reported where it
    /// is written rather than against the whole file.
    pub offset: usize,
}

/// Every function call in every message of a Fluent resource (parse errors tolerated — the
/// partial resource is walked, matching [`message_keys`]).
pub fn function_calls(src: &str) -> Vec<FtlCall> {
    use fluent_syntax::ast::Entry;
    let res = match fluent_syntax::parser::parse(src) {
        Ok(r) => r,
        Err((r, _errs)) => r,
    };
    let mut out = Vec::new();
    for entry in &res.body {
        if let Entry::Message(m) = entry
            && let Some(value) = &m.value
        {
            collect_pattern_calls(src, value, m.id.name, &mut out);
        }
    }
    out
}

fn collect_pattern_calls(
    src: &str,
    p: &fluent_syntax::ast::Pattern<&str>,
    key: &str,
    out: &mut Vec<FtlCall>,
) {
    use fluent_syntax::ast::PatternElement;
    for el in &p.elements {
        if let PatternElement::Placeable { expression } = el {
            collect_expr_calls(src, expression, key, out);
        }
    }
}

fn collect_expr_calls(
    src: &str,
    e: &fluent_syntax::ast::Expression<&str>,
    key: &str,
    out: &mut Vec<FtlCall>,
) {
    use fluent_syntax::ast::Expression;
    match e {
        Expression::Inline(ie) => collect_inline_calls(src, ie, key, out),
        Expression::Select { selector, variants } => {
            collect_inline_calls(src, selector, key, out);
            for v in variants {
                collect_pattern_calls(src, &v.value, key, out);
            }
        }
    }
}

fn collect_inline_calls(
    src: &str,
    ie: &fluent_syntax::ast::InlineExpression<&str>,
    key: &str,
    out: &mut Vec<FtlCall>,
) {
    use fluent_syntax::ast::InlineExpression as X;
    match ie {
        X::FunctionReference { id, arguments } => {
            let named = arguments
                .named
                .iter()
                .filter_map(|n| {
                    let value = match &n.value {
                        X::StringLiteral { value } => value.to_string(),
                        X::NumberLiteral { value } => value.to_string(),
                        _ => return None,
                    };
                    Some((n.name.name.to_string(), value))
                })
                .collect();
            out.push(FtlCall {
                key: key.to_string(),
                name: id.name.to_string(),
                named,
                offset: offset_in(src, id.name).unwrap_or(0),
            });
            for a in &arguments.positional {
                collect_inline_calls(src, a, key, out);
            }
        }
        X::Placeable { expression } => collect_expr_calls(src, expression, key, out),
        _ => {}
    }
}

/// Whether a `select` is a **plural / number** select (selector is a number) rather than a string
/// select (e.g. `$gender -> [male] [female]`): true if any variant key is a number literal or a CLDR
/// plural category other than the ambiguous `other` (which both plural and string selects use).
fn is_number_select(variants: &[fluent_syntax::ast::Variant<&str>]) -> bool {
    use fluent_syntax::ast::VariantKey;
    const PLURAL: &[&str] = &["zero", "one", "two", "few", "many"];
    variants.iter().any(|v| match &v.key {
        VariantKey::NumberLiteral { .. } => true,
        VariantKey::Identifier { name } => PLURAL.contains(&name.to_ascii_lowercase().as_str()),
    })
}

/// A one-line, human-readable rendering of a message value for the generated doc comment
/// (`Hello, { $name }!`, `{ $count -> … }`), whitespace collapsed. Backticks are stripped so the
/// value can be wrapped in a doc-comment code span.
fn pattern_text(p: &fluent_syntax::ast::Pattern<&str>) -> String {
    use fluent_syntax::ast::PatternElement;
    let mut s = String::new();
    for el in &p.elements {
        match el {
            PatternElement::TextElement { value } => s.push_str(value),
            PatternElement::Placeable { expression } => s.push_str(&placeable_text(expression)),
        }
    }
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('`', "'")
}

fn placeable_text(e: &fluent_syntax::ast::Expression<&str>) -> String {
    use fluent_syntax::ast::{Expression, InlineExpression as X};
    match e {
        Expression::Inline(X::VariableReference { id }) => format!("{{ ${} }}", id.name),
        Expression::Inline(X::StringLiteral { value }) => format!("{{ \"{value}\" }}"),
        Expression::Select {
            selector: X::VariableReference { id },
            ..
        } => format!("{{ ${} -> … }}", id.name),
        _ => "{ … }".to_string(),
    }
}

/// A valid Rust identifier: leading `[A-Za-z_]`, remaining `[A-Za-z0-9_]`, and not the bare `_`.
/// Keyword idents still count as valid — `ident_token` raw-escapes them at render time.
fn is_rust_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && s != "_"
}

/// Render a plan to the `day_resources.rs` source text. This file is `include!`d inside the app's
/// `pub mod res { … }`, so the lint waivers are **outer** attributes on each bucket module (an inner
/// `#![…]` is not valid at an `include!` site) and cover a bucket with no constants (unused `use`).
pub fn render(plan: &ResourcePlan) -> String {
    let mut s = String::new();
    s.push_str("// @generated by day-build — do not edit.\n");
    s.push_str("// Regenerated on every build from resource/{images,assets,fonts,locales}.\n\n");
    // `locales::install()` names `day::install_locales`, so the generated file needs the umbrella
    // crate in scope wherever it is included — the same assumption the other buckets make with
    // `day::ImageName`.
    render_bucket(&mut s, "images", "ImageName", &plan.images);
    render_bucket(&mut s, "vectors", "VectorName", &plan.vectors);
    render_assets(&mut s, &plan.assets);
    render_bucket(&mut s, "fonts", "FontFamily", &plan.fonts);
    render_strings(&mut s, &plan.strings);
    render_locales(&mut s, &plan.locales);
    s
}

/// Render the `locales` bucket: the app's whole Fluent catalog, embedded. `install()` is the
/// one-liner an app's `root()` calls — the source list can't drift from the directory because
/// it IS the directory.
///
/// Paths are absolute because this file is `include!`d from `$OUT_DIR`, so a relative
/// `include_str!` would resolve against `$OUT_DIR` rather than the crate. Several `.ftl` files
/// in one locale directory `concat!` into a single source (Fluent bundles are per-locale, and
/// `day_l10n::install` keys them by tag — two entries for one tag would shadow, not merge).
fn render_locales(s: &mut String, locales: &[LocaleEntry]) {
    s.push_str("#[allow(dead_code)]\npub mod locales {\n");
    s.push_str(&format!(
        "    /// The fallback locale — the one whose strings show when the running locale has no\n\
         \x20   /// translation for a key.\n    pub const DEFAULT: &str = {:?};\n\n",
        default_locale(locales)
    ));
    s.push_str(
        "    /// Every locale under `resource/locales/`, embedded at build time: one\n\
     \x20   /// `(tag, fluent-source)` pair per directory.\n\
     \x20   pub const CATALOG: &[(&str, &str)] = &[\n",
    );
    for l in locales {
        // The FULL path, not `display`'s three-component diagnostic form — and `{:?}` escapes it
        // into a valid Rust literal (Windows separators included).
        let sources: Vec<String> = l
            .sources
            .iter()
            .map(|p| format!("include_str!({:?})", p.display().to_string()))
            .collect();
        // `concat!` of one argument is the identity, so the single-file case stays readable.
        let src = if sources.len() == 1 {
            sources.into_iter().next().unwrap_or_default()
        } else {
            format!("concat!({}, \"\\n\")", sources.join(", \"\\n\", "))
        };
        s.push_str(&format!("        ({:?}, {src}),\n", l.locale));
    }
    s.push_str("    ];\n\n");
    s.push_str(
        "    /// Every bundled locale as `(tag, display name)`, for language pickers. The name\n\
     \x20   /// is the catalog's own `language_name` message (each language naming itself), and\n\
     \x20   /// falls back to the tag when a catalog does not carry one (docs/localization.md).\n\
     \x20   pub const ALL: &[(&str, &str)] = &[\n",
    );
    for l in locales {
        s.push_str(&format!(
            "        ({:?}, {:?}),\n",
            l.locale,
            language_name(l).unwrap_or_else(|| l.locale.clone())
        ));
    }
    s.push_str("    ];\n\n");
    s.push_str(
        "    /// Register [`CATALOG`] under [`DEFAULT`] — call once, before the first localized\n\
     \x20   /// string is read (the top of the app's `root()`). For a different fallback:\n\
     \x20   /// `day::install_locales(\"fr\", res::locales::CATALOG)`.\n\
     \x20   pub fn install() {\n        day::install_locales(DEFAULT, CATALOG);\n    }\n",
    );
    s.push_str("}\n\n");
}

/// A locale's self-name: the value of the `language_name` message in its catalog, read at
/// build time. A line scan, not a Fluent parse — the convention is a single-line literal
/// message (`language_name = Français`), and anything fancier falls back to the tag.
fn language_name(l: &LocaleEntry) -> Option<String> {
    for path in &l.sources {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("language_name") {
                let value = value.trim_start();
                if let Some(value) = value.strip_prefix('=') {
                    let value = value.trim();
                    if !value.is_empty() {
                        return Some(value.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Render the `str` bucket: one `pub fn` per localization key whose signature carries the message's
/// parameters, so `res::str::greeting(name)` == `tr("greeting").arg("name", name)` — checked at
/// compile time (a missing key or wrong arity is an error).
fn render_strings(s: &mut String, entries: &[StrEntry]) {
    s.push_str("#[allow(dead_code, unused_imports, non_snake_case, clippy::too_many_arguments)]\n");
    s.push_str("pub mod str {\n");
    for e in entries {
        // Each param is `impl day::IntoFArg<Mn>` — or `IntoNumberFArg` when the message uses it as a
        // plural/`select` selector (a distinct marker generic per arg). The Rust parameter ident is
        // sanitized while the `.arg("…")` string stays the exact Fluent variable.
        let generics: Vec<String> = (0..e.params.len()).map(|i| format!("M{i}")).collect();
        let sig_params: Vec<String> = e
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let ty = if p.numeric {
                    "IntoNumberFArg"
                } else {
                    "IntoFArg"
                };
                format!(
                    "{}: impl day::{ty}<M{i}>",
                    ident_token(&sanitize_ident(&p.name))
                )
            })
            .collect();
        let generic_list = if generics.is_empty() {
            String::new()
        } else {
            format!("<{}>", generics.join(", "))
        };
        let mut body = format!("day::tr({:?})", e.key);
        for p in &e.params {
            body.push_str(&format!(
                ".arg({:?}, {})",
                p.name,
                ident_token(&sanitize_ident(&p.name))
            ));
        }
        // Doc shows the key + the reference-locale value, so IDE hover reveals the actual text.
        let doc = if e.doc.is_empty() {
            format!("`{}`", e.key)
        } else {
            format!("`{}` — `{}`", e.key, e.doc)
        };
        s.push_str(&format!(
            "    /// {doc}\n    pub fn {}{generic_list}({}) -> day::LocalizedText {{ {body} }}\n",
            ident_token(&e.key.replace('.', "_")),
            sig_params.join(", "),
        ));
    }
    s.push_str("}\n\n");
}

/// Render the assets TREE (§18.5): one module per directory, an `AssetDir` const beside each
/// nested module (same name — consts and modules live in different namespaces), an `AssetName`
/// const per file. The root module is `assets`, matching the flat form apps already compile
/// against for top-level files.
fn render_assets(s: &mut String, root: &AssetNode) {
    s.push_str("#[allow(non_upper_case_globals, dead_code, unused_imports)]\n");
    render_asset_node(s, "assets", root, 0);
    s.push('\n');
}

fn render_asset_node(s: &mut String, module: &str, node: &AssetNode, depth: usize) {
    let pad = "    ".repeat(depth);
    s.push_str(&format!("{pad}pub mod {} {{\n", ident_token(module)));
    s.push_str(&format!("{pad}    use day::{{AssetDir, AssetName}};\n"));
    for e in &node.files {
        s.push_str(&format!(
            "{pad}    /// `{}`\n{pad}    pub const {}: AssetName = AssetName::from_static({:?});\n",
            e.source,
            ident_token(&e.symbol),
            e.value,
        ));
    }
    for (sym, sub) in &node.dirs {
        s.push_str(&format!(
            "{pad}    /// `resource/assets/{}` (directory)\n{pad}    pub const {}: AssetDir = AssetDir::from_static({:?});\n",
            sub.path,
            ident_token(sym),
            sub.path,
        ));
        render_asset_node(s, sym, sub, depth + 1);
    }
    s.push_str(&format!("{pad}}}\n"));
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
        assert_eq!(plan.assets.files[0].symbol, "numbers_bin");
        assert_eq!(plan.assets.files[0].value, "numbers.bin");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn asset_tree_nests_modules_and_dir_consts() {
        let root = tmp("assets-tree");
        touch(&root.join("resource/assets"), "top.bin", b"x");
        touch(
            &root.join("resource/assets/web/minisite"),
            "index.html",
            b"x",
        );
        touch(
            &root.join("resource/assets/web/minisite/css"),
            "style.css",
            b"x",
        );
        let plan = plan_resources(&root).unwrap();
        // Values are `/`-relative paths; symbols come from the leaf name alone.
        let web = &plan.assets.dirs[0];
        assert_eq!(web.0, "web");
        let mini = &web.1.dirs[0];
        assert_eq!(mini.1.path, "web/minisite");
        assert_eq!(mini.1.files[0].value, "web/minisite/index.html");
        assert_eq!(
            mini.1.dirs[0].1.files[0].value,
            "web/minisite/css/style.css"
        );
        let code = render(&plan);
        assert!(
            code.contains("pub const top_bin: AssetName = AssetName::from_static(\"top.bin\");")
        );
        assert!(code.contains("pub const web: AssetDir = AssetDir::from_static(\"web\");"));
        assert!(code.contains("pub mod web {"));
        assert!(
            code.contains(
                "pub const minisite: AssetDir = AssetDir::from_static(\"web/minisite\");"
            )
        );
        assert!(code.contains(
            "pub const index_html: AssetName = AssetName::from_static(\"web/minisite/index.html\");"
        ));
        assert!(code.contains(
            "pub const style_css: AssetName = AssetName::from_static(\"web/minisite/css/style.css\");"
        ));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn asset_file_and_dir_symbol_collision_errors() {
        // A file and a directory cannot share a literal name on disk, but their SYMBOLS can
        // collide after sanitization: `site.old` (file) and `site-old/` (dir) both map to
        // `site_old`, and both land in the same module's const namespace.
        let root = tmp("assets-collide");
        touch(&root.join("resource/assets"), "site.old", b"x");
        touch(&root.join("resource/assets/site-old"), "x.bin", b"x");
        let err = plan_resources(&root).unwrap_err();
        assert!(err.contains("site_old"), "{err}");
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
            ..Default::default()
        };
        let code = render(&plan);
        assert!(code.contains("#[allow(non_upper_case_globals, dead_code, unused_imports)]"));
        assert!(code.contains("pub mod images {"));
        assert!(code.contains("use day::ImageName;"));
        assert!(
            code.contains(
                "pub const nav_system: ImageName = ImageName::from_static(\"nav_system\");"
            )
        );
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
        assert!(plan.images.is_empty() && plan.fonts.is_empty());
        assert!(plan.assets.files.is_empty() && plan.assets.dirs.is_empty());
        assert!(plan.strings.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    fn ftl(root: &Path, locale: &str, body: &str) {
        let dir = root.join("resource/locales").join(locale);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("app.ftl"), body).unwrap();
    }

    fn entry<'a>(plan: &'a ResourcePlan, key: &str) -> &'a StrEntry {
        plan.strings
            .iter()
            .find(|e| e.key == key)
            .expect("key present")
    }
    fn names(e: &StrEntry) -> Vec<&str> {
        e.params.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn extracts_keys_params_numeric_and_doc() {
        let root = tmp("str-extract");
        // `counter_value` uses $count in a plural select (multiline) — same variable SET as a flat
        // value, and numeric (a plural selector); `greeting` has one non-numeric param; `nav_home`
        // has none. The doc captures the reference-locale value text (#5).
        ftl(
            &root,
            "en",
            "nav_home = Home\n\
             greeting = Hello, { $name }!\n\
             counter_value = { $count ->\n    [one] { $count } click\n   *[other] { $count } clicks\n}\n",
        );
        let plan = plan_resources(&root).unwrap();
        assert!(names(entry(&plan, "nav_home")).is_empty());
        assert_eq!(names(entry(&plan, "greeting")), vec!["name"]);
        assert_eq!(entry(&plan, "greeting").doc, "Hello, { $name }!"); // #5
        assert!(!entry(&plan, "greeting").params[0].numeric);
        // #2: a plural-select selector is typed numeric.
        assert_eq!(names(entry(&plan, "counter_value")), vec!["count"]);
        assert!(entry(&plan, "counter_value").params[0].numeric);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn string_select_selector_is_not_numeric() {
        let root = tmp("str-gender");
        // A `select` on a string (gender) must NOT force its selector numeric.
        ftl(
            &root,
            "en",
            "hi = { $gender ->\n    [male] Mr\n    [female] Ms\n   *[other] Mx\n} { $name }\n",
        );
        let plan = plan_resources(&root).unwrap();
        let g = entry(&plan, "hi");
        assert!(
            !g.params
                .iter()
                .find(|p| p.name == "gender")
                .unwrap()
                .numeric
        );
        assert!(!g.params.iter().find(|p| p.name == "name").unwrap().numeric);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn numeric_is_ored_across_locales() {
        let root = tmp("str-numeric-or");
        // `en` uses $count as a plural selector (numeric); `zh` uses it as a flat interpolation.
        // The param must be numeric because SOME locale needs a number.
        ftl(
            &root,
            "en",
            "n = { $count ->\n    [one] one\n   *[other] many\n}\n",
        );
        ftl(&root, "zh", "n = { $count } times\n");
        let plan = plan_resources(&root).unwrap();
        assert!(entry(&plan, "n").params[0].numeric);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn message_keys_lists_message_ids_only() {
        // Public parser shared with `day lint`: messages only (terms/comments excluded).
        let keys = message_keys("a = x\n# comment\n-term = y\nb = { $v }\n");
        assert_eq!(keys, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn kebab_key_is_rejected() {
        let root = tmp("str-kebab");
        ftl(&root, "en", "nav-home = Home\n");
        let err = plan_resources(&root).unwrap_err();
        assert!(err.contains("not a valid Rust identifier"), "{err}");
        assert!(err.contains("nav_home"), "{err}"); // suggests the fix
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cross_locale_param_disagreement_is_rejected() {
        let root = tmp("str-params");
        ftl(&root, "en", "greeting = Hello, { $name }!\n");
        ftl(&root, "fr", "greeting = Bonjour, { $nom }!\n");
        let err = plan_resources(&root).unwrap_err();
        assert!(err.contains("different parameters"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn renders_param_typed_functions() {
        let p = |name: &str, numeric: bool| StrParam {
            name: name.into(),
            numeric,
        };
        let plan = ResourcePlan {
            strings: vec![
                StrEntry {
                    key: "hello_world".into(),
                    params: vec![],
                    doc: "Hello!".into(),
                },
                StrEntry {
                    key: "counter_value".into(),
                    params: vec![p("count", true)], // numeric plural → IntoNumberFArg
                    doc: "{ $count -> … }".into(),
                },
                StrEntry {
                    key: "deviceinfo_system".into(),
                    params: vec![p("name", false), p("version", false)],
                    doc: String::new(),
                },
            ],
            ..Default::default()
        };
        let code = render(&plan);
        assert!(code.contains("pub mod str {"));
        assert!(code.contains("/// `hello_world` — `Hello!`")); // #5: doc shows the value
        assert!(
            code.contains(
                "pub fn hello_world() -> day::LocalizedText { day::tr(\"hello_world\") }"
            )
        );
        // #2: a numeric param is `IntoNumberFArg`; non-numeric stays `IntoFArg`.
        assert!(code.contains(
            "pub fn counter_value<M0>(count: impl day::IntoNumberFArg<M0>) -> day::LocalizedText { day::tr(\"counter_value\").arg(\"count\", count) }"
        ));
        assert!(code.contains(
            "pub fn deviceinfo_system<M0, M1>(name: impl day::IntoFArg<M0>, version: impl day::IntoFArg<M1>) -> day::LocalizedText { day::tr(\"deviceinfo_system\").arg(\"name\", name).arg(\"version\", version) }"
        ));
    }

    // ---- Attributes: localized shortcut keys ride beside their command's label ----

    #[test]
    fn message_attributes_generate_dotted_tr_accessors() {
        let root = tmp("attr-accessors");
        ftl(&root, "en", "menu_group = Group\n    .key = g\n");
        // fr omits `.key` — the runtime falls back to the default locale's; the codegen
        // still emits ONE accessor from the union.
        ftl(&root, "fr", "menu_group = Grouper\n");
        let entries = plan_strings(&root.join("resource/locales")).expect("plan");
        let plan = ResourcePlan {
            strings: entries,
            ..Default::default()
        };
        let code = render(&plan);
        assert!(
            code.contains("pub fn menu_group() -> day::LocalizedText { day::tr(\"menu_group\") }")
        );
        assert!(
            code.contains(
                "pub fn menu_group_key() -> day::LocalizedText { day::tr(\"menu_group.key\") }"
            ),
            "{code}"
        );
    }

    #[test]
    fn an_attribute_colliding_with_a_message_fn_name_is_a_build_error() {
        let root = tmp("attr-collision");
        ftl(
            &root,
            "en",
            "menu_group = Group\n    .key = g\nmenu_group_key = Shadow\n",
        );
        let err = plan_strings(&root.join("resource/locales")).expect_err("must collide");
        assert!(err.contains("menu_group_key"), "{err}");
    }

    // ---- The `locales` catalog: the app's whole language list, discovered not declared ----

    #[test]
    fn locales_are_discovered_and_sorted() {
        let root = tmp("locales-discover");
        ftl(&root, "en", "hello = Hello");
        ftl(&root, "fr", "hello = Bonjour");
        ftl(&root, "zh-CN", "hello = 你好");
        let plan = plan_resources(&root).unwrap();
        let tags: Vec<_> = plan.locales.iter().map(|l| l.locale.as_str()).collect();
        assert_eq!(tags, vec!["en", "fr", "zh-CN"]); // sorted → deterministic output
        assert!(plan.locales.iter().all(|l| l.sources.len() == 1));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn locale_catalog_renders_embedded_sources() {
        let root = tmp("locales-render");
        ftl(&root, "en", "hello = Hello");
        ftl(&root, "fr", "hello = Bonjour");
        let plan = plan_resources(&root).unwrap();
        let code = render(&plan);
        assert!(code.contains("pub mod locales {"));
        assert!(code.contains("pub const DEFAULT: &str = \"en\";"));
        assert!(code.contains("pub const CATALOG: &[(&str, &str)] = &["));
        // Absolute paths: the generated file is `include!`d from $OUT_DIR, so a relative
        // `include_str!` would resolve against the wrong directory. Compare against the plan's
        // own source path, not a re-`join`ed one: on Windows, discovery separates components
        // with `\` where a joined `"a/b"` literal keeps its `/`, so the strings differ even
        // when the paths agree. `ends_with` compares components, so it holds on both.
        let en = &plan
            .locales
            .iter()
            .find(|l| l.locale == "en")
            .unwrap()
            .sources[0];
        assert!(
            en.is_absolute() && en.ends_with("en/app.ftl"),
            "{}",
            en.display()
        );
        assert!(
            code.contains(&format!(
                "(\"en\", include_str!({:?}))",
                en.display().to_string()
            )),
            "{code}"
        );
        assert!(code.contains("day::install_locales(DEFAULT, CATALOG);"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn several_ftl_files_in_one_locale_concatenate() {
        let root = tmp("locales-multifile");
        ftl(&root, "en", "hello = Hello"); // app.ftl
        let dir = root.join("resource/locales/en");
        std::fs::write(dir.join("errors.ftl"), "oops = Oops").unwrap();
        let plan = plan_resources(&root).unwrap();
        assert_eq!(plan.locales.len(), 1, "one bundle per locale, not per file");
        assert_eq!(plan.locales[0].sources.len(), 2);
        // Both keys are still generated, and the sources join into ONE catalog entry (a second
        // entry for the same tag would shadow the first in day_l10n's per-locale bundle map).
        let keys: Vec<_> = plan.strings.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, vec!["hello", "oops"]);
        let code = render(&plan);
        // The two files concatenate into a single `("en", concat!(…))` CATALOG entry. Match the
        // concat!-tagged form specifically: the ALL language-picker array carries its own
        // `("en", "en")` pair, so a bare `("en", ` count would see both.
        assert_eq!(code.matches("(\"en\", concat!(").count(), 1);
        assert!(code.contains("concat!(include_str!("), "{code}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn default_locale_prefers_en_then_first() {
        let root = tmp("locales-default-en");
        ftl(&root, "fr", "hello = Bonjour");
        ftl(&root, "en", "hello = Hello");
        assert_eq!(
            default_locale(&plan_resources(&root).unwrap().locales),
            "en"
        );
        std::fs::remove_dir_all(&root).ok();

        // No English: the first tag alphabetically, so the pick is deterministic.
        let root = tmp("locales-default-noen");
        ftl(&root, "fr", "hello = Bonjour");
        ftl(&root, "ar", "hello = مرحبا");
        assert_eq!(
            default_locale(&plan_resources(&root).unwrap().locales),
            "ar"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_locales_yields_an_empty_catalog() {
        // An app with no `resource/locales/` still compiles: `install()` registers nothing and
        // day-l10n's built-in core catalog keeps answering framework keys.
        let root = tmp("locales-none");
        touch(&root.join("resource/images"), "logo.png", b"x");
        let plan = plan_resources(&root).unwrap();
        assert!(plan.locales.is_empty());
        let code = render(&plan);
        assert!(code.contains("pub const DEFAULT: &str = \"en\";"));
        assert!(code.contains("pub const CATALOG: &[(&str, &str)] = &[\n    ];"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn stray_ftl_outside_a_locale_dir_is_ignored() {
        // `resource/locales/loose.ftl` names no language — it must not become a `locales` locale.
        let root = tmp("locales-stray");
        ftl(&root, "en", "hello = Hello");
        std::fs::write(root.join("resource/locales/loose.ftl"), "stray = Stray").unwrap();
        let plan = plan_resources(&root).unwrap();
        let tags: Vec<_> = plan.locales.iter().map(|l| l.locale.as_str()).collect();
        assert_eq!(tags, vec!["en"]);
        std::fs::remove_dir_all(&root).ok();
    }
}
