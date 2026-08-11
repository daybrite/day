// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! daybridge codegen (docs/bridge.md, DESIGN.md §15.6) — the Rust half.
//!
//! Called from a bridged crate's `build.rs`:
//!
//! ```ignore
//! fn main() { day_build::bridge::generate().expect("day-build: bridge codegen"); }
//! ```
//!
//! It reads the crate's own `src/**/*.rs`, finds every `day_bridge::bridge! { … }` block, and
//! writes two things into `$OUT_DIR/day-bridge/`:
//!
//! - `mod.rs` — the Rust side: each declared function, cfg-gated per target, plus a
//!   `<fn>_support()` reporting what this target's arm promises. The `bridge!` macro `include!`s it.
//! - `manifest.json` — every foreign arm, for `day build` to emit adapters from (docs/bridge.md
//!   "What the build does"). Written even when empty so a stale one never lingers.
//!
//! Parsing is a text scan, not a syntax tree, for the same reason `swiftui.rs` scans Swift: the
//! input is *not all Rust*. An arm's body is a raw string holding another language, and the
//! attribute markers are inert tokens rustc never resolves.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// A language an arm can be written in.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Lang {
    Rust,
    Swift,
    Kotlin,
    Java,
    ArkTs,
    Js,
    C,
    Cpp,
}

impl Lang {
    fn parse(s: &str) -> Option<Lang> {
        Some(match s {
            "rust" => Lang::Rust,
            "swift" => Lang::Swift,
            "kotlin" => Lang::Kotlin,
            "java" => Lang::Java,
            "arkts" => Lang::ArkTs,
            "js" => Lang::Js,
            "c" => Lang::C,
            "cpp" => Lang::Cpp,
            _ => return None,
        })
    }

    /// The key used in the manifest and in `day build`'s emitters.
    pub fn key(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::Swift => "swift",
            Lang::Kotlin => "kotlin",
            Lang::Java => "java",
            Lang::ArkTs => "arkts",
            Lang::Js => "js",
            Lang::C => "c",
            Lang::Cpp => "cpp",
        }
    }
}

/// Every platform an arm may claim. `Other` is "whatever no other arm took".
const PLATFORMS: &[&str] = &[
    "ios", "macos", "android", "ohos", "web", "linux", "windows", "other",
];

/// The `cfg` predicate for one platform. `linux` and `ohos` both report `target_os = "linux"`,
/// so they are told apart by `target_env` exactly as day-part-battery's hand-written arms do.
fn cfg_for(platform: &str) -> &'static str {
    match platform {
        "ios" => "target_os = \"ios\"",
        "macos" => "target_os = \"macos\"",
        "android" => "target_os = \"android\"",
        "windows" => "target_os = \"windows\"",
        "web" => "target_arch = \"wasm32\"",
        "linux" => "all(target_os = \"linux\", not(target_env = \"ohos\"))",
        "ohos" => "all(target_os = \"linux\", target_env = \"ohos\")",
        _ => "",
    }
}

/// The v1 type table (docs/bridge.md "Types"). Anything else is a build error, which is how a
/// declaration that four languages cannot agree on is caught before an arm is written against it.
const SCALARS: &[&str] = &["bool", "i32", "i64", "f32", "f64"];

/// One function in a `#[day_bridge::declare] extern "day" { … }` block.
#[derive(Clone, Debug)]
pub struct Decl {
    pub name: String,
    /// `(name, type)` in declaration order.
    pub args: Vec<(String, String)>,
    /// The return type as written, minus `-> `; empty for unit.
    pub ret: String,
    /// Byte offset of the declaration in its source file, for diagnostics.
    pub line: usize,
}

/// One implementation of the declared API for a set of platforms.
#[derive(Clone, Debug)]
pub struct Arm {
    pub lang: Lang,
    pub platforms: Vec<String>,
    /// Inline body (the raw string's contents), or `None` when the arm names a file.
    pub body: Option<String>,
    /// `src = "…"`, relative to the crate root.
    pub src: Option<String>,
    /// Extra keys: `encoding`, `link`, `pkg_config`, `support`.
    pub options: BTreeMap<String, String>,
    /// The crate-relative `.rs` this arm was written in, for `#line` and diagnostics.
    pub source: Option<String>,
    /// The line the attribute sits on — what an error message names.
    pub line: usize,
    /// The line the arm's first line of foreign code sits on — what `#line` maps to, so a
    /// compiler diagnostic lands on the code rather than on the marker above it.
    pub body_line: usize,
}

/// A language's file-level preamble: imports only.
#[derive(Clone, Debug)]
pub struct Prelude {
    pub lang: Lang,
    pub body: String,
    pub line: usize,
}

/// Everything one crate declares.
#[derive(Default, Debug)]
pub struct Bridge {
    pub decls: Vec<Decl>,
    pub arms: Vec<Arm>,
    pub preludes: Vec<Prelude>,
}

/// Read `src/**/*.rs`, generate `$OUT_DIR/day-bridge/{mod.rs,manifest.json}`.
///
/// A crate with no `bridge!` block still gets an (empty) `mod.rs`, so a crate that removes its last
/// bridge does not fail on a stale `include!`.
pub fn generate() -> Result<(), String> {
    let root = std::env::var("CARGO_MANIFEST_DIR").map_err(|_| "CARGO_MANIFEST_DIR unset")?;
    let out = std::env::var("OUT_DIR").map_err(|_| "OUT_DIR unset")?;
    let crate_name = std::env::var("CARGO_PKG_NAME").map_err(|_| "CARGO_PKG_NAME unset")?;
    generate_in(Path::new(&root), Path::new(&out), &crate_name)
}

/// Parse one crate's `bridge!` blocks — the entry point `day build` uses to generate the foreign
/// half. The CLI reads crate SOURCES rather than build-script output, so staging never depends on
/// cargo having run first (docs/bridge.md "What the build does").
pub fn parse_crate(root: &Path) -> Result<Bridge, String> {
    let bridge = scan(root)?;
    validate(&bridge)?;
    Ok(bridge)
}

/// Whether a crate declares any bridge at all — cheap enough to run over a whole dependency graph.
pub fn is_bridged(root: &Path) -> bool {
    let mut sources: Vec<PathBuf> = Vec::new();
    collect_rs(&root.join("src"), &mut sources);
    sources.iter().any(|p| {
        std::fs::read_to_string(p)
            .map(|t| {
                t.lines()
                    .any(|l| !l.trim_start().starts_with("//") && l.contains("bridge!"))
            })
            .unwrap_or(false)
    })
}

/// The generated Swift adapter for `arm`, ready to stage into the DayPieces package.
pub fn swift_adapter(bridge: &Bridge, arm: &Arm, crate_name: &str) -> String {
    render_swift(bridge, arm, crate_name)
}

/// The generated JVM adapter for `arm` — Kotlin or Java — ready to stage into a Gradle source
/// directory. The language decides only the file extension and whether the project needs the
/// Kotlin plugin (see the check in `day lint` and the error in `day build`).
pub fn jvm_adapter(bridge: &Bridge, arm: &Arm, crate_name: &str) -> String {
    match arm.lang {
        Lang::Java => render_java(bridge, arm, crate_name),
        _ => render_kotlin(bridge, arm, crate_name),
    }
}

/// The generated ES module for `arm`, ready to stage beside the day-dom shim.
pub fn js_adapter(bridge: &Bridge, arm: &Arm, crate_name: &str) -> String {
    render_js(bridge, arm, crate_name)
}

/// The generated ArkTS module for `arm`, ready to stage into the HarmonyOS host project.
pub fn arkts_adapter(bridge: &Bridge, arm: &Arm, crate_name: &str) -> String {
    render_arkts(bridge, arm, crate_name)
}

/// The Java package a crate's Kotlin adapter declares — the directory Gradle expects it under.
pub fn kotlin_package_of(crate_name: &str) -> String {
    kotlin_package(crate_name)
}

/// The file name an arm's adapter is staged under.
pub fn adapter_name(arm: &Arm, crate_name: &str) -> String {
    generated_name(arm, crate_name)
}

fn scan(root: &Path) -> Result<Bridge, String> {
    let mut sources: Vec<PathBuf> = Vec::new();
    collect_rs(&root.join("src"), &mut sources);
    sources.sort(); // deterministic output (docs/bridge.md "Determinism and mtimes")

    let mut bridge = Bridge::default();
    for path in &sources {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !text.contains("bridge!") {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();
        parse_into(&text, &rel, &mut bridge).map_err(|e| format!("{rel}: {e}"))?;
    }
    Ok(bridge)
}

/// The testable core of [`generate`]: the Rust side, plus the C/C++ arms cargo itself compiles.
pub fn generate_in(root: &Path, out_dir: &Path, crate_name: &str) -> Result<(), String> {
    // Only a build script may print cargo directives — `parse_crate` is also called by `day build`,
    // where a stray `cargo:` line would land in the CLI's own output (and, once, inside a
    // generated ES module).
    let mut sources: Vec<PathBuf> = Vec::new();
    collect_rs(&root.join("src"), &mut sources);
    sources.sort();
    for path in &sources {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let bridge = parse_crate(root)?;

    let dir = out_dir.join("day-bridge");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    write_if_changed(&dir.join("mod.rs"), &render_rust(&bridge, crate_name))?;
    emit_c(&bridge, &dir, crate_name)?;
    Ok(())
}

/// The platform this build is for, from cargo's own cfg environment — the same distinction the
/// generated `cfg`s make, so exactly one arm is ever active.
fn active_platform() -> Option<String> {
    let os = std::env::var("CARGO_CFG_TARGET_OS").ok()?;
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    Some(
        match (os.as_str(), env.as_str(), arch.as_str()) {
            (_, _, "wasm32") => "web",
            ("linux", "ohos", _) => "ohos",
            ("linux", _, _) => "linux",
            ("ios", _, _) => "ios",
            ("macos", _, _) => "macos",
            ("android", _, _) => "android",
            ("windows", _, _) => "windows",
            _ => return None,
        }
        .to_string(),
    )
}

/// Write every C/C++ arm's translation unit, and compile the one this target selects. Swift,
/// Kotlin, ArkTS and JavaScript adapters are NOT written here — `day build` renders those from the
/// crate's source when it stages them, so each artifact has exactly one producer.
///
/// Sources for inactive arms are written too: they cost nothing, they keep the generated tree
/// diffable, and a cross-compile that switches targets finds them already correct.
fn emit_c(bridge: &Bridge, dir: &Path, crate_name: &str) -> Result<(), String> {
    let active = active_platform();

    // Declare the cfg unconditionally (cargo lints unknown ones), and set it only when `day build`
    // says it is staging and linking this crate's foreign half for the active target.
    println!("cargo:rustc-check-cfg=cfg({STAGED_CFG})");
    println!("cargo:rerun-if-env-changed=DAY_BRIDGE_STAGED");
    let staged_here = std::env::var("DAY_BRIDGE_STAGED").is_ok()
        && bridge.arms.iter().any(|a| {
            staged_by_cli(a.lang)
                && active
                    .as_deref()
                    .is_some_and(|p| a.platforms.iter().any(|x| x == p))
        });
    if staged_here {
        println!("cargo:rustc-cfg={STAGED_CFG}");
    }

    // Swift arms are compiled by `day build`'s prepass into the generated DayPieces package, not
    // by cargo: this writes the adapter and the manifest points at it (docs/bridge.md).
    for arm in bridge.arms.iter().filter(|a| a.lang == Lang::Swift) {
        let file = dir.join(format!("{}-{}.swift", crate_name, arm.platforms.join("-")));
        write_if_changed(&file, &render_swift(bridge, arm, crate_name))?;
    }

    for arm in bridge
        .arms
        .iter()
        .filter(|a| matches!(a.lang, Lang::C | Lang::Cpp))
    {
        let cpp = arm.lang == Lang::Cpp;
        let file = dir.join(format!(
            "{}-{}.{}",
            crate_name,
            arm.platforms.join("-"),
            if cpp { "cpp" } else { "c" }
        ));
        write_if_changed(&file, &render_c(bridge, arm, crate_name))?;

        let selected = active
            .as_deref()
            .is_some_and(|p| arm.platforms.iter().any(|a| a == p));
        if !selected {
            continue;
        }
        let mut build = cc::Build::new();
        build.file(&file).cpp(cpp).warnings(false);
        if cpp {
            build.std("c++17");
        }
        build.compile(&format!("day_bridge_{}", crate_name.replace('-', "_")));
        for lib in arm
            .options
            .get("link")
            .map(|v| v.trim_matches(['[', ']']).to_string())
            .unwrap_or_default()
            .split(',')
            .map(|l| l.trim().trim_matches('"'))
            .filter(|l| !l.is_empty())
        {
            println!("cargo:rustc-link-lib={lib}");
        }
        if let Some(pkg) = arm.options.get("pkg_config") {
            println!("cargo:rustc-link-lib={pkg}");
        }
    }
    Ok(())
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Find every `bridge! { … }` body in `text` and parse its items into `bridge`.
fn parse_into(text: &str, source: &str, bridge: &mut Bridge) -> Result<(), String> {
    let mut at = 0;
    while let Some(found) = text[at..].find("bridge!") {
        let start = at + found;
        // A doc comment showing the macro is not an invocation of it (this crate's own docs do
        // exactly that), so a match whose line is a comment is skipped.
        let line_start = text[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        if text[line_start..start].trim_start().starts_with("//") {
            at = start + "bridge!".len();
            continue;
        }
        // Only a macro invocation: require the next non-space character to open a brace.
        let after = start + "bridge!".len();
        let Some(brace) = text[after..]
            .find(|c: char| !c.is_whitespace())
            .map(|i| after + i)
        else {
            break;
        };
        if text.as_bytes().get(brace) != Some(&b'{') {
            at = after;
            continue;
        }
        let end = match_delim(text, brace, b'{', b'}')
            .ok_or_else(|| "unterminated `bridge! {` block".to_string())?;
        let first = bridge.arms.len();
        parse_body(&text[brace + 1..end], line_of(text, brace), bridge)?;
        for arm in &mut bridge.arms[first..] {
            arm.source = Some(source.to_string());
        }
        at = end + 1;
    }
    Ok(())
}

/// Walk items inside one `bridge!` body. Every item starts with a `#[day_bridge::…]` marker.
fn parse_body(body: &str, base_line: usize, bridge: &mut Bridge) -> Result<(), String> {
    let mut at = 0;
    while let Some(found) = body[at..].find("#[day_bridge::") {
        let start = at + found;
        let open = start + "#[".len() - 1; // the '[' of the attribute
        let close = match_delim(body, open, b'[', b']')
            .ok_or_else(|| "unterminated bridge attribute".to_string())?;
        let attr = &body[start + 2..close];
        // `line_of` is 1-based within the body, and the body starts on the same line as the
        // opening brace — so the two overlap by one line.
        let line = base_line + line_of(body, start) - 1;
        let rest = &body[close + 1..];

        let kind = attr
            .trim_start_matches("day_bridge::")
            .split(['(', ' '])
            .next()
            .unwrap_or("")
            .trim();
        let consumed = match kind {
            "declare" => parse_declare(rest, line, bridge)?,
            "prelude" => parse_prelude(attr, rest, line, bridge)?,
            "impl" => parse_impl(attr, rest, line, bridge)?,
            "data" => 0, // the struct is ordinary Rust; day-cli reads it from the manifest's decls
            other => return Err(format!("line {line}: unknown bridge attribute `{other}`")),
        };
        at = close + 1 + consumed;
    }
    Ok(())
}

/// `extern "day" { fn a(…) -> …; fn b(); }` → one [`Decl`] each.
fn parse_declare(rest: &str, line: usize, bridge: &mut Bridge) -> Result<usize, String> {
    let open = rest
        .find('{')
        .ok_or_else(|| format!("line {line}: `declare` needs an `extern \"day\" {{ … }}` block"))?;
    let close = match_delim(rest, open, b'{', b'}')
        .ok_or_else(|| format!("line {line}: unterminated `extern \"day\"` block"))?;
    for raw in rest[open + 1..close].split(';') {
        let sig = strip_comments(raw);
        let sig = sig.trim();
        if sig.is_empty() {
            continue;
        }
        let sig = sig
            .strip_prefix("fn ")
            .ok_or_else(|| format!("line {line}: `{sig}` is not a `fn` declaration"))?;
        let name_end = sig
            .find('(')
            .ok_or_else(|| format!("line {line}: `{sig}` has no argument list"))?;
        let name = sig[..name_end].trim().to_string();
        let args_end = match_delim(sig, name_end, b'(', b')')
            .ok_or_else(|| format!("line {line}: `{name}` has an unterminated argument list"))?;
        let mut args = Vec::new();
        for arg in split_top(&sig[name_end + 1..args_end], ',') {
            let arg = arg.trim();
            if arg.is_empty() {
                continue;
            }
            let (n, t) = arg
                .split_once(':')
                .ok_or_else(|| format!("line {line}: argument `{arg}` needs a type"))?;
            args.push((n.trim().to_string(), t.trim().to_string()));
        }
        let ret = sig[args_end + 1..]
            .trim()
            .strip_prefix("->")
            .map(|r| r.trim().to_string())
            .unwrap_or_default();
        bridge.decls.push(Decl {
            name,
            args,
            ret,
            line,
        });
    }
    Ok(close + 1)
}

fn parse_prelude(
    attr: &str,
    rest: &str,
    line: usize,
    bridge: &mut Bridge,
) -> Result<usize, String> {
    let lang = attr_lang(attr, line)?;
    let (body, consumed, _) = raw_string_after(rest, line)?;
    for bad in ["package ", "namespace ", "module "] {
        if body.lines().any(|l| l.trim_start().starts_with(bad)) {
            return Err(format!(
                "line {line}: a `{}` line belongs to the generator, not a prelude — daybridge \
                 derives it from the crate name (docs/bridge.md \"Names\")",
                bad.trim()
            ));
        }
    }
    bridge.preludes.push(Prelude { lang, body, line });
    Ok(consumed)
}

fn parse_impl(attr: &str, rest: &str, line: usize, bridge: &mut Bridge) -> Result<usize, String> {
    let lang = attr_lang(attr, line)?;
    let inner = attr
        .split_once('(')
        .map(|(_, v)| v.trim_end().trim_end_matches(')'))
        .unwrap_or("");
    let mut platforms = Vec::new();
    let mut options = BTreeMap::new();
    for part in split_top(inner, ',') {
        let part = part.trim();
        if part.is_empty() || Lang::parse(part).is_some() {
            continue;
        }
        let Some((key, value)) = part.split_once('=') else {
            return Err(format!("line {line}: `{part}` is not `key = value`"));
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"');
        if key == "platforms" {
            for p in value.trim_matches(['[', ']']).split(',') {
                let p = p.trim();
                if p.is_empty() {
                    continue;
                }
                if !PLATFORMS.contains(&p) {
                    return Err(format!(
                        "line {line}: unknown platform `{p}` (expected one of {})",
                        PLATFORMS.join(", ")
                    ));
                }
                platforms.push(p.to_string());
            }
        } else {
            options.insert(key.to_string(), value.to_string());
        }
    }
    if platforms.is_empty() {
        return Err(format!("line {line}: an arm must name `platforms = [ … ]`"));
    }

    // A rust arm is ordinary Rust captured verbatim; every other language rides a raw string, or
    // names a file with `src = "…"`.
    let (body, consumed, body_line) = if lang == Lang::Rust {
        let (body, consumed) = rust_item_after(rest, line)?;
        (Some(body), consumed, line)
    } else if options.contains_key("src") {
        (None, 0, line)
    } else {
        let (body, consumed, skipped) = raw_string_after(rest, line)?;
        (Some(body), consumed, line + skipped + 1)
    };

    bridge.arms.push(Arm {
        lang,
        platforms,
        body,
        src: options.get("src").cloned(),
        options,
        source: None,
        line,
        body_line,
    });
    Ok(consumed)
}

fn attr_lang(attr: &str, line: usize) -> Result<Lang, String> {
    let inner = attr.split_once('(').map(|(_, v)| v).unwrap_or("");
    let first = inner.split([',', ')']).next().unwrap_or("").trim();
    Lang::parse(first).ok_or_else(|| format!("line {line}: unknown bridge language `{first}`"))
}

/// Take the raw-string body of the `lang!(r#"…"#)` invocation that follows an attribute, with the
/// number of lines skipped to reach it (the marker and the opener), so `#line` can be exact.
fn raw_string_after(rest: &str, line: usize) -> Result<(String, usize, usize), String> {
    let open = rest.find("r#\"").ok_or_else(|| {
        format!("line {line}: expected a raw-string body, e.g. `kotlin!(r#\" … \"#)`")
    })?;
    let start = open + 3;
    let end = rest[start..]
        .find("\"#")
        .map(|i| start + i)
        .ok_or_else(|| format!("line {line}: unterminated raw string"))?;
    let skipped = rest[..start].matches('\n').count();
    Ok((dedent(&rest[start..end]), end + 2, skipped))
}

/// Take one complete Rust item (`fn … { … }`) following an attribute, verbatim.
fn rust_item_after(rest: &str, line: usize) -> Result<(String, usize), String> {
    let open = rest
        .find('{')
        .ok_or_else(|| format!("line {line}: expected a Rust `fn` body"))?;
    let close = match_delim(rest, open, b'{', b'}')
        .ok_or_else(|| format!("line {line}: unterminated Rust body"))?;
    Ok((dedent_item(rest[..=close].trim()), close + 1))
}

// ---------------------------------------------------------------------------
// Scanning helpers
// ---------------------------------------------------------------------------

/// Index of the delimiter closing the one at `from`, skipping strings, raw strings, chars and
/// comments — the whole reason this is hand-written rather than a `find`.
fn match_delim(text: &str, from: usize, open: u8, close: u8) -> Option<usize> {
    let b = text.as_bytes();
    let mut depth = 0usize;
    let mut i = from;
    while i < b.len() {
        match b[i] {
            b'r' if b[i..].starts_with(b"r#\"") => {
                i += 3;
                while i < b.len() && !b[i..].starts_with(b"\"#") {
                    i += 1;
                }
                i += 2;
                continue;
            }
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            c if c == open => depth += 1,
            c if c == close => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split on `sep` at nesting depth zero.
fn split_top(text: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in text.chars() {
        match c {
            '(' | '[' | '<' | '{' => depth += 1,
            ')' | ']' | '>' | '}' => depth -= 1,
            _ => {}
        }
        if c == sep && depth == 0 {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

fn strip_comments(text: &str) -> String {
    text.lines()
        .map(|l| l.split_once("//").map(|(a, _)| a).unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn line_of(text: &str, at: usize) -> usize {
    text[..at].matches('\n').count() + 1
}

/// Remove the common leading indentation an inline arm picked up from the `.rs` file it lives in,
/// so the generated foreign source starts at column zero.
fn dedent(body: &str) -> String {
    let indent = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    body.lines()
        .map(|l| {
            if l.len() >= indent {
                &l[indent..]
            } else {
                l.trim_start()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_string()
}

/// Strip the indentation a captured Rust item inherited from the `bridge!` block around it: the
/// first line is already flush, so the rest is re-based on its own minimum.
fn dedent_item(item: &str) -> String {
    let mut lines = item.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };
    let rest: Vec<&str> = lines.collect();
    let indent = rest
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut out = String::from(first);
    for line in rest {
        out.push('\n');
        out.push_str(if line.len() >= indent {
            &line[indent..]
        } else {
            line.trim_start()
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Validation (docs/bridge.md "What fails the build")
// ---------------------------------------------------------------------------

fn validate(bridge: &Bridge) -> Result<(), String> {
    if bridge.decls.is_empty() && bridge.arms.is_empty() {
        return Ok(());
    }

    // Types must be inside the v1 table.
    for decl in &bridge.decls {
        for (arg, ty) in &decl.args {
            check_type(ty, true)
                .map_err(|e| format!("line {}: `{}`'s `{arg}`: {e}", decl.line, decl.name))?;
        }
        if !decl.ret.is_empty() {
            let inner = decl
                .ret
                .strip_prefix("Result<")
                .and_then(|r| r.strip_suffix('>'))
                .map(|r| split_top(r, ',').first().cloned().unwrap_or_default())
                .unwrap_or_else(|| decl.ret.clone());
            let inner = inner.trim();
            if !inner.is_empty() && inner != "()" {
                check_type(inner, false)
                    .map_err(|e| format!("line {}: `{}`'s return: {e}", decl.line, decl.name))?;
            }
        }
    }

    // Value returns ride the JVM's exception channel; C and Swift would need an out-parameter,
    // which v1 has no spelling for. Catch it here rather than generate an adapter that cannot
    // compile.
    for decl in &bridge.decls {
        if result_value(&decl.ret).is_none() {
            continue;
        }
        if let Some(arm) = bridge
            .arms
            .iter()
            .find(|a| matches!(a.lang, Lang::C | Lang::Cpp | Lang::Swift))
        {
            return Err(format!(
                "line {}: `{}` returns a value, which the {} arm cannot express yet — return \
                 `Result<(), day_bridge::Error>` there, or split the value into its own function",
                arm.line,
                decl.name,
                arm.lang.key()
            ));
        }
    }

    // One LANGUAGE per target, and a fallback for everything else. Several arms may share a
    // language and a platform — that is how the rust arm implements one `fn` per item, and how a
    // Kotlin arm can be split — but two languages claiming one target would leave the generator
    // with no answer for which adapter to emit.
    let mut claimed: BTreeMap<&str, (Lang, usize)> = BTreeMap::new();
    for arm in &bridge.arms {
        for p in &arm.platforms {
            match claimed.get(p.as_str()) {
                Some(&(lang, first)) if lang != arm.lang => {
                    return Err(format!(
                        "line {}: platform `{p}` is already claimed by the {} arm on line {first}",
                        arm.line,
                        lang.key()
                    ));
                }
                _ => {
                    claimed.insert(p, (arm.lang, arm.line));
                }
            }
        }
    }
    if !bridge.decls.is_empty() && !claimed.contains_key("other") {
        return Err(
            "no `other` arm: a bridged crate must compile under day-mock on any host \
             (docs/bridge.md \"Platform selection\")"
                .into(),
        );
    }

    // The rust arm's coverage is checkable right here: a missing definition would otherwise be an
    // error inside generated code, pointing at a file nobody wrote.
    let rust: Vec<&str> = bridge
        .arms
        .iter()
        .filter(|a| a.lang == Lang::Rust)
        .filter_map(|a| a.body.as_deref())
        .collect();
    if !rust.is_empty() {
        for decl in &bridge.decls {
            let wanted = format!("fn {}", decl.name);
            if !rust.iter().any(|body| body.contains(&wanted)) {
                return Err(format!(
                    "line {}: no rust arm implements `{}`",
                    decl.line, decl.name
                ));
            }
        }
    }
    Ok(())
}

fn check_type(ty: &str, argument: bool) -> Result<(), String> {
    let ty = ty.trim();
    if SCALARS.contains(&ty) {
        return Ok(());
    }
    if argument && (ty == "&str" || ty == "&[u8]") {
        return Ok(());
    }
    if !argument && (ty == "String" || ty == "Vec<u8>") {
        return Ok(());
    }
    if ty.starts_with("Option<") {
        return Err(format!(
            "`{ty}` does not cross a bridge — model absence in the value, or return `Result` \
             (docs/bridge.md \"Types\")"
        ));
    }
    // A `#[day_bridge::data]` struct is named by the crate; day-cli validates its fields.
    if ty.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        return Ok(());
    }
    Err(format!(
        "`{ty}` is outside the v1 type table (docs/bridge.md \"Types\")"
    ))
}

// ---------------------------------------------------------------------------
// Emitting
// ---------------------------------------------------------------------------

/// The exported symbol for one declared function (docs/bridge.md "Names").
fn symbol(crate_name: &str, decl: &Decl) -> String {
    format!("day_bridge_{}_{}", crate_name.replace('-', "_"), decl.name)
}

/// The C spelling of a v1 type. `&str` is UTF-8 unless the arm opts into UTF-16
/// (docs/bridge.md "Types").
fn c_type(ty: &str, utf16: bool) -> &'static str {
    match ty.trim() {
        "bool" | "i32" => "int32_t",
        "i64" => "int64_t",
        "f32" => "float",
        "f64" => "double",
        "&str" if utf16 => "const char16_t*",
        "&str" => "const char*",
        _ => "const void*",
    }
}

fn rust_c_type(ty: &str, utf16: bool) -> &'static str {
    match ty.trim() {
        "bool" | "i32" => "i32",
        "i64" => "i64",
        "f32" => "f32",
        "f64" => "f64",
        "&str" if utf16 => "*const u16",
        "&str" => "*const std::ffi::c_char",
        _ => "*const std::ffi::c_void",
    }
}

/// The translation unit for one C/C++ arm: the crate's prelude for that language, a `#line`
/// pointing back at the `.rs` the arm was written in, the arm itself, and one exported adapter per
/// declared function. The arm writes plain `speak_native(…)`; the adapter is what carries the
/// prefixed symbol Rust links against, so nothing in the arm has to know the naming scheme.
fn render_c(bridge: &Bridge, arm: &Arm, crate_name: &str) -> String {
    let utf16 = arm.options.get("encoding").map(String::as_str) == Some("utf16");
    let source = arm.source.as_deref().unwrap_or("src/lib.rs");
    let mut out = String::new();
    let _ = writeln!(
        out,
        "/* @generated by day-build from {source}:{} — edit the arm, never this file. */",
        arm.line
    );
    let _ = writeln!(out, "#include <stdint.h>");
    for prelude in bridge.preludes.iter().filter(|p| p.lang == arm.lang) {
        let _ = writeln!(out, "{}", prelude.body);
    }
    let _ = writeln!(out, "\n#line {} {}", arm.body_line, quote(source));
    let _ = writeln!(out, "{}\n", arm.body.as_deref().unwrap_or(""));
    let _ = writeln!(out, "#line 1 {}", quote("<day-bridge adapters>"));
    // A C++ translation unit mangles these names unless they are told not to, and Rust links
    // against the unmangled spelling. C needs no such thing.
    if arm.lang == Lang::Cpp {
        let _ = writeln!(out, "extern \"C\" {{");
    }
    for decl in &bridge.decls {
        let params: Vec<String> = decl
            .args
            .iter()
            .map(|(n, t)| format!("{} {n}", c_type(t, utf16)))
            .collect();
        let names: Vec<&str> = decl.args.iter().map(|(n, _)| n.as_str()).collect();
        let params = if params.is_empty() {
            "void".to_string()
        } else {
            params.join(", ")
        };
        if decl.ret.is_empty() {
            let _ = writeln!(
                out,
                "void {}({params}) {{ {}({}); }}",
                symbol(crate_name, decl),
                decl.name,
                names.join(", ")
            );
        } else {
            let _ = writeln!(
                out,
                "int32_t {}({params}) {{ return {}({}); }}",
                symbol(crate_name, decl),
                decl.name,
                names.join(", ")
            );
        }
    }
    if arm.lang == Lang::Cpp {
        let _ = writeln!(out, "}}");
    }
    out
}

/// The Swift adapter for one arm: the crate's Swift prelude, a `#sourceLocation` back to the
/// `.rs`, the arm itself, and one `@_cdecl` export per declared function. The arm writes ordinary
/// Swift — `func speakNative(text: String) throws` — and never sees the C ABI.
fn render_swift(bridge: &Bridge, arm: &Arm, crate_name: &str) -> String {
    let source = arm.source.as_deref().unwrap_or("src/lib.rs");
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// @generated by day-build from {source}:{} — edit the arm, never this file.",
        arm.line
    );
    let _ = writeln!(out, "import Foundation");
    for prelude in bridge.preludes.iter().filter(|p| p.lang == Lang::Swift) {
        let _ = writeln!(out, "{}", prelude.body);
    }
    // swiftc maps every following line back to the crate's own source, so a type error in an arm
    // names the file its author opened (docs/bridge.md "Diagnostics").
    let _ = writeln!(
        out,
        "\n#sourceLocation(file: {}, line: {})",
        quote(source),
        arm.body_line
    );
    let _ = writeln!(out, "{}", arm.body.as_deref().unwrap_or(""));
    let _ = writeln!(out, "#sourceLocation()\n");

    for decl in &bridge.decls {
        let params: Vec<String> = decl
            .args
            .iter()
            .map(|(n, t)| format!("{n}: {}", swift_abi_type(t)))
            .collect();
        let ret = if decl.ret.is_empty() { "" } else { " -> Int32" };
        let _ = writeln!(out, "@_cdecl({})", quote(&symbol(crate_name, decl)));
        let _ = writeln!(
            out,
            "public func {}({}){ret} {{",
            symbol(crate_name, decl),
            params.join(", ")
        );
        // Marshal each argument into the Swift type the arm declared.
        let mut passed: Vec<String> = Vec::new();
        for (n, t) in &decl.args {
            match t.trim() {
                "&str" => {
                    let _ = writeln!(out, "    let {n}_s = String(cString: {n})");
                    passed.push(format!("{n}: {n}_s"));
                }
                "bool" => {
                    let _ = writeln!(out, "    let {n}_b = {n} != 0");
                    passed.push(format!("{n}: {n}_b"));
                }
                _ => passed.push(format!("{n}: {n}")),
            }
        }
        let call = format!("{}({})", decl.name, passed.join(", "));
        if decl.ret.is_empty() {
            let _ = writeln!(out, "    {call}");
        } else {
            // A `throws` arm becomes a status code: 0 on success, 1 with the message logged.
            let _ = writeln!(out, "    do {{");
            let _ = writeln!(out, "        try {call}");
            let _ = writeln!(out, "        return 0");
            let _ = writeln!(out, "    }} catch {{");
            let _ = writeln!(
                out,
                "        FileHandle.standardError.write(\"day-bridge: \\(error)\\n\".data(using: .utf8)!)"
            );
            let _ = writeln!(out, "        return 1");
            let _ = writeln!(out, "    }}");
        }
        let _ = writeln!(out, "}}\n");
    }
    out
}

/// The generated ES module for a JavaScript arm: the crate's prelude, the arm itself, and a
/// `register(rt)` returning the wasm imports the day-dom shim merges into its `env` object.
///
/// wasm has no C ABI for strings, so a `&str` argument crosses as `(ptr, len)` into the module's
/// linear memory and the runtime helper `rt.str` decodes it (docs/web.md's shim owns `wasm.memory`,
/// not this module). The arm never sees any of that.
fn render_js(bridge: &Bridge, arm: &Arm, crate_name: &str) -> String {
    let source = arm.source.as_deref().unwrap_or("src/lib.rs");
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// @generated by day-build from {source}:{} — edit the arm, never this file.\n\
         //# sourceURL={source}",
        arm.line
    );
    for prelude in bridge.preludes.iter().filter(|p| p.lang == Lang::Js) {
        let _ = writeln!(out, "{}", prelude.body);
    }
    let _ = writeln!(out, "\n{}\n", arm.body.as_deref().unwrap_or(""));

    let _ = writeln!(
        out,
        "// The shim calls this once at boot and spreads the result into the wasm import object."
    );
    let _ = writeln!(out, "export function register(rt) {{");
    let _ = writeln!(out, "  return {{");
    for decl in &bridge.decls {
        let mut params: Vec<String> = Vec::new();
        let mut passed: Vec<String> = Vec::new();
        for (n, t) in &decl.args {
            if t.trim() == "&str" {
                params.push(format!("{n}_ptr"));
                params.push(format!("{n}_len"));
                passed.push(format!("rt.str({n}_ptr, {n}_len)"));
            } else {
                params.push(n.clone());
                passed.push(n.clone());
            }
        }
        let call = format!("{}({})", decl.name, passed.join(", "));
        let _ = writeln!(
            out,
            "    {}({}) {{",
            symbol(crate_name, decl),
            params.join(", ")
        );
        match (decl.ret.is_empty(), result_value(&decl.ret)) {
            (true, _) => {
                let _ = writeln!(out, "      {call};");
            }
            (false, None) => {
                // A thrown error is the failure channel, mapped to the same status code C uses.
                let _ = writeln!(out, "      try {{");
                let _ = writeln!(out, "        {call};");
                let _ = writeln!(out, "        return 0;");
                let _ = writeln!(out, "      }} catch (e) {{");
                let _ = writeln!(
                    out,
                    "        console.error('day-bridge: {}', e);",
                    decl.name
                );
                let _ = writeln!(out, "        return 1;");
                let _ = writeln!(out, "      }}");
            }
            (false, Some(_)) => {
                let _ = writeln!(out, "      return {call};");
            }
        }
        let _ = writeln!(out, "    }},");
    }
    let _ = writeln!(out, "  }};");
    let _ = writeln!(out, "}}");
    out
}

/// The generated ArkTS module for one arm. HarmonyOS compiles ArkTS only from inside the host
/// module, so this lands in the project's `daypieces` tree beside the piece modules (§15.2) and is
/// reached through the `Index.ets` the CLI writes next to it.
fn render_arkts(bridge: &Bridge, arm: &Arm, crate_name: &str) -> String {
    let source = arm.source.as_deref().unwrap_or("src/lib.rs");
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// @generated by day-build from {source}:{} — edit the arm, never this file.",
        arm.line
    );
    for prelude in bridge.preludes.iter().filter(|p| p.lang == Lang::ArkTs) {
        let _ = writeln!(out, "{}", prelude.body);
    }
    let _ = writeln!(out, "\n{}\n", arm.body.as_deref().unwrap_or(""));
    let _ = writeln!(
        out,
        "// The host calls this once at startup; the returned record is registered with the napi\n\
         // module so the Rust side can reach each arm by name."
    );
    let _ = writeln!(
        out,
        "export function register(): Record<string, Function> {{"
    );
    let _ = writeln!(out, "  return {{");
    for decl in &bridge.decls {
        let _ = writeln!(out, "    '{}': {},", symbol(crate_name, decl), decl.name);
    }
    let _ = writeln!(out, "  }};");
    let _ = writeln!(out, "}}");
    out
}

/// The Rust half of a JavaScript arm: wasm imports, with `&str` crossing as `(ptr, len)` into the
/// module's own linear memory — no CString, no allocation, nothing to free.
fn render_js_rust(bridge: &Bridge, crate_name: &str) -> String {
    let mut out = String::new();
    // Without this the linker treats the imports as symbols it must resolve and fails with
    // "undefined symbol"; with it they are wasm imports the host supplies at instantiation, which
    // is exactly how day-dom declares the shim's own entry points (toolkits/day-dom/src/lib.rs).
    let _ = writeln!(out, "#[link(wasm_import_module = \"env\")]");
    let _ = writeln!(out, "unsafe extern \"C\" {{");
    for decl in &bridge.decls {
        let mut params: Vec<String> = Vec::new();
        for (n, t) in &decl.args {
            if t.trim() == "&str" {
                params.push(format!("{n}_ptr: *const u8"));
                params.push(format!("{n}_len: usize"));
            } else {
                params.push(format!("{n}: {}", rust_c_type(t, false)));
            }
        }
        let ret = match (decl.ret.is_empty(), result_value(&decl.ret)) {
            (true, _) => String::new(),
            (false, None) => " -> i32".to_string(),
            (false, Some(ty)) => format!(" -> {ty}"),
        };
        let _ = writeln!(
            out,
            "    fn {}({}){ret};",
            symbol(crate_name, decl),
            params.join(", ")
        );
    }
    let _ = writeln!(out, "}}\n");

    for decl in &bridge.decls {
        let args: Vec<String> = decl.args.iter().map(|(n, t)| format!("{n}: {t}")).collect();
        let ret = if decl.ret.is_empty() {
            String::new()
        } else {
            format!(" -> {}", decl.ret)
        };
        let _ = writeln!(out, "fn {}({}){ret} {{", decl.name, args.join(", "));
        let mut passed: Vec<String> = Vec::new();
        for (n, t) in &decl.args {
            if t.trim() == "&str" {
                passed.push(format!("{n}.as_ptr()"));
                passed.push(format!("{n}.len()"));
            } else if t.trim() == "bool" {
                passed.push(format!("{n} as i32"));
            } else {
                passed.push(n.clone());
            }
        }
        let call = format!(
            "unsafe {{ {}({}) }}",
            symbol(crate_name, decl),
            passed.join(", ")
        );
        match (decl.ret.is_empty(), result_value(&decl.ret)) {
            (true, _) => {
                let _ = writeln!(out, "    {call};");
            }
            (false, None) => {
                let _ = writeln!(out, "    if {call} == 0 {{");
                let _ = writeln!(out, "        Ok(())");
                let _ = writeln!(out, "    }} else {{");
                let _ = writeln!(
                    out,
                    "        Err(day_bridge::Error::Foreign(\"{}\".into()))",
                    decl.name
                );
                let _ = writeln!(out, "    }}");
            }
            (false, Some(_)) => {
                let _ = writeln!(out, "    Ok({call})");
            }
        }
        let _ = writeln!(out, "}}\n");
    }
    out
}

/// The generated Kotlin object for one arm: the crate's Kotlin prelude, the arm itself, and a
/// `@JvmStatic` entry per declared function for JNI to call. The arm writes ordinary Kotlin —
/// `fun speak_native(text: String)` — and never sees JNI.
///
/// The name is the DECLARED one, unchanged: a bridged function is called `speak_native` in Rust,
/// Kotlin, Swift, ArkTS, JavaScript and C alike, so one grep finds the declaration and every arm.
/// It costs the JVM and Swift naming conventions; it buys never having to map a name in your head
/// or in a stack trace (docs/bridge.md "Names").
///
/// Kotlin has no `#line` equivalent, so the header names the source and the arm's line, and long
/// arms belong in their own `.kt` (docs/bridge.md "Diagnostics").
fn render_kotlin(bridge: &Bridge, arm: &Arm, crate_name: &str) -> String {
    let pkg = kotlin_package(crate_name);
    let object = kotlin_object(crate_name);
    let source = arm.source.as_deref().unwrap_or("src/lib.rs");
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// @generated by day-build from {source}:{} — edit the arm, never this file.\n\
         // Kotlin carries no line directive: an error below is at {source}:{} plus the offset.",
        arm.line, arm.body_line
    );
    let _ = writeln!(out, "package {pkg}\n");
    for prelude in bridge.preludes.iter().filter(|p| p.lang == Lang::Kotlin) {
        let _ = writeln!(out, "{}", prelude.body);
    }
    let _ = writeln!(out, "\n{}\n", arm.body.as_deref().unwrap_or(""));

    let _ = writeln!(out, "object {object} {{");
    for decl in &bridge.decls {
        let params: Vec<String> = decl
            .args
            .iter()
            .map(|(n, t)| format!("{n}: {}", kotlin_type(t)))
            .collect();
        let call = format!(
            // Fully qualified so it resolves to the arm's top-level function, never to this
            // object's member of the same name.
            "{pkg}.{}({})",
            decl.name,
            decl.args
                .iter()
                .map(|(n, _)| format!("{n} = {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        // No try/catch: on the JVM an exception IS the error channel, and JNI reports it to
        // the caller — so a Kotlin arm's failure becomes `Error::Foreign` on the Rust side with
        // no status code. C and Swift, having no such channel, use one.
        let value = result_value(&decl.ret);
        let ret = match value.as_deref() {
            None => String::new(),
            Some(ty) => format!(": {}", kotlin_type(ty)),
        };
        let _ = writeln!(out, "    @JvmStatic");
        let _ = writeln!(out, "    fun {}({}){ret} {{", decl.name, params.join(", "));
        if value.is_some() {
            let _ = writeln!(out, "        return {call}");
        } else {
            let _ = writeln!(out, "        {call}");
        }
        let _ = writeln!(out, "    }}");
    }
    let _ = writeln!(out, "}}");
    out
}

/// The generated Java class for one arm — the same shape the Kotlin emitter produces, for a
/// project whose Gradle build has no Kotlin plugin. Java needs none: `com.android.application`
/// compiles `.java` out of any `srcDir`, which is what makes this the arm that always works.
fn render_java(bridge: &Bridge, arm: &Arm, crate_name: &str) -> String {
    let pkg = kotlin_package(crate_name);
    let class = kotlin_object(crate_name);
    let source = arm.source.as_deref().unwrap_or("src/lib.rs");
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// @generated by day-build from {source}:{} — edit the arm, never this file.\n\
         // Java carries no line directive: an error below is at {source}:{} plus the offset.",
        arm.line, arm.body_line
    );
    let _ = writeln!(out, "package {pkg};\n");
    for prelude in bridge.preludes.iter().filter(|p| p.lang == Lang::Java) {
        let _ = writeln!(out, "{}", prelude.body);
    }
    let _ = writeln!(out, "\npublic final class {class} {{");
    let _ = writeln!(out, "    private {class}() {{}}\n");
    // The arm becomes the body of the class, so it writes ordinary `public static` methods and
    // never sees JNI — the same contract the Kotlin arm has.
    for line in arm.body.as_deref().unwrap_or("").lines() {
        if line.trim().is_empty() {
            let _ = writeln!(out);
        } else {
            let _ = writeln!(out, "    {line}");
        }
    }
    let _ = writeln!(out, "}}");
    out
}

/// The Rust half of a Kotlin arm: a JNI static call per function, through day-android's cached JVM
/// and its `dcall_static` helper — the same path day-part-battery's hand-written arm takes today.
fn render_jvm_rust(bridge: &Bridge, crate_name: &str) -> String {
    let class = kotlin_package(crate_name).replace('.', "/") + "/" + &kotlin_object(crate_name);
    let mut out = String::new();
    for decl in &bridge.decls {
        let args: Vec<String> = decl.args.iter().map(|(n, t)| format!("{n}: {t}")).collect();
        let ret = if decl.ret.is_empty() {
            String::new()
        } else {
            format!(" -> {}", decl.ret)
        };
        let _ = writeln!(out, "fn {}({}){ret} {{", decl.name, args.join(", "));
        let _ = writeln!(out, "    use day_android::{{DayEnv, with_env}};");
        let _ = writeln!(out, "    let called = with_env(|env| {{");
        // Marshal arguments into JNI values; a String has to become a local ref first.
        let mut jvalues: Vec<String> = Vec::new();
        for (n, t) in &decl.args {
            match t.trim() {
                "&str" => {
                    let _ = writeln!(out, "        let {n}_j = env.new_string({n}).ok()?;");
                    jvalues.push(format!("(&{n}_j).into()"));
                }
                "bool" => jvalues.push(format!(
                    "day_android::jni::objects::JValue::Bool({n} as u8)"
                )),
                "i32" => jvalues.push(format!("day_android::jni::objects::JValue::Int({n})")),
                "i64" => jvalues.push(format!("day_android::jni::objects::JValue::Long({n})")),
                "f32" => jvalues.push(format!("day_android::jni::objects::JValue::Float({n})")),
                "f64" => jvalues.push(format!("day_android::jni::objects::JValue::Double({n})")),
                _ => jvalues.push(n.clone()),
            }
        }
        let _ = writeln!(
            out,
            "        env.dcall_static({}, {}, {}, &[{}])",
            quote(&class),
            quote(&decl.name),
            quote(&jni_signature(decl)),
            jvalues.join(", ")
        );
        // Three shapes, not two: a bare unit call drops failures, `Result<(), _>` reports them,
        // and `Result<T, _>` also carries a value back.
        match (decl.ret.is_empty(), result_value(&decl.ret)) {
            (true, _) => {
                let _ = writeln!(out, "            .ok()?;");
                let _ = writeln!(out, "        Some(())");
                let _ = writeln!(out, "    }});");
                let _ = writeln!(out, "    let _ = called;");
            }
            (false, None) => {
                let _ = writeln!(out, "            .ok()?;");
                let _ = writeln!(out, "        Some(())");
                let _ = writeln!(out, "    }});");
                let _ = writeln!(
                    out,
                    "    // A Java exception fails `dcall_static`, so a throwing"
                );
                let _ = writeln!(out, "    // arm arrives here as `None`.");
                let _ = writeln!(out, "    match called {{");
                let _ = writeln!(out, "        Some(()) => Ok(()),");
                let _ = writeln!(
                    out,
                    "        None => Err(day_bridge::Error::Foreign(\"{}\".into())),",
                    decl.name
                );
                let _ = writeln!(out, "    }}");
            }
            (false, Some(ty)) => {
                let _ = writeln!(out, "            .ok()?");
                let _ = writeln!(out, "            .{}()", jvalue_accessor(&ty));
                let _ = writeln!(out, "            .ok()");
                let _ = writeln!(out, "    }});");
                let _ = writeln!(
                    out,
                    "    // A Java exception fails `dcall_static`, so a throwing"
                );
                let _ = writeln!(out, "    // arm arrives here as `None`.");
                let _ = writeln!(out, "    match called {{");
                let _ = writeln!(out, "        Some(v) => Ok(v),");
                let _ = writeln!(
                    out,
                    "        None => Err(day_bridge::Error::Foreign(\"{}\".into())),",
                    decl.name
                );
                let _ = writeln!(out, "    }}");
            }
        }
        let _ = writeln!(out, "}}\n");
    }
    out
}

/// The `T` in `Result<T, Error>`, or `None` for `Result<(), Error>` and a unit return.
fn result_value(ret: &str) -> Option<String> {
    let inner = ret
        .trim()
        .strip_prefix("Result<")
        .and_then(|r| r.strip_suffix('>'))?;
    let value = split_top(inner, ',').first()?.trim().to_string();
    (!value.is_empty() && value != "()").then_some(value)
}

/// The `JValueOwned` accessor for a v1 scalar.
fn jvalue_accessor(ty: &str) -> &'static str {
    match ty.trim() {
        "bool" => "z",
        "i32" => "i",
        "i64" => "j",
        "f32" => "f",
        "f64" => "d",
        _ => "i",
    }
}

/// `(Ljava/lang/String;)I` — the descriptor `dcall_static` needs for one declaration.
fn jni_signature(decl: &Decl) -> String {
    let args: String = decl
        .args
        .iter()
        .map(|(_, t)| match t.trim() {
            "bool" => "Z",
            "i32" => "I",
            "i64" => "J",
            "f32" => "F",
            "f64" => "D",
            "&str" => "Ljava/lang/String;",
            _ => "Ljava/lang/Object;",
        })
        .collect();
    let ret = match result_value(&decl.ret).as_deref() {
        None => "V",
        Some("bool") => "Z",
        Some("i32") => "I",
        Some("i64") => "J",
        Some("f32") => "F",
        Some("f64") => "D",
        Some(_) => "Ljava/lang/Object;",
    };
    format!("({args}){ret}")
}

fn kotlin_type(ty: &str) -> &'static str {
    match ty.trim() {
        "bool" => "Boolean",
        "i32" => "Int",
        "i64" => "Long",
        "f32" => "Float",
        "f64" => "Double",
        "&str" => "String",
        _ => "Any",
    }
}

/// `day-part-speech` → `dev.daybrite.day.bridge.day_part_speech` (docs/bridge.md "Names").
fn kotlin_package(crate_name: &str) -> String {
    format!("dev.daybrite.day.bridge.{}", crate_name.replace('-', "_"))
}

/// `day-part-speech` → `DayPartSpeechBridge`.
fn kotlin_object(crate_name: &str) -> String {
    let mut out = String::new();
    for part in crate_name.split('-') {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    format!("{out}Bridge")
}

/// The C-ABI spelling an `@_cdecl` function takes for a v1 type.
fn swift_abi_type(ty: &str) -> &'static str {
    match ty.trim() {
        "bool" | "i32" => "Int32",
        "i64" => "Int64",
        "f32" => "Float",
        "f64" => "Double",
        "&str" => "UnsafePointer<CChar>",
        _ => "UnsafeRawPointer",
    }
}

/// The Rust half of a C/C++ arm: the `extern "C"` declarations plus a safe wrapper per function,
/// converting arguments and turning a nonzero status into [`day_bridge::Error::Foreign`].
fn render_c_rust(bridge: &Bridge, arm: &Arm, crate_name: &str) -> String {
    let utf16 = arm.options.get("encoding").map(String::as_str) == Some("utf16");
    let mut out = String::new();
    let _ = writeln!(out, "unsafe extern \"C\" {{");
    for decl in &bridge.decls {
        let args: Vec<String> = decl
            .args
            .iter()
            .map(|(n, t)| format!("{n}: {}", rust_c_type(t, utf16)))
            .collect();
        let ret = if decl.ret.is_empty() { "" } else { " -> i32" };
        let _ = writeln!(
            out,
            "    fn {}({}){ret};",
            symbol(crate_name, decl),
            args.join(", ")
        );
    }
    let _ = writeln!(out, "}}\n");

    for decl in &bridge.decls {
        let args: Vec<String> = decl.args.iter().map(|(n, t)| format!("{n}: {t}")).collect();
        let ret = if decl.ret.is_empty() {
            String::new()
        } else {
            format!(" -> {}", decl.ret)
        };
        let _ = writeln!(out, "fn {}({}){ret} {{", decl.name, args.join(", "));
        let mut passed: Vec<String> = Vec::new();
        for (n, t) in &decl.args {
            match t.trim() {
                "&str" if utf16 => {
                    let _ = writeln!(
                        out,
                        "    let mut {n}_w: Vec<u16> = {n}.encode_utf16().collect();"
                    );
                    let _ = writeln!(out, "    {n}_w.push(0);");
                    passed.push(format!("{n}_w.as_ptr()"));
                }
                "&str" => {
                    let _ = writeln!(
                        out,
                        "    let Ok({n}_c) = std::ffi::CString::new({n}) else {{"
                    );
                    let _ = writeln!(
                        out,
                        "        return {};",
                        if decl.ret.is_empty() {
                            "".to_string()
                        } else {
                            "Err(day_bridge::Error::Encoding)".to_string()
                        }
                    );
                    let _ = writeln!(out, "    }};");
                    passed.push(format!("{n}_c.as_ptr()"));
                }
                "bool" => passed.push(format!("{n} as i32")),
                _ => passed.push(n.clone()),
            }
        }
        let call = format!(
            "unsafe {{ {}({}) }}",
            symbol(crate_name, decl),
            passed.join(", ")
        );
        if decl.ret.is_empty() {
            let _ = writeln!(out, "    {call};");
        } else {
            let _ = writeln!(out, "    if {call} == 0 {{");
            let _ = writeln!(out, "        Ok(())");
            let _ = writeln!(out, "    }} else {{");
            let _ = writeln!(
                out,
                "        Err(day_bridge::Error::Foreign(\"{} failed\".into()))",
                decl.name
            );
            let _ = writeln!(out, "    }}");
        }
        let _ = writeln!(out, "}}\n");
    }
    out
}

/// Whether an arm's foreign half is built by `day build` rather than by cargo. C and C++ are
/// compiled here through `cc`; Swift, Kotlin, ArkTS and JavaScript are staged into a host project
/// the CLI drives, so a bare `cargo build` has no way to link them.
fn staged_by_cli(lang: Lang) -> bool {
    matches!(
        lang,
        Lang::Swift | Lang::Kotlin | Lang::Java | Lang::ArkTs | Lang::Js
    )
}

/// The cfg naming "this crate's staged foreign half is present in the link".
const STAGED_CFG: &str = "day_bridge_staged";

/// The `cfg` an arm compiles under. `other` is the negation of every claimed platform, which is
/// how one crate's arms partition the target space without any of them naming the others.
fn arm_cfg(arm: &Arm, bridge: &Bridge) -> String {
    if arm.platforms.iter().any(|p| p == "other") {
        let mut claimed: Vec<&str> = bridge
            .arms
            .iter()
            .flat_map(|a| a.platforms.iter())
            .filter(|p| p.as_str() != "other")
            .map(|p| cfg_for(p))
            .collect();
        claimed.sort_unstable();
        claimed.dedup();
        return format!("not(any({}))", claimed.join(", "));
    }
    let mut list: Vec<&str> = arm.platforms.iter().map(|p| cfg_for(p)).collect();
    list.sort_unstable();
    list.dedup();
    let platform = if list.len() == 1 {
        list[0].to_string()
    } else {
        format!("any({})", list.join(", "))
    };
    if staged_by_cli(arm.lang) {
        format!("all({platform}, {STAGED_CFG})")
    } else {
        platform
    }
}

/// The cfg for a staged arm's platforms when the staged half is NOT in the link — a plain
/// `cargo build`, or a `day build` for a target this arm does not claim. The crate keeps
/// compiling and reports `Unsupported`, rather than failing to link a symbol nobody produced.
fn unstaged_cfg(arm: &Arm) -> String {
    let mut list: Vec<&str> = arm.platforms.iter().map(|p| cfg_for(p)).collect();
    list.sort_unstable();
    list.dedup();
    let platform = if list.len() == 1 {
        list[0].to_string()
    } else {
        format!("any({})", list.join(", "))
    };
    format!("all({platform}, not({STAGED_CFG}))")
}

/// `#[cfg(…)]` for a predicate, or `None` when it is always true — which is what the `other` arm's
/// predicate collapses to in a crate whose only arm is the fallback.
fn cfg_attr(pred: &str) -> Option<String> {
    (pred != "not(any())").then(|| format!("#[cfg({pred})]"))
}

fn render_rust(bridge: &Bridge, crate_name: &str) -> String {
    let mut out = String::from(
        "// @generated by day-build from this crate's `day_bridge::bridge!` block.\n\
         // Edit the arms in the crate source, never this file (docs/bridge.md).\n\n",
    );
    if bridge.decls.is_empty() {
        return out;
    }

    for arm in bridge.arms.iter().filter(|a| a.lang == Lang::Rust) {
        if let Some(cfg) = cfg_attr(&arm_cfg(arm, bridge)) {
            let _ = writeln!(out, "{cfg}");
        }
        let _ = writeln!(out, "#[allow(dead_code)]");
        let _ = writeln!(out, "{}\n", arm.body.as_deref().unwrap_or(""));
    }

    // Where a staged arm's foreign half is absent, the fallback stands in — same bodies as the
    // `other` arm, under the staged arm's platforms.
    let fallback: Vec<&Arm> = bridge
        .arms
        .iter()
        .filter(|a| a.lang == Lang::Rust && a.platforms.iter().any(|p| p == "other"))
        .collect();
    for arm in bridge.arms.iter().filter(|a| staged_by_cli(a.lang)) {
        for fb in &fallback {
            let _ = writeln!(out, "#[cfg({})]", unstaged_cfg(arm));
            let _ = writeln!(out, "#[allow(dead_code)]");
            let _ = writeln!(out, "{}\n", fb.body.as_deref().unwrap_or(""));
        }
    }

    // A JavaScript arm rides wasm imports rather than the C ABI: strings cross as (ptr, len).
    for arm in bridge.arms.iter().filter(|a| a.lang == Lang::Js) {
        let block = render_js_rust(bridge, crate_name);
        let cfg = arm_cfg(arm, bridge);
        for item in block.split("\n\n").filter(|i| !i.trim().is_empty()) {
            let _ = writeln!(
                out,
                "#[cfg({cfg})]\n#[allow(dead_code)]\n{}\n",
                item.trim_end()
            );
        }
    }

    // A Kotlin arm is called the other way round — Rust into the JVM — so it gets its own
    // wrappers rather than an extern block.
    for arm in bridge
        .arms
        .iter()
        .filter(|a| matches!(a.lang, Lang::Kotlin | Lang::Java))
    {
        let block = render_jvm_rust(bridge, crate_name);
        let cfg = arm_cfg(arm, bridge);
        for item in block.split("\n\n").filter(|i| !i.trim().is_empty()) {
            let _ = writeln!(
                out,
                "#[cfg({cfg})]\n#[allow(dead_code)]\n{}\n",
                item.trim_end()
            );
        }
    }

    // A C, C++ or Swift arm reaches Rust through the C ABI — Swift's `@_cdecl` exports exactly the
    // symbol C would — so one emitter covers all three. The extern block and the safe wrappers are
    // cfg-gated exactly like the rust arm, so the call site never changes.
    for arm in bridge
        .arms
        .iter()
        .filter(|a| matches!(a.lang, Lang::C | Lang::Cpp | Lang::Swift))
    {
        let block = render_c_rust(bridge, arm, crate_name);
        match cfg_attr(&arm_cfg(arm, bridge)) {
            Some(cfg) => {
                // One cfg per item, so the block stays a set of plain items.
                for item in block.split("\n\n").filter(|i| !i.trim().is_empty()) {
                    let _ = writeln!(out, "{cfg}\n#[allow(dead_code)]\n{}\n", item.trim_end());
                }
            }
            None => {
                let _ = writeln!(out, "{block}");
            }
        }
    }

    // `<fn>_support()`: what this target promises. One definition per distinct cfg, NOT per arm —
    // several arms share a cfg whenever a language needs one item per function (the rust arm
    // always does), and two definitions under one cfg would collide.
    let mut levels: Vec<(String, &'static str)> = Vec::new();
    for arm in &bridge.arms {
        let support = if arm.lang == Lang::Rust && arm.platforms.iter().any(|p| p == "other") {
            "Unsupported"
        } else if arm.options.get("support").map(String::as_str) == Some("emulated") {
            "Emulated"
        } else {
            "Native"
        };
        let cfg = arm_cfg(arm, bridge);
        if !levels.iter().any(|(seen, _)| seen == &cfg) {
            levels.push((cfg, support));
        }
        if staged_by_cli(arm.lang) {
            let cfg = unstaged_cfg(arm);
            if !levels.iter().any(|(seen, _)| seen == &cfg) {
                levels.push((cfg, "Unsupported"));
            }
        }
    }
    for decl in &bridge.decls {
        for (cfg, support) in &levels {
            if let Some(attr) = cfg_attr(cfg) {
                let _ = writeln!(out, "{attr}");
            }
            let _ = writeln!(
                out,
                "#[allow(dead_code)]\npub(crate) fn {}_support() -> day_bridge::Support {{\n    \
                 day_bridge::Support::{support}\n}}\n",
                decl.name
            );
        }
    }
    out
}

/// The file name an arm's adapter is staged under, derived from the crate and the platforms it
/// claims so two crates' adapters can share one directory.
fn generated_name(arm: &Arm, crate_name: &str) -> String {
    // javac requires a public class to sit in a file named after it, so a Java arm takes the class
    // name and nothing else. Every other language's file name is free, and encodes the platforms so
    // two crates' adapters can share one staging directory.
    if arm.lang == Lang::Java {
        return format!("{}.java", kotlin_object(crate_name));
    }
    let ext = match arm.lang {
        Lang::Swift => "swift",
        Lang::Kotlin => "kt",
        Lang::Java => "java",
        Lang::ArkTs => "ets",
        Lang::Js => "js",
        Lang::Cpp => "cpp",
        Lang::C => "c",
        Lang::Rust => "rs",
    };
    format!("{crate_name}-{}.{ext}", arm.platforms.join("-"))
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Touch only when the bytes change (DESIGN §17.5): the native builds behind generated sources key
/// on mtime, so an unconditional write recompiles them on every `day build`.
fn write_if_changed(path: &Path, content: &str) -> Result<(), String> {
    if std::fs::read(path).is_ok_and(|cur| cur == content.as_bytes()) {
        return Ok(());
    }
    std::fs::write(path, content).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEECH: &str = r###"
day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        fn speak_native(text: &str) -> Result<(), day_bridge::Error>;
        fn stop_native();
    }

    #[day_bridge::prelude(kotlin)]
    kotlin!(r#"
        import android.speech.tts.TextToSpeech
    "#);

    #[day_bridge::impl(kotlin, platforms = [android])]
    kotlin!(r#"
        fun speak_native(text: String) { engine?.speak(text) }
    "#);

    #[day_bridge::impl(rust, platforms = [other])]
    fn speak_native(_text: &str) -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn stop_native() {}
}
"###;

    fn parse(src: &str) -> Bridge {
        let mut b = Bridge::default();
        parse_into(src, "src/lib.rs", &mut b).expect("parse");
        b
    }

    #[test]
    fn parses_declarations_arms_and_preludes() {
        let b = parse(SPEECH);
        assert_eq!(b.decls.len(), 2);
        assert_eq!(b.decls[0].name, "speak_native");
        assert_eq!(b.decls[0].args, vec![("text".into(), "&str".into())]);
        assert_eq!(b.decls[0].ret, "Result<(), day_bridge::Error>");
        assert_eq!(b.decls[1].name, "stop_native");
        assert!(b.decls[1].args.is_empty());
        assert_eq!(b.preludes.len(), 1);
        assert_eq!(b.preludes[0].lang, Lang::Kotlin);
        assert_eq!(b.arms.len(), 3);
        assert_eq!(b.arms[0].lang, Lang::Kotlin);
        assert_eq!(b.arms[0].platforms, vec!["android".to_string()]);
        assert!(
            b.arms[0]
                .body
                .as_deref()
                .unwrap()
                .starts_with("fun speak_native")
        );
    }

    #[test]
    fn validates_and_renders_the_rust_arm() {
        let b = parse(SPEECH);
        validate(&b).expect("valid");
        let rust = render_rust(&b, "day-part-speech");
        // The android arm is claimed, so `other` excludes it.
        assert!(
            rust.contains("#[cfg(not(any(target_os = \"android\")))]"),
            "{rust}"
        );
        assert!(rust.contains("fn speak_native(_text: &str)"));
        assert!(rust.contains("pub(crate) fn speak_native_support()"));
        assert!(
            rust.contains("Support::Native"),
            "the android arm reports Native"
        );
        assert!(
            rust.contains("Support::Unsupported"),
            "the fallback reports Unsupported"
        );
    }

    #[test]
    fn rejects_a_type_outside_the_table() {
        let b = parse(
            r###"
            day_bridge::bridge! {
                #[day_bridge::declare]
                extern "day" { fn f(x: Option<i32>); }
                #[day_bridge::impl(rust, platforms = [other])]
                fn f(_x: Option<i32>) {}
            }
            "###,
        );
        let err = validate(&b).unwrap_err();
        assert!(err.contains("does not cross a bridge"), "{err}");
    }

    #[test]
    fn rejects_a_missing_fallback() {
        let b = parse(
            r###"
            day_bridge::bridge! {
                #[day_bridge::declare]
                extern "day" { fn f(); }
                #[day_bridge::impl(kotlin, platforms = [android])]
                kotlin!(r#" fun f() {} "#);
            }
            "###,
        );
        assert!(validate(&b).unwrap_err().contains("no `other` arm"));
    }

    #[test]
    fn rejects_two_arms_claiming_one_platform() {
        let b = parse(
            r###"
            day_bridge::bridge! {
                #[day_bridge::declare]
                extern "day" { fn f(); }
                #[day_bridge::impl(kotlin, platforms = [android])]
                kotlin!(r#" fun f() {} "#);
                #[day_bridge::impl(js, platforms = [android])]
                js!(r#" export function f() {} "#);
                #[day_bridge::impl(rust, platforms = [other])]
                fn f() {}
            }
            "###,
        );
        assert!(validate(&b).unwrap_err().contains("already claimed"));
    }

    #[test]
    fn rejects_a_package_line_in_a_prelude() {
        let mut b = Bridge::default();
        let err = parse_into(
            r###"
            day_bridge::bridge! {
                #[day_bridge::prelude(kotlin)]
                kotlin!(r#"
                    package dev.example.mine
                "#);
            }
            "###,
            "src/lib.rs",
            &mut b,
        )
        .unwrap_err();
        assert!(err.contains("belongs to the generator"), "{err}");
    }

    #[test]
    fn renders_the_kotlin_adapter_and_its_jni_side() {
        let b = parse(SPEECH);
        let arm = b.arms.iter().find(|a| a.lang == Lang::Kotlin).unwrap();
        let kt = render_kotlin(&b, arm, "day-part-speech");
        assert!(
            kt.contains("package dev.daybrite.day.bridge.day_part_speech"),
            "{kt}"
        );
        assert!(
            kt.contains("import android.speech.tts.TextToSpeech"),
            "prelude hoisted:\n{kt}"
        );
        assert!(kt.contains("object DayPartSpeechBridge {"), "{kt}");
        // The entry calls the arm's top-level function by its package-qualified name, so it can
        // never recurse into the object member of the same name.
        assert!(
            kt.contains("dev.daybrite.day.bridge.day_part_speech.speak_native(text = text)"),
            "the declared name is used verbatim, not camel-cased:\n{kt}"
        );
        // No status code and no catch: an exception is the JVM's error channel, and JNI hands it
        // to the caller, which the Rust side turns into `Error::Foreign`.
        assert!(
            !kt.contains("catch ("),
            "the arm's exceptions cross as-is:\n{kt}"
        );

        let rust = render_jvm_rust(&b, "day-part-speech");
        assert!(
            rust.contains(
                "env.dcall_static(\"dev/daybrite/day/bridge/day_part_speech/DayPartSpeechBridge\", \"speak_native\", \"(Ljava/lang/String;)V\""
            ),
            "{rust}"
        );
        assert!(
            rust.contains("let text_j = env.new_string(text).ok()?;"),
            "{rust}"
        );
        assert!(
            rust.contains("Err(day_bridge::Error::Foreign(\"speak_native\".into()))"),
            "a failed call becomes Foreign:\n{rust}"
        );
    }

    #[test]
    fn renders_the_java_adapter_the_jvm_side_shares() {
        // Java and Kotlin arms produce the same class, the same method names, and the same JNI
        // descriptors — only the syntax and the file name differ (docs/bridge.md "Android").
        let b = parse(&SPEECH.replace("kotlin", "java"));
        let arm = b.arms.iter().find(|a| a.lang == Lang::Java).unwrap();
        let java = render_java(&b, arm, "day-part-speech");
        assert!(
            java.contains("package dev.daybrite.day.bridge.day_part_speech;"),
            "{java}"
        );
        assert!(
            java.contains("import android.speech.tts.TextToSpeech"),
            "prelude hoisted:\n{java}"
        );
        assert!(
            java.contains("public final class DayPartSpeechBridge {"),
            "{java}"
        );
        assert!(
            java.contains("speak_native"),
            "the declared name is used verbatim:\n{java}"
        );
        // javac requires the file to be named after its public class; every other language's
        // adapter encodes the platforms instead.
        assert_eq!(
            adapter_name(arm, "day-part-speech"),
            "DayPartSpeechBridge.java"
        );

        // The Rust half is language-blind: one JNI call, whichever language wrote the class.
        let rust = render_jvm_rust(&b, "day-part-speech");
        assert!(
            rust.contains(
                "env.dcall_static(\"dev/daybrite/day/bridge/day_part_speech/DayPartSpeechBridge\", \"speak_native\", \"(Ljava/lang/String;)V\""
            ),
            "{rust}"
        );
    }

    /// A C++ arm's exported adapters must not be mangled — Rust links the plain symbol — and a
    /// UTF-16 arm must be handed `char16_t*` with the conversion happening on the Rust side.
    #[test]
    fn a_cpp_arm_exports_unmangled_utf16_adapters() {
        let b = parse(
            r###"
            day_bridge::bridge! {
                #[day_bridge::declare]
                extern "day" {
                    fn speak_native(text: &str) -> Result<(), day_bridge::Error>;
                    fn stop_native();
                }
                #[day_bridge::impl(cpp, platforms = [windows], encoding = "utf16", link = ["ole32", "sapi"])]
                cpp!(r#" int32_t speak_native(const char16_t* t) { return 0; } "#);
                #[day_bridge::impl(rust, platforms = [other])]
                fn speak_native(_text: &str) -> Result<(), day_bridge::Error> {
                    Err(day_bridge::Error::Unsupported)
                }
                #[day_bridge::impl(rust, platforms = [other])]
                fn stop_native() {}
            }
            "###,
        );
        validate(&b).expect("valid");
        let arm = b.arms.iter().find(|a| a.lang == Lang::Cpp).unwrap();
        let cpp = render_c(&b, arm, "day-part-speech");
        assert!(cpp.contains("extern \"C\" {"), "{cpp}");
        assert!(
            cpp.contains(
                "int32_t day_bridge_day_part_speech_speak_native(const char16_t* text) { return speak_native(text); }"
            ),
            "{cpp}"
        );

        let rust = render_c_rust(&b, arm, "day-part-speech");
        assert!(
            rust.contains("fn day_bridge_day_part_speech_speak_native(text: *const u16) -> i32;"),
            "{rust}"
        );
        assert!(
            rust.contains("let mut text_w: Vec<u16> = text.encode_utf16().collect();")
                && rust.contains("text_w.push(0);"),
            "the wide string is built and NUL-terminated in Rust:\n{rust}"
        );
    }

    /// The same generator, without the C++ rules: a C arm is already unmangled, and its `&str`
    /// stays `const char*`.
    #[test]
    fn a_c_arm_takes_no_extern_c_wrapper() {
        let b = parse(
            r###"
            day_bridge::bridge! {
                #[day_bridge::declare]
                extern "day" { fn f(text: &str); }
                #[day_bridge::impl(c, platforms = [linux])]
                c!(r#" void f(const char* t) {} "#);
                #[day_bridge::impl(rust, platforms = [other])]
                fn f(_text: &str) {}
            }
            "###,
        );
        let arm = b.arms.iter().find(|a| a.lang == Lang::C).unwrap();
        let c = render_c(&b, arm, "day-part-demo");
        assert!(!c.contains("extern \"C\""), "{c}");
        assert!(c.contains("(const char* text)"), "{c}");
    }

    #[test]
    fn jni_descriptors_match_the_declaration() {
        let unit = Decl {
            name: "stop".into(),
            args: vec![],
            ret: String::new(),
            line: 1,
        };
        assert_eq!(jni_signature(&unit), "()V");
        let mixed = Decl {
            name: "f".into(),
            args: vec![
                ("a".into(), "&str".into()),
                ("b".into(), "i64".into()),
                ("c".into(), "bool".into()),
            ],
            ret: "Result<(), day_bridge::Error>".into(),
            line: 1,
        };
        // `Result<(), Error>` returns nothing: the error rides the exception channel.
        assert_eq!(jni_signature(&mixed), "(Ljava/lang/String;JZ)V");

        // A value return carries its own descriptor.
        let valued = Decl {
            name: "level".into(),
            args: vec![],
            ret: "Result<i32, day_bridge::Error>".into(),
            line: 1,
        };
        assert_eq!(jni_signature(&valued), "()I");
        assert_eq!(result_value(&valued.ret).as_deref(), Some("i32"));
    }

    #[test]
    fn the_cli_can_parse_a_crate_without_cargo() {
        // The staging half reads sources, so a foreign adapter is renderable with no OUT_DIR and
        // no build script having run (docs/bridge.md "What the build does").
        let b = parse(SPEECH);
        let kotlin = b.arms.iter().find(|a| a.lang == Lang::Kotlin).unwrap();
        assert_eq!(
            adapter_name(kotlin, "day-part-speech"),
            "day-part-speech-android.kt"
        );
    }
}
