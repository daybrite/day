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

/// The full set of constants to emit, grouped by bucket.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ResourcePlan {
    pub images: Vec<Entry>,
    pub assets: Vec<Entry>,
    pub fonts: Vec<Entry>,
    /// Localization keys → `res::str::<key>(params…)` functions (§18.5).
    pub strings: Vec<StrEntry>,
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
    for bucket in ["images", "assets", "fonts", "locales"] {
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
        strings: plan_strings(&root.join("resource/locales"))?,
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

/// The message keys defined in a Fluent source (terms/attributes/comments ignored). Public so the
/// CLI lint (`day lint` fluent coverage) shares this one `fluent-syntax` parser with the codegen and
/// the runtime resolver, instead of a hand-rolled line scanner.
pub fn message_keys(ftl_src: &str) -> Vec<String> {
    ftl_messages(ftl_src).into_iter().map(|m| m.key).collect()
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
            if !is_rust_ident(&msg.key) {
                return Err(format!(
                    "day-build: localization key {:?} ({loc}) is not a valid Rust identifier — \
                     rename it to snake_case (e.g. `{}`) in every resource/locales/*/*.ftl (Fluent \
                     allows `-`, Rust identifiers do not).",
                    msg.key,
                    msg.key.replace('-', "_")
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

/// Parse a Fluent resource → one [`FtlMessage`] per message (terms/attributes/comments/junk ignored;
/// a parse error on an unrelated entry is tolerated — the partial resource is still walked).
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
        }
    }
    out
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
#[derive(Debug, Clone, PartialEq)]
pub struct FtlCall {
    /// The message key the call appears under.
    pub key: String,
    /// The function name as written (`NUMBER`, `DATETIME`, …).
    pub name: String,
    /// Named options with their literal values (`style: "percent"` → `("style", "percent")`;
    /// non-literal option values are omitted).
    pub named: Vec<(String, String)>,
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
            collect_pattern_calls(value, m.id.name, &mut out);
        }
    }
    out
}

fn collect_pattern_calls(p: &fluent_syntax::ast::Pattern<&str>, key: &str, out: &mut Vec<FtlCall>) {
    use fluent_syntax::ast::PatternElement;
    for el in &p.elements {
        if let PatternElement::Placeable { expression } = el {
            collect_expr_calls(expression, key, out);
        }
    }
}

fn collect_expr_calls(e: &fluent_syntax::ast::Expression<&str>, key: &str, out: &mut Vec<FtlCall>) {
    use fluent_syntax::ast::Expression;
    match e {
        Expression::Inline(ie) => collect_inline_calls(ie, key, out),
        Expression::Select { selector, variants } => {
            collect_inline_calls(selector, key, out);
            for v in variants {
                collect_pattern_calls(&v.value, key, out);
            }
        }
    }
}

fn collect_inline_calls(
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
            });
            for a in &arguments.positional {
                collect_inline_calls(a, key, out);
            }
        }
        X::Placeable { expression } => collect_expr_calls(expression, key, out),
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
    render_bucket(&mut s, "images", "ImageName", &plan.images);
    render_bucket(&mut s, "assets", "AssetName", &plan.assets);
    render_bucket(&mut s, "fonts", "FontFamily", &plan.fonts);
    render_strings(&mut s, &plan.strings);
    s
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
            ident_token(&e.key),
            sig_params.join(", "),
        ));
    }
    s.push_str("}\n\n");
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
        assert!(plan.images.is_empty() && plan.assets.is_empty() && plan.fonts.is_empty());
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
}
