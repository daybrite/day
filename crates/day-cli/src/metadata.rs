//! `day metadata` — the machine-readable project interface (docs/cli.md).
//!
//! IDE tooling (day-vscode) shells out to `day metadata --json` instead of parsing Day.toml
//! itself, so the manifest format can evolve without breaking editors — and the target
//! catalog travels with the CLI instead of being hand-mirrored in each tool. The JSON
//! envelope is VERSIONED and grow-only: add keys freely, never repurpose existing ones.

use crate::meta::Project;
use crate::targets::{self, TargetKind};

fn kind_str(k: TargetKind) -> &'static str {
    match k {
        TargetKind::Desktop => "desktop",
        TargetKind::IosSim => "iosSim",
        TargetKind::Android => "android",
        TargetKind::HarmonyOs => "harmonyOs",
    }
}

fn host_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "linux" => "linux",
        "windows" => "windows",
        other => {
            // Unknown hosts still get a truthful value (tooling dims what it can't build).
            let _ = other;
            "other"
        }
    }
}

pub fn run(project: &Project, json: bool) -> i32 {
    let m = &project.manifest;
    let catalog: Vec<serde_json::Value> = targets::TARGETS
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "toolkit": t.toolkit,
                "kind": kind_str(t.kind),
                "host": t.host,
                "label": t.label,
                "experimental": t.experimental,
            })
        })
        .collect();
    // Per-target identity AFTER [app.<key>] overrides — what each target actually builds with.
    let resolved: serde_json::Map<String, serde_json::Value> = m
        .app
        .targets
        .iter()
        .map(|t| {
            (
                t.clone(),
                serde_json::to_value(m.resolve(t)).unwrap_or_default(),
            )
        })
        .collect();
    let doc = serde_json::json!({
        "schema": 1,
        "project": {
            "root": project.root,
            "name": m.app.name,
            "version": m.app.version,
            "id": m.app.id,
            "title": m.app.title.clone().unwrap_or_else(|| m.app.name.clone()),
            "build": m.app.build,
            "targets": m.app.targets,
            "window": m.window,
            "resolved": resolved,
        },
        "host": { "os": host_os() },
        "targetCatalog": catalog,
    });
    if json {
        match serde_json::to_string_pretty(&doc) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
        return 0;
    }
    // Human-readable summary (the JSON envelope is the stable interface).
    let title = m.app.title.as_deref().unwrap_or(&m.app.name);
    println!("{} ({})", title, m.app.id);
    println!("  name     {}", m.app.name);
    println!("  version  {} (build {})", m.app.version, m.app.build);
    println!("  root     {}", project.root.display());
    println!("  targets  {}", m.app.targets.join(", "));
    for t in &m.app.targets {
        let r = m.resolve(t);
        if r.id != m.app.id || Some(r.title.as_str()) != Some(title) || r.build != m.app.build {
            println!(
                "           {t}: id={} title={:?} build={}",
                r.id, r.title, r.build
            );
        }
    }
    0
}
