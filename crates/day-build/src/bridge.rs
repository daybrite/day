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

/// Every option an arm may carry beside `platforms` (docs/bridge.md). Closed, so a typo fails the
/// build instead of being ignored.
const ARM_OPTIONS: &[&str] = &["src", "link", "pkg_config", "encoding", "support", "sdk"];

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

/// What a completion may carry back (`Done<T>`): nothing, a scalar, a string, or bytes.
fn completion_value_ok(ty: &str) -> bool {
    let ty = ty.trim();
    ty == "()" || SCALARS.contains(&ty) || ty == "String" || ty == "Vec<u8>"
}

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

impl Decl {
    /// The callback handle this function carries, when its last argument is
    /// `day_bridge::Done<T>` or `day_bridge::Emit<T>`: `(argument name, T)`. A function with one is
    /// asynchronous — it returns once the platform ACCEPTED the request and answers later through
    /// the token: once for a `Done` (docs/bridge.md "Callbacks"), any number of times and then an
    /// end for an `Emit` ("Streams"). [`Decl::is_stream`] tells the two apart.
    pub fn done(&self) -> Option<(&str, String)> {
        let (name, ty) = self.args.last()?;
        let ty = ty.trim().trim_start_matches("day_bridge::");
        let inner = ty
            .strip_prefix("Done<")
            .or_else(|| ty.strip_prefix("Emit<"))?
            .strip_suffix('>')?;
        Some((name.as_str(), inner.trim().to_string()))
    }

    /// Whether the handle is a repeating `Emit<T>` stream rather than a one-shot `Done<T>`.
    pub fn is_stream(&self) -> bool {
        self.args.last().is_some_and(|(_, t)| {
            t.trim()
                .trim_start_matches("day_bridge::")
                .starts_with("Emit<")
        })
    }

    /// The arguments an arm marshals: every declared one except the completion handle, which
    /// crosses as a token the generator adds itself.
    pub fn plain_args(&self) -> &[(String, String)] {
        match self.done() {
            Some(_) => &self.args[..self.args.len() - 1],
            None => &self.args,
        }
    }
}

/// One implementation of the declared API for a set of platforms.
#[derive(Clone, Debug)]
pub struct Arm {
    pub lang: Lang,
    pub platforms: Vec<String>,
    /// Inline body (the raw string's contents), or `None` when the arm names a file.
    pub body: Option<String>,
    /// The arm's file-level preamble — imports only, and only where the language needs them
    /// outside the body (a JVM arm's body sits inside the generated class). Per ARM, not per
    /// language: imports are usually platform-specific, and two arms of one language claiming
    /// different platforms must not receive each other's.
    pub prelude: Option<String>,
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

/// Everything one crate declares.
#[derive(Default, Debug)]
pub struct Bridge {
    pub decls: Vec<Decl>,
    pub arms: Vec<Arm>,
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
        // Forward slashes always. This string is baked into every generated artifact — the C
        // `#line`, Swift's `#sourceLocation`, the Kotlin header, the `@generated` banner — so a
        // host separator would make the generated files differ byte-for-byte between Windows and
        // everywhere else, against the determinism this module already sorts its inputs for.
        // Windows-only: a backslash is a legal character in a POSIX filename.
        #[cfg(windows)]
        let rel = rel.replace('\\', "/");
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

    // Swift arms are compiled into the generated DayPieces package by the platform's
    // xcodebuild, not by cargo: this writes the adapter and the manifest points at it
    // (docs/bridge.md).
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
            "prelude" => {
                return Err(format!(
                    "line {line}: a standalone `prelude` attribute no longer exists — write it as \
                     `lang!(prelude = r#\" … \"#, body = r#\" … \"#)` on the arm it belongs to \
                     (docs/bridge.md \"The file\")"
                ));
            }
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
    // Comments come out BEFORE the split: a `;` inside a doc comment would otherwise end the
    // declaration early and leave prose where the next `fn` should be.
    let block = strip_comments(&rest[open + 1..close]);
    for raw in block.split(';') {
        let sig = raw.trim();
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
            // A misspelled key used to be accepted and then ignored, which is the worst outcome:
            // `linkk = ["sapi"]` links nothing and surfaces as an undefined symbol somewhere else
            // entirely. The known set is small and closed, so an unknown key is an error.
            if !ARM_OPTIONS.contains(&key) {
                return Err(format!(
                    "line {line}: unknown arm option `{key}` (expected one of {})",
                    ARM_OPTIONS.join(", ")
                ));
            }
            if key == "encoding" && value != "utf8" && value != "utf16" {
                return Err(format!(
                    "line {line}: `encoding = \"{value}\"` — expected \"utf8\" or \"utf16\""
                ));
            }
            if key == "support" && value != "native" && value != "emulated" {
                return Err(format!(
                    "line {line}: `support = \"{value}\"` — expected \"native\" or \"emulated\""
                ));
            }
            // An ArkTS arm over a Huawei HarmonyOS SDK kit (Core Speech, Push, Map, …) cannot
            // compile against the public OpenHarmony SDK; `sdk = "hms"` lets `day build` stage
            // it only where that SDK is present (docs/bridge.md "Platform selection").
            if key == "sdk" && value != "openharmony" && value != "hms" {
                return Err(format!(
                    "line {line}: `sdk = \"{value}\"` — expected \"openharmony\" or \"hms\""
                ));
            }
            if key == "sdk" && lang != Lang::ArkTs {
                return Err(format!(
                    "line {line}: `sdk` names a HarmonyOS SDK and applies to an `arkts` arm only"
                ));
            }
            options.insert(key.to_string(), value.to_string());
        }
    }
    if platforms.is_empty() {
        return Err(format!("line {line}: an arm must name `platforms = [ … ]`"));
    }

    // A rust arm is ordinary Rust captured verbatim; every other language rides one or two named
    // raw strings, or names a file with `src = "…"`.
    let (prelude, body, consumed, body_line) = if lang == Lang::Rust {
        let (body, consumed) = rust_item_after(rest, line)?;
        (None, Some(body), consumed, line)
    } else if options.contains_key("src") {
        (None, None, 0, line)
    } else {
        let call = macro_call_after(rest, line)?;
        (
            call.prelude,
            Some(call.body),
            call.consumed,
            line + call.body_skipped + 1,
        )
    };
    if let Some(text) = &prelude {
        for bad in ["package ", "namespace ", "module "] {
            if text.lines().any(|l| l.trim_start().starts_with(bad)) {
                return Err(format!(
                    "line {line}: a `{}` line belongs to the generator, not a prelude — daybridge \
                     derives it from the crate name (docs/bridge.md \"Names\")",
                    bad.trim()
                ));
            }
        }
    }

    bridge.arms.push(Arm {
        lang,
        platforms,
        body,
        prelude,
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

/// One `lang!( … )` invocation's raw-string arguments.
struct MacroCall {
    /// `prelude = r#"…"#`, when the arm has one.
    prelude: Option<String>,
    /// The arm itself: the sole argument, or `body = r#"…"#`.
    body: String,
    /// Bytes of `rest` the whole invocation consumed.
    consumed: usize,
    /// Newlines before the body's first character, so `#line` can be exact.
    body_skipped: usize,
}

/// Parse the `lang!( … )` that follows an `impl` attribute.
///
/// Two spellings, because most arms need no preamble and should not pay for one:
///
/// ```text
/// swift!(r#" … "#)                                  // body only
/// swift!(prelude = r#" … "#, body = r#" … "#)       // both, in either order
/// ```
///
/// Any hash count is accepted for each raw string, because one is not always enough: an arm
/// containing the two characters `"#` — `document.querySelector("#speech")` is the everyday
/// example — ends an `r#"…"#` string early and takes the rest of the file with it. Writing that
/// arm as `r##"…"##` is the fix, and it only works if the parser counts hashes as rustc does.
fn macro_call_after(rest: &str, line: usize) -> Result<MacroCall, String> {
    let open = rest.find('(').ok_or_else(|| {
        format!("line {line}: expected a language macro, e.g. `java!(r#\" … \"#)`")
    })?;
    let close = match_delim(rest, open, b'(', b')')
        .ok_or_else(|| format!("line {line}: unterminated language macro"))?;

    let mut prelude: Option<String> = None;
    let mut body: Option<(String, usize)> = None;
    let mut at = open + 1;
    while let Some(rel) = find_raw_open(&rest[at..close]) {
        let r = at + rel;
        let hashes = rest[r + 1..].bytes().take_while(|b| *b == b'#').count();
        let start = r + 1 + hashes + 1; // `r` + hashes + `"`
        let terminator = format!("\"{}", "#".repeat(hashes));
        let end = rest[start..close]
            .find(&terminator)
            .map(|i| start + i)
            .ok_or_else(|| format!("line {line}: unterminated raw string"))?;
        // Whatever sits between the previous argument and this raw string names it.
        let key = rest[at..r]
            .trim()
            .trim_start_matches(',')
            .trim()
            .trim_end_matches('=')
            .trim()
            .to_string();
        let text = dedent(&rest[start..end]);
        match key.as_str() {
            "prelude" => prelude = Some(text),
            "body" | "" => body = Some((text, rest[..start].matches('\n').count())),
            other => {
                return Err(format!(
                    "line {line}: unknown argument `{other}` — a language macro takes `prelude` \
                     and `body` (docs/bridge.md \"The file\")"
                ));
            }
        }
        at = end + terminator.len();
    }

    let (body, body_skipped) = body.ok_or_else(|| {
        format!("line {line}: expected a raw-string body, e.g. `java!(r#\" … \"#)`")
    })?;
    Ok(MacroCall {
        prelude,
        body,
        consumed: close + 1,
        body_skipped,
    })
}

/// Offset of the `r` opening the next raw string (`r"`, `r#"`, `r##"`, …), or `None`.
fn find_raw_open(text: &str) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'r' {
            let mut j = i + 1;
            while j < b.len() && b[j] == b'#' {
                j += 1;
            }
            if b.get(j) == Some(&b'"') {
                return Some(i);
            }
        }
        i += 1;
    }
    None
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
            // A raw string of any hash count, skipped whole: its contents are another language and
            // may hold unbalanced braces, quotes, and `//` (docs/bridge.md).
            b'r' if raw_open_hashes(b, i).is_some() => {
                let hashes = raw_open_hashes(b, i).unwrap_or(0);
                let terminator: Vec<u8> = std::iter::once(b'"')
                    .chain(std::iter::repeat_n(b'#', hashes))
                    .collect();
                i += 1 + hashes + 1;
                while i < b.len() && !b[i..].starts_with(&terminator) {
                    i += 1;
                }
                i += terminator.len();
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

/// The hash count of the raw string opening at `i` (`r"` is 0, `r#"` is 1, …), or `None` when this
/// `r` does not open one.
fn raw_open_hashes(b: &[u8], i: usize) -> Option<usize> {
    if b.get(i) != Some(&b'r') {
        return None;
    }
    let mut j = i + 1;
    while b.get(j) == Some(&b'#') {
        j += 1;
    }
    (b.get(j) == Some(&b'"')).then_some(j - i - 1)
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
        // `indent` is a byte count of ASCII-space/tab indentation in the common case; the boundary
        // check keeps a line indented with a multi-byte Unicode space from panicking the build.
        .map(|l| {
            if l.len() >= indent && l.is_char_boundary(indent) {
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
        out.push_str(if line.len() >= indent && line.is_char_boundary(indent) {
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
        // A completion handle is last, alone, and carries a value the table can spell; the
        // function's own return is then the ACCEPTED/failed-to-start status, never a value.
        let done_positions: Vec<usize> = decl
            .args
            .iter()
            .enumerate()
            .filter(|(_, (_, t))| {
                let t = t.trim().trim_start_matches("day_bridge::");
                t.starts_with("Done<") || t.starts_with("Emit<")
            })
            .map(|(i, _)| i)
            .collect();
        if let Some(&first) = done_positions.first() {
            if done_positions.len() > 1 || first + 1 != decl.args.len() {
                return Err(format!(
                    "line {}: `{}` takes a `Done<T>` or `Emit<T>` that is not its single, last \
                     argument (docs/bridge.md \"Callbacks\")",
                    decl.line, decl.name
                ));
            }
            let (_, value) = decl.done().unwrap_or_default();
            if !completion_value_ok(&value) {
                return Err(format!(
                    "line {}: `{}` completes with `{value}`, which is outside the completion \
                     table — `()`, a scalar, `String`, or `Vec<u8>` (docs/bridge.md \"Callbacks\")",
                    decl.line, decl.name
                ));
            }
            if result_value(&decl.ret).is_some() {
                return Err(format!(
                    "line {}: `{}` both returns a value and completes one; an asynchronous \
                     function returns `Result<(), day_bridge::Error>` (accepted or not) and \
                     answers through its `Done` (docs/bridge.md \"Callbacks\")",
                    decl.line, decl.name
                ));
            }
        }
        for (arg, ty) in decl.plain_args() {
            check_type(ty, true)
                .map_err(|e| format!("line {}: `{}`'s `{arg}`: {e}", decl.line, decl.name))?;
        }
        // A string or bytes argument crosses as a pointer and a length named after it, so no other
        // argument may take either name: the generated JavaScript, C and Swift would declare that
        // parameter twice.
        for (buffer, ty) in decl.plain_args() {
            if !matches!(ty.trim(), "&str" | "&[u8]") {
                continue;
            }
            let reserved = [format!("{buffer}_ptr"), format!("{buffer}_len")];
            if let Some((arg, _)) = decl.args.iter().find(|(name, _)| reserved.contains(name)) {
                return Err(format!(
                    "line {}: `{}`'s `{arg}` takes the name the generated code gives `{buffer}`'s \
                     pointer or length; rename it (docs/bridge.md \"Types\")",
                    decl.line, decl.name
                ));
            }
        }
        if !decl.ret.is_empty() {
            // Every generator spells a returned value as `Result<T, Error>`; a bare `-> bool`
            // would pass the type check below and then emit adapters that only fail to compile
            // in a staged mobile or web build.
            let Some(inner) = decl
                .ret
                .strip_prefix("Result<")
                .and_then(|r| r.strip_suffix('>'))
            else {
                return Err(format!(
                    "line {}: `{}` returns `{}`; a bridged function returns nothing or \
                     `Result<T, day_bridge::Error>`, since any arm can fail (docs/bridge.md \
                     \"Types\")",
                    decl.line, decl.name, decl.ret
                ));
            };
            let inner = split_top(inner, ',').first().cloned().unwrap_or_default();
            let inner = inner.trim();
            if !inner.is_empty() && inner != "()" {
                check_type(inner, false)
                    .map_err(|e| format!("line {}: `{}`'s return: {e}", decl.line, decl.name))?;
            }
        }
    }

    // The v1 type table is the DESIGN surface; `implemented` is the built one. A gap between them
    // must fail here rather than emit an adapter that cannot compile — or, worse, one that
    // compiles and marshals the wrong bytes.
    for arm in &bridge.arms {
        for decl in &bridge.decls {
            for (arg, ty) in decl.plain_args() {
                if !implemented(arm.lang, ty, true) {
                    return Err(format!(
                        "line {}: `{}`'s `{arg}: {ty}` is in the type table but the {} generator \
                         does not marshal it yet (docs/bridge.md \"Types\")",
                        arm.line,
                        decl.name,
                        arm.lang.key()
                    ));
                }
            }
            let Some(ty) = result_value(&decl.ret) else {
                continue;
            };
            if !implemented(arm.lang, &ty, false) {
                return Err(match arm.lang {
                    // The status code owns the return slot on these, and v1 has no spelling for
                    // an out-parameter.
                    Lang::C | Lang::Cpp | Lang::Swift => format!(
                        "line {}: `{}` returns a value, which the {} arm cannot express yet — \
                         return `Result<(), day_bridge::Error>` there, or split the value into \
                         its own function",
                        arm.line,
                        decl.name,
                        arm.lang.key()
                    ),
                    _ => format!(
                        "line {}: `{}` returns `{ty}`, which the {} generator does not marshal \
                         yet (docs/bridge.md \"Types\")",
                        arm.line,
                        decl.name,
                        arm.lang.key()
                    ),
                });
            }
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

/// Whether `lang`'s generator marshals `ty` today, as an argument or as the value of a
/// `Result<T, Error>` return. Narrower than [`check_type`] on purpose: that one polices the v1
/// design surface, this one polices what is actually built (docs/bridge.md "Types").
fn implemented(lang: Lang, ty: &str, argument: bool) -> bool {
    let ty = ty.trim();
    if lang == Lang::Rust {
        return true;
    }
    if argument {
        return SCALARS.contains(&ty) || ty == "&str" || ty == "&[u8]";
    }
    match lang {
        // The JVM's error channel is the exception, so the return slot is free for a value.
        Lang::Kotlin | Lang::Java => SCALARS.contains(&ty) || ty == "String",
        // The JS import and the ArkUI shim's dispatcher both hand a scalar straight back.
        Lang::Js | Lang::ArkTs => SCALARS.contains(&ty),
        // C, C++ and Swift spend the return slot on the status code.
        _ => false,
    }
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

/// The C parameter list a completion value crosses as: a scalar by value, a string by pointer,
/// bytes by pointer and length, nothing for `()`.
fn c_value_params(ty: &str, utf16: bool) -> Vec<(String, String)> {
    match ty.trim() {
        "()" => Vec::new(),
        "String" => vec![("value".into(), c_type("&str", utf16).into())],
        "Vec<u8>" => vec![
            ("value".into(), "const uint8_t*".into()),
            ("value_len".into(), "size_t".into()),
        ],
        t => vec![("value".into(), c_type(t, false).into())],
    }
}

/// The Rust spelling of [`c_value_params`].
fn rust_value_params(ty: &str, utf16: bool) -> Vec<(String, String)> {
    match ty.trim() {
        "()" => Vec::new(),
        "String" => vec![("value".into(), rust_c_type("&str", utf16).into())],
        "Vec<u8>" => vec![
            ("value".into(), "*const u8".into()),
            ("value_len".into(), "usize".into()),
        ],
        t => vec![("value".into(), rust_c_type(t, false).into())],
    }
}

/// The zero a failing completion passes in the value slot(s).
fn c_value_zeros(ty: &str) -> Vec<&'static str> {
    match ty.trim() {
        "()" => Vec::new(),
        "String" => vec!["NULL"],
        "Vec<u8>" => vec!["NULL", "0"],
        "f32" | "f64" => vec!["0.0"],
        _ => vec!["0"],
    }
}

/// The Rust expression turning a C-ABI completion value into `Result<T, Error>`.
fn rust_value_from_c(ty: &str, utf16: bool) -> String {
    match ty.trim() {
        "()" => "Ok(())".into(),
        "bool" => "Ok(value != 0)".into(),
        "String" if utf16 => "{\n                let mut n = 0usize;\n                // SAFETY: the arm passes a NUL-terminated UTF-16 string or null.\n                while !value.is_null() && unsafe { *value.add(n) } != 0 {\n                    n += 1;\n                }\n                let units = if value.is_null() { &[][..] } else { unsafe { std::slice::from_raw_parts(value, n) } };\n                String::from_utf16(units).map_err(|_| day_bridge::Error::Encoding)\n            }".into(),
        "String" => "if value.is_null() {\n                Ok(String::new())\n            } else {\n                // SAFETY: the arm passes a NUL-terminated string or null.\n                unsafe { std::ffi::CStr::from_ptr(value) }\n                    .to_str()\n                    .map(str::to_owned)\n                    .map_err(|_| day_bridge::Error::Encoding)\n            }".into(),
        "Vec<u8>" => "if value.is_null() || value_len == 0 {\n                Ok(Vec::new())\n            } else {\n                // SAFETY: the arm passes `value_len` readable bytes.\n                Ok(unsafe { std::slice::from_raw_parts(value, value_len) }.to_vec())\n            }".into(),
        _ => "Ok(value)".into(),
    }
}

/// The exported symbol a foreign arm completes a `Done` through, or delivers an `Emit` stream's
/// items through (docs/bridge.md "Callbacks", "Streams").
fn complete_symbol(crate_name: &str, decl: &Decl) -> String {
    format!(
        "day_bridge_{}_{}_{}",
        callback_verb(decl),
        crate_name.replace('-', "_"),
        decl.name
    )
}

/// `complete` for a `Done`, `emit` for an `Emit`: the symbol's verb and the helper an arm calls
/// with a value.
fn callback_verb(decl: &Decl) -> &'static str {
    if decl.is_stream() { "emit" } else { "complete" }
}

/// The name of the generated `static` registry for one `Done` or `Emit` declaration.
fn registry_name(decl: &Decl) -> String {
    let kind = if decl.is_stream() { "EMIT" } else { "DONE" };
    format!("DAY_BRIDGE_{kind}_{}", decl.name.to_uppercase())
}

/// How a generated export hands its outcome to the registry: `complete` resolves a `Done`,
/// `deliver` interprets the status for an `Emit` (value, failure, end).
fn registry_call(decl: &Decl, token: &str) -> String {
    if decl.is_stream() {
        format!("deliver({token}, status, outcome)")
    } else {
        format!("complete({token}, outcome)")
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
    let _ = writeln!(out, "#include <stddef.h>");
    if let Some(prelude) = &arm.prelude {
        let _ = writeln!(out, "{}", prelude);
    }
    // The completion symbols an asynchronous arm answers through, declared before the arm so
    // it can call `<fn>_complete(done, value)` / `<fn>_fail(done)` like any local function
    // (docs/bridge.md "Callbacks"). Rust exports the underlying symbol.
    for decl in &bridge.decls {
        let Some((_, value)) = decl.done() else {
            continue;
        };
        let sym = complete_symbol(crate_name, decl);
        let value_params = c_value_params(&value, utf16);
        let mut params = vec!["uint64_t done".to_string(), "int32_t status".to_string()];
        params.extend(value_params.iter().map(|(n, t)| format!("{t} {n}")));
        let linkage = if arm.lang == Lang::Cpp {
            "extern \"C\" "
        } else {
            "extern "
        };
        let _ = writeln!(out, "{linkage}void {sym}({});", params.join(", "));
        let ok_params: Vec<String> = std::iter::once("uint64_t done".to_string())
            .chain(value_params.iter().map(|(n, t)| format!("{t} {n}")))
            .collect();
        let ok_args: Vec<String> = std::iter::once("done".to_string())
            .chain(std::iter::once("0".to_string()))
            .chain(value_params.iter().map(|(n, _)| n.clone()))
            .collect();
        let _ = writeln!(
            out,
            "static void {}_{}({}) {{ {sym}({}); }}",
            decl.name,
            callback_verb(decl),
            ok_params.join(", "),
            ok_args.join(", ")
        );
        let zeros: Vec<String> = c_value_zeros(&value)
            .into_iter()
            .map(String::from)
            .collect();
        let fail_args: Vec<String> = std::iter::once("done".to_string())
            .chain(std::iter::once("1".to_string()))
            .chain(zeros.iter().cloned())
            .collect();
        let _ = writeln!(
            out,
            "static void {}_fail(uint64_t done) {{ {sym}({}); }}",
            decl.name,
            fail_args.join(", ")
        );
        if decl.is_stream() {
            let end_args: Vec<String> = std::iter::once("done".to_string())
                .chain(std::iter::once("2".to_string()))
                .chain(zeros.iter().cloned())
                .collect();
            let _ = writeln!(
                out,
                "static void {}_end(uint64_t done) {{ {sym}({}); }}",
                decl.name,
                end_args.join(", ")
            );
        }
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
        let mut params: Vec<String> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        for (n, t) in decl.plain_args() {
            if t.trim() == "&[u8]" {
                params.push(format!("const uint8_t* {n}"));
                params.push(format!("size_t {n}_len"));
                names.push(n.clone());
                names.push(format!("{n}_len"));
            } else {
                params.push(format!("{} {n}", c_type(t, utf16)));
                names.push(n.clone());
            }
        }
        if let Some((done, _)) = decl.done() {
            params.push(format!("uint64_t {done}"));
            names.push(done.to_string());
        }
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
    if let Some(prelude) = &arm.prelude {
        let _ = writeln!(out, "{}", prelude);
    }
    // swiftc maps every following line back to the crate's own source, so a type error in an arm
    // names the file its author opened (docs/bridge.md "Diagnostics").
    // The completion symbols an asynchronous arm answers through (docs/bridge.md "Callbacks"):
    // `<fn>_complete(done, value)` and `<fn>_fail(done)`, over the C symbol Rust exports.
    for decl in &bridge.decls {
        let Some((_, value)) = decl.done() else {
            continue;
        };
        let sym = complete_symbol(crate_name, decl);
        let verb = callback_verb(decl);
        let raw = format!("__day_bridge_{verb}_{}", decl.name);
        let (abi_params, ok_param, ok_call, fail_call) = match value.as_str() {
            "()" => ("", String::new(), "".to_string(), "".to_string()),
            "String" => (
                ", _ value: UnsafePointer<CChar>?",
                ", _ value: String".to_string(),
                "value.withCString { {raw}(done, 0, $0) }".replace("{raw}", &raw),
                "{raw}(done, 1, nil)".replace("{raw}", &raw),
            ),
            "Vec<u8>" => (
                ", _ value: UnsafePointer<UInt8>?, _ value_len: Int",
                ", _ value: [UInt8]".to_string(),
                "value.withUnsafeBufferPointer { {raw}(done, 0, $0.baseAddress, $0.count) }"
                    .replace("{raw}", &raw),
                "{raw}(done, 1, nil, 0)".replace("{raw}", &raw),
            ),
            "bool" => (
                ", _ value: Int32",
                ", _ value: Bool".to_string(),
                "{raw}(done, 0, value ? 1 : 0)".replace("{raw}", &raw),
                "{raw}(done, 1, 0)".replace("{raw}", &raw),
            ),
            t => {
                let swift = match t {
                    "i32" => "Int32",
                    "i64" => "Int64",
                    "f32" => "Float",
                    _ => "Double",
                };
                let zero = if matches!(t, "f32" | "f64") {
                    "0.0"
                } else {
                    "0"
                };
                (
                    match t {
                        "i32" => ", _ value: Int32",
                        "i64" => ", _ value: Int64",
                        "f32" => ", _ value: Float",
                        _ => ", _ value: Double",
                    },
                    format!(", _ value: {swift}"),
                    "{raw}(done, 0, value)".replace("{raw}", &raw),
                    format!("{raw}(done, 1, {zero})"),
                )
            }
        };
        let _ = writeln!(out, "@_silgen_name({})", quote(&sym));
        let _ = writeln!(
            out,
            "private func {raw}(_ done: UInt64, _ status: Int32{abi_params})"
        );
        if value == "()" {
            let _ = writeln!(
                out,
                "func {}_{verb}(_ done: UInt64) {{ {raw}(done, 0) }}",
                decl.name
            );
            let _ = writeln!(
                out,
                "func {}_fail(_ done: UInt64) {{ {raw}(done, 1) }}",
                decl.name
            );
            if decl.is_stream() {
                let _ = writeln!(
                    out,
                    "func {}_end(_ done: UInt64) {{ {raw}(done, 2) }}",
                    decl.name
                );
            }
        } else {
            let _ = writeln!(
                out,
                "func {}_{verb}(_ done: UInt64{ok_param}) {{ {ok_call} }}",
                decl.name
            );
            let _ = writeln!(
                out,
                "func {}_fail(_ done: UInt64) {{ {fail_call} }}",
                decl.name
            );
            if decl.is_stream() {
                let end_call = fail_call.replacen("(done, 1", "(done, 2", 1);
                let _ = writeln!(
                    out,
                    "func {}_end(_ done: UInt64) {{ {end_call} }}",
                    decl.name
                );
            }
        }
    }
    let _ = writeln!(
        out,
        "\n#sourceLocation(file: {}, line: {})",
        quote(source),
        arm.body_line
    );
    let _ = writeln!(out, "{}", arm.body.as_deref().unwrap_or(""));
    let _ = writeln!(out, "#sourceLocation()\n");

    for decl in &bridge.decls {
        let mut params: Vec<String> = Vec::new();
        for (n, t) in decl.plain_args() {
            if t.trim() == "&[u8]" {
                params.push(format!("{n}: UnsafePointer<UInt8>?"));
                params.push(format!("{n}_len: Int"));
            } else {
                params.push(format!("{n}: {}", swift_abi_type(t)));
            }
        }
        if let Some((done, _)) = decl.done() {
            params.push(format!("{done}: UInt64"));
        }
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
        for (n, t) in decl.plain_args() {
            match t.trim() {
                "&str" => {
                    let _ = writeln!(out, "    let {n}_s = String(cString: {n})");
                    passed.push(format!("{n}: {n}_s"));
                }
                "&[u8]" => {
                    let _ = writeln!(
                        out,
                        "    let {n}_a = Array(UnsafeBufferPointer(start: {n}, count: {n}_len))"
                    );
                    passed.push(format!("{n}: {n}_a"));
                }
                "bool" => {
                    let _ = writeln!(out, "    let {n}_b = {n} != 0");
                    passed.push(format!("{n}: {n}_b"));
                }
                _ => passed.push(format!("{n}: {n}")),
            }
        }
        if let Some((done, _)) = decl.done() {
            passed.push(format!("{done}: {done}"));
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
    if let Some(prelude) = &arm.prelude {
        let _ = writeln!(out, "{}", prelude);
    }
    // The completion helpers an asynchronous arm answers through (docs/bridge.md "Callbacks"):
    // `<fn>_complete(done, value)` and `<fn>_fail(done, message)`, over the wasm export Rust
    // provides. `__rt` is the runtime the shim hands `register`; the exports it reaches are
    // bound after registration, so the accessor is lazy.
    if bridge.decls.iter().any(|d| d.done().is_some()) {
        let _ = writeln!(out, "let __rt = null;");
    }
    for decl in &bridge.decls {
        let Some((_, value)) = decl.done() else {
            continue;
        };
        let sym = complete_symbol(crate_name, decl);
        let (ok_value, zero_value) = match value.as_str() {
            "()" => ("", ""),
            "bool" => ("value ? 1 : 0, ", "0, "),
            "i32" => ("value | 0, ", "0, "),
            "i64" => ("BigInt(value), ", "0n, "),
            "f32" | "f64" => ("+value, ", "0, "),
            "String" => ("...__rt.intoWasm(String(value)), ", "0, 0, "),
            _ => ("...__rt.bytesIntoWasm(value), ", "0, 0, "),
        };
        let value_param = if value == "()" { "" } else { ", value" };
        let _ = writeln!(
            out,
            "function {}_{}(done{value_param}) {{ __rt.exports().{sym}(done, 0, {ok_value}0, 0); }}",
            decl.name,
            callback_verb(decl)
        );
        let _ = writeln!(
            out,
            "function {}_fail(done, message) {{ __rt.exports().{sym}(done, 1, {zero_value}...__rt.intoWasm(String(message ?? ''))); }}",
            decl.name
        );
        if decl.is_stream() {
            let _ = writeln!(
                out,
                "function {}_end(done) {{ __rt.exports().{sym}(done, 2, {zero_value}0, 0); }}",
                decl.name
            );
        }
    }
    let _ = writeln!(out, "\n{}\n", arm.body.as_deref().unwrap_or(""));

    let _ = writeln!(
        out,
        "// The shim calls this once at boot and spreads the result into the wasm import object."
    );
    let _ = writeln!(out, "export function register(rt) {{");
    if bridge.decls.iter().any(|d| d.done().is_some()) {
        let _ = writeln!(out, "  __rt = rt;");
    }
    let _ = writeln!(out, "  return {{");
    for decl in &bridge.decls {
        let mut params: Vec<String> = Vec::new();
        let mut passed: Vec<String> = Vec::new();
        for (n, t) in decl.plain_args() {
            if t.trim() == "&str" {
                params.push(format!("{n}_ptr"));
                params.push(format!("{n}_len"));
                passed.push(format!("rt.str({n}_ptr, {n}_len)"));
            } else if t.trim() == "&[u8]" {
                params.push(format!("{n}_ptr"));
                params.push(format!("{n}_len"));
                passed.push(format!("rt.bytes({n}_ptr, {n}_len)"));
            } else {
                params.push(n.clone());
                passed.push(n.clone());
            }
        }
        if let Some((done, _)) = decl.done() {
            params.push(done.to_string());
            passed.push(done.to_string());
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
            (false, None) if decl.done().is_some() => {
                // An asynchronous arm: a thrown error means "failed to start" (status 1); a
                // returned promise completes the token itself when it settles, so an `async`
                // arm needs no helper call at all.
                let (done, value) = decl.done().unwrap_or_default();
                let on_resolve = if decl.is_stream() {
                    // A stream arm's promise settling says nothing about the stream: it ends
                    // through `<fn>_end`, and only a rejection fails it.
                    "() => {}".to_string()
                } else if value == "()" {
                    format!("(v) => {}_complete({done})", decl.name)
                } else {
                    format!("(v) => {}_complete({done}, v)", decl.name)
                };
                let _ = writeln!(out, "      try {{");
                let _ = writeln!(out, "        const r = {call};");
                let _ = writeln!(out, "        if (r && typeof r.then === 'function') {{");
                let _ = writeln!(
                    out,
                    "          r.then({on_resolve}, (e) => {}_fail({done}, e && e.message ? e.message : e));",
                    decl.name
                );
                let _ = writeln!(out, "        }}");
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
    if bridge.decls.iter().any(|d| d.done().is_some()) {
        let _ = writeln!(out, "import nativeEntry from 'libentry.so';");
    }
    if let Some(prelude) = &arm.prelude {
        let _ = writeln!(out, "{}", prelude);
    }
    // The completion helpers an asynchronous arm answers through (docs/bridge.md "Callbacks"):
    // `<fn>_complete(done, value)` and `<fn>_fail(done, message)`, over the shim's
    // `dayBridgeComplete`, which resolves the Rust export named here inside libentry.so.
    for decl in &bridge.decls {
        let Some((_, value)) = decl.done() else {
            continue;
        };
        let sym = arkts_complete_symbol(crate_name, decl);
        let value_param = match value.as_str() {
            "()" => String::new(),
            "bool" => ", value: boolean".into(),
            "String" => ", value: string".into(),
            "Vec<u8>" => ", value: Uint8Array".into(),
            _ => ", value: number".into(),
        };
        let value_arg = if value == "()" { "undefined" } else { "value" };
        let _ = writeln!(
            out,
            "function {}_{}(done: number{value_param}): void {{\n  nativeEntry.dayBridgeComplete('{sym}', done, 0, {value_arg}, '');\n}}",
            decl.name,
            callback_verb(decl)
        );
        let _ = writeln!(
            out,
            "function {}_fail(done: number, message: string): void {{\n  nativeEntry.dayBridgeComplete('{sym}', done, 1, undefined, message);\n}}",
            decl.name
        );
        if decl.is_stream() {
            let _ = writeln!(
                out,
                "function {}_end(done: number): void {{\n  nativeEntry.dayBridgeComplete('{sym}', done, 2, undefined, '');\n}}",
                decl.name
            );
        }
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

/// The symbol an ArkTS completion resolves — separate from the C-family one because it carries
/// one uniform shape for every value type (the shim marshals a JS value into it).
fn arkts_complete_symbol(crate_name: &str, decl: &Decl) -> String {
    format!(
        "day_bridge_{}_arkts_{}_{}",
        callback_verb(decl),
        crate_name.replace('-', "_"),
        decl.name
    )
}

/// The Rust half of an ArkTS arm: every call goes through `day_bridge::arkts::invoke`, which
/// finds the ArkUI shim's dispatcher at run time and runs the registered function on the JS
/// thread (docs/bridge.md "Callbacks"). A completion comes back through one uniform export.
fn render_arkts_rust(bridge: &Bridge, crate_name: &str) -> String {
    let mut out = String::new();
    for decl in &bridge.decls {
        let args: Vec<String> = decl.args.iter().map(|(n, t)| format!("{n}: {t}")).collect();
        let ret = if decl.ret.is_empty() {
            String::new()
        } else {
            format!(" -> {}", decl.ret)
        };
        let _ = writeln!(out, "fn {}({}){ret} {{", decl.name, args.join(", "));
        let mut items: Vec<String> = Vec::new();
        for (n, t) in decl.plain_args() {
            items.push(match t.trim() {
                "bool" => format!("day_bridge::arkts::Arg::bool({n})"),
                "i32" => format!("day_bridge::arkts::Arg::i32({n})"),
                "i64" => format!("day_bridge::arkts::Arg::i64({n})"),
                "f32" => format!("day_bridge::arkts::Arg::f64(f64::from({n}))"),
                "f64" => format!("day_bridge::arkts::Arg::f64({n})"),
                "&str" => format!("day_bridge::arkts::Arg::str({n})"),
                "&[u8]" => format!("day_bridge::arkts::Arg::bytes({n})"),
                _ => format!("day_bridge::arkts::Arg::i64({n} as i64)"),
            });
        }
        let _ = writeln!(
            out,
            "    let args: [day_bridge::arkts::Arg; {}] = [{}];",
            items.len(),
            items.join(", ")
        );
        let token = match decl.done() {
            Some((done, _)) => format!("{done}.into_token()"),
            None => "0".to_string(),
        };
        let sym = quote(&symbol(crate_name, decl));
        match (decl.ret.is_empty(), result_value(&decl.ret)) {
            (true, _) => {
                let _ = writeln!(
                    out,
                    "    let _ = day_bridge::arkts::invoke({sym}, &args, {token});"
                );
            }
            (false, None) => {
                let _ = writeln!(out, "    day_bridge::arkts::invoke({sym}, &args, {token})");
            }
            (false, Some(ty)) => {
                let read = match ty.as_str() {
                    "bool" => "ret.i != 0",
                    "i32" => "ret.i as i32",
                    "i64" => "ret.i",
                    "f32" => "ret.f as f32",
                    _ => "ret.f",
                };
                let _ = writeln!(
                    out,
                    "    let ret = day_bridge::arkts::invoke_value({sym}, &args, {token})?;"
                );
                let _ = writeln!(out, "    Ok({read})");
            }
        }
        let _ = writeln!(out, "}}\n");
    }

    for decl in &bridge.decls {
        let Some((_, value)) = decl.done() else {
            continue;
        };
        let convert = match value.as_str() {
            "()" => "Ok(())",
            "bool" => "Ok(num != 0)",
            "i32" => "Ok(num as i32)",
            "i64" => "Ok(num)",
            "f32" => "Ok(flt as f32)",
            "f64" => "Ok(flt)",
            "String" => {
                "String::from_utf8(unsafe { day_bridge::arkts::__bytes(ptr, len) }).map_err(|_| day_bridge::Error::Encoding)"
            }
            _ => "Ok(unsafe { day_bridge::arkts::__bytes(ptr, len) })",
        };
        let _ = writeln!(out, "#[unsafe(no_mangle)]");
        let _ = writeln!(
            out,
            "pub extern \"C\" fn {}(done: u64, status: i32, num: i64, flt: f64, ptr: *const u8, len: usize, msg: *const u8, msg_len: usize) {{",
            arkts_complete_symbol(crate_name, decl)
        );
        let _ = writeln!(out, "    day_bridge::guard(|| {{");
        let _ = writeln!(out, "        let _ = (num, flt, ptr, len);");
        let _ = writeln!(
            out,
            "        // SAFETY: the shim's buffers are valid for the duration of this call."
        );
        let _ = writeln!(
            out,
            "        let message = String::from_utf8_lossy(&unsafe {{ day_bridge::arkts::__bytes(msg, msg_len) }}).into_owned();"
        );
        let _ = writeln!(out, "        let outcome = if status == 0 {{");
        let _ = writeln!(out, "            {convert}");
        let _ = writeln!(out, "        }} else {{");
        let _ = writeln!(
            out,
            "            Err(day_bridge::Error::Foreign(if message.is_empty() {{ \"{} failed\".into() }} else {{ message }}))",
            decl.name
        );
        let _ = writeln!(out, "        }};");
        let _ = writeln!(
            out,
            "        {}.{};",
            registry_name(decl),
            registry_call(decl, "done")
        );
        let _ = writeln!(out, "    }});");
        let _ = writeln!(out, "}}\n");
    }
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
        for (n, t) in decl.plain_args() {
            if t.trim() == "&str" || t.trim() == "&[u8]" {
                params.push(format!("{n}_ptr: *const u8"));
                params.push(format!("{n}_len: usize"));
            } else {
                params.push(format!("{n}: {}", rust_c_type(t, false)));
            }
        }
        if let Some((done, _)) = decl.done() {
            params.push(format!("{done}: u64"));
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
        for (n, t) in decl.plain_args() {
            if t.trim() == "&str" || t.trim() == "&[u8]" {
                passed.push(format!("{n}.as_ptr()"));
                passed.push(format!("{n}.len()"));
            } else if t.trim() == "bool" {
                passed.push(format!("{n} as i32"));
            } else {
                passed.push(n.clone());
            }
        }
        if let Some((done, _)) = decl.done() {
            passed.push(format!("{done}.into_token()"));
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

    // The completion export a JavaScript arm calls through `<fn>_complete` / `<fn>_fail`: the
    // value and the failure message both cross as (ptr, len) into memory the shim allocated
    // with `day_dom_alloc`, consumed here exactly once (docs/web.md).
    for decl in &bridge.decls {
        let Some((_, value)) = decl.done() else {
            continue;
        };
        let mut params: Vec<String> = vec!["done: u64".into(), "status: i32".into()];
        match value.as_str() {
            "()" => {}
            "String" | "Vec<u8>" => {
                params.push("value: *mut u8".into());
                params.push("value_len: usize".into());
            }
            t => params.push(format!("value: {}", rust_c_type(t, false))),
        }
        params.push("msg: *mut u8".into());
        params.push("msg_len: usize".into());
        let convert = match value.as_str() {
            "()" => "Ok(())".to_string(),
            "bool" => "Ok(value != 0)".to_string(),
            "String" => "String::from_utf8(unsafe { day_bridge::__take_wasm(value, value_len) }).map_err(|_| day_bridge::Error::Encoding)".to_string(),
            "Vec<u8>" => "Ok(unsafe { day_bridge::__take_wasm(value, value_len) })".to_string(),
            _ => "Ok(value)".to_string(),
        };
        let _ = writeln!(out, "#[unsafe(no_mangle)]");
        let _ = writeln!(
            out,
            "pub extern \"C\" fn {}({}) {{",
            complete_symbol(crate_name, decl),
            params.join(", ")
        );
        let _ = writeln!(out, "    day_bridge::guard(|| {{");
        let _ = writeln!(
            out,
            "        // SAFETY: the shim handed over a `day_dom_alloc` buffer of `msg_len` bytes.\n        let message = String::from_utf8_lossy(&unsafe {{ day_bridge::__take_wasm(msg, msg_len) }}).into_owned();"
        );
        let _ = writeln!(out, "        let outcome = if status == 0 {{");
        let _ = writeln!(out, "            {convert}");
        let _ = writeln!(out, "        }} else {{");
        let _ = writeln!(
            out,
            "            Err(day_bridge::Error::Foreign(if message.is_empty() {{ \"{} failed\".into() }} else {{ message }}))",
            decl.name
        );
        let _ = writeln!(out, "        }};");
        let _ = writeln!(
            out,
            "        {}.{};",
            registry_name(decl),
            registry_call(decl, "done")
        );
        let _ = writeln!(out, "    }});");
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
    if let Some(prelude) = &arm.prelude {
        let _ = writeln!(out, "{}", prelude);
    }
    let _ = writeln!(out, "\n{}\n", arm.body.as_deref().unwrap_or(""));

    let _ = writeln!(out, "object {object} {{");
    // The completion entries an asynchronous arm answers through (docs/bridge.md "Callbacks"):
    // a `private external` JNI method Rust exports, and the two helpers the arm calls.
    for decl in &bridge.decls {
        let Some((_, value)) = decl.done() else {
            continue;
        };
        let kt = jvm_value_type(&value, true);
        let value_param = if value == "()" {
            String::new()
        } else {
            format!(", value: {kt}")
        };
        let value_arg = if value == "()" { "" } else { ", value" };
        let zero = if value == "()" {
            String::new()
        } else {
            format!(", {}", jvm_value_zero(&value))
        };
        let native = jvm_native_name(decl);
        let verb = callback_verb(decl);
        let _ = writeln!(
            out,
            "    @JvmStatic private external fun {native}(done: Long, status: Int{value_param}, message: String?)"
        );
        let _ = writeln!(
            out,
            "    @JvmStatic fun {}_{verb}(done: Long{value_param}) {{ {native}(done, 0{value_arg}, null) }}",
            decl.name
        );
        let _ = writeln!(
            out,
            "    @JvmStatic fun {}_fail(done: Long, message: String?) {{ {native}(done, 1{zero}, message) }}",
            decl.name
        );
        if decl.is_stream() {
            let _ = writeln!(
                out,
                "    @JvmStatic fun {}_end(done: Long) {{ {native}(done, 2{zero}, null) }}",
                decl.name
            );
        }
    }
    for decl in &bridge.decls {
        let mut params: Vec<String> = decl
            .plain_args()
            .iter()
            .map(|(n, t)| format!("{n}: {}", kotlin_type(t)))
            .collect();
        let mut named: Vec<String> = decl
            .plain_args()
            .iter()
            .map(|(n, _)| format!("{n} = {n}"))
            .collect();
        if let Some((done, _)) = decl.done() {
            params.push(format!("{done}: Long"));
            named.push(format!("{done} = {done}"));
        }
        let call = format!(
            // Fully qualified so it resolves to the arm's top-level function, never to this
            // object's member of the same name.
            "{pkg}.{}({})",
            decl.name,
            named.join(", ")
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
    if let Some(prelude) = &arm.prelude {
        let _ = writeln!(out, "{}", prelude);
    }
    let _ = writeln!(out, "\npublic final class {class} {{");
    let _ = writeln!(out, "    private {class}() {{}}\n");
    // The completion entries an asynchronous arm answers through (docs/bridge.md "Callbacks"):
    // a `private static native` method Rust exports, and the two helpers the arm calls.
    for decl in &bridge.decls {
        let Some((_, value)) = decl.done() else {
            continue;
        };
        let jt = jvm_value_type(&value, false);
        let value_param = if value == "()" {
            String::new()
        } else {
            format!(", {jt} value")
        };
        let value_arg = if value == "()" { "" } else { ", value" };
        let zero = if value == "()" {
            String::new()
        } else {
            format!(", {}", jvm_value_zero(&value))
        };
        let native = jvm_native_name(decl);
        let verb = callback_verb(decl);
        let _ = writeln!(
            out,
            "    private static native void {native}(long done, int status{value_param}, String message);"
        );
        let _ = writeln!(
            out,
            "    public static void {}_{verb}(long done{value_param}) {{ {native}(done, 0{value_arg}, null); }}",
            decl.name
        );
        let _ = writeln!(
            out,
            "    public static void {}_fail(long done, String message) {{ {native}(done, 1{zero}, message); }}",
            decl.name
        );
        if decl.is_stream() {
            let _ = writeln!(
                out,
                "    public static void {}_end(long done) {{ {native}(done, 2{zero}, null); }}",
                decl.name
            );
        }
        let _ = writeln!(out);
    }
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
        // A headless part is ordinary Rust anyone may call, including before (or without) a Day
        // app's init — where `with_env` would panic on the missing JVM. Asking first makes that
        // an ordinary `Runtime` error.
        let _ = writeln!(out, "    if !day_android::vm_ready() {{");
        let _ = writeln!(
            out,
            "        return{};",
            if decl.ret.is_empty() {
                String::new()
            } else {
                " Err(day_bridge::Error::Runtime)".to_string()
            }
        );
        let _ = writeln!(out, "    }}");
        if let Some((done, _)) = decl.done() {
            let _ = writeln!(out, "    let {done}_token = {done}.into_token() as i64;");
        }
        let _ = writeln!(out, "    let called = with_env(|env| {{");
        // Marshal arguments into JNI values; a String has to become a local ref first.
        let mut jvalues: Vec<String> = Vec::new();
        for (n, t) in decl.plain_args() {
            match t.trim() {
                "&str" => {
                    let _ = writeln!(out, "        let {n}_j = env.new_string({n}).ok()?;");
                    jvalues.push(format!("(&{n}_j).into()"));
                }
                "&[u8]" => {
                    let _ = writeln!(
                        out,
                        "        let {n}_j = env.byte_array_from_slice({n}).ok()?;"
                    );
                    jvalues.push(format!("(&{n}_j).into()"));
                }
                // jni 0.22's `jboolean` is Rust's `bool`.
                "bool" => jvalues.push(format!("day_android::jni::objects::JValue::Bool({n})")),
                "i32" => jvalues.push(format!("day_android::jni::objects::JValue::Int({n})")),
                "i64" => jvalues.push(format!("day_android::jni::objects::JValue::Long({n})")),
                "f32" => jvalues.push(format!("day_android::jni::objects::JValue::Float({n})")),
                "f64" => jvalues.push(format!("day_android::jni::objects::JValue::Double({n})")),
                _ => jvalues.push(n.clone()),
            }
        }
        if let Some((done, _)) = decl.done() {
            jvalues.push(format!(
                "day_android::jni::objects::JValue::Long({done}_token)"
            ));
        }
        let _ = writeln!(
            out,
            "        let outcome = env.dcall_static({}, {}, {}, &[{}]);",
            quote(&class),
            quote(&decl.name),
            quote(&jni_signature(decl)),
            jvalues.join(", ")
        );
        // A throwing arm leaves the exception PENDING on this thread. `with_env`'s attach guard
        // treats a pending exception as fatal and panics, which would turn the contract's
        // "an exception becomes Error::Foreign" into a contained panic that leaves the UI's
        // reactive state suspect. Logging and clearing it here is what keeps it an ordinary error.
        let _ = writeln!(out, "        if env.exception_check() {{");
        let _ = writeln!(out, "            env.exception_describe(); // → logcat");
        let _ = writeln!(out, "            env.exception_clear();");
        let _ = writeln!(out, "        }}");
        // Three shapes, not two: a bare unit call drops failures, `Result<(), _>` reports them,
        // and `Result<T, _>` also carries a value back.
        match (decl.ret.is_empty(), result_value(&decl.ret)) {
            (true, _) => {
                let _ = writeln!(out, "        outcome.ok()?;");
                let _ = writeln!(out, "        Some(())");
                let _ = writeln!(out, "    }});");
                let _ = writeln!(out, "    let _ = called;");
            }
            (false, None) => {
                let _ = writeln!(out, "        outcome.ok()?;");
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
                if ty == "String" {
                    // Copied out of the JVM immediately (docs/bridge.md "Ownership"); a null
                    // return is the arm saying "nothing", which the caller sees as an empty
                    // string rather than a foreign failure.
                    let _ = writeln!(out, "        let obj = outcome.ok()?.l().ok()?;");
                    let _ = writeln!(out, "        if obj.is_null() {{");
                    let _ = writeln!(out, "            return Some(String::new());");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "        env.dstr(&day_android::as_jstring(obj)).ok()");
                } else {
                    let _ = writeln!(out, "        outcome.ok()?.{}().ok()", jvalue_accessor(&ty));
                }
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

    // The JNI export behind `dayComplete_<fn>`: the JVM helper's `(done, status, value,
    // message)` becomes `Result<T, Error>` and resolves the token. Strings and byte arrays are
    // copied out of the JVM at once (docs/bridge.md "Ownership").
    for decl in &bridge.decls {
        let Some((_, value)) = decl.done() else {
            continue;
        };
        let mut params: Vec<String> = vec![
            "_env: day_android::jni::EnvUnowned<'_>".into(),
            "_class: day_android::jni::objects::JClass<'_>".into(),
            "done: day_android::jni::sys::jlong".into(),
            "status: day_android::jni::sys::jint".into(),
        ];
        match value.as_str() {
            "()" => {}
            "bool" => params.push("value: day_android::jni::sys::jboolean".into()),
            "i32" => params.push("value: day_android::jni::sys::jint".into()),
            "i64" => params.push("value: day_android::jni::sys::jlong".into()),
            "f32" => params.push("value: day_android::jni::sys::jfloat".into()),
            "f64" => params.push("value: day_android::jni::sys::jdouble".into()),
            "String" => params.push("value: day_android::jni::objects::JString<'_>".into()),
            _ => params.push("value: day_android::jni::objects::JByteArray<'_>".into()),
        }
        params.push("message: day_android::jni::objects::JString<'_>".into());
        let convert = match value.as_str() {
            "()" => "Ok(())".to_string(),
            "bool" => "Ok(value)".to_string(),
            "String" => "if value.is_null() { Ok(String::new()) } else { with_env(|env| env.dstr(&value)).map_err(|_| day_bridge::Error::Encoding) }".to_string(),
            "Vec<u8>" => "if value.is_null() { Ok(Vec::new()) } else { with_env(|env| env.convert_byte_array(&value)).map_err(|_| day_bridge::Error::Encoding) }".to_string(),
            _ => "Ok(value)".to_string(),
        };
        let _ = writeln!(out, "#[unsafe(no_mangle)]");
        let _ = writeln!(
            out,
            "pub extern \"system\" fn {}({}) {{",
            jni_export_name(crate_name, &jvm_native_name(decl)),
            params.join(", ")
        );
        let _ = writeln!(out, "    use day_android::{{DayEnv, with_env}};");
        let _ = writeln!(out, "    day_bridge::guard(|| {{");
        let _ = writeln!(out, "        let outcome = if status == 0 {{");
        let _ = writeln!(out, "            {convert}");
        let _ = writeln!(out, "        }} else {{");
        let _ = writeln!(
            out,
            "            let text = if message.is_null() {{ String::new() }} else {{ with_env(|env| env.dstr(&message)).unwrap_or_default() }};"
        );
        let _ = writeln!(
            out,
            "            Err(day_bridge::Error::Foreign(if text.is_empty() {{ \"{} failed\".into() }} else {{ text }}))",
            decl.name
        );
        let _ = writeln!(out, "        }};");
        let _ = writeln!(
            out,
            "        {}.{};",
            registry_name(decl),
            registry_call(decl, "done as u64")
        );
        let _ = writeln!(out, "    }});");
        let _ = writeln!(out, "}}\n");
    }
    out
}

/// The `native` method a JVM arm's helpers call: `dayComplete_<fn>` for a `Done`,
/// `dayEmit_<fn>` for an `Emit`.
fn jvm_native_name(decl: &Decl) -> String {
    let kind = if decl.is_stream() { "Emit" } else { "Complete" };
    format!("day{kind}_{}", decl.name)
}

/// The JVM spelling of a completion value, in Kotlin or Java.
fn jvm_value_type(ty: &str, kotlin: bool) -> &'static str {
    match (ty.trim(), kotlin) {
        ("bool", true) => "Boolean",
        ("bool", false) => "boolean",
        ("i32", true) => "Int",
        ("i32", false) => "int",
        ("i64", true) => "Long",
        ("i64", false) => "long",
        ("f32", true) => "Float",
        ("f32", false) => "float",
        ("f64", true) => "Double",
        ("f64", false) => "double",
        ("String", _) => "String",
        ("Vec<u8>", true) => "ByteArray",
        ("Vec<u8>", false) => "byte[]",
        _ => "",
    }
}

/// The value a failing completion passes in the JVM value slot (the same spelling in Kotlin and
/// Java).
fn jvm_value_zero(ty: &str) -> &'static str {
    match ty.trim() {
        "bool" => "false",
        "i64" => "0L",
        "f32" => "0f",
        "f64" => "0.0",
        "String" | "Vec<u8>" => "null",
        _ => "0",
    }
}

/// JNI's mangling of one name component: `_` becomes `_1`, and a `.` or `/` package separator
/// becomes `_`. Bridged names are ASCII identifiers, so the Unicode escape never applies.
fn jni_mangle(component: &str) -> String {
    let mut out = String::with_capacity(component.len());
    for c in component.chars() {
        match c {
            '_' => out.push_str("_1"),
            '.' | '/' => out.push('_'),
            c => out.push(c),
        }
    }
    out
}

/// `Java_dev_daybrite_day_bridge_day_1part_1speech_DayPartSpeechBridge_dayComplete_1speak_1native`:
/// the exported symbol JNI resolves for a `native` method on the generated class.
fn jni_export_name(crate_name: &str, method: &str) -> String {
    let class = format!(
        "{}.{}",
        kotlin_package(crate_name),
        kotlin_object(crate_name)
    );
    format!("Java_{}_{}", jni_mangle(&class), jni_mangle(method))
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
    let mut args: String = decl
        .plain_args()
        .iter()
        .map(|(_, t)| match t.trim() {
            "bool" => "Z",
            "i32" => "I",
            "i64" => "J",
            "f32" => "F",
            "f64" => "D",
            "&str" => "Ljava/lang/String;",
            "&[u8]" => "[B",
            _ => "Ljava/lang/Object;",
        })
        .collect();
    if decl.done().is_some() {
        args.push('J');
    }
    let ret = match result_value(&decl.ret).as_deref() {
        None => "V",
        Some("bool") => "Z",
        Some("i32") => "I",
        Some("i64") => "J",
        Some("f32") => "F",
        Some("f64") => "D",
        Some("String") => "Ljava/lang/String;",
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
        "&str" | "String" => "String",
        "&[u8]" | "Vec<u8>" => "ByteArray",
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
        let mut args: Vec<String> = Vec::new();
        for (n, t) in decl.plain_args() {
            if t.trim() == "&[u8]" {
                args.push(format!("{n}: *const u8"));
                args.push(format!("{n}_len: usize"));
            } else {
                args.push(format!("{n}: {}", rust_c_type(t, utf16)));
            }
        }
        if let Some((done, _)) = decl.done() {
            args.push(format!("{done}: u64"));
        }
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
        for (n, t) in decl.plain_args() {
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
                "&[u8]" => {
                    passed.push(format!("{n}.as_ptr()"));
                    passed.push(format!("{n}.len()"));
                }
                "bool" => passed.push(format!("{n} as i32")),
                _ => passed.push(n.clone()),
            }
        }
        if let Some((done, _)) = decl.done() {
            passed.push(format!("{done}.into_token()"));
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

    // The completion export a C, C++ or Swift arm calls through `<fn>_complete` / `<fn>_fail`.
    for decl in &bridge.decls {
        let Some((_, value)) = decl.done() else {
            continue;
        };
        let mut params: Vec<String> = vec!["done: u64".into(), "status: i32".into()];
        params.extend(
            rust_value_params(&value, utf16)
                .into_iter()
                .map(|(n, t)| format!("{n}: {t}")),
        );
        let _ = writeln!(out, "#[unsafe(no_mangle)]");
        let _ = writeln!(
            out,
            "pub extern \"C\" fn {}({}) {{",
            complete_symbol(crate_name, decl),
            params.join(", ")
        );
        let _ = writeln!(out, "    day_bridge::guard(|| {{");
        let _ = writeln!(out, "        let outcome = if status == 0 {{");
        let _ = writeln!(out, "            {}", rust_value_from_c(&value, utf16));
        let _ = writeln!(out, "        }} else {{");
        let _ = writeln!(
            out,
            "            Err(day_bridge::Error::Foreign(\"{} failed\".into()))",
            decl.name
        );
        let _ = writeln!(out, "        }};");
        let _ = writeln!(
            out,
            "        {}.{};",
            registry_name(decl),
            registry_call(decl, "done")
        );
        let _ = writeln!(out, "    }});");
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

    // The callback tier's Rust surface, the same on every target: one registry per `Done`
    // declaration, and the `<fn>_async` / `<fn>_future` wrappers over whichever arm is active
    // (docs/bridge.md "Callbacks").
    for decl in &bridge.decls {
        let Some((done, value)) = decl.done() else {
            continue;
        };
        let plain: Vec<String> = decl
            .plain_args()
            .iter()
            .map(|(n, t)| format!("{n}: {t}"))
            .collect();
        let names: Vec<String> = decl.plain_args().iter().map(|(n, _)| n.clone()).collect();
        let reg = registry_name(decl);
        if decl.is_stream() {
            // A stream: one set of consumers per declaration, the `<fn>_stream` wrapper that
            // registers a consumer and calls the arm, and `<fn>_stop` (docs/bridge.md "Streams").
            let _ = writeln!(out, "#[allow(dead_code, clippy::too_many_arguments)]");
            let _ = writeln!(
                out,
                "static {reg}: day_bridge::Streams<{value}> = day_bridge::Streams::new();\n"
            );
            let mut stream_params = plain.clone();
            stream_params.push(format!(
                "on_{done}: impl FnMut(day_bridge::Item<{value}>) + Send + 'static"
            ));
            let mut call_args = names.clone();
            call_args.push(done.to_string());
            let _ = writeln!(out, "#[allow(dead_code, clippy::too_many_arguments)]");
            let _ = writeln!(
                out,
                "pub(crate) fn {}_stream({}) -> Result<u64, day_bridge::Error> {{",
                decl.name,
                stream_params.join(", ")
            );
            let _ = writeln!(
                out,
                "    day_bridge::start_stream(&{reg}, on_{done}, move |{done}| {}({}))",
                decl.name,
                call_args.join(", ")
            );
            let _ = writeln!(out, "}}\n");
            let _ = writeln!(out, "#[allow(dead_code, clippy::too_many_arguments)]");
            let _ = writeln!(
                out,
                "pub(crate) fn {}_stop(token: u64) -> bool {{\n    {reg}.stop(token)\n}}\n",
                decl.name
            );
            continue;
        }
        let _ = writeln!(out, "#[allow(dead_code, clippy::too_many_arguments)]");
        let _ = writeln!(
            out,
            "static {reg}: day_bridge::Registry<{value}> = day_bridge::Registry::new();\n"
        );
        let mut async_params = plain.clone();
        async_params.push(format!(
            "on_{done}: impl FnOnce(Result<{value}, day_bridge::Error>) + Send + 'static"
        ));
        let mut call_args = names.clone();
        call_args.push(done.to_string());
        let _ = writeln!(out, "#[allow(dead_code, clippy::too_many_arguments)]");
        let _ = writeln!(
            out,
            "pub(crate) fn {}_async({}) -> Result<u64, day_bridge::Error> {{",
            decl.name,
            async_params.join(", ")
        );
        let _ = writeln!(
            out,
            "    day_bridge::start_async(&{reg}, on_{done}, move |{done}| {}({}))",
            decl.name,
            call_args.join(", ")
        );
        let _ = writeln!(out, "}}\n");
        let _ = writeln!(out, "#[allow(dead_code, clippy::too_many_arguments)]");
        let _ = writeln!(
            out,
            "pub(crate) fn {}_future({}) -> day_bridge::Completion<{value}> {{",
            decl.name,
            plain.join(", ")
        );
        let _ = writeln!(
            out,
            "    day_bridge::start_future(&{reg}, move |{done}| {}({}))",
            decl.name,
            call_args.join(", ")
        );
        let _ = writeln!(out, "}}\n");
    }

    for arm in bridge.arms.iter().filter(|a| a.lang == Lang::Rust) {
        if let Some(cfg) = cfg_attr(&arm_cfg(arm, bridge)) {
            let _ = writeln!(out, "{cfg}");
        }
        let _ = writeln!(out, "#[allow(dead_code, clippy::too_many_arguments)]");
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
            let _ = writeln!(out, "#[allow(dead_code, clippy::too_many_arguments)]");
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
                "#[cfg({cfg})]\n#[allow(dead_code, clippy::too_many_arguments)]\n{}\n",
                item.trim_end()
            );
        }
    }

    // An ArkTS arm is reached through the ArkUI shim's dispatcher on the JS thread.
    for arm in bridge.arms.iter().filter(|a| a.lang == Lang::ArkTs) {
        let block = render_arkts_rust(bridge, crate_name);
        let cfg = arm_cfg(arm, bridge);
        for item in block.split("\n\n").filter(|i| !i.trim().is_empty()) {
            let _ = writeln!(
                out,
                "#[cfg({cfg})]\n#[allow(dead_code, clippy::too_many_arguments)]\n{}\n",
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
                "#[cfg({cfg})]\n#[allow(dead_code, clippy::too_many_arguments)]\n{}\n",
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
                    let _ = writeln!(
                        out,
                        "{cfg}\n#[allow(dead_code, clippy::too_many_arguments)]\n{}\n",
                        item.trim_end()
                    );
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
                "#[allow(dead_code, clippy::too_many_arguments)]\npub(crate) fn {}_support() -> day_bridge::Support {{\n    \
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

    #[day_bridge::impl(kotlin, platforms = [android])]
    kotlin!(
        prelude = r#"
            import android.speech.tts.TextToSpeech
        "#,
        body = r#"
            fun speak_native(text: String) { engine?.speak(text) }
        "#,
    );

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

    fn parse_err(src: &str) -> String {
        let mut b = Bridge::default();
        parse_into(src, "src/lib.rs", &mut b).expect_err("should not parse")
    }

    #[test]
    fn parses_declarations_and_arms() {
        let b = parse(SPEECH);
        assert_eq!(b.decls.len(), 2);
        assert_eq!(b.decls[0].name, "speak_native");
        assert_eq!(b.decls[0].args, vec![("text".into(), "&str".into())]);
        assert_eq!(b.decls[0].ret, "Result<(), day_bridge::Error>");
        assert_eq!(b.decls[1].name, "stop_native");
        assert!(b.decls[1].args.is_empty());
        assert_eq!(b.arms.len(), 3);
        assert_eq!(
            b.arms[0].prelude.as_deref(),
            Some("import android.speech.tts.TextToSpeech"),
            "the prelude belongs to the arm that declared it"
        );
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
    fn rejects_an_argument_named_after_a_generated_length() {
        let b = parse(
            r###"
            day_bridge::bridge! {
                #[day_bridge::declare]
                extern "day" { fn f(body: &[u8], body_len: i64); }
                #[day_bridge::impl(rust, platforms = [other])]
                fn f(_body: &[u8], _body_len: i64) {}
            }
            "###,
        );
        let err = validate(&b).expect_err("a name the generated length takes");
        assert!(err.contains("`body_len`"), "{err}");
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
    fn rejects_a_bare_return() {
        let b = parse(
            r###"
            day_bridge::bridge! {
                #[day_bridge::declare]
                extern "day" { fn ready() -> bool; }
                #[day_bridge::impl(rust, platforms = [other])]
                fn ready() -> bool { false }
            }
            "###,
        );
        let err = validate(&b).unwrap_err();
        assert!(err.contains("`ready` returns `bool`"), "{err}");
    }

    #[test]
    fn passes_a_bool_to_a_jvm_arm() {
        let b = parse(
            r###"
            day_bridge::bridge! {
                #[day_bridge::declare]
                extern "day" { fn hold(on: bool); }
                #[day_bridge::impl(java, platforms = [android])]
                java!(
                    prelude = r#"import java.util.List;"#,
                    body = r#"public static void hold(boolean on) {}"#,
                );
                #[day_bridge::impl(rust, platforms = [other])]
                fn hold(_on: bool) {}
            }
            "###,
        );
        validate(&b).expect("valid");
        let rust = render_jvm_rust(&b, "day-part-test");
        assert!(rust.contains("JValue::Bool(on)"), "{rust}");
        assert!(rust.contains("\"(Z)V\""), "{rust}");
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

    /// `"#` is ordinary in JavaScript, CSS selectors and C format strings, and it ends an `r#"…"#`
    /// body early — silently, taking the rest of the arm with it. More hashes is the author's fix,
    /// so the parser counts them the way rustc does.
    #[test]
    fn a_body_containing_a_quote_hash_needs_more_hashes() {
        let b = parse(
            r####"
            day_bridge::bridge! {
                #[day_bridge::declare]
                extern "day" { fn focus_native(); }
                #[day_bridge::impl(js, platforms = [web])]
                js!(r##"
                    export function focus_native() { document.querySelector("#speech").focus(); }
                "##);
                #[day_bridge::impl(rust, platforms = [other])]
                fn focus_native() {}
            }
            "####,
        );
        let arm = b.arms.iter().find(|a| a.lang == Lang::Js).unwrap();
        let body = arm.body.as_deref().unwrap();
        assert!(
            body.contains("querySelector(\"#speech\")") && body.ends_with("}"),
            "the whole body survives:\n{body}"
        );
        // The arm after it still parses, which is what an early terminator would have eaten.
        assert!(b.arms.iter().any(|a| a.lang == Lang::Rust), "{:?}", b.arms);
    }

    /// A misspelled option used to be ignored, which surfaced far from the mistake.
    #[test]
    fn rejects_unknown_arm_options_and_values() {
        let bad_key = parse_err(
            r###"
            day_bridge::bridge! {
                #[day_bridge::impl(c, platforms = [linux], linkk = ["speechd"])]
                c!(r#" void f(void) {} "#);
            }
            "###,
        );
        assert!(bad_key.contains("unknown arm option `linkk`"), "{bad_key}");

        let bad_encoding = parse_err(
            r###"
            day_bridge::bridge! {
                #[day_bridge::impl(cpp, platforms = [windows], encoding = "utf-16")]
                cpp!(r#" void f(void) {} "#);
            }
            "###,
        );
        assert!(
            bad_encoding.contains("expected \"utf8\" or \"utf16\""),
            "{bad_encoding}"
        );

        let bad_support = parse_err(
            r###"
            day_bridge::bridge! {
                #[day_bridge::impl(arkts, platforms = [ohos], support = "partial")]
                arkts!(r#" export function f() {} "#);
            }
            "###,
        );
        assert!(
            bad_support.contains("expected \"native\" or \"emulated\""),
            "{bad_support}"
        );
    }

    #[test]
    fn rejects_a_package_line_in_a_prelude() {
        let err = parse_err(
            r###"
            day_bridge::bridge! {
                #[day_bridge::impl(kotlin, platforms = [android])]
                kotlin!(
                    prelude = r#"
                        package dev.example.mine
                    "#,
                    body = r#"
                        fun f() {}
                    "#,
                );
            }
            "###,
        );
        assert!(err.contains("belongs to the generator"), "{err}");
    }

    /// The prelude is per ARM, so two arms of one language claiming different platforms cannot
    /// receive each other's imports — the bug the old per-language prelude had by construction.
    #[test]
    fn a_prelude_reaches_only_its_own_arm() {
        let b = parse(
            r###"
            day_bridge::bridge! {
                #[day_bridge::declare]
                extern "day" { fn f(); }

                #[day_bridge::impl(c, platforms = [linux])]
                c!(
                    prelude = r#"
                        #include <linux_only.h>
                    "#,
                    body = r#" void f(void) {} "#,
                );

                #[day_bridge::impl(c, platforms = [windows])]
                c!(
                    prelude = r#"
                        #include <windows.h>
                    "#,
                    body = r#" void f(void) {} "#,
                );

                #[day_bridge::impl(rust, platforms = [other])]
                fn f() {}
            }
            "###,
        );
        let windows = b
            .arms
            .iter()
            .find(|a| a.platforms.iter().any(|p| p == "windows"))
            .unwrap();
        let c = render_c(&b, windows, "day-part-demo");
        assert!(c.contains("#include <windows.h>"), "{c}");
        assert!(
            !c.contains("linux_only.h"),
            "no leak from the other arm:\n{c}"
        );
    }

    /// The old spelling was a separate item; the error says where it went.
    #[test]
    fn a_standalone_prelude_attribute_says_what_replaced_it() {
        let err = parse_err(
            r###"
            day_bridge::bridge! {
                #[day_bridge::prelude(swift)]
                swift!(r#" import AVFoundation "#);
            }
            "###,
        );
        assert!(err.contains("no longer exists"), "{err}");
        assert!(err.contains("prelude = r#"), "{err}");
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

    const ASYNC: &str = r###"
day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        fn speak_native(text: &str, done: day_bridge::Done<bool>) -> Result<(), day_bridge::Error>;
        fn engine_ready_native(done: Done<()>) -> Result<(), day_bridge::Error>;
        fn stop_native();
    }

    #[day_bridge::impl(java, platforms = [android])]
    java!(r#"
        public static void speak_native(String text, long done) { speak_native_complete(done, true); }
        public static void engine_ready_native(long done) { engine_ready_native_complete(done); }
        public static void stop_native() {}
    "#);

    #[day_bridge::impl(swift, platforms = [ios, macos])]
    swift!(r#"
        func speak_native(text: String, done: UInt64) throws { speak_native_complete(done, true) }
        func engine_ready_native(done: UInt64) throws { engine_ready_native_complete(done) }
        func stop_native() {}
    "#);

    #[day_bridge::impl(c, platforms = [linux])]
    c!(r#"
        int32_t speak_native(const char* text, uint64_t done) { speak_native_complete(done, 1); return 0; }
        int32_t engine_ready_native(uint64_t done) { engine_ready_native_complete(done); return 0; }
        void stop_native(void) {}
    "#);

    #[day_bridge::impl(js, platforms = [web])]
    js!(r#"
        export async function speak_native(text, done) { return true; }
        export function engine_ready_native(done) { engine_ready_native_complete(done); }
        export function stop_native() {}
    "#);

    #[day_bridge::impl(rust, platforms = [other])]
    fn speak_native(_text: &str, done: day_bridge::Done<bool>) -> Result<(), day_bridge::Error> {
        done.complete(Err(day_bridge::Error::Unsupported));
        Ok(())
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn engine_ready_native(done: day_bridge::Done<()>) -> Result<(), day_bridge::Error> {
        done.complete(Err(day_bridge::Error::Unsupported));
        Ok(())
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn stop_native() {}
}
"###;

    #[test]
    fn a_done_argument_is_recognized_and_stripped_from_the_marshaled_list() {
        let b = parse(ASYNC);
        validate(&b).expect("valid");
        let speak = &b.decls[0];
        assert_eq!(speak.done(), Some(("done", "bool".to_string())));
        assert_eq!(
            speak.plain_args(),
            &[("text".to_string(), "&str".to_string())]
        );
        assert_eq!(b.decls[1].done(), Some(("done", "()".to_string())));
        assert_eq!(b.decls[2].done(), None);
    }

    #[test]
    fn the_rust_surface_gains_a_registry_and_two_wrappers_per_done() {
        let b = parse(ASYNC);
        let rust = render_rust(&b, "day-part-speech");
        assert!(
            rust.contains("static DAY_BRIDGE_DONE_SPEAK_NATIVE: day_bridge::Registry<bool> = day_bridge::Registry::new();"),
            "{rust}"
        );
        assert!(
            rust.contains("pub(crate) fn speak_native_async(text: &str, on_done: impl FnOnce(Result<bool, day_bridge::Error>) + Send + 'static) -> Result<u64, day_bridge::Error>"),
            "{rust}"
        );
        assert!(
            rust.contains(
                "pub(crate) fn speak_native_future(text: &str) -> day_bridge::Completion<bool>"
            ),
            "{rust}"
        );
        assert!(
            rust.contains("day_bridge::start_future(&DAY_BRIDGE_DONE_SPEAK_NATIVE, move |done| speak_native(text, done))"),
            "{rust}"
        );
        // The C-family completion export resolves the registry with a bool from an int32.
        assert!(
            rust.contains("pub extern \"C\" fn day_bridge_complete_day_part_speech_speak_native(done: u64, status: i32, value: i32)"),
            "{rust}"
        );
        assert!(rust.contains("Ok(value != 0)"), "{rust}");
        assert!(
            rust.contains("fn day_bridge_day_part_speech_speak_native(text: *const std::ffi::c_char, done: u64) -> i32;"),
            "the token is the trailing argument:\n{rust}"
        );
        // The JVM export carries JNI's mangling of the crate's underscores.
        assert!(
            rust.contains("pub extern \"system\" fn Java_dev_daybrite_day_bridge_day_1part_1speech_DayPartSpeechBridge_dayComplete_1speak_1native("),
            "{rust}"
        );
        assert!(rust.contains("JValue::Long(done_token)"), "{rust}");
        // The wasm export takes the value and the message as (ptr, len) pairs.
        assert!(
            rust.contains("pub extern \"C\" fn day_bridge_complete_day_part_speech_speak_native(done: u64, status: i32, value: i32, msg: *mut u8, msg_len: usize)"),
            "{rust}"
        );
    }

    #[test]
    fn the_c_arm_gets_complete_and_fail_helpers_before_its_body() {
        let b = parse(ASYNC);
        let arm = b.arms.iter().find(|a| a.lang == Lang::C).unwrap();
        let c = render_c(&b, arm, "day-part-speech");
        assert!(
            c.contains("extern void day_bridge_complete_day_part_speech_speak_native(uint64_t done, int32_t status, int32_t value);"),
            "{c}"
        );
        assert!(
            c.contains("static void speak_native_complete(uint64_t done, int32_t value) { day_bridge_complete_day_part_speech_speak_native(done, 0, value); }"),
            "{c}"
        );
        assert!(
            c.contains("static void speak_native_fail(uint64_t done) { day_bridge_complete_day_part_speech_speak_native(done, 1, 0); }"),
            "{c}"
        );
        assert!(
            c.contains("static void engine_ready_native_complete(uint64_t done) { day_bridge_complete_day_part_speech_engine_ready_native(done, 0); }"),
            "{c}"
        );
        let helpers = c.find("speak_native_complete(uint64_t").unwrap();
        let body = c.find("#line").unwrap();
        assert!(
            helpers < body,
            "helpers precede the arm so it can call them:\n{c}"
        );
        assert!(
            c.contains("int32_t day_bridge_day_part_speech_speak_native(const char* text, uint64_t done) { return speak_native(text, done); }"),
            "{c}"
        );
    }

    #[test]
    fn the_swift_arm_completes_through_a_silgen_name_symbol() {
        let b = parse(ASYNC);
        let arm = b.arms.iter().find(|a| a.lang == Lang::Swift).unwrap();
        let swift = render_swift(&b, arm, "day-part-speech");
        assert!(
            swift.contains("@_silgen_name(\"day_bridge_complete_day_part_speech_speak_native\")"),
            "{swift}"
        );
        assert!(
            swift.contains("func speak_native_complete(_ done: UInt64, _ value: Bool) { __day_bridge_complete_speak_native(done, 0, value ? 1 : 0) }"),
            "{swift}"
        );
        assert!(
            swift.contains("func engine_ready_native_complete(_ done: UInt64) { __day_bridge_complete_engine_ready_native(done, 0) }"),
            "{swift}"
        );
        assert!(
            swift.contains("public func day_bridge_day_part_speech_speak_native(text: UnsafePointer<CChar>, done: UInt64) -> Int32 {"),
            "{swift}"
        );
        assert!(
            swift.contains("try speak_native(text: text_s, done: done)"),
            "{swift}"
        );
    }

    #[test]
    fn the_java_class_gets_a_native_completion_and_two_helpers() {
        let b = parse(ASYNC);
        let arm = b.arms.iter().find(|a| a.lang == Lang::Java).unwrap();
        let java = render_java(&b, arm, "day-part-speech");
        assert!(
            java.contains("private static native void dayComplete_speak_native(long done, int status, boolean value, String message);"),
            "{java}"
        );
        assert!(
            java.contains("public static void speak_native_complete(long done, boolean value) { dayComplete_speak_native(done, 0, value, null); }"),
            "{java}"
        );
        assert!(
            java.contains("public static void speak_native_fail(long done, String message) { dayComplete_speak_native(done, 1, false, message); }"),
            "{java}"
        );
        assert!(
            java.contains("private static native void dayComplete_engine_ready_native(long done, int status, String message);"),
            "{java}"
        );
        assert_eq!(jni_signature(&b.decls[0]), "(Ljava/lang/String;J)V");
        assert_eq!(jni_signature(&b.decls[1]), "(J)V");
        let kotlin = render_kotlin(&b, arm, "day-part-speech");
        assert!(
            kotlin.contains("@JvmStatic private external fun dayComplete_speak_native(done: Long, status: Int, value: Boolean, message: String?)"),
            "{kotlin}"
        );
        assert!(
            kotlin.contains("fun speak_native(text: String, done: Long) {"),
            "{kotlin}"
        );
    }

    #[test]
    fn the_js_module_completes_a_returned_promise_itself() {
        let b = parse(ASYNC);
        let arm = b.arms.iter().find(|a| a.lang == Lang::Js).unwrap();
        let js = render_js(&b, arm, "day-part-speech");
        assert!(js.contains("let __rt = null;"), "{js}");
        assert!(js.contains("__rt = rt;"), "{js}");
        assert!(
            js.contains("function speak_native_complete(done, value) { __rt.exports().day_bridge_complete_day_part_speech_speak_native(done, 0, value ? 1 : 0, 0, 0); }"),
            "{js}"
        );
        assert!(
            js.contains("function speak_native_fail(done, message) { __rt.exports().day_bridge_complete_day_part_speech_speak_native(done, 1, 0, ...__rt.intoWasm(String(message ?? ''))); }"),
            "{js}"
        );
        assert!(
            js.contains("day_bridge_day_part_speech_speak_native(text_ptr, text_len, done) {"),
            "{js}"
        );
        assert!(
            js.contains("r.then((v) => speak_native_complete(done, v), (e) => speak_native_fail(done, e && e.message ? e.message : e));"),
            "{js}"
        );
        assert!(
            js.contains(
                "r.then((v) => engine_ready_native_complete(done), (e) => engine_ready_native_fail("
            ),
            "a unit completion ignores the promise's value:\n{js}"
        );
    }

    #[test]
    fn a_done_must_be_last_alone_and_carry_a_table_value() {
        let not_last = ASYNC.replace(
            "fn speak_native(text: &str, done: day_bridge::Done<bool>)",
            "fn speak_native(done: day_bridge::Done<bool>, text: &str)",
        );
        let err = validate(&parse(&not_last)).expect_err("done must be last");
        assert!(err.contains("single, last argument"), "{err}");

        let bad_value = ASYNC.replace("Done<bool>", "Done<Option<bool>>");
        let err = validate(&parse(&bad_value)).expect_err("value outside the table");
        assert!(err.contains("completion table"), "{err}");

        let returns_too = ASYNC.replace(
            "fn engine_ready_native(done: Done<()>) -> Result<(), day_bridge::Error>",
            "fn engine_ready_native(done: Done<()>) -> Result<bool, day_bridge::Error>",
        );
        let err = validate(&parse(&returns_too)).expect_err("a value and a completion");
        assert!(
            err.contains("both returns a value and completes one"),
            "{err}"
        );
    }

    #[test]
    fn an_arkts_arm_may_declare_the_hms_sdk() {
        let src = SPEECH.replace(
            "#[day_bridge::impl(kotlin, platforms = [android])]\n    kotlin!(",
            "#[day_bridge::impl(arkts, platforms = [ohos], sdk = \"hms\")]\n    arkts!(",
        );
        let b = parse(&src);
        assert_eq!(
            b.arms[0].options.get("sdk").map(String::as_str),
            Some("hms")
        );
        let err = parse_err(&SPEECH.replace(
            "#[day_bridge::impl(kotlin, platforms = [android])]",
            "#[day_bridge::impl(kotlin, platforms = [android], sdk = \"hms\")]",
        ));
        assert!(err.contains("`arkts` arm only"), "{err}");
    }

    #[test]
    fn jni_mangling_escapes_underscores_in_every_component() {
        assert_eq!(
            jni_export_name("day-part-speech", "dayComplete_speak_native"),
            "Java_dev_daybrite_day_bridge_day_1part_1speech_DayPartSpeechBridge_dayComplete_1speak_1native"
        );
    }

    #[test]
    fn the_arkts_arm_gets_a_rust_half_and_a_uniform_completion() {
        let src = ASYNC
            .replace(
                "#[day_bridge::impl(js, platforms = [web])]",
                "#[day_bridge::impl(arkts, platforms = [ohos])]",
            )
            .replace("js!(r#\"", "arkts!(r#\"");
        let b = parse(&src);
        validate(&b).expect("valid");
        let arm = b.arms.iter().find(|a| a.lang == Lang::ArkTs).unwrap();
        let ets = render_arkts(&b, arm, "day-part-speech");
        assert!(
            ets.contains("import nativeEntry from 'libentry.so';"),
            "{ets}"
        );
        assert!(
            ets.contains("function speak_native_complete(done: number, value: boolean): void {\n  nativeEntry.dayBridgeComplete('day_bridge_complete_arkts_day_part_speech_speak_native', done, 0, value, '');\n}"),
            "{ets}"
        );
        assert!(
            ets.contains("function engine_ready_native_complete(done: number): void {\n  nativeEntry.dayBridgeComplete('day_bridge_complete_arkts_day_part_speech_engine_ready_native', done, 0, undefined, '');\n}"),
            "{ets}"
        );
        let rust = render_rust(&b, "day-part-speech");
        assert!(
            rust.contains(
                "#[cfg(all(all(target_os = \"linux\", target_env = \"ohos\"), day_bridge_staged))]"
            ),
            "{rust}"
        );
        assert!(
            rust.contains(
                "let args: [day_bridge::arkts::Arg; 1] = [day_bridge::arkts::Arg::str(text)];"
            ),
            "{rust}"
        );
        assert!(
            rust.contains("day_bridge::arkts::invoke(\"day_bridge_day_part_speech_speak_native\", &args, done.into_token())"),
            "{rust}"
        );
        assert!(
            rust.contains(
                "day_bridge::arkts::invoke(\"day_bridge_day_part_speech_stop_native\", &args, 0)"
            ),
            "{rust}"
        );
        assert!(
            rust.contains("pub extern \"C\" fn day_bridge_complete_arkts_day_part_speech_speak_native(done: u64, status: i32, num: i64, flt: f64, ptr: *const u8, len: usize, msg: *const u8, msg_len: usize)"),
            "{rust}"
        );
        assert!(rust.contains("Ok(num != 0)"), "{rust}");
    }

    #[test]
    fn an_arkts_arm_returns_a_scalar_through_the_dispatcher() {
        let src = SPEECH
            .replace("kotlin", "arkts")
            .replace(
                "fn speak_native(text: &str) -> Result<(), day_bridge::Error>;",
                "fn speak_native(text: &str) -> Result<bool, day_bridge::Error>;",
            )
            .replace(
                "fn speak_native(_text: &str) -> Result<(), day_bridge::Error> {",
                "fn speak_native(_text: &str) -> Result<bool, day_bridge::Error> {",
            );
        let b = parse(&src);
        validate(&b).expect("a scalar return is marshaled");
        let rust = render_rust(&b, "day-part-speech");
        assert!(
            rust.contains("let ret = day_bridge::arkts::invoke_value(\"day_bridge_day_part_speech_speak_native\", &args, 0)?;"),
            "{rust}"
        );
        assert!(rust.contains("Ok(ret.i != 0)"), "{rust}");
    }

    const STREAM: &str = r###"
day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        fn watch_native(key: &str, emit: day_bridge::Emit<Vec<u8>>) -> Result<(), day_bridge::Error>;
        fn tick_native(emit: Emit<()>) -> Result<(), day_bridge::Error>;
    }

    #[day_bridge::impl(java, platforms = [android])]
    java!(r#"
        public static void watch_native(String key, long emit) { watch_native_emit(emit, new byte[0]); watch_native_end(emit); }
        public static void tick_native(long emit) { tick_native_emit(emit); tick_native_end(emit); }
    "#);

    #[day_bridge::impl(swift, platforms = [ios, macos])]
    swift!(r#"
        func watch_native(key: String, emit: UInt64) throws { watch_native_emit(emit, []); watch_native_end(emit) }
        func tick_native(emit: UInt64) throws { tick_native_emit(emit); tick_native_end(emit) }
    "#);

    #[day_bridge::impl(c, platforms = [linux])]
    c!(r#"
        int32_t watch_native(const char* key, uint64_t emit) { watch_native_emit(emit, NULL, 0); watch_native_end(emit); return 0; }
        int32_t tick_native(uint64_t emit) { tick_native_emit(emit); tick_native_end(emit); return 0; }
    "#);

    #[day_bridge::impl(js, platforms = [web])]
    js!(r#"
        export async function watch_native(key, emit) { watch_native_emit(emit, new Uint8Array(0)); watch_native_end(emit); }
        export function tick_native(emit) { tick_native_emit(emit); tick_native_end(emit); }
    "#);

    #[day_bridge::impl(arkts, platforms = [ohos])]
    arkts!(r#"
        export function watch_native(key: string, emit: number): void { watch_native_end(emit); }
        export function tick_native(emit: number): void { tick_native_end(emit); }
    "#);

    #[day_bridge::impl(rust, platforms = [other])]
    fn watch_native(_key: &str, emit: day_bridge::Emit<Vec<u8>>) -> Result<(), day_bridge::Error> {
        emit.end();
        Ok(())
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn tick_native(emit: day_bridge::Emit<()>) -> Result<(), day_bridge::Error> {
        emit.end();
        Ok(())
    }
}
"###;

    #[test]
    fn an_emit_argument_is_a_stream_handle() {
        let b = parse(STREAM);
        validate(&b).expect("valid");
        assert!(b.decls[0].is_stream());
        assert_eq!(b.decls[0].done(), Some(("emit", "Vec<u8>".to_string())));
        assert_eq!(
            b.decls[0].plain_args(),
            &[("key".to_string(), "&str".to_string())]
        );
        assert!(!parse(ASYNC).decls[0].is_stream());
    }

    #[test]
    fn the_rust_surface_gains_streams_a_stream_wrapper_and_a_stop() {
        let b = parse(STREAM);
        let rust = render_rust(&b, "day-part-demo");
        assert!(
            rust.contains("static DAY_BRIDGE_EMIT_WATCH_NATIVE: day_bridge::Streams<Vec<u8>> = day_bridge::Streams::new();"),
            "{rust}"
        );
        assert!(
            rust.contains("pub(crate) fn watch_native_stream(key: &str, on_emit: impl FnMut(day_bridge::Item<Vec<u8>>) + Send + 'static) -> Result<u64, day_bridge::Error>"),
            "{rust}"
        );
        assert!(
            rust.contains("day_bridge::start_stream(&DAY_BRIDGE_EMIT_WATCH_NATIVE, on_emit, move |emit| watch_native(key, emit))"),
            "{rust}"
        );
        assert!(
            rust.contains("pub(crate) fn watch_native_stop(token: u64) -> bool {\n    DAY_BRIDGE_EMIT_WATCH_NATIVE.stop(token)\n}"),
            "{rust}"
        );
        assert!(
            rust.contains("pub extern \"C\" fn day_bridge_emit_day_part_demo_watch_native(done: u64, status: i32, value: *const u8, value_len: usize)"),
            "{rust}"
        );
        assert!(
            rust.contains("DAY_BRIDGE_EMIT_WATCH_NATIVE.deliver(done, status, outcome);"),
            "{rust}"
        );
        assert!(
            rust.contains("pub extern \"system\" fn Java_dev_daybrite_day_bridge_day_1part_1demo_DayPartDemoBridge_dayEmit_1watch_1native("),
            "{rust}"
        );
        assert!(
            rust.contains("DAY_BRIDGE_EMIT_WATCH_NATIVE.deliver(done as u64, status, outcome);"),
            "{rust}"
        );
        assert!(
            rust.contains("pub extern \"C\" fn day_bridge_emit_arkts_day_part_demo_watch_native("),
            "{rust}"
        );
    }

    #[test]
    fn each_language_gets_emit_fail_and_end_helpers() {
        let b = parse(STREAM);
        let c_arm = b.arms.iter().find(|a| a.lang == Lang::C).unwrap();
        let c = render_c(&b, c_arm, "day-part-demo");
        assert!(
            c.contains("static void watch_native_emit(uint64_t done, const uint8_t* value, size_t value_len) { day_bridge_emit_day_part_demo_watch_native(done, 0, value, value_len); }"),
            "{c}"
        );
        assert!(
            c.contains("static void watch_native_end(uint64_t done) { day_bridge_emit_day_part_demo_watch_native(done, 2, NULL, 0); }"),
            "{c}"
        );
        assert!(
            c.contains("static void tick_native_end(uint64_t done) { day_bridge_emit_day_part_demo_tick_native(done, 2); }"),
            "{c}"
        );

        let swift_arm = b.arms.iter().find(|a| a.lang == Lang::Swift).unwrap();
        let swift = render_swift(&b, swift_arm, "day-part-demo");
        assert!(
            swift.contains("func watch_native_emit(_ done: UInt64, _ value: [UInt8]) { value.withUnsafeBufferPointer { __day_bridge_emit_watch_native(done, 0, $0.baseAddress, $0.count) } }"),
            "{swift}"
        );
        assert!(
            swift.contains("func watch_native_end(_ done: UInt64) { __day_bridge_emit_watch_native(done, 2, nil, 0) }"),
            "{swift}"
        );
        assert!(
            swift.contains(
                "func tick_native_end(_ done: UInt64) { __day_bridge_emit_tick_native(done, 2) }"
            ),
            "{swift}"
        );

        let js_arm = b.arms.iter().find(|a| a.lang == Lang::Js).unwrap();
        let js = render_js(&b, js_arm, "day-part-demo");
        assert!(
            js.contains("function watch_native_end(done) { __rt.exports().day_bridge_emit_day_part_demo_watch_native(done, 2, 0, 0, 0, 0); }"),
            "{js}"
        );
        assert!(
            js.contains(
                "r.then(() => {}, (e) => watch_native_fail(emit, e && e.message ? e.message : e));"
            ),
            "a resolved stream promise ends nothing:\n{js}"
        );

        let java_arm = b.arms.iter().find(|a| a.lang == Lang::Java).unwrap();
        let java = render_java(&b, java_arm, "day-part-demo");
        assert!(
            java.contains("private static native void dayEmit_watch_native(long done, int status, byte[] value, String message);"),
            "{java}"
        );
        assert!(
            java.contains("public static void watch_native_emit(long done, byte[] value) { dayEmit_watch_native(done, 0, value, null); }"),
            "{java}"
        );
        assert!(
            java.contains("public static void watch_native_end(long done) { dayEmit_watch_native(done, 2, null, null); }"),
            "{java}"
        );
        let kotlin = render_kotlin(&b, java_arm, "day-part-demo");
        assert!(
            kotlin.contains("@JvmStatic fun watch_native_end(done: Long) { dayEmit_watch_native(done, 2, null, null) }"),
            "{kotlin}"
        );

        let arkts_arm = b.arms.iter().find(|a| a.lang == Lang::ArkTs).unwrap();
        let ets = render_arkts(&b, arkts_arm, "day-part-demo");
        assert!(
            ets.contains("function watch_native_end(done: number): void {\n  nativeEntry.dayBridgeComplete('day_bridge_emit_arkts_day_part_demo_watch_native', done, 2, undefined, '');\n}"),
            "{ets}"
        );
    }

    #[test]
    fn a_done_and_an_emit_cannot_share_a_function() {
        let both = STREAM.replace(
            "fn tick_native(emit: Emit<()>) -> Result<(), day_bridge::Error>;",
            "fn tick_native(done: Done<()>, emit: Emit<()>) -> Result<(), day_bridge::Error>;",
        );
        let err = validate(&parse(&both)).expect_err("two handles");
        assert!(err.contains("single, last argument"), "{err}");
    }
}
