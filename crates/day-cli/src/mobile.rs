//! Mobile pipelines (DESIGN.md §16.5, §17.4): ios-uikit via xcodebuild + simctl (the Xcode
//! project's script phase calls back into `day xcode-backend build` for the Rust staticlib);
//! android-mdc via gradle + adb (the gradle scaffold calls `day gradle-backend build`).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::meta::{Project, find_project};
use crate::ops::{BuildOutcome, LaunchSpec, LogStream, emit_log, status};
use crate::targets::Target;

pub(crate) fn rustup_cargo() -> Result<(PathBuf, PathBuf), String> {
    // Shared lookup: honors RUSTUP_HOME and prefers a stable-* toolchain (docs/environment.md).
    day_toolchain::rustup_cargo()
}

/// Run an install/launch step without letting the tool narrate.
///
/// `adb`, `devicectl` and friends each describe the same three operations in their own voice
/// ("Performing Streamed Install", "App installed: • bundleID: …", "Starting: Intent { … }"), on
/// the same stream the app's own output arrives on. Day already says what is happening through
/// [`status`], in one format for every target — so the tool's version is captured and shown only
/// when the step fails, where it is the diagnostic. Build output still streams: there the tool's
/// narration IS the content.
pub(crate) fn run_quiet(cmd: &mut Command, what: &str) -> Result<(), String> {
    let out = crate::ops::run_capture(cmd, what)?;
    if out.status.success() {
        return Ok(());
    }
    if crate::ops::verbose() {
        // `--verbose` already streamed the tool's output live — don't echo the wall of text again.
        return Err(format!("{what} failed"));
    }
    Err(format!(
        "{what} failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ))
}

pub(crate) fn run_logged(cmd: &mut Command, what: &str) -> Result<(), String> {
    let out = cmd.status().map_err(|e| format!("{what}: {e}"))?;
    if out.success() {
        Ok(())
    } else {
        Err(format!("{what} failed"))
    }
}

/// Make a path absolute without requiring it to exist yet (build-output dirs often don't). Build-tool
/// arguments such as xcodebuild's `SYMROOT` MUST be absolute — a relative one is resolved per-target
/// against each target's own working directory, so an app target and its SwiftPM package dependencies
/// scatter their products into different trees.
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path))
    }
}

/// True when a failed xcodebuild is the "a package resource bundle isn't where the app target expected
/// it" class — a stale or split build tree. Worth one clean retry (see [`build_ios`]).
fn is_stale_bundle_failure(out: &std::process::Output) -> bool {
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .to_lowercase();
    all.contains(".bundle") && all.contains("no such file")
}

/// Distill a failed xcodebuild run into something readable. Raw xcodebuild output is mostly a wall of
/// `export FOO=bar` lines; the actionable content is the `error:` lines — surface those first (from
/// both streams), fall back to a non-`export` tail, and add a targeted hint for the resource-bundle
/// "no such file" failure class (a stale/split build tree).
fn diagnose_xcodebuild(out: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut errors: Vec<String> = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .filter(|l| l.starts_with("error:") || l.contains(": error:"))
        .map(str::to_string)
        .collect();
    errors.dedup();

    let mut msg = if errors.is_empty() {
        let tail: Vec<&str> = stdout
            .lines()
            .filter(|l| !l.trim_start().starts_with("export "))
            .rev()
            .take(20)
            .collect();
        tail.into_iter().rev().collect::<Vec<_>>().join("\n")
    } else {
        errors.join("\n")
    };

    let lower = format!("{stdout}{stderr}").to_lowercase();
    if lower.contains(".bundle") && lower.contains("no such file") {
        msg.push_str(
            "\n\nhint: a SwiftPM package resource bundle wasn't where the app target expected it. \
             This is usually a stale or split build tree — remove build/day/ios-uikit and retry \
             (day launch does this automatically on a resource-bundle failure).",
        );
    }
    msg
}

// ---------------------------------------------------------------------------
// xcode-backend: invoked BY the Xcode script phase with Xcode's env (§17.4)
// ---------------------------------------------------------------------------

pub fn xcode_backend_build() -> i32 {
    let get = |k: &str| std::env::var(k).ok();
    let configuration = get("CONFIGURATION").unwrap_or_else(|| "Debug".into());
    let built_products = match get("BUILT_PRODUCTS_DIR") {
        Some(v) => PathBuf::from(v),
        None => {
            eprintln!(
                "day xcode-backend: must run inside an Xcode build (BUILT_PRODUCTS_DIR unset)"
            );
            return 2;
        }
    };
    let platform = get("PLATFORM_NAME").unwrap_or_else(|| "iphonesimulator".into());
    let project_dir = get("PROJECT_DIR").map(PathBuf::from).unwrap_or_default();

    // platform/ios/ → project root two levels up.
    let root = project_dir.join("../..");
    let project = match find_project(Some(&root)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("day xcode-backend: {e}");
            return 2;
        }
    };
    let profile = if configuration.to_lowercase().contains("release") {
        "release"
    } else {
        "debug"
    };
    // macOS builds honor Xcode's ARCHS (host arch under ONLY_ACTIVE_ARCH; both for a
    // universal Release), lipo'd below when there is more than one.
    let (triples, toolkit_feature, target_dir_name): (Vec<&str>, &str, &str) =
        match platform.as_str() {
            "iphonesimulator" => (vec!["aarch64-apple-ios-sim"], "uikit", "ios-uikit"),
            "iphoneos" => (vec!["aarch64-apple-ios"], "uikit", "ios-uikit"),
            "macosx" => {
                let archs = get("ARCHS").unwrap_or_else(|| "arm64".into());
                let mut t = Vec::new();
                for arch in archs.split_whitespace() {
                    match arch {
                        "arm64" => t.push("aarch64-apple-darwin"),
                        "x86_64" => t.push("x86_64-apple-darwin"),
                        other => {
                            eprintln!("day xcode-backend: unsupported ARCHS entry {other:?}");
                            return 2;
                        }
                    }
                }
                (t, "appkit", "macos-appkit")
            }
            other => {
                eprintln!("day xcode-backend: unsupported PLATFORM_NAME {other:?}");
                return 2;
            }
        };
    let (cargo, bin) = match rustup_cargo() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("day xcode-backend: {e}");
            return 3;
        }
    };
    let name = project.manifest.app.name.clone();
    let target_dir = project
        .root
        .join("build/day/cargo")
        .join(target_dir_name)
        .join(profile);
    // One `cargo rustc` per requested arch (macOS universal Release builds ask for two).
    let mut arch_libs: Vec<PathBuf> = Vec::new();
    // Cargo names the artifact after the crate with `-` → `_` (`hello-day` ⇒ libhello_day.a);
    // the pbxproj links `-l<ident>` with the same spelling.
    let ident = name.replace('-', "_");
    for triple in &triples {
        let mut cmd = Command::new(&cargo);
        // Thinned ICU locale data for the declared locale set (crates/day-cli/src/intl.rs).
        crate::intl::apply(&mut cmd, &project);
        // Sanitize Xcode's script-phase env: SDKROOT points at the build SDK (poisoning
        // HOST compiles of proc-macro build scripts), and Xcode's PATH resolves `cc` to the raw
        // toolchain clang, which — unlike the /usr/bin/cc xcrun shim — does NOT auto-select an
        // SDK (ld: library 'System' not found). Reset both; rustc finds per-target SDKs via
        // xcrun.
        for var in [
            "SDKROOT",
            "LIBRARY_PATH",
            "CPATH",
            "IPHONEOS_DEPLOYMENT_TARGET",
            "MACOSX_DEPLOYMENT_TARGET",
        ] {
            cmd.env_remove(var);
        }
        let home = std::env::var("HOME").unwrap_or_default();
        cmd.current_dir(&project.root)
            .env(
                "PATH",
                format!(
                    "{}:{home}/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin",
                    bin.display()
                ),
            )
            .env("CARGO_TARGET_DIR", &target_dir);
        crate::ops::apply_app_identity(&mut cmd, &project);
        cmd
            // `rustc --crate-type staticlib` so the app lib's manifest can stay rlib-only (see
            // the `[lib]` note in the app Cargo.toml); produces the same `lib<name>.a` this
            // expects. `--features` = the toolkit + every standalone piece's `<pkg>/<toolkit>`
            // renderer feature (Tier A.2), so the app needn't re-list per-piece features in its
            // own Cargo.toml.
            .args([
                "rustc",
                "-p",
                &name,
                "--lib",
                "--crate-type",
                "staticlib",
                "--no-default-features",
                "--features",
                &crate::ops::feature_selection(&project, toolkit_feature),
            ])
            .args(["--target", triple]);
        if profile == "release" {
            cmd.arg("--release");
        }
        if run_logged(&mut cmd, "cargo (xcode)").is_err() {
            return 4;
        }
        arch_libs.push(
            target_dir
                .join(triple)
                .join(profile)
                .join(format!("lib{ident}.a")),
        );
    }
    let out_dir = built_products.join("day"); // must match pbxproj LIBRARY_SEARCH_PATHS `$(BUILT_PRODUCTS_DIR)/day`
    if std::fs::create_dir_all(&out_dir).is_err() {
        eprintln!("day xcode-backend: cannot create {}", out_dir.display());
        return 4;
    }
    let dest = out_dir.join(format!("lib{ident}.a"));
    let staged = if arch_libs.len() == 1 {
        std::fs::copy(&arch_libs[0], &dest)
            .map(|_| ())
            .map_err(|e| format!("copy {} → {}: {e}", arch_libs[0].display(), dest.display()))
    } else {
        // Universal: lipo the per-arch staticlibs into one (Xcode links a single file).
        let mut lipo = Command::new("lipo");
        lipo.arg("-create")
            .args(&arch_libs)
            .arg("-output")
            .arg(&dest);
        match lipo.status() {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("lipo exited with {s}")),
            Err(e) => Err(format!("lipo: {e}")),
        }
    };
    if let Err(e) = staged {
        eprintln!("day xcode-backend: {e}");
        return 4;
    }
    // Stage assets/ into the app bundle (§18.1's copy-phase mechanism).
    if let (Some(tbd), Some(res)) = (
        get("TARGET_BUILD_DIR"),
        get("UNLOCALIZED_RESOURCES_FOLDER_PATH"),
    ) {
        let src = project.root.join("resource/assets");
        if src.exists() {
            let dst = PathBuf::from(tbd).join(res).join("assets");
            let _ = std::fs::create_dir_all(&dst);
            if let Ok(entries) = std::fs::read_dir(&src) {
                for e in entries.flatten() {
                    let _ = std::fs::copy(e.path(), dst.join(e.file_name()));
                }
            }
        }
    }
    eprintln!("day xcode-backend: staged {}", dest.display());
    0
}

/// `day xcode-backend stage-resources` — the macOS host project's second script phase:
/// stage the project's images/assets/fonts and the vector trees into the bundle's
/// `Contents/Resources`, the exact layout the packed-app probes already resolve
/// (`../Resources/{images,assets,fonts,vectors/{svg,raster}}` — docs/vectors.md), so an
/// Xcode-built bundle needs no `DAY_*` environment at all. Runs the vector staging first,
/// so a build started from the Xcode IDE is self-contained.
pub fn xcode_backend_stage_resources() -> i32 {
    let get = |k: &str| std::env::var(k).ok();
    let (Some(tbd), Some(res)) = (
        get("TARGET_BUILD_DIR"),
        get("UNLOCALIZED_RESOURCES_FOLDER_PATH"),
    ) else {
        eprintln!("day xcode-backend: must run inside an Xcode build (TARGET_BUILD_DIR unset)");
        return 2;
    };
    let project_dir = get("PROJECT_DIR").map(PathBuf::from).unwrap_or_default();
    // platform/macos/ → project root two levels up.
    let project = match find_project(Some(&project_dir.join("../.."))) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("day xcode-backend: {e}");
            return 2;
        }
    };
    // Refresh the vector caches (raster + glyph SVGs) — cheap and idempotent, and an
    // IDE-initiated build has no earlier `day build` step to have done it.
    let vectors = match crate::resources::prepare_vectors(&project) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("day xcode-backend: vectors: {e}");
            return 4;
        }
    };
    // This host builds the appkit bundle, which renders the staged SVGs — so the raster tree it
    // carries is only whatever art could not be reduced to one (docs/vectors.md).
    if let Err(e) = crate::resources::write_vector_fallbacks(&project, "appkit", &vectors) {
        eprintln!("day xcode-backend: vectors: {e}");
        return 4;
    }
    let resources = PathBuf::from(tbd).join(res);
    let pairs: [(PathBuf, &str); 5] = [
        (project.root.join("resource/images"), "images"),
        (project.root.join("resource/assets"), "assets"),
        (project.root.join("resource/fonts"), "fonts"),
        (
            crate::resources::vector_fallback_dir(&project, "appkit"),
            "vectors/raster",
        ),
        (crate::resources::vector_svg_dir(&project), "vectors/svg"),
    ];
    for (src, sub) in pairs {
        let dst = resources.join(sub);
        // Clear-then-copy: these subtrees are wholly day-owned, so removed sources never
        // linger in the bundle across incremental builds.
        let _ = std::fs::remove_dir_all(&dst);
        if !src.is_dir() {
            continue;
        }
        if let Err(e) = copy_tree_flat(&src, &dst) {
            eprintln!("day xcode-backend: stage {sub}: {e}");
            return 4;
        }
    }
    eprintln!(
        "day xcode-backend: staged resources → {}",
        resources.display()
    );
    0
}

/// `day xcode-backend stage-strings` — the scaffold's `Stage Day Strings` script phase:
/// per-locale `InfoPlist.strings` for the `[[shortcuts]]` titles, written into the built
/// bundle before code signing seals it (docs/deep-links.md).
pub fn xcode_backend_stage_strings() -> i32 {
    let get = |k: &str| std::env::var(k).ok();
    let (Some(tbd), Some(res)) = (
        get("TARGET_BUILD_DIR"),
        get("UNLOCALIZED_RESOURCES_FOLDER_PATH"),
    ) else {
        eprintln!("day xcode-backend: must run inside an Xcode build (TARGET_BUILD_DIR unset)");
        return 2;
    };
    let project_dir = get("PROJECT_DIR").map(PathBuf::from).unwrap_or_default();
    // platform/ios/ → project root two levels up.
    let project = match find_project(Some(&project_dir.join("../.."))) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("day xcode-backend: {e}");
            return 2;
        }
    };
    let bundle = PathBuf::from(tbd).join(res);
    if let Err(e) = crate::shortcuts::stage_ios_strings(&project, &bundle) {
        eprintln!("day xcode-backend: stage-strings: {e}");
        return 4;
    }
    0
}

/// Recursive copy (dirs created as needed) — the resource trees are small and flat-ish.
fn copy_tree_flat(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("mkdir {}: {e}", dst.display()))?;
    let rd = std::fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))?;
    for entry in rd.flatten() {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_tree_flat(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// macos-appkit via the Xcode host project (platform/macos/, §17.4)
// ---------------------------------------------------------------------------

/// Whether `day build`/`day launch` should drive macos-appkit through the Xcode host
/// project: the scaffold exists and `DAY_MACOS_XCODE=0` hasn't opted out (the escape hatch
/// CI capture loops use to stay on the faster bare-cargo path).
pub fn macos_xcode_enabled(project: &Project) -> bool {
    if std::env::var("DAY_MACOS_XCODE").is_ok_and(|v| v == "0") {
        return false;
    }
    project
        .root
        .join("platform/macos/DayApp.xcodeproj")
        .is_dir()
}

/// The `OTHER_LDFLAGS` override that keeps a linked Mach-O reproducible across build directories
/// (DESIGN.md §20.3), the macOS counterpart of the `/Brepro` link argument the xaml build passes.
///
/// ld records an absolute path to every object file it consumed in the debug map — one `N_OSO`
/// stabs entry per `.o` and per archive member, pointing into SYMROOT and into cargo's output.
/// Those strings are the ONLY thing that differs when the same commit is linked from two
/// directories, which is exactly what `day rebuild` compares: `build/.../Runner.build/.../main.o`
/// under one root versus another. `-oso_prefix` strips the leading root, leaving project-relative
/// paths that compare equal from anywhere. Stripping the binary would also remove them, but it
/// would take the symbols crash reports symbolicate with (§13), so the debug map stays — just
/// without the machine-specific prefix.
///
/// The prefix is canonicalized because ld writes the resolved path: on macOS `/tmp/...` reaches
/// the linker as `/private/tmp/...`, and a prefix that doesn't match byte-for-byte is silently
/// ignored. `$(inherited)` keeps whatever the pbxproj already sets — a command-line build setting
/// otherwise replaces it for every target in the project.
///
/// This covers every object the FINAL link consumes, which is 12 of the 13 entries. The one it
/// cannot reach is the SwiftPM package target: Xcode merges DayPieces' objects with `ld -r` into
/// a relocatable `Release/DayPieces.o`, and THAT partial link writes the debug map naming
/// `_DayPieces.o`. The final link copies it through verbatim, so a flag given to the final link
/// arrives too late. Command-line build settings do not reach that step either — `PRELINK_FLAGS`
/// was measured and never appears on its command line — because a package target takes its link
/// settings from the generated Package.swift. Closing the last entry means putting the flag there
/// (day writes that manifest, so it can), which is tracked separately.
fn oso_prefix_setting(project_root: &Path) -> String {
    let root = std::fs::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf());
    format!(
        "OTHER_LDFLAGS=$(inherited) -Wl,-oso_prefix,{}/",
        root.display()
    )
}

/// Build macos-appkit through the Xcode host project. Mirrors [`build_ios_for`]: stage the
/// DayPieces package the pbxproj references (empty is fine — the reference must resolve),
/// run xcodebuild with an absolute SYMROOT, and hand back the built `.app` bundle as the
/// artifact (launch execs its inner binary; the bundle itself carries identity, icon, and
/// resources, so none of the bare-binary launch tricks apply).
pub fn build_macos_xcode(
    project: &Project,
    target: &'static Target,
    profile: &str,
    start: std::time::Instant,
) -> Result<BuildOutcome, String> {
    let configuration = if profile == "release" {
        "Release"
    } else {
        "Debug"
    };
    // Absolute for the same reason as iOS (see build_ios_for): SwiftPM package products
    // must land in the same tree as the app target's.
    let symroot = absolute(&project.root.join("build/day/macos-appkit"))?;
    let day_bin = std::env::current_exe().map_err(|e| e.to_string())?;
    crate::pieces::write_macos_pieces(project, true)?;
    status(
        "Building",
        &format!("{} (xcodebuild {configuration}, macosx)", target.name),
    );
    let mut cmd = Command::new("xcodebuild");
    crate::ops::apply_determinism(&mut cmd);
    cmd.current_dir(project.root.join("platform/macos"))
        .args(["-project", "DayApp.xcodeproj", "-target", "Runner"])
        .args(["-configuration", configuration, "-sdk", "macosx"]);
    if std::env::var("DAY_MACOS_UNIVERSAL").is_ok_and(|v| v == "1") {
        // Universal (arm64 + x86_64): opt-in, because the cargo half needs BOTH Rust
        // stdlibs installed (`rustup target add x86_64-apple-darwin` on Apple silicon) —
        // a requirement most dev machines and single-target CI legs don't meet.
    } else {
        // Legacy `-target` builds have no run destination, so ONLY_ACTIVE_ARCH cannot
        // resolve an active arch and Xcode builds UNIVERSAL — twice the disk and time, and
        // a missing cross stdlib fails the build outright (rustc E0463). Pin the arch the
        // running day binary was built for: it always has a matching stdlib installed.
        let arch = match std::env::consts::ARCH {
            "aarch64" => "arm64",
            other => other,
        };
        cmd.args(["-arch", arch]);
    }
    cmd.arg(format!("SYMROOT={}", symroot.display()))
        .arg(format!("DAY_BIN={}", day_bin.display()))
        .arg(oso_prefix_setting(&project.root))
        .arg("build");
    let out = crate::ops::run_capture(&mut cmd, "xcodebuild")?;
    if !out.status.success() {
        return Err(format!("xcodebuild failed:\n{}", diagnose_xcodebuild(&out)));
    }
    // macosx products land under `<configuration>/` (no SDK suffix, unlike iOS).
    let products = symroot.join(configuration);
    let app = std::fs::read_dir(&products)
        .map_err(|e| format!("reading {}: {e}", products.display()))?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|x| x.to_str()) == Some("app"))
        .ok_or_else(|| format!("no .app under {}", products.display()))?;
    Ok(BuildOutcome {
        target: target.name,
        artifact: app,
        seconds: start.elapsed().as_secs_f64(),
    })
}

// ---------------------------------------------------------------------------
// ios-uikit build + launch (porcelain side)
// ---------------------------------------------------------------------------

/// Keep the app Info.plist's `UIAppFonts` array in sync with the project's `fonts/` directory
/// (§18.4). iOS resolves the listed paths relative to the main bundle; the files themselves ride
/// the DayPieces resource bundle (`DayPieces_DayPieces.bundle/fonts/…`, staged by
/// `write_ios_pieces`), and day-uikit ALSO registers them with CoreText at launch, so a plist
/// that iOS declines to honor still resolves. The managed key is rewritten (or removed) on every
/// build — idempotent, so a committed plist only changes when `fonts/` changes.
/// The committed iOS Info.plist — the scaffold's app target is Runner/ (older scaffolds used
/// DayApp/). `None` when the app ships no iOS platform dir.
pub(crate) fn ios_info_plist(project: &Project) -> Option<PathBuf> {
    [
        "platform/ios/Runner/Info.plist",
        "platform/ios/DayApp/Info.plist",
    ]
    .iter()
    .map(|rel| project.root.join(rel))
    .find(|p| p.exists())
}

pub(crate) fn sync_uiappfonts(project: &Project) -> Result<(), String> {
    let Some(plist) = ios_info_plist(project) else {
        return Ok(());
    };
    let fonts = crate::resources::scan_fonts(project)?;
    let paths: Vec<String> = fonts
        .iter()
        .filter_map(|f| f.path.file_name().and_then(|n| n.to_str()))
        .map(|n| format!("DayPieces_DayPieces.bundle/fonts/{n}"))
        .collect();
    // Written through the same editor as the permission keys, NOT `plutil -replace`. plutil
    // reserializes the document and moves the key it rewrites to the end, so while these were two
    // different writers they swapped each other's entries around on every build and the checked-in
    // plist never stopped churning.
    let before =
        std::fs::read_to_string(&plist).map_err(|e| format!("{}: {e}", plist.display()))?;
    let values = if paths.is_empty() {
        None
    } else {
        Some(paths.as_slice())
    };
    let after = crate::plist::apply_array_key(&before, "UIAppFonts", values)
        .map_err(|e| format!("{}: {e}", plist.display()))?;
    if after != before {
        std::fs::write(&plist, after).map_err(|e| format!("{}: {e}", plist.display()))?;
    }
    Ok(())
}

/// The app Info.plist of the scaffold's app target (older scaffolds used `DayApp/`).
pub(crate) fn app_info_plist(project: &Project) -> Option<std::path::PathBuf> {
    [
        "platform/ios/Runner/Info.plist",
        "platform/ios/DayApp/Info.plist",
    ]
    .iter()
    .map(|rel| project.root.join(rel))
    .find(|p| p.exists())
}

/// Write the `NS…UsageDescription` keys for the app's declared permissions (docs/permissions.md).
///
/// iOS reads these at prompt time, and an app that touches a gated API without the matching key is
/// TERMINATED by TCC — so this is what stands between `[permissions]` in Day.toml and a crash on a
/// device.
///
/// The managed set is DERIVED from the declaration table plus the app's `[permissions.raw].ios`
/// keys, never from a state file: on a fresh clone the table alone still knows which keys are Day's
/// to write and to remove. A key outside that set — one a developer added by hand — is never
/// touched, which is the escape hatch for anything Day doesn't model yet.
pub(crate) fn sync_usage_descriptions(project: &Project, macos: bool) -> Result<(), String> {
    let Some(plist) = app_info_plist(project) else {
        return Ok(());
    };
    let platform = if macos { "macos" } else { "ios" };
    let contributed = crate::pieces::contributed_permissions(project, &["uikit"]);
    let plan = crate::permissions::resolve(&project.manifest, platform, &contributed)
        .map_err(|e| format!("Day.toml: {e}"))?;

    let want = crate::permissions::apple_keys(&plan, macos);
    let mut managed = crate::permissions::apple_managed_keys(macos);
    managed.extend(plan.raw_apple.keys().cloned());
    let remove: std::collections::BTreeSet<String> = managed
        .difference(&want.keys().cloned().collect())
        .cloned()
        .collect();

    let before =
        std::fs::read_to_string(&plist).map_err(|e| format!("{}: {e}", plist.display()))?;
    let after = crate::plist::apply_string_keys(&before, &want, &remove)
        .map_err(|e| format!("{}: {e}", plist.display()))?;
    if after == before {
        return Ok(()); // touch only when changed — keeps Xcode's incremental build warm
    }
    std::fs::write(&plist, &after).map_err(|e| format!("{}: {e}", plist.display()))?;

    // Apple's own parser gets the last word. macOS-only, so elsewhere this costs checking, not
    // correctness — and on failure the original file is restored rather than left corrupt.
    if cfg!(target_os = "macos")
        && let Ok(out) = Command::new("plutil").arg("-lint").arg(&plist).output()
        && !out.status.success()
    {
        let _ = std::fs::write(&plist, &before);
        return Err(format!(
            "generated Info.plist failed `plutil -lint` and was restored: {}",
            String::from_utf8_lossy(&out.stdout).trim()
        ));
    }
    Ok(())
}

/// An installed provisioning profile that covers a given app id.
pub(crate) struct InstalledProfile {
    pub name: String,
    pub path: PathBuf,
}

/// The installed development profile whose app id matches `app_id`. Profiles are CMS signed, so
/// `security cms -D` does the decoding rather than a plist parse.
pub(crate) fn installed_profile(app_id: &str) -> Option<InstalledProfile> {
    let dir = dirs_home()?.join("Library/MobileDevice/Provisioning Profiles");
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("mobileprovision") {
            continue;
        }
        let Ok(out) = Command::new("security")
            .args(["cms", "-D", "-i"])
            .arg(&path)
            .output()
        else {
            continue;
        };
        if !out.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        // `<key>application-identifier</key><string>TEAMID.app.bundle.id</string>`
        let Some(after) = text.split("application-identifier").nth(1) else {
            continue;
        };
        let Some(value) = after
            .split("<string>")
            .nth(1)
            .and_then(|v| v.split("</string>").next())
        else {
            continue;
        };
        let value = value.trim();
        if let Some((team, id)) = value.split_once('.')
            && id == app_id
        {
            let name = text
                .split("<key>Name</key>")
                .nth(1)
                .and_then(|v| v.split("<string>").nth(1))
                .and_then(|v| v.split("</string>").next())
                .unwrap_or_default()
                .trim()
                .to_string();
            let _ = team;
            return Some(InstalledProfile {
                name,
                path: path.clone(),
            });
        }
    }
    None
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

/// Whether the app asks for push. `notifications = true` in Day.toml is the app saying it wants
/// them; on Apple platforms that requires `aps-environment`, which only a provisioning profile can
/// grant. Used to check the two agree before signing rather than after the app fails to register.
pub(crate) fn ios_wants_push(project: &Project) -> Result<bool, String> {
    let contributed = crate::pieces::contributed_permissions(project, &["uikit"]);
    let plan = crate::permissions::resolve(&project.manifest, "ios", &contributed)
        .map_err(|e| format!("Day.toml: {e}"))?;
    Ok(plan.resolved.iter().any(|r| r.spec.name == "notifications"))
}

/// Everything the iOS build stages before xcodebuild runs.
///
/// One function, three call sites (`build_ios`, and both `pack::ios` paths) — because they had
/// already drifted: the signed-archive path never synced `UIAppFonts`, so a released `.ipa` could
/// ship a stale font list.
/// Returns the `IPHONEOS_DEPLOYMENT_TARGET` override (docs/swiftui.md): `Some(floor)` when a
/// piece's `platform` metadata exceeds the scaffold pbxproj's checked-in value. Every xcodebuild
/// invocation downstream must pass it — a command-line setting reaches the app AND the SwiftPM
/// package targets, which is the only way to raise both without editing the scaffold.
pub(crate) fn prepare_ios(project: &Project) -> Result<Option<String>, String> {
    let floor = crate::pieces::write_ios_pieces(project)?;
    sync_uiappfonts(project)?;
    sync_usage_descriptions(project, false)?;
    // Day.toml [[shortcuts]] → UIApplicationShortcutItems, through the same plist editor as
    // the keys above; localized titles are staged into the bundle by the `stage-strings`
    // script phase, which older scaffolds get injected here.
    if let Some(plist) = ios_info_plist(project) {
        crate::shortcuts::sync_ios(project, &plist)?;
    }
    crate::shortcuts::ensure_ios_strings_phase(project)?;
    if let Some(f) = &floor {
        status(
            "Raising",
            &format!(
                "iOS deployment target to {f} (a piece requires it; raise it in \
                 platform/ios/DayApp.xcodeproj for Xcode-IDE builds)"
            ),
        );
    }
    Ok(floor)
}

pub fn build_ios(
    project: &Project,
    target: &'static Target,
    profile: &str,
    start: std::time::Instant,
) -> Result<BuildOutcome, String> {
    build_ios_for(project, target, profile, start, false)
}

/// `physical` swaps the simulator SDK for the device one and turns signing on. A simulator build
/// is unsigned by construction; a device refuses anything that is not signed by a certificate it
/// trusts, listed in a profile that names the device. Everything below the SDK switch is that.
pub fn build_ios_for(
    project: &Project,
    target: &'static Target,
    profile: &str,
    start: std::time::Instant,
    physical: bool,
) -> Result<BuildOutcome, String> {
    let configuration = if profile == "release" {
        "Release"
    } else {
        "Debug"
    };
    // SYMROOT MUST be absolute: xcodebuild resolves a relative build path against each target's own
    // working directory, so the Runner app target and its SwiftPM package dependencies (e.g. Lottie,
    // whose resource bundle the app copies) would land their products in different trees and the copy
    // would fail with "no such file … .bundle". `project.root` is absolute (see meta::find_project),
    // but absolutize here too so this invariant is enforced at the one place that actually matters.
    let symroot = absolute(&project.root.join("build/day/ios-uikit"))?;
    let sdk = if physical {
        "iphoneos"
    } else {
        "iphonesimulator"
    };
    let day_bin = std::env::current_exe().map_err(|e| e.to_string())?;
    // Stage everything xcodebuild needs: the local DayPieces SwiftPM package the .xcodeproj links,
    // the UIAppFonts array, and the permission usage descriptions (docs/permissions.md).
    let floor = prepare_ios(project)?;
    let prov = if physical {
        installed_profile(&project.manifest.app.id)
    } else {
        None
    };
    status(
        "Building",
        &format!("{} (xcodebuild {configuration}, {sdk})", target.name),
    );
    let xcodebuild = || {
        let mut cmd = Command::new("xcodebuild");
        crate::ops::apply_determinism(&mut cmd);
        cmd.current_dir(project.root.join("platform/ios"))
            .args(["-project", "DayApp.xcodeproj", "-target", "Runner"])
            .args([
                "-configuration",
                configuration,
                "-sdk",
                sdk,
                "-arch",
                "arm64",
            ])
            .arg(format!("SYMROOT={}", symroot.display()))
            .arg(format!("DAY_BIN={}", day_bin.display()))
            .arg(oso_prefix_setting(&project.root));
        if let Some(f) = &floor {
            cmd.arg(format!("IPHONEOS_DEPLOYMENT_TARGET={f}"));
        }
        if physical {
            // Build UNSIGNED and sign the bundle ourselves below. Letting xcodebuild sign means
            // choosing between two failures: `Automatic` mints its own "iOS Team Provisioning
            // Profile: *" wildcard, which carries neither this app's certificate nor its push
            // capability; `Manual` names our profile, but command-line settings reach EVERY
            // target, and the SwiftPM package targets (Lottie, DayPieces) refuse a profile at all
            // — "does not support provisioning profiles". Signing afterwards sidesteps both, and
            // takes the identity and entitlements from the profile itself, so the three can't
            // disagree.
            cmd.arg("CODE_SIGNING_ALLOWED=NO")
                .arg("CODE_SIGNING_REQUIRED=NO");
        }
        cmd.arg("build");
        // Capture for the stale-bundle retry + failure distillation below; `run_capture` also
        // forwards the raw build log live under `--verbose`.
        crate::ops::run_capture(&mut cmd, "xcodebuild")
    };
    let mut out = xcodebuild()?;
    if !out.status.success() && is_stale_bundle_failure(&out) {
        // A SwiftPM package resource bundle landed in the wrong tree (stale/split build products).
        // Clear this target's build tree and retry once from clean — self-heals the common case.
        status("Rebuilding", "ios-uikit (clearing stale build tree)");
        let _ = std::fs::remove_dir_all(&symroot);
        out = xcodebuild()?;
    }
    if !out.status.success() {
        // A device build that fails IN the signing phase still leaves the assembled (unsigned)
        // bundle behind, and xcodebuild treats it as up to date next time — so the retry that
        // would have worked silently produces an unsigned app instead. Drop the product.
        if physical {
            let _ = std::fs::remove_dir_all(symroot.join(format!("{configuration}-{sdk}")));
        }
        return Err(format!("xcodebuild failed:\n{}", diagnose_xcodebuild(&out)));
    }
    // The Runner target's product bundle is named after the app's PRODUCT_NAME (per app), so locate
    // the single `.app` in the products dir rather than assuming a fixed name.
    let products = symroot.join(format!("{configuration}-{sdk}"));
    let app = std::fs::read_dir(&products)
        .map_err(|e| format!("reading {}: {e}", products.display()))?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|x| x.to_str()) == Some("app"))
        .ok_or_else(|| format!("no .app bundle in {}", products.display()))?;
    if physical {
        let p = prov.ok_or_else(|| {
            format!(
                "no installed provisioning profile covers {}. Create a development profile for \
                 that app id and install it (double-click the .mobileprovision), then retry.",
                project.manifest.app.id
            )
        })?;
        sign_ios_bundle(project, &app, &p)?;
    }
    Ok(BuildOutcome {
        target: target.name,
        artifact: app,
        seconds: start.elapsed().as_secs_f64(),
    })
}

/// Sign a device bundle against the profile that provisions it.
///
/// Both inputs come from the profile rather than from configuration: the signing identity is the
/// certificate the profile lists (matched by SHA-1, so a machine holding several development
/// certificates picks the right one), and the entitlements are the profile's own. A signature can
/// only claim entitlements its profile grants, so taking them from there makes that true by
/// construction instead of by a file someone has to keep in step.
fn sign_ios_bundle(project: &Project, app: &Path, prof: &InstalledProfile) -> Result<(), String> {
    let tmp = std::env::temp_dir().join("day-ios-sign");
    let _ = std::fs::create_dir_all(&tmp);
    let plist = tmp.join("profile.plist");
    let out = Command::new("security")
        .args(["cms", "-D", "-i"])
        .arg(&prof.path)
        .arg("-o")
        .arg(&plist)
        .output()
        .map_err(|e| format!("security cms: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "could not decode {}: {}",
            prof.path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    // The entitlements the signature will claim.
    let ents = tmp.join("signing.entitlements");
    run_logged(
        Command::new("plutil")
            .args(["-extract", "Entitlements", "xml1", "-o"])
            .arg(&ents)
            .arg(&plist),
        "plutil -extract Entitlements",
    )?;

    // What the app declares and what the profile grants have to agree. Catching it here beats
    // shipping an app to the device that silently cannot register for push.
    if ios_wants_push(project)? {
        let text = std::fs::read_to_string(&ents).unwrap_or_default();
        if !text.contains("aps-environment") {
            return Err(format!(
                "Day.toml declares `notifications`, but the profile {:?} does not grant \
                 aps-environment. Enable Push Notifications on the App ID for {} and regenerate \
                 the profile.",
                prof.name, project.manifest.app.id
            ));
        }
    }

    // The certificate the profile lists, by fingerprint.
    let der = tmp.join("signer.der");
    run_logged(
        Command::new("plutil")
            .args(["-extract", "DeveloperCertificates.0", "raw", "-o"])
            .arg(tmp.join("signer.b64"))
            .arg(&plist),
        "plutil -extract DeveloperCertificates",
    )?;
    let b64 = std::fs::read_to_string(tmp.join("signer.b64")).map_err(|e| e.to_string())?;
    let decoded = Command::new("base64")
        .args(["-d"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            use std::io::Write;
            c.stdin.take().unwrap().write_all(b64.as_bytes())?;
            c.wait_with_output()
        })
        .map_err(|e| format!("base64: {e}"))?;
    std::fs::write(&der, &decoded.stdout).map_err(|e| e.to_string())?;
    let fp = Command::new("openssl")
        .args(["x509", "-inform", "DER", "-in"])
        .arg(&der)
        .args(["-noout", "-fingerprint", "-sha1"])
        .output()
        .map_err(|e| format!("openssl: {e}"))?;
    let sha1 = String::from_utf8_lossy(&fp.stdout)
        .split('=')
        .nth(1)
        .map(|v| v.trim().replace(':', ""))
        .ok_or("could not read the signing certificate's fingerprint")?;

    std::fs::copy(&prof.path, app.join("embedded.mobileprovision"))
        .map_err(|e| format!("embedding the profile: {e}"))?;

    // Inside-out: nested code must be signed before the bundle that contains it (§16.5).
    let mut nested: Vec<PathBuf> = Vec::new();
    for sub in ["Frameworks", "PlugIns"] {
        if let Ok(rd) = std::fs::read_dir(app.join(sub)) {
            nested.extend(rd.flatten().map(|e| e.path()));
        }
    }
    if let Ok(rd) = std::fs::read_dir(app) {
        nested.extend(
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("bundle")),
        );
    }
    nested.sort();
    for item in &nested {
        run_logged(
            Command::new("codesign")
                .args(["--force", "--timestamp=none", "--sign", &sha1])
                .arg(item),
            &format!(
                "codesign {}",
                item.file_name().unwrap_or_default().to_string_lossy()
            ),
        )?;
    }
    status(
        "Signing",
        &format!(
            "{} ({})",
            app.file_name().unwrap_or_default().to_string_lossy(),
            prof.name
        ),
    );
    run_logged(
        Command::new("codesign")
            .args([
                "--force",
                "--timestamp=none",
                "--sign",
                &sha1,
                "--entitlements",
            ])
            .arg(&ents)
            .arg(app),
        "codesign (app)",
    )?;
    Ok(())
}

/// UDIDs of every currently-booted iOS simulator (`simctl list devices booted`). All simulators on
/// a given host share the host arch, so the one `aarch64-apple-ios-sim` build runs on each.
pub(crate) fn booted_sims() -> Vec<String> {
    let out = match Command::new("xcrun")
        .args(["simctl", "list", "devices", "booted"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains("(Booted)"))
        .filter_map(|l| {
            // The UDID is the parenthesized 36-char group before "(Booted)".
            l.split(['(', ')'])
                .map(str::trim)
                .find(|t| t.len() == 36 && t.split('-').count() == 5)
                .map(str::to_string)
        })
        .collect()
}

/// Resolve `--ios-simulator` (a UDID or a device name) against the booted simulators.
///
/// Matching is deliberately restricted to BOOTED devices: a name that exists but is shut down is a
/// clearer error than silently booting something the caller did not ask for, and booting is the
/// caller's decision (it takes tens of seconds and changes the state of their machine).
fn select_sim(booted: &[String], want: &str) -> Result<Vec<String>, String> {
    if booted.iter().any(|u| u.eq_ignore_ascii_case(want)) {
        return Ok(vec![want.to_string()]);
    }
    // Not a booted UDID — try it as a device name, which is what a human passes.
    let listing = Command::new("xcrun")
        .args(["simctl", "list", "devices", "booted"])
        .output()
        .map_err(|e| format!("simctl list: {e}"))?;
    let named: Vec<String> = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .filter(|l| l.contains("(Booted)"))
        .filter(|l| {
            l.split_once('(')
                .map(|(name, _)| name.trim().eq_ignore_ascii_case(want))
                .unwrap_or(false)
        })
        .filter_map(|l| {
            l.split(['(', ')'])
                .map(str::trim)
                .find(|t| t.len() == 36 && t.split('-').count() == 5)
                .map(str::to_string)
        })
        .collect();
    if named.is_empty() {
        return Err(format!(
            "--ios-simulator {want:?} is not a booted iOS simulator (booted: {}). Boot it first: \
             `xcrun simctl boot {want:?}`",
            if booted.is_empty() {
                "none".to_string()
            } else {
                booted.join(", ")
            }
        ));
    }
    Ok(named)
}

/// Physical iOS devices, from `devicectl`. A real device's UDID is 25 characters; simulators
/// report a 36-character GUID through the same list, which is the trap this filter exists for.
pub(crate) fn physical_ios_devices() -> Vec<(String, String)> {
    let tmp = std::env::temp_dir().join("day-devicectl-devices.json");
    let ok = Command::new("xcrun")
        .args(["devicectl", "list", "devices", "--json-output"])
        .arg(&tmp)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        return Vec::new();
    }
    let Ok(text) = std::fs::read_to_string(&tmp) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for d in json["result"]["devices"].as_array().into_iter().flatten() {
        let udid = d["hardwareProperties"]["udid"].as_str().unwrap_or_default();
        let platform = d["hardwareProperties"]["platform"]
            .as_str()
            .unwrap_or_default();
        let name = d["deviceProperties"]["name"].as_str().unwrap_or_default();
        if platform == "iOS" && udid.len() == 25 {
            out.push((udid.to_string(), name.to_string()));
        }
    }
    out
}

/// Install and run on a physical device via `devicectl`. Logs are the device's, so unlike the
/// simulator path there is no stdout to pipe: the app is launched with its console attached.
fn launch_ios_device(
    project: &Project,
    outcome: &BuildOutcome,
    spec: &LaunchSpec,
) -> Result<std::thread::JoinHandle<i32>, String> {
    let bundle_id = project.manifest.app.id.clone();
    let devices = physical_ios_devices();
    if devices.is_empty() {
        return Err(
            "no physical iOS device is paired and reachable. Connect it (or bring it onto \
                    the same network for a wireless pair) and check `xcrun devicectl list devices`."
                .to_string(),
        );
    }
    // `--ios-device` may name either the UDID or the device name shown in Xcode.
    let (udid, name) = match spec.ios_device.as_deref() {
        None if devices.len() == 1 => devices[0].clone(),
        None => {
            return Err(format!(
                "several iOS devices are available — name one with --ios-device: {}",
                devices
                    .iter()
                    .map(|(_, n)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Some(want) => devices
            .iter()
            .find(|(u, n)| u.eq_ignore_ascii_case(want) || n.eq_ignore_ascii_case(want))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "--ios-device {want:?} is not a paired iOS device (available: {})",
                    devices
                        .iter()
                        .map(|(_, n)| n.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?,
    };

    status("Installing", &format!("{} on {name}", outcome.target));
    // Captured, not streamed: devicectl narrates the install in a bullet list (bundleID,
    // installationURL, databaseUUID …) that says nothing a reader needs and looks nothing like
    // any other target's output. The status lines above and below say the same thing in Day's
    // voice; the detail is kept only to be shown if it fails.
    run_quiet(
        Command::new("xcrun")
            .args(["devicectl", "device", "install", "app", "--device", &udid])
            .arg(&outcome.artifact),
        &format!("devicectl install ({name})"),
    )?;

    status(
        "Launching",
        &format!("{} ({bundle_id}) on device {name}", outcome.target),
    );
    let mut launch = Command::new("xcrun");
    launch
        .args([
            "devicectl",
            "device",
            "process",
            "launch",
            "--device",
            &udid,
        ])
        .arg(&bundle_id);
    if spec.attached {
        // Streams the app's own stdout/stderr back, the way `simctl launch --console` does.
        launch.arg("--console");
    }
    for (k, v) in &spec.envs {
        launch.arg("--environment-variables");
        launch.arg(format!("{{\"{k}\":\"{v}\"}}"));
    }
    if let Some(loc) = &spec.locale {
        launch.arg("--environment-variables");
        launch.arg(format!("{{\"DAY_LOCALE\":\"{loc}\"}}"));
    }
    if !spec.attached {
        run_logged(&mut launch, &format!("devicectl launch ({name})"))?;
        return Ok(std::thread::spawn(|| 0));
    }

    launch
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = launch
        .spawn()
        .map_err(|e| format!("devicectl launch: {e}"))?;
    crate::signals::register_child(child.id());
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let label = outcome.target.to_string();
    Ok(std::thread::spawn(move || {
        let l2 = label.clone();
        let t1 = stdout.map(|s| stream_devicectl(label, LogStream::Out, s));
        let t2 = stderr.map(|s| stream_devicectl(l2, LogStream::Err, s));
        let code = child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(1);
        if let Some(t) = t1 {
            let _ = t.join();
        }
        if let Some(t) = t2 {
            let _ = t.join();
        }
        code
    }))
}

/// [`stream_logs_labeled`] with devicectl's own narration filtered out, so what reaches the
/// terminal is the app's output under the same `[target]` prefix every other platform uses.
/// devicectl interleaves its progress on the same stream as the app it launched, and those lines
/// are about devicectl, not about the app.
fn stream_devicectl(
    label: String,
    stream: LogStream,
    src: impl std::io::Read + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut failure: Vec<String> = Vec::new();
        for line in BufReader::new(src).lines().map_while(Result::ok) {
            let t = line.trim().to_string();
            // devicectl reports a failed launch as a nested tree of error domains — a dozen lines
            // whose useful content is one sentence. Buffer from the first ERROR: to the end of the
            // stream (devicectl exits after it) and summarise once, rather than relaying the tree.
            if !failure.is_empty() || t.starts_with("ERROR:") {
                failure.push(t);
                continue;
            }
            let noise = t.is_empty()
                || t.starts_with("Launched application with")
                || t.starts_with("Waiting for the application to terminate")
                || t.starts_with("App installed:")
                || t.starts_with('•')
                || t.starts_with("The app is now running")
                || t.starts_with("Application terminated");
            if !noise {
                emit_log(&label, stream, &line);
            }
        }
        if !failure.is_empty() {
            emit_log(
                &label,
                LogStream::Err,
                &summarize_devicectl_failure(&failure),
            );
        }
    })
}

/// One line for a devicectl failure tree. The locked screen is called out by name because it is
/// the common one and the remedy is not obvious from Apple's wording ("RequestDenied").
fn summarize_devicectl_failure(lines: &[String]) -> String {
    let joined = lines.join(" ");
    if joined.contains("could not be, unlocked")
        || joined.contains("BSErrorCodeDescription = Locked")
    {
        return "the device is locked — unlock it and run again (iOS will not launch an app onto \
                a locked screen)"
            .to_string();
    }
    // Otherwise Apple's own reason, which is the only line in the tree written for a human.
    for l in lines {
        if let Some(reason) = l.strip_prefix("NSLocalizedFailureReason = ") {
            return format!("launch failed: {}", reason.trim());
        }
    }
    lines
        .first()
        .map(|l| l.trim_start_matches("ERROR: ").to_string())
        .unwrap_or_else(|| "launch failed".to_string())
}

pub fn launch_ios(
    project: &Project,
    outcome: &BuildOutcome,
    spec: &LaunchSpec,
) -> Result<std::thread::JoinHandle<i32>, String> {
    if spec.wants_ios_device() {
        return launch_ios_device(project, outcome, spec);
    }
    let bundle_id = project.manifest.app.id.clone();
    let sims = booted_sims();
    if sims.is_empty() {
        return Err(
            "no booted iOS simulator (open Simulator.app or `xcrun simctl boot <device>`); \
                    physical devices need code signing and aren't supported here"
                .into(),
        );
    }
    // `--ios-simulator` narrows to one; without it every booted simulator gets the app.
    let sims = match spec.ios_simulator.as_deref() {
        Some(want) => select_sim(&sims, want)?,
        None => sims,
    };
    let multi = sims.len() > 1;
    let mut log_threads = Vec::new();
    for udid in &sims {
        run_logged(
            Command::new("xcrun")
                .args(["simctl", "install", udid])
                .arg(&outcome.artifact),
            &format!("simctl install ({udid})"),
        )?;
        let _ = Command::new("xcrun")
            .args(["simctl", "terminate", udid, &bundle_id])
            .status();
        let mut cmd = Command::new("xcrun");
        cmd.args(["simctl", "launch"]);
        if spec.attached {
            // `--console` (not `--console-pty`) keeps the app's stdout and stderr on
            // simctl's separate fds, so we can colour them apart.
            cmd.arg("--console");
        }
        cmd.args([udid.as_str(), &bundle_id]);
        for (k, v) in &spec.envs {
            cmd.env(format!("SIMCTL_CHILD_{k}"), v);
        }
        if let Some(locale) = &spec.locale {
            cmd.env("SIMCTL_CHILD_DAY_LOCALE", locale);
        }
        status(
            "Launching",
            &format!("ios-uikit ({bundle_id}) on simulator {udid}"),
        );
        if spec.attached {
            cmd.stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let mut child = cmd.spawn().map_err(|e| format!("simctl launch: {e}"))?;
            crate::signals::register_child(child.id());
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            // Multi-sim runs tag each stream with the UDID so the interleaved logs read apart.
            let (out_label, err_label) = if multi {
                (
                    format!("{}:{}", outcome.target, udid),
                    format!("{}:{}", outcome.target, udid),
                )
            } else {
                (outcome.target.to_string(), outcome.target.to_string())
            };
            log_threads.push(std::thread::spawn(move || {
                let t1 = stdout.map(|s| stream_logs_labeled(out_label, LogStream::Out, s));
                let t2 = stderr.map(|s| stream_logs_labeled(err_label, LogStream::Err, s));
                let code = child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(1);
                if let Some(t) = t1 {
                    let _ = t.join();
                }
                if let Some(t) = t2 {
                    let _ = t.join();
                }
                code
            }));
        } else {
            run_logged(&mut cmd, &format!("simctl launch ({udid})"))?;
        }
    }
    Ok(std::thread::spawn(move || {
        let mut code = 0;
        for t in log_threads {
            if let Ok(c) = t.join()
                && c != 0
                && code == 0
            {
                code = c;
            }
        }
        code
    }))
}

/// Like `ops::stream_logs` but with an owned label (so per-device threads can carry a serial/UDID).
fn stream_logs_labeled(
    label: String,
    stream: LogStream,
    src: impl std::io::Read + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(src).lines().map_while(Result::ok) {
            emit_log(&label, stream, &line);
        }
    })
}

// ---------------------------------------------------------------------------
// android-mdc (gradle + adb) — scaffold lands next; see gradle_backend_build
// ---------------------------------------------------------------------------

pub fn gradle_backend_build() -> i32 {
    // Invoked by the gradle scaffold with DAY_PROJECT_ROOT + DAY_PROFILE + DAY_OUT set.
    let root = match std::env::var("DAY_PROJECT_ROOT") {
        Ok(v) => PathBuf::from(v),
        Err(_) => {
            eprintln!("day gradle-backend: DAY_PROJECT_ROOT unset (run via the gradle scaffold)");
            return 2;
        }
    };
    let profile = std::env::var("DAY_PROFILE").unwrap_or_else(|_| "debug".into());
    let out = std::env::var("DAY_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("build/day/jniLibs"));
    let project = match find_project(Some(&root)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("day gradle-backend: {e}");
            return 2;
        }
    };
    build_android_so(&project, &profile, &out, &android_build_abis())
        .map(|_| 0)
        .unwrap_or(4)
}

/// A connected Android device or emulator, with the ABI it actually runs (queried, not guessed —
/// an emulator matches the host arch, a phone is usually arm64, so we ask each one).
pub(crate) struct AndroidDevice {
    pub serial: String,
    pub abi: String,
}

/// `adb` with an optional device selector (`-s <serial>`). Multi-device installs/launches MUST
/// pin the serial, or adb errors ("more than one device/emulator").
fn adb(serial: Option<&str>) -> Command {
    let mut c = Command::new("adb");
    if let Some(s) = serial {
        c.args(["-s", s]);
    }
    c
}

/// Every device in `adb devices` in the `device` state, paired with its primary ABI
/// (`ro.product.cpu.abi`). `DAY_ANDROID_ABI`, when set, overrides the queried ABI for every device
/// (CI's KVM emulator leg pins `x86_64`); when it holds a LIST, the first entry is the per-device
/// override (a device runs one primary ABI — the full list matters to [`android_build_abis`]).
/// Empty when nothing is connected.
///
/// `--android-device`, else `ANDROID_SERIAL` (adb's own device-selection variable), narrows the
/// list to that one device — so launches, installs, and dayscript sessions target it exclusively
/// when several are attached (the default remains all connected devices).
pub(crate) fn android_devices() -> Vec<AndroidDevice> {
    android_devices_for(None)
}

pub(crate) fn android_devices_for(want: Option<&str>) -> Vec<AndroidDevice> {
    let forced = std::env::var("DAY_ANDROID_ABI")
        .ok()
        .and_then(|v| parse_abi_list(&v).into_iter().next());
    let only = want.map(str::to_string).or_else(|| {
        std::env::var("ANDROID_SERIAL")
            .ok()
            .filter(|s| !s.is_empty())
    });
    let out = match Command::new("adb").arg("devices").output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1) // "List of devices attached"
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let serial = it.next()?;
            if it.next() != Some("device") {
                return None; // skip offline/unauthorized
            }
            if let Some(want) = &only
                && serial != want
            {
                return None;
            }
            let abi = forced.clone().unwrap_or_else(|| {
                Command::new("adb")
                    .args(["-s", serial, "shell", "getprop", "ro.product.cpu.abi"])
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "arm64-v8a".into())
            });
            Some(AndroidDevice {
                serial: serial.to_string(),
                abi,
            })
        })
        .collect()
}

/// The set of ABIs to build for. `DAY_ANDROID_ABI`, when set to a non-empty list, is
/// **authoritative**: exactly those ABIs are built, regardless of any connected device — so a
/// distribution `day pack` carries `lib/<abi>/` for every listed ABI (`arm64-v8a,x86_64`) even
/// while an emulator is attached (each ABI needs its rustup target, e.g.
/// `rustup target add x86_64-linux-android`). Otherwise the ABIs are the distinct ABIs of the
/// connected devices, or — with nothing connected (e.g. `day build` before the emulator boots) —
/// the `arm64-v8a` default, so packaging still succeeds.
pub(crate) fn android_build_abis() -> Vec<String> {
    // An explicit `DAY_ANDROID_ABI` wins over device detection: setting it produces exactly that
    // ABI set (e.g. a dual-ABI pack) even when an emulator/device of a different ABI is connected.
    if let Ok(v) = std::env::var("DAY_ANDROID_ABI") {
        let mut abis = parse_abi_list(&v);
        abis.sort();
        abis.dedup();
        if !abis.is_empty() {
            return abis;
        }
    }
    let mut abis: Vec<String> = android_devices().into_iter().map(|d| d.abi).collect();
    abis.sort();
    abis.dedup();
    if abis.is_empty() {
        abis.push("arm64-v8a".into());
    }
    abis
}

/// Split a `DAY_ANDROID_ABI` value into ABIs: comma- and/or whitespace-separated, empties dropped
/// (`"arm64-v8a,x86_64"` and `"arm64-v8a x86_64"` both parse to two).
fn parse_abi_list(v: &str) -> Vec<String> {
    v.split([',', ' ', '\t'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Cross-compile the app cdylib for every ABI in `abis` into `out/<abi>/lib<name>.so` (one
/// `cargo ndk -t <abi> …` invocation covering them all).
fn build_android_so(
    project: &Project,
    profile: &str,
    out: &Path,
    abis: &[String],
) -> Result<(), String> {
    let (cargo, bin) = rustup_cargo()?;
    let name = project.manifest.app.name.clone();
    let ndk_home = find_ndk()?;
    let target_dir = project
        .root
        .join("build/day/cargo/android-mdc")
        .join(profile);
    let mut cmd = Command::new(&cargo);
    // Thinned ICU locale data for the declared locale set (crates/day-cli/src/intl.rs).
    crate::intl::apply(&mut cmd, project);
    cmd.current_dir(&project.root)
        .env(
            "PATH",
            format!(
                "{}:{}/.cargo/bin:{}",
                bin.display(),
                std::env::var("HOME").unwrap_or_default(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("CARGO_TARGET_DIR", &target_dir)
        .env("ANDROID_NDK_HOME", &ndk_home);
    crate::ops::apply_app_identity(&mut cmd, project);
    cmd.arg("ndk");
    for abi in abis {
        cmd.args(["-t", abi]);
    }
    cmd.arg("-o")
        .arg(out)
        // `rustc --crate-type cdylib` so the app lib's manifest can stay rlib-only (see the
        // `[lib]` note in the app Cargo.toml); produces the same `lib<name>.so` this expects.
        // `--features` = `mdc` + every standalone piece's `<pkg>/mdc` renderer feature (Tier
        // A.2), so the app needn't re-list per-piece features in its own Cargo.toml.
        .arg("rustc")
        .args([
            "-p",
            &name,
            "--lib",
            "--crate-type",
            "cdylib",
            "--no-default-features",
            "--features",
            &crate::ops::feature_selection(project, "mdc"),
        ]);
    if profile == "release" {
        cmd.arg("--release");
    }
    run_logged(&mut cmd, "cargo ndk")?;
    Ok(())
}

/// The Android SDK root: `ANDROID_HOME`, else `ANDROID_SDK_ROOT`, else the macOS default location.
/// Shared with `day doctor` so its diagnosis matches what the build actually probes.
pub(crate) fn android_sdk_dir() -> PathBuf {
    // Shared lookup: ANDROID_HOME / ANDROID_SDK_ROOT, then the per-OS default install location
    // (docs/environment.md).
    day_toolchain::android_sdk_dir()
}

pub(crate) fn find_ndk() -> Result<PathBuf, String> {
    if let Ok(v) = std::env::var("ANDROID_NDK_HOME") {
        return Ok(PathBuf::from(v));
    }
    let sdk = android_sdk_dir();
    let ndk_dir = sdk.join("ndk");
    let mut versions: Vec<_> = std::fs::read_dir(&ndk_dir)
        .map_err(|_| "no Android NDK found (set ANDROID_NDK_HOME)")?
        .flatten()
        .map(|e| e.path())
        .collect();
    versions.sort();
    versions.pop().ok_or_else(|| "empty ndk dir".into())
}

pub fn build_android(
    project: &Project,
    target: &'static Target,
    profile: &str,
    start: std::time::Instant,
) -> Result<BuildOutcome, String> {
    // 1) Rust .so, one per connected device's ABI (so an app built with an arm64 phone AND an
    //    x86_64 emulator attached carries both). Also invoked by gradle's callback; building here
    //    keeps `day build` primary.
    let jni_out = project.root.join("build/day/jniLibs");
    let abis = android_build_abis();
    status(
        "Building",
        &format!("{} (cargo-ndk {})", target.name, abis.join(" ")),
    );
    build_android_so(project, profile, &jni_out, &abis)?;

    // Convey Day.toml identity/version to the Gradle scaffold (§17.5) on every build, so
    // applicationId/versionCode/versionName never go stale in the checked-in scaffold.
    crate::pack::android::write_app_properties(project)?;

    // 2) Discover standalone-piece Android contributions (own Java / Gradle deps) and stage them
    //    for the Gradle build to pick up — a piece ships its backend without editing Day.
    crate::pieces::write_android_manifest(project)?;

    // 3) Gradle assemble.
    let task = if profile == "release" {
        "assembleRelease"
    } else {
        "assembleDebug"
    };
    status("Building", &format!("{} (gradle {task})", target.name));
    let day_bin = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new("gradle");
    cmd.current_dir(project.root.join("platform/android"))
        .env("DAY_BIN", &day_bin)
        .env("DAY_PROJECT_ROOT", &project.root)
        .env("DAY_PROFILE", profile)
        .args([task, "--console=plain"]);
    // Day narrates the phase and surfaces gradle's tail on failure, so gradle runs quiet by default.
    // `--verbose` drops `-q` so it emits its full build log, forwarded live by `run_capture`.
    if !crate::ops::verbose() {
        cmd.arg("-q");
    }
    // AGP 9's minimum is JDK 17, and Gradle 9.6 runs on 17…26 (the scaffold builds on all of them).
    // Respect the caller's JAVA_HOME (CI pins one via setup-java); default to a discovered 17+ JDK
    // when unset.
    if std::env::var_os("JAVA_HOME").is_none()
        && let Some(jdk) = day_toolchain::jdk_home()
    {
        cmd.env("JAVA_HOME", jdk);
    }
    let out = crate::ops::run_capture(&mut cmd, "gradle")?;
    if !out.status.success() {
        if crate::ops::verbose() {
            // Full log already streamed live.
            return Err("gradle failed".into());
        }
        let text = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = text.lines().rev().take(30).collect();
        return Err(format!(
            "gradle failed:\n{}",
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        ));
    }
    let apk_name = if profile == "release" {
        "app-release.apk"
    } else {
        "app-debug.apk"
    };
    let apk_dir = project
        .root
        .join("platform/android/app/build/outputs/apk")
        .join(profile);
    // An unsigned release build is emitted as `app-release-unsigned.apk` — fall back to whatever
    // single .apk the build produced rather than assuming the signed name.
    let conventional = apk_dir.join(apk_name);
    let apk = if conventional.exists() {
        conventional
    } else {
        std::fs::read_dir(&apk_dir)
            .ok()
            .and_then(|entries| {
                entries
                    .flatten()
                    .map(|e| e.path())
                    .find(|p| p.extension().and_then(|x| x.to_str()) == Some("apk"))
            })
            .unwrap_or(conventional)
    };
    Ok(BuildOutcome {
        target: target.name,
        artifact: apk,
        seconds: start.elapsed().as_secs_f64(),
    })
}

pub fn launch_android(
    project: &Project,
    outcome: &BuildOutcome,
    spec: &LaunchSpec,
) -> Result<std::thread::JoinHandle<i32>, String> {
    let app_id = project.manifest.app.id.clone();
    let devices = android_devices_for(spec.android_device.as_deref());
    if devices.is_empty() {
        return Err(match spec.android_device.as_deref() {
            Some(serial) => {
                format!("--android-device {serial:?} is not connected (check `adb devices`)")
            }
            None => "no Android device/emulator connected (check `adb devices`)".into(),
        });
    }
    // Install + launch on EVERY connected device; the one APK already carries each device's ABI.
    let mut log_threads = Vec::new();
    for dev in &devices {
        status(
            "Installing",
            &format!("{} on {}", outcome.target, dev.serial),
        );
        run_quiet(
            adb(Some(&dev.serial))
                .args(["install", "-r"])
                .arg(&outcome.artifact),
            &format!("adb install ({})", dev.serial),
        )?;
        // A still-running instance would just be foregrounded by `am start` — keeping the old
        // run's engine port, theme, and locale (its views were created under the previous
        // configuration). Force-stop first so every launch is a fresh process reading THIS run's
        // extras, mirroring the OHOS launcher.
        run_quiet(
            adb(Some(&dev.serial)).args(["shell", "am", "force-stop", &app_id]),
            &format!("am force-stop ({})", dev.serial),
        )?;
        // EMULATORS ONLY: suppress the system ANR/crash dialogs (the standard test-device
        // setting). A loaded host makes an emulated main thread miss Android's hardcoded 5 s
        // input-dispatch deadline, and the resulting "isn't responding" dialog overlays the app —
        // obscuring screenshots and blocking taps mid-walkthrough. The ANR itself still lands in
        // logcat. Never touched on a physical device (a global, persistent setting); best-effort.
        if dev.serial.starts_with("emulator-") {
            let _ = adb(Some(&dev.serial))
                .args([
                    "shell",
                    "settings",
                    "put",
                    "global",
                    "hide_error_dialogs",
                    "1",
                ])
                .output();
        }
        // DAY_THEME must be in effect BEFORE the activity inflates: the manifest handles the
        // uiMode config change itself (no recreation), so an in-app UiModeManager flip leaves the
        // already-resolved window theme in the old scheme. Setting the DEVICE night mode first —
        // exactly what the system dark-mode toggle does — lets Material DayNight resolve the whole
        // theme coherently from the first frame.
        if let Some(theme) = spec
            .envs
            .iter()
            .find(|(k, _)| k == "DAY_THEME")
            .map(|(_, v)| v)
        {
            let night = match theme.as_str() {
                "dark" => Some("yes"),
                "light" => Some("no"),
                _ => None,
            };
            if let Some(night) = night {
                // Only set on an actual change, and give the system a moment to finish: the
                // config-change ripple is asynchronous, so an immediate `am start` can still
                // inflate the window under the OLD mode (views built moments later then resolve
                // in the new one — a half-themed screen).
                let cur = adb(Some(&dev.serial))
                    .args(["shell", "cmd", "uimode", "night"])
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase())
                    .unwrap_or_default();
                if !cur.contains(&format!(": {night}")) {
                    run_logged(
                        adb(Some(&dev.serial)).args(["shell", "cmd", "uimode", "night", night]),
                        &format!("uimode night {night} ({})", dev.serial),
                    )?;
                    std::thread::sleep(std::time::Duration::from_millis(1500));
                }
            }
        }
        // adb shell joins args into ONE device-shell command line — extras must be shell-quoted.
        let mut cmd = adb(Some(&dev.serial));
        cmd.args([
            "shell",
            "am",
            "start",
            "-n",
            &format!("{app_id}/dev.daybrite.day.bridge.DayActivity"),
        ]);
        for (k, v) in &spec.envs {
            let quoted = format!("'{}'", v.replace('\'', ""));
            if k == "AUTODRIVE" {
                cmd.args(["--es", "day.autodrive", &quoted]);
            } else {
                cmd.args(["--es", &format!("day.env.{k}"), &quoted]);
            }
        }
        if let Some(locale) = &spec.locale {
            cmd.args(["--es", "day.locale", &format!("'{locale}'")]);
        }
        status(
            "Launching",
            &format!("android-mdc ({app_id}) on {} ({})", dev.serial, dev.abi),
        );
        run_quiet(&mut cmd, &format!("am start ({})", dev.serial))?;
        if spec.attached {
            // One-device runs keep the bare `[android-mdc]` prefix; multi-device runs append
            // the serial so the interleaved log streams read apart.
            let label = if devices.len() > 1 {
                format!("{}:{}", outcome.target, dev.serial)
            } else {
                outcome.target.to_string()
            };
            log_threads.push(stream_logcat(dev.serial.clone(), app_id.clone(), label));
        }
    }
    // The returned handle joins every device's log pump; its exit code is the first non-zero.
    Ok(std::thread::spawn(move || {
        let mut code = 0;
        for t in log_threads {
            if let Ok(c) = t.join()
                && c != 0
                && code == 0
            {
                code = c;
            }
        }
        code
    }))
}

/// Stream one device's app logs (day-android redirects the app's stdout/stderr into logcat under
/// tag `day`). `-v tag` prefixes each line with `<prio>/day:`; map the priority to a stream
/// (I→stdout/blue, E/W/F→stderr/yellow) and re-prefix with `label`.
fn stream_logcat(serial: String, app_id: String, label: String) -> std::thread::JoinHandle<i32> {
    std::thread::spawn(move || {
        let pid = (0..20)
            .find_map(|_| {
                let p = adb(Some(&serial))
                    .args(["shell", "pidof", "-s", &app_id])
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();
                if p.is_empty() {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    None
                } else {
                    Some(p)
                }
            })
            .unwrap_or_default();
        if pid.is_empty() {
            emit_log(
                &label,
                LogStream::Err,
                "app pid not found; logs unavailable",
            );
            return 1;
        }
        // Clear this device's backlog so we only stream this run's output.
        let _ = adb(Some(&serial)).args(["logcat", "-c"]).status();
        let mut child = match adb(Some(&serial))
            .args(["logcat", "--pid", &pid, "-v", "tag", "day:V", "*:S"])
            .stdout(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                emit_log(&label, LogStream::Err, &format!("adb logcat: {e}"));
                return 1;
            }
        };
        crate::signals::register_child(child.id());
        if let Some(out) = child.stdout.take() {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let (prio, msg) = match line.split_once(':') {
                    Some((head, rest)) => {
                        (head.trim().chars().next().unwrap_or('I'), rest.trim_start())
                    }
                    None => ('I', line.as_str()),
                };
                let stream = if prio == 'E' || prio == 'F' || prio == 'W' {
                    LogStream::Err
                } else {
                    LogStream::Out
                };
                emit_log(&label, stream, msg);
            }
        }
        child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(0)
    })
}

#[cfg(test)]
mod abi_tests {
    use super::{android_build_abis, parse_abi_list};
    use std::sync::Mutex;

    /// Serialize `DAY_ANDROID_ABI` mutation (`set_var` is unsafe under concurrency in edition 2024).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn abi_list_parses_commas_spaces_and_empties() {
        assert_eq!(parse_abi_list("arm64-v8a"), vec!["arm64-v8a"]);
        assert_eq!(
            parse_abi_list("arm64-v8a,x86_64"),
            vec!["arm64-v8a", "x86_64"]
        );
        assert_eq!(
            parse_abi_list("arm64-v8a x86_64"),
            vec!["arm64-v8a", "x86_64"]
        );
        assert_eq!(
            parse_abi_list(" arm64-v8a , x86_64 "),
            vec!["arm64-v8a", "x86_64"]
        );
        assert!(parse_abi_list("").is_empty());
        assert!(parse_abi_list(" , ").is_empty());
    }

    #[test]
    fn day_android_abi_overrides_connected_devices() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // When set, the override is authoritative and short-circuits device detection (so this
        // test never touches adb): exactly the listed ABIs are built, deduped and sorted.
        // SAFETY: env access is serialized by ENV_LOCK and the var is restored before returning.
        unsafe { std::env::set_var("DAY_ANDROID_ABI", "x86_64,arm64-v8a,x86_64") };
        assert_eq!(android_build_abis(), vec!["arm64-v8a", "x86_64"]);
        unsafe { std::env::set_var("DAY_ANDROID_ABI", "x86_64") };
        assert_eq!(android_build_abis(), vec!["x86_64"]);
        unsafe { std::env::remove_var("DAY_ANDROID_ABI") };
    }
}
