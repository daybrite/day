// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `day prepare` (docs/icons.md, DESIGN.md §16.5) — every platform's derived host files from
//! ONE master, kept in sync and kept OUT of git.
//!
//! Master discovery (first hit wins): an explicit path argument, else
//! `resource/icons/icon.svg`, `resource/icons/day-icon.svg`, `resource/icons/icon.png`. A
//! per-family override beside it (`resource/icons/ios.svg`, `macos.svg`, `android.svg`,
//! `linux.svg`, `windows.svg`, `harmony.svg`, `png.svg`) replaces the master for that family
//! alone, and a checked-in `resource/icons/ios/AppIcon.icon/` bundle is copied through instead
//! of generated — the rule being that anything in git is a SOURCE, and everything derived lives
//! under [`HOST_DIR`] where the host projects reference it (docs/project-structure.md).
//!
//! An SVG master may mark top-level groups as semantic layers by id:
//! `day:background`, `day:foreground` (any number), `day:monochrome`, `day:dark`.
//! The composite (background+foregrounds) feeds every full-bleed output; the split layers feed
//! Android's adaptive icon. An unlayered SVG (or a PNG master) still produces the full legacy
//! set — the adaptive foreground is then the whole art in the safe zone over a derived
//! background color. `day:monochrome`/`day:dark` are reserved for the modern formats
//! (Icon Composer, themed icons) and are excluded from every composite today.
//!
//! Everything renders in memory first; `--check` compares those bytes against `build/day/host`
//! and exits 5 when anything is missing or stale (the duty-matrix pattern — CI's gate, and what
//! `day lint` and the VS Code extension ask before opening a native project), a plain run writes
//! them plus `host.lock.json` recording the master and generator. [`ensure`] is the cheap form
//! every build takes: it regenerates only when the lock says the master or the generator moved.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use day_vector::tiny_skia;

use crate::meta::Project;
use crate::ops::status;

/// Where every derived host file lives, relative to the project root — never in git. The Xcode
/// projects reference `ios/Assets.xcassets` and `macos/Assets.xcassets` here by relative path,
/// the Gradle module adds `platform/android/res` as a resource source set, `day pack` reads the Linux
/// and Windows icons here, and the HarmonyOS media directories are symlinks into `harmony/media`.
pub const HOST_DIR: &str = "build/day/host";
/// Lock path, relative to the project root.
const LOCK: &str = "build/day/host/host.lock.json";
/// The legacy lock, read once by [`migrate`] to tell a generated file from a hand-edited one.
const LEGACY_LOCK: &str = "resource/icons/icons.lock.json";
/// The macOS margin convention: 824 pt of art on the 1024 canvas, radius 184.
const MAC_INSET: f32 = 100.0;
const MAC_ART: f32 = 824.0;
const MAC_RADIUS: f32 = 184.0;
/// Android adaptive canvas and safe zone (108 dp canvas, 66 dp safe → 432/264 px at xxxhdpi).
const ADAPTIVE_PX: u32 = 432;
const SAFE_PX: f32 = 264.0;

pub struct IconOptions {
    pub master: Option<PathBuf>,
    pub check: bool,
    pub platforms: Vec<String>,
}

/// Resolve a `--seed` / `--icon-seed` spec: a bare integer is used as-is, anything else is
/// hashed ([`day_vector::icongen::seed_from_str`] — how `day new` seeds from the app id),
/// and `None` draws fresh entropy. Always tell the user the number (via the return), so a
/// liked random icon can be reproduced.
pub fn resolve_seed(spec: Option<&str>) -> u64 {
    match spec {
        Some(s) => s
            .parse::<u64>()
            .unwrap_or_else(|_| day_vector::icongen::seed_from_str(s)),
        None => {
            use std::hash::{BuildHasher as _, Hasher as _};
            let clock = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
                .unwrap_or(0);
            let keyed = std::collections::hash_map::RandomState::new()
                .build_hasher()
                .finish();
            clock ^ keyed
        }
    }
}

/// `day icon --generate`: write the seeded master to `resource/icons/icon.svg`. Refuses to
/// clobber an existing master (any discovery candidate) unless `overwrite` — a hand-drawn
/// icon is unrecoverable. The caller then runs [`run`] to regenerate every output.
pub fn generate_master(project: &Project, seed: u64, overwrite: bool) -> Result<PathBuf, String> {
    if !overwrite {
        for candidate in [
            "resource/icons/icon.svg",
            "resource/icons/day-icon.svg",
            "resource/icons/icon.png",
        ] {
            let p = project.root.join(candidate);
            if p.exists() {
                return Err(format!(
                    "a master icon already exists ({candidate}) — pass --overwrite to replace it"
                ));
            }
        }
    }
    let dest = project.root.join("resource/icons/icon.svg");
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    }
    std::fs::write(&dest, day_vector::icongen::generate(seed))
        .map_err(|e| format!("write {}: {e}", dest.display()))?;
    Ok(dest)
}

/// `day icon --generate --out <file.svg>`: preview mode — write the seeded master to an
/// arbitrary path (no project needed, nothing else touched) plus a 512 px PNG render beside
/// it, so seeds can be browsed before committing to one.
pub fn generate_preview(path: &Path, seed: u64) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    }
    let svg = day_vector::icongen::generate(seed);
    std::fs::write(path, &svg).map_err(|e| format!("write {}: {e}", path.display()))?;
    let tree = day_vector::parse(svg.as_bytes())?;
    let png = day_vector::render_png(&tree, 512)?;
    let png_path = path.with_extension("png");
    std::fs::write(&png_path, png).map_err(|e| format!("write {}: {e}", png_path.display()))?;
    Ok(())
}

/// Drift lines (for exit code 5), or a hard error.
pub enum IconError {
    Drift(Vec<String>),
    Other(String),
}

impl From<String> for IconError {
    fn from(s: String) -> Self {
        IconError::Other(s)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Png,
    Ios,
    Macos,
    Linux,
    Windows,
    Android,
    Ohos,
}

fn family_of_target(t: &str) -> Vec<Family> {
    match t {
        "ios-uikit" => vec![Family::Ios],
        "android-mdc" => vec![Family::Android],
        "harmony-arkui" => vec![Family::Ohos],
        "windows-xaml" | "windows-gtk" | "windows-qt" => vec![Family::Windows],
        "macos-appkit" | "macos-gtk" | "macos-qt" => vec![Family::Macos],
        "linux-gtk" | "linux-qt" => vec![Family::Linux],
        "web-dom" => vec![Family::Png],
        _ => vec![],
    }
}

pub fn run(project: &Project, opts: &IconOptions) -> Result<usize, IconError> {
    let master = discover(project, opts.master.as_deref())?;
    let families: Vec<Family> = if opts.platforms.is_empty() {
        vec![
            Family::Png,
            Family::Ios,
            Family::Macos,
            Family::Linux,
            Family::Windows,
            Family::Android,
            Family::Ohos,
        ]
    } else {
        let mut fams: Vec<Family> = opts
            .platforms
            .iter()
            .flat_map(|p| family_of_target(p))
            .collect();
        fams.push(Family::Png); // the shared exports underpin every family
        fams.dedup();
        fams
    };

    status(
        if opts.check { "Checking" } else { "Preparing" },
        &format!("host files from {}", master.display()),
    );
    let outputs = generate(project, &master, &families)?;

    if opts.check {
        let mut drift = Vec::new();
        for (rel, bytes) in &outputs {
            match std::fs::read(project.root.join(rel)) {
                Ok(on_disk) if &on_disk == bytes => {}
                Ok(_) => drift.push(format!("{rel}: stale (differs from the master's render)")),
                Err(_) => drift.push(format!("{rel}: missing")),
            }
        }
        for (link, target) in harmony_links(project, &families) {
            let path = project.root.join(&link);
            if path.is_dir() {
                continue;
            }
            // A link that exists but resolves to nothing is a different failure from no link
            // at all: it is what a target spelled with the wrong separator looks like.
            if std::fs::symlink_metadata(&path).is_ok() {
                drift.push(format!("{link}: links to nothing (expected {target})"));
            } else {
                drift.push(format!("{link}: not linked"));
            }
        }
        // Same-version guard: a lock from another generator makes byte comparison unfair.
        if let Ok(lock) = std::fs::read_to_string(project.root.join(LOCK))
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(&lock)
            && let Some(generator) = v.get("generator").and_then(|g| g.as_str())
            && generator != generator_id()
        {
            drift.push(format!(
                "host.lock.json was written by {generator}; this is {} — run `day prepare`",
                generator_id()
            ));
        }
        if drift.is_empty() {
            status("Verified", &format!("{} host files current", outputs.len()));
            Ok(outputs.len())
        } else {
            Err(IconError::Drift(drift))
        }
    } else {
        let mut lock_outputs = BTreeMap::new();
        for (rel, bytes) in &outputs {
            let path = project.root.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
            }
            // Unchanged bytes are left alone, so actool and aapt2 see no new mtime.
            if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
                std::fs::write(&path, bytes).map_err(|e| format!("write {rel}: {e}"))?;
            }
            lock_outputs.insert(rel.clone(), sha256_hex(bytes));
        }
        for (link, target) in harmony_links(project, &families) {
            link_dir(&project.root, &link, &target)?;
        }
        let master_bytes = std::fs::read(&master).map_err(|e| e.to_string())?;
        let mut sources = serde_json::Map::new();
        sources.insert(
            master
                .strip_prefix(&project.root)
                .unwrap_or(&master)
                .to_string_lossy()
                .into_owned(),
            serde_json::Value::String(sha256_hex(&master_bytes)),
        );
        for (rel, bytes) in override_sources(project) {
            sources.insert(rel, serde_json::Value::String(sha256_hex(&bytes)));
        }
        let lock = serde_json::json!({
            "generator": generator_id(),
            "sources": sources,
            "outputs": lock_outputs,
        });
        let lock_path = project.root.join(LOCK);
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
        std::fs::write(
            &lock_path,
            serde_json::to_string_pretty(&lock).unwrap_or_default() + "\n",
        )
        .map_err(|e| format!("write lock: {e}"))?;
        status(
            "Prepared",
            &format!("{} host files under {HOST_DIR}", outputs.len()),
        );
        Ok(outputs.len())
    }
}

/// The cheap form every build takes: regenerate only when the lock cannot vouch for the
/// current master, its overrides, and this generator, or when a listed output has gone missing.
/// A build never fails for want of an icon it can render itself.
pub fn ensure(project: &Project, platforms: &[&str]) -> Result<(), String> {
    // No master, nothing to derive: an app that ships hand-made platform icons keeps them,
    // and its host projects go on referencing what they reference. Said once per run.
    if discover(project, None).is_err() {
        static SAID: std::sync::Once = std::sync::Once::new();
        SAID.call_once(|| {
            status(
                "Note",
                "no resource/icons/icon.svg (or icon.png) master — host icon files are not \
                 generated; add one (`day icon --generate`) and run `day prepare --migrate`",
            );
        });
        return Ok(());
    }
    let fresh = || -> Option<bool> {
        let lock = std::fs::read_to_string(project.root.join(LOCK)).ok()?;
        let v: serde_json::Value = serde_json::from_str(&lock).ok()?;
        if v.get("generator")?.as_str()? != generator_id() {
            return Some(false);
        }
        let master = discover(project, None).ok()?;
        let mut want = serde_json::Map::new();
        want.insert(
            master
                .strip_prefix(&project.root)
                .unwrap_or(&master)
                .to_string_lossy()
                .into_owned(),
            serde_json::Value::String(sha256_hex(&std::fs::read(&master).ok()?)),
        );
        for (rel, bytes) in override_sources(project) {
            want.insert(rel, serde_json::Value::String(sha256_hex(&bytes)));
        }
        if v.get("sources")?.as_object()? != &want {
            return Some(false);
        }
        let outputs = v.get("outputs")?.as_object()?;
        let families: Vec<Family> = platforms.iter().flat_map(|p| family_of_target(p)).collect();
        let needed = |rel: &str| {
            platforms.is_empty()
                || families
                    .iter()
                    .any(|f| rel.starts_with(&format!("{HOST_DIR}/{}/", family_dir(*f))))
                || rel.starts_with(&format!("{HOST_DIR}/png/"))
        };
        Some(
            outputs
                .keys()
                .filter(|rel| needed(rel))
                .all(|rel| project.root.join(rel).is_file()),
        )
    };
    if fresh() == Some(true) {
        // The links are cheap to re-assert, and a fresh clone has a lock only after a first
        // run — so this is the path that repairs a link someone removed.
        let families: Vec<Family> = if platforms.is_empty() {
            vec![Family::Ohos]
        } else {
            platforms.iter().flat_map(|p| family_of_target(p)).collect()
        };
        for (link, target) in harmony_links(project, &families) {
            link_dir(&project.root, &link, &target)?;
        }
        return Ok(());
    }
    let opts = IconOptions {
        master: None,
        check: false,
        platforms: platforms.iter().map(|p| p.to_string()).collect(),
    };
    match run(project, &opts) {
        Ok(_) => Ok(()),
        Err(IconError::Other(e)) => Err(e),
        Err(IconError::Drift(lines)) => Err(lines.join("; ")),
    }
}

/// The per-family override sources beside the master, as `(project-relative path, bytes)`.
/// Their digests ride in the lock, so editing one invalidates it like editing the master does.
fn override_sources(project: &Project) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for f in [
        Family::Png,
        Family::Ios,
        Family::Macos,
        Family::Linux,
        Family::Windows,
        Family::Android,
        Family::Ohos,
    ] {
        let rel = format!("resource/icons/{}.svg", family_dir(f));
        if let Ok(bytes) = std::fs::read(project.root.join(&rel)) {
            out.push((rel, bytes));
        }
    }
    let composer = project.root.join(ICON_COMPOSER_OVERRIDE);
    if composer.is_dir() {
        let mut files: Vec<_> = walk_files(&composer);
        files.sort();
        for path in files {
            if let (Ok(rel), Ok(bytes)) = (path.strip_prefix(&project.root), std::fs::read(&path)) {
                out.push((rel.to_string_lossy().into_owned(), bytes));
            }
        }
    }
    out
}

/// A checked-in Icon Composer bundle that replaces the generated one (docs/icons.md).
const ICON_COMPOSER_OVERRIDE: &str = "resource/icons/ios/AppIcon.icon";

fn walk_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk_files(&path));
            } else {
                out.push(path);
            }
        }
    }
    out
}

/// The output subdirectory under [`HOST_DIR`] for a family.
fn family_dir(f: Family) -> &'static str {
    match f {
        Family::Png => "png",
        Family::Ios => "ios",
        Family::Macos => "macos",
        Family::Linux => "linux",
        Family::Windows => "windows",
        Family::Android => "android",
        Family::Ohos => "harmony",
    }
}

/// The HarmonyOS media directories hvigor insists on, as `(link path, link target)` pairs
/// relative to the project root. hvigor reads `resources/base/media` from a fixed place under
/// each module, so the generated media is linked there rather than referenced (the one host
/// that needs a link; Xcode and Gradle take a path). None when the project has no harmony host.
fn harmony_links(project: &Project, families: &[Family]) -> Vec<(String, String)> {
    if !families.contains(&Family::Ohos) {
        return Vec::new();
    }
    let hroot = crate::ohos::harmony_dir(project);
    if !hroot.is_dir() {
        return Vec::new();
    }
    let hrel = hroot
        .strip_prefix(&project.root)
        .unwrap_or(&hroot)
        .to_string_lossy()
        .into_owned();
    let media = format!("{HOST_DIR}/harmony/media");
    [
        format!("{hrel}/entry/src/main/resources/base/media"),
        format!("{hrel}/AppScope/resources/base/media"),
    ]
    .into_iter()
    .filter(|link| project.root.join(link).parent().is_some_and(|p| p.is_dir()))
    .map(|link| (link.clone(), media.clone()))
    .collect()
}

/// Make `link` (project-relative) a symlink to `target` (project-relative), relative so the
/// checkout can move. An existing correct link is left alone; a stale one is replaced; a real
/// directory in the way is the pre-`prepare` layout and is reported (`day prepare --migrate`
/// removes it). Where symlinks are unavailable (a Windows host without the privilege) the
/// target is copied instead, and the copy is refreshed on every run.
fn link_dir(root: &Path, link: &str, target: &str) -> Result<(), String> {
    let link_path = root.join(link);
    let target_path = root.join(target);
    std::fs::create_dir_all(&target_path).map_err(|e| format!("mkdir {target}: {e}"))?;
    let rel = link_relative(link, target);
    match std::fs::symlink_metadata(&link_path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            if std::fs::read_link(&link_path).ok().as_deref() == Some(rel.as_path()) {
                return Ok(());
            }
            std::fs::remove_file(&link_path).map_err(|e| format!("unlink {link}: {e}"))?;
        }
        Ok(meta) if meta.is_dir() => {
            // A copied stand-in from a host without symlinks refreshes; a directory holding
            // files that are not ours is the old checked-in layout.
            if link_path.join(".day-copied").is_file() {
                std::fs::remove_dir_all(&link_path).map_err(|e| format!("rm {link}: {e}"))?;
            } else {
                return Err(format!(
                    "{link} is a real directory, but the derived media now lives under {target} \
                     and is linked here — run `day prepare --migrate` to remove the checked-in \
                     copies (or delete the directory yourself)"
                ));
            }
        }
        Ok(_) => {
            std::fs::remove_file(&link_path).map_err(|e| format!("rm {link}: {e}"))?;
        }
        Err(_) => {}
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&rel, &link_path)
            .map_err(|e| format!("link {link} → {}: {e}", rel.display()))?;
    }
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_dir(&rel, &link_path).is_err() {
            status(
                "Note",
                &format!("symlinks are unavailable here; copying {target} to {link} instead"),
            );
            copy_dir(&target_path, &link_path)?;
            return std::fs::write(link_path.join(".day-copied"), b"")
                .map_err(|e| format!("mark {link}: {e}"));
        }
    }
    // Creating the link proves nothing about where it points: prove it resolves here, in the
    // command that made it, rather than one command later in `--check`.
    if link_path.is_dir() {
        Ok(())
    } else {
        Err(format!(
            "link {link} → {} was created but does not resolve to {target}",
            rel.display()
        ))
    }
}

/// The path `link` stores to reach `target`, both project-relative: enough `..` to climb from
/// the link's directory to the project root, then the target. Built component by component so
/// every separator is the host's own — Windows stores a symlink target verbatim in the reparse
/// point and its resolver does not treat `/` as a separator, so a target pushed as one
/// `build/day/host/…` string made a link that created fine and resolved to nothing.
fn link_relative(link: &str, target: &str) -> PathBuf {
    let depth = Path::new(link).components().count().saturating_sub(1);
    let mut rel = PathBuf::new();
    for _ in 0..depth {
        rel.push("..");
    }
    for component in Path::new(target).components() {
        rel.push(component);
    }
    rel
}

#[cfg(windows)]
fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("mkdir {}: {e}", to.display()))?;
    for entry in std::fs::read_dir(from)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| format!("copy {}: {e}", src.display()))?;
        }
    }
    Ok(())
}

/// `day prepare --migrate`: move a project from committed derived files to the generated
/// layout. Idempotent, and careful about what it deletes: a file is removed only when the
/// legacy lock proves it was generated (its digest matches); anything hand-edited stays, and
/// the summary says so, because that file has become a source override or a mistake, and
/// only its author knows which. The host projects are rewritten in place to reference
/// `build/day/host`, `.gitignore` learns the HarmonyOS links, and `prepare` then runs so the
/// project builds at once. Nothing touches git: the deletions show up in `git status`.
pub fn migrate(project: &Project) -> Result<(), String> {
    let root = &project.root;
    // The host projects are about to reference build/day/host; without a master there would be
    // nothing there, and Xcode would open onto a missing catalog.
    let master = discover(project, None).map_err(|e| {
        format!(
            "{e}\n  migrate needs a master to derive from: put the app's 1024 px icon at \
             resource/icons/icon.png (a raster master), or an SVG at resource/icons/icon.svg, \
             then run `day prepare --migrate` again"
        )
    })?;
    let legacy: BTreeMap<String, String> = std::fs::read_to_string(root.join(LEGACY_LOCK))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| {
            v.get("outputs")?.as_object().map(|o| {
                o.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                    .collect()
            })
        })
        .unwrap_or_default();
    // A second witness beside the legacy lock: the files this generator renders NOW. A committed
    // copy whose bytes equal a fresh render was generated, whatever the lock says — the macOS
    // catalog PNG, for one, was staged by `day new` from the same render and never locked.
    let rendered: std::collections::BTreeSet<String> = generate(
        project,
        &master,
        &[
            Family::Png,
            Family::Ios,
            Family::Macos,
            Family::Linux,
            Family::Windows,
            Family::Android,
            Family::Ohos,
        ],
    )?
    .iter()
    .map(|(_, b)| sha256_hex(b))
    .collect();
    let hroot = crate::ohos::harmony_dir(project);
    let hrel = hroot
        .strip_prefix(root)
        .unwrap_or(&hroot)
        .to_string_lossy()
        .into_owned();
    // Every place the old layout put a derived file. Directories are walked; each file inside
    // is judged on its own.
    let candidates = [
        "resource/icons/png".to_string(),
        "resource/icons/linux".to_string(),
        "resource/icons/windows".to_string(),
        "resource/icons/macos".to_string(),
        "resource/icons/ios".to_string(),
        "resource/icons/android".to_string(),
        "platform/ios/Assets.xcassets".to_string(),
        "platform/ios/AppIcon.icon".to_string(),
        "platform/macos/Assets.xcassets".to_string(),
        "platform/android/app/src/main/res/mipmap-xxxhdpi".to_string(),
        "platform/android/app/src/main/res/mipmap-anydpi-v26".to_string(),
        "platform/android/app/src/main/res/drawable/ic_launcher_monochrome.xml".to_string(),
        "platform/android/app/src/main/res/drawable-xxxhdpi/ic_launcher_monochrome.png".to_string(),
        format!("{hrel}/entry/src/main/resources/base/media"),
        format!("{hrel}/AppScope/resources/base/media"),
    ];
    let mut removed = Vec::new();
    let mut kept = Vec::new();
    for cand in &candidates {
        let path = root.join(cand);
        if std::fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            continue; // already the new layout
        }
        if !path.exists() {
            continue;
        }
        let files: Vec<PathBuf> = if path.is_dir() {
            walk_files(&path)
        } else {
            vec![path.clone()]
        };
        for file in files {
            let rel = file
                .strip_prefix(root)
                .unwrap_or(&file)
                .to_string_lossy()
                .into_owned();
            // Under `platform/`, the host render now supplies every one of these names: a kept
            // copy would collide with the host source set (Gradle refuses duplicate resources)
            // or sit where the harmony link has to go, and the template's placeholder catalog
            // PNG was never in any lock. So the whole tree is superseded, whatever the bytes.
            // Under `resource/icons/`, the old export tree, a file is only removed when it is
            // provably a render: its digest is in the legacy lock or equals a fresh render.
            // Finder litter goes either way.
            let superseded = !rel.starts_with("resource/icons/") || rel.ends_with(".DS_Store");
            let digest = std::fs::read(&file)
                .map(|b| sha256_hex(&b))
                .unwrap_or_default();
            let generated =
                legacy.get(&rel).is_some_and(|d| *d == digest) || rendered.contains(&digest);
            if generated || superseded {
                std::fs::remove_file(&file).map_err(|e| format!("rm {rel}: {e}"))?;
                removed.push(rel);
            } else {
                kept.push(rel);
            }
        }
        // Drop what is now empty, so the link (or nothing) can take the directory's place.
        if path.is_dir() {
            let _ = remove_empty_dirs(&path);
        }
    }
    let legacy_lock = root.join(LEGACY_LOCK);
    if legacy_lock.is_file() {
        std::fs::remove_file(&legacy_lock).map_err(|e| format!("rm {LEGACY_LOCK}: {e}"))?;
        removed.push(LEGACY_LOCK.to_string());
    }

    // The host projects: Xcode file references and the Gradle source set.
    let mut rewritten = Vec::new();
    for (platform, host) in [("ios", "ios"), ("macos", "macos")] {
        let rel = format!("platform/{platform}/DayApp.xcodeproj/project.pbxproj");
        let path = root.join(&rel);
        if let Ok(text) = std::fs::read_to_string(&path) {
            let updated = point_xcode_at_host(&text, host);
            if updated != text {
                std::fs::write(&path, updated).map_err(|e| format!("write {rel}: {e}"))?;
                rewritten.push(rel);
            }
        }
    }
    let gradle_rel = "platform/android/app/build.gradle.kts";
    let gradle = root.join(gradle_rel);
    if let Ok(text) = std::fs::read_to_string(&gradle) {
        let updated = point_gradle_at_host(&text);
        if updated != text {
            std::fs::write(&gradle, updated).map_err(|e| format!("write {gradle_rel}: {e}"))?;
            rewritten.push(gradle_rel.to_string());
        }
    }
    let ignore = root.join(".gitignore");
    let text = std::fs::read_to_string(&ignore).unwrap_or_default();
    let updated = gitignore_with_links(&text, &hrel);
    if updated != text {
        std::fs::write(&ignore, updated).map_err(|e| format!("write .gitignore: {e}"))?;
        rewritten.push(".gitignore".to_string());
    }

    for rel in &removed {
        status("Removed", rel);
    }
    for rel in &rewritten {
        status("Rewrote", rel);
    }
    for rel in &kept {
        status(
            "Kept",
            &format!("{rel} (not a generated file — a source override, or edited by hand)"),
        );
    }
    status(
        "Migrated",
        &format!(
            "{} derived file(s) removed, {} project file(s) rewritten — review `git status`, \
             then commit the deletions",
            removed.len(),
            rewritten.len()
        ),
    );
    ensure(project, &[])
}

fn remove_empty_dirs(dir: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        if entry.path().is_dir() {
            let _ = remove_empty_dirs(&entry.path());
        }
    }
    if std::fs::read_dir(dir)?.next().is_none() {
        std::fs::remove_dir(dir)?;
    }
    Ok(())
}

/// Repoint an Xcode project's `Assets.xcassets` file reference at the generated catalog.
/// Idempotent: the template's reference (`path = Assets.xcassets`) becomes a named reference
/// with a relative path, and an already-repointed project is returned unchanged.
pub(crate) fn point_xcode_at_host(pbxproj: &str, host: &str) -> String {
    let target = format!("../../{HOST_DIR}/{host}/Assets.xcassets");
    pbxproj.replace(
        "lastKnownFileType = folder.assetcatalog; path = Assets.xcassets; sourceTree = \"<group>\";",
        &format!(
            "lastKnownFileType = folder.assetcatalog; name = Assets.xcassets; path = \"{target}\"; sourceTree = \"<group>\";"
        ),
    )
}

/// Add the generated resource directory to the Gradle module's `res` source set, beside the
/// staged-images line the template already carries. Idempotent.
pub(crate) fn point_gradle_at_host(gradle: &str) -> String {
    let host_line = format!(
        "            res.srcDir(rootProject.projectDir.resolve(\"../../{HOST_DIR}/android/res\"))\n"
    );
    if gradle.contains(host_line.trim()) {
        return gradle.to_string();
    }
    let anchor =
        "            res.srcDir(rootProject.projectDir.resolve(\"../../build/day/android/res\"))\n";
    match gradle.find(anchor) {
        Some(i) => {
            let end = i + anchor.len();
            format!(
                "{}            // The launcher icon set (mipmaps, the adaptive and themed drawables), rendered by\n            // `day prepare` from resource/icons/icon.svg (docs/icons.md). Never checked in.\n{}{}",
                &gradle[..end],
                host_line,
                &gradle[end..]
            )
        }
        None => gradle.to_string(),
    }
}

/// The HarmonyOS media links belong in `.gitignore`: they are created by `day prepare`, and
/// a committed symlink would point at a build directory the clone does not have yet.
pub(crate) fn gitignore_with_links(text: &str, harmony_rel: &str) -> String {
    let lines = [
        format!("/{harmony_rel}/entry/src/main/resources/base/media"),
        format!("/{harmony_rel}/AppScope/resources/base/media"),
    ];
    if lines.iter().all(|l| text.lines().any(|t| t == l)) {
        return text.to_string();
    }
    let mut out = text.to_string();
    if !out.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    out.push_str(
        "\n# HarmonyOS reads its icon media from a fixed place under each module, so `day prepare`\n\
         # links these two directories to the generated set under build/day/host/harmony/media.\n",
    );
    for l in lines {
        if !text.lines().any(|t| t == l) {
            out.push_str(&l);
            out.push('\n');
        }
    }
    out
}

fn generator_id() -> String {
    format!(
        "day-cli {} ({})",
        env!("CARGO_PKG_VERSION"),
        day_vector::ENGINE
    )
}

fn discover(project: &Project, explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        let p = if p.is_absolute() {
            p.to_path_buf()
        } else {
            project.root.join(p)
        };
        return if p.exists() {
            Ok(p)
        } else {
            Err(format!("master {} does not exist", p.display()))
        };
    }
    for candidate in [
        "resource/icons/icon.svg",
        "resource/icons/day-icon.svg",
        "resource/icons/icon.png",
    ] {
        let p = project.root.join(candidate);
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(
        "no icon master — add resource/icons/icon.svg (layered via day: group ids, see \
         docs/icons.md), or icon.png for a raster-only set"
            .into(),
    )
}

// ---------------------------------------------------------------------------
// Generation: master → Vec<(project-relative path, bytes)>
// ---------------------------------------------------------------------------

fn generate(
    project: &Project,
    master: &Path,
    families: &[Family],
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let master_art = load_art(master)?;
    // Per-family overrides beside the master (`resource/icons/ios.svg`, …): a whole other
    // drawing for one platform, which is how a designer ships the macOS margin variant or a
    // simpler Android glyph without forking the master.
    let mut overrides: Vec<(Family, Art)> = Vec::new();
    for f in families {
        let path = project
            .root
            .join(format!("resource/icons/{}.svg", family_dir(*f)));
        if path.is_file() {
            overrides.push((*f, load_art(&path)?));
        }
    }
    let art_of = |f: Family| -> &Art {
        overrides
            .iter()
            .find(|(of, _)| *of == f)
            .map(|(_, a)| a)
            .unwrap_or(&master_art)
    };

    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let has = |f: Family| families.contains(&f);
    let host = |f: Family, rest: &str| format!("{HOST_DIR}/{}/{rest}", family_dir(f));

    if has(Family::Png) {
        let art = art_of(Family::Png);
        // 192 is the web app manifest's required small icon (docs/web.md "Home screen and
        // offline"); the rest are the classic powers of two.
        for px in [16u32, 32, 64, 128, 192, 256, 512, 1024] {
            out.push((
                host(Family::Png, &format!("day-icon-{px}.png")),
                art.composite(px)?,
            ));
        }
    }
    if has(Family::Linux) {
        let art = art_of(Family::Linux);
        for px in [48u32, 128, 256, 512] {
            out.push((
                host(Family::Linux, &format!("day-icon-{px}.png")),
                art.composite(px)?,
            ));
        }
    }
    if has(Family::Windows) {
        let art = art_of(Family::Windows);
        out.push((
            host(Family::Windows, "day-icon-256.png"),
            art.composite(256)?,
        ));
        let entries: Vec<(u32, Vec<u8>)> = [16u32, 32, 48, 256]
            .iter()
            .map(|&px| Ok((px, art.composite(px)?)))
            .collect::<Result<_, String>>()?;
        out.push((
            host(Family::Windows, "day.ico"),
            day_vector::pack_ico(&entries),
        ));
    }
    if has(Family::Macos) {
        let art = art_of(Family::Macos);
        let mut icns_entries = Vec::new();
        for px in [16u32, 32, 64, 128, 256, 512, 1024] {
            let png = art.squircle(px)?;
            if px != 64 {
                // 64 is rendered only for the icns ladder — the export set never carried it.
                out.push((
                    host(Family::Macos, &format!("day-icon-macos-{px}.png")),
                    png.clone(),
                ));
            }
            if px == 1024 {
                // The asset catalog the macOS Xcode project references (§17.4): one 1024 px
                // rendition at the `mac` idiom, pre-shaped, since macOS applies no mask.
                out.push((
                    host(
                        Family::Macos,
                        "Assets.xcassets/AppIcon.appiconset/AppIcon-1024.png",
                    ),
                    png.clone(),
                ));
            }
            icns_entries.push((px, png));
        }
        out.push((
            host(Family::Macos, "day-icon.icns"),
            day_vector::pack_icns(&icns_entries)?,
        ));
        out.push((
            host(Family::Macos, "Assets.xcassets/Contents.json"),
            CATALOG_ROOT_JSON.as_bytes().to_vec(),
        ));
        out.push((
            host(
                Family::Macos,
                "Assets.xcassets/AppIcon.appiconset/Contents.json",
            ),
            MACOS_APPICONSET_JSON.as_bytes().to_vec(),
        ));
    }
    if has(Family::Ios) {
        let art = art_of(Family::Ios);
        // The catalog the iOS Xcode project references: one opaque 1024 px universal image
        // (App Store validation rejects alpha), which Xcode scales for every slot.
        out.push((
            host(
                Family::Ios,
                "Assets.xcassets/AppIcon.appiconset/AppIcon-1024.png",
            ),
            art.flat_composite(1024)?,
        ));
        out.push((
            host(Family::Ios, "Assets.xcassets/Contents.json"),
            CATALOG_ROOT_JSON.as_bytes().to_vec(),
        ));
        out.push((
            host(
                Family::Ios,
                "Assets.xcassets/AppIcon.appiconset/Contents.json",
            ),
            IOS_APPICONSET_JSON.as_bytes().to_vec(),
        ));
        // Icon Composer package (Xcode 26 Liquid Glass, docs/icons.md): SVG layers + icon.json.
        // A checked-in `resource/icons/ios/AppIcon.icon/` (tuned in Icon Composer) is copied
        // through as-is; otherwise a layered SVG master produces one.
        let composer = project.root.join(ICON_COMPOSER_OVERRIDE);
        if composer.is_dir() {
            for path in walk_files(&composer) {
                if let (Ok(rel), Ok(bytes)) = (path.strip_prefix(&composer), std::fs::read(&path)) {
                    out.push((
                        host(
                            Family::Ios,
                            &format!("AppIcon.icon/{}", rel.to_string_lossy()),
                        ),
                        bytes,
                    ));
                }
            }
        } else if let Some(files) = art.icon_composer_package()? {
            for (name, bytes) in files {
                out.push((host(Family::Ios, &format!("AppIcon.icon/{name}")), bytes));
            }
        }
    }
    if has(Family::Android) {
        let art = art_of(Family::Android);
        let fg = art.adaptive_foreground()?;
        let bg = art.adaptive_background()?;
        let legacy = art.flat_composite(192)?;
        let play = art.flat_composite(512)?;
        // Store collateral, beside the resource tree rather than inside it.
        out.push((host(Family::Android, "play-store-512.png"), play));
        out.push((
            host(Family::Android, "ic_launcher-legacy-192.png"),
            legacy.clone(),
        ));
        // The resource tree Gradle adds as a source set (`res.srcDir`, docs/project-structure.md).
        out.push((
            host(Family::Android, "res/mipmap-xxxhdpi/ic_launcher.png"),
            legacy,
        ));
        out.push((
            host(
                Family::Android,
                "res/mipmap-xxxhdpi/ic_launcher_foreground.png",
            ),
            fg,
        ));
        out.push((
            host(
                Family::Android,
                "res/mipmap-xxxhdpi/ic_launcher_background.png",
            ),
            bg,
        ));
        // Themed icon (Android 13, docs/icons.md): a monochrome layer the system tints. A
        // `day:monochrome` layer becomes a VectorDrawable when it fits the subset; otherwise
        // (and for unlayered/raster masters) the adaptive foreground's alpha serves as the
        // mask, as a bitmap drawable. The adaptive-icon XML is generated with it.
        let mono_rel = match art.monochrome_drawable()? {
            MonoDrawable::Vector(xml) => {
                out.push((
                    host(Family::Android, "res/drawable/ic_launcher_monochrome.xml"),
                    xml.into_bytes(),
                ));
                "@drawable/ic_launcher_monochrome"
            }
            MonoDrawable::Bitmap(png) => {
                out.push((
                    host(
                        Family::Android,
                        "res/drawable-xxxhdpi/ic_launcher_monochrome.png",
                    ),
                    png,
                ));
                "@drawable/ic_launcher_monochrome"
            }
        };
        out.push((
            host(Family::Android, "res/mipmap-anydpi-v26/ic_launcher.xml"),
            format!(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
                 <!-- Adaptive launcher icon (API 26+): motif inside the 66dp safe zone on the 108dp canvas;\n\
                 \x20    the launcher applies its own mask/shape. Generated by `day prepare` (docs/icons.md). -->\n\
                 <adaptive-icon xmlns:android=\"http://schemas.android.com/apk/res/android\">\n\
                 \x20   <background android:drawable=\"@mipmap/ic_launcher_background\" />\n\
                 \x20   <foreground android:drawable=\"@mipmap/ic_launcher_foreground\" />\n\
                 \x20   <monochrome android:drawable=\"{mono_rel}\" />\n\
                 </adaptive-icon>\n"
            )
            .into_bytes(),
        ));
    }
    if has(Family::Ohos) {
        let art = art_of(Family::Ohos);
        // Layered icon (docs/icons.md): fg/bg media + the layered_image.json descriptor, wired
        // into app.json5/module.json5's icon slots (startWindowIcon keeps the flat startIcon).
        // One media set; hvigor's two resource roots are links to it (`harmony_links`).
        const LAYERED: &str = "{\n  \"layered-image\": {\n    \"background\": \"$media:background\",\n    \"foreground\": \"$media:foreground\"\n  }\n}\n";
        out.push((
            host(Family::Ohos, "media/startIcon.png"),
            art.flat_composite(512)?,
        ));
        out.push((
            host(Family::Ohos, "media/foreground.png"),
            art.ohos_foreground()?,
        ));
        out.push((
            host(Family::Ohos, "media/background.png"),
            art.ohos_background()?,
        ));
        out.push((
            host(Family::Ohos, "media/layered_image.json"),
            LAYERED.as_bytes().to_vec(),
        ));
        // The manifests are SOURCE files under platform/; this is the one edit prepare makes to
        // one, and only when it still names the flat icon. Idempotent.
        let hroot = crate::ohos::harmony_dir(project);
        let hrel = hroot
            .strip_prefix(&project.root)
            .unwrap_or(&hroot)
            .to_string_lossy()
            .into_owned();
        for manifest in [
            format!("{hrel}/AppScope/app.json5"),
            format!("{hrel}/entry/src/main/module.json5"),
        ] {
            let path = project.root.join(&manifest);
            if let Ok(text) = std::fs::read_to_string(&path) {
                let updated = text.replace(
                    "\"icon\": \"$media:startIcon\"",
                    "\"icon\": \"$media:layered_image\"",
                );
                if updated != text {
                    out.push((manifest, updated.into_bytes()));
                }
            }
        }
    }
    Ok(out)
}

/// A master or override file as renderable art.
fn load_art(path: &Path) -> Result<Art, String> {
    let is_svg = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    if is_svg {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        if text.contains("<text") {
            return Err(format!(
                "{} contains <text> — outline text in your editor (text shaping is \
                 deliberately not compiled into day; see docs/icons.md)",
                path.display()
            ));
        }
        Art::from_svg(&text)
    } else {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        Art::from_png(&bytes)
    }
}

/// The asset-catalog manifests the Xcode projects compile (what the template used to carry).
const CATALOG_ROOT_JSON: &str = "{ \"info\" : { \"author\" : \"day\", \"version\" : 1 } }\n";
const IOS_APPICONSET_JSON: &str = "{\n  \"images\" : [\n    { \"filename\" : \"AppIcon-1024.png\", \"idiom\" : \"universal\", \"platform\" : \"ios\", \"size\" : \"1024x1024\" }\n  ],\n  \"info\" : { \"author\" : \"day\", \"version\" : 1 }\n}\n";
const MACOS_APPICONSET_JSON: &str = "{\n  \"images\" : [\n    {\n      \"filename\" : \"AppIcon-1024.png\",\n      \"idiom\" : \"mac\",\n      \"scale\" : \"2x\",\n      \"size\" : \"512x512\"\n    }\n  ],\n  \"info\" : {\n    \"author\" : \"day\",\n    \"version\" : 1\n  }\n}\n";

// ---------------------------------------------------------------------------
// The master's renderable forms
// ---------------------------------------------------------------------------

/// `.icon` package members: (bundle-relative name, bytes).
type PackageFiles = Vec<(String, Vec<u8>)>;

/// A monochrome themed-icon drawable: VectorDrawable XML, or a bitmap alpha mask.
enum MonoDrawable {
    Vector(String),
    Bitmap(Vec<u8>),
}

/// A prepared master: SVG documents per role, or a decoded raster.
enum Art {
    Svg {
        /// Whole art minus the reserved layers — every full-bleed output.
        composite: String,
        /// Foreground-only document (layered masters), pre-tightened to its content box.
        foreground: Option<String>,
        /// Background-only document (layered masters).
        background: Option<String>,
        /// Monochrome-only document (`day:monochrome`) — Android themed icons, `.icon` Tinted.
        monochrome: Option<String>,
    },
    Raster(tiny_skia::Pixmap),
}

impl Art {
    fn from_svg(text: &str) -> Result<Art, String> {
        let layers = day_layers(text)?;
        let composite = splice_out(text, &[&layers.monochrome, &layers.dark]);
        // Sanity-parse now so errors carry the master's name, not an output's.
        day_vector::parse(composite.as_bytes())?;
        let (foreground, background) =
            if !layers.background.is_empty() || !layers.foreground.is_empty() {
                let fg = splice_out(
                    text,
                    &[&layers.background, &layers.monochrome, &layers.dark],
                );
                let bg_ranges: Vec<Range<usize>> = layers.foreground.clone();
                let bg = splice_out(text, &[&bg_ranges, &layers.monochrome, &layers.dark]);
                (Some(fg), Some(bg))
            } else {
                (None, None)
            };
        let monochrome = if layers.monochrome.is_empty() {
            None
        } else {
            let mut keep = vec![&layers.background, &layers.foreground, &layers.dark];
            let bg_fg_dark: Vec<Range<usize>> =
                keep.drain(..).flat_map(|v| v.iter().cloned()).collect();
            Some(unhide_layer(
                splice_out(text, &[&bg_fg_dark]),
                "day:monochrome",
            ))
        };
        Ok(Art::Svg {
            composite,
            foreground,
            background,
            monochrome,
        })
    }

    fn from_png(bytes: &[u8]) -> Result<Art, String> {
        let pm = tiny_skia::Pixmap::decode_png(bytes).map_err(|e| format!("png master: {e}"))?;
        if pm.width() < 1024 || pm.height() < 1024 {
            status(
                "Warning",
                &format!(
                    "png master is {}×{} — 1024×1024 or larger avoids upscaled large slots",
                    pm.width(),
                    pm.height()
                ),
            );
        }
        Ok(Art::Raster(pm))
    }

    /// Full-bleed square render (transparent background preserved).
    fn composite(&self, px: u32) -> Result<Vec<u8>, String> {
        match self {
            Art::Svg { composite, .. } => {
                let tree = day_vector::parse(composite.as_bytes())?;
                day_vector::render_png(&tree, px)
            }
            Art::Raster(pm) => scale_png(pm, px),
        }
    }

    /// Composite flattened over the derived background color (opaque — iOS/store slots).
    fn flat_composite(&self, px: u32) -> Result<Vec<u8>, String> {
        let png = self.composite(px)?;
        flatten(&png, self.backdrop()?)
    }

    /// The macOS shape: art inset 824/1024 with the 184-radius rounded clip, transparent margin.
    fn squircle(&self, px: u32) -> Result<Vec<u8>, String> {
        match self {
            Art::Svg { composite, .. } => {
                let vb = view_box(composite)?;
                let inner = inner_markup(composite)?;
                let doc = format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1024 1024\">\
                     <defs><clipPath id=\"day-squircle\"><rect x=\"{MAC_INSET}\" y=\"{MAC_INSET}\" width=\"{MAC_ART}\" height=\"{MAC_ART}\" rx=\"{MAC_RADIUS}\"/></clipPath></defs>\
                     <g clip-path=\"url(#day-squircle)\"><svg x=\"{MAC_INSET}\" y=\"{MAC_INSET}\" width=\"{MAC_ART}\" height=\"{MAC_ART}\" viewBox=\"{vb}\" preserveAspectRatio=\"xMidYMid slice\">{inner}</svg></g></svg>"
                );
                let tree = day_vector::parse(doc.as_bytes())?;
                day_vector::render_png(&tree, px)
            }
            Art::Raster(pm) => {
                let art_px = (px as f32 * MAC_ART / 1024.0).round() as u32;
                let inset = ((px as f32 - art_px as f32) / 2.0).round() as i32;
                let radius = px as f32 * MAC_RADIUS / 1024.0;
                let scaled = tiny_skia::Pixmap::decode_png(&scale_png(pm, art_px)?)
                    .map_err(|e| e.to_string())?;
                let mut canvas = tiny_skia::Pixmap::new(px, px).ok_or("pixmap")?;
                canvas.draw_pixmap(
                    inset,
                    inset,
                    scaled.as_ref(),
                    &tiny_skia::PixmapPaint::default(),
                    tiny_skia::Transform::identity(),
                    None,
                );
                apply_round_mask(&mut canvas, inset as f32, art_px as f32, radius);
                canvas.encode_png().map_err(|e| e.to_string())
            }
        }
    }

    /// The adaptive foreground: content tightened, centered in the 66/108 safe zone, transparent.
    fn adaptive_foreground(&self) -> Result<Vec<u8>, String> {
        let inset = (ADAPTIVE_PX as f32 - SAFE_PX) / 2.0;
        match self {
            Art::Svg {
                foreground: Some(fg),
                ..
            } => {
                let tree = day_vector::parse(fg.as_bytes())?;
                let b = day_vector::content_bbox(&tree)
                    .ok_or("the day:foreground layers render no content")?;
                let (bx, by, bw, bh) = bbox_in_viewbox_units(fg, &tree, b)?;
                let inner = inner_markup(fg)?;
                let doc = format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {ADAPTIVE_PX} {ADAPTIVE_PX}\">\
                     <svg x=\"{inset}\" y=\"{inset}\" width=\"{SAFE_PX}\" height=\"{SAFE_PX}\" viewBox=\"{bx} {by} {bw} {bh}\" preserveAspectRatio=\"xMidYMid meet\">{inner}</svg></svg>",
                );
                let tree = day_vector::parse(doc.as_bytes())?;
                day_vector::render_png(&tree, ADAPTIVE_PX)
            }
            // Unlayered/raster: the whole art in the safe zone (over the derived background).
            _ => {
                let art = tiny_skia::Pixmap::decode_png(&self.composite(SAFE_PX as u32)?)
                    .map_err(|e| e.to_string())?;
                let mut canvas =
                    tiny_skia::Pixmap::new(ADAPTIVE_PX, ADAPTIVE_PX).ok_or("pixmap")?;
                canvas.draw_pixmap(
                    inset as i32,
                    inset as i32,
                    art.as_ref(),
                    &tiny_skia::PixmapPaint::default(),
                    tiny_skia::Transform::identity(),
                    None,
                );
                canvas.encode_png().map_err(|e| e.to_string())
            }
        }
    }

    /// The adaptive background: the day:background layer full-bleed, else the derived color.
    fn adaptive_background(&self) -> Result<Vec<u8>, String> {
        if let Art::Svg {
            background: Some(bg),
            ..
        } = self
        {
            let vb = view_box(bg)?;
            let inner = inner_markup(bg)?;
            let doc = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {ADAPTIVE_PX} {ADAPTIVE_PX}\">\
                 <svg width=\"{ADAPTIVE_PX}\" height=\"{ADAPTIVE_PX}\" viewBox=\"{vb}\" preserveAspectRatio=\"xMidYMid slice\">{inner}</svg></svg>"
            );
            let tree = day_vector::parse(doc.as_bytes())?;
            return day_vector::render_png(&tree, ADAPTIVE_PX);
        }
        let mut pm = tiny_skia::Pixmap::new(ADAPTIVE_PX, ADAPTIVE_PX).ok_or("pixmap")?;
        let c = self.backdrop()?;
        pm.fill(c);
        pm.encode_png().map_err(|e| e.to_string())
    }

    /// The Android themed-icon monochrome drawable: the `day:monochrome` layer as a
    /// VectorDrawable when it fits the subset, else (and without the layer) the adaptive
    /// foreground's alpha as a bitmap mask — the system tints either.
    fn monochrome_drawable(&self) -> Result<MonoDrawable, String> {
        if let Art::Svg {
            monochrome: Some(mono),
            ..
        } = self
        {
            let tree = day_vector::parse(mono.as_bytes())?;
            if let Some(b) = day_vector::content_bbox(&tree) {
                let (bx, by, bw, bh) = bbox_in_viewbox_units(mono, &tree, b)?;
                let inset = (ADAPTIVE_PX as f32 - SAFE_PX) / 2.0;
                let inner = inner_markup(mono)?;
                // Safe-zone fit as an EXPLICIT transform, not a nested <svg> viewport: usvg
                // models a nested svg as a clipped group, which is outside the
                // VectorDrawable subset — the very conversion this document exists for. The
                // math is `xMidYMid meet` by hand: uniform scale to the safe square,
                // centered, then offset by the zone inset.
                let s = SAFE_PX / bw.max(bh).max(1e-3);
                let tx = inset + (SAFE_PX - s * bw) / 2.0 - s * bx;
                let ty = inset + (SAFE_PX - s * bh) / 2.0 - s * by;
                let doc = format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {ADAPTIVE_PX} {ADAPTIVE_PX}\">\
                     <g transform=\"translate({tx} {ty}) scale({s})\">{inner}</g></svg>"
                );
                let boxed = day_vector::parse(doc.as_bytes())?;
                match day_vector::to_vector_drawable(&boxed) {
                    Ok(xml) => return Ok(MonoDrawable::Vector(xml)),
                    Err(why) => status(
                        "Warning",
                        &format!(
                            "day:monochrome → bitmap mask ({why} is outside the VectorDrawable subset)"
                        ),
                    ),
                }
                return Ok(MonoDrawable::Bitmap(day_vector::render_png(
                    &boxed,
                    ADAPTIVE_PX,
                )?));
            }
        }
        Ok(MonoDrawable::Bitmap(self.adaptive_foreground()?))
    }

    /// The HarmonyOS layered-icon foreground: the motif centered in a safe zone on a 216 canvas.
    fn ohos_foreground(&self) -> Result<Vec<u8>, String> {
        // Reuse the Android adaptive geometry, downscaled to the OHOS 216 canvas.
        let png = self.adaptive_foreground()?;
        let pm = tiny_skia::Pixmap::decode_png(&png).map_err(|e| e.to_string())?;
        scale_png(&pm, 216)
    }

    /// The HarmonyOS layered-icon background: full-bleed at 216.
    fn ohos_background(&self) -> Result<Vec<u8>, String> {
        let png = self.adaptive_background()?;
        let pm = tiny_skia::Pixmap::decode_png(&png).map_err(|e| e.to_string())?;
        scale_png(&pm, 216)
    }

    /// The Icon Composer `.icon` package files (Xcode 26): SVG layer assets + `icon.json`.
    /// Only for layered SVG masters — the package's value IS the layer split; a flat master
    /// has nothing to feed the Liquid Glass modes.
    fn icon_composer_package(&self) -> Result<Option<PackageFiles>, String> {
        let Art::Svg {
            foreground: Some(fg),
            background,
            monochrome,
            ..
        } = self
        else {
            return Ok(None);
        };
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        // Layer assets, top-first in the group below.
        files.push(("Assets/foreground.svg".into(), fg.clone().into_bytes()));
        let mut layers = vec![serde_json::json!({
            "image-name": "foreground.svg",
            "name": "foreground",
        })];
        if let Some(bg) = background {
            files.push(("Assets/background.svg".into(), bg.clone().into_bytes()));
            layers.push(serde_json::json!({
                "image-name": "background.svg",
                "name": "background",
            }));
        }
        if let Some(mono) = monochrome {
            // Not referenced by the default group: Icon Composer assigns it to the Tinted
            // appearance when the designer opts in — shipping the asset makes that a drag.
            files.push(("Assets/monochrome.svg".into(), mono.clone().into_bytes()));
        }
        let json = serde_json::json!({
            "fill": "automatic",
            "groups": [ { "layers": layers } ],
            "supported-platforms": { "squares": "shared" },
        });
        files.push((
            "icon.json".into(),
            (serde_json::to_string_pretty(&json).unwrap_or_default() + "\n").into_bytes(),
        ));
        Ok(Some(files))
    }

    /// The flattening/back-plate color: the composite's corner pixel (a full-bleed master's own
    /// background), or white when the corner is transparent.
    fn backdrop(&self) -> Result<tiny_skia::Color, String> {
        let pm = tiny_skia::Pixmap::decode_png(&self.composite(64)?).map_err(|e| e.to_string())?;
        let px = pm.pixel(1, 1).ok_or("empty pixmap")?;
        if px.alpha() == 255 {
            Ok(tiny_skia::Color::from_rgba8(
                px.red(),
                px.green(),
                px.blue(),
                255,
            ))
        } else {
            Ok(tiny_skia::Color::WHITE)
        }
    }
}

// ---------------------------------------------------------------------------
// SVG text helpers (layer slicing is textual — see the module docs)
// ---------------------------------------------------------------------------

struct Layers {
    background: Vec<Range<usize>>,
    foreground: Vec<Range<usize>>,
    monochrome: Vec<Range<usize>>,
    dark: Vec<Range<usize>>,
}

fn day_layers(xml: &str) -> Result<Layers, String> {
    let doc = day_vector::roxmltree::Document::parse(xml).map_err(|e| format!("master: {e}"))?;
    let mut layers = Layers {
        background: Vec::new(),
        foreground: Vec::new(),
        monochrome: Vec::new(),
        dark: Vec::new(),
    };
    for child in doc.root_element().children() {
        let Some(id) = child.attribute("id") else {
            continue;
        };
        match id {
            "day:background" => layers.background.push(child.range()),
            "day:monochrome" => layers.monochrome.push(child.range()),
            "day:dark" => layers.dark.push(child.range()),
            _ if id.starts_with("day:foreground") => layers.foreground.push(child.range()),
            _ => {}
        }
    }
    Ok(layers)
}

/// The document with the given ranges removed (spliced back-to-front so offsets stay valid).
fn splice_out(xml: &str, removals: &[&Vec<Range<usize>>]) -> String {
    let mut ranges: Vec<Range<usize>> = removals.iter().flat_map(|v| v.iter().cloned()).collect();
    ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
    let mut out = xml.to_string();
    for r in ranges {
        out.replace_range(r, "");
    }
    out
}

/// Drop a `display="none"` from the named layer's OPEN TAG. A master may hide a reserved
/// layer (`day:monochrome`, `day:dark`) so plain SVG viewers show the icon as shipped — the
/// generated masters do — and the layer-only documents re-enable it here.
fn unhide_layer(doc: String, id: &str) -> String {
    let Some(at) = doc.find(&format!("id=\"{id}\"")) else {
        return doc;
    };
    let Some(end) = doc[at..].find('>').map(|e| at + e) else {
        return doc;
    };
    match doc[at..end].find(" display=\"none\"") {
        Some(rel) => {
            let mut out = doc;
            out.replace_range(at + rel..at + rel + " display=\"none\"".len(), "");
            out
        }
        None => doc,
    }
}

/// The root `<svg>`'s viewBox, or one derived from width/height.
/// A usvg content box mapped back into the document's own viewBox units.
///
/// usvg normalizes a parsed tree to the svg's width/height, so when a master declares e.g.
/// `viewBox="0 0 120 120" width="1024"`, [`day_vector::content_bbox`] answers in 1024-space —
/// while the raw inner markup the safe-zone wrappers re-parse is still in 120-space. Windowing
/// the markup with unconverted bounds selects a region outside the art entirely (an empty
/// adaptive foreground). Identity when the viewBox and the tree size already agree.
fn bbox_in_viewbox_units(
    doc: &str,
    tree: &day_vector::usvg::Tree,
    b: day_vector::usvg::Rect,
) -> Result<(f32, f32, f32, f32), String> {
    let vb = view_box(doc)?;
    let parts: Vec<f32> = vb
        .split_whitespace()
        .filter_map(|p| p.parse::<f32>().ok())
        .collect();
    let [vx, vy, vw, vh] = parts.as_slice() else {
        return Err(format!("unparseable viewBox {vb:?}"));
    };
    let size = tree.size();
    let sx = vw / size.width().max(1e-6);
    let sy = vh / size.height().max(1e-6);
    Ok((
        vx + b.x() * sx,
        vy + b.y() * sy,
        b.width() * sx,
        b.height() * sy,
    ))
}

fn view_box(xml: &str) -> Result<String, String> {
    let doc = day_vector::roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if let Some(vb) = root.attribute("viewBox") {
        return Ok(vb.to_string());
    }
    let dim = |a: &str| {
        root.attribute(a)
            .and_then(|s| s.trim_end_matches("px").parse::<f32>().ok())
    };
    match (dim("width"), dim("height")) {
        (Some(w), Some(h)) => Ok(format!("0 0 {w} {h}")),
        _ => Err("the master SVG has neither viewBox nor width/height".into()),
    }
}

/// Everything between the root `<svg …>` tag and its `</svg>` close.
fn inner_markup(xml: &str) -> Result<&str, String> {
    let open_end = xml
        .find("<svg")
        .and_then(|at| xml[at..].find('>').map(|o| at + o + 1))
        .ok_or("no <svg> root")?;
    let close = xml.rfind("</svg>").ok_or("no </svg>")?;
    Ok(&xml[open_end..close])
}

// ---------------------------------------------------------------------------
// Raster helpers
// ---------------------------------------------------------------------------

fn scale_png(pm: &tiny_skia::Pixmap, px: u32) -> Result<Vec<u8>, String> {
    let mut out = tiny_skia::Pixmap::new(px, px).ok_or("pixmap")?;
    let scale = px as f32 / pm.width().max(pm.height()) as f32;
    let paint = tiny_skia::PixmapPaint {
        quality: tiny_skia::FilterQuality::Bilinear,
        ..Default::default()
    };
    out.draw_pixmap(
        0,
        0,
        pm.as_ref(),
        &paint,
        tiny_skia::Transform::from_scale(scale, scale),
        None,
    );
    out.encode_png().map_err(|e| e.to_string())
}

fn flatten(png: &[u8], color: tiny_skia::Color) -> Result<Vec<u8>, String> {
    let img = tiny_skia::Pixmap::decode_png(png).map_err(|e| e.to_string())?;
    let mut base = tiny_skia::Pixmap::new(img.width(), img.height()).ok_or("pixmap")?;
    base.fill(color);
    base.draw_pixmap(
        0,
        0,
        img.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        tiny_skia::Transform::identity(),
        None,
    );
    base.encode_png().map_err(|e| e.to_string())
}

/// Intersect the canvas alpha with a rounded rect (the raster-master squircle mask).
fn apply_round_mask(canvas: &mut tiny_skia::Pixmap, inset: f32, edge: f32, radius: f32) {
    let mut mask = match tiny_skia::Pixmap::new(canvas.width(), canvas.height()) {
        Some(m) => m,
        None => return,
    };
    let mut pb = tiny_skia::PathBuilder::new();
    // Rounded rect from four lines + four cubic corners (kappa circle approximation).
    let k = 0.5523 * radius;
    let (x0, y0, x1, y1) = (inset, inset, inset + edge, inset + edge);
    pb.move_to(x0 + radius, y0);
    pb.line_to(x1 - radius, y0);
    pb.cubic_to(x1 - radius + k, y0, x1, y0 + radius - k, x1, y0 + radius);
    pb.line_to(x1, y1 - radius);
    pb.cubic_to(x1, y1 - radius + k, x1 - radius + k, y1, x1 - radius, y1);
    pb.line_to(x0 + radius, y1);
    pb.cubic_to(x0 + radius - k, y1, x0, y1 - radius + k, x0, y1 - radius);
    pb.line_to(x0, y0 + radius);
    pb.cubic_to(x0, y0 + radius - k, x0 + radius - k, y0, x0 + radius, y0);
    pb.close();
    let Some(path) = pb.finish() else { return };
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(tiny_skia::Color::WHITE);
    paint.anti_alias = true;
    mask.fill_path(
        &path,
        &paint,
        tiny_skia::FillRule::Winding,
        tiny_skia::Transform::identity(),
        None,
    );
    // canvas.alpha *= mask.alpha, per pixel.
    let mask_px: Vec<u8> = mask.pixels().iter().map(|p| p.alpha()).collect();
    for (px, m) in canvas.pixels_mut().iter_mut().zip(mask_px) {
        let a = (px.alpha() as u16 * m as u16 / 255) as u8;
        let scale = if px.alpha() == 0 {
            0.0
        } else {
            a as f32 / px.alpha() as f32
        };
        *px = tiny_skia::PremultipliedColorU8::from_rgba(
            (px.red() as f32 * scale) as u8,
            (px.green() as f32 * scale) as u8,
            (px.blue() as f32 * scale) as u8,
            a,
        )
        .unwrap_or(*px);
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYERED: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\">\
        <defs><linearGradient id=\"g\"/></defs>\
        <rect id=\"day:background\" width=\"100\" height=\"100\" fill=\"#123456\"/>\
        <g id=\"day:foreground\"><circle cx=\"50\" cy=\"50\" r=\"20\" fill=\"#fff\"/></g>\
        <g id=\"day:monochrome\"><circle cx=\"50\" cy=\"50\" r=\"20\"/></g></svg>";

    #[test]
    fn link_relative_climbs_to_the_root_in_host_separators() {
        let rel = link_relative(
            "platform/harmony/entry/src/main/resources/base/media",
            "build/day/host/harmony/media",
        );
        let expected: PathBuf = ["..", "..", "..", "..", "..", "..", ".."]
            .iter()
            .chain(["build", "day", "host", "harmony", "media"].iter())
            .collect();
        assert_eq!(rel, expected);
        // No `/` survives on a host whose separator is `\`: the reparse resolver reads it as
        // part of a name.
        let text = rel.to_string_lossy();
        assert!(!text.contains(if cfg!(windows) { '/' } else { '\\' }));
        assert_eq!(text.matches(std::path::MAIN_SEPARATOR).count(), 11);
    }

    #[test]
    fn layers_split_and_splice() {
        let l = day_layers(LAYERED).unwrap();
        assert_eq!(l.background.len(), 1);
        assert_eq!(l.foreground.len(), 1);
        assert_eq!(l.monochrome.len(), 1);
        // Composite drops the monochrome layer but keeps bg+fg+defs.
        let composite = splice_out(LAYERED, &[&l.monochrome, &l.dark]);
        assert!(composite.contains("day:background"));
        assert!(composite.contains("day:foreground"));
        assert!(!composite.contains("day:monochrome"));
        day_vector::parse(composite.as_bytes()).unwrap();
        // Foreground-only drops the background.
        let fg = splice_out(LAYERED, &[&l.background, &l.monochrome, &l.dark]);
        assert!(!fg.contains("day:background"));
        assert!(fg.contains("day:foreground"));
    }

    #[test]
    fn adaptive_foreground_survives_viewbox_size_mismatch() {
        // A master may declare `viewBox="0 0 120 120" width="1024"`. usvg reports content
        // bounds in 1024-space while the raw markup the safe-zone wrapper re-parses is in
        // 120-space; unconverted bounds window a region outside the art and the adaptive
        // foreground renders EMPTY (the Day-Showcase sunrise master, 2026-08-07).
        let master = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 120 120\" \
             width=\"1024\" height=\"1024\">\
             <rect id=\"day:background\" width=\"120\" height=\"120\" fill=\"#123456\"/>\
             <g id=\"day:foreground\"><circle cx=\"60\" cy=\"60\" r=\"30\" fill=\"#fff\"/></g>\
             </svg>";
        let art = Art::from_svg(master).unwrap();
        let png = art.adaptive_foreground().unwrap();
        let pm = tiny_skia::Pixmap::decode_png(&png).unwrap();
        let visible = pm.pixels().iter().filter(|p| p.alpha() > 0).count();
        assert!(
            visible > 1000,
            "adaptive foreground is (nearly) empty: {visible} visible px"
        );
    }

    #[test]
    fn view_box_falls_back_to_width_height() {
        assert_eq!(view_box(LAYERED).unwrap(), "0 0 100 100");
        let wh = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64px\" height=\"32\"></svg>";
        assert_eq!(view_box(wh).unwrap(), "0 0 64 32");
    }

    #[test]
    fn generated_families_cover_the_legacy_set() {
        // A pure-function sanity: family mapping is total over the shipping targets.
        for t in [
            "ios-uikit",
            "android-mdc",
            "harmony-arkui",
            "windows-xaml",
            "macos-appkit",
            "linux-gtk",
            "linux-qt",
            "web-dom",
        ] {
            assert!(!family_of_target(t).is_empty(), "{t} maps to no family");
        }
    }
}
