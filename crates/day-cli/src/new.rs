//! `day new` — scaffold Day extension crates and apps (DESIGN.md §8/§15). Three shapes:
//!
//! * `day new piece <name>` — a COMPOSITE piece (pure composition, every backend for free, no
//!   per-backend code).
//! * `day new piece <name> --toolkits <csv>` — a NATIVE piece (a distinct native control per toolkit,
//!   registered link-time with `renderer!`).
//! * `day new part <name> [--platforms <csv>]` — a headless PART (a cross-platform capability with no
//!   UI, dispatched by `#[cfg(target_os)]`).
//!
//! Every scaffold is its OWN cargo workspace, carries a README + .gitignore, and BUILDS out of the box.
//!
//! Dependencies default to **versioned crates.io** deps pinned to this CLI's own version (`day-cli x.y.z`
//! scaffolds against `day x.y.z`), so a scaffold builds standalone with no repo checkout and no network to
//! GitHub. `--git` points them at the `day` git remote instead; the hidden `--local <path>` flag (or the
//! `DAY_LOCAL` env var) emits `path` deps rooted at a local `day` checkout — this is what CI uses to build
//! a freshly-scaffolded crate against the day tree under test.

use std::path::{Path, PathBuf};

use crate::interactive::Prompt;
use crate::ops;
use crate::targets;

/// The `day` git remote, used for scaffold deps under `--git`.
const GIT_URL: &str = "https://github.com/daybrite/day.git";

/// The toolkits a NATIVE piece can carry a backend renderer for.
const TOOLKITS: &[&str] = &["appkit", "gtk", "qt", "uikit", "widget", "winui"];
/// The platforms a PART can carry a native impl for.
const PLATFORMS: &[&str] = &["macos", "ios", "android", "linux", "windows"];

// ---------------------------------------------------------------------------
// Dependency source: versioned crates.io (default), git remote (--git), or a local path (--local / CI).
// ---------------------------------------------------------------------------

enum Deps {
    /// Versioned crates.io deps pinned to this CLI's version (`--registry`) — `day = { version =
    /// "x.y.z" }`. Becomes the default once the day framework crates are published to crates.io.
    Version(&'static str),
    /// The `day` git remote (the CURRENT default — the framework crates are not yet on
    /// crates.io) — `day = { git = "https://github.com/daybrite/day.git" }`.
    Git,
    /// A local `day` checkout (`--local <path>` / `DAY_LOCAL`) — `day = { path = "<root>/<sub>" }`. A
    /// normalized, forward-slash, TOML-safe absolute path. Used by CI and framework development.
    Local(String),
}

impl Deps {
    /// Resolve the dependency source: a local checkout (`--local` or `DAY_LOCAL`) wins, then
    /// `--registry` (versioned crates.io deps pinned to this CLI's own version, so a `day-cli
    /// x.y.z` binary scaffolds an app depending on `day x.y.z`), otherwise the default — the
    /// git remote, because the day framework crates are NOT yet published to crates.io. Flip
    /// the default back to Version (and retire `--git`) when they are.
    fn resolve(local: Option<&Path>, git: bool, registry: bool) -> Self {
        let picked = local
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("DAY_LOCAL").map(PathBuf::from));
        if let Some(p) = picked {
            // Cargo accepts forward slashes on every host; normalize separators and strip Windows'
            // `\\?\` verbatim prefix so the path is a valid (unescaped) TOML basic string.
            let p = p.canonicalize().unwrap_or(p);
            let s = p.to_string_lossy().replace('\\', "/");
            let s = s.strip_prefix("//?/").map(str::to_string).unwrap_or(s);
            return Deps::Local(s);
        }
        if registry && !git {
            return Deps::Version(env!("CARGO_PKG_VERSION"));
        }
        Deps::Git
    }

    /// A full dependency line for a day workspace crate, with `extra` (e.g. `, optional = true`)
    /// spliced inside the braces. Version/git forms ignore the subpath (cargo resolves by package name).
    fn dep(&self, name: &str, extra: &str) -> String {
        match self {
            Deps::Version(v) => format!("{name} = {{ version = \"{v}\"{extra} }}"),
            Deps::Git => format!("{name} = {{ git = \"{GIT_URL}\"{extra} }}"),
            Deps::Local(root) => format!(
                "{name} = {{ path = \"{root}/{sub}\"{extra} }}",
                sub = subpath(name)
            ),
        }
    }
}

/// The workspace-relative directory of a day crate (used only by the local-path form).
fn subpath(crate_name: &str) -> String {
    match crate_name {
        "day" => "crates/day".into(),
        n if n.starts_with("day-piece-") => format!("pieces/{n}"),
        n if n.starts_with("day-part-") => format!("parts/{n}"),
        "day-appkit" | "day-gtk" | "day-qt" | "day-qt-sys" | "day-uikit" | "day-android"
        | "day-winui" | "day-winui-sys" | "day-arkui" | "day-arkui-sys" => {
            format!("toolkits/{crate_name}")
        }
        n => format!("crates/{n}"),
    }
}

// ---------------------------------------------------------------------------
// Name → identifiers.
// ---------------------------------------------------------------------------

struct Repl {
    crate_name: String,  // the crate + directory name, verbatim (e.g. `day-piece-foo`)
    crate_ident: String, // the crate's Rust extern name (hyphens → underscores)
    snake: String,       // a snake_case identifier stem (e.g. `foo`)
    pascal: String,      // PascalCase (e.g. `Foo`) for types + the `Day<Name>` factory class
    id: String,          // reverse-DNS id, also the piece KIND + the Java package
    pkg_slash: String,   // id with `.` → `/` (Java source dir)
    class_slash: String, // `<pkg_slash>/Day<Pascal>` (the JNI class path)
}

impl Repl {
    fn new(name: &str, id: Option<&str>) -> Self {
        let snake = snake_ident(name);
        let pascal = pascalize(&snake);
        let id = id
            .map(String::from)
            .unwrap_or_else(|| format!("dev.example.{snake}"));
        let pkg_slash = id.replace('.', "/");
        Repl {
            crate_name: name.to_string(),
            crate_ident: name.replace('-', "_"),
            class_slash: format!("{pkg_slash}/Day{pascal}"),
            pkg_slash,
            snake,
            pascal,
            id,
        }
    }

    fn expand(&self, tpl: &str) -> String {
        tpl.replace("__PASCAL__", &self.pascal)
            .replace("__SNAKE__", &self.snake)
            .replace("__KIND__", &self.id)
            .replace("__CRATE__", &self.crate_name)
            .replace("__CRATE_IDENT__", &self.crate_ident)
            .replace("__CLASSPATH__", &self.class_slash)
            .replace("__PKG_DOTS__", &self.id)
            .replace("__PKG_SLASH__", &self.pkg_slash)
    }
}

/// A lowercase snake_case stem from an arbitrary name (non-alphanumerics collapse to `_`).
fn snake_ident(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let s = out.trim_matches('_').to_string();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        format!("day_{s}")
    } else {
        s
    }
}

/// PascalCase from a snake_case stem.
fn pascalize(snake: &str) -> String {
    snake
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut cs = w.chars();
            match cs.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + cs.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Public entry points (called from cli.rs) + the flag↔dialog resolvers.
//
// FORMAL LINK between the command-line flags and the `day new` interactive dialog: every field has
// exactly ONE resolver (`resolve_name` / `resolve_id` / the per-kind target/toolkit/platform blocks
// below). Each takes the value parsed from the flags and, when it is absent AND a terminal is
// present, asks the corresponding question. There is no separate wizard; the dialog is the fallback
// branch of the flags. `day new` with no subcommand ([`interactive`]) calls a resolver with every
// value unset, so the whole dialog runs — and any flag the user *did* pass simply skips its question.
// ---------------------------------------------------------------------------

/// `day new` with no subcommand: ask what to build, then run that kind's resolver with no flags set.
pub fn interactive() -> i32 {
    let p = Prompt::new(false);
    if !p.enabled() {
        eprintln!(
            "error: `day new` with no arguments needs an interactive terminal.\n       In a script or CI, use `day new app|piece|part <name> …` with flags."
        );
        return 2;
    }
    let kind = p.choose(
        "What kind of Day project would you like to create?",
        &[
            "App: a complete Day app".into(),
            "Part: a custom platform-integration component (headless)".into(),
            "Piece: a custom user-interface component (a widget)".into(),
        ],
        0,
    );
    match kind {
        0 => app(
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            false,
            false,
        ),
        1 => part(None, None, None, None, false, false, false),
        _ => piece(None, None, false, None, None, false, false, false),
    }
}

/// Resolve the required project name: the positional if given, else prompt (interactive) or error.
fn resolve_name(p: &Prompt, name: Option<&str>) -> Option<String> {
    if let Some(n) = name {
        let n = n.trim();
        if !n.is_empty() {
            return Some(n.to_string());
        }
    }
    if p.enabled() {
        let n = p.line("Project name", None);
        if n.is_empty() {
            // Empty here means EOF (Ctrl-D) at the prompt — report it like the non-interactive path.
            eprintln!("error: a <name> is required.");
            None
        } else {
            Some(n)
        }
    } else {
        eprintln!("error: a <name> is required (e.g. `day new app my-app`).");
        None
    }
}

/// Resolve the reverse-DNS id: the flag if given, else prompt with `default` (interactive) or `default`.
fn resolve_id(p: &Prompt, question: &str, id: Option<&str>, default: &str) -> String {
    if let Some(s) = id {
        let s = s.trim();
        if !s.is_empty() {
            return s.to_string();
        }
    }
    if p.enabled() {
        p.line(question, Some(default))
    } else {
        default.to_string()
    }
}

fn default_id(name: &str) -> String {
    format!("dev.example.{}", snake_ident(name))
}

/// Parse + validate a comma-separated toolkit list for a NATIVE piece.
fn parse_toolkits(csv: &str) -> Result<Vec<String>, i32> {
    let mut v = Vec::new();
    for t in csv.split(',') {
        let t = t.trim().to_ascii_lowercase();
        if t.is_empty() {
            continue;
        }
        if !TOOLKITS.contains(&t.as_str()) {
            eprintln!(
                "error: unknown toolkit {t:?} (choose from {})",
                TOOLKITS.join(", ")
            );
            return Err(2);
        }
        if !v.contains(&t) {
            v.push(t);
        }
    }
    Ok(v)
}

/// Parse + validate a comma-separated platform list for a PART.
fn parse_platforms(csv: &str) -> Result<Vec<String>, i32> {
    let mut v = Vec::new();
    for pl in csv.split(',') {
        let pl = pl.trim().to_ascii_lowercase();
        if pl.is_empty() {
            continue;
        }
        if !PLATFORMS.contains(&pl.as_str()) {
            eprintln!(
                "error: unknown platform {pl:?} (choose from {})",
                PLATFORMS.join(", ")
            );
            return Err(2);
        }
        if !v.contains(&pl) {
            v.push(pl);
        }
    }
    Ok(v)
}

fn toolkit_label(tk: &str) -> String {
    let human = match tk {
        "appkit" => "AppKit — macOS",
        "gtk" => "GTK — Linux / macOS / Windows",
        "qt" => "Qt — Linux / macOS / Windows",
        "uikit" => "UIKit — iOS",
        "widget" => "Android — Views / Compose",
        "winui" => "WinUI — Windows",
        _ => tk,
    };
    format!("{human}  ({tk})")
}

fn platform_label(pl: &str) -> String {
    let human = match pl {
        "macos" => "macOS",
        "ios" => "iOS",
        "android" => "Android",
        "linux" => "Linux",
        "windows" => "Windows",
        _ => pl,
    };
    format!("{human}  ({pl})")
}

/// The TOOLKITS index of the host's own desktop toolkit — the sensible preselection for a native piece.
fn host_toolkit_index() -> usize {
    let want = match targets::host_os() {
        "linux" => "gtk",
        "windows" => "winui",
        _ => "appkit",
    };
    TOOLKITS.iter().position(|&t| t == want).unwrap_or(0)
}

fn target_menu_label(t: &targets::Target) -> String {
    if t.experimental {
        format!("{}  ({})  [EXPERIMENTAL]", t.label, t.name)
    } else {
        format!("{}  ({})", t.label, t.name)
    }
}

/// Scaffold a piece. No `--toolkits` (and not interactively chosen native) ⇒ a COMPOSITE piece.
#[allow(clippy::too_many_arguments)] // one arg per `day new piece` flag, resolved in order
pub fn piece(
    name: Option<&str>,
    toolkits_csv: Option<&str>,
    composite: bool,
    id: Option<&str>,
    local: Option<&Path>,
    git: bool,
    registry: bool,
    no_input: bool,
) -> i32 {
    let p = Prompt::new(no_input);
    let Some(name) = resolve_name(&p, name) else {
        return 2;
    };
    let dir = PathBuf::from(&name);
    if dir.exists() {
        eprintln!("error: {name:?} already exists");
        return 1;
    }
    let deps = Deps::resolve(local, git, registry);

    // Toolkits: an explicit --toolkits list wins; --composite forces empty; otherwise ask (or, when
    // non-interactive, default to a composite piece — the zero-config choice).
    let toolkits: Vec<String> = if composite {
        Vec::new()
    } else if let Some(csv) = toolkits_csv {
        match parse_toolkits(csv) {
            Ok(v) => v,
            Err(code) => return code,
        }
    } else if p.enabled() {
        let native = p.choose(
            "What kind of piece?",
            &[
                "Composite — pure composition; every backend for free, no per-backend code".into(),
                "Native — a distinct native control, one implementation per toolkit".into(),
            ],
            0,
        ) == 1;
        if native {
            let opts: Vec<String> = TOOLKITS.iter().map(|t| toolkit_label(t)).collect();
            let picked = p.choose_multi(
                "Which toolkits should it support?",
                &opts,
                &[host_toolkit_index()],
            );
            if picked.is_empty() {
                eprintln!("error: a native piece needs at least one toolkit.");
                return 2;
            }
            picked.iter().map(|i| TOOLKITS[*i].to_string()).collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let rid = resolve_id(
        &p,
        "Reverse-DNS id (also the piece KIND)",
        id,
        &default_id(&name),
    );
    let repl = Repl::new(&name, Some(rid.as_str()));

    let (files, next) = if toolkits.is_empty() {
        (composite_piece_files(&repl, &deps), COMPOSITE_NEXT)
    } else {
        (native_piece_files(&repl, &deps, &toolkits), NATIVE_NEXT)
    };
    let code = write_all(&dir, &files, &name);
    if code == 0 {
        eprintln!("{}", repl.expand(next));
    }
    code
}

/// Scaffold a headless part. No `--platforms` (and not interactively chosen) ⇒ all platforms.
pub fn part(
    name: Option<&str>,
    platforms_csv: Option<&str>,
    id: Option<&str>,
    local: Option<&Path>,
    git: bool,
    registry: bool,
    no_input: bool,
) -> i32 {
    let p = Prompt::new(no_input);
    let Some(name) = resolve_name(&p, name) else {
        return 2;
    };
    let dir = PathBuf::from(&name);
    if dir.exists() {
        eprintln!("error: {name:?} already exists");
        return 1;
    }
    let deps = Deps::resolve(local, git, registry);

    let platforms: Vec<String> = if let Some(csv) = platforms_csv {
        match parse_platforms(csv) {
            Ok(v) => v,
            Err(code) => return code,
        }
    } else if p.enabled() {
        let opts: Vec<String> = PLATFORMS.iter().map(|pl| platform_label(pl)).collect();
        let all: Vec<usize> = (0..PLATFORMS.len()).collect();
        let picked = p.choose_multi("Which platforms should it support?", &opts, &all);
        if picked.is_empty() {
            eprintln!("error: a part needs at least one platform.");
            return 2;
        }
        picked.iter().map(|i| PLATFORMS[*i].to_string()).collect()
    } else {
        PLATFORMS.iter().map(|s| s.to_string()).collect()
    };

    let rid = resolve_id(
        &p,
        "Reverse-DNS id (also the Java package)",
        id,
        &default_id(&name),
    );
    let repl = Repl::new(&name, Some(rid.as_str()));
    let files = part_files(&repl, &deps, &platforms);
    let code = write_all(&dir, &files, &name);
    if code == 0 {
        eprintln!("{}", repl.expand(PART_NEXT));
    }
    code
}

/// Scaffold a Day APP. Targets come from repeated `--toolkit <target>` and/or a `--targets <csv>`;
/// absent ⇒ interactive multi-select (or, non-interactively, the host's default target). `--appid` /
/// `--bundleid` / `--id` all name the same reverse-DNS id.
#[allow(clippy::too_many_arguments)] // one arg per `day new app` flag, resolved in order
pub fn app(
    name: Option<&str>,
    toolkits: &[String],
    appid: Option<&str>,
    bundleid: Option<&str>,
    id: Option<&str>,
    title: Option<&str>,
    template: Option<&str>,
    targets_csv: Option<&str>,
    local: Option<&Path>,
    git: bool,
    registry: bool,
    no_input: bool,
) -> i32 {
    let p = Prompt::new(no_input);
    let Some(name) = resolve_name(&p, name) else {
        return 2;
    };
    let dir = PathBuf::from(&name);
    if dir.exists() {
        eprintln!("error: {name:?} already exists");
        return 1;
    }
    let deps = Deps::resolve(local, git, registry);

    // --appid / --bundleid / --id all name the same reverse-DNS id; reject a genuine conflict.
    let flag_id = match (appid.map(str::trim), bundleid.map(str::trim)) {
        (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() && a != b => {
            eprintln!("error: --appid ({a:?}) and --bundleid ({b:?}) must match");
            return 2;
        }
        (Some(a), _) if !a.is_empty() => Some(a),
        (_, Some(b)) if !b.is_empty() => Some(b),
        _ => id,
    };
    let rid = resolve_id(
        &p,
        "Bundle id / application id (reverse-DNS)",
        flag_id,
        &default_id(&name),
    );

    // Targets: repeated --toolkit + optional --targets csv, each comma-splittable.
    let mut requested: Vec<String> = Vec::new();
    for raw in toolkits.iter().map(String::as_str).chain(targets_csv) {
        for t in raw.split(',') {
            let t = t.trim();
            if !t.is_empty() && !requested.iter().any(|x| x == t) {
                requested.push(t.to_string());
            }
        }
    }

    let targets: Vec<String> = if !requested.is_empty() {
        for t in &requested {
            if targets::find(t).is_none() {
                eprintln!(
                    "error: unknown target {t:?}\n       choose from: {}",
                    targets::TARGETS
                        .iter()
                        .map(|t| t.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                return 2;
            }
        }
        requested
    } else if p.enabled() {
        let opts: Vec<String> = targets::TARGETS.iter().map(target_menu_label).collect();
        let default_idx = targets::TARGETS
            .iter()
            .position(|t| t.name == targets::host_default())
            .unwrap_or(0);
        let picked = p.choose_multi(
            "Which platforms/toolkits should the app support?",
            &opts,
            &[default_idx],
        );
        if picked.is_empty() {
            eprintln!("error: an app needs at least one target.");
            return 2;
        }
        picked
            .iter()
            .map(|i| targets::TARGETS[*i].name.to_string())
            .collect()
    } else {
        vec![targets::host_default().to_string()]
    };

    let title = match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => t.to_string(),
        None => {
            let d = default_title(&name);
            p.line("App title (window / store display name)", Some(&d))
        }
    };

    let repl = Repl::new(&name, Some(rid.as_str()));
    let ctx = template_context(&repl, title, &deps, &targets);
    let first = ctx["first_target"].clone();

    let files = match load_template(template) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    // Only the host projects the chosen targets need — `day app add-toolkit` materializes the
    // rest from the same template later.
    let files = crate::template::filter_for_targets(files, &targets);
    let rendered = match crate::template::render(&files, &ctx) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let code = write_all_bytes(&dir, &rendered, &name);
    if code == 0 {
        eprintln!("\n  next:\n    cd {name}\n    day doctor\n    day launch -p {first}\n");
    }
    code
}

/// The template context (docs/cli.md): every {{placeholder}} a template may use — built ONCE
/// here so `day new app` and `day app add-toolkit` render the same template identically.
fn template_context(
    repl: &Repl,
    title: String,
    deps: &Deps,
    targets: &[String],
) -> std::collections::BTreeMap<&'static str, String> {
    let mut ctx = std::collections::BTreeMap::new();
    ctx.insert("name", repl.crate_name.clone());
    ctx.insert("ident", repl.crate_ident.clone());
    ctx.insert("snake", repl.snake.clone());
    ctx.insert("pascal", repl.pascal.clone());
    ctx.insert("title", title);
    ctx.insert("id", repl.id.clone());
    // A deep-link URI scheme derived from the name (schemes allow only ALPHA/DIGIT/+/-/.).
    let scheme: String = repl
        .crate_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    ctx.insert(
        "scheme",
        if scheme.is_empty() {
            "dayapp".into()
        } else {
            scheme
        },
    );
    ctx.insert("day_dep", deps.dep("day", ""));
    // The resource-constant codegen helper the app's build.rs calls (§18.5) — same source (git /
    // version / local path) as the `day` dep so it resolves identically.
    ctx.insert("day_build_dep", deps.dep("day-build", ""));
    ctx.insert(
        "targets_toml",
        targets
            .iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(", "),
    );
    ctx.insert(
        "first_target",
        targets
            .first()
            .map(String::as_str)
            .unwrap_or("macos-appkit")
            .to_string(),
    );
    ctx
}

/// The `--template` source (dir / git URL), or the embedded default.
fn load_template(source: Option<&str>) -> Result<Vec<crate::template::TemplateFile>, String> {
    match source {
        Some(s) => crate::template::load(s),
        None => Ok(crate::template::builtin_app()),
    }
}

/// `day app add-toolkit <target>…` — add targets to an EXISTING app: append them to
/// Day.toml's `targets:` (textually, preserving comments and formatting) and materialize any
/// native host projects they need from the SAME template `day new app` scaffolds from.
pub fn add_toolkit(
    project: &crate::meta::Project,
    requested: &[String],
    template: Option<&str>,
) -> i32 {
    // Requested targets: repeatable and comma-splittable, validated against the target table.
    let mut wanted: Vec<String> = Vec::new();
    for raw in requested {
        for t in raw.split(',') {
            let t = t.trim();
            if !t.is_empty() && !wanted.iter().any(|x| x == t) {
                wanted.push(t.to_string());
            }
        }
    }
    if wanted.is_empty() {
        eprintln!(
            "error: no target given\n       usage: day app add-toolkit <target>… (e.g. android-widget)"
        );
        return 2;
    }
    for t in &wanted {
        if targets::find(t).is_none() {
            eprintln!(
                "error: unknown target {t:?}\n       choose from: {}",
                targets::TARGETS
                    .iter()
                    .map(|t| t.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            return 2;
        }
    }
    let existing = &project.manifest.app.targets;
    let new_targets: Vec<String> = wanted
        .iter()
        .filter(|t| !existing.contains(t))
        .cloned()
        .collect();
    for already in wanted.iter().filter(|t| existing.contains(t)) {
        eprintln!("day: {already} is already a target of this app — skipping");
    }
    if new_targets.is_empty() {
        return 0;
    }

    // The SAME context `day new app` renders with, rebuilt from the app's own Day.toml.
    let app = &project.manifest.app;
    let repl = Repl::new(&app.name, Some(app.id.as_str()));
    let title = app
        .title
        .clone()
        .unwrap_or_else(|| default_title(&app.name));
    let deps = Deps::resolve(None, false, false);
    let all_targets: Vec<String> = existing.iter().chain(new_targets.iter()).cloned().collect();
    let ctx = template_context(&repl, title, &deps, &all_targets);

    let files = match load_template(template) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let files = crate::template::platform_files_for_targets(files, &new_targets);
    let rendered = match crate::template::render(&files, &ctx) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    // Write the host-project files, never overwriting anything already in the project.
    let mut written = 0usize;
    let mut skipped = 0usize;
    for (path, content) in &rendered {
        let full = project.root.join(path);
        if full.exists() {
            skipped += 1;
            continue;
        }
        if let Some(parent) = full.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&full, content) {
            eprintln!("error writing {}: {e}", full.display());
            return 1;
        }
        written += 1;
    }

    // Day.toml: append to `[app] targets` via toml_edit — comments and formatting survive.
    let day_toml = project.root.join("Day.toml");
    let text = match std::fs::read_to_string(&day_toml) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error reading {}: {e}", day_toml.display());
            return 1;
        }
    };
    let refs: Vec<&str> = new_targets.iter().map(String::as_str).collect();
    let updated = match add_targets_to_day_toml(&text, &refs) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if let Err(e) = std::fs::write(&day_toml, updated) {
        eprintln!("error writing {}: {e}", day_toml.display());
        return 1;
    }

    let list = new_targets.join(", ");
    let files_note = if skipped > 0 {
        format!("{written} file(s) added, {skipped} already present")
    } else {
        format!("{written} file(s) added")
    };
    ops::status("Added", &format!("{list} → Day.toml ({files_note})"));
    let first = &new_targets[0];
    // `day doctor` groups by its own toolkit ids, which differ from the backend feature names
    // for the two mobile toolkits.
    let toolkit = match targets::find(first).map(|t| t.toolkit).unwrap_or_default() {
        "widget" => "android",
        "arkui" => "harmonyos",
        other => other,
    };
    eprintln!("\n  next:\n    day doctor --toolkit {toolkit}\n    day launch -p {first}\n");
    0
}

/// Append targets to Day.toml's `[app] targets` array via `toml_edit` — the format- and
/// comment-preserving TOML layer cargo itself uses, so the rest of the file (and the array's
/// own style) comes back byte-identical. Creates the array (or the [app] table) if absent.
fn add_targets_to_day_toml(text: &str, new_targets: &[&str]) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("Day.toml: {e}"))?;
    let app = doc
        .entry("app")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let app = app.as_table_mut().ok_or("Day.toml: [app] is not a table")?;
    let targets = app
        .entry("targets")
        .or_insert(toml_edit::value(toml_edit::Array::new()));
    let arr = targets
        .as_array_mut()
        .ok_or("Day.toml: app.targets is not an array")?;
    for t in new_targets {
        arr.push(*t);
    }
    Ok(doc.to_string())
}

/// `hello-world` ⇒ `Hello World`: the default display title from a crate-style name.
fn default_title(name: &str) -> String {
    name.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut cs = w.chars();
            match cs.next() {
                Some(f) => f.to_uppercase().to_string() + cs.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn write_all_bytes(dir: &Path, files: &[(String, Vec<u8>)], name: &str) -> i32 {
    for (path, content) in files {
        let full = dir.join(path);
        if let Some(parent) = full.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&full, content) {
            eprintln!("error writing {}: {e}", full.display());
            return 1;
        }
    }
    ops::status("Created", &format!("{name}/ ({} files)", files.len()));
    0
}

fn write_all(dir: &Path, files: &[(String, String)], name: &str) -> i32 {
    for (path, content) in files {
        let full = dir.join(path);
        if let Some(parent) = full.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&full, content) {
            eprintln!("error writing {}: {e}", full.display());
            return 1;
        }
    }
    ops::status("Created", &format!("{name}/ ({} files)", files.len()));
    0
}

// ---------------------------------------------------------------------------
// COMPOSITE piece — pure composition, no features, works on every backend.
// ---------------------------------------------------------------------------

fn composite_piece_files(r: &Repl, deps: &Deps) -> Vec<(String, String)> {
    let cargo = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

# A COMPOSITE Day piece: a reusable widget built PURELY from Day's core primitives — no native /
# per-backend code and NO cargo features, so it works on every backend for free. Depend on it with a
# plain `{{ workspace = true }}` (or git) line and call the builder from `use day::prelude::*` code.

[dependencies]
{day_pieces}
{day_core}
{day_reactive}

[workspace]
"#,
        name = r.crate_name,
        day_pieces = deps.dep("day-pieces", ""),
        day_core = deps.dep("day-core", ""),
        day_reactive = deps.dep("day-reactive", ""),
    );

    vec![
        ("Cargo.toml".into(), cargo),
        (".gitignore".into(), GITIGNORE.into()),
        ("README.md".into(), r.expand(COMPOSITE_README)),
        ("src/lib.rs".into(), r.expand(COMPOSITE_LIB)),
    ]
}

// ---------------------------------------------------------------------------
// NATIVE piece — a distinct native control per toolkit, two-way bound to a Signal<String>.
// ---------------------------------------------------------------------------

fn native_piece_files(r: &Repl, deps: &Deps, toolkits: &[String]) -> Vec<(String, String)> {
    let has = |t: &str| toolkits.iter().any(|x| x == t);
    let needs_build_rs = has("qt") || has("winui");

    // [features]
    let mut features = String::new();
    for t in toolkits {
        let entry = match t.as_str() {
            "appkit" => {
                "appkit = [\"dep:day-appkit\", \"dep:objc2\", \"dep:objc2-app-kit\", \"dep:objc2-foundation\"]"
            }
            "gtk" => "gtk = [\"dep:day-gtk\", \"dep:gtk4\"]",
            "qt" => "qt = [\"dep:day-qt\"]",
            "uikit" => {
                "uikit = [\"dep:day-uikit\", \"dep:objc2\", \"dep:objc2-ui-kit\", \"dep:objc2-foundation\", \"dep:objc2-core-foundation\"]"
            }
            "widget" => "widget = [\"dep:day-android\"]",
            "winui" => "winui = [\"dep:day-winui\", \"dep:day-winui-sys\"]",
            _ => continue,
        };
        features.push_str(entry);
        features.push('\n');
    }
    // A no-renderer mock feature so an app can enable `<pkg>/mock` uniformly (the kind then falls back
    // to day's placeholder leaf under the mock backend).
    features.push_str("mock = []\n");

    // [package.metadata.day.piece].backends
    let mut backends: Vec<String> = toolkits.iter().map(|t| format!("\"{t}\"")).collect();
    backends.push("\"mock\"".into());

    // day-crate deps (optional, gated by the features above).
    let mut day_deps = String::new();
    let mut push_dep = |s: String| {
        day_deps.push_str(&s);
        day_deps.push('\n');
    };
    if has("appkit") {
        push_dep(deps.dep("day-appkit", ", optional = true"));
    }
    if has("gtk") {
        push_dep(deps.dep("day-gtk", ", optional = true"));
    }
    if has("qt") {
        push_dep(deps.dep("day-qt", ", optional = true"));
    }
    if has("uikit") {
        push_dep(deps.dep("day-uikit", ", optional = true"));
    }
    if has("widget") {
        push_dep(deps.dep("day-android", ", optional = true"));
    }
    if has("winui") {
        push_dep(deps.dep("day-winui", ", optional = true"));
        push_dep(deps.dep("day-winui-sys", ", optional = true"));
    }

    // crates.io deps for the native bindings (only for chosen toolkits).
    let mut ext_deps = String::new();
    if has("gtk") {
        ext_deps.push_str("gtk4 = { version = \"0.11\", optional = true }\n");
    }
    if has("appkit") || has("uikit") {
        ext_deps.push_str("objc2 = { version = \"0.6\", optional = true }\n");
        ext_deps.push_str("objc2-foundation = { version = \"0.3\", optional = true, features = [\"NSString\", \"NSNotification\"] }\n");
    }
    if has("appkit") {
        ext_deps.push_str("objc2-app-kit = { version = \"0.3\", optional = true, features = [\"NSControl\", \"NSTextField\", \"NSView\", \"NSResponder\"] }\n");
    }
    if has("uikit") {
        ext_deps.push_str("objc2-ui-kit = { version = \"0.3\", optional = true, features = [\"UITextField\", \"UIControl\", \"UIView\", \"UIResponder\"] }\n");
        ext_deps.push_str("objc2-core-foundation = { version = \"0.3\", optional = true, features = [\"CFCGTypes\"] }\n");
    }

    // Android / iOS backend-contribution metadata.
    let mut meta = String::new();
    if has("widget") {
        meta.push_str(
            "\n# Standalone-piece Android contribution: `day build` reads this from `cargo metadata`\n\
             # and folds the piece's own Java into the app's Gradle build, without touching day-android.\n\
             [package.metadata.day.android]\n\
             java = [\"android/java\"]\n\
             # res = [\"android/res\"]\n\
             # gradle-dependencies = [\"group:artifact:version\"]\n\
             # permissions = [\"android.permission.INTERNET\"]\n",
        );
    }
    if has("uikit") {
        meta.push_str(
            "\n# Standalone-piece iOS contribution: system frameworks to link, and any SwiftPM packages\n\
             # or Swift shim dirs. A plain UITextField needs none — left empty as a template.\n\
             [package.metadata.day.ios]\n\
             frameworks = []\n\
             # swift = [\"ios/swift\"]\n",
        );
    }

    let build_line = if needs_build_rs {
        "build = \"build.rs\"\n"
    } else {
        ""
    };
    let build_deps = if needs_build_rs {
        // day-toolchain: shared SDK discovery (cppwinrt headers etc.) with env overrides
        // (docs/environment.md) — same remote/local source as the other day deps.
        format!(
            "\n[build-dependencies]\ncc = \"1\"\n{}\n",
            deps.dep("day-toolchain", "")
        )
    } else {
        String::new()
    };

    let cargo = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
{build_line}
# A NATIVE Day piece: a two-way text input realized as a DISTINCT native control per toolkit,
# registered link-time into each backend's renderer slice without touching any core day crate.
# Depend on it with a plain `{{ workspace = true }}` (or git) line — `day` unions `<pkg>/<backend>`
# into the app build, so an app never re-lists these per-backend features.

[features]
{features}
# Backends this piece carries a native-renderer [features] entry for.
[package.metadata.day.piece]
backends = [{backends}]

[dependencies]
{day_spec}
{day_core}
{day_pieces}
{day_reactive}
linkme = "0.3"
{day_deps}{ext_deps}{build_deps}{meta}"#,
        name = r.crate_name,
        features = features.trim_end(),
        backends = backends.join(", "),
        day_spec = deps.dep("day-spec", ""),
        day_core = deps.dep("day-core", ""),
        day_pieces = deps.dep("day-pieces", ""),
        day_reactive = deps.dep("day-reactive", ""),
        day_deps = day_deps,
        ext_deps = ext_deps,
    );

    // src/lib.rs front-end: one glue_modules! call for the chosen toolkits instead of the
    // hand-written per-toolkit cfg blocks (docs/extending.md §2; "mock" has no glue module).
    let glue: Vec<String> = toolkits
        .iter()
        .filter(|t| t.as_str() != "mock")
        .cloned()
        .collect();
    let mod_decls = if glue.is_empty() {
        String::new()
    } else {
        format!("day_pieces::glue_modules!({});", glue.join(", "))
    };
    let lib = r.expand(&NATIVE_LIB.replace("__MOD_DECLS__", &mod_decls));

    let mut files = vec![
        ("Cargo.toml".into(), cargo),
        (".gitignore".into(), GITIGNORE.into()),
        ("README.md".into(), r.expand(NATIVE_README)),
        ("src/lib.rs".into(), lib),
    ];

    if has("appkit") {
        files.push(("src/lib-appkit.rs".into(), r.expand(APPKIT_IMPL)));
    }
    if has("gtk") {
        files.push(("src/lib-gtk.rs".into(), r.expand(GTK_IMPL)));
    }
    if has("qt") {
        files.push(("src/lib-qt.rs".into(), r.expand(QT_IMPL)));
        files.push(("src/lib-qt-shim.cpp".into(), r.expand(QT_SHIM)));
    }
    if has("uikit") {
        files.push(("src/lib-uikit.rs".into(), r.expand(UIKIT_IMPL)));
    }
    if has("widget") {
        files.push(("src/lib-android.rs".into(), r.expand(ANDROID_IMPL)));
        files.push((
            format!("android/java/{}/Day{}.java", r.pkg_slash, r.pascal),
            r.expand(ANDROID_JAVA),
        ));
    }
    if has("winui") {
        files.push(("src/lib-winui.rs".into(), r.expand(WINUI_IMPL)));
        files.push(("src/lib-winui-shim.cpp".into(), r.expand(WINUI_SHIM)));
    }
    if needs_build_rs {
        files.push(("build.rs".into(), r.expand(BUILD_RS)));
    }

    files
}

// ---------------------------------------------------------------------------
// PART — a headless cross-platform capability.
// ---------------------------------------------------------------------------

fn part_files(r: &Repl, deps: &Deps, platforms: &[String]) -> Vec<(String, String)> {
    let has = |p: &str| platforms.iter().any(|x| x == p);

    // Per-platform cfg/path module declarations for src/lib.rs.
    let mut cfg_mods = String::new();
    let mut push_mod = |cfg: &str, file: &str| {
        cfg_mods.push_str(&format!(
            "#[cfg({cfg})]\n#[path = \"{file}\"]\nmod imp;\n\n"
        ));
    };
    if has("macos") {
        push_mod("target_os = \"macos\"", "macos.rs");
    }
    if has("ios") {
        push_mod("target_os = \"ios\"", "ios.rs");
    }
    if has("windows") {
        push_mod("target_os = \"windows\"", "windows.rs");
    }
    if has("linux") {
        push_mod(
            "all(target_os = \"linux\", not(target_env = \"ohos\"))",
            "linux.rs",
        );
    }
    if has("android") {
        push_mod("target_os = \"android\"", "android.rs");
    }

    // The MANDATORY catch-all fallback: every target NOT covered above returns None.
    let os_terms: Vec<String> = platforms
        .iter()
        .map(|p| format!("target_os = \"{p}\""))
        .collect();

    // Target-gated deps: only Android rides on the day runtime (its Java shim needs day-android).
    let mut dep_sections = String::new();
    if has("android") {
        dep_sections.push_str(&format!(
            "\n# Android reads through a Java shim + day-android's cached JVM/Context — the one platform\n\
             # where a headless part rides on the day runtime (like the pieces' Android backends).\n\
             [target.'cfg(target_os = \"android\")'.dependencies]\n{}\n",
            deps.dep("day-android", ""),
        ));
    }

    // Backend-contribution metadata.
    let mut meta = String::new();
    if has("android") {
        meta.push_str(
            "\n# `day build` stages this Java into the app's Gradle build (and merges any permissions),\n\
             # without touching day-android. This headless part registers NO renderer.\n\
             [package.metadata.day.android]\n\
             java = [\"android/java\"]\n\
             # permissions = [\"android.permission.INTERNET\"]\n",
        );
    }
    if has("ios") || has("macos") {
        meta.push_str(
            "\n# System frameworks the app must link on iOS (Rust `#[link]` is honored only when cargo\n\
             # drives the final link — on iOS xcodebuild links the staticlib and ignores it). Empty template.\n\
             [package.metadata.day.ios]\n\
             frameworks = []\n",
        );
    }

    let cargo = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

# A HEADLESS Day part: a cross-platform capability with NO UI. Any Rust code can depend on it and call
# `{ident}::status()`. Platform selection is by `#[cfg(target_os)]` (it depends on the OS, not a widget
# toolkit), so there are NO backend features — it "just works" per target.

[dependencies]
# Most platforms need no crates for a native reading (plain std / C FFI). Add per-platform deps as you
# implement each, e.g.:
#   [target.'cfg(target_os = "macos")'.dependencies]
#   core-foundation = "0.10"
{dep_sections}{meta}
[workspace]
"#,
        name = r.crate_name,
        ident = r.crate_ident,
        dep_sections = dep_sections,
        meta = meta,
    );

    let lib = r.expand(
        &PART_LIB
            .replace("__CFG_MODS__", cfg_mods.trim_end())
            .replace("__NOT_ANY__", &os_terms.join(",\n    ")),
    );

    let mut files = vec![
        ("Cargo.toml".into(), cargo),
        (".gitignore".into(), GITIGNORE.into()),
        ("README.md".into(), r.expand(PART_README)),
        ("src/lib.rs".into(), lib),
        (format!("examples/{}.rs", r.snake), r.expand(PART_EXAMPLE)),
    ];
    if has("macos") {
        files.push(("src/macos.rs".into(), r.expand(&part_stub("macOS"))));
    }
    if has("ios") {
        files.push(("src/ios.rs".into(), r.expand(&part_stub("iOS"))));
    }
    if has("windows") {
        files.push(("src/windows.rs".into(), r.expand(&part_stub("Windows"))));
    }
    if has("linux") {
        files.push(("src/linux.rs".into(), r.expand(&part_stub("Linux"))));
    }
    if has("android") {
        files.push(("src/android.rs".into(), r.expand(PART_ANDROID)));
        files.push((
            format!("android/java/{}/Day{}.java", r.pkg_slash, r.pascal),
            r.expand(PART_ANDROID_JAVA),
        ));
    }
    files
}

/// A per-OS stub returning a sample value (replace the body with the real native reading).
fn part_stub(os: &str) -> String {
    format!(
        "// {os}: TODO — read your capability via the platform's native API. This stub returns a sample.\n\n\
         pub fn status() -> Option<super::Sample> {{\n    \
         Some(super::Sample {{ value: 42 }})\n}}\n"
    )
}

// ===========================================================================
// Templates. `__PASCAL__` / `__SNAKE__` / `__KIND__` / `__CRATE__` / `__CRATE_IDENT__` /
// `__CLASSPATH__` / `__PKG_DOTS__` / `__PKG_SLASH__` are substituted by `Repl::expand`.
// ===========================================================================

const GITIGNORE: &str = "/target\nCargo.lock\n";

const COMPOSITE_NEXT: &str = "\n  next:\n    cd __CRATE__\n    cargo build            # builds against day on crates.io (--git / --local override the source)\n    # then, from an app:  __CRATE_IDENT__::__SNAKE__(\"Hello\")\n";
const NATIVE_NEXT: &str = "\n  next:\n    cd __CRATE__\n    cargo build --features <toolkit>   # e.g. appkit / gtk / qt\n    # wire it into an app: add __CRATE__ as a dependency and call __CRATE_IDENT__::__SNAKE__(signal)\n";
const PART_NEXT: &str = "\n  next:\n    cd __CRATE__\n    cargo build            # host platform\n    cargo run --example __SNAKE__\n";

// --- COMPOSITE piece --------------------------------------------------------

const COMPOSITE_LIB: &str = r#"//! __CRATE__ — a COMPOSITE Day piece (built PURELY from Day's core primitives).
//!
//! There is no per-backend/native code and no cargo features here: this widget works on every backend
//! for free. Drop the crate in as a plain dependency and call [`__SNAKE__`] from `use day::prelude::*`
//! code. This sample is a rounded "chip" badge — replace it with your own composition.

use day_pieces::prelude::*;

/// The chip's fill (iOS system blue). Swap for your own palette.
const CHIP_BG: Color = Color::hex(0x0A_84_FF);

/// A small rounded, padded, colored label — the "hello world" of composite pieces. Pure composition
/// over [`label`] + the [`Decorate`] modifiers, so it renders natively on every backend.
///
/// ```ignore
/// use day::prelude::*;
/// column((
///     label("Downloads"),
///     __SNAKE__("3 new"),
/// ))
/// ```
pub fn __SNAKE__(text: impl Into<String>) -> AnyPiece {
    label(text.into())
        .font(Font::Caption)
        .color(Color::WHITE)
        .padding(Insets::symmetric(10.0, 4.0))
        .background(CHIP_BG)
        .corner_radius(10.0)
        .any()
}
"#;

const COMPOSITE_README: &str = r#"# __CRATE__

A **composite** Day piece — a reusable widget built purely from Day's core primitives. There is no
per-backend or native code and no cargo features, so it works on every backend (AppKit, GTK, Qt,
UIKit, Android, WinUI) for free.

## Use

Add it as a dependency (versioned, from crates.io by default) and call the builder from your app:

```rust
use day::prelude::*;
use __CRATE_IDENT__::__SNAKE__;

fn view() -> AnyPiece {
    column((
        label("Downloads"),
        __SNAKE__("3 new"),
    )).any()
}
```

## Build

```sh
cargo build                 # compiles the library against day on crates.io
```

Composite pieces have no runnable binary of their own — they are verified by compiling and by being
used from an app. Scaffold against the day git remote with `--git`, or against a local day checkout
with `DAY_LOCAL` set / `day new piece … --local <path>`.

## Next steps

- Rename `__SNAKE__` and give it real parameters / builder methods.
- Compose from `row` / `column` / `canvas` / `label` and the `Decorate` modifiers.
- Bind reactive attributes to a `Signal<_>` for live updates.
"#;

// --- NATIVE piece -----------------------------------------------------------

const NATIVE_LIB: &str = r#"//! __CRATE__ — a NATIVE Day piece: a two-way text input realized as a DISTINCT native control per
//! toolkit (NSTextField / GtkEntry / a QLineEdit shim / UITextField / an Android EditText / a WinUI
//! TextBox), registered link-time into each backend's renderer slice without touching day.
//!
//! It is bound **two-way** to a `Signal<String>`: a native edit dispatches `Event::TextChanged` back
//! to Rust which `set`s the signal, and an external signal change patches the control via
//! [`__PASCAL__Patch::SetText`]. A per-build echo guard remembers the last value that arrived FROM the
//! native control so its own change is not written straight back (a feedback loop).
//!
//! ```ignore
//! let text = Signal::new(String::new());
//! __SNAKE__(text).placeholder("Type here…")
//! ```

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_pieces::{IntoText, TextSource};
use day_reactive::{Signal, bind_seeded};
use day_spec::Event;
use std::cell::RefCell;
use std::rc::Rc;

/// The unique piece kind key (every backend renderer registers under the same `kind:`).
pub const KIND: &str = "__KIND__";

/// Full props (build-time realize). Only `text` changes after build (via [`__PASCAL__Patch`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct __PASCAL__Props {
    pub text: String,
    pub placeholder: String,
}

/// The single imperative update: replace the control's text (programmatic sync from the signal).
#[derive(Clone, Debug, PartialEq)]
pub enum __PASCAL__Patch {
    SetText(String),
}

/// A native text input bound two-way to `value`. Configure a prompt with [`__PASCAL__::placeholder`].
pub struct __PASCAL__ {
    value: Signal<String>,
    placeholder: Option<TextSource>,
}

/// `__SNAKE__(value)` — a native text input whose text mirrors `value` in both directions.
pub fn __SNAKE__(value: Signal<String>) -> __PASCAL__ {
    __PASCAL__ {
        value,
        placeholder: None,
    }
}

impl __PASCAL__ {
    /// The empty-state prompt (evaluated once for the initial value; not reactive after build).
    pub fn placeholder<M>(mut self, t: impl IntoText<M>) -> Self {
        self.placeholder = Some(t.into_text());
        self
    }
}

impl Piece for __PASCAL__ {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let __PASCAL__ { value, placeholder } = self;
        let initial = value.get_untracked();
        let ph = placeholder.map(|p| p.initial()).unwrap_or_default();
        let node = cx.leaf(
            KIND,
            &__PASCAL__Props {
                text: initial.clone(),
                placeholder: ph,
            },
            // A text input fills the available width and keeps its natural (single-line) height.
            Flex {
                grow_w: true,
                ..Default::default()
            },
        );
        // Controlled input with origin tracking: the echo guard remembers the last value that arrived
        // FROM the native widget so bind_seeded does not patch that same value straight back.
        let guard: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let g = guard.clone();
        bind_seeded(
            initial,
            move || value.get(),
            move |t: &String| {
                let from_native = g.borrow_mut().take().as_deref() == Some(t.as_str());
                if !from_native {
                    with_tree(|tr| {
                        tr.patch(node, Box::new(__PASCAL__Patch::SetText(t.clone())), false)
                    });
                }
            },
        );
        cx.on(node, move |ev| {
            if let Event::TextChanged(t) = ev {
                *guard.borrow_mut() = Some(t.clone());
                value.set(t.clone());
            }
        });
        node
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers — one file per backend, each registering a `Renderer` link-time into
// its backend's `RENDERERS` slice. `#[cfg]` gates each to its feature + target; `#[path]` keeps the
// files grouped next to lib.rs.
// ---------------------------------------------------------------------------

__MOD_DECLS__
"#;

const APPKIT_IMPL: &str = r#"// AppKit: an editable NSTextField. A per-node delegate implements controlTextDidChange: and dispatches
// Event::TextChanged; programmatic setStringValue does NOT fire the delegate (no echo guard needed on
// this backend — update only writes when the value actually differs).

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

use day_appkit::AppKit;
use day_spec::{NodeId, Proposal, Size};
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSControlTextEditingDelegate, NSTextField, NSTextFieldDelegate, NSView};
use objc2_foundation::{NSNotification, NSObject, NSString};

struct FieldIvars {
    node: NodeId,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "Day__PASCAL__Target"]
    #[ivars = FieldIvars]
    struct FieldTarget;

    unsafe impl NSObjectProtocol for FieldTarget {}
    unsafe impl NSTextFieldDelegate for FieldTarget {}

    unsafe impl NSControlTextEditingDelegate for FieldTarget {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, notification: &NSNotification) {
            let node = self.ivars().node;
            if let Some(obj) = notification.object()
                && let Ok(tf) = obj.downcast::<NSTextField>()
            {
                day_appkit::emit(node, Event::TextChanged(tf.stringValue().to_string()));
            }
        }
    }
);

impl FieldTarget {
    fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FieldIvars { node });
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    // Keep each field's delegate alive for the view's lifetime (the control holds it weakly).
    static TARGETS: RefCell<HashMap<usize, Retained<FieldTarget>>> = RefCell::new(HashMap::new());
}

fn make(backend: &mut AppKit, p: &__PASCAL__Props, id: NodeId) -> Retained<NSView> {
    let mtm = backend.mtm();
    let field = NSTextField::new(mtm);
    if !p.placeholder.is_empty() {
        field.setPlaceholderString(Some(&NSString::from_str(&p.placeholder)));
    }
    field.setStringValue(&NSString::from_str(&p.text));
    let target = FieldTarget::new(mtm, id);
    unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*target))) };
    let ns: Retained<NSView> = Retained::from(<NSTextField as AsRef<NSView>>::as_ref(&field));
    TARGETS.with(|m| {
        m.borrow_mut()
            .insert((ns.as_ref() as *const NSView) as usize, target)
    });
    ns
}

fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &__PASCAL__Patch) {
    let __PASCAL__Patch::SetText(t) = patch;
    if let Some(field) = h.downcast_ref::<NSTextField>()
        && field.stringValue().to_string() != *t
    {
        field.setStringValue(&NSString::from_str(t));
    }
}

fn measure(_backend: &mut AppKit, h: &Retained<NSView>, p: Proposal) -> Size {
    // Grow to the proposed width; natural single-line height.
    let fit = h.fittingSize();
    let w = p.width.unwrap_or(fit.width).max(120.0);
    Size::new(w, fit.height.ceil().max(22.0))
}

day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: __PASCAL__Props, patch: __PASCAL__Patch,
    make: make, update: update, measure: measure);
"#;

const GTK_IMPL: &str = r#"// GTK: a GtkEntry. Its "changed" signal fires on user input AND on programmatic set_text, so a
// per-node `suppress` cell guards the programmatic sync in `update` from echoing back.

use super::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use day_gtk::Gtk;
use day_spec::{NodeId, Proposal, Size};
use gtk4::prelude::*;

struct FieldState {
    entry: gtk4::Entry,
    suppress: Rc<Cell<bool>>,
}

thread_local! {
    static STATE: RefCell<HashMap<usize, FieldState>> = RefCell::new(HashMap::new());
}

fn key(w: &gtk4::Widget) -> usize {
    w.as_ptr() as usize
}

fn make(_backend: &mut Gtk, p: &__PASCAL__Props, id: NodeId) -> gtk4::Widget {
    let entry = gtk4::Entry::new();
    if !p.placeholder.is_empty() {
        entry.set_placeholder_text(Some(&p.placeholder));
    }
    if !p.text.is_empty() {
        entry.set_text(&p.text);
    }
    let suppress = Rc::new(Cell::new(false));
    let sup = suppress.clone();
    entry.connect_changed(move |e| {
        if sup.get() {
            return;
        }
        day_gtk::emit(id, Event::TextChanged(e.text().to_string()));
    });
    let w: gtk4::Widget = entry.clone().upcast();
    STATE.with(|m| {
        m.borrow_mut()
            .insert(key(&w), FieldState { entry, suppress })
    });
    w
}

fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &__PASCAL__Patch) {
    let __PASCAL__Patch::SetText(t) = patch;
    STATE.with(|m| {
        let m = m.borrow();
        let Some(st) = m.get(&key(h)) else {
            return;
        };
        if st.entry.text().as_str() != t {
            st.suppress.set(true);
            st.entry.set_text(t);
            st.suppress.set(false);
        }
    });
}

fn measure(_backend: &mut Gtk, h: &gtk4::Widget, p: Proposal) -> Size {
    let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
    let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
    let w = p.width.unwrap_or(nat_w as f64).max(120.0);
    Size::new(w, (nat_h as f64).max(24.0))
}

day_pieces::renderer!(day_gtk::RENDERERS, Gtk,
    kind: KIND, props: __PASCAL__Props, patch: __PASCAL__Patch,
    make: make, update: update, measure: measure);
"#;

const QT_IMPL: &str = r#"// Qt: this crate's OWN shim (src/lib-qt-shim.cpp) — a QLineEdit behind a flat C ABI. textChanged
// dispatches Event::TextChanged; programmatic setText is wrapped in blockSignals so it never echoes.

use super::*;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

use day_qt::{Qt, QtHandle};
use day_spec::{NodeId, Proposal, Size};

unsafe extern "C" {
    fn day___SNAKE___new(
        placeholder: *const c_char,
        initial: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, *const c_char),
    ) -> *mut c_void;
    fn day___SNAKE___set_text(w: *mut c_void, text: *const c_char);
    // From day-qt-sys (already linked into the binary):
    fn day_qt_size_hint(w: *mut c_void, out_w: *mut f64, out_h: *mut f64);
}

extern "C" fn on_text(id: u64, text: *const c_char) {
    let s = if text.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    };
    day_qt::emit(NodeId(id), Event::TextChanged(s));
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

fn make(_backend: &mut Qt, p: &__PASCAL__Props, id: NodeId) -> QtHandle {
    QtHandle(unsafe {
        day___SNAKE___new(
            cstr(&p.placeholder).as_ptr(),
            cstr(&p.text).as_ptr(),
            id.0,
            on_text,
        )
    })
}

fn update(_backend: &mut Qt, h: &QtHandle, patch: &__PASCAL__Patch) {
    let __PASCAL__Patch::SetText(t) = patch;
    unsafe { day___SNAKE___set_text(h.0, cstr(t).as_ptr()) };
}

fn measure(_backend: &mut Qt, h: &QtHandle, p: Proposal) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_qt_size_hint(h.0, &mut w, &mut hh) };
    let width = p.width.unwrap_or(w).max(120.0);
    Size::new(width, hh.max(24.0))
}

day_pieces::renderer!(day_qt::RENDERERS, Qt,
    kind: KIND, props: __PASCAL__Props, patch: __PASCAL__Patch,
    make: make, update: update, measure: measure);
"#;

const QT_SHIM: &str = r#"// This piece's OWN Qt shim behind a flat C ABI: a QLineEdit. textChanged reports edits back to Rust as
// a UTF-8 C string (valid only during the callback; Rust copies it); programmatic setText is wrapped in
// blockSignals so it never echoes back as a change. Qt libs are already linked by day-qt-sys.

#include <QLineEdit>
#include <QString>

#include <cstdint>

class Day__PASCAL__ : public QLineEdit {
public:
    void setTextGuarded(const QString &t) {
        if (text() != t) {
            blockSignals(true); // programmatic ⇒ no textChanged echo
            setText(t);
            blockSignals(false);
        }
    }
};

extern "C" {

void *day___SNAKE___new(const char *placeholder, const char *initial, uint64_t id,
                        void (*cb)(uint64_t, const char *)) {
    Day__PASCAL__ *w = new Day__PASCAL__();
    w->setPlaceholderText(QString::fromUtf8(placeholder));
    if (initial && *initial)
        w->setText(QString::fromUtf8(initial));
    QObject::connect(w, &QLineEdit::textChanged, [id, cb](const QString &t) {
        QByteArray b = t.toUtf8();
        cb(id, b.constData());
    });
    return w;
}

void day___SNAKE___set_text(void *w, const char *text) {
    static_cast<Day__PASCAL__ *>(w)->setTextGuarded(QString::fromUtf8(text));
}

} // extern "C"
"#;

const UIKIT_IMPL: &str = r#"// UIKit: a UITextField. A per-node target fires on UIControlEvents::EditingChanged and dispatches
// Event::TextChanged; programmatic setText does NOT fire EditingChanged (no echo guard needed here).

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

use day_spec::{NodeId, Proposal, Size};
use day_uikit::Uikit;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_core_foundation::CGSize;
use objc2_foundation::NSString;
use objc2_ui_kit::{UIControlEvents, UITextField, UIView};

struct FieldIvars {
    node: NodeId,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayUIKit__PASCAL__Target"]
    #[ivars = FieldIvars]
    struct FieldTarget;

    unsafe impl NSObjectProtocol for FieldTarget {}

    impl FieldTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, sender: &AnyObject) {
            if let Some(tf) = sender.downcast_ref::<UITextField>() {
                let s = tf.text().map(|s| s.to_string()).unwrap_or_default();
                day_uikit::emit(self.ivars().node, Event::TextChanged(s));
            }
        }
    }
);

impl FieldTarget {
    fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FieldIvars { node });
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    static TARGETS: RefCell<HashMap<usize, Retained<FieldTarget>>> = RefCell::new(HashMap::new());
}

fn make(_backend: &mut Uikit, p: &__PASCAL__Props, id: NodeId) -> Retained<UIView> {
    let mtm = MainThreadMarker::new().unwrap();
    let field = UITextField::new(mtm);
    if !p.placeholder.is_empty() {
        field.setPlaceholder(Some(&NSString::from_str(&p.placeholder)));
    }
    if !p.text.is_empty() {
        field.setText(Some(&NSString::from_str(&p.text)));
    }
    let target = FieldTarget::new(mtm, id);
    unsafe {
        field.addTarget_action_forControlEvents(
            Some(&target),
            sel!(fire:),
            UIControlEvents::EditingChanged,
        );
    }
    let ns: Retained<UIView> = Retained::from(<UITextField as AsRef<UIView>>::as_ref(&field));
    TARGETS.with(|m| {
        m.borrow_mut()
            .insert((ns.as_ref() as *const UIView) as usize, target)
    });
    ns
}

fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &__PASCAL__Patch) {
    let __PASCAL__Patch::SetText(t) = patch;
    if let Some(field) = (**h).downcast_ref::<UITextField>() {
        let cur = field.text().map(|s| s.to_string()).unwrap_or_default();
        if cur != *t {
            field.setText(Some(&NSString::from_str(t)));
        }
    }
}

fn measure(_backend: &mut Uikit, h: &Retained<UIView>, p: Proposal) -> Size {
    let fit = h.sizeThatFits(CGSize::new(1.0e6, 1.0e6));
    let w = p.width.unwrap_or(fit.width).max(120.0);
    Size::new(w, fit.height.ceil().max(28.0))
}

day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
    kind: KIND, props: __PASCAL__Props, patch: __PASCAL__Patch,
    make: make, update: update, measure: measure);
"#;

const ANDROID_IMPL: &str = r#"// Android: an EditText. This crate's OWN Java factory (Day__PASCAL__) is bundled under android/java and
// pulled into the app's Gradle build via [package.metadata.day.android] — no edits to day-android. A
// TextWatcher dispatches edits back to Rust via DayBridge.nativeOnEvent(id, 1, …) (kind 1 = TextChanged).

use super::*;
use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, DayEnv, with_env};
use day_spec::{NodeId, Proposal, Size};

const FIELD_CLASS: &str = "__CLASSPATH__";

fn make(_backend: &mut Android, p: &__PASCAL__Props, id: NodeId) -> AHandle {
    with_env(|env| {
        let ph = env.new_string(&p.placeholder).expect("placeholder");
        let init = env.new_string(&p.text).expect("initial");
        let view = env
            .dcall_static(
                FIELD_CLASS,
                "makeField",
                "(JLjava/lang/String;Ljava/lang/String;)Landroid/view/View;",
                &[
                    JValue::Long(id.0 as i64),
                    JValue::Object(&ph),
                    JValue::Object(&init),
                ],
            )
            .expect("Day__PASCAL__.makeField")
            .l()
            .expect("View");
        AHandle(std::sync::Arc::new(env.new_global_ref(view).expect("global ref")))
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &__PASCAL__Patch) {
    let __PASCAL__Patch::SetText(t) = patch;
    with_env(|env| {
        let s = env.new_string(t).expect("text");
        let _ = env.dcall_static(
            FIELD_CLASS,
            "setFieldText",
            "(Landroid/view/View;Ljava/lang/String;)V",
            &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
        );
    });
}

fn measure(_backend: &mut Android, _h: &AHandle, p: Proposal) -> Size {
    // Fill the proposed width (grow_w leaf); natural single-line height.
    Size::new(p.width.unwrap_or(180.0), p.height.unwrap_or(44.0))
}

day_pieces::renderer!(day_android::RENDERERS, Android,
    kind: KIND, props: __PASCAL__Props, patch: __PASCAL__Patch,
    make: make, update: update, measure: measure);
"#;

const ANDROID_JAVA: &str = r#"// This piece's OWN Android factory — bundled with the crate and pulled into the app's Gradle build
// via [package.metadata.day.android], without touching day-android. It uses only day-android's PUBLIC
// Java surface: DayBridge.ctx (the Android Context) and DayBridge.nativeOnEvent (the event trampoline).
package __PKG_DOTS__;

import android.text.Editable;
import android.text.InputType;
import android.text.TextWatcher;
import android.view.View;
import android.widget.EditText;

import dev.daybrite.day.bridge.DayBridge;

public final class Day__PASCAL__ {
    // A single-line EditText. Every edit reports back via DayBridge.nativeOnEvent kind 1 (TextChanged).
    public static View makeField(final long id, String placeholder, String initial) {
        EditText e = new EditText(DayBridge.ctx);
        e.setSingleLine(true);
        e.setInputType(InputType.TYPE_CLASS_TEXT);
        e.setHint(placeholder);
        if (initial != null && !initial.isEmpty()) {
            e.setText(initial);
            e.setSelection(initial.length());
        }
        e.addTextChangedListener(new TextWatcher() {
            public void afterTextChanged(Editable s) {
                DayBridge.nativeOnEvent(id, 1, 0, s.toString());
            }
            public void beforeTextChanged(CharSequence s, int a, int b, int c) {}
            public void onTextChanged(CharSequence s, int a, int b, int c) {}
        });
        return e;
    }

    // Programmatic sync from the bound signal. Guard on equality so setting the same text is a no-op.
    public static void setFieldText(View v, String text) {
        EditText e = (EditText) v;
        if (!e.getText().toString().equals(text)) {
            e.setText(text);
            e.setSelection(text.length());
        }
    }
}
"#;

const WINUI_IMPL: &str = r#"// WinUI: this crate's OWN C++/WinRT shim (src/lib-winui-shim.cpp) — a TextBox boxed into a Day handle
// via the day_winui_box/unbox seam that day-winui-sys exports. Windows-only; built in CI, not verified
// on non-Windows hosts.

use super::*;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

use day_spec::{NodeId, Proposal, Size};
use day_winui::{WinHandle, WinUi};

unsafe extern "C" {
    fn day___SNAKE___winui_new(
        placeholder: *const c_char,
        initial: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, *const c_char),
    ) -> *mut c_void;
    fn day___SNAKE___winui_set_text(w: *mut c_void, text: *const c_char);
    // Generic size hint from day-winui-sys (already linked).
    fn day_winui_measure(
        w: *mut c_void,
        avail_w: f64,
        avail_h: f64,
        out_w: *mut f64,
        out_h: *mut f64,
    );
}

extern "C" fn on_text(id: u64, text: *const c_char) {
    let s = if text.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    };
    day_winui::emit(NodeId(id), Event::TextChanged(s));
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

fn make(_backend: &mut WinUi, p: &__PASCAL__Props, id: NodeId) -> WinHandle {
    WinHandle(unsafe {
        day___SNAKE___winui_new(
            cstr(&p.placeholder).as_ptr(),
            cstr(&p.text).as_ptr(),
            id.0,
            on_text,
        )
    })
}

fn update(_backend: &mut WinUi, h: &WinHandle, patch: &__PASCAL__Patch) {
    let __PASCAL__Patch::SetText(t) = patch;
    unsafe { day___SNAKE___winui_set_text(h.0, cstr(t).as_ptr()) };
}

fn measure(_backend: &mut WinUi, h: &WinHandle, p: Proposal) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_winui_measure(h.0, -1.0, -1.0, &mut w, &mut hh) };
    let width = p.width.unwrap_or(w).max(160.0);
    Size::new(width, hh.max(32.0))
}

day_pieces::renderer!(day_winui::RENDERERS, WinUi,
    kind: KIND, props: __PASCAL__Props, patch: __PASCAL__Patch,
    make: make, update: update, measure: measure);
"#;

const WINUI_SHIM: &str = r#"// This piece's OWN C++/WinRT shim — a TextBox boxed into a Day handle via the day_winui_box/unbox seam
// that day-winui-sys exports. TextChanged reports edits back to Rust as a UTF-8 C string; programmatic
// Text(...) is guarded so it only re-writes on a real change. Windows-only; compiled by build.rs.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>

#include <windows.h>

#include <cstdint>
#include <string>

using namespace winrt;
namespace WUX = winrt::Windows::UI::Xaml;
namespace WUXC = winrt::Windows::UI::Xaml::Controls;

// The boxing seam, exported by day-winui-sys (already linked into the app).
extern "C" void *day_winui_box(void *iinspectable_abi);
extern "C" void *day_winui_unbox(void *handle);

static winrt::hstring hs(const char *s) {
    if (!s || !*s)
        return winrt::hstring{};
    int len = MultiByteToWideChar(CP_UTF8, 0, s, -1, nullptr, 0);
    if (len <= 1)
        return winrt::hstring{};
    std::wstring w(static_cast<size_t>(len - 1), L'\0');
    MultiByteToWideChar(CP_UTF8, 0, s, -1, w.data(), len);
    return winrt::hstring{w};
}

static std::string to_utf8(winrt::hstring const &h) {
    if (h.empty())
        return std::string{};
    int len = WideCharToMultiByte(CP_UTF8, 0, h.c_str(), -1, nullptr, 0, nullptr, nullptr);
    if (len <= 1)
        return std::string{};
    std::string s(static_cast<size_t>(len - 1), '\0');
    WideCharToMultiByte(CP_UTF8, 0, h.c_str(), -1, s.data(), len, nullptr, nullptr);
    return s;
}

extern "C" {

void *day___SNAKE___winui_new(const char *placeholder, const char *initial, uint64_t id,
                              void (*cb)(uint64_t, const char *)) {
    WUXC::TextBox box;
    box.PlaceholderText(hs(placeholder));
    if (initial && *initial)
        box.Text(hs(initial));
    // The TextChanged delegate's sender is IInspectable (NOT DependencyObject) — cppwinrt
    // reconstructs it as such and can't downcast to a narrower type, so declaring anything else
    // fails the delegate's noexcept Invoke to compile. Query the TextBox back out of it.
    box.TextChanged([id, cb](winrt::Windows::Foundation::IInspectable const &s,
                             WUXC::TextChangedEventArgs const &) {
        if (auto tb = s.try_as<WUXC::TextBox>()) {
            std::string t = to_utf8(tb.Text());
            cb(id, t.c_str());
        }
    });
    return day_winui_box(winrt::get_abi(box));
}

void day___SNAKE___winui_set_text(void *handle, const char *text) {
    WUX::UIElement e{nullptr};
    winrt::copy_from_abi(e, day_winui_unbox(handle));
    if (auto box = e.try_as<WUXC::TextBox>()) {
        auto nt = hs(text);
        if (box.Text() != nt)
            box.Text(nt);
    }
}

} // extern "C"
"#;

const BUILD_RS: &str = r#"//! Compiles this piece's OWN native shims when their feature is on — a native Day piece carrying C++
//! without touching Day's toolkit crates. Qt uses `cc` + pkg-config; WinUI uses `cc` (MSVC) + the
//! Windows SDK cppwinrt projection, mirroring day-winui-sys.

fn main() {
    println!("cargo:rerun-if-changed=src/lib-qt-shim.cpp");
    println!("cargo:rerun-if-changed=src/lib-winui-shim.cpp");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_FEATURE_QT").is_ok() {
        build_qt();
    }
    // Windows-only, and only when the app targets WinUI.
    if std::env::var("CARGO_FEATURE_WINUI").is_ok() && std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        build_winui();
    }
}

fn build_qt() {
    let cflags = std::process::Command::new("pkg-config")
        .args(["--cflags", "Qt6Widgets"])
        .output()
        .expect("pkg-config Qt6Widgets");
    let mut build = cc::Build::new();
    build.cpp(true).std("c++17").file("src/lib-qt-shim.cpp");
    for tok in String::from_utf8_lossy(&cflags.stdout).split_whitespace() {
        build.flag(tok);
    }
    build.flag_if_supported("-Wno-unused-parameter");
    build.compile("day__SNAKE__qtshim");
    // Qt libs themselves are already linked by day-qt-sys.
}

fn build_winui() {
    // Shared, env-overridable lookup (DAY_CPPWINRT / DAY_WINDOWS_KITS_ROOT / WindowsSdkDir —
    // docs/environment.md); also emits the matching rerun-if-env-changed lines.
    let cppwinrt = day_toolchain::cppwinrt_include_for_build_script().expect(
        "Windows 10/11 SDK cppwinrt headers not found. Install the Windows SDK \
         (Visual Studio 'Desktop development with C++'), or point DAY_CPPWINRT / \
         DAY_WINDOWS_KITS_ROOT at a relocated install.",
    );
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++20")
        .define("_SILENCE_EXPERIMENTAL_COROUTINE_DEPRECATION_WARNINGS", None)
        .file("src/lib-winui-shim.cpp")
        .include(&cppwinrt)
        .flag("/EHsc")
        .flag("/bigobj")
        .flag_if_supported("/permissive-");
    build.compile("day__SNAKE__winuishim");
    // WindowsApp.lib + the day_winui_box/unbox seam are already linked by day-winui-sys.
}

"#;

const NATIVE_README: &str = r#"# __CRATE__

A **native** Day piece: a two-way text input realized as a distinct native control per toolkit,
registered link-time into each backend's renderer slice without touching day.

## Use

Add it as a dependency (versioned, from crates.io by default) and call the builder from your app. Because
it declares its backends in `[package.metadata.day.piece]`, `day` unions `<pkg>/<backend>` into the app
build automatically — you never re-list the per-backend features:

```rust
use day::prelude::*;
use __CRATE_IDENT__::__SNAKE__;

fn view() -> AnyPiece {
    let text = Signal::new(String::new());
    __SNAKE__(text).placeholder("Type here…").any()
}
```

## Build a single backend

```sh
cargo build --features appkit    # or gtk / qt / uikit / widget / winui
```

- `appkit` / `uikit` build on macOS with the iOS-sim target respectively.
- `qt` / `winui` compile a small C++ shim (`build.rs`).
- `widget` carries its own Java factory under `android/java` (staged into the app's Gradle build).

## Next steps

- Rename the `__PASCAL__` type / `__SNAKE__` builder and adjust `__PASCAL__Props` / `__PASCAL__Patch`.
- Wire your control's real events in each `src/lib-<backend>.rs`.
- Drop any backends you don't need from `[features]` and `[package.metadata.day.piece]`.
"#;

// --- PART -------------------------------------------------------------------

const PART_LIB: &str = r#"//! __CRATE__ — a HEADLESS Day part: a cross-platform capability with no UI. Any Rust code can depend on
//! this crate and call [`status`] to read a snapshot through the platform's NATIVE API.
//!
//! ```no_run
//! if let Some(s) = __CRATE_IDENT__::status() {
//!     println!("value = {}", s.value);
//! }
//! ```
//!
//! Platform selection is purely `#[cfg(target_os)]` (a capability is an OS concern, not a widget-toolkit
//! one), so there are no backend features — it "just works" per target. Platforms without an impl return
//! `None`.

/// A sample snapshot. Replace `value` with your capability's real fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// A stand-in reading. Replace with the real data your part exposes.
    pub value: i64,
}

/// Read the current snapshot via the platform's native API, or `None` where unsupported.
pub fn status() -> Option<Sample> {
    imp::status()
}

// ---------------------------------------------------------------------------
// Per-OS implementations. Each exposes `fn status() -> Option<Sample>`.
// ---------------------------------------------------------------------------

__CFG_MODS__

// Any other platform: no native API. (MANDATORY catch-all — keeps the crate building everywhere.)
#[cfg(not(any(
    __NOT_ANY__
)))]
mod imp {
    pub fn status() -> Option<super::Sample> {
        None
    }
}

#[cfg(test)]
mod tests {
    // Reading must never panic, whatever the host.
    #[test]
    fn status_does_not_panic() {
        let _ = super::status();
    }
}
"#;

const PART_ANDROID: &str = r#"// Android: read through this crate's OWN Java shim (android/java/…/Day__PASCAL__.java) — staged into the
// app's Gradle build by `day build` via [package.metadata.day.android], without touching day-android
// (it registers NO renderer). The Java uses day-android's cached Context (DayBridge.ctx); Rust calls it
// through day-android's re-exported `jni`.

use day_android::{DayEnv, with_env};

const CLASS: &str = "__CLASSPATH__";

pub fn status() -> Option<super::Sample> {
    let value: i64 = with_env(|env| {
        env.dcall_static(CLASS, "read", "()J", &[])
            .ok()
            .and_then(|v| v.j().ok())
    })?;
    if value < 0 {
        return None; // -1 = unavailable (no Context / capability)
    }
    Some(super::Sample { value })
}
"#;

const PART_ANDROID_JAVA: &str = r#"// __CRATE__'s OWN Android backend — a headless capability shim (no UI), bundled with this crate and
// folded into the app's Gradle build via [package.metadata.day.android], without touching day-android.
package __PKG_DOTS__;

public final class Day__PASCAL__ {
    private Day__PASCAL__() {}

    /**
     * Returns a sample reading, or -1 when unavailable. Replace the body with a real native reading —
     * the Android Context is available as dev.daybrite.day.bridge.DayBridge.ctx.
     */
    public static long read() {
        return 42L;
    }
}
"#;

const PART_EXAMPLE: &str = r#"// A tiny driver: `cargo run --example __SNAKE__`.
fn main() {
    match __CRATE_IDENT__::status() {
        Some(s) => println!("__SNAKE__ sample: value = {}", s.value),
        None => println!("__SNAKE__: unavailable on this platform"),
    }
}
"#;

const PART_README: &str = r#"# __CRATE__

A **headless** Day part: a cross-platform capability with no UI. Any Rust code can depend on it and call
`status()`; platform selection is purely `#[cfg(target_os)]`, so there are no backend features.

## Use

```rust
if let Some(s) = __CRATE_IDENT__::status() {
    println!("value = {}", s.value);
}
```

## Build & run

```sh
cargo build                    # host platform
cargo run --example __SNAKE__   # prints a sample reading
```

Each `src/<os>.rs` is a stub returning a sample `Sample { value: 42 }`. Android reads through a bundled
Java shim (`android/java/…/Day__PASCAL__.java`) that `day build` stages into the app's Gradle build.

## Next steps

- Replace `Sample`'s fields with your capability's real data.
- Fill in each `src/<os>.rs` with the platform's native API (add per-platform deps to `Cargo.toml`).
- The catch-all `mod imp { fn status() -> None }` fallback keeps the crate compiling on every target —
  keep it.
"#;

#[cfg(test)]
mod tests {
    use super::add_targets_to_day_toml;

    #[test]
    fn day_toml_append_preserves_comments_and_formatting() {
        let input = "# my app\nschema = 1\n\n[app]\nid = \"dev.example.foo\"   # bundle id\n# the platforms we ship on\ntargets = [\"ios-uikit\", \"macos-appkit\"]\n\n[window]\nwidth = 960\n";
        let out = add_targets_to_day_toml(input, &["android-widget"]).unwrap();
        let expected = "# my app\nschema = 1\n\n[app]\nid = \"dev.example.foo\"   # bundle id\n# the platforms we ship on\ntargets = [\"ios-uikit\", \"macos-appkit\", \"android-widget\"]\n\n[window]\nwidth = 960\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn day_toml_append_preserves_multiline_array_style() {
        let input = "schema = 1\n[app]\nid = \"x\"\ntargets = [\n  \"ios-uikit\",\n]\n";
        let out = add_targets_to_day_toml(input, &["linux-gtk"]).unwrap();
        let doc: toml_edit::DocumentMut = out.parse().unwrap();
        let arr = doc["app"]["targets"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn day_toml_append_creates_missing_array() {
        let out =
            add_targets_to_day_toml("schema = 1\n[app]\nid = \"x\"\n", &["macos-qt"]).unwrap();
        let doc: toml_edit::DocumentMut = out.parse().unwrap();
        assert_eq!(doc["app"]["targets"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn day_toml_wrong_shape_is_rejected() {
        assert!(
            add_targets_to_day_toml("schema = 1\n[app]\ntargets = 3\n", &["macos-qt"]).is_err()
        );
        assert!(add_targets_to_day_toml("not [ valid toml", &["macos-qt"]).is_err());
    }
}
