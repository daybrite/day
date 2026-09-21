// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Template-driven scaffolding for `day new app` (docs/cli.md).
//!
//! The default template is a real directory tree (`crates/day-cli/templates/app/`) embedded in
//! the binary, so a fresh `cargo install day-cli` scaffolds offline. `--template <dir>` swaps in
//! a local directory with the same conventions, and `--template <git-url>[#ref]` shallow-clones
//! a remote one (the create-tauri-app / flutter-create model: templates are ordinary projects
//! with placeholders, not code that prints projects).
//!
//! Conventions, applied uniformly to built-in and user templates:
//! * Every UTF-8 file is rendered with handlebars (`{{name}}`, `{{title}}`, `{{id}}`, …) in
//!   both its content and its path (so `src/{{name}}.rs` works). Strict mode: a typo'd
//!   placeholder is an error, not silent empty output.
//! * Non-UTF-8 files (icons, jars) are copied verbatim.
//! * A trailing `.hbs` on a filename is stripped after rendering; used where the literal name
//!   would confuse tooling scanning the template tree (`Cargo.toml.hbs` keeps cargo from
//!   treating the template as a nested package).
//! * A file named `_gitignore` becomes `.gitignore`, and `_vscode/` and `_github/` become
//!   `.vscode/` and `.github/` (a real dot-file inside the template would be applied by git and
//!   `cargo package` instead of shipped, and a template repository's own `.github/` is its own
//!   CI rather than the scaffolded app's).

use std::path::Path;
use std::process::Command;

use include_dir::{Dir, include_dir};

static APP_TEMPLATE: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/templates/app");

/// One template entry: a forward-slash relative path plus raw bytes.
pub struct TemplateFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// The embedded default app template.
pub fn builtin_app() -> Vec<TemplateFile> {
    let mut out = Vec::new();
    collect_embedded(&APP_TEMPLATE, &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn collect_embedded(dir: &Dir, out: &mut Vec<TemplateFile>) {
    for f in dir.files() {
        out.push(TemplateFile {
            path: f.path().to_string_lossy().replace('\\', "/"),
            bytes: f.contents().to_vec(),
        });
    }
    for d in dir.dirs() {
        collect_embedded(d, out);
    }
}

/// Load a `--template` source: a local directory, or a git URL (optionally `#ref`) that is
/// shallow-cloned to a temp dir. Returns the file set with `.git`, `target` and `.github` pruned.
pub fn load(source: &str) -> Result<Vec<TemplateFile>, String> {
    if !is_git_url(source) {
        let root = Path::new(source);
        if !root.is_dir() {
            return Err(format!("template directory {source:?} not found"));
        }
        return read_tree(root);
    }
    let (url, reference) = match source.split_once('#') {
        Some((u, r)) if !r.is_empty() => (u, Some(r)),
        _ => (source, None),
    };
    let tmp = std::env::temp_dir().join(format!("day-template-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let mut cmd = Command::new("git");
    cmd.args(["clone", "--depth", "1"]);
    if let Some(r) = reference {
        cmd.args(["--branch", r]);
    }
    cmd.arg(url).arg(&tmp);
    let status = cmd
        .status()
        .map_err(|e| format!("running git clone: {e} (is git installed?)"))?;
    if !status.success() {
        return Err(format!("git clone of {url:?} failed"));
    }
    let files = read_tree(&tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    files
}

fn is_git_url(s: &str) -> bool {
    s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("git@")
        || s.starts_with("ssh://")
        || s.starts_with("git+")
}

fn read_tree(root: &Path) -> Result<Vec<TemplateFile>, String> {
    let mut out = Vec::new();
    fn walk(root: &Path, dir: &Path, out: &mut Vec<TemplateFile>) -> Result<(), String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                // `.git` and `target` are the checkout's own; `.github` is the TEMPLATE
                // repository's CI, which has nothing to do with the app being scaffolded — an
                // app's workflows travel as `_github/` and are mapped on the way out.
                if name == ".git" || name == "target" || name == ".github" {
                    continue;
                }
                walk(root, &p, out)?;
            } else {
                let rel = p
                    .strip_prefix(root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                out.push(TemplateFile { path: rel, bytes });
            }
        }
        Ok(())
    }
    walk(root, root, &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// The platform (OS) a `platform/<os>/…` template path belongs to, or None for a
/// target-agnostic file. Matches the target naming convention: `android-mdc`'s platform is
/// `android`, so it owns `platform/android/`.
fn file_platform(path: &str) -> Option<&str> {
    path.strip_prefix("platform/")?.split('/').next()
}

/// Keep the target-agnostic files plus the `platform/<os>/` subtrees belonging to `targets`
/// (`day new app` scaffolds only the host projects its targets need; `day project add-target`
/// materializes the rest later from the same template).
pub fn filter_for_targets(files: Vec<TemplateFile>, targets: &[String]) -> Vec<TemplateFile> {
    // Resolve through the target table, not by splitting the name: `harmony-arkui`'s platform
    // dir is `ohos` (see `Target::os`).
    let resolved: Vec<&'static crate::targets::Target> = targets
        .iter()
        .filter_map(|t| crate::targets::find(t))
        .collect();
    let platforms: Vec<&str> = resolved.iter().map(|t| t.os).collect();
    // `store/` is the App Store / Play listing (§16.6). An app that ships to neither store has no
    // use for it, and scaffolding TODOs nobody will ever fill in is how a lint gets ignored.
    let ships_to_a_store = resolved.iter().any(|t| crate::store::is_store_target(t));
    files
        .into_iter()
        .filter(|f| {
            if f.path == "store" || f.path.starts_with("store/") {
                return ships_to_a_store;
            }
            match file_platform(&f.path) {
                Some(os) => platforms.contains(&os),
                None => true,
            }
        })
        .collect()
}

/// The `platform/<os>/` subtrees belonging to `targets`, which is what `day project add-target` adds
/// to an existing project (the target-agnostic files already exist there), plus the `store/`
/// listing skeleton when any of them ships to a store: an app scaffolded desktop-only never
/// got one, and gaining its first store target is exactly when it becomes needed. (The caller
/// never overwrites, so a project that already has `store/` keeps it untouched.)
pub fn platform_files_for_targets(
    files: Vec<TemplateFile>,
    targets: &[String],
) -> Vec<TemplateFile> {
    let resolved: Vec<&'static crate::targets::Target> = targets
        .iter()
        .filter_map(|t| crate::targets::find(t))
        .collect();
    let platforms: Vec<&str> = resolved.iter().map(|t| t.os).collect();
    let ships_to_a_store = resolved.iter().any(|t| crate::store::is_store_target(t));
    files
        .into_iter()
        .filter(|f| {
            file_platform(&f.path).is_some_and(|os| platforms.contains(&os))
                || (ships_to_a_store && f.path.starts_with("store/"))
        })
        .collect()
}

/// Render a template against `ctx` (any serde-serializable map): paths and UTF-8 contents go
/// through handlebars; binary files pass through. Returns (relative path, bytes) pairs.
pub fn render<S: serde::Serialize>(
    files: &[TemplateFile],
    ctx: &S,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut hb = handlebars::Handlebars::new();
    hb.set_strict_mode(true); // a typo'd {{placeholder}} is an error, not empty output
    hb.register_escape_fn(handlebars::no_escape); // scaffolds are code, not HTML
    let mut out = Vec::with_capacity(files.len());
    for f in files {
        let mut path = hb
            .render_template(&f.path, ctx)
            .map_err(|e| format!("template path {:?}: {e}", f.path))?;
        if let Some(stripped) = path.strip_suffix(".hbs") {
            path = stripped.to_string();
        }
        if let Some(rest) = path.strip_suffix("_gitignore") {
            path = format!("{rest}.gitignore");
        }
        if let Some(rest) = path.strip_prefix("_vscode/") {
            path = format!(".vscode/{rest}");
        }
        if let Some(rest) = path.strip_prefix("_github/") {
            path = format!(".github/{rest}");
        }
        let bytes = match std::str::from_utf8(&f.bytes) {
            Ok(text) => hb
                .render_template(text, ctx)
                .map_err(|e| format!("template {:?}: {e}", f.path))?
                .into_bytes(),
            Err(_) => f.bytes.clone(), // binary (icons, …): copy verbatim
        };
        out.push((path, bytes));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn ctx() -> BTreeMap<&'static str, String> {
        let mut m = BTreeMap::new();
        m.insert("name", "hello-world".to_string());
        // The repository name, which keeps the case the user typed (new.rs `Repl::repo`).
        m.insert("repo", "Hello-World".to_string());
        m.insert("ident", "hello_world".to_string());
        m.insert("snake", "hello_world".to_string());
        m.insert("pascal", "HelloWorld".to_string());
        m.insert("title", "Hello World".to_string());
        m.insert("id", "dev.example.hello_world".to_string());
        // The app id's organization segment, which website/site.toml builds its Pages host from.
        m.insert("org", "example".to_string());
        m.insert("scheme", "helloworld".to_string());
        m.insert("day_dep", "day = { version = \"0.0.0\" }".to_string());
        m.insert(
            "day_build_dep",
            "day-build = { version = \"0.0.0\" }".to_string(),
        );
        m.insert("targets_toml", "\"macos-appkit\"".to_string());
        // The same targets unquoted, for a workflow input (new.rs `targets_list`).
        m.insert("targets_list", "macos-appkit".to_string());
        m.insert("first_target", "macos-appkit".to_string());
        m.insert(
            "day_piece_deps",
            "day-piece-datetime = { version = \"0.0.0\" }".to_string(),
        );
        m
    }

    #[test]
    fn builtin_template_renders() {
        let files = builtin_app();
        assert!(!files.is_empty(), "embedded template is not empty");
        let rendered = render(&files, &ctx()).expect("builtin template renders cleanly");
        let paths: Vec<&str> = rendered.iter().map(|(p, _)| p.as_str()).collect();
        for expected in [
            "Day.toml",
            "Cargo.toml",        // .hbs stripped
            ".gitignore",        // _gitignore mapped
            "website/site.toml", // the daysite config `day new app` ships by default
            "website/theme.css",
            "src/main.rs",
            "src/lib.rs",
        ] {
            assert!(paths.contains(&expected), "missing {expected} in {paths:?}");
        }
        // No unrendered placeholders or convention suffixes survive.
        for (p, bytes) in &rendered {
            assert!(!p.contains("{{") && !p.ends_with(".hbs"), "path {p}");
            if let Ok(text) = std::str::from_utf8(bytes) {
                assert!(!text.contains("{{name}}"), "unrendered placeholder in {p}");
            }
        }
        let cargo = rendered.iter().find(|(p, _)| p == "Cargo.toml").unwrap();
        let cargo = std::str::from_utf8(&cargo.1).unwrap();
        assert!(cargo.contains("name = \"hello-world\""));
        assert!(cargo.contains("day = { version = \"0.0.0\" }"));
    }

    /// The dot-directory conventions: a template repository keeps its own CI under `.github/`,
    /// so the app's travels as `_github/` and is mapped on the way out — the same trick
    /// `_gitignore` and `_vscode/` use.
    #[test]
    fn underscore_directories_become_dot_directories() {
        let files = vec![
            TemplateFile {
                path: "_github/workflows/ci.yml".into(),
                bytes: b"name: ci\n".to_vec(),
            },
            TemplateFile {
                path: "_vscode/extensions.json".into(),
                bytes: b"{}".to_vec(),
            },
            TemplateFile {
                path: "_gitignore".into(),
                bytes: b"/target\n".to_vec(),
            },
        ];
        let rendered = render(&files, &ctx()).expect("renders");
        let paths: Vec<&str> = rendered.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                ".github/workflows/ci.yml",
                ".vscode/extensions.json",
                ".gitignore"
            ]
        );
    }

    /// A template repository's own CI, and the app's. Only the second is the app's business.
    #[test]
    fn a_templates_own_github_directory_is_not_the_apps() {
        let tmp = std::env::temp_dir().join(format!("day-template-github-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        for (rel, body) in [
            (".github/workflows/template.yml", "name: template\n"),
            (".github/README.md", "the template's own docs\n"),
            ("_github/workflows/ci.yml", "name: ci\n"),
            ("Day.toml", "schema = 1\n"),
        ] {
            let path = tmp.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        let files = load(tmp.to_str().unwrap()).expect("loads");
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["Day.toml", "_github/workflows/ci.yml"]);
        let rendered = render(&files, &ctx()).expect("renders");
        assert!(
            rendered
                .iter()
                .any(|(p, _)| p == ".github/workflows/ci.yml"),
            "the app's workflow arrives as .github/: {rendered:?}",
            rendered = rendered.iter().map(|(p, _)| p).collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn target_filtering_scopes_platform_subtrees() {
        let files = builtin_app();
        let ios_only = filter_for_targets(builtin_app(), &["ios-uikit".to_string()]);
        assert!(ios_only.iter().any(|f| f.path.starts_with("platform/ios/")));
        assert!(
            !ios_only
                .iter()
                .any(|f| f.path.starts_with("platform/android/"))
        );
        assert!(
            !ios_only
                .iter()
                .any(|f| f.path.starts_with("platform/harmony/"))
        );
        assert!(ios_only.iter().any(|f| f.path == "Day.toml")); // agnostic files stay
        assert!(
            !ios_only
                .iter()
                .any(|f| f.path.starts_with("platform/macos/"))
        );

        // macos-appkit ships the Xcode host project (platform/macos/) and nothing else's.
        let desktop = filter_for_targets(builtin_app(), &["macos-appkit".to_string()]);
        assert!(
            desktop
                .iter()
                .any(|f| f.path.starts_with("platform/macos/"))
        );
        assert!(!desktop.iter().any(|f| f.path.starts_with("platform/ios/")));

        // The other desktop targets still need no platform subtree at all.
        let gtk = filter_for_targets(builtin_app(), &["linux-gtk".to_string()]);
        assert!(!gtk.iter().any(|f| f.path.starts_with("platform/")));

        // add-target's view: the new target's subtree plus (for a store target) the store/
        // listing skeleton, and nothing else agnostic.
        let add = platform_files_for_targets(files, &["android-mdc".to_string()]);
        assert!(!add.is_empty());
        assert!(
            add.iter()
                .all(|f| f.path.starts_with("platform/android/") || f.path.starts_with("store/"))
        );
        assert!(add.iter().any(|f| f.path.starts_with("store/")));

        // A desktop-only addition brings no store skeleton along.
        let add_desktop = platform_files_for_targets(builtin_app(), &["macos-appkit".to_string()]);
        assert!(
            add_desktop
                .iter()
                .all(|f| f.path.starts_with("platform/macos/"))
        );
    }

    #[test]
    fn strict_mode_rejects_unknown_placeholders() {
        let files = vec![TemplateFile {
            path: "a.txt".into(),
            bytes: b"{{not_a_real_key}}".to_vec(),
        }];
        assert!(render(&files, &ctx()).is_err());
    }

    #[test]
    fn binary_files_copy_verbatim() {
        let png = vec![0x89u8, b'P', b'N', b'G', 0xFF, 0xFE];
        let files = vec![TemplateFile {
            path: "icon.png".into(),
            bytes: png.clone(),
        }];
        let out = render(&files, &ctx()).unwrap();
        assert_eq!(out[0].1, png);
    }
}
