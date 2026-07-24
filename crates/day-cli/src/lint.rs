//! day lint v0 (DESIGN.md §16.5): fluent coverage (missing/unused/unknown keys), duplicate
//! element ids, unknown navigation routes, Day.toml schema (validated by parsing). Fast —
//! sources + locales + scripts only.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::meta::Project;
use crate::term::{SUCCESS, WARN};
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
                "target" | "build" | "platform" | "resource" | "dayscript" | ".git"
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

/// Collect `route:` values from dayscript `navigate:` / `assert_route:` steps in
/// `dayscript/*.yaml` — the same route namespace `navigate()` uses (docs/navigation.md).
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
                if !(l.starts_with("- navigate:") || l.starts_with("- assert_route:")) {
                    continue;
                }
                // rfind: `assert_route:` itself contains "route:" — the value's key is last.
                if let Some(i) = l.rfind("route:") {
                    let v = l[i + "route:".len()..]
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

pub fn run(project: &Project, strict: bool) -> i32 {
    let mut findings: Vec<Finding> = Vec::new();

    // --- Day.toml structure ---
    // Syntax + schema are enforced at load (a project that reaches here parsed); lint adds the
    // semantic checks: every [app] target is a known combo, and every [app.<key>] override
    // table names a known platform, toolkit, or target.
    for t in &project.manifest.app.targets {
        if crate::targets::find(t).is_none() {
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
            if let Some(platform) = t.name.split('-').next() {
                known.insert(platform); // "macos"
            }
        }
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
                if !f.path().extension().is_some_and(|x| x == "ftl") {
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

    for f in &findings {
        eprintln!("{WARN}warning{WARN:#} {:<32} {}", f.code, f.message);
    }
    finish(findings.len(), strict)
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

fn finish(n: usize, strict: bool) -> i32 {
    if n == 0 {
        eprintln!("{SUCCESS}✓{SUCCESS:#} no lint findings");
        0
    } else {
        eprintln!("{n} finding(s)");
        if strict { 10 } else { 0 }
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
        std::fs::remove_dir_all(&dir).ok();
    }
}
