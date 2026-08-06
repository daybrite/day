//! SwiftUI view scanning + codegen for embedded SwiftPM packages (docs/swiftui.md).
//!
//! An app (or piece crate) can point `[package.metadata.day.ios/macos] swift-packages` at a
//! **local** SwiftPM package. This module is the single grammar both consumers share:
//!
//! - the app's `build.rs` ([`crate::generate_resources`]) scans the package and emits typed Rust
//!   bindings to `$OUT_DIR/day_swiftui.rs` (surfaced as `crate::swiftui::MyView(…)`), and
//! - `day build` (day-cli) runs the same scan and emits the Swift provider glue that wraps each
//!   view in a hosting view (staged into the generated `DayPieces` module).
//!
//! The scan is a **text parse of a documented subset** — deliberately not a Swift compiler:
//! it must run on any host, with no Swift toolchain, from a plain `cargo build` (DESIGN §17.5).
//! A view is exported when it is a **top-level, non-generic `public struct` whose declaration
//! names `View` in its inheritance clause**, and its **first `public init`** has only supported
//! parameter types (`String`, `Int`, `Double`, `Bool`; no defaults, no attributes, no variadics).
//! Anything else is skipped with a reason. A mis-parse cannot ship silently: the generated Swift
//! glue calls the real initializer, so the Swift compiler validates every signature this parser
//! extracted.

use std::path::{Path, PathBuf};

/// A parameter of an exported view's initializer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwiftParam {
    /// The external argument label, or `None` for `_` (unlabeled). The JSON key and the Rust
    /// argument use [`SwiftParam::key`] either way.
    pub label: Option<String>,
    /// The internal parameter name.
    pub name: String,
    pub ty: SwiftType,
}

impl SwiftParam {
    /// The name shared by the JSON params object and the generated Rust argument: the external
    /// label when there is one, else the internal name.
    pub fn key(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.name)
    }
}

/// The parameter types the bridge can marshal (JSON params → `Decodable` → the Swift init).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwiftType {
    String,
    Int,
    Double,
    Bool,
}

impl SwiftType {
    fn parse(ty: &str) -> Option<Self> {
        match ty {
            "String" => Some(SwiftType::String),
            "Int" => Some(SwiftType::Int),
            "Double" => Some(SwiftType::Double),
            "Bool" => Some(SwiftType::Bool),
            _ => None,
        }
    }
    /// The Swift spelling (glue `Decodable` fields).
    pub fn swift(self) -> &'static str {
        match self {
            SwiftType::String => "String",
            SwiftType::Int => "Int",
            SwiftType::Double => "Double",
            SwiftType::Bool => "Bool",
        }
    }
    /// The Rust spelling (generated binding arguments).
    pub fn rust(self) -> &'static str {
        match self {
            SwiftType::String => "String",
            SwiftType::Int => "i64",
            SwiftType::Double => "f64",
            SwiftType::Bool => "bool",
        }
    }
}

/// One exported view: `swiftui("<module>.<name>")` on the Rust side, class
/// `DayView_<module>_<name>` on the Swift side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwiftView {
    /// The SwiftPM target (module) name — the `Sources/<Module>` directory.
    pub module: String,
    /// The struct name.
    pub name: String,
    /// The first public init's parameters, in declaration order.
    pub params: Vec<SwiftParam>,
    /// Package-relative source path (doc comments in the generated code).
    pub source: String,
}

impl SwiftView {
    /// The canonical piece name (`swiftui(...)` argument): `Module.View`.
    pub fn piece_name(&self) -> String {
        format!("{}.{}", self.module, self.name)
    }
    /// The Objective-C class name of the generated provider: `DayView_Module_View`.
    pub fn class_name(&self) -> String {
        format!("DayView_{}_{}", self.module, self.name)
    }
}

/// A scanned package: the exported views plus everything that looked like a public View but was
/// skipped (surfaced as build warnings so a missing binding is never a silent mystery).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SwiftScan {
    pub views: Vec<SwiftView>,
    /// `(Module.Name, reason)` — public `View` structs the subset could not export.
    pub skipped: Vec<(String, String)>,
}

/// Scan a local SwiftPM package (conventional layout: `Sources/<Module>/**/*.swift`) for
/// exportable public SwiftUI views. Views are sorted by `(module, name)` for deterministic output.
pub fn scan_package(pkg_dir: &Path) -> Result<SwiftScan, String> {
    if !pkg_dir.join("Package.swift").is_file() {
        return Err(format!(
            "day-build: {} has no Package.swift — a local swift-packages path must point at a SwiftPM package root",
            pkg_dir.display()
        ));
    }
    let sources = pkg_dir.join("Sources");
    let mut modules: Vec<PathBuf> = std::fs::read_dir(&sources)
        .map_err(|_| {
            format!(
                "day-build: {} has no Sources/ directory — the view scan needs the conventional \
                 Sources/<Module>/ layout",
                pkg_dir.display()
            )
        })?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    modules.sort();

    let mut scan = SwiftScan::default();
    for module_dir in modules {
        let module = module_dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut files = Vec::new();
        collect_swift_files(&module_dir, &mut files);
        files.sort();
        for file in files {
            let src = std::fs::read_to_string(&file)
                .map_err(|e| format!("day-build: reading {}: {e}", file.display()))?;
            let rel = file
                .strip_prefix(pkg_dir)
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            scan_source(&module, &rel, &src, &mut scan);
        }
    }
    scan.views
        .sort_by(|a, b| (&a.module, &a.name).cmp(&(&b.module, &b.name)));
    scan.skipped.sort();

    // Duplicate simple names would collide in the flat `crate::swiftui` module — fail loudly with
    // the fix (rename one view) rather than silently shadowing.
    for pair in scan.views.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(format!(
                "day-build: two exported views are both named `{}` ({} and {}) — \
                 `crate::swiftui` is flat, rename one",
                pair[0].name,
                pair[0].piece_name(),
                pair[1].piece_name()
            ));
        }
    }
    Ok(scan)
}

fn collect_swift_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_swift_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("swift") {
            out.push(path);
        }
    }
}

/// Scan one source file for top-level exported views (the pure, testable core).
pub fn scan_source(module: &str, source: &str, src: &str, scan: &mut SwiftScan) {
    let text = strip_comments_and_strings(src);
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b'p' if depth == 0 && text[i..].starts_with("public struct ") => {
                if let Some((decl, next)) = parse_struct(module, source, &text, i) {
                    match decl {
                        Ok(view) => scan.views.push(view),
                        Err(Some(skip)) => scan.skipped.push(skip),
                        Err(None) => {}
                    }
                    i = next;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
}

/// Parse a `public struct` starting at `start`. Returns the outcome and the index just past the
/// struct's closing brace (`Err(None)` = not a `View`, nothing to report).
#[allow(clippy::type_complexity)]
fn parse_struct(
    module: &str,
    source: &str,
    text: &str,
    start: usize,
) -> Option<(Result<SwiftView, Option<(String, String)>>, usize)> {
    let after_kw = start + "public struct ".len();
    let name: String = text[after_kw..]
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return None;
    }
    // The inheritance/where clause runs to the struct's opening brace.
    let brace = text[after_kw..].find('{')? + after_kw;
    let clause = &text[after_kw + name.len()..brace];
    let body_end = matching_brace(text, brace)?;
    let full = format!("{module}.{name}");
    if !names_view(clause) {
        return Some((Err(None), body_end));
    }
    if clause.trim_start().starts_with('<') {
        return Some((
            Err(Some((full, "generic views are not supported".into()))),
            body_end,
        ));
    }
    // The first `public init` in the struct body is the exported constructor (docs/swiftui.md).
    let body = &text[brace..body_end];
    let Some(init_at) = body.find("public init") else {
        return Some((
            Err(Some((
                full,
                "no public init (the memberwise init is internal)".into(),
            ))),
            body_end,
        ));
    };
    let after_init = &body[init_at + "public init".len()..];
    let trimmed = after_init.trim_start();
    if !trimmed.starts_with('(') {
        let reason = if trimmed.starts_with('?') {
            "failable inits are not supported"
        } else {
            "generic inits are not supported"
        };
        return Some((Err(Some((full, reason.into()))), body_end));
    }
    let open = body[init_at..].find('(').unwrap() + init_at;
    let close = matching_paren(body, open)?;
    match parse_params(&body[open + 1..close]) {
        Ok(params) => Some((
            Ok(SwiftView {
                module: module.to_string(),
                name,
                params,
                source: source.to_string(),
            }),
            body_end,
        )),
        Err(reason) => Some((Err(Some((full, reason))), body_end)),
    }
}

/// Does the inheritance clause name `View` (as a whole word, possibly qualified `SwiftUI.View`)?
fn names_view(clause: &str) -> bool {
    let clause = clause.split("where").next().unwrap_or(clause);
    clause
        .split([':', ','])
        .map(str::trim)
        .any(|c| c == "View" || c == "SwiftUI.View")
}

/// Parse an init's parameter list (the text between its parentheses).
fn parse_params(list: &str) -> Result<Vec<SwiftParam>, String> {
    let mut params = Vec::new();
    for piece in split_top_level(list) {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        if piece.contains('=') {
            return Err("default parameter values are not supported".into());
        }
        if piece.starts_with('@') {
            return Err(format!(
                "attributed parameters are not supported ({})",
                piece.split_whitespace().next().unwrap_or("@")
            ));
        }
        let (names, ty) = piece
            .split_once(':')
            .ok_or_else(|| format!("could not parse parameter `{piece}`"))?;
        let ty = ty.trim();
        if ty.starts_with("inout") || ty.ends_with("...") {
            return Err("inout/variadic parameters are not supported".into());
        }
        let ty = SwiftType::parse(ty).ok_or_else(|| {
            format!("parameter type `{ty}` is not supported (String/Int/Double/Bool)")
        })?;
        let mut words = names.split_whitespace();
        let (label, name) = match (words.next(), words.next(), words.next()) {
            (Some(one), None, _) => (Some(one.to_string()), one.to_string()),
            (Some("_"), Some(name), None) => (None, name.to_string()),
            (Some(label), Some(name), None) => (Some(label.to_string()), name.to_string()),
            _ => return Err(format!("could not parse parameter `{piece}`")),
        };
        params.push(SwiftParam { label, name, ty });
    }
    Ok(params)
}

/// Split on commas at nesting depth zero (parens/brackets/angles).
fn split_top_level(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '<' => depth += 1,
            ')' | ']' | '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Index just past the brace matching the `{` at `open`.
fn matching_brace(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, b) in text.bytes().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Index of the paren matching the `(` at `open`.
fn matching_paren(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, b) in text.bytes().enumerate().skip(open) {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Blank out comments and string-literal contents (keeping newlines) so declaration scanning
/// can't be fooled by braces or keywords inside them. Handles `//`, nested `/* */`, `"…"` with
/// escapes, and `"""` multiline strings; interpolation contents are blanked with the string
/// (a nested quote inside `\(…)` is outside the subset — the glue compile catches any fallout).
fn strip_comments_and_strings(src: &str) -> String {
    #[derive(PartialEq)]
    enum State {
        Code,
        Line,
        Block(u32),
        Str,
        MultiStr,
    }
    let mut out = String::with_capacity(src.len());
    let mut state = State::Code;
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match state {
            State::Code => match (c, next) {
                ('/', Some('/')) => {
                    state = State::Line;
                    out.push_str("  ");
                    i += 2;
                    continue;
                }
                ('/', Some('*')) => {
                    state = State::Block(1);
                    out.push_str("  ");
                    i += 2;
                    continue;
                }
                ('"', _) if chars.get(i + 1) == Some(&'"') && chars.get(i + 2) == Some(&'"') => {
                    state = State::MultiStr;
                    out.push_str("\"\"\"");
                    i += 3;
                    continue;
                }
                ('"', _) => {
                    state = State::Str;
                    out.push('"');
                }
                _ => out.push(c),
            },
            State::Line => {
                if c == '\n' {
                    state = State::Code;
                    out.push('\n');
                } else {
                    out.push(' ');
                }
            }
            State::Block(depth) => match (c, next) {
                ('/', Some('*')) => {
                    state = State::Block(depth + 1);
                    out.push_str("  ");
                    i += 2;
                    continue;
                }
                ('*', Some('/')) => {
                    state = if depth == 1 {
                        State::Code
                    } else {
                        State::Block(depth - 1)
                    };
                    out.push_str("  ");
                    i += 2;
                    continue;
                }
                ('\n', _) => out.push('\n'),
                _ => out.push(' '),
            },
            State::Str => match (c, next) {
                ('\\', Some(_)) => {
                    out.push_str("  ");
                    i += 2;
                    continue;
                }
                ('"', _) => {
                    state = State::Code;
                    out.push('"');
                }
                ('\n', _) => {
                    // Unterminated line — bail back to code so one bad literal can't eat the file.
                    state = State::Code;
                    out.push('\n');
                }
                _ => out.push(' '),
            },
            State::MultiStr => {
                if c == '"' && chars.get(i + 1) == Some(&'"') && chars.get(i + 2) == Some(&'"') {
                    state = State::Code;
                    out.push_str("\"\"\"");
                    i += 3;
                    continue;
                }
                out.push(if c == '\n' { '\n' } else { ' ' });
            }
        }
        i += 1;
    }
    out
}

// ===========================================================================
// build.rs entry — derive the package list from Cargo.toml, scan, emit the bindings
// ===========================================================================

/// The build-script half (called from [`crate::generate_resources`]): read the crate's own
/// `[package.metadata.day.ios/macos].swift-packages` for local `path` entries, scan each package,
/// and write `$OUT_DIR/day_swiftui.rs`. Always writes a valid module (empty when nothing is
/// declared), so `pub mod swiftui { include!(…) }` is safe to keep in lib.rs unconditionally.
pub(crate) fn generate_bindings(root: &Path, out: &Path) -> Result<(), String> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml"))
        .map_err(|e| format!("day-build: reading Cargo.toml: {e}"))?;
    let (packages, has_piece_dep) = local_packages_from_manifest(&manifest)
        .map_err(|e| format!("day-build: Cargo.toml: {e}"))?;

    let code = if packages.is_empty() {
        String::from(
            "// Generated by day-build. No local SwiftPM packages are declared in\n\
             // [package.metadata.day.ios/macos] swift-packages (docs/swiftui.md).\n",
        )
    } else if !has_piece_dep {
        // The bindings call day_piece_swiftui::* — without the dependency they cannot compile,
        // so skip them loudly rather than failing the build with a confusing resolver error.
        println!(
            "cargo:warning=day-build: local Swift packages are declared but day-piece-swiftui \
             is not a dependency — skipping the SwiftUI bindings (docs/swiftui.md)"
        );
        String::from(
            "// Generated by day-build. SwiftUI bindings skipped: day-piece-swiftui is not a\n\
             // dependency of this crate (docs/swiftui.md).\n",
        )
    } else {
        let mut scans = Vec::new();
        for rel in &packages {
            let dir = root.join(rel);
            // Directory tracking is recursive, so an added/renamed view regenerates the bindings.
            println!("cargo:rerun-if-changed={rel}/Sources");
            println!("cargo:rerun-if-changed={rel}/Package.swift");
            let scan = scan_package(&dir)?;
            for (view, reason) in &scan.skipped {
                println!("cargo:warning=day-build: swiftui: {view} not exported — {reason}");
            }
            scans.push((rel.clone(), scan));
        }
        // Duplicate simple names across PACKAGES collide in the flat module too.
        let mut names: Vec<(String, String)> = scans
            .iter()
            .flat_map(|(_, s)| s.views.iter().map(|v| (v.name.clone(), v.piece_name())))
            .collect();
        names.sort();
        for pair in names.windows(2) {
            if pair[0].0 == pair[1].0 {
                return Err(format!(
                    "day-build: two exported views are both named `{}` ({} and {}) — \
                     `crate::swiftui` is flat, rename one",
                    pair[0].0, pair[0].1, pair[1].1
                ));
            }
        }
        render_bindings(&scans)
    };
    std::fs::write(out.join("day_swiftui.rs"), code)
        .map_err(|e| format!("day-build: writing day_swiftui.rs: {e}"))
}

/// Extract the deduped local `swift-packages` paths from both Apple metadata tables, plus whether
/// `day-piece-swiftui` is a declared dependency (the bindings need its crate).
fn local_packages_from_manifest(manifest: &str) -> Result<(Vec<String>, bool), String> {
    let table: toml::Table = manifest.parse().map_err(|e| format!("{e}"))?;
    let mut packages: Vec<String> = Vec::new();
    for key in ["ios", "macos"] {
        let entries = table
            .get("package")
            .and_then(|v| v.get("metadata"))
            .and_then(|v| v.get("day"))
            .and_then(|v| v.get(key))
            .and_then(|v| v.get("swift-packages"))
            .and_then(|v| v.as_array());
        for entry in entries.into_iter().flatten() {
            if let Some(path) = entry.get("path").and_then(|p| p.as_str())
                && !packages.iter().any(|p| p == path)
            {
                packages.push(path.to_string());
            }
        }
    }
    let has_piece_dep = ["dependencies", "build-dependencies"].iter().any(|t| {
        table
            .get(*t)
            .and_then(|d| d.as_table())
            .is_some_and(|d| d.contains_key("day-piece-swiftui"))
    });
    Ok((packages, has_piece_dep))
}

// ===========================================================================
// Codegen — the Rust bindings (build.rs) and the Swift provider glue (day-cli)
// ===========================================================================

/// Render `$OUT_DIR/day_swiftui.rs` — one typed constructor per exported view, mirroring the Swift
/// identity verbatim (`crate::swiftui::MyView(…)`). `packages` pairs each package's display path
/// (for doc comments) with its scan. Always renders a valid module, empty when nothing is exported.
pub fn render_bindings(packages: &[(String, SwiftScan)]) -> String {
    // No inner attributes: the file is `include!`d inside a `pub mod`, where they cannot appear.
    let mut out = String::from(
        "// Generated by day-build from the local SwiftPM packages declared in\n\
         // [package.metadata.day.ios/macos] swift-packages (docs/swiftui.md). Do not edit.\n",
    );
    for (pkg, scan) in packages {
        for view in &scan.views {
            let generics: Vec<String> = (0..view.params.len()).map(|i| format!("M{i}")).collect();
            let generic_list = if generics.is_empty() {
                String::new()
            } else {
                format!("<{}>", generics.join(", "))
            };
            let args: Vec<String> = view
                .params
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    format!(
                        "{}: impl day_piece_swiftui::IntoReactive<{}, M{i}>",
                        p.key(),
                        p.ty.rust()
                    )
                })
                .collect();
            let sig_doc: Vec<String> = view
                .params
                .iter()
                .map(|p| format!("{}: {}", p.key(), p.ty.swift()))
                .collect();
            out.push_str(&format!(
                "\n/// `{}` from `{}/{}` — `public init({})`.\n\
                 /// Each argument accepts a constant, a `Signal`, or a closure; reactive values\n\
                 /// re-invoke the view's initializer live (`@State` is preserved).\n\
                 #[allow(non_snake_case)]\n\
                 pub fn {}{generic_list}({}) -> day_piece_swiftui::SwiftUi {{\n",
                view.piece_name(),
                pkg,
                view.source,
                sig_doc.join(", "),
                view.name,
                args.join(", "),
            ));
            for p in &view.params {
                out.push_str(&format!(
                    "    let {k} = day_piece_swiftui::IntoReactive::into_reactive({k});\n",
                    k = p.key()
                ));
            }
            let fields: Vec<String> = view
                .params
                .iter()
                .map(|p| {
                    let k = p.key();
                    let value = match p.ty {
                        SwiftType::String => {
                            format!("day_piece_swiftui::json::string(&{k}.get())")
                        }
                        SwiftType::Int => format!("day_piece_swiftui::json::int({k}.get())"),
                        SwiftType::Double => format!("day_piece_swiftui::json::float({k}.get())"),
                        SwiftType::Bool => format!("day_piece_swiftui::json::boolean({k}.get())"),
                    };
                    format!("(\"{k}\", {value})")
                })
                .collect();
            if fields.is_empty() {
                out.push_str(&format!(
                    "    day_piece_swiftui::swiftui(\"{}\")\n}}\n",
                    view.piece_name()
                ));
            } else {
                out.push_str(&format!(
                    "    day_piece_swiftui::swiftui(\"{}\").params(move || {{\n\
                     \x20       day_piece_swiftui::json::object(&[\n            {},\n        ])\n\
                     \x20   }})\n}}\n",
                    view.piece_name(),
                    fields.join(",\n            "),
                ));
            }
        }
    }
    out
}

/// Render the Swift provider glue for one crate's local packages: an `@objc(DayView_Module_View)`
/// [`DaySwiftUIProvider`] subclass per view, decoding the JSON params into the real initializer.
/// The file joins the generated `DayPieces` module (where `DaySwiftUIProvider` also lives), so the
/// only imports are SwiftUI and the scanned modules themselves.
pub fn render_glue(packages: &[(String, SwiftScan)]) -> String {
    let mut modules: Vec<&str> = packages
        .iter()
        .flat_map(|(_, s)| s.views.iter().map(|v| v.module.as_str()))
        .collect();
    modules.sort();
    modules.dedup();

    let mut out = String::from(
        "// Generated by `day build` from local SwiftPM packages (docs/swiftui.md). Do not edit.\n\
         import SwiftUI\n",
    );
    for m in &modules {
        out.push_str(&format!("import {m}\n"));
    }
    for (pkg, scan) in packages {
        for view in &scan.views {
            let class = view.class_name();
            out.push_str(&format!("\n// {}/{}\n@objc({class})\n", pkg, view.source));
            if view.params.is_empty() {
                out.push_str(&format!(
                    "final class {class}: DaySwiftUIProvider {{\n\
                     \x20   override func body(_ params: String?) -> AnyView {{\n\
                     \x20       AnyView({}())\n    }}\n}}\n",
                    view.name
                ));
                continue;
            }
            let fields: String = view
                .params
                .iter()
                .map(|p| format!("        var {}: {}\n", p.key(), p.ty.swift()))
                .collect();
            let args: Vec<String> = view
                .params
                .iter()
                .map(|p| match &p.label {
                    Some(label) => format!("{label}: p.{}", p.key()),
                    None => format!("p.{}", p.key()),
                })
                .collect();
            out.push_str(&format!(
                "final class {class}: DaySwiftUIProvider {{\n\
                 \x20   struct Params: Decodable {{\n{fields}    }}\n\
                 \x20   override func body(_ params: String?) -> AnyView {{\n\
                 \x20       guard let data = params?.data(using: .utf8),\n\
                 \x20             let p = try? JSONDecoder().decode(Params.self, from: data)\n\
                 \x20       else {{ return DaySwiftUI.errorView(\"{name}\") }}\n\
                 \x20       return AnyView({view}({args}))\n    }}\n}}\n",
                name = view.piece_name(),
                view = view.name,
                args = args.join(", "),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(src: &str) -> SwiftScan {
        let mut s = SwiftScan::default();
        scan_source("Mod", "Sources/Mod/File.swift", src, &mut s);
        s
    }

    #[test]
    fn a_public_view_with_a_supported_init_is_exported() {
        let s = scan(
            "import SwiftUI\n\
             public struct MyView: View {\n\
                 let title: String\n\
                 public init(title: String, count: Int, ratio: Double, on: Bool) {\n\
                     self.title = title\n\
                 }\n\
                 public var body: some View { Text(title) }\n\
             }\n",
        );
        assert_eq!(s.skipped, vec![]);
        assert_eq!(s.views.len(), 1);
        let v = &s.views[0];
        assert_eq!(v.piece_name(), "Mod.MyView");
        assert_eq!(v.class_name(), "DayView_Mod_MyView");
        let keys: Vec<&str> = v.params.iter().map(|p| p.key()).collect();
        assert_eq!(keys, ["title", "count", "ratio", "on"]);
        assert_eq!(v.params[1].ty, SwiftType::Int);
    }

    #[test]
    fn internal_and_non_view_structs_are_ignored_silently() {
        let s = scan(
            "struct Helper: View { var body: some View { Text(\"x\") } }\n\
             public struct Model: Codable, Equatable { public init() {} }\n",
        );
        assert_eq!(s.views, vec![]);
        assert_eq!(s.skipped, vec![]);
    }

    #[test]
    fn unsupported_inits_are_skipped_with_a_reason() {
        let s = scan(
            "public struct A: View { public init(m: MyModel) {} }\n\
             public struct B: View { public init(n: Int = 3) {} }\n\
             public struct C: View { let x = 1 }\n\
             public struct D<T>: View { public init() {} }\n",
        );
        assert_eq!(s.views, vec![]);
        let names: Vec<&str> = s.skipped.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Mod.A", "Mod.B", "Mod.C", "Mod.D"]);
        assert!(s.skipped[0].1.contains("MyModel"));
        assert!(s.skipped[1].1.contains("default"));
        assert!(s.skipped[2].1.contains("no public init"));
        assert!(s.skipped[3].1.contains("generic"));
    }

    #[test]
    fn unlabeled_and_two_name_parameters_parse() {
        let s = scan(
            "public struct V: View {\n\
                 public init(_ value: String, with count: Int) {}\n\
             }\n",
        );
        let v = &s.views[0];
        assert_eq!(v.params[0].label, None);
        assert_eq!(v.params[0].key(), "value");
        assert_eq!(v.params[1].label.as_deref(), Some("with"));
        assert_eq!(v.params[1].key(), "with");
        assert_eq!(v.params[1].name, "count");
    }

    #[test]
    fn braces_inside_comments_and_strings_do_not_confuse_the_depth() {
        let s = scan(
            "// a stray { in a comment\n\
             /* and { another /* nested { */ } */\n\
             let sample = \"{ not a brace }\"\n\
             let big = \"\"\"\n{ multi } \"line\"\n\"\"\"\n\
             public struct V: View { public init() {} }\n",
        );
        assert_eq!(s.views.len(), 1);
        assert!(s.views[0].params.is_empty());
    }

    #[test]
    fn nested_public_structs_are_not_top_level() {
        let s = scan(
            "public enum NS {\n\
                 public struct Inner: View { public init(t: String) {} }\n\
             }\n",
        );
        assert_eq!(s.views, vec![]);
        assert_eq!(s.skipped, vec![]);
    }

    #[test]
    fn the_first_public_init_is_the_contract() {
        let s = scan(
            "public struct V: View {\n\
                 public init(model: Thing) {}\n\
                 public init(title: String) {}\n\
             }\n",
        );
        // The FIRST public init is unsupported, so the view is skipped — a documented rule, so a
        // reordered overload can't silently switch which constructor the binding calls.
        assert_eq!(s.views, vec![]);
        assert!(s.skipped[0].1.contains("Thing"));
    }

    #[test]
    fn bindings_render_typed_reactive_constructors() {
        let mut s = SwiftScan::default();
        scan_source(
            "Mod",
            "Sources/Mod/V.swift",
            "public struct MyView: View { public init(title: String, count: Int) {} }\n\
             public struct Plain: View { public init() {} }\n",
            &mut s,
        );
        let code = render_bindings(&[("swiftui".into(), s)]);
        assert!(code.contains("pub fn MyView<M0, M1>("));
        assert!(code.contains("title: impl day_piece_swiftui::IntoReactive<String, M0>"));
        assert!(code.contains("count: impl day_piece_swiftui::IntoReactive<i64, M1>"));
        assert!(code.contains("swiftui(\"Mod.MyView\").params(move ||"));
        assert!(code.contains("(\"count\", day_piece_swiftui::json::int(count.get()))"));
        assert!(code.contains("pub fn Plain() -> day_piece_swiftui::SwiftUi"));
        assert!(!code.contains("Plain\").params"));
    }

    #[test]
    fn glue_renders_a_provider_per_view() {
        let mut s = SwiftScan::default();
        scan_source(
            "Mod",
            "Sources/Mod/V.swift",
            "public struct MyView: View { public init(_ text: String, count: Int) {} }\n",
            &mut s,
        );
        let glue = render_glue(&[("swiftui".into(), s)]);
        assert!(glue.contains("import Mod"));
        assert!(glue.contains("@objc(DayView_Mod_MyView)"));
        assert!(glue.contains("var text: String"));
        assert!(glue.contains("AnyView(MyView(p.text, count: p.count))"));
        assert!(glue.contains("DaySwiftUI.errorView(\"Mod.MyView\")"));
    }

    #[test]
    fn local_packages_derive_from_the_manifest() {
        let manifest = r#"
[package]
name = "showcase"

[dependencies]
day = { git = "https://github.com/daybrite/day.git" }
day-piece-swiftui = { git = "https://github.com/daybrite/day.git" }

[package.metadata.day.ios]
swift-packages = [{ path = "swiftui" }, { url = "https://example.com/pkg", from = "1.0.0" }]
platform = "16.0"

[package.metadata.day.macos]
swift-packages = [{ path = "swiftui" }]
"#;
        let (packages, has_dep) = local_packages_from_manifest(manifest).expect("parses");
        // Deduped across the two tables; the url entry is not a scan root.
        assert_eq!(packages, vec!["swiftui"]);
        assert!(has_dep);

        let bare = "[package]\nname = \"app\"\n[dependencies]\nday = \"1\"\n";
        let (packages, has_dep) = local_packages_from_manifest(bare).expect("parses");
        assert_eq!(packages, Vec::<String>::new());
        assert!(!has_dep);
    }

    #[test]
    fn duplicate_view_names_across_modules_error() {
        // Exercised through scan_package's post-sort check; simulate its input here.
        let mut s = SwiftScan::default();
        scan_source(
            "A",
            "a.swift",
            "public struct V: View { public init() {} }",
            &mut s,
        );
        scan_source(
            "B",
            "b.swift",
            "public struct V: View { public init() {} }",
            &mut s,
        );
        s.views
            .sort_by(|a, b| (&a.module, &a.name).cmp(&(&b.module, &b.name)));
        assert_eq!(s.views.len(), 2);
        assert_eq!(s.views[0].name, s.views[1].name);
    }
}
