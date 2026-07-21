//! The extension-dependency layering rule (docs/extending.md §4): pieces may depend on parts;
//! parts must not depend on day-pieces or any day-piece-*; tweaks may depend on day-pieces
//! (the built-ins they configure — `Decorate::tweak` lives there) but not on any satellite
//! day-piece-* or day-part-*. Enforced over `cargo metadata` so a violating edge fails
//! `cargo test` on the host instead of quietly knotting the graph.

use std::collections::HashMap;
use std::process::Command;

#[derive(serde::Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    resolve: Resolve,
}

#[derive(serde::Deserialize)]
struct Package {
    id: String,
    name: String,
}

#[derive(serde::Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(serde::Deserialize)]
struct Node {
    id: String,
    deps: Vec<Dep>,
}

#[derive(serde::Deserialize)]
struct Dep {
    pkg: String,
}

#[test]
fn parts_and_tweaks_stay_below_pieces() {
    let workspace_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1"])
        .current_dir(workspace_root)
        .output()
        .expect("cargo metadata runs");
    assert!(out.status.success(), "cargo metadata failed");
    let meta: Metadata = serde_json::from_slice(&out.stdout).expect("metadata parses");

    let name_of: HashMap<&str, &str> = meta
        .packages
        .iter()
        .map(|p| (p.id.as_str(), p.name.as_str()))
        .collect();

    let mut violations = Vec::new();
    for node in &meta.resolve.nodes {
        let Some(&name) = name_of.get(node.id.as_str()) else {
            continue;
        };
        let is_part = name.starts_with("day-part-");
        let is_tweak = name.starts_with("day-tweak-");
        if !is_part && !is_tweak {
            continue;
        }
        for dep in &node.deps {
            let Some(&dep_name) = name_of.get(dep.pkg.as_str()) else {
                continue;
            };
            let dep_is_core_pieces = dep_name == "day-pieces";
            let dep_is_satellite_piece = dep_name.starts_with("day-piece-");
            let dep_is_part = dep_name.starts_with("day-part-");
            if is_part && (dep_is_core_pieces || dep_is_satellite_piece) {
                violations.push(format!("part {name} -> {dep_name}"));
            }
            if is_tweak && (dep_is_satellite_piece || dep_is_part) {
                violations.push(format!("tweak {name} -> {dep_name}"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "layering violations (docs/extending.md §4): {violations:?}"
    );
}
