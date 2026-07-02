//! day lint v0 (DESIGN.md §16.5): fluent coverage (missing/unused/unknown keys), duplicate
//! element ids, day.yaml schema (validated by parsing). Fast — sources + locales only.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::meta::Project;

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

pub fn run(project: &Project, strict: bool) -> i32 {
    let mut findings: Vec<Finding> = Vec::new();

    // --- Fluent coverage ---
    let locales_dir = project.root.join("locales");
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
                    message: format!("tr({k:?}) has no message in locales/{default_name}"),
                });
            }
        }
        for k in &default_keys {
            if !used.contains(k) {
                findings.push(Finding {
                    code: "day::lint::unused-key",
                    message: format!("locales/{default_name}: {k} is never referenced"),
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
                        message: format!("locales/{name}: missing {k}"),
                    });
                }
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
        eprintln!("\x1b[33mwarning\x1b[0m {:<32} {}", f.code, f.message);
    }
    let n = findings.len();
    if n == 0 {
        eprintln!("\x1b[32m✓\x1b[0m no lint findings");
        0
    } else {
        eprintln!("{n} finding(s)");
        if strict { 10 } else { 0 }
    }
}
