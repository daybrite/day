// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-toolchain — ONE place that knows where host toolchains and SDKs live, shared by the
//! `day` CLI and by crate build scripts (day-xaml-sys, every `day-piece-*`/`day-tweak-*` that
//! compiles its own native shim, and the scaffolds `day new` generates).
//!
//! Two rules govern every lookup here (docs/environment.md):
//!   1. **An environment variable always wins.** Each function documents its override(s).
//!   2. **No literal install paths.** Default locations are derived from the platform's own
//!      environment (`%ProgramFiles%`, `$HOME`, `%LOCALAPPDATA%`) — never a hardwired `C:\…`,
//!      so relocated installs (Windows Kits on `D:`, a portable SDK) work by setting one var.
//!
//! Functions that are meant to be called from build scripts have `_for_build_script` variants
//! that also emit the matching `cargo:rerun-if-env-changed=` lines, so changing an override
//! re-runs the script instead of silently keeping stale results.

use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Windows Kits (the Windows 10/11 SDK): cppwinrt headers + bin tools
// ---------------------------------------------------------------------------

/// Candidate `Windows Kits\10`-style roots, best first.
///
/// Overrides: `DAY_WINDOWS_KITS_ROOT` (the `…\Windows Kits\10` directory itself), then the
/// MS-standard `WindowsSdkDir` (set by Visual Studio developer shells). Fallbacks derive from
/// `%ProgramFiles(x86)%` / `%ProgramFiles%` — the env vars, not literal `C:\` paths.
pub fn windows_kits_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(v) = std::env::var("DAY_WINDOWS_KITS_ROOT") {
        roots.push(PathBuf::from(v));
    }
    if let Ok(v) = std::env::var("WindowsSdkDir") {
        roots.push(PathBuf::from(v));
    }
    for pf in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Ok(v) = std::env::var(pf) {
            roots.push(PathBuf::from(v).join("Windows Kits").join("10"));
        }
    }
    roots.dedup();
    roots
}

/// The newest `Include\<version>\cppwinrt` directory (the C++/WinRT projection headers), for
/// compiling XAML shims with `cc`.
///
/// Overrides: `DAY_CPPWINRT` (the exact cppwinrt include dir — highest priority), then the
/// roots from [`windows_kits_roots`]. Validated by `winrt/base.h`.
pub fn cppwinrt_include() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("DAY_CPPWINRT") {
        let p = PathBuf::from(v);
        if p.join("winrt").join("base.h").exists() {
            return Some(p);
        }
        // An explicit override that doesn't validate is a configuration error worth surfacing
        // loudly in a build script; returning None lets the caller's expect() name the fix.
        return None;
    }
    let mut found: Vec<PathBuf> = Vec::new();
    for root in windows_kits_roots() {
        let Ok(rd) = std::fs::read_dir(root.join("Include")) else {
            continue;
        };
        for entry in rd.flatten() {
            let cppwinrt = entry.path().join("cppwinrt");
            if cppwinrt.join("winrt").join("base.h").exists() {
                found.push(cppwinrt);
            }
        }
    }
    found.sort(); // version dirs sort lexicographically; newest last
    found.pop()
}

/// [`cppwinrt_include`] for build scripts: also emits the `rerun-if-env-changed` lines so an
/// override change re-runs the script.
pub fn cppwinrt_include_for_build_script() -> Option<PathBuf> {
    for var in ["DAY_CPPWINRT", "DAY_WINDOWS_KITS_ROOT", "WindowsSdkDir"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    cppwinrt_include()
}

/// A Windows-Kits bin tool (`signtool.exe`, `makeappx.exe`, …): newest SDK version, host arch.
///
/// Overrides: `DAY_WINDOWS_KIT` (a bin directory containing the tool), then the tool on `PATH`,
/// then `bin\<version>\<arch>` under each [`windows_kits_roots`] root.
pub fn windows_kit_tool(tool: &str) -> Option<PathBuf> {
    if let Ok(root) = std::env::var("DAY_WINDOWS_KIT") {
        let p = PathBuf::from(root).join(tool);
        if p.exists() {
            return Some(p);
        }
    }
    if let Some(p) = on_path(tool) {
        return Some(p);
    }
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    for root in windows_kits_roots() {
        let Ok(rd) = std::fs::read_dir(root.join("bin")) else {
            continue;
        };
        let mut versions: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("10."))
            })
            .collect();
        versions.sort();
        while let Some(v) = versions.pop() {
            let candidate = v.join(arch).join(tool);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// NSIS
// ---------------------------------------------------------------------------

/// The `makensis` NSIS compiler (cross-platform: apt/brew/choco all put it on PATH).
///
/// Overrides: `DAY_MAKENSIS` (the executable itself), then `PATH`, then the conventional
/// Windows install dir under `%ProgramFiles(x86)%` / `%ProgramFiles%`.
pub fn makensis() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("DAY_MAKENSIS") {
        let p = PathBuf::from(v);
        if p.is_file() {
            return Some(p);
        }
        return None; // explicit override that doesn't exist = configuration error, don't mask it
    }
    if let Some(p) = on_path("makensis").or_else(|| on_path("makensis.exe")) {
        return Some(p);
    }
    for pf in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Ok(v) = std::env::var(pf) {
            let p = PathBuf::from(v).join("NSIS").join("makensis.exe");
            if p.exists() {
                return Some(p);
            }
        }
    }
    // Chocolatey (`choco install nsis`) — the way CI and most Windows devs get it. Its shim lands
    // in the chocolatey bin dir, which IS on the machine PATH, but a PATH edit made by an install
    // does not reach an ALREADY-RUNNING process: GitHub Actions hands every step the environment
    // captured when the job started, so `choco install` in one step leaves the next step's PATH
    // untouched. Probing the location directly is what makes the install usable in the same job.
    let choco = std::env::var("ChocolateyInstall")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\ProgramData\chocolatey"));
    let shim = choco.join("bin").join("makensis.exe");
    if shim.is_file() {
        return Some(shim);
    }
    // The package's own tree, when it unpacks rather than shimming. The directory under `tools`
    // carries the NSIS version, so scan one level instead of guessing it.
    let tools = choco.join("lib").join("nsis").join("tools");
    if let Ok(entries) = std::fs::read_dir(&tools) {
        for entry in entries.flatten() {
            for candidate in [
                entry.path().join("makensis.exe"),
                entry.path().join("Bin").join("makensis.exe"),
            ] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Android SDK + JDK
// ---------------------------------------------------------------------------

/// The Android SDK root.
///
/// Overrides: `ANDROID_HOME`, then `ANDROID_SDK_ROOT` (both standard). Falls back to each
/// platform's default install location: `~/Library/Android/sdk` (macOS),
/// `%LOCALAPPDATA%\Android\Sdk` (Windows), `~/Android/Sdk` (Linux — Android Studio's default).
pub fn android_sdk_dir() -> PathBuf {
    if let Ok(v) = std::env::var("ANDROID_HOME").or_else(|_| std::env::var("ANDROID_SDK_ROOT")) {
        return PathBuf::from(v);
    }
    if cfg!(target_os = "windows")
        && let Ok(v) = std::env::var("LOCALAPPDATA")
    {
        return PathBuf::from(v).join("Android").join("Sdk");
    }
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    if cfg!(target_os = "macos") {
        home.join("Library/Android/sdk")
    } else {
        home.join("Android/Sdk")
    }
}

/// A JDK home for the Gradle/AGP build. AGP 9's minimum is JDK 17, and Gradle must support the
/// exact version — Gradle 9.6 runs on 17…26 (verified: the day scaffold builds on 17, 21 and 26
/// alike, so the old "21 exactly / 22+ breaks the jdk-image transform" restriction was an AGP-8-era
/// carryover and no longer holds).
///
/// Overrides: `JAVA_HOME` (trusted as-is — Gradle's own contract). Fallbacks: macOS's
/// `/usr/libexec/java_home -v 17+` registry (the newest install ≥ 17), then a Homebrew `openjdk`
/// keg — the unversioned latest first, then pinned 17+ kegs (both Apple-Silicon and Intel
/// prefixes). Callers export the result as `JAVA_HOME` for the Gradle child process.
pub fn jdk_home() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("JAVA_HOME") {
        return Some(PathBuf::from(v));
    }
    if cfg!(target_os = "macos") {
        // The canonical macOS JDK registry (also finds Temurin/Zulu installs, not just brew).
        if let Ok(out) = std::process::Command::new("/usr/libexec/java_home")
            .args(["-v", "17+"])
            .output()
            && out.status.success()
        {
            let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if p.join("bin/java").exists() {
                return Some(p);
            }
        }
        // Newest keg first: the unversioned `openjdk` is Homebrew's current, then LTS/common pins.
        for keg in ["openjdk", "openjdk@21", "openjdk@17"] {
            for prefix in ["/opt/homebrew", "/usr/local"] {
                let p = PathBuf::from(prefix).join("opt").join(keg);
                if p.join("bin/java").exists() {
                    return Some(p);
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// rustup
// ---------------------------------------------------------------------------

/// The rustup toolchain to use for cross-std builds (mobile targets need rustup's target std;
/// a Homebrew/system rustc has none), as `(cargo_path, bin_dir)`. The bin dir is prepended to
/// `PATH` so the toolchain's own `rustc` — not one earlier on `PATH` — is what cargo invokes.
///
/// Overrides: `RUSTUP_HOME` (standard; default `~/.rustup`). Among installed toolchains a
/// `stable-*` one is preferred, then the lexicographically first — deterministic where the old
/// first-directory-wins behavior depended on filesystem order.
pub fn rustup_cargo() -> Result<(PathBuf, PathBuf), String> {
    let rustup_home = std::env::var("RUSTUP_HOME")
        .map(PathBuf::from)
        .or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".rustup"))
                .map_err(|e| e.to_string())
        })?;
    let toolchains = rustup_home.join("toolchains");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&toolchains)
        .map_err(|_| "no rustup toolchains (cross-std needs rustup, not Homebrew rust)")?
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    let chosen = entries
        .iter()
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("stable-"))
        })
        .or_else(|| entries.first())
        .ok_or("empty rustup toolchains dir")?;
    let bin = chosen.join("bin");
    Ok((bin.join("cargo"), bin))
}

// ---------------------------------------------------------------------------

fn on_path(tool: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(tool))
        .find(|p| p.is_file())
}

/// True when `dir` looks like a usable directory (exists and is a dir) — small helper for
/// callers validating overrides.
pub fn is_dir(dir: &Path) -> bool {
    dir.is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kits_roots_honor_override_first() {
        // SAFETY: test-local env mutation; tests touch distinct vars.
        unsafe { std::env::set_var("DAY_WINDOWS_KITS_ROOT", "/custom/kits/10") };
        let roots = windows_kits_roots();
        assert_eq!(roots[0], PathBuf::from("/custom/kits/10"));
        unsafe { std::env::remove_var("DAY_WINDOWS_KITS_ROOT") };
    }

    #[test]
    fn android_sdk_honors_android_home() {
        unsafe { std::env::set_var("ANDROID_HOME", "/custom/android") };
        assert_eq!(android_sdk_dir(), PathBuf::from("/custom/android"));
        unsafe { std::env::remove_var("ANDROID_HOME") };
    }

    #[test]
    fn explicit_cppwinrt_override_must_validate() {
        unsafe { std::env::set_var("DAY_CPPWINRT", "/does/not/exist") };
        assert_eq!(cppwinrt_include(), None); // bad override surfaces, not masked by fallbacks
        unsafe { std::env::remove_var("DAY_CPPWINRT") };
    }

    #[test]
    fn explicit_makensis_override_must_validate() {
        unsafe { std::env::set_var("DAY_MAKENSIS", "/does/not/exist/makensis.exe") };
        assert_eq!(makensis(), None); // same contract as the other overrides: never masked
        unsafe { std::env::remove_var("DAY_MAKENSIS") };
    }

    /// The layouts `choco install nsis` can leave behind. Each is built for real under a temp
    /// `ChocolateyInstall` so the probe is exercised rather than assumed — this is the lookup that
    /// failed a release build after NSIS had actually been installed.
    #[test]
    fn makensis_found_in_chocolatey_layouts() {
        let base = std::env::temp_dir().join(format!("day-choco-probe-{}", std::process::id()));
        let shimmed = base.join("shim");
        let unpacked = base.join("unpacked");
        let nested = base.join("nested");
        let _ = std::fs::remove_dir_all(&base);

        // 1. the shim chocolatey drops in its bin dir
        let shim_exe = shimmed.join("bin").join("makensis.exe");
        std::fs::create_dir_all(shim_exe.parent().unwrap()).unwrap();
        std::fs::write(&shim_exe, b"").unwrap();

        // 2. unpacked under lib/nsis/tools/<versioned dir>/
        let flat = unpacked
            .join("lib/nsis/tools")
            .join("nsis-3.10")
            .join("makensis.exe");
        std::fs::create_dir_all(flat.parent().unwrap()).unwrap();
        std::fs::write(&flat, b"").unwrap();

        // 3. …with the executable one level deeper, in Bin/
        let deep = nested
            .join("lib/nsis/tools")
            .join("nsis-3.10")
            .join("Bin")
            .join("makensis.exe");
        std::fs::create_dir_all(deep.parent().unwrap()).unwrap();
        std::fs::write(&deep, b"").unwrap();

        // The earlier probes must not answer first, or this proves nothing — and on a machine that
        // really has NSIS in Program Files they would. Saved and put back below: PATH in particular
        // is process-global, and leaving it empty would poison every test that runs after this one.
        let (path, pf, pf86) = (
            std::env::var_os("PATH"),
            std::env::var_os("ProgramFiles"),
            std::env::var_os("ProgramFiles(x86)"),
        );
        unsafe {
            std::env::remove_var("DAY_MAKENSIS");
            std::env::set_var("PATH", "");
            std::env::set_var("ProgramFiles", base.join("no-such-pf"));
            std::env::set_var("ProgramFiles(x86)", base.join("no-such-pf86"));
        }

        let found: Vec<_> = [(&shimmed, &shim_exe), (&unpacked, &flat), (&nested, &deep)]
            .iter()
            .map(|(root, want)| {
                unsafe { std::env::set_var("ChocolateyInstall", root) };
                (makensis(), (*want).clone())
            })
            .collect();

        unsafe {
            std::env::remove_var("ChocolateyInstall");
            match path {
                Some(v) => std::env::set_var("PATH", v),
                None => std::env::remove_var("PATH"),
            }
            match pf {
                Some(v) => std::env::set_var("ProgramFiles", v),
                None => std::env::remove_var("ProgramFiles"),
            }
            match pf86 {
                Some(v) => std::env::set_var("ProgramFiles(x86)", v),
                None => std::env::remove_var("ProgramFiles(x86)"),
            }
        }
        let _ = std::fs::remove_dir_all(&base);

        // Asserted only after the environment is back, so a failure can't take the rest with it.
        for (got, want) in found {
            assert_eq!(got.as_ref(), Some(&want), "chocolatey layout {want:?}");
        }
    }
}
