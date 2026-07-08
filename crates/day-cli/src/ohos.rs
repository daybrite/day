//! HarmonyOS / OpenHarmony (`ohos-arkui`) pipeline — the OHOS analogue of mobile.rs's android/iOS
//! pipelines. `build_ohos` cross-compiles the app to `libentry.so`, then packages + signs a `.hap`
//! via the ArkTS host project under `<project>/harmony/`; `launch_ohos` installs + starts it on a
//! connected emulator/device over `hdc`.
//!
//! The reference emulator is the openharmony-rs `emulator-action` Oniro QEMU image: an **x86_64**,
//! software-emulated (TCG), NETWORKED hdc target — so every hdc call carries `-t <connect-key>`
//! (default `127.0.0.1:55555`; override with `DAY_OHOS_TARGET`). Building a `.hap` needs `hvigor` +
//! `ohpm` on PATH (from the OpenHarmony command-line-tools), the SDK via `OHOS_BASE_SDK_HOME` /
//! `OHOS_NDK_HOME` (e.g. from `openharmony-rs/setup-ohos-sdk`), and a JDK for signing. Two OHOS-only
//! gotchas the code accounts for (see the CI research): `aa start` exits 0 even when the launch is
//! refused (so we parse its output for `Error Code:`), and `snapshot_display` writes JPEG (so the
//! screenshot path prefers `uitest screenCap`, which writes PNG). See docs/harmonyos.md.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::meta::Project;
use crate::mobile::{run_logged, rustup_cargo};
use crate::ops::{BuildOutcome, LaunchSpec, LogStream, emit_log, status};
use crate::targets::Target;

/// The hdc target key (`-t`) for the emulator/device. Oniro's QEMU emulator is a networked target
/// reachable at the emulator-action connect-key `127.0.0.1:55555`; override via `DAY_OHOS_TARGET`
/// (a real device's connect key, or a different port).
pub fn ohos_target() -> String {
    std::env::var("DAY_OHOS_TARGET").unwrap_or_else(|_| "127.0.0.1:55555".into())
}

/// A fresh `hdc` command with the networked-target selector applied. `hdc` is taken from PATH when
/// present; otherwise it is resolved from the SDK install's sibling `toolchains/` dir (the public
/// SDK ships it there, next to the `native` NDK) — so `day launch -p ohos-arkui` works from
/// GUI-launched editors whose environment has neither the variable nor the PATH entry.
pub fn hdc() -> Command {
    static HDC: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let program = HDC.get_or_init(|| {
        // On PATH? Use it as-is.
        let on_path = std::env::var("PATH")
            .is_ok_and(|path| std::env::split_paths(&path).any(|d| d.join("hdc").is_file()));
        if on_path {
            return "hdc".into();
        }
        // Else probe the SDK roots the NDK detection knows about.
        if let Ok(ndk) = find_ohos_ndk() {
            let cand = Path::new(&ndk).parent().map(|p| p.join("toolchains/hdc"));
            if let Some(c) = cand
                && c.is_file()
            {
                return c.to_string_lossy().into_owned();
            }
        }
        "hdc".into()
    });
    let mut c = Command::new(program);
    c.args(["-t", &ohos_target()]);
    c
}

/// The OpenHarmony NDK (`native` dir) for the cross-linker: `OHOS_NDK_HOME` (set by CI's
/// setup-ohos-sdk) if present, else a couple of common local install paths (see docs/harmonyos.md:
/// extract the public SDK's `native` component). Validated by the presence of `llvm/bin`.
pub(crate) fn find_ohos_ndk() -> Result<String, String> {
    if let Ok(v) = std::env::var("OHOS_NDK_HOME") {
        return Ok(v);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    for cand in [
        format!("{home}/ohos/ndk-extract/native"),
        format!("{home}/ohos-sdk/native"),
    ] {
        if Path::new(&cand).join("llvm/bin").is_dir() {
            return Ok(cand);
        }
    }
    Err(
        "OHOS_NDK_HOME is not set and no OpenHarmony NDK was found — set it to the SDK's `native` \
         directory (see docs/harmonyos.md)"
            .into(),
    )
}

/// Which ABI to cross-compile: the Oniro emulator runs an x86_64 image; a real device is arm64.
/// Overridable with `DAY_OHOS_ARCH` (`emulator` | `device`).
fn build_arch() -> &'static str {
    match std::env::var("DAY_OHOS_ARCH").ok().as_deref() {
        Some("device") | Some("arm64") | Some("arm64-v8a") => "device",
        _ => "emulator",
    }
}

pub fn build_ohos(
    project: &Project,
    target: &'static Target,
    profile: &str,
    start: std::time::Instant,
) -> Result<BuildOutcome, String> {
    let harmony = project.root.join("harmony");
    if !harmony.join("build-profile.json5").exists() {
        return Err(format!(
            "ohos-arkui: no ArkTS host project at {} — a HarmonyOS app needs a `harmony/` project \
             (the hvigor project + sign-hap.mjs), like apps/showcase/harmony. See docs/harmonyos.md.",
            harmony.display()
        ));
    }

    // 1) Cross-compile the app to a cdylib for the emulator/device ABI, then stage it as
    //    entry/libs/<abi>/libentry.so — the .so the ArkTS host imports (its NAPI module is "entry").
    //    Uses the OHOS NDK cross-linker (OHOS_NDK_HOME) + a rustup toolchain (Homebrew rustc ships no
    //    OHOS std) and `feature_selection("arkui")` (the arkui toolkit feature + every standalone
    //    piece's `<pkg>/arkui` renderer feature, Tier A.2), exactly like the android/iOS legs.
    let (triple, abi) = match build_arch() {
        "device" => ("aarch64-unknown-linux-ohos", "arm64-v8a"),
        _ => ("x86_64-unknown-linux-ohos", "x86_64"),
    };
    let ndk = find_ohos_ndk()?;
    let (cargo, bin) = rustup_cargo()?;
    let name = project.manifest.app.name.clone();
    let target_dir = project
        .root
        .join("build/day/cargo/ohos-arkui")
        .join(profile);
    let linker_var = format!(
        "CARGO_TARGET_{}_LINKER",
        triple.to_uppercase().replace('-', "_")
    );
    status("Building", &format!("{} (cargo cdylib {abi})", target.name));
    let mut cmd = Command::new(&cargo);
    cmd.current_dir(&project.root)
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("CARGO_TARGET_DIR", &target_dir)
        .env(&linker_var, format!("{ndk}/llvm/bin/{triple}-clang"))
        // day-arkui-sys's build.rs compiles the C++ shim with the NDK clang and reads this
        // variable itself — export the RESOLVED path so auto-detected local installs work even
        // when the parent environment (a GUI-launched editor) never set it.
        .env("OHOS_NDK_HOME", &ndk)
        .args([
            "rustc",
            "-p",
            &name,
            "--lib",
            "--crate-type",
            "cdylib",
            "--no-default-features",
            "--features",
            &crate::ops::feature_selection(project, "arkui"),
            "--target",
            triple,
        ]);
    if profile == "release" {
        cmd.arg("--release");
    }
    run_logged(&mut cmd, "cargo (ohos)")?;
    // The cdylib is `lib<[lib].name>.so` (libentry.so for a crate whose `[lib] name = "entry"`, else
    // lib<crate>.so) — find the single produced .so and stage it AS libentry.so for the ArkTS host.
    let out_dir = target_dir.join(triple).join(profile);
    let so = std::fs::read_dir(&out_dir)
        .map_err(|e| format!("reading {}: {e}", out_dir.display()))?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|x| x.to_str()) == Some("so"))
        .ok_or_else(|| format!("no cdylib .so produced in {}", out_dir.display()))?;
    let libs = harmony.join("entry/libs").join(abi);
    std::fs::create_dir_all(&libs).map_err(|e| format!("mkdir {}: {e}", libs.display()))?;
    std::fs::copy(&so, libs.join("libentry.so")).map_err(|e| format!("stage libentry.so: {e}"))?;

    // 2) Assemble the .hap with hvigor (compiles the ArkTS host + packs the native libs + resources).
    //    hvigor + ohpm come from the OpenHarmony command-line-tools (on PATH); the SDK from
    //    OHOS_BASE_SDK_HOME. `ohpm install` is best-effort (the app has only a local dependency).
    status(
        "Building",
        &format!("{} (hvigorw assembleHap)", target.name),
    );
    let _ = Command::new("ohpm")
        .arg("install")
        .current_dir(&harmony)
        .status();

    let mode = if profile == "release" {
        "release"
    } else {
        "debug"
    };
    // A missing hvigor otherwise surfaces as a bare spawn ENOENT — check up front and say what to
    // install (it is NOT part of the public SDK; the `native` NDK alone only covers the Rust step).
    let hvigor_on_path = std::env::var("PATH")
        .is_ok_and(|p| std::env::split_paths(&p).any(|d| d.join("hvigorw").is_file()));
    if !hvigor_on_path {
        return Err(
            "hvigorw not found on PATH — the Rust cross-compile succeeded, but packaging the .hap \
             needs the OpenHarmony command-line-tools (hvigor + ohpm; bundled with DevEco Studio). \
             Install them and put their bin/ on PATH — see docs/harmonyos.md."
                .into(),
        );
    }
    let mut hv = Command::new("hvigorw");
    hv.current_dir(&harmony).args([
        "assembleHap",
        "--mode",
        "module",
        "-p",
        "product=default",
        "-p",
        &format!("buildMode={mode}"),
        "--no-daemon",
    ]);
    run_logged(&mut hv, "hvigorw assembleHap")?;

    // 3) Patch + sign the assembled (unsigned) .hap via harmony/sign-hap.mjs: it rewrites module.json's
    //    compileSdkType to "OpenHarmony" (so the emulator skips code-sign verification — see the script)
    //    then signs with the OpenHarmony public release material.
    let hap = sign_hap(&harmony, &ndk)?;
    status("Built", &format!("{} → {}", target.name, hap.display()));
    Ok(BuildOutcome {
        target: target.name,
        artifact: hap,
        seconds: start.elapsed().as_secs_f64(),
    })
}

/// Recursively find the first `*.hap` under `dir` whose file name satisfies `pred`.
fn find_hap(dir: &Path, pred: impl Fn(&str) -> bool) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("hap") {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if pred(name) {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Patch + sign the hvigor-built unsigned hap via the project's `sign-hap.mjs <unsigned> <signed>`
/// (Node — hvigor already requires it). The script rewrites module.json's compileSdkType to
/// "OpenHarmony" so the emulator skips code-sign verification (the public release cert's code
/// signature is otherwise rejected with 9568393), then signs with the SDK's release material.
fn sign_hap(harmony: &Path, ndk: &str) -> Result<PathBuf, String> {
    let build = harmony.join("entry/build");
    // hvigor emits `entry-<product>-unsigned.hap`; fall back to any hap.
    let unsigned = find_hap(&build, |n| n.contains("unsigned"))
        .or_else(|| find_hap(&build, |_| true))
        .ok_or_else(|| format!("no .hap produced under {}", build.display()))?;
    let sign = harmony.join("sign-hap.mjs");
    if !sign.exists() {
        // No patcher/signer — hand back the unsigned hap and let the install surface the rejection.
        return Ok(unsigned);
    }
    let signed = unsigned.with_file_name("day-signed.hap");
    status("Signing", &signed.display().to_string());
    let mut cmd = Command::new("node");
    cmd.arg(&sign)
        .arg(&unsigned)
        .arg(&signed)
        // The script locates the SDK signing material relative to the NDK (its findLib probes
        // OHOS_NDK_HOME first) — hand it the resolved path, like the cargo step.
        .env("OHOS_NDK_HOME", ndk)
        .current_dir(harmony);
    run_logged(&mut cmd, "sign-hap.mjs")?;
    Ok(signed)
}

/// Combined stdout+stderr of a finished command, as one string.
fn combined(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Is `bundle` installed on the target? `hdc install`/`bm install` can print `error: failed to
/// execute your command` yet still install (and yet exit 0), so verify the end state with
/// `bm dump -a` — the flat list of every installed bundle name — a clean membership test (unlike
/// `bm dump -n <bundle>`, whose per-bundle JSON can itself contain the words "error"/"failed").
fn bundle_installed(bundle: &str) -> bool {
    hdc()
        .args(["shell", "bm", "dump", "-a"])
        .output()
        .map(|o| combined(&o).contains(bundle))
        .unwrap_or(false)
}

pub fn launch_ohos(
    project: &Project,
    outcome: &BuildOutcome,
    spec: &LaunchSpec,
) -> Result<std::thread::JoinHandle<i32>, String> {
    let bundle = project.manifest.app.id.clone();

    // Keep the screen awake + in never-doze power mode so it doesn't re-lock mid-run (best-effort).
    let _ = hdc().args(["shell", "power-shell", "wakeup"]).status();
    let _ = hdc()
        .args(["shell", "power-shell", "setmode", "602"])
        .status();

    // Install (reinstall over any existing copy), RETRYING: right after boot the bundle-manager
    // service may not accept installs yet, and `hdc install`'s exit code + its "error: failed to
    // execute your command" message are BOTH unreliable on Oniro (the app often installs anyway).
    // Gate on `bm dump -a` actually listing the bundle rather than on the install command's output.
    status(
        "Installing",
        &format!("ohos-arkui ({bundle}) on {}", ohos_target()),
    );
    let mut install_log = String::new();
    let mut installed = false;
    for attempt in 1..=10u32 {
        if let Ok(out) = hdc()
            .args(["install", "-r"])
            .arg(&outcome.artifact)
            .output()
        {
            install_log = combined(&out);
        }
        if bundle_installed(&bundle) {
            installed = true;
            break;
        }
        if attempt < 10 {
            let _ = hdc().args(["shell", "power-shell", "wakeup"]).status();
            std::thread::sleep(Duration::from_secs(3));
        }
    }
    if !installed {
        return Err(format!(
            "hdc install: {bundle} not installed after 10 tries:\n{}",
            install_log.trim()
        ));
    }

    // The `aa start` args: the dayscript engine port/token + locale as `--ps` string parameters (all
    // shell-safe single tokens). EntryAbility.ets applies them to the process env (via the native
    // `setEnv`) before `start()` runs the engine — mirrors Android's intent extras. Built once, since
    // the launch is retried below.
    let mut args: Vec<String> = ["shell", "aa", "start", "-a", "EntryAbility", "-b", &bundle]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for (k, v) in &spec.envs {
        let key = match k.as_str() {
            "DAYSCRIPT_PORT" => "day.dayscript.port".to_string(),
            "DAYSCRIPT_TOKEN" => "day.dayscript.token".to_string(),
            other => format!("day.env.{other}"),
        };
        args.extend(["--ps".to_string(), key, v.clone()]);
    }
    if let Some(locale) = &spec.locale {
        args.extend(["--ps".to_string(), "day.locale".to_string(), locale.clone()]);
    }

    status(
        "Launching",
        &format!("ohos-arkui ({bundle}) on {}", ohos_target()),
    );
    // The emulator boots with the keyguard up; it AUTO-DISMISSES a few seconds after boot but
    // `aa start` is refused until then (Error 10106102) — and there is NO hdc command to force-unlock
    // in developer mode (it is disabled by design). So retry, re-waking the screen between tries, until
    // it stops failing — the keyguard-readiness poll the Eclipse Oniro CI uses. `aa start` also EXITS 0
    // EVEN WHEN REFUSED, so we inspect its output for the failure markers.
    let mut last = String::new();
    let mut launched = false;
    for attempt in 1..=20u32 {
        let out = hdc()
            .args(&args)
            .output()
            .map_err(|e| format!("hdc aa start: {e}"))?;
        let text = combined(&out);
        if out.status.success()
            && !text.contains("Error Code:")
            && !text.to_lowercase().contains("failed to start")
        {
            launched = true;
            break;
        }
        last = text;
        if attempt < 20 {
            let _ = hdc().args(["shell", "power-shell", "wakeup"]).status();
            std::thread::sleep(Duration::from_secs(3));
        }
    }
    if !launched {
        return Err(format!(
            "hdc aa start refused after 20 tries (keyguard/launch):\n{}",
            last.trim()
        ));
    }

    // Attached (interactive) runs stream the device log, best-effort. In script mode the run is
    // detached, so this is skipped — the dayscript runner drives the app over the hdc-forwarded port.
    if spec.attached {
        let name = outcome.target;
        Ok(std::thread::spawn(move || {
            match hdc()
                .args(["shell", "hilog"])
                .stdout(Stdio::piped())
                .spawn()
            {
                Ok(mut child) => {
                    crate::signals::register_child(child.id());
                    if let Some(out) = child.stdout.take() {
                        crate::ops::stream_logs(name, LogStream::Out, out)
                            .join()
                            .ok();
                    }
                    child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(0)
                }
                Err(e) => {
                    emit_log(name, LogStream::Err, &format!("hdc hilog: {e}"));
                    1
                }
            }
        }))
    } else {
        Ok(std::thread::spawn(|| 0))
    }
}
