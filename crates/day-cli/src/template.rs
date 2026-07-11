//! Template-driven scaffolding for `day new app` (docs/cli.md).
//!
//! The default template is a real directory tree (`crates/day-cli/templates/app/`) embedded in
//! the binary, so a fresh `cargo install day-cli` scaffolds offline. `--template <dir>` swaps in
//! a local directory with the same conventions, and `--template <git-url>[#ref]` shallow-clones
//! a remote one (the create-tauri-app / flutter-create model: templates are ordinary projects
//! with placeholders, not code that prints projects).
//!
//! Conventions, applied uniformly to built-in and user templates:
//! * Every UTF-8 file is rendered with handlebars — `{{name}}`, `{{title}}`, `{{id}}`, … — in
//!   its CONTENT and in its PATH (so `src/{{name}}.rs` works). Strict mode: a typo'd
//!   placeholder is an error, not silent empty output.
//! * Non-UTF-8 files (icons, jars) are copied verbatim.
//! * A trailing `.hbs` on a filename is stripped after rendering — used where the literal name
//!   would confuse tooling scanning the template tree (`Cargo.toml.hbs` keeps cargo from
//!   treating the template as a nested package).
//! * A file named `_gitignore` becomes `.gitignore` (a real dot-file inside the template would
//!   be APPLIED by git and `cargo package` instead of shipped).

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
/// shallow-cloned to a temp dir. Returns the file set with `.git`/`target` pruned.
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
                if name == ".git" || name == "target" {
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
        m.insert("ident", "hello_world".to_string());
        m.insert("snake", "hello_world".to_string());
        m.insert("pascal", "HelloWorld".to_string());
        m.insert("title", "Hello World".to_string());
        m.insert("id", "dev.example.hello_world".to_string());
        m.insert("scheme", "helloworld".to_string());
        m.insert("day_dep", "day = { version = \"0.0.0\" }".to_string());
        m.insert("targets_yaml", "  - macos-appkit".to_string());
        m.insert("first_target", "macos-appkit".to_string());
        m
    }

    #[test]
    fn builtin_template_renders() {
        let files = builtin_app();
        assert!(!files.is_empty(), "embedded template is not empty");
        let rendered = render(&files, &ctx()).expect("builtin template renders cleanly");
        let paths: Vec<&str> = rendered.iter().map(|(p, _)| p.as_str()).collect();
        for expected in [
            "day.yaml",
            "Cargo.toml", // .hbs stripped
            ".gitignore", // _gitignore mapped
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
