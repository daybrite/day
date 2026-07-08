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

/// Bring up the Oniro/OpenHarmony QEMU emulator as a native window (the OHOS analogue of
/// `skip android emulator launch`). No VNC, no Screen Sharing: on macOS the QEMU `cocoa` backend
/// opens a real window; `--headless` uses no display (hdc-only, for CI). Self-contained — it builds
/// the QEMU command itself, so it doesn't depend on the emulator distribution's shell launcher.
///
/// The image directory is `DAY_OHOS_EMULATOR` (a dir holding `bzImage`, `ramdisk.img`, `system.img`,
/// `vendor.img`, `updater.img`, `userdata.img`) or the default `~/ohos/emulator/images`. The host
/// hdc port comes from `DAY_OHOS_TARGET` (default `127.0.0.1:55555`), forwarded to the guest's 55555.
pub fn emulator_launch(headless: bool) -> Result<(), String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let images = std::env::var("DAY_OHOS_EMULATOR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(&home).join("ohos/emulator/images"));
    for f in [
        "bzImage",
        "ramdisk.img",
        "system.img",
        "vendor.img",
        "userdata.img",
    ] {
        if !images.join(f).exists() {
            return Err(format!(
                "OpenHarmony emulator images not found at {} (missing {f}). Download the Oniro \
                 emulator and set DAY_OHOS_EMULATOR to its image dir (see docs/harmonyos.md).",
                images.display()
            ));
        }
    }
    let qemu = "qemu-system-x86_64";
    if Command::new(qemu).arg("--version").output().is_err() {
        return Err(format!(
            "{qemu} not found — install QEMU (`brew install qemu`) to run the OpenHarmony emulator."
        ));
    }
    // Host hdc port from the connect key (guest hdc always listens on 55555). Kill any stale hdc
    // server first so it can't hold the host port before QEMU binds the forward.
    let target = ohos_target();
    let host_port = target.rsplit(':').next().unwrap_or("55555").to_string();
    let _ = Command::new(hdc_bin()).arg("kill").output();

    // The display backend: a native window locally (cocoa on macOS), none when headless.
    let display: &[&str] = if headless {
        &["-display", "none"]
    } else if cfg!(target_os = "macos") {
        &["-display", "cocoa"]
    } else {
        &["-display", "gtk"]
    };

    // Kernel cmdline + block devices are fixed for the Oniro x86_general image.
    let append = "ip=dhcp loglevel=4 console=ttyS0,115200 init=init root=/dev/ram0 rw \
                  ohos.boot.hardware=x86_general \
                  ohos.required_mount.system=/dev/block/vdb@/usr@ext4@ro,barrier=1@wait,required \
                  ohos.required_mount.vendor=/dev/block/vdc@/vendor@ext4@ro,barrier=1@wait,required \
                  ohos.required_mount.misc=/dev/block/vda@/misc@none@none=@wait,required";
    let hostfwd = format!("user,id=net0,hostfwd=tcp:127.0.0.1:{host_port}-:55555");
    let gpu = "virtio-gpu-pci,xres=360,yres=720,max_outputs=1,addr=08.0";

    let mut cmd = Command::new(qemu);
    cmd.current_dir(&images)
        .args([
            "-machine", "q35", "-smp", "6", "-m", "4096M", "-boot", "c", "-vga", "none",
        ])
        .args(["-device", gpu])
        .args(display)
        .args(["-rtc", "base=utc,clock=host", "-device", "es1370"])
        .args(["-initrd", "ramdisk.img", "-kernel", "bzImage"])
        .args([
            "-drive",
            "if=none,file=updater.img,format=raw,id=updater,index=0",
        ])
        .args(["-device", "virtio-blk-pci,drive=updater"])
        .args([
            "-drive",
            "if=none,file=system.img,format=raw,id=system,index=1",
        ])
        .args(["-device", "virtio-blk-pci,drive=system"])
        .args([
            "-drive",
            "if=none,file=vendor.img,format=raw,id=vendor,index=2",
        ])
        .args(["-device", "virtio-blk-pci,drive=vendor"])
        .args([
            "-drive",
            "if=none,file=userdata.img,format=raw,id=userdata,index=3",
        ])
        .args(["-device", "virtio-blk-pci,drive=userdata"])
        .args(["-serial", "none", "-append", append])
        .args(["-accel", "tcg,thread=multi", "-cpu", "max"])
        .args(["-netdev", &hostfwd, "-device", "virtio-net-pci,netdev=net0"]);
    status(
        "Emulator",
        &format!(
            "OpenHarmony ({}) — {}",
            images.display(),
            if headless { "headless" } else { "windowed" }
        ),
    );
    let mut child = cmd.spawn().map_err(|e| format!("qemu: {e}"))?;
    crate::signals::register_child(child.id());

    // Wait for hdc to see the target booted (TCG boot is slow), like `skip android emulator launch`.
    status("Emulator", "waiting for boot (TCG is slow — up to ~4 min)…");
    for _ in 0..48 {
        if let Some(code) = child.try_wait().ok().flatten() {
            return Err(format!("qemu exited early ({code})"));
        }
        let _ = Command::new(hdc_bin()).args(["tconn", &target]).output();
        let booted = Command::new(hdc_bin())
            .args([
                "-t",
                &target,
                "shell",
                "param",
                "get",
                "bootevent.boot.completed",
            ])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
            .unwrap_or(false);
        if booted {
            status("Emulator", &format!("booted — hdc target {target}"));
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(5));
    }
    Err("emulator did not report boot within the timeout (still starting?)".into())
}

/// The hdc target key (`-t`) for the emulator/device. Oniro's QEMU emulator is a networked target
/// reachable at the emulator-action connect-key `127.0.0.1:55555`; override via `DAY_OHOS_TARGET`
/// (a real device's connect key, or a different port).
pub fn ohos_target() -> String {
    std::env::var("DAY_OHOS_TARGET").unwrap_or_else(|_| "127.0.0.1:55555".into())
}

/// The `hdc` executable: on PATH if present, else resolved from the SDK install's sibling
/// `toolchains/` dir (the public SDK ships it there, next to the `native` NDK) — so
/// `day launch -p ohos-arkui` works from GUI-launched editors whose environment has neither the
/// variable nor the PATH entry.
fn hdc_bin() -> &'static str {
    static HDC: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HDC.get_or_init(|| {
        let on_path = std::env::var("PATH")
            .is_ok_and(|path| std::env::split_paths(&path).any(|d| d.join("hdc").is_file()));
        if on_path {
            return "hdc".into();
        }
        if let Ok(ndk) = find_ohos_ndk() {
            let cand = Path::new(&ndk).parent().map(|p| p.join("toolchains/hdc"));
            if let Some(c) = cand
                && c.is_file()
            {
                return c.to_string_lossy().into_owned();
            }
        }
        "hdc".into()
    })
}

/// A fresh `hdc` command targeting the default connect key (`DAY_OHOS_TARGET`).
pub fn hdc() -> Command {
    hdc_for(&ohos_target())
}

/// A fresh `hdc` command pinned to connect key `key` (`-t <key>`), for multi-device install/launch.
fn hdc_for(key: &str) -> Command {
    let mut c = Command::new(hdc_bin());
    c.args(["-t", key]);
    c
}

/// A connected OpenHarmony target: its `hdc` connect key + the arch it runs (queried via `uname -m`,
/// mapped to the Rust triple + hap ABI dir). An emulator is x86_64; a device is arm64 — we ask.
pub(crate) struct OhosDevice {
    pub key: String,
    pub triple: &'static str,
    pub abi: &'static str,
}

/// arch string from `uname -m` → (Rust triple, hap ABI dir).
fn arch_triple(uname: &str) -> Option<(&'static str, &'static str)> {
    match uname.trim() {
        "aarch64" | "arm64" => Some(("aarch64-unknown-linux-ohos", "arm64-v8a")),
        "x86_64" | "amd64" => Some(("x86_64-unknown-linux-ohos", "x86_64")),
        _ => None,
    }
}

/// Connected OHOS targets. `hdc list targets` lists USB/attached keys; the networked emulator is
/// reached via `DAY_OHOS_TARGET`, so that key is always included (after a best-effort `tconn`). Each
/// target's arch is queried with `uname -m`. Unreachable targets are dropped.
pub(crate) fn ohos_devices() -> Vec<OhosDevice> {
    let mut keys: Vec<String> = Vec::new();
    // The default/networked target: connect + include it.
    let default_key = ohos_target();
    let _ = Command::new(hdc_bin())
        .args(["tconn", &default_key])
        .output();
    keys.push(default_key);
    // Any additional attached targets.
    if let Ok(out) = Command::new(hdc_bin()).args(["list", "targets"]).output() {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let k = line.trim();
            if !k.is_empty() && !k.starts_with('[') && !keys.iter().any(|e| e == k) {
                keys.push(k.to_string());
            }
        }
    }
    keys.into_iter()
        .filter_map(|key| {
            let uname = Command::new(hdc_bin())
                .args(["-t", &key, "shell", "uname", "-m"])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            let (triple, abi) = arch_triple(&uname)?;
            Some(OhosDevice { key, triple, abi })
        })
        .collect()
}

/// The (triple, abi) set to build for: the distinct arches of the connected targets, or — with none
/// reachable — the `DAY_OHOS_ARCH` override / emulator default, so `day build` still produces a hap.
fn ohos_build_arches() -> Vec<(&'static str, &'static str)> {
    let mut arches: Vec<(&'static str, &'static str)> = ohos_devices()
        .into_iter()
        .map(|d| (d.triple, d.abi))
        .collect();
    arches.sort();
    arches.dedup();
    if arches.is_empty() {
        arches.push(match std::env::var("DAY_OHOS_ARCH").ok().as_deref() {
            Some("device") | Some("arm64") | Some("arm64-v8a") => {
                ("aarch64-unknown-linux-ohos", "arm64-v8a")
            }
            _ => ("x86_64-unknown-linux-ohos", "x86_64"),
        });
    }
    arches
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

    // 1) Cross-compile the app to a cdylib for EACH connected target's arch (an emulator is x86_64,
    //    a device arm64 — the hap carries both so it installs on either), staging each as
    //    entry/libs/<abi>/libentry.so — the .so the ArkTS host imports (its NAPI module is "entry").
    //    Uses the OHOS NDK cross-linker (OHOS_NDK_HOME) + a rustup toolchain (Homebrew rustc ships no
    //    OHOS std) and `feature_selection("arkui")` (the arkui toolkit feature + every standalone
    //    piece's `<pkg>/arkui` renderer feature, Tier A.2), exactly like the android/iOS legs.
    let ndk = find_ohos_ndk()?;
    let (cargo, bin) = rustup_cargo()?;
    let name = project.manifest.app.name.clone();
    for (triple, abi) in ohos_build_arches() {
        let target_dir = project
            .root
            .join("build/day/cargo/ohos-arkui")
            .join(abi)
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
        run_logged(&mut cmd, &format!("cargo (ohos {abi})"))?;
        // The cdylib is `lib<[lib].name>.so` (libentry.so for a crate whose `[lib] name = "entry"`,
        // else lib<crate>.so) — find the single produced .so and stage it AS libentry.so.
        let out_dir = target_dir.join(triple).join(profile);
        let so = std::fs::read_dir(&out_dir)
            .map_err(|e| format!("reading {}: {e}", out_dir.display()))?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().and_then(|x| x.to_str()) == Some("so"))
            .ok_or_else(|| format!("no cdylib .so produced in {}", out_dir.display()))?;
        let libs = harmony.join("entry/libs").join(abi);
        std::fs::create_dir_all(&libs).map_err(|e| format!("mkdir {}: {e}", libs.display()))?;
        std::fs::copy(&so, libs.join("libentry.so"))
            .map_err(|e| format!("stage libentry.so: {e}"))?;
        // libentry.so links the NDK's SHARED libc++ (the day-arkui-sys C++ shim), which OpenHarmony
        // does NOT provide on-device for apps — an unbundled hap dies at load with MUSL-LDSO's
        // "Error loading shared library libc++_shared.so". Stage it next to libentry.so so hvigor
        // packs it into the hap (the exact analogue of the Android jniLibs bundling). The NDK's
        // per-arch lib dir uses the CLANG triple (`x86_64-linux-ohos`), not the Rust triple — drop
        // the `unknown-` vendor field.
        let clang_triple = triple.replace("unknown-", "");
        let libcxx = PathBuf::from(&ndk)
            .join("llvm/lib")
            .join(&clang_triple)
            .join("libc++_shared.so");
        if libcxx.exists() {
            std::fs::copy(&libcxx, libs.join("libc++_shared.so"))
                .map_err(|e| format!("stage libc++_shared.so: {e}"))?;
        } else {
            status(
                "Warning",
                &format!(
                    "libc++_shared.so not found at {} — the hap may fail to load",
                    libcxx.display()
                ),
            );
        }
    }

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
fn bundle_installed_on(bundle: &str, key: &str) -> bool {
    hdc_for(key)
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
    let devices = ohos_devices();
    if devices.is_empty() {
        return Err(format!(
            "no OpenHarmony target reachable (hdc). Boot an emulator (`day ohos emulator launch`) \
             or attach a device; the default connect key is {}.",
            ohos_target()
        ));
    }
    // The dayscript runner drives ONE target over the hdc-forwarded port — the default key — so a
    // scripted run stays deterministic even with several targets attached.
    let multi = devices.len() > 1;
    let mut log_threads = Vec::new();
    for dev in &devices {
        install_and_start(&bundle, &dev.key, outcome, spec)?;
        if spec.attached {
            let label = if multi {
                format!("{}:{}", outcome.target, dev.key)
            } else {
                outcome.target.to_string()
            };
            let key = dev.key.clone();
            log_threads.push(std::thread::spawn(move || stream_hilog(&key, &label)));
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

/// Install (reinstall) + `aa start` the bundle on the target `key`, with the Oniro retry dances.
fn install_and_start(
    bundle: &str,
    key: &str,
    outcome: &BuildOutcome,
    spec: &LaunchSpec,
) -> Result<(), String> {
    // Keep the screen awake + in never-doze power mode so it doesn't re-lock mid-run (best-effort).
    let _ = hdc_for(key)
        .args(["shell", "power-shell", "wakeup"])
        .status();
    let _ = hdc_for(key)
        .args(["shell", "power-shell", "setmode", "602"])
        .status();

    // Install (reinstall over any existing copy), RETRYING: right after boot the bundle-manager
    // service may not accept installs yet, and `hdc install`'s exit code + its "error: failed to
    // execute your command" message are BOTH unreliable on Oniro (the app often installs anyway).
    // Gate on `bm dump -a` actually listing the bundle rather than on the install command's output.
    status("Installing", &format!("ohos-arkui ({bundle}) on {key}"));
    let mut install_log = String::new();
    let mut installed = false;
    for attempt in 1..=10u32 {
        if let Ok(out) = hdc_for(key)
            .args(["install", "-r"])
            .arg(&outcome.artifact)
            .output()
        {
            install_log = combined(&out);
        }
        if bundle_installed_on(bundle, key) {
            installed = true;
            break;
        }
        if attempt < 10 {
            let _ = hdc_for(key)
                .args(["shell", "power-shell", "wakeup"])
                .status();
            std::thread::sleep(Duration::from_secs(3));
        }
    }
    if !installed {
        return Err(format!(
            "hdc install: {bundle} not installed on {key} after 10 tries:\n{}",
            install_log.trim()
        ));
    }

    // The `aa start` args: the dayscript engine port/token + locale as `--ps` string parameters (all
    // shell-safe single tokens). EntryAbility.ets applies them to the process env (via the native
    // `setEnv`) before `start()` runs the engine — mirrors Android's intent extras.
    let mut args: Vec<String> = ["shell", "aa", "start", "-a", "EntryAbility", "-b", bundle]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for (k, v) in &spec.envs {
        let param = match k.as_str() {
            "DAYSCRIPT_PORT" => "day.dayscript.port".to_string(),
            "DAYSCRIPT_TOKEN" => "day.dayscript.token".to_string(),
            other => format!("day.env.{other}"),
        };
        args.extend(["--ps".to_string(), param, v.clone()]);
    }
    if let Some(locale) = &spec.locale {
        args.extend(["--ps".to_string(), "day.locale".to_string(), locale.clone()]);
    }

    status("Launching", &format!("ohos-arkui ({bundle}) on {key}"));
    // The emulator boots with the keyguard up; it AUTO-DISMISSES a few seconds after boot but
    // `aa start` is refused until then (Error 10106102) — and there is NO hdc command to force-unlock
    // in developer mode (it is disabled by design). So retry, re-waking the screen between tries, until
    // it stops failing — the keyguard-readiness poll the Eclipse Oniro CI uses. `aa start` also EXITS 0
    // EVEN WHEN REFUSED, so we inspect its output for the failure markers.
    let mut last = String::new();
    for attempt in 1..=20u32 {
        let out = hdc_for(key)
            .args(&args)
            .output()
            .map_err(|e| format!("hdc aa start: {e}"))?;
        let text = combined(&out);
        if out.status.success()
            && !text.contains("Error Code:")
            && !text.to_lowercase().contains("failed to start")
        {
            return Ok(());
        }
        last = text;
        if attempt < 20 {
            let _ = hdc_for(key)
                .args(["shell", "power-shell", "wakeup"])
                .status();
            std::thread::sleep(Duration::from_secs(3));
        }
    }
    Err(format!(
        "hdc aa start refused on {key} after 20 tries (keyguard/launch):\n{}",
        last.trim()
    ))
}

/// Stream one target's hilog into the day log with `label` (best-effort). Returns its exit code.
fn stream_hilog(key: &str, label: &str) -> i32 {
    match hdc_for(key)
        .args(["shell", "hilog"])
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(mut child) => {
            crate::signals::register_child(child.id());
            if let Some(out) = child.stdout.take() {
                for line in
                    std::io::BufRead::lines(std::io::BufReader::new(out)).map_while(Result::ok)
                {
                    emit_log(label, LogStream::Out, &line);
                }
            }
            child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(0)
        }
        Err(e) => {
            emit_log(label, LogStream::Err, &format!("hdc hilog: {e}"));
            1
        }
    }
}
