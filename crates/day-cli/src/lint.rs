// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day lint v0 (DESIGN.md §16.5): fluent coverage (missing/unused/unknown keys), duplicate
//! element ids, unknown navigation routes, Day.toml schema (validated by parsing). Fast —
//! sources + locales + scripts only.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::meta::Project;
use crate::ops::{gha_escape, github_actions};
use crate::term::{DIM, SUCCESS, WARN};
use anstream::eprintln;

#[derive(Debug)]
pub struct Finding {
    pub code: &'static str,
    pub message: String,
}

/// Collect keys referenced via the generated `res::str::<key>(…)` functions (§18.5). Unlike
/// `tr("key")` these aren't quote-delimited: after `res::str::` (possibly through a `crate::`/module
/// path) read the Rust identifier, stripping a `r#` raw prefix — that identifier is the Fluent key.
fn scan_res_str(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            scan_res_str(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs")
            && let Ok(src) = std::fs::read_to_string(&p)
        {
            let pat = "res::str::";
            let mut rest = src.as_str();
            while let Some(i) = rest.find(pat) {
                rest = &rest[i + pat.len()..];
                let s = rest.strip_prefix("r#").unwrap_or(rest);
                let end = s
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(s.len());
                if end > 0 {
                    out.push(s[..end].to_string());
                }
            }
        }
    }
}

/// Collect portable permissions referenced in code as `Permission::<Variant>` (docs/permissions.md).
///
/// Reads an identifier rather than a quoted literal, the same shape as [`scan_res_str`]. The
/// contract with `day-part-permissions` is that its enum is called `Permission` and its variants are
/// the table's `variant` spellings — pinned by `tests/permissions_parity.rs`.
fn scan_permission_uses(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            scan_permission_uses(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs")
            && let Ok(src) = std::fs::read_to_string(&p)
        {
            let pat = "Permission::";
            let mut rest = src.as_str();
            while let Some(i) = rest.find(pat) {
                rest = &rest[i + pat.len()..];
                let end = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len());
                if end > 0 {
                    out.push(rest[..end].to_string());
                }
            }
        }
    }
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

fn scan_sources(dir: &Path, pat: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            scan_sources(&p, pat, out);
        } else if p.extension().is_some_and(|x| x == "rs")
            && let Ok(src) = std::fs::read_to_string(&p)
        {
            let mut rest = src.as_str();
            while let Some(i) = rest.find(pat) {
                rest = &rest[i + pat.len()..];
                if let Some(end) = rest.find('"') {
                    out.push(rest[..end].to_string());
                    rest = &rest[end..];
                }
            }
        }
    }
}

/// The first path segment of a route string (`"a/b?x=1"` → `"a"`) — the part a lint can check
/// against declared selector/tabs item keys. Deeper segments are open-ended (stack destination
/// builders accept any key), so only the first is validated.
fn route_first_segment(route: &str) -> &str {
    route.split(['/', '?']).next().unwrap_or("")
}

/// Collect the `Variant => "key"` literals declared inside `routes! { … }` blocks — typed
/// selectors declare their keys there instead of at `.item("key", …)` call sites.
fn scan_routes_macro_keys(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            scan_routes_macro_keys(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs")
            && let Ok(src) = std::fs::read_to_string(&p)
        {
            let mut rest = src.as_str();
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
                        out.push(body[..q].to_string());
                        body = &body[q..];
                    }
                }
                rest = &rest[end..];
            }
        }
    }
}

/// Collect `route:` values from dayscript `navigate:` / `assert_route:` steps — and the
/// route inside every `deep_link:` step's `url:` (docs/deep-links.md) — in
/// `dayscript/*.yaml`: the same route namespace `navigate()` uses (docs/navigation.md).
fn scan_script_routes(dir: &Path, out: &mut Vec<String>) {
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
                            out.push(route.to_string());
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
                        out.push(v.to_string());
                    }
                }
            }
        }
    }
}

/// Does `--allow CODE` cover this finding? The `day::lint::` prefix is optional, so
/// `--allow store-placeholder` and `--allow day::lint::store-placeholder` name the same one.
fn allowed(code: &str, allow: &[String]) -> bool {
    allow.iter().any(|a| {
        let a = a.trim();
        code == a || code.strip_prefix("day::lint::") == Some(a)
    })
}

pub fn run(project: &Project, strict: bool, allow: &[String]) -> i32 {
    let mut findings: Vec<Finding> = Vec::new();

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
            findings.push(Finding {
                code: "day::lint::unknown-target",
                message: format!("Day.toml: targets entry {t:?} is not a known target"),
            });
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
                findings.push(Finding {
                    code: "day::lint::unknown-override",
                    message: format!(
                        "Day.toml: [app.{key}] does not name a known platform, toolkit, or \
                         target"
                    ),
                });
            }
        }
    }

    // --- Store listings (§16.6) ---
    // Held to the stores' own rules, because the alternative is learning them from a rejection
    // days after the upload. Silent for an app that ships to neither store.
    match crate::store::read(project) {
        Ok(listing) => {
            for p in crate::store::lint(project, &listing) {
                findings.push(Finding {
                    code: p.code,
                    message: p.message,
                });
            }
        }
        Err(e) => findings.push(Finding {
            code: "day::lint::store-unreadable",
            message: e,
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
            });
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
        used.sort();
        used.dedup();
        let declared = &project.manifest.permissions.declared;
        for variant in &used {
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
                }),
                Some(decl) if !decl.enabled() => findings.push(Finding {
                    code: "day::lint::undeclared-permission",
                    message: format!(
                        "code requests Permission::{variant}, but Day.toml declares {:?} = false",
                        spec.name
                    ),
                }),
                Some(decl) if spec.needs_reason && decl.reason_for("ios").is_none() => findings
                    .push(Finding {
                        code: "day::lint::missing-reason",
                        message: format!(
                            "[permissions] {:?} has no reason — it is the text iOS and HarmonyOS \
                             show the user when they prompt",
                            spec.name
                        ),
                    }),
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
                });
            }
        }
    }

    // --- Fluent coverage ---
    let locales_dir = project.root.join("resource/locales");
    let mut locales: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(&locales_dir) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                let name = e.file_name().to_string_lossy().to_string();
                let mut keys = BTreeSet::new();
                if let Ok(files) = std::fs::read_dir(e.path()) {
                    for f in files.flatten() {
                        if f.path().extension().is_some_and(|x| x == "ftl")
                            && let Ok(src) = std::fs::read_to_string(f.path())
                        {
                            keys.extend(day_build::message_keys(&src));
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
    let used: BTreeSet<String> = used_keys.into_iter().collect();

    // Default = "en" if present, else first.
    let default_name = if locales.contains_key("en") {
        "en".to_string()
    } else {
        locales.keys().next().cloned().unwrap_or_default()
    };
    if let Some(default_keys) = locales.get(&default_name).cloned() {
        for k in &used {
            if !default_keys.contains(k) {
                findings.push(Finding {
                    code: "day::lint::unknown-key",
                    message: format!("tr({k:?}) has no message in resource/locales/{default_name}"),
                });
            }
        }
        for k in &default_keys {
            // Convention keys the framework consumes at build time, not from app source:
            // `language_name` is read by day-build's generated `res::locales::ALL` (each catalog
            // naming its own language for pickers — docs/localization.md), so no `res::str::` or
            // `tr("…")` reference exists for the scan to find.
            if k == "language_name" {
                continue;
            }
            if !used.contains(k) {
                findings.push(Finding {
                    code: "day::lint::unused-key",
                    message: format!("resource/locales/{default_name}: {k} is never referenced"),
                });
            }
        }
        for (name, keys) in &locales {
            if name == &default_name {
                continue;
            }
            for k in &default_keys {
                if !keys.contains(k) {
                    findings.push(Finding {
                        code: "day::lint::missing-translation",
                        message: format!("resource/locales/{name}: missing {k}"),
                    });
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
                for call in day_build::function_calls(&src) {
                    findings.extend(lint_ftl_call(&locale, &call));
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
        let declared: BTreeSet<String> = declared_keys.into_iter().collect();
        let mut used_routes: Vec<(String, String)> = Vec::new();
        let mut nav_calls = Vec::new();
        for r in &roots {
            scan_sources(r, "navigate(\"", &mut nav_calls);
        }
        used_routes.extend(nav_calls.into_iter().map(|r| ("navigate".to_string(), r)));
        let mut script_routes = Vec::new();
        scan_script_routes(&project.root.join("dayscript"), &mut script_routes);
        used_routes.extend(
            script_routes
                .into_iter()
                .map(|r| ("dayscript".to_string(), r)),
        );
        // [[shortcuts]] routes are saved deep links (docs/deep-links.md) — same check,
        // query params stripped the way the route parser will strip them.
        used_routes.extend(project.manifest.shortcuts.iter().map(|s| {
            let route = s.route.split('?').next().unwrap_or(&s.route).to_string();
            ("Day.toml [[shortcuts]]".to_string(), route)
        }));
        for (origin, route) in &used_routes {
            let first = route_first_segment(route);
            if !first.is_empty() && !declared.contains(first) {
                findings.push(Finding {
                    code: "day::lint::unknown-route",
                    message: format!(
                        "{origin}: route {route:?} starts with {first:?}, which no `.item(…)` \
                         or `routes! {{ … }}` declares"
                    ),
                });
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
            }),
            Ok(_) => {}
            Err(e) => findings.push(Finding {
                code: "day::lint::shortcut-label",
                message: e,
            }),
        }
    }

    // --- Duplicate ids ---
    let mut ids = Vec::new();
    for r in &roots {
        scan_sources(r, ".id(\"", &mut ids);
    }
    let mut seen = BTreeSet::new();
    for id in &ids {
        if !seen.insert(id.clone()) {
            findings.push(Finding {
                code: "day::lint::duplicate-id",
                message: format!("element id {id:?} used more than once"),
            });
        }
    }

    // An allowed code still reports, one summary line per code rather than per finding: a
    // scaffold's 84 store placeholders would otherwise bury the warnings that do matter. The
    // count and a sample are enough to see what a stale `--allow` is covering.
    let mut waived: BTreeMap<&str, (usize, &str)> = BTreeMap::new();
    let gha = github_actions();
    let mut active: Vec<&Finding> = Vec::new();
    for f in &findings {
        if allowed(f.code, allow) {
            let e = waived.entry(f.code).or_insert((0, f.message.as_str()));
            e.0 += 1;
        } else {
            eprintln!("{WARN}warning{WARN:#} {:<32} {}", f.code, f.message);
            if gha {
                // GitHub reads workflow commands off STDOUT (the human report above is stderr,
                // which never becomes an annotation). `title` carries the finding code so the
                // annotation list groups legibly. Newlines must be %0A-escaped per the docs.
                println!(
                    "::warning title=day lint {}::{}",
                    f.code,
                    gha_escape(&f.message)
                );
            }
            active.push(f);
        }
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
fn lint_ftl_call(locale: &str, call: &day_build::FtlCall) -> Vec<Finding> {
    let at = format!("resource/locales/{locale}: {}", call.key);
    let bad = |opt: &str, val: &str, expected: &str| Finding {
        code: "day::lint::bad-format-option",
        message: format!("{at}: {}({opt}: {val:?}) — expected {expected}", call.name),
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
        }),
    }
    out
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
        if strict { 10 } else { 0 }
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
            });
            continue;
        }
        match day_vector::parse(glyph.as_bytes()) {
            Err(e) => findings.push(Finding {
                code: "day::lint::vector-parse",
                message: format!("resource/vectors/{fname}: {e}"),
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
        let calls = day_build::function_calls(
            r#"
a = { NUMBER($n, style: "percent", minimumFractionDigits: 2) }
b = { NUMBER($n, style: "currency", currency: "USD") }
c = { NUMBER($n, stlye: "percent") }
d = { DATETIME($d, dateStyle: "extra-long") }
e = { PLATFORM() }
"#,
        );
        let findings: Vec<Finding> = calls.iter().flat_map(|c| lint_ftl_call("en", c)).collect();
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
        out.sort();
        assert_eq!(out, ["home", "stack"]);
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
        out.sort();
        assert_eq!(out, ["controls", "stack/1", "tabs"]);
        // A step carrying a filter is still a route of "webview": the value ends at the next key
        // in the inline map. Reading to the closing brace made every filtered step a finding.
        let f = dir.join("filtered.yaml");
        std::fs::write(
            &f,
            "flow:\n  - navigate: { route: webview, skip_on: [harmony-arkui] }\n",
        )
        .expect("write");
        let mut routes = Vec::new();
        scan_script_routes(&dir, &mut routes);
        assert!(routes.contains(&"webview".to_string()), "{routes:?}");
        assert!(
            !routes.iter().any(|r| r.contains("skip_on")),
            "the filter is not part of the route: {routes:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
