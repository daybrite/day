// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The macos-appkit Swift prepass (docs/swiftui.md). When any dependency contributes macOS Swift
//! (`[package.metadata.day.macos]` — the day-piece-swiftui shim, an app's local SwiftPM package),
//! `day build` compiles the generated `build/day/macos/DayPieces` package with `swift build` and
//! statically links its archives into the cargo binary.
//!
//! This is the **bare-cargo** half of macos-appkit's dual-mode build (§16.5, 2026-08): an app
//! carrying `platform/macos/DayApp.xcodeproj` builds through xcodebuild instead
//! ([`crate::mobile::macos_xcode_enabled`]), and there the pbxproj references the same generated
//! package rather than this prepass linking it. `DAY_MACOS_XCODE=0` or a project without the
//! scaffold takes the path below. Either way the Swift *runtime* is the OS's own dylibs
//! (`/usr/lib/swift`, macOS ≥ 10.14.4), so `day pack`'s codesign/notarize flow is untouched.

use std::path::PathBuf;
use std::process::Command;

use crate::cli::Profile;
use crate::meta::Project;
use crate::ops::status;
use crate::pieces::MacosSwift;

/// The link inputs a completed Swift prepass hands the cargo invocation.
pub struct SwiftLink {
    /// The products directory (`swift build --show-bin-path`) — the `-L` search path.
    search: PathBuf,
    /// The force-loaded DayPieces archive: `-force_load` keeps every object in it, so the
    /// provider classes nothing references by symbol (they're found via NSClassFromString)
    /// survive the link (docs/swiftui.md).
    force_load: PathBuf,
    /// The other static archives in the products dir (the local packages' modules), linked
    /// normally — the force-loaded glue references what it needs from them.
    libs: Vec<String>,
    /// System frameworks from `[package.metadata.day.macos].frameworks`.
    frameworks: Vec<String>,
    /// The deployment target the Swift objects were built for — exported as
    /// `MACOSX_DEPLOYMENT_TARGET` so the cargo link agrees on the minimum OS.
    pub platform: String,
}

impl SwiftLink {
    /// The trailing rustc arguments for `cargo rustc -- …`. They fingerprint only the final bin
    /// crate (the xaml-manifest precedent in ops.rs), so no RUSTFLAGS and no workspace rebuilds;
    /// ordering is deterministic so an unchanged prepass produces an unchanged command line.
    pub fn rustc_args(&self) -> Vec<String> {
        let mut args = vec![
            format!("-Clink-arg=-Wl,-force_load,{}", self.force_load.display()),
            format!("-Lnative={}", self.search.display()),
        ];
        for lib in &self.libs {
            args.push(format!("-lstatic={lib}"));
        }
        // The Swift runtime + concurrency stubs: the toolchain's autolink entries (LC_LINKER_OPTION
        // in every Swift object) name libswiftCore & friends; these search paths let ld resolve
        // them against the SDK's .tbd stubs, and the installed binary uses the OS dylibs.
        args.push("-Clink-arg=-L/usr/lib/swift".into());
        if let Some(sdk) = sdk_path() {
            args.push(format!("-Clink-arg=-L{sdk}/usr/lib/swift"));
        }
        for fw in &self.frameworks {
            args.push("-Clink-arg=-framework".into());
            args.push(format!("-Clink-arg={fw}"));
        }
        args
    }
}

/// `swift build` the generated DayPieces package for `profile` and locate its archives.
/// Fails with a targeted hint when the Swift toolchain is missing — only builds that actually
/// embed Swift ever get here.
pub fn build_day_pieces(
    project: &Project,
    profile: Profile,
    swift: &MacosSwift,
) -> Result<SwiftLink, String> {
    if Command::new("swift")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        return Err(
            "a dependency contributes macOS Swift sources (docs/swiftui.md) but `swift` is not \
             runnable — install Xcode or the command-line tools (`xcode-select --install`)"
                .into(),
        );
    }

    let configuration = profile.as_str();
    // One scratch dir per package (not per profile — SwiftPM keys artifacts by configuration
    // inside it), kept out of the package dir so regeneration can't invalidate it.
    let scratch = project.root.join("build/day/macos/swift");
    status(
        "Building",
        &format!("DayPieces (swift build, {configuration})"),
    );
    let mut cmd = Command::new("swift");
    crate::ops::apply_determinism(&mut cmd);
    cmd.arg("build")
        .arg("--package-path")
        .arg(&swift.package)
        .args(["--configuration", configuration])
        .arg("--scratch-path")
        .arg(&scratch);
    let out = crate::ops::run_capture(&mut cmd, "swift build")?;
    if !out.status.success() {
        return Err(format!(
            "swift build failed for {}:\n{}",
            swift.package.display(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let bin = Command::new("swift")
        .arg("build")
        .arg("--package-path")
        .arg(&swift.package)
        .args(["--configuration", configuration])
        .arg("--scratch-path")
        .arg(&scratch)
        .arg("--show-bin-path")
        .output()
        .map_err(|e| format!("swift build --show-bin-path: {e}"))?;
    let search = PathBuf::from(String::from_utf8_lossy(&bin.stdout).trim().to_string());

    // Every static archive in the products dir: libDayPieces.a (force-loaded) plus one per local
    // package module the target depends on.
    let mut libs = Vec::new();
    let mut force_load = None;
    for entry in std::fs::read_dir(&search)
        .map_err(|e| format!("{}: {e}", search.display()))?
        .flatten()
    {
        let path = entry.path();
        let Some(name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix("lib"))
            .and_then(|n| n.strip_suffix(".a"))
        else {
            continue;
        };
        if name == "DayPieces" {
            force_load = Some(path);
        } else {
            libs.push(name.to_string());
        }
    }
    libs.sort();
    let force_load = force_load.ok_or_else(|| {
        format!(
            "swift build produced no libDayPieces.a in {} — the generated package should declare \
             a static library product",
            search.display()
        )
    })?;

    Ok(SwiftLink {
        search,
        force_load,
        libs,
        frameworks: swift.frameworks.clone(),
        platform: swift.platform.clone(),
    })
}

/// The macOS SDK path (`xcrun --sdk macosx --show-sdk-path`), for the Swift stub-library search.
fn sdk_path() -> Option<String> {
    let out = Command::new("xcrun")
        .args(["--sdk", "macosx", "--show-sdk-path"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}
