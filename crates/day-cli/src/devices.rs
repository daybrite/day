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
/// `devicectl device orientation set` is the actuator: absolute rather than a relative rotate,
/// and it works with no Simulator.app and no GUI session — which is the whole point, because the
/// route it replaced (clicking Simulator.app's `Device ▸ Orientation` through AppleScript) failed
/// outright on a CI runner. `simctl` has never had a rotate command, and the
/// `SimulatorWindowOrientation` preference describes the app's WINDOW rather than the device: set
/// before a headless boot it leaves the framebuffer in portrait. Both were measured.
///
/// Read back with `orientation get` rather than by measuring a screenshot, which is a trap worth
/// recording: an iPhone's SPRINGBOARD does not rotate, so a correctly-turned iPhone still
/// screenshots portrait at the home screen. Checking pixels there reports a working rotation as
/// broken. `deviceOrientationNonFlat` is the device's own answer and is exact, so it also tells
/// `portrait` from `portraitUpsideDown`, which an aspect-ratio check never could.
fn set_orientation(udid: &str, orientation: &str) -> Result<(), CliError> {
    let want = match orientation.trim().to_ascii_lowercase().as_str() {
        "portrait" => "portrait",
        "portrait-upside-down" => "portraitUpsideDown",
        "landscape" | "landscape-left" => "landscapeLeft",
        "landscape-right" => "landscapeRight",
        other => {
            return Err(CliError::usage(format!(
                "unknown orientation {other:?} — portrait, landscape, landscape-right, \
                 portrait-upside-down"
            )));
        }
    };
    // Already there? Then nothing needs actuating. This is the common case for `portrait` — a
    // simulator boots that way — and skipping it keeps the usual CI job off devicectl's write
    // path entirely.
    if device_orientation(udid).as_deref() == Some(want) {
        return Ok(());
    }
    crate::ops::status("Orienting", orientation);
    match rotate(udid, want, orientation) {
        Ok(()) => Ok(()),
        // A failed rotation TO portrait leaves the device where it already was: portrait is where
        // a simulator boots, so the requested state holds and the captures are honestly labeled.
        // Warn and carry on rather than failing a build over a rotation that changes nothing.
        //
        // Landscape gets the opposite treatment on purpose — there the device really is still in
        // portrait, and publishing those captures under a landscape device name would be a lie
        // that nobody downstream could detect.
        Err(e) if want == "portrait" => {
            let why = e.to_string();
            let why = why.lines().next().unwrap_or_default().trim();
            crate::ops::status(
                "Warning",
                &format!(
                    "could not turn the simulator to portrait ({why}) — but that is where it \
                     boots, so continuing"
                ),
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Turn the device and confirm it turned.
fn rotate(udid: &str, want: &str, orientation: &str) -> Result<(), CliError> {
    let out = Command::new("xcrun")
        .args([
            "devicectl",
            "device",
            "orientation",
            "set",
            "--device",
            udid,
            want,
            "--quiet",
        ])
        .output()
        .map_err(|e| {
            CliError::failure(format!(
                "could not run `devicectl` ({e}) — setting a simulator's orientation needs it, \
                 and it comes with Xcode"
            ))
        })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(CliError::failure(format!(
            "devicectl could not set the orientation to {orientation}: {}\n{}",
            err.trim(),
            CORE_DEVICE_FLOOR
        )));
    }
    // Confirm the device really turned. A CoreDevice that does not drive simulators can still
    // exit zero, so the exit code alone does not distinguish "done" from "ignored".
    for attempt in 1..=5 {
        std::thread::sleep(std::time::Duration::from_millis(600));
        if device_orientation(udid).as_deref() == Some(want) {
            return Ok(());
        }
        if attempt == 5 {
            return Err(CliError::failure(format!(
                "devicectl accepted `orientation set {want}` but the device reports {:?}.\n{}",
                device_orientation(udid).unwrap_or_else(|| "nothing".into()),
                CORE_DEVICE_FLOOR
            )));
        }
    }
    Ok(())
}

/// What actually gates simulator orientation, spelled out wherever it is reported.
///
/// The XCODE, not the macOS — and the reason that is worth writing down is that the two look
/// interchangeable from one machine. Xcode's `devicectl` is a short shell wrapper that `exec`s
/// `/Library/Developer/PrivateFrameworks/CoreDevice.framework/…/devicectl`, which is easy to read
/// as "a system framework, so macOS owns it". It is not: Xcode INSTALLS that framework. The
/// wrapper compares the installed CoreDevice against the version its own Xcode ships and runs
/// `xcodebuild -runFirstLaunch` when they differ, so whichever Xcode last did its first launch
/// decides what every `devicectl` on the machine can do.
///
/// Xcode 26.6 ships CoreDevice 518.33, which cannot see a simulator at all: it answers
/// `orientation set` with "The specified device was not found" for one that is booted and visible
/// to simctl, and rejects `--omit-deprecated-fields-in-json` outright. Xcode 27 ships 642.15,
/// which turns them. Two machines on the SAME macOS therefore differ completely.
const CORE_DEVICE_FLOOR: &str = "Turning a simulator needs Xcode 27 or newer — its CoreDevice, \
     not the macOS version: Xcode installs the framework its `devicectl` execs, so an Xcode 26.6 \
     machine cannot see simulators at all. Select a newer Xcode with `xcode-select -s`, or drop \
     `--orientation`.";

/// Whether devicectl can see this simulator at all, which is the precondition for turning it.
///
/// Asked BEFORE booting, because booting costs minutes on a CI runner and the answer does not
/// depend on it: shut-down simulators appear in the listing too (72 of them on the host this was
/// measured on). A run that cannot rotate should learn it in a second, not after a three-minute
/// boot it is about to waste.
fn devicectl_sees(udid: &str) -> bool {
    devicectl_simulators().is_ok_and(|list| {
        sims_from_devicectl(&list)
            .iter()
            .any(|(_, id, _)| id == udid)
    })
}

/// The device's own current orientation (`portrait`, `landscapeLeft`, …), ignoring face-up and
/// face-down. `None` when devicectl cannot answer.
fn device_orientation(udid: &str) -> Option<String> {
    let out = Command::new("xcrun")
        .args([
            "devicectl",
            "device",
            "orientation",
            "get",
            "--device",
            udid,
            "-j",
            "-",
            "--quiet",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let doc: Value = serde_json::from_slice(&out.stdout).ok()?;
    doc.get("result")?
        .get("deviceOrientationNonFlat")?
        .as_str()
        .map(str::to_string)
}

/// What `day devices boot` was asked for: an explicit id, or a device to resolve.
pub struct BootSpec<'a> {
    pub id: Option<&'a str>,
    pub device: Option<&'a str>,
    pub os: Option<&'a str>,
    pub wait: bool,
    pub orientation: Option<&'a str>,
    /// Run the Android emulator with no window (CI). Ignored by the other targets, which have no
    /// equivalent: a simulator is already headless and the OpenHarmony emulator has no such flag.
    pub headless: bool,
}

/// What `day devices setup` builds: one AVD, described the way a device profile describes it.
pub struct SetupSpec<'a> {
    /// AVD name. Defaults to one derived from the device and API level.
    pub name: Option<&'a str>,
    /// A device profile id from `avdmanager list device` (`pixel_tablet`, `pixel_5`).
    pub device: &'a str,
    /// API level, spelled `36`, `API 36` or `android-36`.
    pub os: &'a str,
    /// ABI: `x86_64` on a CI runner, `arm64-v8a` on Apple Silicon. Defaults to the host's.
    pub arch: Option<&'a str>,
    /// System-image tag — `google_apis` by default, which is what an app needs and what every
    /// API level publishes for both ABIs.
    pub tag: Option<&'a str>,
    pub orientation: Option<&'a str>,
}

/// Resolve `--device`/`--os` to a simulator UDID.
///
/// `device` matches as a NAME PREFIX and `os` as a MAJOR VERSION taking the newest point release
/// installed, because runner images rotate both: an exact "iPhone 15" on "iOS 26.2" starts failing
/// the day the image moves, and the failure looks like a broken app rather than a stale pin. An
/// unmatched request is an error listing what the machine does have — silently taking some other
/// device would capture the wrong form factor under this profile's name.
/// Every simulator devicectl knows about, or why it could not say.
///
/// Fails loudly on an empty or unparseable answer rather than treating it as "no devices": those
/// are the shapes a broken CoreDevice produces, and reading them as an empty machine would turn a
/// tooling fault into "no simulator matched", which sends the reader hunting for the wrong thing.
fn devicectl_simulators() -> Result<Vec<Value>, String> {
    let out = Command::new("xcrun")
        .args(["devicectl", "list", "devices", "-j", "-"])
        .output()
        .map_err(|e| format!("could not be run: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        let code = out
            .status
            .code()
            .map_or_else(|| "abnormally".to_string(), |c| c.to_string());
        return Err(if err.is_empty() {
            format!("exited {code}")
        } else {
            format!("exited {code}: {err}")
        });
    }
    if out.stdout.iter().all(u8::is_ascii_whitespace) {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        return Err(if err.is_empty() {
            "printed nothing".to_string()
        } else {
            format!("printed no JSON: {err}")
        });
    }
    let json: Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("printed bad JSON: {e}"))?;
    Ok(json["result"]["devices"]
        .as_array()
        .cloned()
        .unwrap_or_default())
}

/// One simulator, as the matcher needs it: display name, UDID, and a runtime spelled "iOS 26.5".
type Sim = (String, String, String);

/// Simulators from devicectl, falling back to simctl when devicectl cannot answer.
///
/// devicectl is preferred for its schema — the OS version arrives as a plain "26.5", and
/// `hardware.reality` separates a simulator from a physical device, where simctl keys everything
/// by runtime IDENTIFIER ("com.apple.CoreSimulator.SimRuntime.iOS-26-5") that has to be unpicked
/// back into a version. Preferred, not required: it talks to CoreDevice, and a CI runner has been
/// seen to answer nothing at all, which is not worth failing a build over when simctl needs no
/// daemon and has enumerated simulators for far longer.
fn simulators() -> Result<Vec<Sim>, CliError> {
    let fallback = |why: String| {
        // Loud, not silent. A runner where devicectl is broken says so once in the log, instead of
        // a fallback quietly working and the fault only surfacing later at `devicectl orientation`,
        // which has no second source.
        crate::ops::status(
            "Warning",
            &format!("devicectl {why} — using simctl to find simulators instead"),
        );
        simctl_simulators()
    };
    let list = match devicectl_simulators() {
        Ok(list) => list,
        Err(why) => return fallback(why),
    };
    let sims = sims_from_devicectl(&list);
    // A parseable answer naming no iOS simulator is the other shape a half-working CoreDevice
    // produces. Treating it as "this machine has none" would report a missing device when the
    // real fault is the tool, so simctl gets the same chance to answer.
    if sims.is_empty() {
        return fallback("listed no iOS simulators".to_string());
    }
    Ok(sims)
}

/// The iOS simulators in a `devicectl list devices` payload.
fn sims_from_devicectl(list: &[Value]) -> Vec<Sim> {
    let mut sims = Vec::new();
    for d in list {
        let props = &d["properties"];
        let hw = &props["hardware"];
        // Simulators only: `--ios-simulator` is what this resolves for, and a physical device
        // reached the same way would be installed onto by accident.
        if hw["reality"].as_str() != Some("simulated") || hw["platform"].as_str() != Some("iOS") {
            continue;
        }
        let (Some(name), Some(udid)) = (props["state"]["name"].as_str(), d["identifier"].as_str())
        else {
            continue;
        };
        let version = props["software"]["osVersionNumber"]["stringValue"]
            .as_str()
            .unwrap_or("?");
        sims.push((name.to_string(), udid.to_string(), format!("iOS {version}")));
    }
    sims
}

/// Every available iOS simulator, from the enumeration `day devices list` already runs.
fn simctl_simulators() -> Result<Vec<Sim>, CliError> {
    let out = Command::new("xcrun")
        .args(["simctl", "list", "devices", "available", "--json"])
        .output()
        .map_err(|e| CliError::failure(format!("could not run `xcrun simctl list`: {e}")))?;
    if !out.status.success() {
        return Err(CliError::failure(format!(
            "`xcrun simctl list` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let parsed: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| CliError::failure(format!("`xcrun simctl list` printed bad JSON: {e}")))?;
    Ok(sims_from_simctl(&parsed))
}

/// The iOS simulators in a `simctl list devices --json` payload, whose runtime IDENTIFIER
/// ("com.apple.CoreSimulator.SimRuntime.iOS-26-5") becomes the same "iOS 26.5" devicectl reports
/// directly, so the matcher cannot tell which source it was handed.
fn sims_from_simctl(parsed: &Value) -> Vec<Sim> {
    let mut sims = Vec::new();
    for (runtime_key, list) in parsed["devices"].as_object().into_iter().flatten() {
        // iOS runtimes only. `simctl list` reports every installed platform, and an ios-uikit app
        // cannot be installed onto an Apple Watch.
        if !runtime_key.contains("SimRuntime.iOS-") {
            continue;
        }
        let runtime = runtime_label(runtime_key);
        for d in list.as_array().into_iter().flatten() {
            let (name, udid) = (str_of(d, "name"), str_of(d, "udid"));
            if !name.is_empty() && !udid.is_empty() {
                sims.push((name, udid, runtime.clone()));
            }
        }
    }
    sims
}

fn resolve_simulator(device: &str, os: Option<&str>) -> Result<(String, String, String), CliError> {
    let sims = simulators()?;
    let mut best: Option<(String, String, String)> = None;
    for (name, udid, runtime) in &sims {
        if !name.starts_with(device) {
            continue;
        }
        if let Some(want) = os
            && !runtime_matches(runtime, want)
        {
            continue;
        }
        // Newest runtime wins, so `os=iOS 26` lands on the highest 26.x present.
        let better = best
            .as_ref()
            .is_none_or(|(_, _, r)| runtime.as_str() > r.as_str());
        if better {
            best = Some((udid.clone(), name.clone(), runtime.clone()));
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
        let have: Vec<String> = sims
            .iter()
            .map(|(name, _, runtime)| format!("{name} ({runtime})"))
            .collect();
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
            // Ask before the boot, not after it: on a host whose CoreDevice cannot drive
            // simulators this is the difference between a one-second answer and a three-minute
            // boot that ends in "The specified device was not found".
            let turnable = spec.orientation.is_none() || devicectl_sees(&udid);
            if !turnable {
                crate::ops::status(
                    "Warning",
                    &format!(
                        "devicectl cannot see this simulator, so it cannot be turned to {} — \
                         capturing in the orientation it boots in instead. {}",
                        spec.orientation.unwrap_or_default(),
                        CORE_DEVICE_FLOOR
                    ),
                );
            }
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
            if let Some(o) = spec.orientation
                && turnable
            {
                set_orientation(&udid, o)?;
            }
            Ok(0)
        }
        TargetKind::Android => {
            // An AVD by exact name, or `--device` as a NAME PREFIX the way the simulator side
            // resolves — so one profile string ("pixel_tablet") works whether the AVD is called
            // that or `pixel_tablet_api36`.
            let avd = match (spec.id, spec.device) {
                (Some(id), _) => id.to_string(),
                (None, Some(want)) => {
                    let have: Vec<String> = avds().iter().map(|a| str_of(a, "name")).collect();
                    let found = have
                        .iter()
                        .find(|n| n.as_str() == want)
                        .or_else(|| have.iter().find(|n| n.starts_with(want)))
                        .cloned();
                    match found {
                        Some(name) => name,
                        // Nothing enumerated AT ALL is "this machine could not be asked", not
                        // "there is no such AVD" — and the two deserve opposite treatment. A CI
                        // runner that had just created one successfully listed none, and refusing
                        // there turned a working setup into a failed build with an error naming
                        // nothing. Take the caller at their word and let the emulator answer,
                        // which it does by name and with its own diagnosis.
                        None if have.is_empty() => {
                            crate::ops::status(
                                "Warning",
                                &format!(
                                    "no AVD could be enumerated on this machine; trying {want} \
                                     anyway — `day devices list -p android-mdc` shows what the \
                                     tools report"
                                ),
                            );
                            want.to_string()
                        }
                        None => {
                            return Err(CliError::failure(format!(
                                "no AVD named \"{want}…\". This machine has:\n  {}\n\
                                 Create one with `day devices setup`.",
                                have.join("\n  ")
                            )));
                        }
                    }
                }
                (None, None) => {
                    return Err(CliError::usage(
                        "name an AVD: `day devices boot -p android-mdc <AVD>`, or --device"
                            .to_string(),
                    ));
                }
            };
            enable_hw_keyboard(&avd);
            // Already running? Then this command means "make sure it is up and facing this way",
            // not "start another one". Without this, a re-run — a retried CI step, a developer
            // running the same line twice — starts a SECOND emulator on the next free port, and
            // the two then fight over the app, the screenshots and the adb default device.
            let running = android_serials()
                .into_iter()
                .find(|s| avd_of_serial(s).as_deref() == Some(avd.as_str()));
            let serial = match running {
                Some(s) => {
                    crate::ops::status("Found", &format!("emulator {avd} already up as {s}"));
                    s
                }
                None => {
                    // A FIXED console port, so the serial is known before the emulator exists.
                    // Discovering it afterwards means diffing `adb devices`, which races every
                    // other emulator on the machine — and a developer's Mac usually has one.
                    let port = free_emulator_port();
                    let serial = format!("emulator-{port}");
                    crate::ops::status("Booting", &format!("emulator {avd} as {serial}"));
                    spawn_emulator(&avd, port, spec.headless)?;
                    serial
                }
            };
            // Turning needs a booted device, so an orientation implies the wait even when the
            // caller did not ask for one — the alternative is rotating a device that is not there
            // and reporting a failure that is really a race.
            if spec.wait || spec.orientation.is_some() {
                wait_for_android_boot(&serial, 600)?;
            }
            if let Some(o) = spec.orientation {
                rotate_android(&serial, o)?;
            }
            // The serial on STDOUT (status lines go to stderr), so a workflow can capture it:
            //   SERIAL=$(day devices boot -p android-mdc --device pixel_tablet --wait)
            println!("{serial}");
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

/// Run an SDK tool, forwarding everything it prints to STDERR, optionally feeding it `stdin`.
///
/// The SDK tools print progress on STDOUT, and this command's stdout carries a machine-readable
/// value a caller captures. Left alone they mix: a CI run did `AVD="$(day devices setup …)"` and
/// captured three minutes of download bars along with the name, then handed the whole blob to
/// `--device`. Forwarding byte-for-byte rather than line-by-line keeps the progress bars live,
/// since they redraw with carriage returns and never emit a newline until the end.
fn run_sdk_tool(cmd: &mut Command, what: &str, feed: Option<&[u8]>) -> Result<(), CliError> {
    use std::io::{Read, Write};
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .stdin(if feed.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        });
    let mut child = cmd
        .spawn()
        .map_err(|e| CliError::failure(format!("could not run {what} ({e})")))?;
    if let (Some(bytes), Some(mut si)) = (feed, child.stdin.take()) {
        let _ = si.write_all(bytes);
        // Dropped here on purpose: the tool waits for EOF before it decides the answer is final.
        drop(si);
    }
    if let Some(mut out) = child.stdout.take() {
        let mut buf = [0u8; 4096];
        let mut err = std::io::stderr();
        while let Ok(n) = out.read(&mut buf) {
            if n == 0 {
                break;
            }
            let _ = err.write_all(&buf[..n]);
        }
    }
    let st = child
        .wait()
        .map_err(|e| CliError::failure(format!("{what}: {e}")))?;
    if !st.success() {
        return Err(CliError::failure(format!("{what} failed")));
    }
    Ok(())
}

/// `day devices setup` — create (or refresh) one AVD from a device profile.
///
/// AVD creation is knowledge the CLI already had scattered across a workflow and a README: which
/// system image an API level needs, that `avdmanager` writes `hw.keyboard=no`, that a capture run
/// wants a fixed orientation. Putting it behind a command means CI and a developer set a device up
/// the same way, and that the workflow holds no Android SDK trivia of its own.
///
/// Idempotent: an AVD of that name is left in place and only its config is brought up to date, so
/// a cached system image plus a re-run costs seconds.
pub fn setup(target: &str, spec: &SetupSpec<'_>) -> Result<i32, CliError> {
    let t = crate::targets::find(target)
        .ok_or_else(|| CliError::usage(format!("unknown target {target}")))?;
    if t.kind != TargetKind::Android {
        return Err(CliError::usage(format!(
            "`day devices setup` creates Android AVDs; {target} has nothing to create — \
             iOS simulators come with Xcode, and the OpenHarmony emulator with its SDK"
        )));
    }
    // "36", "API 36", "android-36" — a profile is written by a person, and all three spellings
    // turn up in one workflow file.
    let api: u32 = spec
        .os
        .trim()
        .trim_start_matches("android-")
        .trim_start_matches("API")
        .trim_start_matches("api")
        .trim()
        .parse()
        .map_err(|_| {
            CliError::usage(format!(
                "could not read an API level from {:?} — write `36`, `API 36` or `android-36`",
                spec.os
            ))
        })?;
    // The host's ABI by default: an emulator only runs a system image its CPU can execute, and
    // the two hosts that matter here are an x86_64 CI runner and an Apple Silicon Mac.
    let arch = spec.arch.unwrap_or(match std::env::consts::ARCH {
        "aarch64" => "arm64-v8a",
        _ => "x86_64",
    });
    let tag = spec.tag.unwrap_or("google_apis");
    let name = spec
        .name
        .map(str::to_string)
        .unwrap_or_else(|| format!("day_{}_api{api}", spec.device));
    let pkg = format!("system-images;android-{api};{tag};{arch}");

    // The image is a directory in the SDK, which is what makes it cacheable in CI: restore the
    // directory and this step finds it installed and downloads nothing.
    let image_dir = day_toolchain::android_sdk_dir()
        .join("system-images")
        .join(format!("android-{api}"))
        .join(tag)
        .join(arch);
    if image_dir.is_dir() {
        crate::ops::status("Found", &format!("system image {pkg}"));
    } else {
        crate::ops::status("Installing", &format!("system image {pkg}"));
        // `--licenses` is not enough on a cold SDK: the install itself prompts, and a CI runner
        // has no one to answer. Feeding `y` covers both.
        run_sdk_tool(
            // `--sdk_root` is not optional even though the tool has a default. A `sdkmanager`
            // found on PATH — a Homebrew install, say — defaults to ITS OWN SDK root, so the
            // image lands somewhere this command never looks: the "already installed?" check
            // above reads `android_sdk_dir()`, and so does the CI cache. Measured: a download
            // reported success and left `$ANDROID_HOME/system-images/android-33` empty, which
            // re-downloads every run and caches nothing.
            Command::new(cmdline_tool("sdkmanager"))
                .arg(format!(
                    "--sdk_root={}",
                    day_toolchain::android_sdk_dir().display()
                ))
                .arg(&pkg),
            "sdkmanager",
            Some(&b"y\n".repeat(32)),
        )
        .map_err(|e| {
            CliError::failure(format!(
                "{e} — could not install {pkg}; check that the API level, tag and ABI exist \
                 (`sdkmanager --list | grep system-images`)"
            ))
        })?;
    }

    // avdmanager takes its SDK root from where the TOOL lives (`-Dcom.android.sdkmanager.toolsdir`
    // in its launcher), not from `ANDROID_HOME` and with no flag to override it. So a copy found on
    // PATH — a Homebrew install outside the SDK — creates AVDs against ITS root, referencing images
    // the emulator in `android_sdk_dir()` cannot resolve. Installing the SDK's own cmdline-tools
    // makes the two agree, which is the layout a CI image already has, so this is a no-op there.
    let sdk = day_toolchain::android_sdk_dir();
    if !std::path::Path::new(&cmdline_tool("avdmanager")).starts_with(&sdk) {
        crate::ops::status(
            "Installing",
            "cmdline-tools into the SDK — avdmanager reads its SDK root from its own location, \
             and the copy on PATH points at a different one",
        );
        run_sdk_tool(
            Command::new(cmdline_tool("sdkmanager"))
                .arg(format!("--sdk_root={}", sdk.display()))
                .arg("cmdline-tools;latest"),
            "sdkmanager",
            Some(&b"y\n".repeat(32)),
        )?;
    }

    let existing = avds().iter().any(|a| str_of(a, "name") == name);
    if existing {
        crate::ops::status("Found", &format!("AVD {name}"));
    } else {
        crate::ops::status(
            "Creating",
            &format!("AVD {name} ({} on {pkg})", spec.device),
        );
        // It asks whether to start from a custom hardware profile; the device profile named with
        // `-d` already IS the answer.
        run_sdk_tool(
            // Same reason as the `--sdk_root` above, by the route avdmanager takes: it resolves
            // the system image through the SDK root it reads from the environment.
            with_avd_home(&mut Command::new(cmdline_tool("avdmanager")))
                .env("ANDROID_HOME", day_toolchain::android_sdk_dir())
                .env("ANDROID_SDK_ROOT", day_toolchain::android_sdk_dir())
                .args(["create", "avd", "-n", &name, "-k", &pkg, "-d", spec.device]),
            "avdmanager",
            Some(b"no\n"),
        )
        .map_err(|e| {
            CliError::failure(format!(
                "{e} — could not create {name}; is {:?} a device profile? \
                 (`avdmanager list device`)",
                spec.device
            ))
        })?;
    }

    // Config last, and on every run: it is the part that has to be true whether the AVD was just
    // created or restored from a cache, and it is cheap.
    enable_hw_keyboard(&name);
    if let Some(o) = spec.orientation {
        let value = match o.trim().to_ascii_lowercase().as_str() {
            "portrait" | "portrait-upside-down" => "portrait",
            "landscape" | "landscape-left" | "landscape-right" => "landscape",
            other => {
                return Err(CliError::usage(format!(
                    "unknown orientation {other:?} — portrait or landscape"
                )));
            }
        };
        // A starting orientation, not the authority: `day devices boot --orientation` sets the
        // display afterwards and verifies it. This only saves the emulator from booting one way
        // and being turned the other, which a capture run would otherwise photograph mid-turn.
        set_avd_config(&name, "hw.initialOrientation", value);
    }
    println!("{name}");
    Ok(0)
}

/// Set one key in an AVD's `config.ini`, adding it when absent. True when the file changed.
fn set_avd_config(avd: &str, key: &str, value: &str) -> bool {
    let Some(cfg) = avd_config_path(avd) else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(&cfg) else {
        return false;
    };
    let mut seen = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|line| match line.split_once('=') {
            Some((k, _)) if k.trim() == key => {
                seen = true;
                format!("{key}={value}")
            }
            _ => line.to_string(),
        })
        .collect();
    if !seen {
        out.push(format!("{key}={value}"));
    }
    let mut body = out.join("\n");
    body.push('\n');
    if body == text {
        return false;
    }
    std::fs::write(&cfg, body).is_ok()
}

/// Where an AVD keeps its `config.ini`, following the `.ini` pointer when there is one.
fn avd_config_path(avd: &str) -> Option<std::path::PathBuf> {
    for home in avd_homes() {
        // The `.ini` beside the directory is a POINTER — it carries `path=`, and an AVD created
        // under a relocated home does not sit next to it.
        if let Ok(ini) = std::fs::read_to_string(home.join(format!("{avd}.ini")))
            && let Some(dir) = ini.lines().find_map(|l| {
                l.strip_prefix("path=")
                    .map(|p| std::path::PathBuf::from(p.trim()))
            })
        {
            return Some(dir.join("config.ini"));
        }
        let guess = home.join(format!("{avd}.avd")).join("config.ini");
        if guess.is_file() {
            return Some(guess);
        }
    }
    None
}

// ── Android emulator control ─────────────────────────────────────────────────────────────────

/// The SDK's `emulator` binary, or the bare name when the SDK layout does not have it.
fn emulator_bin() -> String {
    let exe = if cfg!(windows) {
        "emulator.exe"
    } else {
        "emulator"
    };
    let bin = day_toolchain::android_sdk_dir().join("emulator").join(exe);
    if bin.is_file() {
        bin.display().to_string()
    } else {
        exe.to_string()
    }
}

/// An SDK command-line tool (`avdmanager`, `sdkmanager`).
///
/// `cmdline-tools/latest` first, then any other versioned directory, then PATH — the three places
/// it lands across a CI image (which installs `latest`), a Homebrew install (PATH only), and an
/// Android Studio one (a versioned directory).
fn cmdline_tool(name: &str) -> String {
    let base = day_toolchain::android_sdk_dir().join("cmdline-tools");
    let exe = if cfg!(windows) {
        format!("{name}.bat")
    } else {
        name.to_string()
    };
    let latest = base.join("latest").join("bin").join(&exe);
    if latest.is_file() {
        return latest.display().to_string();
    }
    if let Ok(dirs) = std::fs::read_dir(&base) {
        let mut found: Vec<std::path::PathBuf> = dirs
            .flatten()
            .map(|e| e.path().join("bin").join(&exe))
            .filter(|p| p.is_file())
            .collect();
        found.sort();
        if let Some(p) = found.pop() {
            return p.display().to_string();
        }
    }
    exe
}

/// Serials `adb` currently lists, in any state.
fn android_serials() -> Vec<String> {
    Command::new("adb")
        .arg("devices")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .skip(1)
                .filter_map(|l| l.split_whitespace().next())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// An emulator console port nothing is using, so the SERIAL is known before the emulator starts.
///
/// Letting the emulator pick means discovering its serial afterwards by diffing `adb devices`,
/// which races every other emulator on the machine — and a developer's Mac usually has one. Ports
/// are even and start at 5554 (`emulator-5554`), the odd port beside each being the adb channel.
fn free_emulator_port() -> u16 {
    let busy: Vec<u16> = android_serials()
        .iter()
        .filter_map(|s| s.strip_prefix("emulator-")?.parse().ok())
        .collect();
    (5554..=5584)
        .step_by(2)
        .find(|p| !busy.contains(p))
        .unwrap_or(5554)
}

/// Which AVD a running emulator is, via its console (`adb emu avd name`).
fn avd_of_serial(serial: &str) -> Option<String> {
    let out = Command::new("adb")
        .args(["-s", serial, "emu", "avd", "name"])
        .output()
        .ok()?;
    out.status.success().then(|| {
        // Two lines: the name, then "OK".
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()
    })
}

/// Start an emulator detached on a known console port.
fn spawn_emulator(avd: &str, port: u16, headless: bool) -> Result<(), CliError> {
    let mut cmd = Command::new(emulator_bin());
    with_avd_home(&mut cmd).args(["-avd", avd, "-port", &port.to_string()]);
    if headless {
        // `swiftshader_indirect` rather than the default `auto`: a runner has no GPU, and auto
        // picks host acceleration and then fails to initialize.
        cmd.args([
            "-no-window",
            "-gpu",
            "swiftshader_indirect",
            "-noaudio",
            "-no-boot-anim",
            "-no-snapshot",
        ]);
    }
    // Detached on purpose: the emulator outlives this command, the way `day launch` expects to
    // find it later. Its own window is where its output belongs.
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            CliError::failure(format!(
                "could not start the Android emulator ({e}) — is the SDK's emulator/ on PATH, or \
                 ANDROID_HOME set?"
            ))
        })?;
    Ok(())
}

/// One `adb -s <serial> shell …`, trimmed. `None` when adb itself fails.
fn adb_shell(serial: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("adb")
        .args(["-s", serial, "shell"])
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Block until the emulator has finished booting, or give up after `secs`.
///
/// `sys.boot_completed` is the only property worth waiting on: `adb wait-for-device` returns as
/// soon as adbd answers, which is minutes before the launcher exists, and installing into that
/// window fails in ways that read as a broken app. `init.svc.bootanim` is checked too because a
/// device reports boot_completed while the boot animation still owns the screen, and a capture
/// taken then is of the animation.
fn wait_for_android_boot(serial: &str, secs: u64) -> Result<(), CliError> {
    crate::ops::status("Waiting", &format!("{serial} to finish booting"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        let booted = adb_shell(serial, &["getprop", "sys.boot_completed"]).as_deref() == Some("1");
        let anim_done =
            adb_shell(serial, &["getprop", "init.svc.bootanim"]).as_deref() != Some("running");
        if booted && anim_done {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    Err(CliError::failure(format!(
        "{serial} did not finish booting within {secs}s (`adb -s {serial} shell getprop \
         sys.boot_completed` never reported 1)"
    )))
}

/// Turn an emulator's DISPLAY to `orientation`, and confirm by the SHAPE it ends up.
///
/// Two things here were each measured being wrong the obvious way.
///
/// `user_rotation` counts quarter-turns from the device's NATURAL orientation, and that differs
/// per device: a phone is born portrait, a tablet landscape. On the pixel_tablet measured here
/// `user_rotation=0` IS landscape (2560x1600) and `1` is portrait — the exact opposite of a phone.
/// So the target is derived from the natural size rather than assumed, and every check asks what
/// SHAPE the display now is, never what number it holds.
///
/// And the window manager is asked, not the settings provider. Writing
/// `settings put system user_rotation` is only a REQUEST: the foreground app still decides, so on
/// a phone the portrait-locked launcher snapped straight back and the write reported success
/// against a display that had already reverted — a false pass, and the app kept portrait even
/// after it started. `cmd window fixed-to-user-rotation enabled` takes that decision away from the
/// app, and `cmd window user-rotation lock N` is absolute, so both form factors land where they
/// were told. The settings path stays as a fallback for an image whose `cmd window` predates
/// those subcommands.
fn rotate_android(serial: &str, orientation: &str) -> Result<(), CliError> {
    let want_landscape = match orientation.trim().to_ascii_lowercase().as_str() {
        "portrait" | "portrait-upside-down" => false,
        "landscape" | "landscape-left" | "landscape-right" => true,
        other => {
            return Err(CliError::usage(format!(
                "unknown orientation {other:?} — portrait or landscape"
            )));
        }
    };
    let natural_landscape = android_natural_landscape(serial).ok_or_else(|| {
        CliError::failure(format!(
            "could not read {serial}'s screen size (`adb -s {serial} shell wm size`)"
        ))
    })?;
    // Quarter-turns from natural. A device already the right shape at rest needs none.
    let want: u8 = u8::from(want_landscape != natural_landscape);
    crate::ops::status("Orienting", &format!("{serial} to {orientation}"));
    // Landscape when the display sits an even number of quarter-turns from a landscape natural
    // orientation, and so on — the shape, not the number.
    let facing = |r: u8| r.is_multiple_of(2) == natural_landscape;
    let mut stable = 0;
    for attempt in 1..=20 {
        adb_shell(
            serial,
            &["cmd", "window", "fixed-to-user-rotation", "enabled"],
        );
        let locked = adb_shell(
            serial,
            &["cmd", "window", "user-rotation", "lock", &want.to_string()],
        )
        .is_some_and(|o| !o.contains("Unknown command") && !o.contains("Error"));
        if !locked {
            // Older image: no `cmd window user-rotation`. Auto-rotate off first, or Android keeps
            // the sensor as the authority and offers a rotate button in the navigation bar, which
            // then sits in every capture.
            adb_shell(
                serial,
                &["settings", "put", "system", "accelerometer_rotation", "0"],
            );
            adb_shell(
                serial,
                &[
                    "settings",
                    "put",
                    "system",
                    "user_rotation",
                    &want.to_string(),
                ],
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        if android_rotation(serial).map(facing) == Some(want_landscape) {
            // Hold it before believing it: a device still finishing boot, or one whose launcher
            // is portrait-locked, agrees once and then reverts. Two agreements a second apart is
            // what tells a real turn from a bounce.
            stable += 1;
            if stable == 2 {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }
        stable = 0;
        if attempt == 20 {
            return Err(CliError::failure(format!(
                "{serial} would not stay turned to {orientation}: it reports rotation {:?} \
                 against a natural orientation of {}",
                android_rotation(serial),
                if natural_landscape {
                    "landscape"
                } else {
                    "portrait"
                }
            )));
        }
    }
    Ok(())
}

/// Whether the device's NATURAL (unrotated) screen is wider than it is tall.
///
/// `wm size` reports the physical panel, which does not move when the display rotates — which is
/// exactly why it is the right thing to compare a rotation against.
fn android_natural_landscape(serial: &str) -> Option<bool> {
    let out = adb_shell(serial, &["wm", "size"])?;
    let line = out.lines().find(|l| l.contains("Physical size:"))?;
    let (w, h) = line.rsplit_once(':')?.1.trim().split_once('x')?;
    Some(w.trim().parse::<u32>().ok()? > h.trim().parse::<u32>().ok()?)
}

/// The display's current rotation as 0/1/2/3, read from the window manager.
fn android_rotation(serial: &str) -> Option<u8> {
    let out = adb_shell(serial, &["dumpsys", "window", "displays"])?;
    let named = |v: &str| match v.trim_end_matches(',') {
        "ROTATION_0" | "0" => Some(0),
        "ROTATION_90" | "1" => Some(1),
        "ROTATION_180" | "2" => Some(2),
        "ROTATION_270" | "3" => Some(3),
        _ => None,
    };
    // `mCurrentRotation=ROTATION_90` on modern Android; older images spell it `rotation=1`.
    for line in out.lines() {
        if let Some(i) = line.find("mCurrentRotation=") {
            let v = line[i + "mCurrentRotation=".len()..]
                .split_whitespace()
                .next()
                .unwrap_or_default();
            return named(v);
        }
    }
    out.split_whitespace()
        .find_map(|w| named(w.strip_prefix("rotation=")?))
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
    if set_avd_config(avd, "hw.keyboard", "yes") {
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
    let mut names = avd_names();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|name| json!({ "id": name.clone(), "name": name }))
        .collect()
}

/// Every AVD name this machine has, from three sources unioned because none answers everywhere.
///
/// `avdmanager list avd -c` is the authoritative one: it is the tool that CREATED the AVD, so it
/// knows where it put it whatever the environment says. `emulator -list-avds` is the documented
/// one but needs the emulator package installed and its own resolution to agree — on a CI runner
/// that had installed the emulator moments earlier it returned nothing. The directory scan is the
/// backstop for a machine whose cmdline-tools are missing or broken.
///
/// The directory has to be SEARCHED FOR as well as read: the SDK tools keep `.android` wherever
/// `ANDROID_USER_HOME` (or the older `ANDROID_SDK_HOME`) points, and a CI image sets one of those
/// away from `$HOME`. Scanning `~/.android/avd` alone was measured finding nothing on a runner
/// that had just created an AVD successfully, which turned into "no AVD named …" listing nothing
/// at all.
fn avd_names() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut take = |text: &str| {
        names.extend(
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.contains(' '))
                .map(str::to_string),
        );
    };
    // 1. The tool that created it. `-c` prints bare names, one per line.
    if let Ok(out) = with_avd_home(&mut Command::new(cmdline_tool("avdmanager")))
        .args(["list", "avd", "-c"])
        .output()
        && out.status.success()
    {
        take(&String::from_utf8_lossy(&out.stdout));
    }
    // 2. The emulator's own view.
    if let Ok(out) = with_avd_home(&mut Command::new(emulator_bin()))
        .arg("-list-avds")
        .output()
        && out.status.success()
    {
        take(&String::from_utf8_lossy(&out.stdout));
    }
    // 3. Whatever is on disk, in every place the tools might have used.
    for home in avd_homes() {
        for entry in std::fs::read_dir(home).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "ini")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                names.push(stem.to_string());
            }
        }
    }
    names
}

/// Pin `ANDROID_AVD_HOME` so `avdmanager` and the `emulator` agree on where AVDs live.
///
/// They resolve that directory INDEPENDENTLY, from an overlapping set of variables
/// (`ANDROID_AVD_HOME`, `ANDROID_USER_HOME`, the older `ANDROID_SDK_HOME`, `$HOME`), and they do
/// not have to reach the same answer. A CI runner created an AVD successfully and then reported
/// having none, because the tool that made it and the tool that lists them were looking in
/// different places. Naming the directory for both ends the disagreement.
///
/// A caller who has already set `ANDROID_AVD_HOME` is left alone — that is them choosing, and the
/// two tools already agree because both read it first.
fn with_avd_home(cmd: &mut Command) -> &mut Command {
    if std::env::var_os("ANDROID_AVD_HOME").is_none()
        && let Some(home) = avd_homes().into_iter().next()
    {
        let _ = std::fs::create_dir_all(&home);
        cmd.env("ANDROID_AVD_HOME", home);
    }
    cmd
}

/// Every directory an AVD might live in, most specific first.
fn avd_homes() -> Vec<std::path::PathBuf> {
    let mut out: Vec<std::path::PathBuf> = Vec::new();
    let mut push = |p: std::path::PathBuf| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    if let Some(v) = std::env::var_os("ANDROID_AVD_HOME") {
        push(std::path::PathBuf::from(v));
    }
    // `ANDROID_USER_HOME` is the current name for what `ANDROID_SDK_HOME` used to mean; both
    // relocate `.android`, and an image may set either.
    if let Some(v) = std::env::var_os("ANDROID_USER_HOME") {
        push(std::path::PathBuf::from(v).join("avd"));
    }
    if let Some(v) = std::env::var_os("ANDROID_SDK_HOME") {
        push(std::path::PathBuf::from(v).join(".android").join("avd"));
    }
    if let Ok(home) = std::env::var("HOME") {
        push(std::path::PathBuf::from(home).join(".android").join("avd"));
    }
    out
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

    /// The two enumerations must be interchangeable, because one silently stands in for the other:
    /// when devicectl cannot answer, simctl's payload reaches the same matcher, and a difference in
    /// how they spell a name or a runtime would resolve `--device`/`--os` differently depending on
    /// which source happened to answer. Both payloads are the real shapes, trimmed to the read keys.
    #[test]
    fn both_enumerations_describe_a_simulator_the_same_way() {
        let devicectl: Value = serde_json::from_str(
            r#"[{
                "identifier": "68932305-F238-4D37-A2E3-FD73FEA39CD8",
                "properties": {
                    "hardware": { "reality": "simulated", "platform": "iOS" },
                    "state": { "name": "iPad Pro 13-inch (M5)" },
                    "software": { "osVersionNumber": { "stringValue": "26.5" } }
                }
            }, {
                "identifier": "PHYSICAL-PHONE",
                "properties": {
                    "hardware": { "reality": "physical", "platform": "iOS" },
                    "state": { "name": "Marc's iPhone" },
                    "software": { "osVersionNumber": { "stringValue": "26.5" } }
                }
            }, {
                "identifier": "A-WATCH",
                "properties": {
                    "hardware": { "reality": "simulated", "platform": "watchOS" },
                    "state": { "name": "Apple Watch Series 11" },
                    "software": { "osVersionNumber": { "stringValue": "12.0" } }
                }
            }]"#,
        )
        .expect("fixture parses");
        let simctl: Value = serde_json::from_str(
            r#"{ "devices": {
                "com.apple.CoreSimulator.SimRuntime.iOS-26-5": [
                    { "name": "iPad Pro 13-inch (M5)", "udid": "68932305-F238-4D37-A2E3-FD73FEA39CD8", "state": "Shutdown" }
                ],
                "com.apple.CoreSimulator.SimRuntime.watchOS-12-0": [
                    { "name": "Apple Watch Series 11", "udid": "A-WATCH", "state": "Shutdown" }
                ]
            } }"#,
        )
        .expect("fixture parses");

        let want = vec![(
            "iPad Pro 13-inch (M5)".to_string(),
            "68932305-F238-4D37-A2E3-FD73FEA39CD8".to_string(),
            "iOS 26.5".to_string(),
        )];
        // A physical phone and a watch are dropped by both: installing an ios-uikit app onto
        // either is a guaranteed failure, and the phone would be installed onto by ACCIDENT.
        assert_eq!(
            sims_from_devicectl(devicectl.as_array().expect("array")),
            want
        );
        assert_eq!(sims_from_simctl(&simctl), want);
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
