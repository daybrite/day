// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `day devices` — what a mobile target can be launched onto right now (docs/cli.md).
//!
//! The launch paths have always been able to enumerate simulators, phones and emulators, but only
//! ever privately: `--ios-simulator`, `--android-device` and `--ohos-device` name a device the user
//! already knew about, and the error for getting it wrong was the only listing on offer. This
//! command turns that into an answer — for a person at a terminal, and for the editors that drive
//! the CLI (day-vscode fills its device picker from `--json`).
//!
//! The JSON envelope follows `day metadata`'s contract: versioned and GROW-ONLY, so add keys freely
//! and never repurpose an existing one. Two shapes in it carry weight:
//!
//!   * each device names the **flag** that selects it. iOS alone needs two different flags
//!     depending on whether the pick is a simulator or a physical phone, and keeping that mapping
//!     here rather than in every editor means a new device class costs no editor release.
//!   * a target that cannot be enumerated reports `available: false` with a `note` rather than an
//!     empty list, so a caller can say WHY there is nothing to choose. One missing toolchain must
//!     never blank out the other two — the same reason `day metadata` degrades instead of failing.
//!
//! Enumerating Android starts an `adb` server daemon that outlives the command; that is adb's
//! design, not ours, but it is why this is a command a caller runs deliberately rather than
//! something the CLI does on the side.

use std::process::Command;

use serde_json::{Value, json};

use crate::cli::CliError;
use crate::targets::{Target, TargetKind};

/// The mobile targets this command reports on, in the order a listing shows them.
const MOBILE: [&str; 3] = ["ios-uikit", "android-mdc", "harmony-arkui"];

/// One target's enumeration: what can be launched onto, and what could be started first.
struct Report {
    available: bool,
    note: Option<String>,
    devices: Vec<Value>,
    bootable: Vec<Value>,
}

impl Report {
    fn unavailable(note: impl Into<String>) -> Self {
        Report {
            available: false,
            note: Some(note.into()),
            devices: Vec::new(),
            bootable: Vec::new(),
        }
    }
}

/// `day devices list` — enumerate every mobile target, or just `only` when one was named.
pub fn list(only: Option<&str>, json: bool) -> Result<i32, CliError> {
    if let Some(name) = only
        && !MOBILE.contains(&name)
    {
        return Err(CliError::usage(format!(
            "`day devices` covers the mobile targets ({}); {name} has no device to choose",
            MOBILE.join(", ")
        )));
    }
    let wanted: Vec<&'static Target> = MOBILE
        .iter()
        .filter(|n| only.is_none_or(|o| o == **n))
        .filter_map(|n| crate::targets::find(n))
        .collect();

    // Enumerated concurrently: each target shells out to a different tool and they wait on
    // unrelated things — `devicectl` scanning for paired phones and `hdc` probing a connect key
    // are about a second each, and running them one after another made the editor's device picker
    // wait for the sum rather than the slowest.
    let reports: Vec<(&'static Target, Report)> = std::thread::scope(|scope| {
        let running: Vec<_> = wanted
            .iter()
            .map(|t| (*t, scope.spawn(move || enumerate(t))))
            .collect();
        running
            .into_iter()
            .map(|(t, h)| {
                let report = h
                    .join()
                    .unwrap_or_else(|_| Report::unavailable("enumerating this target panicked"));
                (t, report)
            })
            .collect()
    });

    if json {
        let targets: Vec<Value> = reports
            .iter()
            .map(|(t, r)| {
                json!({
                    "target": t.name,
                    "kind": kind_str(t.kind),
                    "available": r.available,
                    "note": r.note,
                    "devices": r.devices,
                    "bootable": r.bootable,
                })
            })
            .collect();
        let doc = json!({
            "schema": 1,
            "host": { "os": crate::targets::host_os() },
            "targets": targets,
        });
        let s = serde_json::to_string_pretty(&doc).map_err(|e| CliError::failure(e.to_string()))?;
        println!("{s}");
        return Ok(0);
    }

    for (t, r) in &reports {
        println!("{}", t.name);
        if !r.available {
            println!(
                "  unavailable — {}",
                r.note.as_deref().unwrap_or("no reason given")
            );
            continue;
        }
        if r.devices.is_empty() && r.bootable.is_empty() {
            println!("  nothing connected");
        }
        for d in &r.devices {
            println!(
                "  {:<38} {:<10} {}",
                str_of(d, "name"),
                str_of(d, "state"),
                str_of(d, "id")
            );
        }
        // Bootable devices are summarized rather than listed: a stock Xcode carries dozens of
        // shut-down simulators, and burying two connected phones under forty of them helps no
        // one. `--json` still carries every one, which is what a picker needs.
        const SHOWN: usize = 6;
        for b in r.bootable.iter().take(SHOWN) {
            println!(
                "  {:<38} {:<10} {}",
                str_of(b, "name"),
                "shutdown",
                str_of(b, "id")
            );
        }
        if r.bootable.len() > SHOWN {
            println!(
                "  … and {} more not booted (`--json` lists them all)",
                r.bootable.len() - SHOWN
            );
        }
    }
    Ok(0)
}

/// `day devices boot` — start a simulator, an AVD, or the OpenHarmony emulator, so a picker's
/// "nothing is running" is one action away from a device rather than a dead end.
///
/// iOS is the case that forces this: `simctl install` cannot reach a shut-down simulator, so
/// selecting one has always meant leaving the editor to run `xcrun simctl boot` by hand.
/// Put a booted simulator into `portrait` or `landscape` (docs/screenshots.md).
///
/// Driven through Simulator.app's `Device ▸ Orientation` menu, which is the only route there is:
/// `simctl` has no rotate command, and the `SimulatorWindowOrientation` preference turns out to
/// describe the app's WINDOW rather than the device — set before a headless boot it leaves the
/// framebuffer in portrait, which was measured, not assumed. So this needs the machine's GUI
/// session; GitHub's macOS runners have one.
///
/// Clicked, then VERIFIED against the framebuffer's own dimensions and retried, because a single
/// click is not reliable: it lands only once the app is frontmost, and the menu is rebuilt as the
/// device becomes ready. Verified, it took one try every time across repeated switches.
fn set_orientation(udid: &str, orientation: &str) -> Result<(), CliError> {
    let (item, want_landscape) = match orientation.trim().to_ascii_lowercase().as_str() {
        "portrait" => ("Portrait", false),
        "landscape" | "landscape-left" => ("Landscape Left", true),
        "landscape-right" => ("Landscape Right", true),
        other => {
            return Err(CliError::usage(format!(
                "unknown orientation {other:?} — portrait, landscape, landscape-right"
            )));
        }
    };
    crate::ops::status("Orienting", &format!("{orientation} ({item})"));
    let script = format!(
        "tell application \"Simulator\" to activate\n\
         delay 0.5\n\
         tell application \"System Events\" to tell process \"Simulator\" to \
         click menu item \"{item}\" of menu 1 of menu item \"Orientation\" of menu 1 \
         of menu bar item \"Device\" of menu bar 1"
    );
    for attempt in 1..=6 {
        let _ = Command::new("osascript").args(["-e", &script]).output();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        if let Some(landscape) = framebuffer_is_landscape(udid)
            && landscape == want_landscape
        {
            return Ok(());
        }
        if attempt == 6 {
            return Err(CliError::failure(format!(
                "could not put the simulator into {orientation}. This needs a GUI session \
                 (Simulator.app must be able to come to the front); a headless session cannot \
                 rotate a simulator."
            )));
        }
    }
    Ok(())
}

/// Is the device's current framebuffer wider than it is tall? `None` when it cannot be captured.
///
/// The screenshot IS the check: it is what a capture run will produce, so it cannot disagree with
/// what the gallery ends up showing the way a queried setting could.
fn framebuffer_is_landscape(udid: &str) -> Option<bool> {
    let path = std::env::temp_dir().join(format!("day-orient-{udid}.png"));
    let ok = Command::new("xcrun")
        .args(["simctl", "io", udid, "screenshot"])
        .arg(&path)
        .output()
        .ok()?
        .status
        .success();
    if !ok {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    // PNG IHDR: width and height at bytes 16..24.
    if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some(w > h)
}

/// What `day devices boot` was asked for: an explicit id, or a device to resolve.
pub struct BootSpec<'a> {
    pub id: Option<&'a str>,
    pub device: Option<&'a str>,
    pub os: Option<&'a str>,
    pub wait: bool,
    pub orientation: Option<&'a str>,
}

/// Resolve `--device`/`--os` to a simulator UDID.
///
/// `device` matches as a NAME PREFIX and `os` as a MAJOR VERSION taking the newest point release
/// installed, because runner images rotate both: an exact "iPhone 15" on "iOS 26.2" starts failing
/// the day the image moves, and the failure looks like a broken app rather than a stale pin. An
/// unmatched request is an error listing what the machine does have — silently taking some other
/// device would capture the wrong form factor under this profile's name.
fn resolve_simulator(device: &str, os: Option<&str>) -> Result<(String, String, String), CliError> {
    let out = Command::new("xcrun")
        .args(["simctl", "list", "devices", "available", "--json"])
        .output()
        .map_err(|e| CliError::failure(format!("xcrun: {e}")))?;
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| CliError::failure(format!("simctl: {e}")))?;
    // `devices` is keyed by runtime identifier ("com.apple.CoreSimulator.SimRuntime.iOS-26-5").
    let mut best: Option<(String, String, String)> = None;
    let mut have: Vec<String> = Vec::new();
    for (runtime, list) in json["devices"].as_object().into_iter().flatten() {
        let pretty = runtime_label(runtime);
        for d in list.as_array().into_iter().flatten() {
            let (Some(name), Some(udid)) = (d["name"].as_str(), d["udid"].as_str()) else {
                continue;
            };
            have.push(format!("{name} ({pretty})"));
            if !name.starts_with(device) {
                continue;
            }
            if let Some(want) = os
                && !runtime_matches(&pretty, want)
            {
                continue;
            }
            // Newest runtime wins, so `os=iOS 26` lands on the highest 26.x present.
            let better = best
                .as_ref()
                .is_none_or(|(_, _, r)| pretty.as_str() > r.as_str());
            if better {
                best = Some((udid.to_string(), name.to_string(), pretty.clone()));
            }
        }
    }
    best.ok_or_else(|| {
        // List the devices of the OS FAMILY that was asked for. Naming every simulator on the
        // machine buries an iOS request under a page of watchOS and tvOS devices, which makes the
        // list unreadable exactly when someone is reading it to find the right name.
        let family = os
            .and_then(|o| o.split_whitespace().next())
            .unwrap_or("iOS")
            .to_ascii_lowercase();
        let mut listed: Vec<String> = have
            .iter()
            .filter(|d| d.to_ascii_lowercase().contains(&family))
            .cloned()
            .collect();
        if listed.is_empty() {
            listed = have;
        }
        listed.sort();
        listed.dedup();
        let want = match os {
            Some(o) => format!("no simulator named \"{device}…\" on {o} is available"),
            None => format!("no simulator named \"{device}…\" is available"),
        };
        CliError::failure(format!(
            "{want}. This machine has:\n  {}",
            listed.join("\n  ")
        ))
    })
}

/// Does runtime `have` ("iOS 26.5") satisfy `want` ("iOS 26", "iOS 26.5", "26")? Compared by
/// dotted components, so a request with fewer of them is a prefix match on the version.
fn runtime_matches(have: &str, want: &str) -> bool {
    let split = |s: &str| {
        let s = s.trim();
        match s.split_once(char::is_whitespace) {
            Some((name, ver)) => (name.to_ascii_lowercase(), ver.trim().to_string()),
            None => (String::new(), s.to_string()),
        }
    };
    let (hn, hv) = split(have);
    let (wn, wv) = split(want);
    if !wn.is_empty() && wn != hn {
        return false;
    }
    let hp: Vec<&str> = hv.split('.').collect();
    let wp: Vec<&str> = wv.split('.').collect();
    wp.len() <= hp.len() && wp.iter().zip(&hp).all(|(w, h)| w == h)
}

pub fn boot(target: &str, spec: &BootSpec<'_>) -> Result<i32, CliError> {
    let t = crate::targets::find(target)
        .ok_or_else(|| CliError::usage(format!("unknown target {target}")))?;
    match t.kind {
        TargetKind::IosSim => {
            let (udid, name) = match (spec.id, spec.device) {
                (Some(id), _) => (id.to_string(), id.to_string()),
                (None, Some(device)) => {
                    let (udid, name, runtime) = resolve_simulator(device, spec.os)?;
                    crate::ops::status("Device", &format!("{name} ({runtime}) — {udid}"));
                    (udid, name)
                }
                (None, None) => {
                    return Err(CliError::usage(
                        "name a device: an id, or --device \"iPad Pro\"".to_string(),
                    ));
                }
            };
            crate::ops::status("Booting", &format!("simulator {name}"));
            let out = Command::new("xcrun")
                .args(["simctl", "boot", &udid])
                .output()
                .map_err(|e| CliError::failure(format!("xcrun: {e}")))?;
            let err = String::from_utf8_lossy(&out.stderr);
            // Already booted is the success this command means: the caller wanted a device it can
            // install onto, and there is one.
            if !out.status.success() && !err.contains("Unable to boot device in current state") {
                return Err(CliError::failure(format!(
                    "simctl boot failed: {}",
                    err.trim()
                )));
            }
            // Without the UI the simulator boots headless, which is rarely what someone watching
            // for their app to appear wants — and an orientation needs it (see below). Best-effort
            // otherwise: a failure here is not a failed boot.
            let _ = Command::new("open").args(["-a", "Simulator"]).status();
            if spec.wait {
                let st = Command::new("xcrun")
                    .args(["simctl", "bootstatus", &udid, "-b"])
                    .status()
                    .map_err(|e| CliError::failure(format!("xcrun: {e}")))?;
                if !st.success() {
                    return Err(CliError::failure(format!("{name} never finished booting")));
                }
            }
            if let Some(o) = spec.orientation {
                set_orientation(&udid, o)?;
            }
            Ok(0)
        }
        TargetKind::Android => {
            let id = spec.id.or(spec.device).ok_or_else(|| {
                CliError::usage("name an AVD: `day devices boot -p android-mdc <AVD>`".to_string())
            })?;
            let sdk = day_toolchain::android_sdk_dir();
            let exe = if cfg!(windows) {
                "emulator.exe"
            } else {
                "emulator"
            };
            let bin = sdk.join("emulator").join(exe);
            let cmd = if bin.is_file() {
                bin.display().to_string()
            } else {
                exe.to_string()
            };
            // An emulator only forwards the host's keystrokes to the guest when its AVD says it
            // has a hardware keyboard, and `avdmanager create avd` writes `hw.keyboard=no`
            // (Android Studio's wizard writes `yes`, which is why AVDs made in the GUI type fine
            // and AVDs made at a terminal do not). The emulator has no flag to override it —
            // `-use-keycode-forwarding` changes how keys are translated, not whether the guest
            // has a keyboard at all — so the AVD is the only place to fix it. Left alone, the app
            // under development answers every keystroke with Android's on-screen keyboard and no
            // text field ever fills, which reads as the APP swallowing input.
            enable_hw_keyboard(id);
            crate::ops::status("Booting", &format!("emulator {id}"));
            // Detached on purpose: the emulator outlives this command, the way `day launch`
            // expects to find it later. Its own window is where its output belongs.
            Command::new(cmd)
                .args(["-avd", id])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| {
                    CliError::failure(format!(
                        "could not start the Android emulator ({e}) — is the SDK's emulator/ on \
                         PATH, or ANDROID_HOME set?"
                    ))
                })?;
            Ok(0)
        }
        TargetKind::HarmonyOs => {
            // One bundled image rather than a list, so the id is advisory; `emulator_launch` owns
            // the QEMU command line and the port slide.
            crate::ohos::emulator_launch(false)
                .map(|()| 0)
                .map_err(CliError::failure)
        }
        _ => Err(CliError::usage(format!(
            "{target} has no device to boot — `day devices` covers {}",
            MOBILE.join(", ")
        ))),
    }
}

/// Point an AVD at a hardware keyboard, so the keys typed on this machine reach the app.
///
/// A no-op when the AVD already says `hw.keyboard=yes` (every AVD Android Studio made) or when
/// its config cannot be found or rewritten — the emulator still boots either way, and a boot that
/// refused to start over a preferences file would be the worse trade. The value lives in the AVD,
/// so this is a one-time repair per AVD rather than something every boot pays for.
///
/// The AVD's directory comes from its `<name>.ini` (`path=`), which is where the SDK tools record
/// it — an AVD may live outside the AVD home, and `avdmanager --path` puts it wherever it is told.
fn enable_hw_keyboard(avd: &str) {
    let home = std::env::var_os("ANDROID_AVD_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
                .join(".android")
                .join("avd")
        });
    let dir = std::fs::read_to_string(home.join(format!("{avd}.ini")))
        .ok()
        .and_then(|ini| {
            ini.lines().find_map(|l| {
                l.strip_prefix("path=")
                    .map(|p| std::path::PathBuf::from(p.trim()))
            })
        })
        .unwrap_or_else(|| home.join(format!("{avd}.avd")));
    let cfg = dir.join("config.ini");
    let Ok(text) = std::fs::read_to_string(&cfg) else {
        return;
    };
    // The key is written both spaced and unspaced depending on which tool wrote the file, so
    // match on the key rather than on a literal line.
    let mut seen = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|line| match line.split_once('=') {
            Some((k, v)) if k.trim() == "hw.keyboard" => {
                seen = true;
                if v.trim() == "yes" {
                    line.to_string()
                } else {
                    "hw.keyboard=yes".to_string()
                }
            }
            _ => line.to_string(),
        })
        .collect();
    if seen && text.lines().eq(out.iter().map(String::as_str)) {
        return; // already yes — nothing to say and nothing to write
    }
    if !seen {
        out.push("hw.keyboard=yes".to_string());
    }
    let mut body = out.join("\n");
    body.push('\n');
    if std::fs::write(&cfg, body).is_ok() {
        crate::ops::status(
            "Enabling",
            &format!("hardware keyboard for {avd} (hw.keyboard=yes) — typing reaches the app"),
        );
    }
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn kind_str(k: TargetKind) -> &'static str {
    match k {
        TargetKind::Desktop => "desktop",
        TargetKind::IosSim => "iosSim",
        TargetKind::Android => "android",
        TargetKind::HarmonyOs => "harmonyOs",
        TargetKind::Web => "web",
    }
}

fn enumerate(t: &'static Target) -> Report {
    match t.kind {
        TargetKind::IosSim => ios(),
        TargetKind::Android => android(),
        TargetKind::HarmonyOs => ohos(),
        // Unreachable through `list`, which only walks MOBILE — but a total match keeps this
        // honest if the roster grows.
        _ => Report::unavailable("this target has no device to choose"),
    }
}

// ── iOS ──────────────────────────────────────────────────────────────────────────────────────

/// Simulators from `simctl` plus physical devices from `devicectl`, which are different runtimes
/// selected by different flags — the one place a target's devices are not interchangeable.
fn ios() -> Report {
    if !cfg!(target_os = "macos") {
        return Report::unavailable("iOS devices are only reachable from macOS");
    }
    let out = Command::new("xcrun")
        .args(["simctl", "list", "devices", "available", "--json"])
        .output();
    let Ok(out) = out else {
        return Report::unavailable("xcrun not found — install Xcode and its command-line tools");
    };
    if !out.status.success() {
        return Report::unavailable(format!(
            "`xcrun simctl list` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let parsed: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| json!({}));
    let mut devices = Vec::new();
    let mut bootable = Vec::new();
    if let Some(map) = parsed.get("devices").and_then(Value::as_object) {
        for (runtime_key, list) in map {
            // iOS runtimes only. `simctl list` reports every installed platform, and an
            // ios-uikit app cannot be installed onto an Apple Watch — offering one is offering a
            // guaranteed failure.
            if !runtime_key.contains("SimRuntime.iOS-") {
                continue;
            }
            let runtime = runtime_label(runtime_key);
            for d in list.as_array().unwrap_or(&Vec::new()) {
                let name = str_of(d, "name");
                let udid = str_of(d, "udid");
                if name.is_empty() || udid.is_empty() {
                    continue;
                }
                // Booted-only is a real constraint, not an oversight: `simctl install` cannot
                // reach a shut-down simulator, so the rest go to `bootable` for `devices boot`.
                if str_of(d, "state") == "Booted" {
                    devices.push(json!({
                        "id": udid, "name": name, "kind": "simulator", "state": "booted",
                        "runtime": runtime, "flag": "--ios-simulator",
                    }));
                } else {
                    bootable.push(json!({ "id": udid, "name": name, "runtime": runtime }));
                }
            }
        }
    }
    for (udid, name) in crate::mobile::physical_ios_devices() {
        devices.push(json!({
            "id": udid, "name": name, "kind": "device", "state": "connected",
            "flag": "--ios-device",
        }));
    }
    devices.sort_by_key(|d| str_of(d, "name"));
    bootable.sort_by_key(|d| str_of(d, "name"));
    Report {
        available: true,
        note: None,
        devices,
        bootable,
    }
}

/// `com.apple.CoreSimulator.SimRuntime.iOS-18-2` → `iOS 18.2`, and anything unrecognized verbatim.
fn runtime_label(key: &str) -> String {
    let Some(tail) = key.rsplit('.').next() else {
        return key.to_string();
    };
    match tail.split_once('-') {
        Some((os, version)) => format!("{os} {}", version.replace('-', ".")),
        None => tail.to_string(),
    }
}

// ── Android ──────────────────────────────────────────────────────────────────────────────────

fn android() -> Report {
    let out = Command::new("adb").arg("devices").output();
    let Ok(out) = out else {
        return Report::unavailable(
            "adb not found — install the Android SDK platform-tools and put them on PATH",
        );
    };
    if !out.status.success() {
        return Report::unavailable(format!(
            "`adb devices` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    // Straight from `adb devices` rather than through `mobile::android_devices`, which drops
    // everything that is not in the `device` state — an unauthorized phone is exactly what a
    // listing must show, because "it is plugged in but you have not tapped Allow" is the answer.
    let mut devices = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines().skip(1) {
        let mut it = line.split_whitespace();
        let (Some(serial), Some(state)) = (it.next(), it.next()) else {
            continue;
        };
        let ready = state == "device";
        devices.push(json!({
            "id": serial,
            "name": if serial.starts_with("emulator-") { format!("Emulator ({serial})") } else { serial.to_string() },
            "kind": if serial.starts_with("emulator-") { "emulator" } else { "device" },
            "state": if ready { "connected" } else { state },
            "arch": ready.then(|| device_abi(serial)),
            "flag": "--android-device",
        }));
    }
    Report {
        available: true,
        note: None,
        devices,
        bootable: avds(),
    }
}

fn device_abi(serial: &str) -> String {
    Command::new("adb")
        .args(["-s", serial, "shell", "getprop", "ro.product.cpu.abi"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
}

/// Defined but not running AVDs — what `day devices boot` can start.
fn avds() -> Vec<Value> {
    let sdk = day_toolchain::android_sdk_dir();
    let exe = if cfg!(windows) {
        "emulator.exe"
    } else {
        "emulator"
    };
    let bin = sdk.join("emulator").join(exe);
    let cmd = if bin.is_file() {
        bin.display().to_string()
    } else {
        exe.to_string()
    };
    Command::new(cmd)
        .arg("-list-avds")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.contains(' '))
                .map(|name| json!({ "id": name, "name": name }))
                .collect()
        })
        .unwrap_or_default()
}

// ── OpenHarmony ──────────────────────────────────────────────────────────────────────────────

fn ohos() -> Report {
    if !crate::ohos::hdc_available() {
        return Report::unavailable(
            "hdc not found — install the OpenHarmony command-line tools, or set OHOS_NDK_HOME so \
             its sibling toolchains/ dir is found",
        );
    }
    let devices = crate::ohos::ohos_devices()
        .into_iter()
        .map(|d| {
            json!({
                "id": d.key,
                "name": d.key,
                "kind": if d.key.contains(':') { "emulator" } else { "device" },
                "state": "connected",
                "arch": d.abi,
                "flag": "--ohos-device",
            })
        })
        .collect();
    Report {
        available: true,
        note: None,
        devices,
        // The bundled Oniro emulator is started by `day ohos emulator launch` rather than picked
        // from a list of images, so there is nothing to enumerate here yet.
        bootable: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// The runtime key is the only part of simctl's output this module reshapes, and a picker
    /// shows it beside every simulator name.
    #[test]
    fn simulator_runtimes_read_as_versions() {
        let cases = [
            ("com.apple.CoreSimulator.SimRuntime.iOS-18-2", "iOS 18.2"),
            (
                "com.apple.CoreSimulator.SimRuntime.watchOS-11-0",
                "watchOS 11.0",
            ),
            (
                "com.apple.CoreSimulator.SimRuntime.iOS-26-0-1",
                "iOS 26.0.1",
            ),
        ];
        for (key, want) in cases {
            assert_eq!(runtime_label(key), want, "for {key}");
        }
        // An unrecognized shape is passed through rather than mangled into something wrong.
        assert_eq!(runtime_label("something-else"), "something else");
    }

    /// Every device a listing reports has to name the flag that selects it — that mapping is the
    /// contract editors depend on, and iOS is where it actually differs per device.
    #[test]
    fn kinds_map_to_the_documented_strings() {
        let mut seen = BTreeMap::new();
        for name in MOBILE {
            let t = crate::targets::find(name).expect("mobile target in the catalog");
            seen.insert(name, kind_str(t.kind));
        }
        assert_eq!(seen["ios-uikit"], "iosSim");
        assert_eq!(seen["android-mdc"], "android");
        assert_eq!(seen["harmony-arkui"], "harmonyOs");
    }
}
