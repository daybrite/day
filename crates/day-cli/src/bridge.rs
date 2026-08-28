// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! daybridge staging (docs/bridge.md, DESIGN.md §15.6) — the CLI half.
//!
//! Every bridged crate declares its arms in its own `src/**.rs`. This module finds those crates in
//! the app's dependency graph, renders the adapter for the target being built, and hands it to the
//! host project that compiles it: Swift into the generated `DayPieces` package, Kotlin or Java
//! into a Gradle `srcDir`, ArkTS into the hvigor module, JavaScript into the day-dom shim.
//!
//! **Adapters are rendered from source, not read out of build output.** day-build writes the
//! *Rust* half into `OUT_DIR` while cargo runs, which is far too late for staging that has to
//! finish before the platform build compiles; parsing the crate's own sources — through
//! `day_build::bridge`, the same parser the build script uses — makes staging independent of
//! whether cargo has ever run. It is also how the macOS Swift staging already treats
//! `[package.metadata.day.macos]` shims.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::meta::Project;

/// One target's staged adapters, keyed by the crate that declared them. The key becomes a
/// namespace directory, so two bridged crates cannot collide inside the one Swift module.
#[derive(Debug, Default, PartialEq)]
pub struct Staged {
    /// crate name → (file name, contents)
    pub swift: BTreeMap<String, (String, String)>,
    /// crate name → (package-relative path, contents) for the Android leg. The path's extension
    /// is `.kt` or `.java` depending on the arm's language, which is the only thing that differs.
    pub jvm: BTreeMap<String, (String, String)>,
    /// crate name → (module file name, contents) for web-dom.
    pub js: BTreeMap<String, (String, String)>,
    /// crate name → (module file name, contents) for HarmonyOS.
    pub arkts: BTreeMap<String, (String, String)>,
}

impl Staged {
    /// Whether this build stages nothing — the common case, and what callers short-circuit on.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.swift.is_empty() && self.jvm.is_empty() && self.js.is_empty() && self.arkts.is_empty()
    }

    /// Write the staged Swift into `sources`, one directory per crate, recording each path so the
    /// caller's prune keeps it (mtime-stable, like every other generated tree — DESIGN §17.5).
    pub fn write_swift(&self, sources: &Path, expected: &mut Vec<PathBuf>) -> Result<(), String> {
        for (krate, (file, contents)) in &self.swift {
            let dir = sources.join(krate.replace('-', "_"));
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let path = dir.join(file);
            crate::pieces::write_if_changed(&path, contents)?;
            expected.push(path);
        }
        Ok(())
    }
}

/// Render every bridge adapter that applies to `platform` (`macos`, `ios`, …) from the crates in
/// this app's dependency graph.
///
/// A crate declaring no bridge costs one directory scan. A parse error in one crate warns and is
/// skipped rather than failing the build here: day-build reports it properly, with the line, when
/// that crate compiles.
pub fn stage(project: &Project, platform: &str) -> Staged {
    let mut staged = Staged::default();
    for (name, root) in bridged_crates(project) {
        let bridge = match day_build::bridge::parse_crate(&root) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("day: {name}: bridge declaration ignored ({e})");
                continue;
            }
        };
        for arm in bridge
            .arms
            .iter()
            .filter(|a| a.platforms.iter().any(|p| p == platform))
        {
            match arm.lang {
                day_build::bridge::Lang::Swift => {
                    staged.swift.insert(
                        name.clone(),
                        (
                            day_build::bridge::adapter_name(arm, &name),
                            day_build::bridge::swift_adapter(&bridge, arm, &name),
                        ),
                    );
                }
                day_build::bridge::Lang::Kotlin | day_build::bridge::Lang::Java => {
                    // Gradle finds a class by its package path, so the file lands under the
                    // directories its `package` line names.
                    let dir = day_build::bridge::kotlin_package_of(&name).replace('.', "/");
                    staged.jvm.insert(
                        name.clone(),
                        (
                            format!("{dir}/{}", day_build::bridge::adapter_name(arm, &name)),
                            day_build::bridge::jvm_adapter(&bridge, arm, &name),
                        ),
                    );
                }
                day_build::bridge::Lang::Js => {
                    staged.js.insert(
                        name.clone(),
                        (
                            day_build::bridge::adapter_name(arm, &name),
                            day_build::bridge::js_adapter(&bridge, arm, &name),
                        ),
                    );
                }
                day_build::bridge::Lang::ArkTs => {
                    staged.arkts.insert(
                        name.clone(),
                        (
                            day_build::bridge::adapter_name(arm, &name),
                            day_build::bridge::arkts_adapter(&bridge, arm, &name),
                        ),
                    );
                }
                // C and C++ are compiled by cargo itself (day-build drives `cc`), so nothing is
                // staged for them here.
                _ => {}
            }
        }
    }
    staged
}

/// Write every staged JVM adapter — Kotlin or Java — under `root`, returning that directory when
/// anything landed; the caller adds it to Gradle's `java.srcDirs` through `day-pieces.json`.
pub fn write_jvm(project: &Project, staged: &Staged) -> Result<Option<PathBuf>, String> {
    let root = project.root.join("build/day/android/bridge");
    if staged.jvm.is_empty() {
        let _ = std::fs::remove_dir_all(&root);
        return Ok(None);
    }
    let mut expected: Vec<PathBuf> = Vec::new();
    for (relative, contents) in staged.jvm.values() {
        let path = root.join(relative);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        crate::pieces::write_if_changed(&path, contents)?;
        expected.push(path);
    }
    crate::pieces::prune_except(&root, &expected.into_iter().collect());
    Ok(Some(root))
}

/// Write every staged ES module into `dist/bridge/`, returning the import lines the day-dom shim
/// needs. Each module exports `register(rt)`; the shim spreads the results into its wasm imports,
/// which is what keeps a bridged crate's web arm in its own crate instead of in the shim.
pub fn write_js(dist: &Path, staged: &Staged) -> Result<Vec<String>, String> {
    let root = dist.join("bridge");
    if staged.js.is_empty() {
        let _ = std::fs::remove_dir_all(&root);
        return Ok(Vec::new());
    }
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let mut expected: Vec<PathBuf> = Vec::new();
    let mut modules: Vec<String> = Vec::new();
    for (file, contents) in staged.js.values() {
        let path = root.join(file);
        crate::pieces::write_if_changed(&path, contents)?;
        expected.push(path);
        modules.push(format!("./bridge/{file}"));
    }
    crate::pieces::prune_except(&root, &expected.into_iter().collect());
    Ok(modules)
}

/// Write every staged ArkTS module into the HarmonyOS host's `daypieces` tree, plus the
/// `DayBridges.ets` aggregator the host page imports. hvigor compiles ArkTS only from inside the
/// module, so these land in the project rather than under `build/day/` — the same rule the piece
/// modules follow (§15.2).
pub fn write_arkts(harmony: &Path, staged: &Staged) -> Result<(), String> {
    let root = harmony.join("entry/src/main/ets/daybridge");
    if staged.arkts.is_empty() {
        let _ = std::fs::remove_dir_all(&root);
        return Ok(());
    }
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let mut expected: Vec<PathBuf> = Vec::new();
    let mut imports = String::new();
    let mut calls = String::new();
    for (i, (file, contents)) in staged.arkts.values().enumerate() {
        let path = root.join(file);
        crate::pieces::write_if_changed(&path, contents)?;
        expected.push(path);
        let stem = file.trim_end_matches(".ets");
        imports.push_str(&format!(
            "import {{ register as register{i} }} from './{stem}';\n"
        ));
        calls.push_str(&format!("  Object.assign(out, register{i}());\n"));
    }
    let aggregator = root.join("DayBridges.ets");
    let body = format!(
        "// @generated by `day build` — the ArkTS arms of every bridged crate in this app\n         // (docs/bridge.md). The host calls `registerDayBridges()` once at startup.\n         {imports}\n         export function registerDayBridges(): Record<string, Function> {{\n         \x20 const out: Record<string, Function> = {{}};\n{calls}\x20 return out;\n}}\n"
    );
    crate::pieces::write_if_changed(&aggregator, &body)?;
    expected.push(aggregator);
    crate::pieces::prune_except(&root, &expected.into_iter().collect());
    Ok(())
}

/// The bridge platform a target stages for, or `None` when nothing is staged: C and C++ arms are
/// compiled by cargo itself, and the GTK/Qt/XAML desktop targets have no host project to stage a
/// foreign source INTO. `macos` is appkit-only for the same reason — only appkit has the
/// `platform/macos/` Xcode host to compile a staged Swift source.
pub fn platform_of(target_name: &str) -> Option<&'static str> {
    match target_name {
        "macos-appkit" => Some("macos"),
        "ios-uikit" => Some("ios"),
        "android-mdc" => Some("android"),
        "harmony-arkui" => Some("ohos"),
        "web-dom" => Some("web"),
        _ => None,
    }
}

/// Tell cargo that this target's staged foreign half will be in the link, which turns on the
/// `day_bridge_staged` cfg and with it the real arms (docs/bridge.md). Without this a bridged
/// crate compiles against its fallback and reports `Unsupported` — correct, but not what an app
/// that just staged a Kotlin adapter wants.
pub fn apply_staged(cmd: &mut std::process::Command, project: &Project, target_name: &str) {
    let Some(platform) = platform_of(target_name) else {
        return;
    };
    if !stage(project, platform).is_empty() {
        cmd.env("DAY_BRIDGE_STAGED", "1");
    }
}

/// Whether the app's Android scaffold can compile Kotlin at all.
///
/// `com.android.application` compiles `.java` from any source directory but routes `.kt` only
/// through the KOTLIN source set — and if nothing wires one, Gradle ignores the file **silently**:
/// the APK builds, installs, runs, and the first bridged call dies with `ClassNotFoundException`.
/// The reliable signal is the JetBrains Kotlin plugin. AGP 9 registers a `kotlin` extension but
/// does not compile `.kt` from a Day scaffold's source sets without it — wiring the staged root
/// into that extension was tried and silently compiled nothing, which is the same failure this
/// check exists to prevent.
///
/// A project that wires Kotlin some other way says so with `day: kotlin-ok` in the same file; the
/// check is a guard against a silent failure, not a claim to know every Gradle setup.
pub fn android_compiles_kotlin(project: &Project) -> bool {
    let gradle = project.root.join("platform/android/app/build.gradle.kts");
    std::fs::read_to_string(gradle)
        .map(|t| gradle_compiles_kotlin(&t))
        .unwrap_or(false)
}

fn gradle_compiles_kotlin(gradle: &str) -> bool {
    gradle.contains("org.jetbrains.kotlin.android")
        || gradle.contains("kotlin(\"android\")")
        || gradle.contains("day: kotlin-ok")
}

/// The crates staging a Kotlin (not Java) arm for Android — the ones that need the plugin.
pub fn kotlin_arm_crates(project: &Project) -> Vec<String> {
    let mut out = Vec::new();
    for (name, root) in bridged_crates(project) {
        let Ok(bridge) = day_build::bridge::parse_crate(&root) else {
            continue;
        };
        if bridge.arms.iter().any(|a| {
            a.lang == day_build::bridge::Lang::Kotlin && a.platforms.iter().any(|p| p == "android")
        }) {
            out.push(name);
        }
    }
    out
}

/// Libraries a bridged C/C++ arm asks the linker for that this host cannot resolve, as
/// `(crate, library)`.
///
/// `link = ["speechd"]` becomes `-lspeechd`, which needs the library's development package at
/// build time — and, once linked, at every launch. A missing one surfaces as a wall of linker
/// output naming no crate at all, usually on a CI machine rather than the author's. Probing here
/// turns that into a sentence.
///
/// Only the arms claiming THIS host's platform are probed: a Windows arm's `ole32` says nothing
/// about a Linux box. If there is no C compiler to probe with, nothing is reported — a missing
/// toolchain is a different problem with its own message.
pub fn unresolved_link_libs(project: &Project) -> Vec<(String, String)> {
    let host = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (name, root) in bridged_crates(project) {
        let Ok(bridge) = day_build::bridge::parse_crate(&root) else {
            continue;
        };
        for arm in bridge.arms.iter().filter(|a| {
            matches!(
                a.lang,
                day_build::bridge::Lang::C | day_build::bridge::Lang::Cpp
            ) && a.platforms.iter().any(|p| p == host)
        }) {
            for lib in arm
                .options
                .get("link")
                .map(|v| v.trim_matches(['[', ']']).to_string())
                .unwrap_or_default()
                .split(',')
                .map(|l| l.trim().trim_matches('"').to_string())
                .filter(|l| !l.is_empty())
            {
                if !links(&lib) {
                    out.push((name.clone(), lib));
                }
            }
        }
    }
    out
}

/// Whether the host's C compiler can link `-l<lib>`. Any failure to RUN the probe answers `true`,
/// so a machine with no compiler produces no findings rather than a false one.
fn links(lib: &str) -> bool {
    let dir = std::env::temp_dir().join(format!("day-linkprobe-{}", std::process::id()));
    if std::fs::create_dir_all(&dir).is_err() {
        return true;
    }
    let src = dir.join("probe.c");
    if std::fs::write(&src, "int main(void) { return 0; }\n").is_err() {
        return true;
    }
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let ok = std::process::Command::new(cc)
        .arg(&src)
        .arg(format!("-l{lib}"))
        .arg("-o")
        .arg(dir.join("probe"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(true);
    let _ = std::fs::remove_dir_all(&dir);
    ok
}

/// What to do about a library the linker cannot find.
pub fn link_help(missing: &[(String, String)]) -> String {
    let list = missing
        .iter()
        .map(|(krate, lib)| format!("{krate} → -l{lib}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "a bridged C/C++ arm links a library this host does not have: {list}.\n\
         \n\
         Install the DEVELOPMENT package that provides it — on Debian/Ubuntu that is usually \
         `lib<name>-dev`, on Fedora `<name>-devel`, on macOS a Homebrew formula. The runtime \
         package alone is not enough: the linker needs the unversioned `lib<name>.so` symlink \
         that only the -dev package installs.\n\
         \n\
         Consider whether the arm should link it at all. `link = [...]` writes a hard dependency \
         into the binary, so the app will not START on a machine without that library — right for \
         a system component, wrong for an optional service. Loading it with `dlopen` at first use \
         keeps the app launchable everywhere and lets the feature report Unsupported instead \
         (docs/bridge.md \"Linking\"; parts/day-part-speech's Linux arm is the worked example)."
    )
}

/// The fix, spelled out — the same text `day lint` and `day build` both print, so a developer who
/// hits it once recognizes it wherever it appears.
pub fn kotlin_plugin_help(crates: &[String]) -> String {
    format!(
        "{} declare(s) a Kotlin bridge arm, but platform/android/app/build.gradle.kts applies no \
         Kotlin plugin. Gradle would compile the generated Kotlin into nothing and the first call \
         would fail on the device with ClassNotFoundException, so `day build` refuses it.\n\
         \n\
         Pick whichever fits the project:\n\
         \n\
         1. Write the arm in Java — `#[day_bridge::impl(java, …)]`. It generates the same class \
         and the same JNI binding, and `.java` compiles in every Android project with no plugin at \
         all. This is what the shipped parts do (docs/bridge.md).\n\
         \n\
         2. Apply the Kotlin plugin in platform/android/app/build.gradle.kts:\n\
         \n\
         \x20      plugins {{\n\
         \x20          id(\"com.android.application\")\n\
         \x20          id(\"org.jetbrains.kotlin.android\") version \"2.2.0\"\n\
         \x20      }}\n\
         \n\
         \x20  On AGP 9 this can fail with \"Cannot add extension with name 'kotlin'\", since AGP \
         registers that extension itself; there, use its own Kotlin support instead of the \
         JetBrains plugin.\n\
         \n\
         3. If the project already compiles Kotlin some other way, add the staged root \
         (build/day/android/bridge) to its Kotlin source set and put `day: kotlin-ok` in that \
         gradle file to silence this check. Confirm the class actually lands in the APK — Gradle \
         ignores a .kt in an unwired source set without a word.",
        crates.join(", ")
    )
}

/// Every crate in the app's dependency graph that declares a bridge, as `(name, crate root)`.
fn bridged_crates(project: &Project) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = crate::pieces::dependency_roots(project)
        .into_iter()
        // A crate that declares a bridge depends on day-bridge, by construction. Requiring that
        // skips the two crates that merely CONTAIN the text `bridge!` — day-bridge itself, whose
        // doc comment shows the macro, and day-build, whose tests carry fixtures.
        .filter(|(name, root)| name != "day-bridge" && depends_on_day_bridge(root))
        .filter(|(_, root)| day_build::bridge::is_bridged(root))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Whether a crate's manifest lists day-bridge as a dependency.
fn depends_on_day_bridge(root: &Path) -> bool {
    std::fs::read_to_string(root.join("Cargo.toml"))
        .map(|t| t.contains("day-bridge"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    /// A crate whose source declares a Swift arm renders one adapter, with the prefixed symbol and
    /// the line mapping — all from source, with no cargo run and no `OUT_DIR`.
    #[test]
    fn renders_a_swift_arm_from_source() {
        let tmp = std::env::temp_dir().join(format!("day-bridge-stage-{}", std::process::id()));
        let src = tmp.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            r###"
day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" { fn hello_native() -> Result<(), day_bridge::Error>; }

    #[day_bridge::impl(swift, platforms = [ios, macos])]
    swift!(r#"
        func helloNative() throws { print("hi") }
    "#);

    #[day_bridge::impl(rust, platforms = [other])]
    fn hello_native() -> Result<(), day_bridge::Error> { Err(day_bridge::Error::Unsupported) }
}
"###,
        )
        .unwrap();

        assert!(day_build::bridge::is_bridged(&tmp));
        let bridge = day_build::bridge::parse_crate(&tmp).expect("parse");
        let arm = bridge
            .arms
            .iter()
            .find(|a| a.lang == day_build::bridge::Lang::Swift)
            .expect("swift arm");
        let swift = day_build::bridge::swift_adapter(&bridge, arm, "day-part-demo");

        assert!(swift.contains("func helloNative() throws"), "{swift}");
        assert!(
            swift.contains("@_cdecl(\"day_bridge_day_part_demo_hello_native\")"),
            "the adapter carries the prefixed symbol:\n{swift}"
        );
        assert!(
            swift.contains("#sourceLocation(file: \"src/lib.rs\""),
            "a swiftc error must name the .rs:\n{swift}"
        );
        assert_eq!(
            day_build::bridge::adapter_name(arm, "day-part-demo"),
            "day-part-demo-ios-macos.swift"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The scaffold Gradle file compiles `.java` but not `.kt`, so a Kotlin arm in a project
    /// without a Kotlin plugin has to be a build error — the alternative is an APK that installs
    /// and then dies with `ClassNotFoundException` (docs/bridge.md "Android").
    #[test]
    fn a_kotlin_arm_needs_the_plugin_and_java_does_not() {
        let scaffold = r#"
            plugins {
                id("com.android.application")
            }
            android { namespace = "dev.example.app" }
        "#;
        assert!(!super::gradle_compiles_kotlin(scaffold));
        assert!(super::gradle_compiles_kotlin(&scaffold.replace(
            "id(\"com.android.application\")",
            "id(\"com.android.application\")\n id(\"org.jetbrains.kotlin.android\")"
        )));
        // A project that wires Kotlin its own way opts out by saying so.
        assert!(super::gradle_compiles_kotlin(
            "// day: kotlin-ok — wired below\nplugins { id(\"com.android.application\") }"
        ));

        // The message names the crates and leads with the fix that always works.
        let help = super::kotlin_plugin_help(&["day-part-demo".to_string()]);
        assert!(help.contains("day-part-demo"), "{help}");
        assert!(help.contains("ClassNotFoundException"), "{help}");
        assert!(
            help.find("impl(java").unwrap() < help.find("org.jetbrains.kotlin.android").unwrap(),
            "Java is the first option offered:\n{help}"
        );
    }

    /// The probe has to be right in both directions: a false positive fails a build that would
    /// have linked, and a false negative is the linker wall this rule exists to replace.
    #[test]
    #[cfg(unix)]
    fn the_link_probe_tells_present_from_missing() {
        // libm is on every unix with a C compiler; if there is no compiler at all the probe
        // answers `true` by design, which this assertion also accepts.
        assert!(super::links("m"));
        assert!(
            !super::links("day-no-such-library-anywhere"),
            "a nonexistent library must not probe as linkable"
        );

        let help = super::link_help(&[("day-part-demo".into(), "speechd".into())]);
        assert!(help.contains("day-part-demo → -lspeechd"), "{help}");
        assert!(
            help.contains("-dev"),
            "names the package to install:\n{help}"
        );
        assert!(help.contains("dlopen"), "offers the alternative:\n{help}");
    }

    #[test]
    fn a_crate_without_a_bridge_is_skipped() {
        let tmp = std::env::temp_dir().join(format!("day-bridge-none-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("src")).unwrap();
        std::fs::write(tmp.join("src/lib.rs"), "pub fn ordinary() {}\n").unwrap();
        assert!(!day_build::bridge::is_bridged(&tmp));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
