//! day-toolchain — ONE place that knows where host toolchains and SDKs live, shared by the
//! `day` CLI and by crate build scripts (day-winui-sys, every `day-piece-*`/`day-tweak-*` that
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
/// compiling WinUI shims with `cc`.
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
    if cfg!(target_os = "windows") {
        if let Ok(v) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(v).join("Android").join("Sdk");
        }
    }
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    if cfg!(target_os = "macos") {
        home.join("Library/Android/sdk")
    } else {
        home.join("Android/Sdk")
    }
}

/// A JDK 21 home for Gradle (AGP needs 21 exactly — newer JDKs break the jdk-image transform).
///
/// Overrides: `JAVA_HOME` (trusted as-is — Gradle's own contract). Fallbacks: macOS's
/// `/usr/libexec/java_home -v 21` registry, then Homebrew's `openjdk@21` keg (both Apple-Silicon
/// and Intel prefixes). Callers export the result as `JAVA_HOME` for the Gradle child process.
pub fn jdk21_home() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("JAVA_HOME") {
        return Some(PathBuf::from(v));
    }
    if cfg!(target_os = "macos") {
        // The canonical macOS JDK registry (also finds Temurin/Zulu installs, not just brew).
        if let Ok(out) = std::process::Command::new("/usr/libexec/java_home")
            .args(["-v", "21"])
            .output()
            && out.status.success()
        {
            let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if p.join("bin/java").exists() {
                return Some(p);
            }
        }
        for prefix in ["/opt/homebrew", "/usr/local"] {
            let p = PathBuf::from(prefix).join("opt/openjdk@21");
            if p.join("bin/java").exists() {
                return Some(p);
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
}
