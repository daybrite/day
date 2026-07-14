//! day lint v0 (DESIGN.md §16.5): fluent coverage (missing/unused/unknown keys), duplicate
//! element ids, unknown navigation routes, Day.toml schema (validated by parsing). Fast —
//! sources + locales + scripts only.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::meta::Project;
use crate::term::{SUCCESS, WARN};
use anstream::eprintln;

pub struct Finding {
    pub code: &'static str,
    pub message: String,
}

fn ftl_keys(src: &str) -> BTreeSet<String> {
    src.lines()
        .filter_map(|l| {
            let l = l.trim_start();
            if l.starts_with('#') {
                return None;
            }
            let (k, _) = l.split_once('=')?;
            let k = k.trim();
            if !k.is_empty()
                && k.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                && !k.starts_with('-')
            {
                Some(k.to_string())
            } else {
                None
            }
        })
        .collect()
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
                            keys.extend(ftl_keys(&src));
                        }
                    }
                }
                locales.insert(name, keys);
            }
        }
    }
    let mut used_keys = Vec::new();
    scan_sources(&project.root.join("src"), "tr(\"", &mut used_keys);
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

    // --- Unknown routes (docs/navigation.md) ---
    // Literal `navigate("…")` calls and dayscript navigate / assert_route steps must START
    // with a declared item key — `.item("key", …)` for string-keyed apps, `routes! { X =>
    // "key" }` for typed ones (typed `.item(Section::X, …)` call sites are already
    // compile-checked; this covers the scripts and raw strings). Skipped when the app
    // declares no keys either way (a pure-stack app's routes are open-ended).
    let mut declared_keys = Vec::new();
    scan_sources(&project.root.join("src"), ".item(\"", &mut declared_keys);
    scan_routes_macro_keys(&project.root.join("src"), &mut declared_keys);
    if !declared_keys.is_empty() {
        let declared: BTreeSet<String> = declared_keys.into_iter().collect();
        let mut used_routes: Vec<(String, String)> = Vec::new();
        let mut nav_calls = Vec::new();
        scan_sources(&project.root.join("src"), "navigate(\"", &mut nav_calls);
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
    scan_sources(&project.root.join("src"), ".id(\"", &mut ids);
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
    let n = findings.len();
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
