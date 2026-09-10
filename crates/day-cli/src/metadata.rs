// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

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
        TargetKind::Web => "web",
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

pub fn run(project: &Project, json: bool) -> Result<(), crate::cli::CliError> {
    let m = &project.manifest;
    let mut catalog: Vec<serde_json::Value> = targets::TARGETS
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
    // Externally declared targets (docs/extending.md) ride the same catalog with two extra
    // fields — `external` and the declaring `crate` — so tooling that groups or filters can
    // tell them apart. Grow-only, per the envelope's contract. A discovery failure degrades to
    // the builtin catalog with a warning: metadata is read by editors, which must keep working
    // while a Cargo.toml is mid-edit.
    match crate::external::resolve(project) {
        Ok(ext) => {
            for t in ext {
                catalog.push(serde_json::json!({
                    "name": t.target.name,
                    "toolkit": t.target.toolkit,
                    "kind": kind_str(t.target.kind),
                    "host": t.target.host,
                    "label": t.target.label,
                    "experimental": true,
                    "external": true,
                    "crate": t.crate_name,
                }));
            }
        }
        Err(e) => eprintln!("warning: external toolkit discovery failed: {e}"),
    }
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
            // The packaged-artifact filename stem, before any `[app.<target>]` override (those
            // are under `resolved`). Release CI reads it to name the web-dom zip, which is the
            // one shipped artifact no `day pack` produces.
            "artifact": crate::meta::slug(m.app.artifact.as_deref().unwrap_or_else(|| {
                m.app.title.as_deref().unwrap_or(&m.app.name)
            })),
            "build": m.app.build,
            "targets": m.app.targets,
            "window": m.window,
            // Where the app is listed (docs/store.md "Listed apps"): ids plus the listing URLs
            // they resolve to, so a consumer need not know each store's URL shape.
            "store": {
                "apple-app-id": m.store.apple_app_id,
                "google-play-id": m.store.google_play_id,
                "apple-url": m.store.apple_url(),
                "google-url": m.store.google_url(),
            },
            // The home-screen presentation of the web build (docs/web.md).
            "web": m.web,
            "resolved": resolved,
            "permissions": declared_permissions(project),
            // `[permissions.raw]`, with the same per-locale reasons (docs/permissions.md).
            "rawPermissions": raw_permissions(project),
        },
        "host": { "os": host_os() },
        "targetCatalog": catalog,
        "permissionCatalog": permission_catalog(),
    });
    if json {
        let s = serde_json::to_string_pretty(&doc)
            .map_err(|e| crate::cli::CliError::failure(e.to_string()))?;
        println!("{s}");
        return Ok(());
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
    Ok(())
}

/// The app's declared permissions, resolved from Day.toml alone — no `cargo metadata`, so
/// `day metadata` stays as fast as it has always been. Library contributions are therefore NOT
/// included here; `day build` unions them at build time (docs/permissions.md).
fn declared_permissions(project: &Project) -> Vec<serde_json::Value> {
    let catalog = crate::permissions::Catalog::load(&project.root);
    project
        .manifest
        .permissions
        .declared
        .iter()
        .filter(|(_, d)| d.enabled())
        .filter_map(|(name, decl)| {
            let spec = day_build::permissions::find(name)?;
            // The reason in every locale (docs/permissions.md, "Localized reasons"): the
            // catalog's `permission_<id>` message, Day.toml's inline text for the default.
            let mut reasons = catalog_texts(&catalog, &crate::permissions::message_id(spec.name));
            if let Some(inline) = decl.reason_for("ios")
                && !reasons.contains_key(&catalog.default_locale)
            {
                reasons.insert(catalog.default_locale.clone(), inline.to_string());
            }
            Some(serde_json::json!({
                "name": spec.name,
                "variant": spec.variant,
                "reason": reasons.get(&catalog.default_locale),
                "reasons": reasons,
                "android": spec.android.iter().map(|p| p.name).collect::<Vec<_>>(),
                "ios": spec.ios,
                "macos": spec.macos,
                "ohos": spec.ohos.iter().map(|p| p.name).collect::<Vec<_>>(),
            }))
        })
        .collect()
}

/// `id` in every locale the catalog has it: locale → text.
fn catalog_texts(
    catalog: &crate::permissions::Catalog,
    id: &str,
) -> std::collections::BTreeMap<String, String> {
    catalog
        .locales()
        .filter_map(|l| catalog.text(l, id).map(|t| (l.clone(), t.to_string())))
        .collect()
}

/// `[permissions.raw]`: the Android names, the Apple keys with their reasons per locale, and
/// the HarmonyOS entries with theirs.
fn raw_permissions(project: &Project) -> serde_json::Value {
    let catalog = crate::permissions::Catalog::load(&project.root);
    let raw = &project.manifest.permissions.raw;
    let with_reasons = |id: &str, inline: Option<&str>| {
        let mut reasons = catalog_texts(&catalog, id);
        if let Some(inline) = inline
            && !reasons.contains_key(&catalog.default_locale)
        {
            reasons.insert(catalog.default_locale.clone(), inline.to_string());
        }
        serde_json::json!({
            "reason": reasons.get(&catalog.default_locale),
            "reasons": reasons,
        })
    };
    let apple = |table: &std::collections::BTreeMap<String, crate::meta::RawText>| {
        table
            .iter()
            .filter(|(_, v)| v.enabled())
            .map(|(k, v)| {
                (
                    k.clone(),
                    with_reasons(&crate::permissions::message_id(k), v.literal()),
                )
            })
            .collect::<serde_json::Map<_, _>>()
    };
    serde_json::json!({
        "android": raw.android,
        "ios": apple(&raw.ios),
        "macos": apple(&raw.macos),
        "ohos": raw.ohos.iter().map(|p| {
            let mut v = with_reasons(&crate::permissions::message_id(&p.name), p.reason.as_deref());
            v["name"] = serde_json::Value::String(p.name.clone());
            v["when"] = serde_json::Value::String(p.when.clone().unwrap_or_else(|| "inuse".into()));
            v
        }).collect::<Vec<_>>(),
    })
}

/// Every permission Day can declare, mirroring `targetCatalog`: tooling (day-vscode's completion
/// for `[permissions]`) reads it from here instead of hand-mirroring the table.
fn permission_catalog() -> Vec<serde_json::Value> {
    day_build::permissions::ALL
        .iter()
        .map(|spec| {
            serde_json::json!({
                "name": spec.name,
                "variant": spec.variant,
                "needsReason": spec.needs_reason,
                "android": spec.android.iter().map(|p| serde_json::json!({
                    "name": p.name, "maxSdk": p.max_sdk,
                })).collect::<Vec<_>>(),
                "ios": spec.ios,
                "macos": spec.macos,
                "ohos": spec.ohos.iter().map(|p| serde_json::json!({
                    "name": p.name, "when": p.when.as_str(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect()
}
