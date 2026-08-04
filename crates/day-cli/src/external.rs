//! External platform-toolkit discovery (docs/extending.md "External toolkits").
//!
//! A toolkit implemented OUTSIDE this repository registers its platform-toolkit pair by declaring
//! it in the toolkit crate's `Cargo.toml`:
//!
//! ```toml
//! [package.metadata.day.toolkit]
//! target = "netbsd-wxwidgets"    # <os>-<toolkit>; the toolkit half names the app's cargo feature
//! host = "any"                   # optional: restrict to "macos" | "linux" | "windows"
//! label = "wxWidgets"            # optional: pickers and error listings
//! doctor = "wx-config --version" # optional: `day doctor` probe (command + space-separated args)
//! ```
//!
//! The CLI resolves `-p <name>` against the builtin catalog first and then against these
//! declarations, read from `cargo metadata` exactly as the piece contracts are (pieces.rs) — so
//! registering a toolkit is pure Cargo.toml data on a crate the app depends on. A declared target
//! inherits the DESKTOP pipeline: `cargo build --features <toolkit-half>`, run the binary, stream
//! logs, dayscript. That single supported shape is deliberate (Stage 0): a new platform KIND
//! (another mobile OS) means new build/launch/pack code, which cannot come from a crate.
//!
//! What external targets do NOT get: `day pack` (guarded with a clear error), `day new`
//! scaffolding, and the in-repo pieces' native renderers (their kinds draw placeholders unless the
//! external ecosystem ships renderer crates). The toolkit SPI itself — day-spec's `Toolkit` and
//! `Platform`, `Event`, `Cap`, the props structs — is UNSTABLE and unpublished: an external
//! toolkit pins the day crates to a git revision and expects breakage between revisions
//! (docs/extending.md spells this out).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;

use crate::meta::Project;
use crate::targets::{self, Target, TargetKind};

/// The `[package.metadata.day.toolkit]` table, as a toolkit crate declares it.
#[derive(Deserialize)]
struct ToolkitMeta {
    /// The platform-toolkit pair, `<os>-<toolkit>`. The toolkit half (everything after the first
    /// `-`) is also the cargo feature the app must declare — the same convention the builtin
    /// targets follow (`macos-gtk`/`linux-gtk` share the `gtk` feature).
    target: String,
    /// Pipeline kind. Stage 0 accepts only `"desktop"` (the default).
    #[serde(default)]
    kind: Option<String>,
    /// Host OS that can build this target (`"any"` default — a wrong host fails in cargo with
    /// the toolchain's own error, which is at least accurate).
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    label: Option<String>,
    /// `day doctor` probe: a command and space-separated arguments, run without a shell.
    #[serde(default)]
    doctor: Option<String>,
}

/// A resolved external toolkit: the leaked catalog entry plus what the CLI needs beyond it.
#[derive(Debug)]
pub struct ExternalToolkit {
    pub target: &'static Target,
    /// The declaring crate, for error messages and listings.
    pub crate_name: String,
    pub doctor: Option<String>,
}

/// Resolved catalogs by project root. Keyed (not a bare `OnceLock`) so tests — and any future
/// multi-project invocation — never see another project's toolkits. Successes are cached and
/// leaked ('static borrows out of a static map); failures are NOT cached, so a fixed Cargo.toml
/// is picked up by the next call without restarting anything.
fn cache() -> &'static Mutex<HashMap<PathBuf, &'static [ExternalToolkit]>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, &'static [ExternalToolkit]>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Every external toolkit the project's dependency graph declares. Runs `cargo metadata` on the
/// first call per project (the same cost pieces::feature_union already pays during a build);
/// scans the full package set rather than a feature closure, because the toolkit crate sits
/// behind the very optional feature its declaration names — a closure would need the answer to
/// compute the question.
pub fn resolve(project: &Project) -> Result<&'static [ExternalToolkit], String> {
    if let Some(hit) = cache()
        .lock()
        .ok()
        .and_then(|c| c.get(&project.root).copied())
    {
        return Ok(hit);
    }
    let meta = crate::pieces::cargo_metadata_all_features(project)?;
    let mut decls: Vec<(String, ToolkitMeta)> = Vec::new();
    for pkg in &meta.packages {
        if let Some(m) = crate::pieces::piece_meta::<ToolkitMeta>(pkg, "toolkit") {
            decls.push((pkg.name.clone(), m));
        }
    }
    let catalog = build_catalog(decls)?;
    let leaked: &'static [ExternalToolkit] = Box::leak(catalog.into_boxed_slice());
    if let Ok(mut c) = cache().lock() {
        c.insert(project.root.clone(), leaked);
    }
    Ok(leaked)
}

/// Validate declarations and leak them into catalog entries. Pure (no I/O), so the rules are
/// unit-testable: name shape, desktop-only kind, and collisions — with the builtin catalog and
/// between declarations — are all hard errors that name the offending crate.
fn build_catalog(decls: Vec<(String, ToolkitMeta)>) -> Result<Vec<ExternalToolkit>, String> {
    fn leak(s: String) -> &'static str {
        Box::leak(s.into_boxed_str())
    }
    let mut out: Vec<ExternalToolkit> = Vec::new();
    for (crate_name, m) in decls {
        let name = m.target;
        let Some((os, toolkit)) = name.split_once('-') else {
            return Err(format!(
                "{crate_name}: [package.metadata.day.toolkit] target {name:?} is not an \
                 <os>-<toolkit> pair"
            ));
        };
        if os.is_empty()
            || toolkit.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(format!(
                "{crate_name}: [package.metadata.day.toolkit] target {name:?} must be lowercase \
                 ascii <os>-<toolkit>"
            ));
        }
        match m.kind.as_deref() {
            None | Some("desktop") => {}
            Some(other) => {
                return Err(format!(
                    "{crate_name}: [package.metadata.day.toolkit] kind {other:?} is not \
                     supported — external toolkits are \"desktop\" only today (a new pipeline \
                     kind needs CLI code, which cannot come from a crate)"
                ));
            }
        }
        if targets::find(&name).is_some() {
            return Err(format!(
                "{crate_name}: [package.metadata.day.toolkit] target {name:?} collides with the \
                 builtin target of the same name"
            ));
        }
        if let Some(prev) = out.iter().find(|t| t.target.name == name) {
            return Err(format!(
                "target {name:?} is declared by both {} and {crate_name}",
                prev.crate_name
            ));
        }
        // `os` is the name prefix BY CONTRACT for external targets (no override key): the
        // `[app.<os>]` table and the platform namespace follow the name. Builtins get to differ
        // (harmony-arkui's os is "ohos") because their table entry says so; an external target
        // with a divergent os would strand every consumer that only has the name to split.
        let target: &'static Target = Box::leak(Box::new(Target {
            name: leak(name.clone()),
            toolkit: leak(toolkit.to_string()),
            kind: TargetKind::Desktop,
            os: leak(os.to_string()),
            host: leak(m.host.unwrap_or_else(|| "any".into())),
            label: leak(m.label.unwrap_or(name)),
            experimental: true,
        }));
        out.push(ExternalToolkit {
            target,
            crate_name,
            doctor: m.doctor,
        });
    }
    Ok(out)
}

/// The combined lookup every target-taking command uses: builtin first, then the project's
/// declarations. The error carries both catalogs, so a typo's correction is on screen.
pub fn find_target(project: &Project, name: &str) -> Result<&'static Target, String> {
    if let Some(t) = targets::find(name) {
        return Ok(t);
    }
    let builtin = || {
        targets::TARGETS
            .iter()
            .map(|t| t.name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    match resolve(project) {
        Ok(ext) => {
            if let Some(t) = ext.iter().find(|t| t.target.name == name) {
                return Ok(t.target);
            }
            let declared = if ext.is_empty() {
                "none".to_string()
            } else {
                ext.iter()
                    .map(|t| format!("{} ({})", t.target.name, t.crate_name))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            Err(format!(
                "unknown target {name:?}\n  builtin:  {}\n  declared by this project's crates: \
                 {declared}",
                builtin()
            ))
        }
        Err(e) => Err(format!(
            "unknown target {name:?} (builtin: {}) — and external toolkit discovery failed: {e}",
            builtin()
        )),
    }
}

/// Is `name` a target this project can launch (builtin or declared)? Lint's cheap boolean; a
/// discovery failure counts as unknown rather than erroring the lint run.
pub fn known(project: &Project, name: &str) -> bool {
    find_target(project, name).is_ok()
}

/// Is this catalog entry an external declaration (vs. a builtin)? Externals are exactly the
/// entries the builtin table does not contain — used to guard `day pack`.
pub fn is_external(target: &Target) -> bool {
    targets::find(target.name).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole discovery path against a REAL `cargo metadata`: a scratch project whose app
    /// depends (optionally — behind the very feature the declaration names) on a toolkit crate
    /// declaring `[package.metadata.day.toolkit]`. Proves the package scan sees optional deps
    /// and that the unknown-target error lists the declaration. No compilation involved —
    /// `cargo metadata` only resolves.
    #[test]
    fn a_fixture_project_declares_a_target_end_to_end() {
        let root =
            std::env::temp_dir().join(format!("day-external-fixture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let tk = root.join("toolkit");
        let app = root.join("app");
        std::fs::create_dir_all(tk.join("src")).unwrap();
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::write(
            tk.join("Cargo.toml"),
            r#"[package]
name = "day-toolkit-fixture"
version = "0.0.0"
edition = "2021"

[package.metadata.day.toolkit]
target = "testos-fixturetk"
label = "Fixture TK"
doctor = "true"
"#,
        )
        .unwrap();
        std::fs::write(tk.join("src/lib.rs"), "").unwrap();
        std::fs::write(
            app.join("Cargo.toml"),
            r#"[package]
name = "fixture-app"
version = "0.0.0"
edition = "2021"

[features]
fixturetk = ["dep:day-toolkit-fixture"]

[dependencies]
day-toolkit-fixture = { path = "../toolkit", optional = true }

[workspace]
"#,
        )
        .unwrap();
        std::fs::write(app.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(
            app.join("Day.toml"),
            r#"schema = 1
[app]
id = "dev.test.fixture"
targets = ["testos-fixturetk"]
"#,
        )
        .unwrap();

        let project = crate::meta::find_project(Some(&app)).expect("fixture project loads");
        let ext = resolve(&project).expect("discovery succeeds");
        assert_eq!(ext.len(), 1);
        assert_eq!(ext[0].target.name, "testos-fixturetk");
        assert_eq!(ext[0].crate_name, "day-toolkit-fixture");
        assert_eq!(ext[0].doctor.as_deref(), Some("true"));

        assert!(find_target(&project, "testos-fixturetk").is_ok());
        assert!(
            find_target(&project, "macos-appkit").is_ok(),
            "builtins still resolve"
        );
        let err = find_target(&project, "bogus").expect_err("unknown stays unknown");
        assert!(
            err.contains("testos-fixturetk (day-toolkit-fixture)"),
            "the error lists the declaration: {err}"
        );
        assert!(crate::external::known(&project, "testos-fixturetk"));

        let _ = std::fs::remove_dir_all(&root);
    }

    fn meta(target: &str) -> ToolkitMeta {
        ToolkitMeta {
            target: target.into(),
            kind: None,
            host: None,
            label: None,
            doctor: None,
        }
    }

    #[test]
    fn a_declaration_becomes_a_desktop_target_with_defaults() {
        let out = build_catalog(vec![("day-toolkit-wx".into(), meta("netbsd-wxwidgets"))])
            .expect("valid declaration");
        let t = out[0].target;
        assert_eq!(t.name, "netbsd-wxwidgets");
        assert_eq!(t.toolkit, "wxwidgets", "feature = the toolkit half");
        assert_eq!(t.os, "netbsd", "os = the name prefix, by contract");
        assert_eq!(t.host, "any");
        assert_eq!(t.kind, TargetKind::Desktop);
        assert!(t.experimental);
    }

    #[test]
    fn a_builtin_collision_is_an_error_naming_the_crate() {
        let err = build_catalog(vec![("evil".into(), meta("macos-appkit"))])
            .expect_err("must not shadow a builtin");
        assert!(err.contains("evil") && err.contains("collides"), "{err}");
    }

    #[test]
    fn a_duplicate_declaration_names_both_crates() {
        let err = build_catalog(vec![
            ("crate-a".into(), meta("netbsd-wxwidgets")),
            ("crate-b".into(), meta("netbsd-wxwidgets")),
        ])
        .expect_err("duplicates are ambiguous");
        assert!(err.contains("crate-a") && err.contains("crate-b"), "{err}");
    }

    #[test]
    fn only_the_desktop_kind_is_accepted() {
        let mut m = meta("solaris-motif");
        m.kind = Some("mobile".into());
        let err = build_catalog(vec![("x".into(), m)]).expect_err("kinds need CLI code");
        assert!(err.contains("desktop"), "{err}");
    }

    #[test]
    fn malformed_names_are_rejected() {
        for bad in ["wxwidgets", "NetBSD-wx", "-wx", "netbsd-", "net bsd-wx"] {
            assert!(
                build_catalog(vec![("x".into(), meta(bad))]).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }
}
