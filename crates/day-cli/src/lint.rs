// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day lint v0 (DESIGN.md §16.5): fluent coverage (missing/unused/unknown keys), duplicate
//! element ids, unknown navigation routes, Day.toml schema (validated by parsing). Fast —
//! sources + locales + scripts only.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::meta::Project;
use crate::ops::{gha_escape, github_actions};
use crate::term::{DIM, ERROR, SUCCESS, WARN};
use anstream::eprintln;

/// How much a finding matters. A property of the RULE, not of the instance — see
/// [`severity_of`], which is the single place the policy lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Severity {
    /// Names something that does not exist, or that will misbehave at runtime.
    Error,
    #[default]
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// Where a finding is, when the rule can say. Project-RELATIVE path, 1-based line and column —
/// the shape an editor wants and the shape a human reads in a terminal.
#[derive(Debug, Clone, Default)]
pub struct Location {
    pub file: String,
    pub line: usize,
    pub column: usize,
}

/// A repair a rule can describe precisely enough to apply unattended.
///
/// Only rules whose remedy is both SAFE (reversible, no content invented) and UNAMBIGUOUS (exactly
/// one right answer) carry one — `day lint --fix` applies these without asking, so a fix that
/// needed a human decision would be a way to lose work rather than a convenience.
#[derive(Debug, Clone)]
pub struct Fix {
    /// What the fix does, in the imperative — shown by `--fix` and used as the editor's lightbulb
    /// title.
    pub title: String,
    /// Project-relative file to rewrite.
    pub file: String,
    /// Its complete new contents. Whole-file rather than a range so applying is a single
    /// deterministic write, with nothing to reconcile against a file that moved underneath.
    pub contents: String,
}

#[derive(Debug, Default)]
pub struct Finding {
    pub code: &'static str,
    pub message: String,
    /// Where it is, for rules that know. `None` for findings about something ABSENT — a missing
    /// directory, a locale that exists on no surface, a package missing from the host.
    pub location: Option<Location>,
    /// A safe, unambiguous repair, for the few rules that have one.
    pub fix: Option<Fix>,
}

impl Finding {
    pub fn located(mut self, at: Location) -> Self {
        self.location = Some(at);
        self
    }

    /// Attach a place only when the caller has one — the shape most checks are in, where a
    /// position exists for a key that was FOUND and not for one that was missing.
    pub fn maybe_located(mut self, at: Option<Location>) -> Self {
        self.location = at;
        self
    }

    pub fn severity(&self) -> Severity {
        severity_of(self.code)
    }
}

impl Location {
    /// A place inside a file whose text we have, from a byte offset into it — how the Fluent
    /// parser and the source scanners both report a match.
    pub fn in_file(file: impl Into<String>, src: &str, offset: usize) -> Location {
        let (line, column) = day_build::line_col(src, offset);
        Location {
            file: file.into(),
            line,
            column,
        }
    }

    /// The top of a file, for a finding that is ABOUT the file rather than about a line in it.
    pub fn head(file: impl Into<String>) -> Location {
        Location {
            file: file.into(),
            line: 1,
            column: 1,
        }
    }
}

/// Which rules are errors rather than warnings.
///
/// The test is whether the finding names something that DOES NOT EXIST, or that will misbehave at
/// runtime: a `tr("…")` with no message renders its own key, a route nothing declares navigates
/// nowhere, an unknown target or manifest override is simply not read. Everything else — coverage
/// gaps, store text, style — stays a warning.
///
/// Presentational only: `--strict` still fails on ANY active finding, error or warning, so this
/// changes what a reader sees and what an editor squiggles red, never whether existing CI passes.
pub fn severity_of(code: &str) -> Severity {
    const ERRORS: &[&str] = &[
        "day::lint::unknown-key",
        "day::lint::unknown-route",
        "day::lint::unknown-target",
        "day::lint::unknown-override",
        "day::lint::unknown-function",
        "day::lint::bad-format-option",
        "day::lint::undeclared-permission",
        "day::lint::duplicate-id",
        "day::lint::vector-parse",
        "day::lint::vector-unreadable",
        "day::lint::store-unreadable",
        "day::lint::shortcut-label",
    ];
    if ERRORS.contains(&code) {
        Severity::Error
    } else {
        Severity::Warning
    }
}

/// Collect keys referenced via the generated `res::str::<key>(…)` functions (§18.5). Unlike
/// `tr("key")` these aren't quote-delimited: after `res::str::` (possibly through a `crate::`/module
/// path) read the Rust identifier, stripping a `r#` raw prefix — that identifier is the Fluent key.
/// A literal (or identifier) found in source, with WHERE it was found — so a finding about it can
/// point at the line rather than at the project as a whole.
#[derive(Debug, Clone)]
struct Hit {
    text: String,
    file: std::path::PathBuf,
    line: usize,
    column: usize,
}

impl Hit {
    /// Record `text`, whose position is taken from where it sits inside `src`. `text` must be a
    /// subslice of `src` — every scan below carves it out of the file it just read.
    fn found(file: &Path, src: &str, text: &str) -> Hit {
        let (line, column) = day_build::line_col(src, day_build::offset_in(src, text).unwrap_or(0));
        Hit {
            text: text.to_string(),
            file: file.to_path_buf(),
            line,
            column,
        }
    }

    fn location(&self, root: &Path) -> Location {
        Location {
            file: rel(root, &self.file),
            line: self.line,
            column: self.column,
        }
    }
}

/// A path as a finding reports it: relative to the project, forward slashes on every platform —
/// which is what an editor resolves against the workspace folder.
fn rel(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Just the texts, for the checks that only ask whether something was mentioned.
fn texts(hits: &[Hit]) -> BTreeSet<String> {
    hits.iter().map(|h| h.text.clone()).collect()
}

/// Walk `dir` for Rust sources, handing each one's path and text to `f`. Every scan below shares
/// this walk and differs only in what it matches.
fn for_each_rs(dir: &Path, f: &mut impl FnMut(&Path, &str)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            for_each_rs(&p, f);
        } else if p.extension().is_some_and(|x| x == "rs")
            && let Ok(src) = std::fs::read_to_string(&p)
        {
            f(&p, &src);
        }
    }
}

/// Keys referenced through the generated typed functions (`res::str::<key>(…)`, §18.5) — the
/// symbol IS the key, so a call counts as a reference exactly like `tr("key")`.
fn scan_res_str(dir: &Path, out: &mut Vec<Hit>) {
    for_each_rs(dir, &mut |path, src| {
        let pat = "res::str::";
        let mut rest = src;
        while let Some(i) = rest.find(pat) {
            rest = &rest[i + pat.len()..];
            let s = rest.strip_prefix("r#").unwrap_or(rest);
            let end = s
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(s.len());
            if end > 0 {
                out.push(Hit::found(path, src, &s[..end]));
            }
        }
    });
}

/// Collect portable permissions referenced in code as `Permission::<Variant>` (docs/permissions.md).
///
/// Reads an identifier rather than a quoted literal. The contract with `day-part-permissions` is
/// that its enum is called `Permission` and its variants are the table's `variant` spellings —
/// pinned by `tests/permissions_parity.rs`.
fn scan_permission_uses(dir: &Path, out: &mut Vec<Hit>) {
    for_each_rs(dir, &mut |path, src| {
        let pat = "Permission::";
        let mut rest = src;
        while let Some(i) = rest.find(pat) {
            rest = &rest[i + pat.len()..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            if end > 0 {
                out.push(Hit::found(path, src, &rest[..end]));
            }
        }
    });
}

/// Every Rust source root the lint scans: the project package's `src/` plus each WORKSPACE
/// MEMBER crate's `src/` inside the project directory (a multi-crate app keeps its
/// `tr("key")` / `.id("…")` literals in member crates too — Day-Games' games live in
/// `games/<name>/src`). A member is any `src/` directory beside a `Cargo.toml`, found by a
/// shallow walk that skips build products and the native host projects.
fn source_roots(root: &Path) -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    fn walk(dir: &Path, depth: usize, roots: &mut Vec<std::path::PathBuf>) {
        if depth > 3 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if matches!(
                name.as_str(),
                "target" | "build" | "platform" | "resource" | "store" | "dayscript" | ".git"
            ) {
                continue;
            }
            if name == "src" && dir.join("Cargo.toml").exists() {
                roots.push(p);
                continue;
            }
            walk(&p, depth + 1, roots);
        }
    }
    walk(root, 0, &mut roots);
    roots.sort();
    roots
}

/// Every quoted string in the app's Rust sources that could BE a key.
///
/// A key is not always reached through `tr("…")` or `res::str::…`: naming a set of them in a const
/// array and resolving with `tr(*k)` is how an app enumerates options, and the scaffold's own
/// `KINDS` does exactly that. Those keys are used, and the two scans above cannot see it.
///
/// Consulted ONLY by the unused-key check. It must not feed `unknown-key`, which asks the opposite
/// question — every literal in the program is not a claim that a message exists.
fn scan_key_like_literals(dir: &Path, out: &mut BTreeSet<String>) {
    for_each_rs(dir, &mut |_, src| {
        for lit in src.split('"').skip(1).step_by(2) {
            // Fluent key shape: lowercase, digits, `_` and `-`, and never empty.
            if !lit.is_empty()
                && lit.len() <= 64
                && lit
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
            {
                out.insert(lit.to_string());
            }
        }
    });
}

/// Every literal that follows `pat` up to the closing quote — `tr("`, `.id("`, `navigate("`.
fn scan_sources(dir: &Path, pat: &str, out: &mut Vec<Hit>) {
    for_each_rs(dir, &mut |path, src| {
        let mut rest = src;
        while let Some(i) = rest.find(pat) {
            rest = &rest[i + pat.len()..];
            if let Some(end) = rest.find('"') {
                out.push(Hit::found(path, src, &rest[..end]));
                rest = &rest[end..];
            }
        }
    });
}

/// The first path segment of a route string (`"a/b?x=1"` → `"a"`) — the part a lint can check
/// against declared selector/tabs item keys. Deeper segments are open-ended (stack destination
/// builders accept any key), so only the first is validated.
fn route_first_segment(route: &str) -> &str {
    route.split(['/', '?']).next().unwrap_or("")
}

/// Collect the `Variant => "key"` literals declared inside `routes! { … }` blocks — typed
/// selectors declare their keys there instead of at `.item("key", …)` call sites.
fn scan_routes_macro_keys(dir: &Path, out: &mut Vec<Hit>) {
    for_each_rs(dir, &mut |path, src| {
        let mut rest = src;
        while let Some(i) = rest.find("routes!") {
            rest = &rest[i + "routes!".len()..];
            // The macro body is the outermost `{ … }` after `routes!` (brace-balanced).
            let Some(open) = rest.find('{') else { continue };
            let mut depth = 0usize;
            let mut end = rest.len();
            for (j, c) in rest[open..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + j;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let mut body = &rest[open..end];
            while let Some(k) = body.find("=> \"") {
                body = &body[k + 4..];
                if let Some(q) = body.find('"') {
                    out.push(Hit::found(path, src, &body[..q]));
                    body = &body[q..];
                }
            }
            rest = &rest[end..];
        }
    });
}

/// Cross-reference every dayscript `screenshot:` step's localized `title:`/`caption:` locale
/// keys against the app's translation locales (see the caller's comment for the rules).
fn check_screenshot_locales(
    root: &Path,
    dir: &Path,
    app_locales: &[String],
    findings: &mut Vec<Finding>,
) {
    let lang = |t: &str| t.split(['-', '_']).next().unwrap_or(t).to_ascii_lowercase();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            check_screenshot_locales(root, &p, app_locales, findings);
            continue;
        }
        if !p.extension().is_some_and(|x| x == "yaml" || x == "yml") {
            continue;
        }
        let file = p.file_name().map(|f| f.to_string_lossy().into_owned());
        let file = file.as_deref().unwrap_or("dayscript");
        // The step's own line is inside a parsed script this check never sees as text; the script
        // it is in is the place to send the reader.
        let at = Location::head(rel(root, &p));
        for (shot, meta) in crate::screenshot::script_screenshot_meta(&p) {
            for (kind, text) in [("title", &meta.title), ("caption", &meta.caption)] {
                let Some(text) = text else { continue };
                let keys = text.locales();
                if keys.is_empty() {
                    continue; // a plain string localizes nothing — nothing to check
                }
                for l in app_locales {
                    if !keys.iter().any(|k| lang(k) == lang(l)) {
                        findings.push(
                            Finding {
                                code: "day::lint::screenshot-locales",
                                message: format!(
                                    "{file}: screenshot {shot:?} {kind} has no {l:?} — that \
                                 locale's gallery page falls back to English"
                                ),
                                ..Default::default()
                            }
                            .located(at.clone()),
                        );
                    }
                }
                for k in &keys {
                    if !app_locales.iter().any(|l| lang(l) == lang(k)) {
                        findings.push(
                            Finding {
                                code: "day::lint::screenshot-locales",
                                message: format!(
                                    "{file}: screenshot {shot:?} {kind} names {k:?}, which is not \
                                 one of the app's locales ({})",
                                    app_locales.join(", ")
                                ),
                                ..Default::default()
                            }
                            .located(at.clone()),
                        );
                    }
                }
            }
        }
    }
}

/// Collect `route:` values from dayscript `navigate:` / `assert_route:` steps — and the
/// route inside every `deep_link:` step's `url:` (docs/deep-links.md) — in
/// `dayscript/*.yaml`: the same route namespace `navigate()` uses (docs/navigation.md).
fn scan_script_routes(dir: &Path, out: &mut Vec<Hit>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            scan_script_routes(&p, out);
        } else if p.extension().is_some_and(|x| x == "yaml" || x == "yml")
            && let Ok(src) = std::fs::read_to_string(&p)
        {
            for line in src.lines() {
                let l = line.trim_start();
                if l.starts_with("- deep_link:") {
                    if let Some(i) = l.rfind("url:") {
                        let rest = &l[i + "url:".len()..];
                        let v = rest
                            .split(',')
                            .next()
                            .unwrap_or(rest)
                            .trim()
                            .trim_end_matches(['}', ' '])
                            .trim()
                            .trim_matches(['"', '\'']);
                        if !v.is_empty() {
                            // The route half only — the lint checks route keys, not schemes;
                            // params are the destination's concern. Mirrors
                            // `day_spec::route_of_url` (day-cli doesn't link day-spec).
                            let route = v.split_once("://").map(|(_, r)| r).unwrap_or(v);
                            let route = route.split('?').next().unwrap_or(route);
                            out.push(Hit::found(&p, &src, route));
                        }
                    }
                    continue;
                }
                if !(l.starts_with("- navigate:") || l.starts_with("- assert_route:")) {
                    continue;
                }
                // rfind: `assert_route:` itself contains "route:" — the value's key is last.
                if let Some(i) = l.rfind("route:") {
                    // The value ends at the next key in the inline map, not at the end of the
                    // line: `{ route: webview, skip_on: [harmony-arkui] }` is a route of
                    // "webview", and reading to the brace reported the whole tail as an unknown
                    // route on every scripted step that carries a filter.
                    let rest = &l[i + "route:".len()..];
                    let v = rest
                        .split(',')
                        .next()
                        .unwrap_or(rest)
                        .trim()
                        .trim_end_matches(['}', ' '])
                        .trim()
                        .trim_matches(['"', '\'']);
                    if !v.is_empty() {
                        out.push(Hit::found(&p, &src, v));
                    }
                }
            }
        }
    }
}

/// Where `needle` first appears in a file's text, as a place a finding can point at.
///
/// For the checks whose subject came out of a PARSER that kept no spans — the manifest, chiefly.
/// Searching the source for the value is approximate (a string that occurs twice reports the
/// first), and better than sending the reader to line 1.
fn locate_in(file: &str, src: &str, needle: &str) -> Option<Location> {
    src.find(needle).map(|at| Location::in_file(file, src, at))
}

/// Does `--allow CODE` cover this finding? The `day::lint::` prefix is optional, so
/// `--allow store-placeholder` and `--allow day::lint::store-placeholder` name the same one.
fn allowed(code: &str, allow: &[String]) -> bool {
    allow.iter().any(|a| {
        let a = a.trim();
        code == a || code.strip_prefix("day::lint::") == Some(a)
    })
}

/// Check the project and report. `json` swaps the human report for the editor envelope; `fix`
/// applies the repairs the rules proposed before either.
pub fn run(project: &Project, strict: bool, allow: &[String], json: bool, fix: bool) -> i32 {
    let mut findings = collect(project);
    if fix {
        if !findings
            .iter()
            .any(|f| f.fix.is_some() && !allowed(f.code, allow))
        {
            eprintln!("{DIM}--fix{DIM:#} no finding proposes a fix that can be applied unattended");
        }
        // Two rules can propose a repair for the SAME file — a keyword list with both stray
        // spaces and trailing whitespace — and each was computed against the text as it was, so
        // only one of them can be applied per pass. Re-check and go again until nothing is left.
        for _ in 0..8 {
            if apply_fixes(project, &findings, allow) == 0 {
                break;
            }
            findings = collect(project);
        }
    }
    if json {
        return report_json(project, &findings, allow, strict);
    }
    report(&findings, allow, strict)
}

/// Write every safe fix that is not waived, one file at a time, saying what happened to each.
///
/// Waived codes are skipped on purpose: `--allow` says a finding may stand, and rewriting the file
/// it named would be the opposite of letting it stand.
fn apply_fixes(project: &Project, findings: &[Finding], allow: &[String]) -> usize {
    let mut applied = 0;
    let mut written: BTreeSet<String> = BTreeSet::new();
    for f in findings {
        let Some(fix) = &f.fix else { continue };
        if allowed(f.code, allow) || !written.insert(fix.file.clone()) {
            continue;
        }
        let path = project.root.join(&fix.file);
        match std::fs::write(&path, &fix.contents) {
            Ok(()) => {
                eprintln!(
                    "{SUCCESS}fixed{SUCCESS:#}   {:<32} {}: {}",
                    f.code, fix.file, fix.title
                );
                applied += 1;
            }
            Err(e) => eprintln!("{ERROR}unfixed{ERROR:#} {:<32} {}: {e}", f.code, fix.file),
        }
    }
    applied
}

/// Everything the rules found, in no particular order — the reporting below decides what to do
/// with them. Split out so `--fix` can re-check after writing without re-entering the report.
fn collect(project: &Project) -> Vec<Finding> {
    let mut findings: Vec<Finding> = Vec::new();
    // The manifest as TEXT. It parsed to reach here, but the parsed form keeps no spans, so the
    // checks below find their own value in the source to report a line.
    let manifest_src = std::fs::read_to_string(project.root.join("Day.toml")).ok();

    // --- resource/vectors/ (docs/vectors.md) ---
    // Parse every vector source and surface the problems a device test would otherwise find
    // first: unparseable art, glyph-embedded <text> (shaping is not compiled in), a template
    // without its canonical Regular variant, and art outside the VectorDrawable subset (which
    // ships as a raster fallback on Android — worth knowing, not an error).
    lint_vectors(project, &mut findings);

    // --- daybridge (docs/bridge.md) ---
    // A Kotlin arm needs the Kotlin Gradle plugin; without it Gradle ignores the generated .kt
    // and the failure surfaces as a runtime ClassNotFoundException on a device, which is the
    // worst possible place to learn it.
    if project
        .manifest
        .app
        .targets
        .iter()
        .any(|t| t == "android-mdc")
    {
        let kotlin_arms = crate::bridge::kotlin_arm_crates(project);
        if !kotlin_arms.is_empty() && !crate::bridge::android_compiles_kotlin(project) {
            findings.push(Finding {
                code: "day::lint::bridge-kotlin-plugin",
                message: crate::bridge::kotlin_plugin_help(&kotlin_arms),
                ..Default::default()
            });
        }
    }

    // A bridged C/C++ arm's `link = [...]` needs the library's dev package on THIS machine, and
    // the failure without it is a linker wall naming no crate (docs/bridge.md "Linking").
    let missing = crate::bridge::unresolved_link_libs(project);
    if !missing.is_empty() {
        findings.push(Finding {
            code: "day::lint::bridge-link-missing",
            message: crate::bridge::link_help(&missing),
            ..Default::default()
        });
    }

    // --- Day.toml structure ---
    // Syntax + schema are enforced at load (a project that reaches here parsed); lint adds the
    // semantic checks: every [app] target is a known combo, and every [app.<key>] override
    // table names a known platform, toolkit, or target.
    for t in &project.manifest.app.targets {
        // Combined catalog: a target declared by a dependency crate's
        // [package.metadata.day.toolkit] is as known as a builtin (docs/extending.md).
        if !crate::external::known(project, t) {
            findings.push(
                Finding {
                    code: "day::lint::unknown-target",
                    message: format!("Day.toml: targets entry {t:?} is not a known target"),
                    ..Default::default()
                }
                .maybe_located(
                    manifest_src
                        .as_deref()
                        .and_then(|src| locate_in("Day.toml", src, &format!("{t:?}"))),
                ),
            );
        }
    }
    {
        use std::collections::BTreeSet;
        let mut known: BTreeSet<&str> = BTreeSet::new();
        for t in crate::targets::TARGETS {
            known.insert(t.name); // "macos-appkit"
            known.insert(t.toolkit); // "appkit"
            known.insert(t.os); // "macos" — and "harmony" for harmony-arkui
        }
        // The pre-rename spelling of the harmony platform key — still honored by the
        // override resolution (meta.rs), so it isn't an unknown table.
        known.insert("ohos");
        for key in project.manifest.app.overrides.keys() {
            if !known.contains(key.as_str()) {
                findings.push(
                    Finding {
                        code: "day::lint::unknown-override",
                        message: format!(
                            "Day.toml: [app.{key}] does not name a known platform, toolkit, or \
                             target"
                        ),
                        ..Default::default()
                    }
                    .maybe_located(
                        manifest_src
                            .as_deref()
                            .and_then(|src| locate_in("Day.toml", src, &format!("[app.{key}]"))),
                    ),
                );
            }
        }
    }

    // --- Store listings (§16.6) ---
    // Held to the stores' own rules, because the alternative is learning them from a rejection
    // days after the upload. Silent for an app that ships to neither store.
    match crate::store::read(project) {
        Ok(listing) => {
            for p in crate::store::lint(project, &listing) {
                // A listing field is one value in one small file, so the head of that file IS the
                // finding's place; the rules that carry a repair rewrite the file whole.
                findings.push(Finding {
                    code: p.code,
                    message: p.message,
                    location: p.file.map(Location::head),
                    fix: p.fix,
                });
            }
        }
        Err(e) => findings.push(Finding {
            code: "day::lint::store-unreadable",
            message: e,
            ..Default::default()
        }),
    }

    // --- Locale surface sync (`day localize`, DESIGN.md §16.5) ---
    // The four locale surfaces (resource/locales/, store/, Xcode's knownRegions, the website's
    // locales array) drift the moment one is edited by hand. Each present surface is compared
    // against the union of all of them, with the same advice `day localize list` prints.
    {
        let survey = crate::localize::survey(&project.root);
        for (message, advice) in crate::localize::sync_findings(&survey) {
            findings.push(Finding {
                code: "day::lint::locale-sync",
                message: format!("{message} — {advice}"),
                ..Default::default()
            });
        }

        // --- Screenshot gallery metadata locales (DESIGN.md §14.7) ---
        // A `screenshot:` step's localized `title:`/`caption:` feeds the published gallery
        // index (`day screenshot index`), so its locale keys must track the app's translation
        // locales: an app locale the map lacks silently ships the English title on that
        // locale's gallery page, and a key naming a locale the app does not have is dead
        // weight — usually a typo. Comparison is by primary language (`fr` covers `fr-FR`),
        // the same rule the gallery's own resolution uses. A plain-string title is fine: an
        // app that does not localize its gallery has nothing to keep in sync.
        if !survey.fluent.is_empty() {
            check_screenshot_locales(
                &project.root,
                &project.root.join("dayscript"),
                &survey.fluent,
                &mut findings,
            );
        }
    }

    // --- Permission declarations (docs/permissions.md) ---
    // The backstop for the whole declaration pipeline: an undeclared permission reports Restricted
    // on Android and TERMINATES the app on iOS the first time it touches the API. Catching it here
    // turns a crash on a device into a lint failure.
    {
        let mut used = Vec::new();
        for root in source_roots(&project.root) {
            scan_permission_uses(&root, &mut used);
        }
        // One finding per VARIANT, reported at its first use: a permission requested from six
        // call sites is still one missing declaration.
        let mut seen_variants = BTreeSet::new();
        used.retain(|h| seen_variants.insert(h.text.clone()));
        used.sort_by(|a, b| a.text.cmp(&b.text));
        let declared = &project.manifest.permissions.declared;
        for hit in &used {
            let variant = &hit.text;
            let at = hit.location(&project.root);
            let Some(spec) = day_build::permissions::find_variant(variant) else {
                continue; // Raw(…) and any non-portable variant have nothing to declare
            };
            match declared.get(spec.name) {
                None => findings.push(Finding {
                    code: "day::lint::undeclared-permission",
                    message: format!(
                        "code requests Permission::{variant}, but Day.toml has no [permissions] \
                         entry for {:?} — iOS terminates an app that touches the API without its \
                         usage description",
                        spec.name
                    ),
                    ..Default::default()
                }
                .located(at.clone())),
                Some(decl) if !decl.enabled() => findings.push(Finding {
                    code: "day::lint::undeclared-permission",
                    message: format!(
                        "code requests Permission::{variant}, but Day.toml declares {:?} = false",
                        spec.name
                    ),
                    ..Default::default()
                }
                .located(at.clone())),
                Some(decl) if spec.needs_reason && decl.reason_for("ios").is_none() => findings
                    .push(Finding {
                        code: "day::lint::missing-reason",
                        message: format!(
                            "[permissions] {:?} has no reason — it is the text iOS and HarmonyOS \
                             show the user when they prompt",
                            spec.name
                        ),
                        ..Default::default()
                    }
                    .located(at.clone())),
                Some(_) => {}
            }
        }

        // Has a build actually written the declarations into the checked-in iOS manifest? The
        // Android overlay is gitignored and regenerated every build, so there is nothing stale to
        // find there — checking it would only produce false alarms on a fresh clone.
        if let Some(plist) = crate::mobile::app_info_plist(project)
            && let Ok(text) = std::fs::read_to_string(&plist)
            && let Ok(plan) = crate::permissions::resolve(&project.manifest, "ios", &[])
        {
            let have = crate::plist::read_string_keys(&text);
            let want = crate::permissions::apple_keys(&plan, false);
            let missing: Vec<&String> = want
                .iter()
                .filter(|(k, v)| have.get(*k) != Some(*v))
                .map(|(k, _)| k)
                .collect();
            if !missing.is_empty() {
                findings.push(Finding {
                    code: "day::lint::stale-manifest",
                    message: format!(
                        "platform/ios/Runner/Info.plist is missing or out of date for {} — run \
                         `day build -p ios-uikit` to regenerate it",
                        missing
                            .iter()
                            .map(|k| k.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    ..Default::default()
                });
            }
        }
    }

    // --- Fluent coverage ---
    let locales_dir = project.root.join("resource/locales");
    // locale → message key → where that key is DEFINED. Carrying the definition site is what lets
    // a finding about a key open the .ftl at its line instead of at the directory.
    let mut locales: BTreeMap<String, BTreeMap<String, Location>> = BTreeMap::new();
    // locale → one .ftl to blame for a key the catalog is MISSING, which has no line of its own.
    let mut locale_files: BTreeMap<String, String> = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(&locales_dir) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                let name = e.file_name().to_string_lossy().to_string();
                let mut keys: BTreeMap<String, Location> = BTreeMap::new();
                if let Ok(files) = std::fs::read_dir(e.path()) {
                    // read_dir order is arbitrary; sorting keeps the reported file stable across
                    // runs and machines, which a diagnostic that moves would not be.
                    let mut paths: Vec<std::path::PathBuf> =
                        files.flatten().map(|f| f.path()).collect();
                    paths.sort();
                    for path in paths {
                        if path.extension().is_some_and(|x| x == "ftl")
                            && let Ok(src) = std::fs::read_to_string(&path)
                        {
                            let file = rel(&project.root, &path);
                            locale_files.entry(name.clone()).or_insert(file.clone());
                            for (key, offset) in day_build::ftl_key_offsets(&src) {
                                keys.entry(key)
                                    .or_insert_with(|| Location::in_file(&file, &src, offset));
                            }
                        }
                    }
                }
                locales.insert(name, keys);
            }
        }
    }
    let roots = source_roots(&project.root);
    let mut used_keys = Vec::new();
    for r in &roots {
        scan_sources(r, "tr(\"", &mut used_keys);
        // Keys referenced through the generated typed functions (`res::str::<key>(…)`, §18.5) —
        // the symbol IS the key (snake_case), so they count as used like a `tr("key")` literal.
        scan_res_str(r, &mut used_keys);
    }
    let used = texts(&used_keys);
    // Where each key is first referenced, so `unknown-key` points at the `tr("…")` that will fail.
    let mut first_use: BTreeMap<String, Location> = BTreeMap::new();
    for h in &used_keys {
        first_use
            .entry(h.text.clone())
            .or_insert_with(|| h.location(&project.root));
    }
    // See `scan_key_like_literals`: a key named in a plain string literal and resolved later is
    // still referenced, and only the unused-key check may consider it.
    let mut literals: BTreeSet<String> = BTreeSet::new();
    for r in &roots {
        scan_key_like_literals(r, &mut literals);
    }

    // Default = "en" if present, else first.
    let default_name = if locales.contains_key("en") {
        "en".to_string()
    } else {
        locales.keys().next().cloned().unwrap_or_default()
    };
    if let Some(default_keys) = locales.get(&default_name).cloned() {
        for k in &used {
            if !default_keys.contains_key(k) {
                findings.push(
                    Finding {
                        code: "day::lint::unknown-key",
                        message: format!(
                            "tr({k:?}) has no message in resource/locales/{default_name}"
                        ),
                        ..Default::default()
                    }
                    .maybe_located(first_use.get(k).cloned()),
                );
            }
        }
        for (k, at) in &default_keys {
            // Convention keys the framework consumes at build time, not from app source:
            // `language_name` is read by day-build's generated `res::locales::ALL` (each catalog
            // naming its own language for pickers — docs/localization.md), so no `res::str::` or
            // `tr("…")` reference exists for the scan to find.
            if k == "language_name" {
                continue;
            }
            if !used.contains(k) && !literals.contains(k) {
                findings.push(
                    Finding {
                        code: "day::lint::unused-key",
                        message: format!(
                            "resource/locales/{default_name}: {k} is never referenced"
                        ),
                        ..Default::default()
                    }
                    .located(at.clone()),
                );
            }
        }
        for (name, keys) in &locales {
            if name == &default_name {
                continue;
            }
            for k in default_keys.keys() {
                if !keys.contains_key(k) {
                    findings.push(
                        Finding {
                            code: "day::lint::missing-translation",
                            message: format!("resource/locales/{name}: missing {k}"),
                            ..Default::default()
                        }
                        // The key is ABSENT, so there is no line to point at — the catalog that
                        // should have it is as close as this gets.
                        .maybe_located(locale_files.get(name).map(Location::head)),
                    );
                }
            }
        }
    }

    // --- Fluent formatting functions (docs/localization.md "Formatted values") ---
    // day-l10n registers exactly NUMBER and DATETIME on every bundle; anything else renders as an
    // error marker at runtime, and a misspelled option silently falls back to defaults — both are
    // author mistakes worth catching per locale file here.
    if let Ok(entries) = std::fs::read_dir(&locales_dir) {
        for e in entries.flatten() {
            if !e.path().is_dir() {
                continue;
            }
            let locale = e.file_name().to_string_lossy().to_string();
            let Ok(files) = std::fs::read_dir(e.path()) else {
                continue;
            };
            for f in files.flatten() {
                if f.path().extension().is_none_or(|x| x != "ftl") {
                    continue;
                }
                let Ok(src) = std::fs::read_to_string(f.path()) else {
                    continue;
                };
                let file = rel(&project.root, &f.path());
                for call in day_build::function_calls(&src) {
                    findings.extend(lint_ftl_call(&locale, &file, &src, &call));
                }
            }
        }
    }

    // --- Unknown routes (docs/navigation.md) ---
    // Literal `navigate("…")` calls and dayscript navigate / assert_route steps must START
    // with a declared item key — `.item("key", …)` for string-keyed apps, `routes! { X =>
    // "key" }` for typed ones (typed `.item(Section::X, …)` call sites are already
    // compile-checked; this covers the scripts and raw strings). Skipped when the app
    // declares no keys either way (a pure-stack app's routes are open-ended).
    let mut declared_keys = Vec::new();
    for r in &roots {
        scan_sources(r, ".item(\"", &mut declared_keys);
        scan_routes_macro_keys(r, &mut declared_keys);
    }
    if !declared_keys.is_empty() {
        let declared = texts(&declared_keys);
        let mut used_routes: Vec<(String, String, Option<Location>)> = Vec::new();
        let mut nav_calls = Vec::new();
        for r in &roots {
            scan_sources(r, "navigate(\"", &mut nav_calls);
        }
        used_routes.extend(nav_calls.into_iter().map(|h| {
            let at = h.location(&project.root);
            ("navigate".to_string(), h.text, Some(at))
        }));
        let mut script_routes = Vec::new();
        scan_script_routes(&project.root.join("dayscript"), &mut script_routes);
        used_routes.extend(script_routes.into_iter().map(|h| {
            let at = h.location(&project.root);
            ("dayscript".to_string(), h.text, Some(at))
        }));
        // [[shortcuts]] routes are saved deep links (docs/deep-links.md) — same check,
        // query params stripped the way the route parser will strip them.
        used_routes.extend(project.manifest.shortcuts.iter().map(|s| {
            let route = s.route.split('?').next().unwrap_or(&s.route).to_string();
            let at = manifest_src
                .as_deref()
                .and_then(|src| locate_in("Day.toml", src, &format!("{:?}", s.route)));
            ("Day.toml [[shortcuts]]".to_string(), route, at)
        }));
        for (origin, route, at) in &used_routes {
            let first = route_first_segment(route);
            if !first.is_empty() && !declared.contains(first) {
                findings.push(
                    Finding {
                        code: "day::lint::unknown-route",
                        message: format!(
                            "{origin}: route {route:?} starts with {first:?}, which no `.item(…)` \
                             or `routes! {{ … }}` declares"
                        ),
                        ..Default::default()
                    }
                    .maybe_located(at.clone()),
                );
            }
        }
    }

    // --- Shortcut labels (docs/deep-links.md) ---
    // Every [[shortcuts]] label must be a single-line static message present in EVERY locale:
    // the native launcher renders the conveyed string with no formatter behind it. `day build`
    // enforces the same rules; lint catches them without needing a platform build.
    if !project.manifest.shortcuts.is_empty() {
        match crate::shortcuts::resolved(project) {
            Ok(list) if list.len() > 4 => findings.push(Finding {
                code: "day::lint::shortcut-count",
                message: format!(
                    "{} shortcuts declared; launchers show at most about four, so the rest \
                     may be dropped",
                    list.len()
                ),
                ..Default::default()
            }),
            Ok(_) => {}
            Err(e) => findings.push(Finding {
                code: "day::lint::shortcut-label",
                message: e,
                ..Default::default()
            }),
        }
    }

    // --- Duplicate ids ---
    let mut ids = Vec::new();
    for r in &roots {
        scan_sources(r, ".id(\"", &mut ids);
    }
    let mut first_id: BTreeMap<String, Location> = BTreeMap::new();
    for hit in &ids {
        let at = hit.location(&project.root);
        match first_id.get(&hit.text) {
            Some(first) => findings.push(
                Finding {
                    code: "day::lint::duplicate-id",
                    message: format!(
                        "element id {:?} is already used at {}:{}",
                        hit.text, first.file, first.line
                    ),
                    ..Default::default()
                }
                .located(at),
            ),
            None => {
                first_id.insert(hit.text.clone(), at);
            }
        }
    }

    findings
}

/// The human report: one line per active finding, one summary line per waived code.
fn report(findings: &[Finding], allow: &[String], strict: bool) -> i32 {
    // An allowed code still reports, one summary line per code rather than per finding: a
    // scaffold's 84 store placeholders would otherwise bury the warnings that do matter. The
    // count and a sample are enough to see what a stale `--allow` is covering.
    let mut waived: BTreeMap<&str, (usize, &str)> = BTreeMap::new();
    let gha = github_actions();
    let mut active: Vec<&Finding> = Vec::new();
    for f in findings {
        if allowed(f.code, allow) {
            let e = waived.entry(f.code).or_insert((0, f.message.as_str()));
            e.0 += 1;
            continue;
        }
        let where_ = match &f.location {
            Some(at) => format!(" {DIM}({}:{}){DIM:#}", at.file, at.line),
            None => String::new(),
        };
        match f.severity() {
            Severity::Error => {
                eprintln!(
                    "{ERROR}error{ERROR:#}   {:<32} {}{where_}",
                    f.code, f.message
                )
            }
            Severity::Warning => {
                eprintln!("{WARN}warning{WARN:#} {:<32} {}{where_}", f.code, f.message)
            }
        }
        if gha {
            // GitHub reads workflow commands off STDOUT (the human report above is stderr, which
            // never becomes an annotation). With a file and line the annotation lands ON the
            // offending line in the PR diff; without one it stays a job-level note. Newlines must
            // be %0A-escaped per the docs.
            let place = match &f.location {
                Some(at) => format!(",file={},line={},col={}", at.file, at.line, at.column),
                None => String::new(),
            };
            println!(
                "::{} title=day lint {}{place}::{}",
                f.severity().as_str(),
                f.code,
                gha_escape(&f.message)
            );
        }
        active.push(f);
    }
    for (code, (n, sample)) in &waived {
        eprintln!("{DIM}allowed{DIM:#} {code:<32} {n} finding(s), e.g. {sample}");
    }
    if gha {
        write_step_summary(&active, &waived);
    }
    let waived_n: usize = waived.values().map(|(n, _)| n).sum();
    finish(findings.len() - waived_n, waived_n, strict)
}

/// The editor envelope: every finding with its place, its severity and its repair, so a tool can
/// draw squiggles and offer a lightbulb without re-deriving any of it.
///
/// Waived findings are INCLUDED, flagged rather than dropped — an editor showing them greyed is a
/// better way to notice a stale `--allow` than their silent absence. `schema` is grow-only: fields
/// get added, never removed or repurposed.
fn report_json(project: &Project, findings: &[Finding], allow: &[String], strict: bool) -> i32 {
    println!("{}", envelope(&project.root, findings, allow));
    let waived = findings.iter().filter(|f| allowed(f.code, allow)).count();
    // Same exit contract as the human report — a tool reading JSON still gets to fail a job.
    if findings.len() > waived && strict {
        crate::cli::ErrKind::Lint.exit_code()
    } else {
        0
    }
}

fn envelope(root: &Path, findings: &[Finding], allow: &[String]) -> serde_json::Value {
    use serde_json::json;
    let rows: Vec<serde_json::Value> = findings
        .iter()
        .map(|f| {
            let mut row = json!({
                "code": f.code,
                "severity": f.severity().as_str(),
                "message": f.message,
                "waived": allowed(f.code, allow),
            });
            let map = row.as_object_mut().expect("built as an object just above");
            if let Some(at) = &f.location {
                map.insert("file".into(), json!(at.file));
                map.insert("line".into(), json!(at.line));
                map.insert("column".into(), json!(at.column));
            }
            if let Some(fix) = &f.fix {
                map.insert(
                    "fix".into(),
                    json!({ "title": fix.title, "file": fix.file, "contents": fix.contents }),
                );
            }
            row
        })
        .collect();
    json!({
        "schema": 1,
        "project": root.to_string_lossy(),
        "findings": rows,
        "counts": {
            "errors": findings
                .iter()
                .filter(|f| !allowed(f.code, allow) && f.severity() == Severity::Error)
                .count(),
            "warnings": findings
                .iter()
                .filter(|f| !allowed(f.code, allow) && f.severity() == Severity::Warning)
                .count(),
            "waived": findings.iter().filter(|f| allowed(f.code, allow)).count(),
            "fixable": findings
                .iter()
                .filter(|f| f.fix.is_some() && !allowed(f.code, allow))
                .count(),
        },
    })
}

/// Append a markdown findings table to the job's run-summary page.
///
/// Three GitHub files look alike and do different things: `$GITHUB_OUTPUT` carries `name=value`
/// step outputs (annotation syntax written there is silently ignored), `$GITHUB_STEP_SUMMARY` is
/// the markdown the run page renders, and annotations come from `::warning::` commands on stdout
/// (above). Findings therefore go to the latter two — stdout for the highlighted PR/file
/// annotations, the summary file for the run page.
fn write_step_summary(active: &[&Finding], waived: &BTreeMap<&str, (usize, &str)>) {
    let Ok(path) = std::env::var("GITHUB_STEP_SUMMARY") else {
        return;
    };
    use std::fmt::Write as _;
    let mut md = String::from("## day lint\n\n");
    if active.is_empty() {
        md.push_str("✅ no findings");
    } else {
        let _ = writeln!(md, "⚠️ {} finding(s)\n", active.len());
        md.push_str("| code | finding |\n| --- | --- |\n");
        for f in active {
            let _ = writeln!(
                md,
                "| `{}` | {} |",
                f.code,
                f.message.replace('|', "\\|").replace('\n', "<br>")
            );
        }
    }
    for (code, (n, _)) in waived {
        let _ = writeln!(md, "\n_{n} `{code}` finding(s) waived by `--allow`_");
    }
    md.push('\n');
    // Appending, not truncating: earlier steps' summaries are theirs to keep. Best-effort — a
    // failed summary write must never fail the lint.
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
    {
        use std::io::Write as _;
        let _ = file.write_all(md.as_bytes());
    }
}

/// Validate one Fluent formatting-function call (docs/localization.md "Formatted values"):
/// day-l10n provides exactly `NUMBER()` and `DATETIME()`; unknown names render as error markers
/// at runtime, and a misspelled/invalid option silently falls back to defaults.
fn lint_ftl_call(locale: &str, file: &str, src: &str, call: &day_build::FtlCall) -> Vec<Finding> {
    let at = format!("resource/locales/{locale}: {}", call.key);
    let bad = |opt: &str, val: &str, expected: &str| Finding {
        code: "day::lint::bad-format-option",
        message: format!("{at}: {}({opt}: {val:?}) — expected {expected}", call.name),
        ..Default::default()
    };
    let mut out = Vec::new();
    match call.name.as_str() {
        "NUMBER" => {
            for (opt, val) in &call.named {
                match opt.as_str() {
                    "style" => match val.as_str() {
                        "decimal" | "percent" => {}
                        "currency" => out.push(Finding {
                            code: "day::lint::unsupported-format-option",
                            message: format!(
                                "{at}: NUMBER(style: \"currency\") is not supported yet — \
                                 it renders as a plain decimal"
                            ),
                            ..Default::default()
                        }),
                        other => out.push(bad("style", other, "\"decimal\" or \"percent\"")),
                    },
                    "useGrouping" => {
                        if !matches!(val.as_str(), "true" | "false") {
                            out.push(bad("useGrouping", val, "\"true\" or \"false\""));
                        }
                    }
                    // Plural-category selection type — handled by fluent-bundle itself.
                    "type" => {}
                    "currency" | "currencyDisplay" => out.push(Finding {
                        code: "day::lint::unsupported-format-option",
                        message: format!("{at}: NUMBER {opt} is not supported yet"),
                        ..Default::default()
                    }),
                    "minimumIntegerDigits"
                    | "minimumFractionDigits"
                    | "maximumFractionDigits"
                    | "minimumSignificantDigits"
                    | "maximumSignificantDigits" => {
                        if val.parse::<u32>().is_err() {
                            out.push(bad(opt, val, "a digit count"));
                        }
                    }
                    other => out.push(bad(other, val, "a NUMBER option (ECMA-402 names)")),
                }
            }
        }
        "DATETIME" => {
            for (opt, val) in &call.named {
                match opt.as_str() {
                    "dateStyle" | "timeStyle" => {
                        if !matches!(val.as_str(), "full" | "long" | "medium" | "short" | "none") {
                            out.push(bad(opt, val, "full|long|medium|short|none"));
                        }
                    }
                    other => out.push(bad(other, val, "dateStyle or timeStyle")),
                }
            }
        }
        other => out.push(Finding {
            code: "day::lint::unknown-function",
            message: format!(
                "{at}: unknown function {other}() — day provides NUMBER() and DATETIME()"
            ),
            ..Default::default()
        }),
    }
    // Every one of these is about the same call, so the position is attached once here rather than
    // repeated at each push above.
    let at = Location::in_file(file, src, call.offset);
    out.into_iter().map(|f| f.located(at.clone())).collect()
}

fn finish(n: usize, waived: usize, strict: bool) -> i32 {
    let waived_note = match waived {
        0 => String::new(),
        w => format!(" ({w} allowed)"),
    };
    if n == 0 {
        eprintln!("{SUCCESS}✓{SUCCESS:#} no lint findings{waived_note}");
        0
    } else {
        eprintln!("{n} finding(s){waived_note}");
        // The findings above are the report; --strict turns them into the lint exit code
        // (the kind→code map in cli.rs is the one place that number lives).
        if strict {
            crate::cli::ErrKind::Lint.exit_code()
        } else {
            0
        }
    }
}

/// The `resource/vectors/` checks (docs/vectors.md).
fn lint_vectors(project: &Project, findings: &mut Vec<Finding>) {
    let dir = project.root.join("resource/vectors");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut paths: Vec<std::path::PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let fname = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if fname.starts_with('.') {
            continue;
        }
        let svg_path = if path.is_file() && fname.to_ascii_lowercase().ends_with(".svg") {
            path.clone()
        } else if path.is_dir() && fname.to_ascii_lowercase().ends_with(".symbolset") {
            match std::fs::read_dir(&path).ok().and_then(|d| {
                d.flatten()
                    .map(|e| e.path())
                    .find(|p| p.extension().and_then(|x| x.to_str()) == Some("svg"))
            }) {
                Some(inner) => inner,
                None => {
                    findings.push(Finding {
                        code: "day::lint::vector-empty-symbolset",
                        message: format!("resource/vectors/{fname}: no inner .svg in the bundle"),
                        ..Default::default()
                    });
                    continue;
                }
            }
        } else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&svg_path) else {
            findings.push(Finding {
                code: "day::lint::vector-unreadable",
                message: format!("resource/vectors/{fname}: unreadable"),
                ..Default::default()
            });
            continue;
        };
        let template = day_vector::classify(&text) == day_vector::SourceKind::SfTemplate;
        let glyph = if template {
            match day_vector::extract_variant(&text, "Regular", "M") {
                Ok(g) => g,
                Err(e) => {
                    findings.push(Finding {
                        code: "day::lint::vector-template",
                        message: format!("resource/vectors/{fname}: {e}"),
                        ..Default::default()
                    });
                    continue;
                }
            }
        } else {
            text
        };
        if glyph.contains("<text") {
            findings.push(Finding {
                code: "day::lint::vector-text",
                message: format!(
                    "resource/vectors/{fname}: glyph contains <text> — outline it (docs/vectors.md)"
                ),
                ..Default::default()
            });
            continue;
        }
        match day_vector::parse(glyph.as_bytes()) {
            Err(e) => findings.push(Finding {
                code: "day::lint::vector-parse",
                message: format!("resource/vectors/{fname}: {e}"),
                ..Default::default()
            }),
            Ok(tree) => {
                if project
                    .manifest
                    .app
                    .targets
                    .iter()
                    .any(|t| t == "android-mdc")
                    && let Err(why) = day_vector::to_vector_drawable(&tree)
                {
                    findings.push(Finding {
                        code: "day::lint::vector-raster-fallback",
                        message: format!(
                            "resource/vectors/{fname}: {why} is outside the VectorDrawable \
                             subset — Android ships a raster fallback"
                        ),
                        ..Default::default()
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ftl_function_lint() {
        const SRC: &str = r#"
a = { NUMBER($n, style: "percent", minimumFractionDigits: 2) }
b = { NUMBER($n, style: "currency", currency: "USD") }
c = { NUMBER($n, stlye: "percent") }
d = { DATETIME($d, dateStyle: "extra-long") }
e = { PLATFORM() }
"#;
        let calls = day_build::function_calls(SRC);
        let src = SRC;
        let findings: Vec<Finding> = calls
            .iter()
            .flat_map(|c| lint_ftl_call("en", "resource/locales/en/app.ftl", src, c))
            .collect();
        let codes: Vec<&str> = findings.iter().map(|f| f.code).collect();
        assert_eq!(
            codes,
            [
                "day::lint::unsupported-format-option", // b: style currency
                "day::lint::unsupported-format-option", // b: currency:
                "day::lint::bad-format-option",         // c: stlye typo
                "day::lint::bad-format-option",         // d: dateStyle value
                "day::lint::unknown-function",          // e
            ],
            "{findings:?}"
        );
        // Each finding points at the LINE its call is on, not at the top of the catalog: the
        // source above starts with a newline, so `b` is line 3 and `e` is line 6.
        let lines: Vec<usize> = findings
            .iter()
            .map(|f| f.location.as_ref().expect("every call has a place").line)
            .collect();
        assert_eq!(lines, [3, 3, 4, 5, 6], "{findings:?}");
    }

    #[test]
    fn the_envelope_carries_place_fix_and_waiver() {
        let findings = vec![
            Finding {
                code: "day::lint::unknown-target",
                message: "not a target".into(),
                // `targets` starts at byte 8 of this fixture, which is line 3.
                location: Some(Location::in_file(
                    "Day.toml",
                    "[app]\nid = \"x\"\ntargets = [\"atari-tos\"]",
                    "[app]\nid = \"x\"\n".len(),
                )),
                fix: None,
            },
            Finding {
                code: "day::lint::store-whitespace",
                message: "trailing space".into(),
                location: Some(Location::head("store/en/name.txt")),
                fix: Some(Fix {
                    title: "Trim the surrounding whitespace".into(),
                    file: "store/en/name.txt".into(),
                    contents: "Name\n".into(),
                }),
            },
            Finding {
                code: "day::lint::store-placeholder",
                message: "still TODO".into(),
                ..Default::default()
            },
        ];
        let allow = vec!["store-placeholder".into()];
        let doc = envelope(Path::new("/app"), &findings, &allow);
        let rows = doc["findings"].as_array().expect("findings is an array");

        // An unknown target is an ERROR and points at the line the parser could not report.
        assert_eq!(rows[0]["severity"], "error");
        assert_eq!(rows[0]["line"], 3);
        assert_eq!(rows[0]["column"], 1);
        assert_eq!(rows[0]["waived"], false);
        assert!(rows[0].get("fix").is_none());

        // A repair travels with the finding, so an editor offers it without re-deriving anything.
        assert_eq!(rows[1]["fix"]["contents"], "Name\n");
        assert_eq!(rows[1]["severity"], "warning");

        // A waived finding is REPORTED and flagged, not dropped: a stale `--allow` is easier to
        // notice as a greyed row than as an absence.
        assert_eq!(rows[2]["waived"], true);
        assert!(rows[2].get("file").is_none(), "nothing to point at");

        assert_eq!(doc["counts"]["errors"], 1);
        assert_eq!(doc["counts"]["warnings"], 1);
        assert_eq!(doc["counts"]["waived"], 1);
        assert_eq!(doc["counts"]["fixable"], 1);
    }

    #[test]
    fn a_waived_finding_is_never_rewritten() {
        // `--allow` says the finding may stand. Applying its fix anyway would be the opposite of
        // standing, and would edit a file the author deliberately left alone.
        let f = Finding {
            code: "day::lint::store-whitespace",
            message: "trailing space".into(),
            fix: Some(Fix {
                title: "Trim".into(),
                file: "store/en/name.txt".into(),
                contents: "Name\n".into(),
            }),
            ..Default::default()
        };
        let doc = envelope(Path::new("/app"), &[f], &["store-whitespace".to_string()]);
        assert_eq!(doc["counts"]["fixable"], 0);
    }

    #[test]
    fn severity_is_reserved_for_findings_about_something_that_does_not_exist() {
        assert_eq!(severity_of("day::lint::unknown-key"), Severity::Error);
        assert_eq!(severity_of("day::lint::unknown-route"), Severity::Error);
        // Coverage and store copy are worth reporting and are not broken references.
        assert_eq!(
            severity_of("day::lint::missing-translation"),
            Severity::Warning
        );
        assert_eq!(
            severity_of("day::lint::store-placeholder"),
            Severity::Warning
        );
        assert_eq!(
            severity_of("day::lint::whatever-comes-next"),
            Severity::Warning
        );
    }

    #[test]
    fn allow_matches_bare_and_qualified_codes() {
        let allow = vec![
            "store-placeholder".into(),
            " day::lint::duplicate-id ".into(),
        ];
        assert!(allowed("day::lint::store-placeholder", &allow));
        assert!(allowed("day::lint::duplicate-id", &allow));
        // A prefix is not a code: allowing `store-placeholder` must not waive `store-missing`,
        // and the bare form must not match some other namespace's same-named finding.
        assert!(!allowed("day::lint::store-missing", &allow));
        assert!(!allowed("day::store::store-placeholder", &allow));
        assert!(!allowed("day::lint::store-placeholder", &[]));
    }

    #[test]
    fn first_segment_extraction() {
        assert_eq!(route_first_segment("stack/item-42?hint=x"), "stack");
        assert_eq!(route_first_segment("controls"), "controls");
        assert_eq!(route_first_segment("a?x=1"), "a");
        assert_eq!(route_first_segment(""), "");
    }

    #[test]
    fn routes_macro_key_extraction() {
        let dir = std::env::temp_dir().join(format!("day-lint-routes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("lib.rs"),
            "day::routes! {\n    pub(crate) enum Section { Home => \"home\", Stack => \"stack\" }\n}\nfn f() { let x = match y { A => \"not-a-key\" }; }\n",
        )
        .unwrap();
        let mut out = Vec::new();
        scan_routes_macro_keys(&dir, &mut out);
        let mut keys: Vec<String> = out.iter().map(|h| h.text.clone()).collect();
        keys.sort();
        assert_eq!(keys, ["home", "stack"]);
        // Both keys are on the enum's line, which is line 2 of the file written above.
        assert!(out.iter().all(|h| h.line == 2), "{out:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn script_route_extraction() {
        let dir = std::env::temp_dir().join(format!("day-lint-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("walk.yaml"),
            "flow:\n  - navigate: { route: controls }\n  - assert_route: { route: \"stack/1\" }\n  - tap: { id: x }\n  - navigate: { route: 'tabs' }\n",
        )
        .unwrap();
        let mut out = Vec::new();
        scan_script_routes(&dir, &mut out);
        let mut routes: Vec<String> = out.iter().map(|h| h.text.clone()).collect();
        routes.sort();
        assert_eq!(routes, ["controls", "stack/1", "tabs"]);
        // The steps are on lines 2, 3 and 5 of the script — a finding about one of them opens
        // the file there rather than at the top.
        let mut lines: Vec<usize> = out.iter().map(|h| h.line).collect();
        lines.sort();
        assert_eq!(lines, [2, 3, 5], "{out:?}");
        // A step carrying a filter is still a route of "webview": the value ends at the next key
        // in the inline map. Reading to the closing brace made every filtered step a finding.
        let f = dir.join("filtered.yaml");
        std::fs::write(
            &f,
            "flow:\n  - navigate: { route: webview, skip_on: [harmony-arkui] }\n",
        )
        .expect("write");
        let mut hits = Vec::new();
        scan_script_routes(&dir, &mut hits);
        let routes: Vec<&str> = hits.iter().map(|h| h.text.as_str()).collect();
        assert!(routes.contains(&"webview"), "{routes:?}");
        assert!(
            !routes.iter().any(|r| r.contains("skip_on")),
            "the filter is not part of the route: {routes:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
