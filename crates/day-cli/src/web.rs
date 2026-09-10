// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! web-dom build + launch (DESIGN.md §9, docs/web.md). Build compiles the app's lib crate as a
//! wasm32 cdylib and assembles a self-contained `dist/` — host page + shim + stylesheet
//! (embedded from `resources/web/` at CLI compile time), the wasm module, bundled
//! images, and fonts with a `fonts.json` manifest the shim pre-loads. Launch serves `dist/`
//! over loopback (browsers won't instantiate wasm from `file:`) and opens the default browser.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::Profile;
use crate::meta::Project;
use crate::ops::{BuildOutcome, LaunchSpec, apply_app_identity, feature_selection, status};
use crate::targets::Target;

// The host page trio, embedded so an installed CLI needs no source checkout. They live INSIDE
// this crate (not next to `toolkits/day-dom`, whose `extern "C"` block shim.js implements)
// because `include_str!` may not reach outside the package: `cargo package` copies only this
// directory, so a path into the workspace vanishes on crates.io and `cargo install day-cli`
// fails to compile — which is exactly what shipped in 0.0.15. Editing shim.js means rebuilding
// the CLI, and `day-dom`'s crate docs point here.
const HOST_INDEX: &str = include_str!("../resources/web/index.html");
const HOST_SHIM: &str = include_str!("../resources/web/shim.js");
const HOST_CSS: &str = include_str!("../resources/web/day.css");
// The day-sql worker page (docs/persistence.md): SQLite over OPFS sync access handles,
// serving the main thread over the SharedArrayBuffer channel. Embedded like the shim, and
// only useful because the server sends COOP/COEP (SharedArrayBuffer needs isolation).
const HOST_SQL_WORKER: &str = include_str!("../resources/web/day-sql-worker.js");
// The DAY_WEB_DRIVER page-driver (`day web driver`, docs/web.md): the Playwright browser
// day-cli spawns for scripted web runs. Embedded like the shim, so the driver protocol
// (screenshot + quit on the control port) always matches the CLI that speaks it — CI installs
// playwright, asks `day web driver` for this script's path, and sets DAY_WEB_DRIVER to it.
const WEB_DRIVER: &str = include_str!("../resources/web/webdom-driver.mjs");

/// Write the bundled DAY_WEB_DRIVER script to a version-stamped temp location and return its
/// path (`day web driver`). Idempotent: same version, same path, rewritten only on drift.
pub fn materialize_driver() -> Result<std::path::PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("day-webdriver-{}", env!("CARGO_PKG_VERSION")));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join("webdom-driver.mjs");
    let fresh = std::fs::read_to_string(&path).is_ok_and(|s| s == WEB_DRIVER);
    if !fresh {
        std::fs::write(&path, WEB_DRIVER).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(path)
}

pub fn build_web(
    project: &Project,
    target: &'static Target,
    profile: Profile,
    start: std::time::Instant,
) -> Result<BuildOutcome, String> {
    let name = &project.manifest.app.name;
    let features = feature_selection(project, target.toolkit);
    let cargo_dir = crate::ops::cargo_dir(project, target, profile);

    let mut cmd = Command::new("cargo");
    crate::patch::apply_day_src(&mut cmd);
    cmd.current_dir(&project.root)
        .env("CARGO_TARGET_DIR", &cargo_dir)
        // The app's lib as a cdylib (the same shape as Android/HarmonyOS): `day_start_web!` exports
        // `day_dom_main`, which the host page calls after instantiation.
        .args(["rustc", "-p", name, "--lib", "--no-default-features"])
        .args(["--features", &features])
        .args(["--target", "wasm32-unknown-unknown"]);
    apply_app_identity(&mut cmd, project);
    crate::bridge::apply_staged(&mut cmd, project, "web-dom");
    if profile == Profile::Release {
        cmd.arg("--release");
    }
    cmd.args(["--crate-type", "cdylib"]);
    // A `persistence` app compiles the bundled SQLite through cc-rs, whose default `clang`
    // cannot emit wasm on a Mac (Apple's has no wasm32 backend). Resolve a capable compiler
    // the same way doctor reports it and export it; a set CC variable is cc-rs's to honor,
    // and a Missing resolution is not an error here — a UI-only app compiles no C at all.
    if let day_toolchain::WasmCc::Fallback(cc) = day_toolchain::wasm_cc() {
        status("Using", &format!("{} (wasm32 C compiler)", cc.display()));
        cmd.env("CC_wasm32_unknown_unknown", &cc);
    }
    // Route entropy to day-dom's getrandom bridge (docs/web.md): the raw-wasm pipeline has no
    // wasm-bindgen runtime, so getrandom v0.3's own `wasm_js` backend cannot work here — the
    // `custom` backend cfg points it at `__getrandom_v03_custom`, which day-dom answers from
    // the shim's `crypto.getRandomValues`. APPENDED to an inherited RUSTFLAGS (CI sets one);
    // inert for app graphs that never pull getrandom.
    let mut rustflags = std::env::var("RUSTFLAGS").unwrap_or_default();
    if !rustflags.is_empty() {
        rustflags.push(' ');
    }
    rustflags.push_str("--cfg getrandom_backend=\"custom\"");
    cmd.env("RUSTFLAGS", rustflags);
    if profile == Profile::Release {
        // Debug symbols are most of a release wasm's bytes and no browser tool reads them
        // from a stripped build; keep the shipped module small.
        cmd.args(["--", "-Cstrip=symbols"]);
    } else {
        // Debug keeps the NAME section (browser backtraces read it) but drops DWARF, which
        // no browser reads and which multiplies the module several times over — the page AND
        // the day-sql worker each decode this module, so its size is boot time twice.
        //
        // KNOWN LIMIT: a DEBUG persistence app can die opening its store on WebKit only —
        // "RangeError: Maximum call stack size exceeded" from the day-sql worker. Unoptimized
        // SQLite open recurses deeper than WebKit's machine stack allows for wasm frames (a
        // VM budget `-zstack-size` cannot raise; Chromium copes, release fits everywhere).
        // Scripted WebKit runs should build `--profile release`, as CI does.
        cmd.args(["--", "-Cstrip=debuginfo"]);
    }
    status("Building", &format!("{} ({profile})", target.name));
    let out = cmd.status().map_err(|e| format!("cargo: {e}"))?;
    if !out.success() {
        return Err(format!(
            "cargo build failed for {} — see the cargo output above; `day doctor --toolkit dom` \
             checks the toolchain (the wasm32-unknown-unknown rustup target; with `persistence`, \
             a wasm-capable clang)",
            target.name
        ));
    }

    // Assemble dist/. The wasm artifact is named after the LIB TARGET — `dayapp.wasm` for a
    // scaffolded app, whose `[lib] name` is pinned to that constant (DESIGN.md §17.5). Deriving
    // it from the PACKAGE name is what broke this build when the pin landed.
    let wasm = cargo_dir
        .join("wasm32-unknown-unknown")
        .join(profile.as_str())
        .join(format!("{}.wasm", project.lib_name()));
    let dist = cargo_dir.join("dist");
    std::fs::create_dir_all(&dist).map_err(|e| format!("dist dir: {e}"))?;
    std::fs::write(dist.join("shim.js"), HOST_SHIM).map_err(|e| format!("shim: {e}"))?;
    // daybridge web arms (docs/bridge.md): one ES module per bridged crate, listed for the shim to
    // import and merge into the wasm imports before instantiation.
    let bridges = crate::bridge::write_js(&dist, &crate::bridge::stage(project, "web"))?;
    std::fs::write(dist.join("day.css"), HOST_CSS).map_err(|e| format!("css: {e}"))?;
    std::fs::write(dist.join("day-sql-worker.js"), HOST_SQL_WORKER)
        .map_err(|e| format!("sql worker: {e}"))?;
    std::fs::copy(&wasm, dist.join("app.wasm")).map_err(|e| format!("{}: {e}", wasm.display()))?;

    // Vector glyphs (docs/vectors.md): the SVG is what day-dom asks for and what the browser
    // renders at display size, so only the raster FALLBACKS land beside the images — art the
    // vector pipeline could not express. Every browser that can run a wasm app renders SVG, so
    // a second PNG copy of a convertible glyph is weight that would also hide a broken vector
    // path behind art that still looks right. The page learns which names are vectors via
    // `window.__DAY_VECTORS` (injected into index.html below), read back through the shim's
    // `vector:` env keys.
    let mut vector_names: Vec<String> = Vec::new();
    let vectors_cache = crate::resources::vector_fallback_dir(project, target.toolkit);
    if vectors_cache.is_dir() {
        let images = dist.join("assets/images");
        std::fs::create_dir_all(&images).map_err(|e| format!("images dir: {e}"))?;
        for entry in std::fs::read_dir(&vectors_cache)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let p = entry.path();
            if p.extension().and_then(|x| x.to_str()) == Some("png")
                && let Some(name) = p.file_name()
            {
                std::fs::copy(&p, images.join(name)).map_err(|e| format!("vector copy: {e}"))?;
            }
        }
    }
    let svg_cache = crate::resources::vector_svg_dir(project);
    if svg_cache.is_dir() {
        let images = dist.join("assets/images");
        std::fs::create_dir_all(&images).map_err(|e| format!("images dir: {e}"))?;
        for entry in std::fs::read_dir(&svg_cache)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let p = entry.path();
            if p.extension().and_then(|x| x.to_str()) == Some("svg")
                && let (Some(name), Some(stem)) =
                    (p.file_name(), p.file_stem().and_then(|s| s.to_str()))
            {
                std::fs::copy(&p, images.join(name)).map_err(|e| format!("vector copy: {e}"))?;
                vector_names.push(stem.to_string());
            }
        }
        vector_names.sort();
    }
    // Resource names are `[a-z0-9_]` (day-build enforces this), so plain quoting is JS-safe.
    let vectors_json = vector_names
        .iter()
        .map(|n| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(",");
    let bridges_json = bridges
        .iter()
        .map(|m| format!("\"{m}\""))
        .collect::<Vec<_>>()
        .join(",");
    // The home-screen set (docs/web.md "Home screen and offline"): the icons the manifest
    // names, the manifest itself, and the head tags that point at both.
    let home = home_screen(project);
    let icons = stage_home_icons(project, &dist)?;
    std::fs::write(
        dist.join("manifest.webmanifest"),
        manifest_json(&home, &icons),
    )
    .map_err(|e| format!("manifest: {e}"))?;
    std::fs::write(
        dist.join("index.html"),
        HOST_INDEX
            .replacen(
                "lang=\"en\"",
                &format!("lang=\"{}\"", attr_escape(&home.lang)),
                1,
            )
            .replace("<!--day:head-->", &head_tags(&home, &icons))
            .replace("[/*day:vectors*/]", &format!("[{vectors_json}]"))
            .replace("[/*day:bridges*/]", &format!("[{bridges_json}]")),
    )
    .map_err(|e| format!("index: {e}"))?;
    // Bundled images, flat under assets/images/ — the paths day-dom writes into `src` attrs.
    let images_src = project.root.join("resource/images");
    if images_src.is_dir() {
        let images = dist.join("assets/images");
        std::fs::create_dir_all(&images).map_err(|e| format!("images dir: {e}"))?;
        for f in std::fs::read_dir(&images_src)
            .map_err(|e| format!("images: {e}"))?
            .flatten()
        {
            if f.path().is_file() {
                std::fs::copy(f.path(), images.join(f.file_name()))
                    .map_err(|e| format!("{}: {e}", f.path().display()))?;
            }
        }
    }

    // Bundled data assets, the whole TREE (§18.5), under assets/data/ — same-origin URLs for
    // anything that browses them (the inline web view's `assets/data/<site>/…` base above all).
    let data_src = project.root.join("resource/assets");
    if data_src.is_dir() {
        crate::pack::copy_tree(&data_src, &dist.join("assets/data"))?;
    }

    // Bundled fonts + the fonts.json manifest (family name from the font's own name table, the
    // same resolution day-build codegen uses) — the shim registers each FontFace before the
    // first layout so custom families measure correctly.
    let fonts = crate::resources::scan_fonts(project)?;
    if !fonts.is_empty() {
        let dir = dist.join("assets/fonts");
        std::fs::create_dir_all(&dir).map_err(|e| format!("fonts dir: {e}"))?;
        let mut manifest = String::from("[");
        for (i, f) in fonts.iter().enumerate() {
            let staged = f.staged_name();
            std::fs::copy(&f.path, dir.join(&staged))
                .map_err(|e| format!("{}: {e}", f.path.display()))?;
            if i > 0 {
                manifest.push(',');
            }
            manifest.push_str(&format!(
                "{{\"family\":\"{}\",\"url\":\"assets/fonts/{staged}\"}}",
                f.family.replace('"', "\\\"")
            ));
        }
        manifest.push(']');
        std::fs::write(dir.join("fonts.json"), manifest).map_err(|e| format!("fonts.json: {e}"))?;
    }

    // LAST, once every file is in place: the service worker's precache list is the dist's
    // file list, and its cache version is a digest of their bytes, so a rebuild with any
    // change installs a fresh cache and drops the old one.
    let files = dist_files(&dist)?;
    std::fs::write(dist.join("sw.js"), service_worker(&files))
        .map_err(|e| format!("sw.js: {e}"))?;

    Ok(BuildOutcome {
        target: target.name,
        artifact: dist,
        seconds: start.elapsed().as_secs_f64(),
    })
}

// ---------------------------------------------------------------------------
// Home screen and offline (docs/web.md): the web app manifest, the icon set, the head tags,
// and the service worker — what makes "Add to Home Screen" install a real app: its own name
// and icon, no browser chrome, and a launch that works with the network away.
// ---------------------------------------------------------------------------

/// The texts and colors the manifest and the head carry. Assembled from the store listing
/// (the same name and short description the stores show), `[app]`, and `[web]`.
#[derive(Debug, Clone, PartialEq)]
pub struct HomeScreen {
    pub name: String,
    pub short_name: String,
    pub description: Option<String>,
    pub lang: String,
    pub theme_color: String,
    pub theme_color_dark: String,
    pub background_color: String,
    pub display: crate::meta::WebDisplay,
}

/// The icon sizes the dist ships under `icons/icon-<px>.png`, from the png family `day icon`
/// renders: 64 for the tab favicon, 192 and 512 for the manifest (the sizes Chrome requires
/// for an install), 192 doubling as the apple-touch-icon (iOS scales it).
const HOME_ICON_SIZES: [u32; 3] = [64, 192, 512];

/// The default light and dark chrome colors, where `[web]` names none.
const DEFAULT_THEME_LIGHT: &str = "#ffffff";
const DEFAULT_THEME_DARK: &str = "#000000";

/// Read the home-screen texts and colors for `project`.
pub fn home_screen(project: &Project) -> HomeScreen {
    let app = &project.manifest.app;
    let web = &project.manifest.web;
    let lang = crate::store::default_locale(&crate::store::app_locales(project))
        .unwrap_or_else(|| "en".to_string());
    let listing = match crate::store::read(project) {
        Ok(l) => l,
        Err(e) => {
            status(
                "Warning",
                &format!("store listing not read ({e}); the web manifest takes [app] title"),
            );
            Default::default()
        }
    };
    let field = |f: crate::store::Field| -> Option<String> {
        listing
            .locales
            .get(&lang)
            .and_then(|m| m.get(&f))
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
    };
    let name = field(crate::store::Field::Name)
        .or_else(|| app.title.clone())
        .unwrap_or_else(|| app.name.clone());
    let short_name = web
        .short_name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| short_name_of(&name));
    let description =
        field(crate::store::Field::Short).or_else(|| field(crate::store::Field::Subtitle));
    let color = |v: &Option<String>, default: &str| -> String {
        match v.as_deref().map(str::trim) {
            Some(c) if is_hex_color(c) => c.to_ascii_lowercase(),
            Some(c) => {
                status(
                    "Warning",
                    &format!("[web] color {c:?} is not #rrggbb; using {default}"),
                );
                default.to_string()
            }
            None => default.to_string(),
        }
    };
    let theme_color = color(&web.theme_color, DEFAULT_THEME_LIGHT);
    let theme_color_dark = color(&web.theme_color_dark, DEFAULT_THEME_DARK);
    let background_color = color(&web.background_color, &theme_color);
    HomeScreen {
        name,
        short_name,
        description,
        lang,
        theme_color,
        theme_color_dark,
        background_color,
        display: web.display,
    }
}

/// A `#rrggbb` color.
fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// The name under a home-screen icon: the whole name when it fits twelve characters (the
/// width launchers show before clipping), else its leading words that do, else its first
/// twelve characters.
pub fn short_name_of(name: &str) -> String {
    const MAX: usize = 12;
    let name = name.trim();
    if name.chars().count() <= MAX {
        return name.to_string();
    }
    let mut out = String::new();
    for word in name.split_whitespace() {
        let next = if out.is_empty() {
            word.to_string()
        } else {
            format!("{out} {word}")
        };
        if next.chars().count() > MAX {
            break;
        }
        out = next;
    }
    if out.is_empty() {
        out = name.chars().take(MAX).collect();
    }
    out
}

/// Copy the manifest's icon sizes from the rendered png family into `dist/icons/`, rendering
/// the family first when a size is missing (an older lock predates 192). Returns the sizes
/// staged; a project without an icon master stages none and the manifest lists none.
fn stage_home_icons(project: &Project, dist: &Path) -> Result<Vec<u32>, String> {
    let png = |px: u32| {
        project
            .root
            .join(crate::icon::HOST_DIR)
            .join("png")
            .join(format!("day-icon-{px}.png"))
    };
    if HOME_ICON_SIZES.iter().any(|&px| !png(px).is_file()) {
        let opts = crate::icon::IconOptions {
            master: None,
            check: false,
            platforms: vec!["web-dom".to_string()],
        };
        // No master is the one legitimate miss (ensure() said so once already); anything
        // else is a real error.
        match crate::icon::run(project, &opts) {
            Ok(_) => {}
            Err(crate::icon::IconError::Other(e)) if e.contains("master") => {}
            Err(crate::icon::IconError::Other(e)) => return Err(format!("icons: {e}")),
            Err(crate::icon::IconError::Drift(lines)) => {
                return Err(format!("icons: {}", lines.join("; ")));
            }
        }
    }
    let mut staged = Vec::new();
    let dir = dist.join("icons");
    for px in HOME_ICON_SIZES {
        let src = png(px);
        if !src.is_file() {
            continue;
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("icons dir: {e}"))?;
        std::fs::copy(&src, dir.join(format!("icon-{px}.png")))
            .map_err(|e| format!("{}: {e}", src.display()))?;
        staged.push(px);
    }
    Ok(staged)
}

/// The web app manifest (W3C Web Application Manifest) for the dist. Every URL is relative
/// to the manifest's own location, so the same file serves from a Pages root, a project
/// subpath, or a site's `webapp/` directory; `id` is left to its default (the resolved
/// `start_url`), which is what a hosting site's own manifest points back at.
pub fn manifest_json(home: &HomeScreen, icons: &[u32]) -> String {
    let mut icon_list: Vec<serde_json::Value> = icons
        .iter()
        .map(|px| {
            serde_json::json!({
                "src": format!("icons/icon-{px}.png"),
                "sizes": format!("{px}x{px}"),
                "type": "image/png",
                "purpose": "any",
            })
        })
        .collect();
    // The master's background layer fills the whole square (docs/icons.md), so the same
    // render is a valid maskable icon: launchers may crop it to any shape.
    if icons.contains(&512) {
        icon_list.push(serde_json::json!({
            "src": "icons/icon-512.png",
            "sizes": "512x512",
            "type": "image/png",
            "purpose": "maskable",
        }));
    }
    let mut m = serde_json::Map::new();
    m.insert("name".into(), home.name.clone().into());
    m.insert("short_name".into(), home.short_name.clone().into());
    if let Some(d) = &home.description {
        m.insert("description".into(), d.clone().into());
    }
    m.insert("lang".into(), home.lang.clone().into());
    m.insert("dir".into(), "auto".into());
    m.insert("start_url".into(), "./".into());
    m.insert("scope".into(), "./".into());
    m.insert("display".into(), home.display.as_str().into());
    m.insert(
        "background_color".into(),
        home.background_color.clone().into(),
    );
    m.insert("theme_color".into(), home.theme_color.clone().into());
    m.insert("icons".into(), serde_json::Value::Array(icon_list));
    let mut out = serde_json::to_string_pretty(&serde_json::Value::Object(m))
        .unwrap_or_else(|_| "{}".to_string());
    out.push('\n');
    out
}

/// The `<head>` lines that make the page installable and name it: the title, the manifest
/// link, the theme colors (one per appearance), the icons, and the iOS home-screen metas that
/// predate the manifest and are still what Safari reads for the status bar and the title.
pub fn head_tags(home: &HomeScreen, icons: &[u32]) -> String {
    let mut h = String::new();
    let line = |h: &mut String, l: &str| {
        h.push_str(l);
        h.push('\n');
        h.push_str("  ");
    };
    line(
        &mut h,
        &format!("<title>{}</title>", text_escape(&home.name)),
    );
    if let Some(d) = &home.description {
        line(
            &mut h,
            &format!("<meta name=\"description\" content=\"{}\">", attr_escape(d)),
        );
    }
    line(
        &mut h,
        &format!(
            "<meta name=\"application-name\" content=\"{}\">",
            attr_escape(&home.short_name)
        ),
    );
    line(
        &mut h,
        "<link rel=\"manifest\" href=\"manifest.webmanifest\">",
    );
    line(
        &mut h,
        &format!(
            "<meta name=\"theme-color\" media=\"(prefers-color-scheme: light)\" content=\"{}\">",
            home.theme_color
        ),
    );
    line(
        &mut h,
        &format!(
            "<meta name=\"theme-color\" media=\"(prefers-color-scheme: dark)\" content=\"{}\">",
            home.theme_color_dark
        ),
    );
    if icons.contains(&64) {
        line(
            &mut h,
            "<link rel=\"icon\" type=\"image/png\" sizes=\"64x64\" href=\"icons/icon-64.png\">",
        );
    }
    if icons.contains(&192) {
        line(
            &mut h,
            "<link rel=\"apple-touch-icon\" href=\"icons/icon-192.png\">",
        );
    }
    line(
        &mut h,
        "<meta name=\"mobile-web-app-capable\" content=\"yes\">",
    );
    line(
        &mut h,
        "<meta name=\"apple-mobile-web-app-capable\" content=\"yes\">",
    );
    line(
        &mut h,
        "<meta name=\"apple-mobile-web-app-status-bar-style\" content=\"default\">",
    );
    line(
        &mut h,
        &format!(
            "<meta name=\"apple-mobile-web-app-title\" content=\"{}\">",
            attr_escape(&home.short_name)
        ),
    );
    h.trim_end().to_string()
}

fn text_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn attr_escape(s: &str) -> String {
    text_escape(s).replace('"', "&quot;")
}

/// Every file under `dist` as `(relative path with '/' separators, bytes)`, sorted by path —
/// the service worker's precache list and the input to its cache version. `sw.js` itself is
/// left out: a worker never caches its own script.
fn dist_files(dist: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
        for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out)?;
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if rel == "sw.js" {
                continue;
            }
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            out.push((rel, bytes));
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dist, dist, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// The service worker for a dist: a cache named by a digest of every file, filled with all of
/// them at install, and served network-first — the network's answer when there is one (and
/// the cache refreshed from it), the cache's when there is not, `index.html` for any
/// navigation neither can answer. Same-origin GETs inside the worker's scope only; the
/// dayscript socket and every other origin pass through untouched. A rebuild changes the
/// digest, so the next visit installs a fresh cache and the activation drops the old one.
pub fn service_worker(files: &[(String, Vec<u8>)]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    for (rel, bytes) in files {
        hasher.update(rel.as_bytes());
        hasher.update([0u8]);
        hasher.update(bytes);
        hasher.update([0u8]);
    }
    let digest = hasher.finalize();
    let version: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    let list = files
        .iter()
        .map(|(rel, _)| serde_json::Value::String(rel.clone()))
        .collect::<Vec<_>>();
    let precache =
        serde_json::to_string(&serde_json::Value::Array(list)).unwrap_or_else(|_| "[]".into());
    format!(
        r#"// Generated by `day build -p web-dom` (docs/web.md "Home screen and offline"): the offline
// shell. Cache version {version}, from the bytes of every file listed below.
const CACHE = 'day-{version}';
const PRECACHE = {precache};

self.addEventListener('install', (event) => {{
  event.waitUntil((async () => {{
    const cache = await caches.open(CACHE);
    // One request per file, each tolerated: a host that serves a subset still installs.
    await Promise.allSettled(PRECACHE.map((path) => cache.add(new Request(path, {{ cache: 'reload' }}))));
    await self.skipWaiting();
  }})());
}});

self.addEventListener('activate', (event) => {{
  event.waitUntil((async () => {{
    for (const key of await caches.keys()) {{
      if (key.startsWith('day-') && key !== CACHE) await caches.delete(key);
    }}
    await self.clients.claim();
  }})());
}});

self.addEventListener('fetch', (event) => {{
  const request = event.request;
  if (request.method !== 'GET') return;
  const url = new URL(request.url);
  const scope = new URL(self.registration.scope);
  if (url.origin !== scope.origin || !url.pathname.startsWith(scope.pathname)) return;
  event.respondWith((async () => {{
    const cache = await caches.open(CACHE);
    try {{
      const fresh = await fetch(request);
      if (fresh.ok && fresh.type === 'basic') cache.put(request, fresh.clone());
      return fresh;
    }} catch (err) {{
      const hit = await cache.match(request, {{ ignoreSearch: true }});
      if (hit) return hit;
      if (request.mode === 'navigate') {{
        const index = await cache.match(new URL('index.html', scope).href);
        if (index) return index;
      }}
      throw err;
    }}
  }})());
}});
"#
    )
}

/// Percent-encode a query key/value: keep unreserved characters (RFC 3986), escape the rest —
/// `URLSearchParams` on the page decodes them back.
fn query_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Serve the assembled `dist/` on a loopback port and open a browser at it. The returned
/// handle runs the accept loop; `day launch` stays in the foreground (Ctrl-C stops the
/// server). `--locale` and a `DAY_THEME` env ride as query parameters (`?locale=`,
/// `?theme=`), and every other `--env` pair as `?<key>=<value>` for `day::env` to read back;
/// the session's dayscript invitation rides as `?dayscript=<token>`, and the
/// server bridges the page's `/dayscript` WebSocket to the plain TCP protocol the runner
/// speaks on `DAYSCRIPT_PORT` — `--script` and `day drive` work unchanged (docs/web.md).
pub fn launch_web(
    project: &Project,
    outcome: &BuildOutcome,
    spec: &LaunchSpec,
) -> Result<std::thread::JoinHandle<i32>, String> {
    let dist = outcome.artifact.clone();
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("bind: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("addr: {e}"))?
        .port();
    let env_of = |key: &str| {
        spec.envs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    let mut params: Vec<String> = Vec::new();
    if let Some(locale) = &spec.locale {
        params.push(format!("locale={locale}"));
    }
    if let Some(theme) = env_of("DAY_THEME") {
        params.push(format!("theme={theme}"));
    }
    if let Some(token) = env_of("DAYSCRIPT_TOKEN") {
        params.push(format!("dayscript={token}"));
    }
    // Every other `--env` pair travels as its own query parameter: a browser sandbox has no
    // process environment, so the page URL is the delivery channel and `day::env` reads it
    // back through the shim's `day_dom_env` (docs/web.md). DAYSCRIPT_PORT stays host-side,
    // and DAYSCRIPT_TOKEN/DAY_THEME already travel under their reserved names above.
    for (k, v) in &spec.envs {
        if k == "DAYSCRIPT_PORT" || k == "DAYSCRIPT_TOKEN" || k == "DAY_THEME" {
            continue;
        }
        params.push(format!("{}={}", query_escape(k), query_escape(v)));
    }
    let mut url = format!("http://127.0.0.1:{port}/");
    if !params.is_empty() {
        url.push('?');
        url.push_str(&params.join("&"));
    }
    status("Serving", &format!("{} → {url}", dist.display()));
    let _ = &project.root; // identity/env vars are baked into the wasm at build time
    if let Some(script_port) = env_of("DAYSCRIPT_PORT").and_then(|p| p.parse::<u16>().ok()) {
        start_runner_bridge(script_port)?;
    }
    // Drop the PREVIOUS launch's bridge endpoints before opening this page (capture matrix:
    // several launches share one process and one bridge). The old page's socket is closed but
    // still WRITABLE — TCP buffers the first write after a peer close and errors only on the
    // next — so a stale PAGE_WS swallows the new run's first step whole: forwarded "successfully",
    // no reply ever, the runner burns its whole window and reports the engine lost. Cleared
    // slots make forward_to_page genuinely wait for THIS page's registration instead.
    {
        *PAGE_WS.lock().expect("page slot") = None;
        *RUNNER.lock().expect("runner slot") = None;
    }
    open_page(&url)?;
    let handle = std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let dist = dist.clone();
            std::thread::spawn(move || serve_one(stream, &dist));
        }
        0
    });
    Ok(handle)
}

/// Open the page: through the `DAY_WEB_DRIVER` command when set (a scripted/CI browser that
/// also answers screenshot requests — see [`driver_screenshot`]), else the default browser.
/// The driver is spawned as `<cmd…> <url> <control-port>` and serves `GET /screenshot` (PNG)
/// and `GET /quit` on the control port.
fn open_page(url: &str) -> Result<(), String> {
    let Ok(driver) = std::env::var("DAY_WEB_DRIVER") else {
        open_in_browser(url);
        return Ok(());
    };
    // Reserve a loopback port for the driver's control server (bind-then-drop; the driver
    // rebinds it immediately).
    let control = TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map_err(|e| format!("driver control port: {e}"))?
        .port();
    let mut words = driver.split_whitespace();
    let program = words.next().ok_or("DAY_WEB_DRIVER is empty")?;
    // A previous variant's browser (capture matrix) shows the OLD page — retire it first, or
    // its control port would keep answering screenshot requests with stale pixels.
    stop_driver();
    let child = Command::new(program)
        .args(words)
        .arg(url)
        .arg(control.to_string())
        .spawn()
        .map_err(|e| format!("DAY_WEB_DRIVER {driver:?}: {e}"))?;
    crate::signals::register_child(child.id());
    *DRIVER.lock().expect("driver slot") = Some((control, child));
    status("Driver", &format!("{driver} (control port {control})"));
    Ok(())
}

fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let opener = Command::new("open").arg(url).status();
    #[cfg(target_os = "linux")]
    let opener = Command::new("xdg-open").arg(url).status();
    #[cfg(target_os = "windows")]
    let opener = Command::new("cmd").args(["/C", "start", "", url]).status();
    if opener.is_err() {
        status("Open", url);
    }
}

/// Answer one HTTP request: static GETs resolved strictly inside `dist`, plus the two dynamic
/// paths — the `/dayscript` WebSocket and the `/day-http-ok` echo endpoint.
fn serve_one(mut stream: TcpStream, dist: &Path) {
    let mut buf = [0u8; 4096];
    let n = match stream.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return,
    };
    let head = String::from_utf8_lossy(&buf[..n]).into_owned();
    let path = head
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/");
    if path == "/dayscript" {
        // The page's dayscript WebSocket (docs/web.md) — this thread becomes its pump.
        serve_dayscript_ws(stream, &buf[..n]);
        return;
    }
    if path == "/day-http-ok" {
        // day-part-http's same-origin demo endpoint (docs/web.md): a browser tab can host no
        // loopback listener, so apps whose HTTP demo would spin one (the showcase's Platform
        // services page) fetch this path instead. Same bodies as that native one-shot server —
        // GET answers `day-http-ok`, any other method echoes `day-http-ok:<METHOD>` — so
        // walkthrough asserts are byte-identical on web.
        let method = head.split_whitespace().next().unwrap_or("GET");
        let body = if method == "GET" {
            "day-http-ok".to_string()
        } else {
            format!("day-http-ok:{method}")
        };
        let _ = stream.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        );
        return;
    }
    match resolve(dist, path).and_then(|p| std::fs::read(&p).ok().map(|b| (p, b))) {
        Some((p, body)) => {
            let mime = mime_of(&p);
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nCross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Embedder-Policy: require-corp\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            );
            let _ = stream.write_all(&body);
        }
        None => {
            let _ = stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    }
}

/// Map a request path to a file inside `dist`, rejecting anything that could escape it.
fn resolve(dist: &Path, path: &str) -> Option<PathBuf> {
    let rel = path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel
        .split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return None;
    }
    let p = dist.join(rel);
    p.is_file().then_some(p)
}

fn mime_of(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        // Required exactly: `WebAssembly.instantiateStreaming` refuses other types.
        "wasm" => "application/wasm",
        "json" => "application/json",
        "webmanifest" => "application/manifest+json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
}

// ---------------------------------------------------------------------------
// dayscript bridge (docs/web.md, §14.5): the runner speaks its usual newline-JSON TCP
// protocol to DAYSCRIPT_PORT; the page speaks WebSocket to `/dayscript` on the dev server;
// this bridge pipes lines between the two. The engine in the wasm validates the token, so
// the bridge itself is a dumb pipe.
// ---------------------------------------------------------------------------

/// The page half of the bridge: the accepted `/dayscript` WebSocket (write side).
static PAGE_WS: std::sync::Mutex<Option<TcpStream>> = std::sync::Mutex::new(None);
/// The runner half: the accepted DAYSCRIPT_PORT connection (write side, for replies).
static RUNNER: std::sync::Mutex<Option<TcpStream>> = std::sync::Mutex::new(None);
/// The `DAY_WEB_DRIVER` control port + child of the CURRENT launch. A `Mutex<Option<…>>`, not a
/// `OnceLock`: a capture-matrix launch (`--themes`/`--locales`) opens the page once per variant
/// IN ONE PROCESS, and a once-only slot would leave every later screenshot request talking to
/// the FIRST variant's browser — silently capturing the wrong theme and locale.
#[allow(clippy::type_complexity)]
static DRIVER: std::sync::Mutex<Option<(u16, std::process::Child)>> = std::sync::Mutex::new(None);

/// Ports this process already runs a dayscript bridge on. The bridge listener is a forever
/// thread; a second launch on the same port in the same process (again: the capture matrix)
/// must REUSE it — rebinding is EADDRINUSE — and the accept loop already hands each new runner
/// connection and page WebSocket to the current slots.
static BRIDGED: std::sync::Mutex<Option<std::collections::HashSet<u16>>> =
    std::sync::Mutex::new(None);

/// Accept runner connections on the dayscript port and forward each request line to the
/// page's WebSocket (waiting for the page to connect — it is still loading when the runner's
/// first step arrives).
fn start_runner_bridge(port: u16) -> Result<(), String> {
    {
        let mut bridged = BRIDGED.lock().expect("bridged ports");
        let set = bridged.get_or_insert_with(std::collections::HashSet::new);
        if !set.insert(port) {
            // Already ours: the listener thread below is still accepting, and the next runner
            // connection and page WebSocket will replace the RUNNER/PAGE_WS slots.
            return Ok(());
        }
    }
    let listener =
        TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("dayscript bridge: {e}"))?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            *RUNNER.lock().expect("runner slot") = stream.try_clone().ok();
            let mut reader = std::io::BufReader::new(stream);
            let mut line = String::new();
            loop {
                line.clear();
                match std::io::BufRead::read_line(&mut reader, &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                if !forward_to_page(line.trim_end()) {
                    // The page never connected (or died): answer for it so the runner fails
                    // with a diagnosis instead of a timeout.
                    let reply = "{\"ok\":false,\"error\":\"web page not connected (dayscript bridge)\",\"retryable\":false}\n";
                    if let Some(r) = RUNNER.lock().expect("runner slot").as_mut() {
                        let _ = r.write_all(reply.as_bytes());
                    }
                }
            }
        }
    });
    Ok(())
}

/// Send one request line to the page as a WebSocket text frame, waiting up to ~20 s for the
/// page to connect first. False when it never did or the write failed.
fn forward_to_page(line: &str) -> bool {
    for _ in 0..80 {
        {
            let mut slot = PAGE_WS.lock().expect("page slot");
            if let Some(ws) = slot.as_mut() {
                if ws_send_text(ws, line).is_ok() {
                    return true;
                }
                *slot = None; // page went away; wait for a reconnect
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    false
}

/// Upgrade an accepted HTTP connection to the `/dayscript` WebSocket and pump its text
/// frames (engine reply lines) to the runner. `head` is the raw request bytes already read;
/// any bytes past the header end are the start of the frame stream.
fn serve_dayscript_ws(mut stream: TcpStream, head: &[u8]) {
    let text = String::from_utf8_lossy(head);
    let Some(key) = text.lines().find_map(|l| {
        l.split_once(':').and_then(|(name, v)| {
            name.eq_ignore_ascii_case("sec-websocket-key")
                .then(|| v.trim().to_string())
        })
    }) else {
        return;
    };
    let accept = crate::script::b64encode_public(&sha1(
        format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
    ));
    let resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    if stream.write_all(resp.as_bytes()).is_err() {
        return;
    }
    let leftover = head
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| head[i + 4..].to_vec())
        .unwrap_or_default();
    *PAGE_WS.lock().expect("page slot") = stream.try_clone().ok();
    ws_read_loop(stream, leftover);
}

/// Read client frames forever: text frames carry engine reply lines for the runner; ping is
/// answered; close (or a read error) ends the connection.
fn ws_read_loop(mut stream: TcpStream, mut pending: Vec<u8>) {
    let mut message: Vec<u8> = Vec::new();
    loop {
        let mut hdr = [0u8; 2];
        if ws_read_exact(&mut stream, &mut hdr, &mut pending).is_err() {
            return;
        }
        let fin = hdr[0] & 0x80 != 0;
        let opcode = hdr[0] & 0x0f;
        let masked = hdr[1] & 0x80 != 0;
        let mut len = (hdr[1] & 0x7f) as u64;
        if len == 126 {
            let mut ext = [0u8; 2];
            if ws_read_exact(&mut stream, &mut ext, &mut pending).is_err() {
                return;
            }
            len = u64::from(u16::from_be_bytes(ext));
        } else if len == 127 {
            let mut ext = [0u8; 8];
            if ws_read_exact(&mut stream, &mut ext, &mut pending).is_err() {
                return;
            }
            len = u64::from_be_bytes(ext);
        }
        if len > 16 * 1024 * 1024 {
            return; // a reply line should never be this large — refuse
        }
        let mut mask = [0u8; 4];
        if masked && ws_read_exact(&mut stream, &mut mask, &mut pending).is_err() {
            return;
        }
        let mut payload = vec![0u8; len as usize];
        if ws_read_exact(&mut stream, &mut payload, &mut pending).is_err() {
            return;
        }
        if masked {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
        }
        match opcode {
            0x8 => return, // close
            0x9 => {
                // ping → pong (unfragmented control frame)
                let mut pong = vec![0x8a, payload.len() as u8];
                pong.extend_from_slice(&payload);
                if stream.write_all(&pong).is_err() {
                    return;
                }
            }
            0x1 | 0x0 => {
                message.extend_from_slice(&payload);
                if fin {
                    let line = String::from_utf8_lossy(&message).into_owned();
                    message.clear();
                    if let Some(r) = RUNNER.lock().expect("runner slot").as_mut() {
                        let mut out = line.trim_end().to_string();
                        out.push('\n');
                        let _ = r.write_all(out.as_bytes());
                    }
                }
            }
            _ => {} // pong / reserved: ignore
        }
    }
}

/// Fill `buf` from the handshake leftover first, then the socket.
fn ws_read_exact(
    stream: &mut TcpStream,
    buf: &mut [u8],
    pending: &mut Vec<u8>,
) -> std::io::Result<()> {
    let from_pending = pending.len().min(buf.len());
    buf[..from_pending].copy_from_slice(&pending[..from_pending]);
    pending.drain(..from_pending);
    if from_pending < buf.len() {
        std::io::Read::read_exact(stream, &mut buf[from_pending..])?;
    }
    Ok(())
}

/// Send one unmasked server→client text frame.
fn ws_send_text(stream: &mut TcpStream, text: &str) -> std::io::Result<()> {
    let payload = text.as_bytes();
    let mut frame = Vec::with_capacity(payload.len() + 10);
    frame.push(0x81);
    match payload.len() {
        n if n < 126 => frame.push(n as u8),
        n if n < 65536 => {
            frame.push(126);
            frame.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            frame.push(127);
            frame.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(payload);
    stream.write_all(&frame)
}

/// SHA-1, for the WebSocket accept key only (RFC 6455 mandates it; this is not used for any
/// security purpose beyond the handshake's anti-cache echo).
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    let ml = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&ml.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// Driver control (script.rs): screenshots + teardown for the DAY_WEB_DRIVER browser.
// ---------------------------------------------------------------------------

/// Fetch a PNG of the page from the driver's control server and write it to `path`.
pub(crate) fn driver_screenshot(path: &Path) -> Result<(), String> {
    let port = match DRIVER.lock().expect("driver slot").as_ref() {
        Some((port, _)) => *port,
        None => {
            return Err("no web driver (set DAY_WEB_DRIVER to a browser driver command)".into());
        }
    };
    let body = control_get(port, "/screenshot")?;
    if body.is_empty() {
        return Err("web driver returned an empty screenshot".into());
    }
    std::fs::write(path, &body).map_err(|e| format!("{}: {e}", path.display()))
}

/// Stop the driver browser (end of a scripted run): ask it to quit, then reap the child.
pub(crate) fn stop_driver() {
    if let Some((port, mut child)) = DRIVER.lock().expect("driver slot").take() {
        let _ = control_get(port, "/quit");
        let _ = child.wait();
    }
}

/// Minimal HTTP GET against the driver's loopback control server (Connection: close).
fn control_get(port: u16, path: &str) -> Result<Vec<u8>, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| format!("driver: {e}"))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .ok();
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .map_err(|e| format!("driver: {e}"))?;
    let mut all = Vec::new();
    std::io::Read::read_to_end(&mut stream, &mut all).map_err(|e| format!("driver: {e}"))?;
    let split = all
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("driver: malformed response")?;
    Ok(all[split + 4..].to_vec())
}

#[cfg(test)]
mod home_screen_tests {
    use super::*;

    fn home() -> HomeScreen {
        HomeScreen {
            name: "Day \"Showcase\" <demo>".into(),
            short_name: "Day Showcase".into(),
            description: Some("A & B".into()),
            lang: "fr".into(),
            theme_color: "#123246".into(),
            theme_color_dark: "#000000".into(),
            background_color: "#123246".into(),
            display: crate::meta::WebDisplay::Standalone,
        }
    }

    #[test]
    fn short_name_keeps_whole_words_within_twelve_characters() {
        assert_eq!(short_name_of("Day Showcase"), "Day Showcase");
        assert_eq!(short_name_of("Day Showcase Deluxe Edition"), "Day Showcase");
        assert_eq!(short_name_of("Supercalifragilistic App"), "Supercalifra");
        assert_eq!(short_name_of("  Notes  "), "Notes");
    }

    #[test]
    fn manifest_is_relative_and_lists_the_staged_icons_only() {
        let json = manifest_json(&home(), &[64, 192, 512]);
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v["start_url"], "./");
        assert_eq!(v["scope"], "./");
        assert_eq!(v["display"], "standalone");
        assert_eq!(v["lang"], "fr");
        assert_eq!(v["name"], "Day \"Showcase\" <demo>");
        assert!(
            v.get("id").is_none(),
            "id defaults to the resolved start_url"
        );
        let icons = v["icons"].as_array().expect("icons");
        assert_eq!(icons.len(), 4, "64, 192, 512 any + 512 maskable");
        assert_eq!(icons[3]["purpose"], "maskable");
        let none = manifest_json(&home(), &[]);
        let v: serde_json::Value = serde_json::from_str(&none).expect("valid JSON");
        assert_eq!(v["icons"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn head_escapes_text_and_names_both_appearances() {
        let head = head_tags(&home(), &[64, 192]);
        assert!(head.contains("<title>Day \"Showcase\" &lt;demo&gt;</title>"));
        assert!(head.contains("content=\"A &amp; B\""));
        assert!(head.contains("(prefers-color-scheme: light)\" content=\"#123246\""));
        assert!(head.contains("(prefers-color-scheme: dark)\" content=\"#000000\""));
        assert!(head.contains("rel=\"apple-touch-icon\" href=\"icons/icon-192.png\""));
        assert!(head.contains("rel=\"manifest\" href=\"manifest.webmanifest\""));
        let bare = head_tags(&home(), &[]);
        assert!(!bare.contains("rel=\"icon\""));
    }

    #[test]
    fn service_worker_precaches_every_file_and_versions_by_content() {
        let a = vec![
            ("index.html".to_string(), b"<html>".to_vec()),
            ("app.wasm".to_string(), vec![0, 1]),
        ];
        let sw = service_worker(&a);
        assert!(sw.contains("const PRECACHE = [\"index.html\",\"app.wasm\"];"));
        let mut b = a.clone();
        b[1].1.push(2);
        assert_ne!(
            service_worker(&a).lines().nth(2),
            service_worker(&b).lines().nth(2),
            "a changed byte changes the cache name"
        );
        assert_eq!(
            service_worker(&a),
            sw,
            "the same files give the same worker"
        );
    }

    #[test]
    fn dist_files_skips_the_worker_and_sorts() {
        let dir = std::env::temp_dir().join(format!("day-dist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets/images")).expect("dir");
        std::fs::write(dir.join("sw.js"), "x").expect("sw");
        std::fs::write(dir.join("index.html"), "i").expect("index");
        std::fs::write(dir.join("assets/images/a.png"), "p").expect("png");
        let files = dist_files(&dir).expect("walk");
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["assets/images/a.png", "index.html"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
