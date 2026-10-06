// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The dayscript runner (DESIGN.md §14, §16.5): launches the app with the engine invited
//! (token + runner-chosen port; the port-0 handshake-file refinement is post-MVP), connects
//! over TCP (adb-forwarded on Android), executes the YAML flow, saves screenshots, prints
//! per-step results, and returns exit code 5 on assertion failure.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::meta::Project;
use crate::targets::{Target, TargetKind};
use crate::term::{BOLD, ERROR, SUCCESS, WARN};
use anstream::eprintln;

pub struct ScriptRun {
    pub steps_total: usize,
    pub steps_skipped: usize,
    pub steps_aborted: usize,
    pub steps_failed: usize,
    /// How many of `steps_failed` the engine marked retryable: an element not realized yet,
    /// an assert still pending. Those are the failures a race can produce, so a run whose only
    /// failure is one of them is a retry candidate (cli.rs); a non-retryable failure is a
    /// verdict and re-running would only spend the time again.
    pub retryable_failed: usize,
    pub screenshots: Vec<PathBuf>,
}

/// Why a scripted run could not run to completion.
#[derive(Debug)]
pub enum ScriptError {
    /// The engine socket could not be reached, or died mid-run: the app process is gone (or
    /// never came up). `steps_failed` counts failures seen before the loss; a loss with zero
    /// failures on the iOS simulator is the known app-death flake, which the launch path
    /// retries once. Both CI workflows used to grep the log for exactly this distinction
    /// (`grep "engine connection lost" && ! grep ✗`); typing it here replaced those greps.
    EngineLost {
        steps_failed: usize,
        detail: String,
    },
    Other(String),
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScriptError::EngineLost { detail, .. } => {
                write!(f, "engine connection lost: {detail}")
            }
            ScriptError::Other(e) => f.write_str(e),
        }
    }
}

/// Parse a walkthrough file into engine steps: each flow entry is a single-key mapping
/// (`- tap: { id: x, repeat: 3 }`, `- screenshot: home`, `- wait_idle:`).
/// Expand `${project}` in every string a step carries. A script that hands a real file to a
/// picker (`respond: { path: … }`) needs a fixture that lives in the repository, so the run
/// works on any machine and on CI, and the repository's location is known only here, on the
/// host. The engine itself runs inside the app, where a relative path resolves against the
/// app's writable directory (which on a device is nowhere near the checkout).
fn expand_project(v: &mut serde_json::Value, root: &str) {
    match v {
        serde_json::Value::String(s) => {
            if s.contains("${project}") {
                *s = s.replace("${project}", root);
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(|e| expand_project(e, root)),
        serde_json::Value::Object(m) => m.values_mut().for_each(|e| expand_project(e, root)),
        _ => {}
    }
}

#[derive(Debug, PartialEq)]
struct Flow {
    steps: Vec<(String, serde_json::Value)>,
    stop_on_failure: bool,
}

fn parse_flow(path: &Path, project_root: &Path) -> Result<Flow, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_flow_text(&text, project_root)
}

fn parse_flow_text(text: &str, project_root: &Path) -> Result<Flow, String> {
    let doc: serde_json::Value =
        serde_norway::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    let stop_on_failure = match doc.get("on_failure").and_then(|v| v.as_str()) {
        None if doc.get("on_failure").is_none() => false,
        Some("continue") => false,
        Some("stop") => true,
        _ => return Err("on_failure must be stop or continue".into()),
    };
    let flow = doc
        .get("flow")
        .and_then(|f| f.as_array())
        .ok_or("script has no `flow:` sequence")?;
    let mut steps = Vec::new();
    for entry in flow {
        let obj = entry
            .as_object()
            .ok_or("flow entries must be single-key mappings")?;
        let (op, params) = obj.iter().next().ok_or("empty flow entry")?;
        let mut step = serde_json::Map::new();
        step.insert("op".into(), serde_json::Value::String(op.clone()));
        match params {
            serde_json::Value::Object(m) => {
                for (k, v) in m {
                    step.insert(k.clone(), v.clone());
                }
            }
            serde_json::Value::String(s) if op == "screenshot" => {
                step.insert("name".into(), serde_json::Value::String(s.clone()));
            }
            serde_json::Value::Number(n) if op == "pause" => {
                step.insert("secs".into(), serde_json::Value::Number(n.clone()));
            }
            // `- resize: auto` is the same spelling `size_class: { width: auto }` uses for
            // "back to what the device actually is".
            serde_json::Value::String(s) if op == "resize" && s == "auto" => {
                step.insert("restore".into(), serde_json::Value::Bool(true));
            }
            serde_json::Value::Null => {}
            other => {
                return Err(format!("step {op}: unsupported params {other}"));
            }
        }
        let mut step = serde_json::Value::Object(step);
        expand_project(&mut step, &project_root.to_string_lossy());
        steps.push((op.clone(), step));
    }
    Ok(Flow {
        steps,
        stop_on_failure,
    })
}

/// Perform a `resize:` step's geometry change, host-side (docs/size-classes.md).
///
/// A mobile window belongs to the system: there is no in-app call that resizes it, which is why
/// this half is the runner's. Where the platform gives the host no lever either, the step fails
/// loudly, naming what to do instead. A silent pass would be worse than useless: the assertions
/// after it would all hold at the old size and the walkthrough would report a resize it never did.
fn apply_resize(target: &crate::targets::Target, step: &serde_json::Value) -> Result<(), String> {
    let restore = step
        .get("restore")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let dims = (
        step.get("width").and_then(|v| v.as_f64()),
        step.get("height").and_then(|v| v.as_f64()),
    );
    let (w, h) = match (restore, dims) {
        (true, _) => (0.0, 0.0),
        (false, (Some(w), Some(h))) if w > 0.0 && h > 0.0 => (w, h),
        _ => {
            return Err(
                "needs `width:` and `height:` in points, or `resize: auto` to restore".into(),
            );
        }
    };

    match target.kind {
        // `wm size` resizes the display, which is the one lever that reaches a full-screen
        // activity, and it delivers exactly the configuration change this feature is about
        // (`screenLayout`, `smallestScreenSize`), so it tests the manifest as well as the layout.
        // Points are dp here, which is what Day's breakpoints are in.
        TargetKind::Android => {
            let density = adb_density().unwrap_or(1.0);
            let mut cmd = adb_for_script();
            if restore {
                cmd.args(["shell", "wm", "size", "reset"]);
            } else {
                cmd.args([
                    "shell",
                    "wm",
                    "size",
                    &format!("{}x{}", (w * density).round(), (h * density).round()),
                ]);
            }
            let out = cmd.output().map_err(|e| format!("adb: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "adb wm size failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            // `wm size` returns as soon as the request is queued, not once the display has
            // reconfigured, and a display reconfiguration is a heavier thing than a window
            // resize: SurfaceFlinger reallocates buffers and every visible surface redraws.
            // Screenshots taken before that finished came out torn, the same page composited
            // twice side by side, which reads as a layout bug and is not one.
            //
            // So wait for the display itself, then give the compositor a beat. The engine half
            // of the step is the other barrier (it waits for the app to have seen the new
            // size), but it can only ask about Day's state, not about the surface.
            android_await_display(if restore { None } else { Some((w, h)) });
            Ok(())
        }
        // No public API resizes a simulator scene, and Xcode 27's Device Hub drag is not
        // scriptable. iOS coverage for a width-class crossing comes from running the same
        // walkthrough on an iPhone and an iPad device instead (docs/size-classes.md).
        TargetKind::IosSim => Err(
            "ios-uikit cannot be resized from the host — no simctl or public API does it. \
             Run this walkthrough on an iPad device as well, or gate the step with \
             `skip_on: [ios-uikit]`"
                .into(),
        ),
        TargetKind::HarmonyOs => Err(
            "harmony-arkui cannot be resized from the host yet — gate the step with \
             `skip_on: [harmony-arkui]`"
                .into(),
        ),
        // The desktops and the web own their windows, so both could take a real resize; neither
        // is wired yet, and saying so beats passing a step that moved nothing.
        TargetKind::Desktop | TargetKind::Web => Err(format!(
            "`resize:` is not implemented for {} yet — gate the step with `skip_on: [{}]`",
            target.name, target.name
        )),
    }
}

/// Wait until `wm size` reports the display we asked for, then let the compositor catch up.
///
/// `want` is `Some((w, h))` in dp for a resize, `None` for a restore (wait for the override to be
/// gone). Bounded and best-effort: the engine half of the step is the assertion that matters, so a
/// device that never reports the expected line should not hang the run.
fn android_await_display(want: Option<(f64, f64)>) {
    let density = adb_density().unwrap_or(1.0);
    let expected = want.map(|(w, h)| {
        format!(
            "{}x{}",
            (w * density).round() as i64,
            (h * density).round() as i64
        )
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let out = adb_for_script().args(["shell", "wm", "size"]).output();
        let text = out
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let override_line = text
            .lines()
            .find(|l| l.trim_start().starts_with("Override"));
        let settled = match (&expected, override_line) {
            (Some(want), Some(line)) => line.contains(want.as_str()),
            (None, None) => true,
            _ => false,
        };
        if settled {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    // The display is reconfigured; every visible surface still has to redraw at the new size.
    std::thread::sleep(Duration::from_millis(800));
}

/// `adb`, pinned to the same device the engine forward went to.
fn adb_for_script() -> Command {
    let serial = std::env::var("ANDROID_SERIAL").ok().or_else(|| {
        crate::mobile::android_devices()
            .first()
            .map(|d| d.serial.clone())
    });
    let mut cmd = Command::new(day_toolchain::adb_bin());
    if let Some(serial) = serial {
        cmd.args(["-s", &serial]);
    }
    cmd
}

/// The device's display density, so a resize expressed in dp (what Day's breakpoints are in)
/// reaches `wm size`, which speaks pixels.
fn adb_density() -> Option<f64> {
    let out = adb_for_script()
        .args(["shell", "wm", "density"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // "Physical density: 420" plus an "Override density: 320" line when one is set; the
    // override is what the window actually uses.
    let pick = |label: &str| {
        text.lines()
            .find(|l| l.trim_start().starts_with(label))
            .and_then(|l| l.rsplit(':').next())
            .and_then(|v| v.trim().parse::<f64>().ok())
    };
    let dpi = pick("Override density").or_else(|| pick("Physical density"))?;
    Some(dpi / 160.0)
}

/// Sleep for `total`, returning early with a reason if the engine connection closes meanwhile.
///
/// The dayscript protocol is strictly request/response, so nothing is readable on an idle healthy
/// connection: a zero-byte peek means the peer is gone (the app died), and a timeout means it is
/// alive and idle. Unexpected readable bytes are left for the next roundtrip to interpret rather
/// than guessed at here.
fn sleep_watching_engine(stream: &TcpStream, total: Duration) -> Option<String> {
    const SLICE: Duration = Duration::from_millis(250);
    let deadline = Instant::now() + total;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        std::thread::sleep(SLICE.min(deadline - now));
        let previous = stream.read_timeout().ok().flatten();
        let _ = stream.set_read_timeout(Some(Duration::from_millis(1)));
        let mut byte = [0u8; 1];
        let peeked = stream.peek(&mut byte);
        let _ = stream.set_read_timeout(previous);
        match peeked {
            Ok(0) => return Some("the app closed the dayscript connection".into()),
            Ok(_) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Some(e.to_string()),
        }
    }
}

/// How long to keep (re)trying the engine connection, in seconds. Override with
/// `DAYSCRIPT_CONNECT_SECS`; the default is per-target: 20 s for local targets, 120 s for
/// HarmonyOS, whose software-emulated (TCG) guest can spend minutes between `aa start` and the
/// app-side engine binding its socket (and whose forwarded hdc channel drops with transient
/// connection resets that the roundtrip retry below rides out).
pub(crate) fn connect_window_secs(kind: TargetKind) -> u64 {
    std::env::var("DAYSCRIPT_CONNECT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(match kind {
            TargetKind::HarmonyOs => 120,
            _ => 20,
        })
}

/// Outwait both the engine's implicit retries and its last UI-thread dispatch. The engine may
/// begin that dispatch just before the retry deadline. Its default dispatch allowance is 30 s
/// (day-script's DEFAULT_MAIN_TIMEOUT_SECS), even for a step with a 5 s implicit wait: the old
/// 20 s socket timeout disconnected a healthy engine before it could answer a slow startup.
pub(crate) fn read_window(window_secs: u64, budget_secs: f64) -> Duration {
    reply_window(
        window_secs,
        budget_secs,
        std::env::var("DAY_SCRIPT_MAIN_TIMEOUT_SECS")
            .ok()
            .as_deref(),
    )
}

fn reply_window(window_secs: u64, budget_secs: f64, main_override: Option<&str>) -> Duration {
    // Keep the default and override validation in sync with day-script::main_thread_budget.
    let main_secs = main_override
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(30.0)
        .max(budget_secs);
    let floor = Duration::from_secs(window_secs.max(20));
    Duration::from_secs_f64(budget_secs + main_secs + 10.0).max(floor)
}

#[cfg(test)]
mod reply_window_tests {
    use super::*;

    #[test]
    fn waits_for_a_slow_ui_dispatch_and_its_reply() {
        assert_eq!(reply_window(20, 5.0, None), Duration::from_secs(45));
        assert_eq!(reply_window(20, 5.0, Some("90")), Duration::from_secs(105));
        // A retry begun just before the step deadline can consume another full UI budget.
        assert_eq!(reply_window(20, 120.0, None), Duration::from_secs(250));
        assert_eq!(reply_window(120, 5.0, None), Duration::from_secs(120));
    }

    #[test]
    fn rejects_the_same_invalid_overrides_as_the_engine() {
        for value in ["", "bad", "0", "-1", "NaN", "inf"] {
            assert_eq!(reply_window(20, 5.0, Some(value)), Duration::from_secs(45));
        }
    }
}

/// The least the runner waits for a run's first reply, in seconds. The engine's socket accepts
/// before the app's main thread can answer, so the first step also waits out the rest of the
/// launch: a WinUI cold start took 18 s on a CI runner, and the 20 s local window then reported
/// a healthy app as a lost engine.
const STARTUP_SECS: u64 = 60;

/// Extend a computed [`read_window`] for the first roundtrip to cover the app's startup.
fn first_read_window(window: Duration) -> Duration {
    window.max(Duration::from_secs(STARTUP_SECS))
}

pub(crate) fn connect(port: u16, window_secs: u64) -> Result<TcpStream, String> {
    let attempts = window_secs * 4; // 250 ms apart
    for _ in 0..attempts {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = s.set_nodelay(true);
            // A floor for the handshake only; `roundtrip` resets this per step from the
            // step's wait budget.
            s.set_read_timeout(Some(read_window(window_secs, 0.0))).ok();
            return Ok(s);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!(
        "could not connect to the dayscript engine on 127.0.0.1:{port}"
    ))
}

/// Where a run's screenshots land: `build/day/screenshots/<target>[/<device>]/<subdir>/`. The
/// subdir is the `--variant` name when given (themed/localized capture sets: light / dark / fr),
/// else the locale, else "default".
///
/// The device level is inserted only when `--device` named one, so a run that does not use it
/// writes exactly where it always has: every existing script, site build and local capture tree
/// is unaffected. Where it is used, it separates form factors that would otherwise overwrite each
/// other: one target, one script, an iPhone tree and an iPad tree (docs/screenshots.md).
fn shot_dir(
    project: &Project,
    target: &Target,
    locale: Option<&str>,
    variant: Option<&str>,
    device: Option<&str>,
) -> PathBuf {
    let mut dir = crate::ops::staged_root(project)
        .join("screenshots")
        .join(target.name);
    if let Some(device) = device {
        dir = dir.join(device);
    }
    dir.join(variant.or(locale).unwrap_or("default"))
}

/// Whole-screen Android capture is accepted only between two successful window checks.
fn android_screenshot(serial: &str, path: &Path) -> Result<(), String> {
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    // A system dialog left over the app would be captured with it. Only emulators
    // permit automatic dismissal; physical devices are inspected without mutation.
    crate::mobile::clear_system_dialogs(serial)?;
    let mut cmd = Command::new(day_toolchain::adb_bin());
    cmd.args(["-s", serial, "exec-out", "screencap", "-p"]);
    // Do not use the verbose text-command helper: its tee would log PNG bytes.
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() || !out.stdout.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("adb screencap failed or returned no PNG".into());
    }
    crate::mobile::verify_android_capture(serial)?;
    std::fs::write(path, &out.stdout).map_err(|e| e.to_string())
}

#[cfg(all(test, unix))]
mod android_capture_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    // Run each fake SDK in a separate process: no global environment mutation or
    // chance of changing settings on a developer's connected Android device.
    #[test]
    fn refuses_unverified_android_screenshots() {
        const CHILD: &str = "DAY_TEST_CAPTURE_CASE";
        if let Ok(case) = std::env::var(CHILD) {
            let path = PathBuf::from(std::env::var_os("DAY_TEST_CAPTURE_PATH").unwrap());
            std::fs::write(&path, b"stale image from previous run").unwrap();
            let result = android_screenshot("test-physical-device", &path);
            if case == "clean" {
                assert!(result.is_ok(), "{result:?}");
                assert!(
                    std::fs::read(&path)
                        .unwrap()
                        .starts_with(b"\x89PNG\r\n\x1a\n")
                );
            } else {
                assert!(result.is_err(), "accepted unsafe case: {case}");
                assert!(!path.exists(), "left stale/unsafe image for {case}");
            }
            return;
        }
        let root = std::env::temp_dir().join(format!("day-capture-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("platform-tools")).unwrap();
        let adb = root.join("platform-tools/adb");
        std::fs::write(
            &adb,
            r#"#!/bin/sh
case "$3" in
  shell)
    case "$DAY_TEST_CAPTURE_CASE" in
      probe-failed) exit 1 ;;
      empty-probe) exit 0 ;;
      dialog) echo '  Window #0 Window{abc u0 Application Not Responding: com.android.systemui}:' ;;
      appeared-during-capture)
        if [ -f "$DAY_TEST_CAPTURE_PATH.captured" ]; then
          echo '  Window #0 Window{abc u0 Application Error: dev.daybrite.showcase}:'
        else echo '  Window #0 Window{abc u0 StatusBar}:'; fi ;;
      *) echo '  Window #0 Window{abc u0 StatusBar}:' ;;
    esac ;;
  exec-out)
    touch "$DAY_TEST_CAPTURE_PATH.captured"
    [ "$DAY_TEST_CAPTURE_CASE" = capture-failed ] && exit 1
    [ "$DAY_TEST_CAPTURE_CASE" = invalid-png ] && { echo 'not a PNG'; exit 0; }
    printf '\211PNG\r\n\032\n' ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&adb, std::fs::Permissions::from_mode(0o755)).unwrap();
        for case in [
            "clean",
            "probe-failed",
            "empty-probe",
            "dialog",
            "appeared-during-capture",
            "capture-failed",
            "invalid-png",
        ] {
            let out = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "script::android_capture_tests::refuses_unverified_android_screenshots",
                    "--nocapture",
                ])
                .env(CHILD, case)
                .env("ANDROID_HOME", &root)
                .env("DAY_TEST_CAPTURE_PATH", root.join(format!("{case}.png")))
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{case}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// Device-level capture fallback for targets whose in-process snapshot is unsupported.
/// A modern app acknowledges a render checkpoint before this call. Older Harmony apps
/// retain the configurable conservative delay; identical images are valid and never retried.
fn device_screenshot(target: &Target, path: &Path, ready: bool) -> Result<(), String> {
    match target.kind {
        TargetKind::IosSim => {
            // The simulator this run launched on, else the first booted one, pinned either way
            // so multiple booted sims don't make `simctl … booted` ambiguous. Without the first
            // arm a `--ios-simulator` run photographed whichever sim happened to boot first.
            let udid = crate::ops::selected_ios_simulator()
                .map(str::to_string)
                .or_else(|| crate::mobile::booted_sims().into_iter().next())
                .unwrap_or_else(|| "booted".into());
            let ok = Command::new("xcrun")
                .args(["simctl", "io", &udid, "screenshot"])
                .arg(path)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                Ok(())
            } else {
                Err("simctl screenshot failed".into())
            }
        }
        TargetKind::Android => {
            // Pin the device the runner forwarded to (`android_devices` is already narrowed to
            // this run's selection), else `adb` errors with several attached.
            // Launch already pinned the device. Re-enumerating here ran `adb devices`
            // and an ABI probe for every PNG; neither contributes to capture readiness.
            // The window checks and screencap still verify the selected device is alive.
            let serial = crate::ops::selected_android_serial()
                .map(str::to_string)
                .or_else(|| {
                    std::env::var("ANDROID_SERIAL")
                        .ok()
                        .filter(|s| !s.is_empty())
                })
                .or_else(|| {
                    crate::mobile::android_devices()
                        .into_iter()
                        .next()
                        .map(|dev| dev.serial)
                })
                .ok_or("no Android device available for screenshot")?;
            android_screenshot(&serial, path)
        }
        TargetKind::Desktop => {
            // Engine (in-process) snapshot unavailable. On an X11 session (the CI linux legs run
            // under xvfb) capture the root window with ImageMagick's `import`: with the xvfb
            // screen sized to the app window (ci.yml passes `-screen 0 1000x720x24`) the root is
            // the window. Elsewhere there is nothing portable to call.
            if cfg!(target_os = "linux") && std::env::var_os("DISPLAY").is_some() {
                let ok = Command::new("import")
                    .args(["-window", "root", "-silent"])
                    .arg(path)
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                if ok {
                    return Ok(());
                }
            }
            Err("desktop snapshot returned unsupported".into())
        }
        TargetKind::Web => {
            // The engine's in-page snapshot is unsupported (a DOM can't rasterize itself);
            // the DAY_WEB_DRIVER browser answers instead (docs/web.md).
            crate::web::driver_screenshot(path)
        }
        TargetKind::HarmonyOs => {
            // `uitest screenCap` writes a real PNG; `snapshot_display` writes JPEG (so its bytes
            // in a .png file are wrong), so prefer uitest and fall back to snapshot_display. Then
            // `hdc file recv`.
            // Re-wake the display first (best-effort): a sleeping screen captures as a black frame.
            let _ = crate::ohos::hdc()
                .args(["shell", "power-shell", "wakeup"])
                .status();
            if !ready {
                let settle = std::env::var("DAY_OHOS_SHOT_SETTLE_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(4000);
                eprintln!("    (legacy app: no render checkpoint; waiting {settle}ms)");
                std::thread::sleep(Duration::from_millis(settle));
            }
            let dev = "/data/local/tmp/day-shot.png";
            // screenCap writes into an existing file without truncating it, so a smaller
            // capture would keep the previous shot's tail after its IEND (every shot of a
            // run came out the size of the first). Start each capture from no file.
            let _ = crate::ohos::hdc().args(["shell", "rm", "-f", dev]).status();
            let cap = crate::ohos::hdc()
                .args(["shell", "uitest", "screenCap", "-p", dev])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
                || crate::ohos::hdc()
                    .args(["shell", "snapshot_display", "-f", dev])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
            if !cap {
                return Err("hdc screenshot failed (uitest screenCap / snapshot_display)".into());
            }
            let ok = crate::ohos::hdc()
                .args(["file", "recv", dev])
                .arg(path)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                return Err("hdc file recv failed".into());
            }
            Ok(())
        }
    }
}

/// Reach the in-app dayscript engine from the host: device targets need a TCP forward
/// (adb / hdc); desktop and the iOS simulator answer on loopback directly.
/// Public entry points for `day drive` (drive.rs): the same primitives run_scripts uses.
pub(crate) fn b64decode_public(s: &str) -> Vec<u8> {
    day_script_b64::b64decode(s)
}
pub(crate) fn b64encode_public(bytes: &[u8]) -> String {
    day_script_b64::b64encode(bytes)
}
pub(crate) fn device_screenshot_public(
    target: &Target,
    path: &Path,
    ready: bool,
) -> Result<(), String> {
    device_screenshot(target, path, ready)
}

pub(crate) fn forward_engine(kind: TargetKind, port: u16) {
    if kind == TargetKind::Android {
        // The dayscript runner drives one device; with several attached, `adb forward` (no
        // `-s`) errors ("more than one device"). ANDROID_SERIAL is the device-selection
        // contract everywhere else in the CLI (`--android-device` sets it), so honor it here
        // too; pinning the first enumerated device instead sent the forward to a bystander
        // phone while the app ran on the emulator.
        let serial = std::env::var("ANDROID_SERIAL").ok().or_else(|| {
            crate::mobile::android_devices()
                .first()
                .map(|d| d.serial.clone())
        });
        let mut cmd = Command::new(day_toolchain::adb_bin());
        if let Some(serial) = serial {
            cmd.args(["-s", &serial]);
        }
        let _ = cmd
            .args(["forward", &format!("tcp:{port}"), &format!("tcp:{port}")])
            .status();
    }
    if kind == TargetKind::HarmonyOs {
        // hdc's `adb forward` equivalent: host tcp:port → the app's tcp:port on the launched
        // target, so `connect(port)` reaches the in-app dayscript engine (docs/harmonyos.md;
        // pinned to the discovered device + retried through hdc server recycles in ohos.rs).
        crate::ohos::fport_engine(port);
    }
}

#[allow(clippy::too_many_arguments)] // a straight CLI-flag pass-through, not an API surface
pub fn run_scripts(
    project: &Project,
    target: &'static Target,
    port: u16,
    token: &str,
    scripts: &[PathBuf],
    locale: Option<&str>,
    variant: Option<&str>,
    device: Option<&str>,
    keep_alive: bool,
    attached: bool,
    fast: bool,
) -> Result<ScriptRun, ScriptError> {
    forward_engine(target.kind, port);
    let default_locale = crate::store::default_locale(&crate::store::app_locales(project));
    let window_secs = connect_window_secs(target.kind);
    // A connect failure is an engine loss (the app died during startup, or never bound): the
    // same condition the mid-run loss reports, and the same one the CI retry used to catch.
    let mut stream = connect(port, window_secs).map_err(|detail| ScriptError::EngineLost {
        steps_failed: 0,
        detail,
    })?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|e| ScriptError::Other(e.to_string()))?,
    );

    // adb-forwarded ports accept host connections before the device listener exists; a
    // request/reply that hits EOF reconnects and retries within a bounded window.
    //
    // `budget` is the step's own implicit-wait budget (§14.3). The engine polls a retryable step
    // on the main thread for that long before answering, so the runner must out-wait it: sizing
    // the socket read from `window_secs` alone made any step declaring a longer `timeout_secs`
    // time out runner-side first and report "engine connection lost", a healthy, idle app
    // mislabeled as a dead one.
    // Set once the engine has answered: until then a read also waits out the app's startup.
    let answered = std::cell::Cell::new(false);
    let roundtrip = |stream: &mut TcpStream,
                     reader: &mut BufReader<TcpStream>,
                     line: &str,
                     budget: f64|
     -> Result<String, String> {
        let window = read_window(window_secs, budget);
        let window = if answered.get() {
            window
        } else {
            first_read_window(window)
        };
        let _ = stream.set_read_timeout(Some(window));
        let deadline = std::time::Instant::now() + window;
        loop {
            let attempt = (|| -> Result<String, String> {
                stream
                    .write_all(line.as_bytes())
                    .map_err(|e| e.to_string())?;
                let mut reply = String::new();
                let n = reader.read_line(&mut reply).map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("EOF".into());
                }
                Ok(reply)
            })();
            match attempt {
                Ok(r) => {
                    answered.set(true);
                    return Ok(r);
                }
                Err(e) if std::time::Instant::now() < deadline => {
                    let _ = e;
                    std::thread::sleep(Duration::from_millis(500));
                    if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
                        let _ = s.set_nodelay(true);
                        s.set_read_timeout(Some(window)).ok();
                        *reader = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
                        *stream = s;
                    }
                }
                Err(e) => return Err(e),
            }
        }
    };

    let dir = shot_dir(project, target, locale, variant, device);
    let _ = std::fs::create_dir_all(&dir);

    // Said once per run: the retired `store:` key on a screenshot step (§14.7).
    let mut warned_store = false;
    // New CLI + old app (or an env delivery failure) must keep animation pauses.
    // Learn this from ordinary replies; no extra startup round trip is required.
    let mut app_fast = false;
    let script_started = Instant::now();
    let mut screenshot_engine = Duration::ZERO;
    let mut screenshot_capture = Duration::ZERO;
    let mut run = ScriptRun {
        steps_total: 0,
        steps_skipped: 0,
        steps_aborted: 0,
        steps_failed: 0,
        retryable_failed: 0,
        screenshots: Vec::new(),
    };
    // Captures this run saved, for the per-target gallery index (screenshot.rs §14.7).
    let mut index_entries: Vec<crate::screenshot::TargetEntry> = Vec::new();
    for script in scripts {
        let Flow {
            steps,
            stop_on_failure,
        } = parse_flow(script, &project.root).map_err(ScriptError::Other)?;
        let failures_before_script = run.steps_failed;
        let planned = steps.len();
        if stop_on_failure {
            // A stopped retry must not leave old "successful" captures for steps it never
            // reached. Clear only this flow's named outputs in the current variant.
            for (op, step) in &steps {
                if op == "screenshot" {
                    let name = step.get("name").and_then(|v| v.as_str()).unwrap_or("shot");
                    let path = dir.join(format!("{name}.png"));
                    if path.exists() {
                        std::fs::remove_file(path)
                            .map_err(|e| ScriptError::Other(e.to_string()))?;
                    }
                }
            }
        }
        // `expect_exit` tolerates the app dying, so it must be terminal: a step after it could
        // never run (the connection is gone). Reject a misplaced one before driving anything.
        if let Some(pos) = steps.iter().position(|(op, _)| op == "expect_exit")
            && pos != steps.len() - 1
        {
            return Err(ScriptError::Other(format!(
                "{}: expect_exit must be the last step",
                script.display()
            )));
        }
        eprintln!(
            "{BOLD}     Script{BOLD:#} {} on {} ({} steps)",
            script.display(),
            target.name,
            steps.len()
        );
        // Desktop renders in-process (there is no device tool to ask); everything else is
        // captured from the device, which decides whether the engine's payload is wanted.
        let device_first = target.kind != TargetKind::Desktop;
        // What the per-step gates below match, beyond the target name and toolkit: the build
        // flavor (§16.6), as `flavor:<name>`, or `flavor:none` for the base app. A flavor changes
        // what the app says and which pages it has, so the script that covers both says which
        // step belongs to which — `skip_on: [flavor:custom]` beside `only_on: [flavor:custom]`,
        // rather than a second copy of the whole walkthrough.
        let flavor_gate = match crate::flavor::active() {
            Some(name) => format!("flavor:{name}"),
            None => "flavor:none".to_string(),
        };
        // A gate token nothing can match is a typo, reported once per token: `only_on: [iso]`
        // would otherwise drop the step on every target without a word.
        let mut warned_gates: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (index, (mut op, step)) in steps.into_iter().enumerate() {
            if stop_on_failure && run.steps_failed > failures_before_script {
                let remaining = planned - index;
                run.steps_aborted += remaining;
                eprintln!(
                    "  {WARN}–{WARN:#} stopping {} after failure; {remaining} dependent steps not run",
                    script.display()
                );
                break;
            }
            run.steps_total += 1;
            // The target gates run before the runner-side steps below (`pause`, `expect_exit`):
            // those `continue` on their own, so evaluating them first made a gated `pause` sleep
            // on every target regardless of its `only_on`/`skip_on`: 10s a variant on Android
            // and 21s on HarmonyOS, spent waiting for blocks those targets never run.
            //
            // `skip_on:` is a per-step target filter: the step is dropped on the named targets
            // or toolkits (`skip_on: [web-dom]`), so one walkthrough runs across platforms
            // that lack a capability (docs/agent.md).
            for list in ["skip_on", "only_on"] {
                for token in step
                    .get(list)
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str())
                {
                    if !gate_is_known(token) && warned_gates.insert(token.to_string()) {
                        eprintln!(
                            "  {WARN}!{WARN:#} {op}: `{list}: [{token}]` names no target, toolkit, \
                             platform or flavor — check the spelling"
                        );
                    }
                }
            }
            if let Some(skips) = step.get("skip_on").and_then(|v| v.as_array()) {
                let hit = skips
                    .iter()
                    .filter_map(|v| v.as_str())
                    .any(|s| gate_names(s, target, &flavor_gate));
                if hit {
                    eprintln!("  {WARN}–{WARN:#} {op} (skipped on {})", target.name);
                    run.steps_skipped += 1;
                    continue;
                }
            }
            // `only_on:` is skip_on's mirror, for a step whose expectations are per-target (an
            // `assert_no_placeholders` allow-list differs sharply between, say, appkit and
            // web-dom, so the script carries one step per target group).
            if let Some(onlys) = step.get("only_on").and_then(|v| v.as_array()) {
                let hit = onlys
                    .iter()
                    .filter_map(|v| v.as_str())
                    .any(|s| gate_names(s, target, &flavor_gate));
                if !hit {
                    // Name what actually excluded it: a step kept for one flavor reads as
                    // "not for flavor:none" on the base app, where "not for macos-appkit" would
                    // point at the wrong reason.
                    let scope = if onlys
                        .iter()
                        .filter_map(|v| v.as_str())
                        .any(|s| s.starts_with("flavor:"))
                    {
                        flavor_gate.as_str()
                    } else {
                        target.name
                    };
                    eprintln!("  {WARN}\u{2013}{WARN:#} {op} (not for {scope})");
                    run.steps_skipped += 1;
                    continue;
                }
            }
            let mut step = step;
            // Only explicitly decorative pauses may be omitted in fast mode. Still
            // drain UI work and honor the toolkit transition gate. Literal pauses retain
            // their duration, including playback, physics and crash-watch tests.
            if fast
                && app_fast
                && op == "pause"
                && step.get("animation").and_then(|v| v.as_bool()) == Some(true)
            {
                op = "wait_idle".into();
                step = serde_json::json!({"op": "wait_idle"});
            }
            // `pause` sleeps runner-side (the engine must not block the UI thread).
            if op == "pause" {
                let secs = step.get("secs").and_then(|v| v.as_f64()).unwrap_or(0.5);
                // Sleeping runner-side means a pause touches nothing, so a dead app used to go
                // unnoticed for the whole tail of a walkthrough: every remaining `pause` and every
                // step this target skips reported ✓ against a process that was already gone, and
                // the loss only surfaced minutes later at the next step that needed the app.
                // Watching the socket while we wait ends the run where the app actually died.
                if let Some(detail) = sleep_watching_engine(&stream, Duration::from_secs_f64(secs))
                {
                    eprintln!("  {ERROR}✗{ERROR:#} pause {secs}s — {detail}");
                    return Err(ScriptError::EngineLost {
                        steps_failed: run.steps_failed,
                        detail,
                    });
                }
                eprintln!("  {SUCCESS}✓{SUCCESS:#} pause {secs}s");
                continue;
            }
            // `resize` is runner-side first, then engine: a device's window belongs to the
            // system, so only the host can change it, and the engine half that follows is what
            // waits for the app to have seen the new geometry (docs/size-classes.md).
            if op == "resize"
                && let Err(why) = apply_resize(target, &step)
            {
                run.steps_failed += 1;
                eprintln!("  {ERROR}✗{ERROR:#} resize — {why}");
                continue;
            }
            // `expect_exit` is runner-side: a prior step triggered an expected exit/crash, so
            // here the connection is supposed to drop. Probe until it does (success) or the
            // window elapses (the app survived: failure). Never sent to the engine.
            if op == "expect_exit" {
                let within = step.get("within").and_then(|v| v.as_f64()).unwrap_or(15.0);
                let deadline = std::time::Instant::now() + Duration::from_secs_f64(within);
                let probe = serde_json::json!({"token": token, "step": {"op": "wait_idle"}});
                let mut probe_line = serde_json::to_string(&probe).unwrap();
                probe_line.push('\n');
                let mut exited = false;
                while std::time::Instant::now() < deadline {
                    // Direct write+read with NO reconnect: a dropped connection is the goal.
                    if stream.write_all(probe_line.as_bytes()).is_err() {
                        exited = true;
                        break;
                    }
                    let mut reply = String::new();
                    match reader.read_line(&mut reply) {
                        Ok(0) => {
                            exited = true;
                            break;
                        }
                        Ok(_) => std::thread::sleep(Duration::from_millis(250)),
                        Err(_) => {
                            exited = true;
                            break;
                        }
                    }
                }
                if exited {
                    eprintln!("  {SUCCESS}✓{SUCCESS:#} expect_exit (app terminated as expected)");
                } else {
                    run.steps_failed += 1;
                    eprintln!(
                        "  {ERROR}✗{ERROR:#} expect_exit — app still running after {within}s"
                    );
                }
                continue;
            }
            let mut shot_meta = crate::screenshot::ShotMeta::default();
            if let Some(map) = step.as_object_mut() {
                map.remove("skip_on");
                map.remove("only_on");
                // A screenshot step's gallery metadata (`title:`/`caption:`/`source:`, §14.7)
                // is the runner's: it feeds the per-target gallery.json below and must not
                // reach the engine, because apps predate it and never need it.
                if op == "screenshot" {
                    shot_meta = crate::screenshot::extract_meta(map);
                    if shot_meta.legacy_store && !warned_store {
                        warned_store = true;
                        eprintln!(
                            "  {WARN}▸{WARN:#} a screenshot step carries `store:`, which is ignored: \
                             the listing is declared in store/storefront.toml [storefront…screenshots] now \
                             (docs/store.md)"
                        );
                    }
                    // A device target's capture comes from `simctl`/`adb` below, so the
                    // engine's own render would be encoded and discarded. Ask it not to.
                    // The fallback path re-asks, so nothing is lost when a device refuses.
                    if device_first {
                        map.insert("in_process".into(), serde_json::Value::Bool(false));
                    }
                }
            }
            // Kept for the fallback re-request: `step` itself is moved into the request.
            let step_for_retry = step.clone();
            let req = serde_json::json!({"token": token, "step": step});
            let mut line = serde_json::to_string(&req).unwrap();
            line.push('\n');
            let budget = step
                .get("timeout_secs")
                .and_then(|v| v.as_f64())
                .filter(|t| *t > 0.0)
                .unwrap_or(5.0);
            // A roundtrip that gives up reconnecting means the app process is gone. Carry
            // the failure count seen so far, so the caller can tell a clean-run flake (retry)
            // from a failing run that then died (report).
            let failed_before = run.steps_failed;
            let checkpoint_started = Instant::now();
            let reply_line =
                roundtrip(&mut stream, &mut reader, &line, budget).map_err(|detail| {
                    ScriptError::EngineLost {
                        steps_failed: failed_before,
                        detail,
                    }
                })?;
            let reply: serde_json::Value = serde_json::from_str(reply_line.trim())
                .map_err(|e| ScriptError::Other(e.to_string()))?;
            app_fast = reply.get("fast_animations").and_then(|v| v.as_bool()) == Some(true);
            let ok = reply.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let detail = step
                .get("id")
                .and_then(|v| v.as_str())
                .or_else(|| step.get("name").and_then(|v| v.as_str()))
                .unwrap_or("");
            if ok {
                eprintln!("  {SUCCESS}✓{SUCCESS:#} {op} {detail}");
            } else {
                run.steps_failed += 1;
                if reply
                    .get("retryable")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    run.retryable_failed += 1;
                }
                let err = reply
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("failed");
                eprintln!("  {ERROR}✗{ERROR:#} {op} {detail} — {err}");
            }
            if op == "tests" && ok {
                print_test_listing(&reply);
            }
            if op == "run_tests" && ok {
                // The in-app runner's report (docs/testing.md): one line per test, the evidence
                // beside the captures, the captures under tests/<test>/. A failed test fails
                // the script the way a failed step does, so CI reads it the same way.
                let report = reply
                    .get("data")
                    .cloned()
                    .ok_or_else(|| ScriptError::Other("run_tests answered no report".into()))?;
                let failed = print_test_report(&report);
                run.steps_failed += failed;
                match write_evidence(&dir, target, variant.or(locale), device, &report) {
                    Ok(path) => eprintln!("      {BOLD}Evidence{BOLD:#} {}", path.display()),
                    Err(e) => eprintln!("  {WARN}▸{WARN:#} evidence not written: {e}"),
                }
            }
            if op == "screenshot" && ok {
                screenshot_engine += checkpoint_started.elapsed();
                let capture_started = Instant::now();
                let name = step.get("name").and_then(|v| v.as_str()).unwrap_or("shot");
                let path = dir.join(format!("{name}.png"));
                // A failed recapture must not publish a previous run's image at this path.
                if path.exists() {
                    std::fs::remove_file(&path).map_err(|e| ScriptError::Other(e.to_string()))?;
                }
                let in_process = reply
                    .get("png_base64")
                    .and_then(|v| v.as_str())
                    .map(day_script_b64::b64decode);
                let write_in_process = |bytes: &[u8]| std::fs::write(&path, bytes).is_ok();

                // On a device or simulator the device capture stays the one that is saved. It is
                // the whole screen (status bar, home indicator, system chrome), which is what every
                // published mobile gallery shot shows; the in-process capture frames the app's
                // view tree alone, so preferring it would silently re-crop every one of them.
                //
                // The mobile backends that grew an in-process capture (docs/window-image.md) are
                // the fallback instead: a refusing device tool used to abandon the shot outright.
                // Desktop is the other way round: the in-process render is the capture there
                // and `device_screenshot` has no desktop arm at all.
                let ready = reply
                    .get("capture_revision")
                    .and_then(|v| v.as_u64())
                    .is_some();
                let mut saved = false;
                if device_first {
                    match device_screenshot(target, &path, ready) {
                        Ok(()) => saved = true,
                        Err(e) => {
                            // The payload was skipped above, so fetch it now; this arm
                            // runs when a device tool refuses, not once per shot.
                            let mut again = step_for_retry.clone();
                            if let Some(m) = again.as_object_mut() {
                                m.insert("in_process".into(), serde_json::Value::Bool(true));
                            }
                            let req = serde_json::json!({"token": token, "step": again});
                            let mut l = serde_json::to_string(&req).unwrap();
                            l.push('\n');
                            let bytes = roundtrip(&mut stream, &mut reader, &l, 0.0)
                                .ok()
                                .and_then(|r| {
                                    serde_json::from_str::<serde_json::Value>(r.trim()).ok()
                                })
                                .and_then(|r| {
                                    r.get("png_base64")
                                        .and_then(|v| v.as_str())
                                        .map(day_script_b64::b64decode)
                                });
                            match bytes {
                                Some(b) => {
                                    eprintln!(
                                        "    (device screenshot failed: {e} — captured \
                                         in-process instead: app content only)"
                                    );
                                    saved = write_in_process(&b);
                                }
                                None => eprintln!("    (device screenshot failed: {e})"),
                            }
                        }
                    }
                } else {
                    if let Some(bytes) = in_process.as_deref() {
                        saved = write_in_process(bytes);
                    }
                    // Desktop's own fallback, unchanged: the engine declines (no window on screen,
                    // or a backend with no capture) and the Linux CI legs read the xvfb root.
                    if !saved {
                        match device_screenshot(target, &path, ready) {
                            Ok(()) => saved = true,
                            Err(e) => eprintln!("    (desktop screenshot failed: {e})"),
                        }
                    }
                }
                if saved {
                    // One file shape for every target, whatever tool took the capture
                    // (screenshot.rs `normalize_capture`). `DAY_SCREENSHOT_RAW=1` keeps the
                    // file as the capture tool wrote it, for diagnosing that tool.
                    if std::env::var_os("DAY_SCREENSHOT_RAW").is_none() {
                        match crate::screenshot::normalize_capture(&path) {
                            Ok(true) => {}
                            Ok(false) => eprintln!(
                                "    (screenshot {name} kept as captured: it is 16-bit or embeds a color profile other than sRGB)"
                            ),
                            Err(e) => eprintln!("    (screenshot {name} kept as captured: {e})"),
                        }
                    }
                    // Record the capture in the target's gallery index (screenshot.rs): the
                    // step's metadata plus the saved file's facts. The subdir name is the
                    // variant key the published index uses.
                    let vname = variant.or(locale).unwrap_or("default");
                    // The entry carries the locale outright: the run's, else the app's
                    // default, so the index never has to decode the variant name.
                    if let Some(entry) = crate::screenshot::target_entry(
                        &path,
                        vname,
                        device,
                        name,
                        locale.or(default_locale.as_deref()),
                        Some(&shot_meta),
                    ) {
                        index_entries.push(entry);
                    }
                    run.screenshots.push(path);
                    screenshot_capture += capture_started.elapsed();
                } else {
                    run.steps_failed += 1;
                    eprintln!("  {ERROR}✗{ERROR:#} screenshot {name} — no safe capture available");
                }
            }
        }
    }
    eprintln!(
        "  Script timing: {:.3}s; {} screenshots: engine/checkpoints {:.3}s, capture/save {:.3}s",
        script_started.elapsed().as_secs_f64(),
        run.screenshots.len(),
        screenshot_engine.as_secs_f64(),
        screenshot_capture.as_secs_f64()
    );
    if keep_alive {
        // Interactive script development (docs/agent.md): leave the app running so its session
        // stays drivable (`day drive`) and scripts can be built and debugged incrementally.
        // Attached: `day` stays in the foreground streaming the app's console output until the
        // app exits or the run is stopped. Detached: `day` exits now and the app lives on.
        //
        // A device app is stopped by a registered command rather than by dying with its parent
        // (signals.rs), and the exit path runs those unconditionally, which would take down the
        // very app this flag exists to keep. Retract them: `--keep-alive` is the explicit wish,
        // and it outranks the interrupt contract.
        crate::signals::forget_remote_stops();
        // And the desktop half: there the app is a child of this process, so the exit path's
        // `kill_all` would take it down moments after this line promised it was staying up.
        crate::signals::forget_app_children();
        if attached {
            eprintln!(
                "  {WARN}▸{WARN:#} {} left running (--keep-alive): streaming logs — stop the task \
                 (or Ctrl-C) to quit; drive it from another shell with `day drive -p {}`",
                target.name, target.name
            );
        } else {
            eprintln!(
                "  {WARN}▸{WARN:#} {} left running (--keep-alive): drive it with `day drive -p {}`",
                target.name, target.name
            );
        }
    } else {
        // Terminate the app now that the run is over (and drop its session entry).
        terminate(project, target);
        crate::sessions::remove(&project.root, target.name);
    }
    // Refresh the machine-local screenshot gallery (an at-a-glance index of every capture
    // set under build/day/screenshots/) after each run that saved captures, and fold this
    // run's captures into the target's machine-readable gallery index (screenshot.rs);
    // `day screenshot index` merges those per-target files into the published gallery.json.
    if !index_entries.is_empty() {
        crate::screenshot::record_target_entries(
            &crate::ops::staged_root(project).join("screenshots"),
            target.name,
            index_entries,
        );
    }
    if !run.screenshots.is_empty() {
        write_gallery(&crate::ops::staged_root(project).join("screenshots"));
    }
    Ok(run)
}

/// Regenerate `build/day/screenshots/index.html`: one labeled thumbnail per capture, grouped
/// by `<target>/<variant>`, each linking to the full-size image: a quick browsable index of
/// everything captured on this machine (open it with `open build/day/screenshots/index.html`).
fn write_gallery(root: &Path) {
    fn dirs(p: &Path) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(p)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;")
    }
    let mut body = String::new();
    let mut shots = 0usize;
    for target in dirs(root) {
        let tname = target
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        // A target's children are variant directories, or device directories that each hold
        // variants (`ios-uikit/ipad/dark/`, docs/screenshots.md). Both are walked, and a device
        // is named in the heading beside its variant so two form factors of the same capture
        // set are distinguishable at a glance.
        let levels: Vec<(Option<String>, PathBuf)> = dirs(&target)
            .into_iter()
            .flat_map(|d| {
                let name = d
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let inner = dirs(&d);
                let has_png = std::fs::read_dir(&d)
                    .map(|rd| {
                        rd.flatten()
                            .any(|e| e.path().extension().is_some_and(|x| x == "png"))
                    })
                    .unwrap_or(false);
                if !inner.is_empty() && !has_png {
                    inner
                        .into_iter()
                        .map(|v| (Some(name.clone()), v))
                        .collect::<Vec<_>>()
                } else {
                    vec![(None, d)]
                }
            })
            .collect();
        for (device, variant) in levels {
            let vname = match &device {
                Some(d) => format!(
                    "{d} · {}",
                    variant.file_name().unwrap_or_default().to_string_lossy()
                ),
                None => variant
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            };
            let mut pngs: Vec<PathBuf> = std::fs::read_dir(&variant)
                .map(|rd| {
                    rd.flatten()
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|e| e == "png"))
                        .collect()
                })
                .unwrap_or_default();
            pngs.sort();
            if pngs.is_empty() {
                continue;
            }
            body.push_str(&format!(
                "<section><h2>{} <span class=\"v\">{}</span></h2><div class=\"grid\">",
                esc(&tname),
                esc(&vname)
            ));
            for png in &pngs {
                let name = png
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let rel = format!("{}/{}/{}.png", tname, vname, name);
                body.push_str(&format!(
                    "<a href=\"{rel}\"><figure><img loading=\"lazy\" src=\"{rel}\" alt=\"{n}\"><figcaption>{n}</figcaption></figure></a>",
                    rel = esc(&rel),
                    n = esc(&name)
                ));
                shots += 1;
            }
            body.push_str("</div></section>");
        }
    }
    let html = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>day screenshots</title><style>\
         body{{font:14px system-ui;margin:24px;background:#16181d;color:#e8eaf0}}\
         h1{{font-size:1.2rem}} h2{{font-size:0.9rem;margin:28px 0 10px;text-transform:uppercase;letter-spacing:0.08em}}\
         h2 .v{{color:#8bd5d3;margin-left:6px}} a{{color:inherit;text-decoration:none}}\
         .grid{{display:flex;flex-wrap:wrap;gap:14px}} figure{{margin:0;width:120px}}\
         img{{width:120px;border:1px solid #333a44;border-radius:6px;display:block;background:#0f1115}}\
         figcaption{{font-size:11px;color:#9aa0ad;text-align:center;margin-top:4px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}}\
         </style><h1>day screenshots — {shots} captures</h1>{body}"
    );
    let _ = std::fs::write(root.join("index.html"), html);
}

/// Quote a literal for use inside the extended regular expression `pkill -f` takes. The project
/// root goes into that pattern, and a checkout path containing `+`, `(` or `[` would otherwise be
/// read as syntax and match the wrong processes (or none).
fn ere_escape(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len());
    for c in literal.chars() {
        if "\\.[]{}()*+?^$|".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Poll until nothing matches `pattern` any more. `true` if the processes went away inside
/// `budget`. Used to make [`terminate`] mean "it is gone", not "it has been asked to go".
fn await_exit(pattern: &str, budget: Duration) -> bool {
    let deadline = std::time::Instant::now() + budget;
    loop {
        // pgrep exits non-zero with no output when nothing matches, and never reports itself.
        let alive = Command::new("pgrep")
            .args(["-f", pattern])
            .output()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false);
        if !alive {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The `pkill -f` pattern for a desktop app built into `build_root`, matching both desktop
/// layouts: `<build_root>/cargo/<target>/…` and `<build_root>/<target>/…`.
fn desktop_pattern(build_root: &Path, target: &str) -> String {
    format!(
        "^{}/(cargo/)?{target}/",
        ere_escape(&build_root.to_string_lossy())
    )
}

pub(crate) fn terminate(project: &Project, target: &Target) {
    match target.kind {
        TargetKind::Desktop if cfg!(windows) => {
            // No pkill on Windows; kill the app by image name (taskkill is on every runner).
            let _ = Command::new("taskkill")
                .args(["/F", "/IM", &format!("{}.exe", project.manifest.app.name)])
                .status();
        }
        TargetKind::Desktop => {
            // Match the launch directory, never the app name. Two layouts have to be covered,
            // because macos-appkit now builds through a scaffolded Xcode host project (§17.4)
            // while every other desktop target is still a bare cargo binary:
            //
            //   <build_root>/cargo/<target>/<profile>/<name>                     cargo
            //   <build_root>/<target>/<config>/<Name>.app/Contents/MacOS/<Name>  xcodebuild
            //
            // and the executable's name is not common ground between them: `app.name` is the
            // crate name (`day-skies`), while an Xcode bundle's binary is named by the pbxproj's
            // PRODUCT_NAME (`DaySkies`). A pattern built from `app.name` matches nothing at all
            // under the second layout. The directory is the one thing both agree on, and it is
            // also what makes this project-specific: two checkouts building the same target
            // would otherwise terminate each other's apps.
            //
            // The root is `ops::build_root`, the one the launch built into, not a literal
            // `build/day`: a flavor builds under `build/day/flavors/<flavor>/` and `--day-src`
            // under `build/day/day-src/<slug>/`, and a pattern that misses those matches nothing.
            //
            // Getting this wrong is not a leaked process so much as a corrupted run: the
            // survivor holds the dayscript engine's port, the next launch cannot bind, and the
            // runner then drives the old app, which shares the run's token and answers every
            // step, so a locale sweep quietly re-photographs the first locale.
            let pattern = desktop_pattern(&crate::ops::build_root(project), target.name);
            let _ = Command::new("pkill").args(["-f", &pattern]).status();
            // `pkill` only delivers the signal; the app still has to run its teardown, and
            // it holds the engine port until it does. Returning here would hand the next launch
            // a port that is still bound: the same corrupted run as above, reached by a race
            // instead of by a bad pattern. So wait for the process table to clear, and
            // escalate to SIGKILL for an app that will not go on its own.
            if !await_exit(&pattern, Duration::from_secs(10)) {
                let _ = Command::new("pkill").args(["-9", "-f", &pattern]).status();
                let _ = await_exit(&pattern, Duration::from_secs(5));
            }
        }
        // The three device branches are bounded (`status_within`): each talks to a device over a
        // tool that waits for an unresponsive one indefinitely, and this runs between a matrix
        // run's variants, so a wedged emulator here stops the run rather than the app.
        TargetKind::IosSim => {
            let _ = crate::ops::status_within(
                Command::new("xcrun").args([
                    "simctl",
                    "terminate",
                    "booted",
                    &project.manifest.resolve(target.name).id,
                ]),
                DEVICE_CMD,
            );
        }
        TargetKind::Android => {
            let _ = crate::ops::status_within(
                Command::new(day_toolchain::adb_bin()).args([
                    "shell",
                    "am",
                    "force-stop",
                    &project.manifest.resolve(target.name).id,
                ]),
                DEVICE_CMD,
            );
        }
        TargetKind::HarmonyOs => {
            let key = crate::ops::selected_ohos_key()
                .map(str::to_owned)
                .unwrap_or_else(crate::ohos::ohos_target);
            crate::ohos::stop_hilog(&key);
            let _ = crate::ops::status_within(
                crate::ohos::hdc_for(&key).args([
                    "shell",
                    "aa",
                    "force-stop",
                    &project.manifest.resolve(target.name).id,
                ]),
                DEVICE_CMD,
            );
        }
        // Stop the DAY_WEB_DRIVER browser when one is running; an interactively opened
        // browser tab is the user's own, and the dev server dies with `day`.
        TargetKind::Web => crate::web::stop_driver(),
    }
}

/// How long a single device command gets before it is treated as a device that stopped
/// answering. Force-stopping an app or asking a shell to echo is a sub-second operation; a
/// multiple of that leaves room for a loaded runner without leaving room for a hang.
const DEVICE_CMD: Duration = Duration::from_secs(30);

/// Whether this target's device still answers. Asked after the engine is lost, to decide
/// whether there is any point running the next variant.
///
/// The probe is the device's own shell rather than a host-side listing: a wedged emulator is
/// still *connected*, so `adb get-state` cheerfully reports `device` for one that will never
/// answer another command. Running something on it is the only question worth asking, and
/// `status_within` supplies the deadline the tools do not have.
///
/// A desktop or web target has no device to lose, so it is always live.
pub(crate) fn device_alive(target: &Target) -> bool {
    let mut probe = match target.kind {
        TargetKind::Android => {
            let mut c = Command::new(day_toolchain::adb_bin());
            c.args(["shell", "true"]);
            c
        }
        TargetKind::HarmonyOs => {
            let mut c = crate::ohos::hdc();
            c.args(["shell", "echo", "ok"]);
            c
        }
        TargetKind::IosSim => {
            let mut c = Command::new("xcrun");
            c.args(["simctl", "list", "devices", "booted"]);
            c
        }
        TargetKind::Desktop | TargetKind::Web => return true,
    };
    crate::ops::output_within(&mut probe, DEVICE_CMD).is_some_and(|o| o.status.success())
}

/// A port for the launch's dayscript engine to bind. The pid-based start keeps concurrent
/// `day` invocations in different ranges; the bind probe then takes the first port that is
/// free; the arithmetic alone handed out ports something else already held. Falls back to the
/// base when the whole range is busy (the old behavior: let the launch report it).
///
/// The constant exists to keep the range below 32768. Linux's default ephemeral range is
/// 32768–60999 (`net.ipv4.ip_local_port_range`), which Android inherits, so a port picked from
/// inside it can already be the local end of some unrelated outbound connection, and `bind`
/// fails with EADDRINUSE. The probe below cannot see that: it tests the host, while the engine
/// binds inside the emulator, where a long-lived connection (adbd's, the emulator's services)
/// holds the number for the whole session. That is exactly how it presented:
/// `day-script: bind 127.0.0.1:34951 failed after 15s: Address already in use`, on every
/// launch of the job, while neighboring ports in other jobs were fine.
///
/// Below 32768 the kernel never hands the number out on its own, so only a real listener can
/// collide, on either side.
const ENGINE_PORT_BASE: u16 = 20000;

pub fn pick_port(index: usize) -> u16 {
    let base = ENGINE_PORT_BASE + (std::process::id() % 9000) as u16 + index as u16;
    for port in base..base.saturating_add(100) {
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    base
}

pub fn make_token() -> String {
    format!(
        "{:x}-{:x}",
        std::process::id(),
        std::time::UNIX_EPOCH
            .elapsed()
            .map(|d| d.as_millis())
            .unwrap_or(0)
    )
}

/// The platform a target belongs to, as its `skip_on:`/`only_on:` token: the first part of the
/// target name (`ios-uikit` → `ios`, `web-dom` → `web`, `harmony-arkui` → `harmony`).
fn gate_platform(target_name: &str) -> &str {
    target_name.split('-').next().unwrap_or(target_name)
}

/// Whether one `skip_on:`/`only_on:` token names this run: the target (`ios-uikit`), its
/// toolkit (`uikit`), its platform (`ios`, `android`, `macos`, `linux`, `windows`, `harmony`,
/// `web`), or the build flavor as `flavor:<name>` (`flavor:none` for the base app). The
/// platform form is what lets a step opt IN by where it applies, `only_on: [ios, android]`,
/// instead of listing every toolkit it does not.
///
/// `windows-winui` is the XAML backend built against WinUI 3 (docs/winui.md), so a gate naming
/// the XAML backend (`xaml`, `windows-xaml`) names it too: a step written for XAML's behavior is
/// WinUI's behavior as well. `winui` / `windows-winui` single the WinUI build out.
fn gate_names(token: &str, target: &crate::targets::Target, flavor_gate: &str) -> bool {
    let xaml_family = target.toolkit == "winui" && matches!(token, "xaml" | "windows-xaml");
    token == target.name
        || token == target.toolkit
        || token == gate_platform(target.name)
        || token == flavor_gate
        || xaml_family
}

/// Whether a gate token could name anything at all (some target, toolkit or platform in the
/// catalog, or a `flavor:` token), so a misspelling is reported rather than matching nothing.
fn gate_is_known(token: &str) -> bool {
    token.starts_with("flavor:")
        || crate::targets::TARGETS
            .iter()
            .any(|t| token == t.name || token == t.toolkit || token == gate_platform(t.name))
}

#[cfg(test)]
mod gate_tests {
    use super::{gate_is_known, gate_names};

    #[test]
    fn a_failed_prerequisite_stops_only_the_opted_in_flow() {
        use std::io::{BufRead, Write};
        for policy in ["stop", "continue"] {
            let root = std::env::temp_dir()
                .join(format!("day-flow-failure-{}-{policy}", std::process::id()));
            std::fs::create_dir_all(&root).unwrap();
            let script = root.join("flow.yaml");
            std::fs::write(&script, format!("on_failure: {policy}\nflow:\n- wait_for: {{ id: missing }}\n- tap: {{ id: must-not-run }}\n- screenshot: stale\n")).unwrap();
            let project = crate::meta::Project {
                root: root.clone(),
                manifest: toml::from_str("schema = 1\n[app]\nid = 'test.fixture'").unwrap(),
            };
            let stale = super::shot_dir(
                &project,
                crate::targets::find("macos-appkit").unwrap(),
                None,
                None,
                None,
            )
            .join("stale.png");
            std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
            std::fs::write(&stale, b"old fixture capture").unwrap();
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut seen = Vec::new();
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) => break,
                        // Windows can report a reset when the runner drops its socket after
                        // the flow. Only accept it between complete requests; the assertions
                        // below still require every expected step to have reached the server.
                        Err(e)
                            if e.kind() == std::io::ErrorKind::ConnectionReset
                                && line.is_empty() =>
                        {
                            break;
                        }
                        Ok(_) => {}
                        Err(e) => panic!("fixture request read failed: {e}"),
                    }
                    let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                    seen.push(request["step"]["op"].as_str().unwrap().to_string());
                    writeln!(
                        stream,
                        "{{\"ok\":false,\"retryable\":true,\"error\":\"fixture failure\"}}"
                    )
                    .unwrap();
                }
                seen
            });
            let run = super::run_scripts(
                &project,
                crate::targets::find("macos-appkit").unwrap(),
                port,
                "fixture-token",
                &[script],
                None,
                None,
                None,
                true,
                false,
                false,
            )
            .unwrap();
            let seen = server.join().unwrap();
            if policy == "stop" {
                assert_eq!(seen, ["wait_for"]);
                assert_eq!(run.steps_failed, 1);
                assert_eq!(run.retryable_failed, 1);
                assert_eq!(run.steps_aborted, 2);
                assert!(!stale.exists());
            } else {
                assert_eq!(seen, ["wait_for", "tap", "screenshot"]);
                assert_eq!(run.steps_failed, 3);
                assert_eq!(run.steps_aborted, 0);
            }
            std::fs::remove_dir_all(root).unwrap();
        }
        let root = std::path::Path::new("/fixture");
        assert!(super::parse_flow_text("on_failure: typo\nflow: []", root).is_err());
        assert!(
            !super::parse_flow_text("flow: []", root)
                .unwrap()
                .stop_on_failure
        );
    }

    #[test]
    fn windows_utf8_bom_preserves_script_text_and_project_expansion() {
        let script = "flow:\n  - input: { id: name, text: 'Français ${project}' }\n";
        let root = std::path::Path::new("C:/Showcase");
        let plain = super::parse_flow_text(script, root).unwrap();
        let bom = super::parse_flow_text(&format!("\u{feff}{script}"), root).unwrap();
        assert_eq!(bom, plain);
        assert_eq!(bom.steps[0].1["text"], "Français C:/Showcase");
    }

    /// A gate opts a step in or out by target, toolkit, platform or flavor; nothing else.
    #[test]
    fn a_gate_matches_the_target_its_toolkit_its_platform_or_the_flavor() {
        let ios = crate::targets::find("ios-uikit").expect("ios-uikit");
        let web = crate::targets::find("web-dom").expect("web-dom");
        let harmony = crate::targets::find("harmony-arkui").expect("harmony-arkui");
        for token in ["ios-uikit", "uikit", "ios", "flavor:none"] {
            assert!(gate_names(token, ios, "flavor:none"), "{token}");
        }
        for token in ["android", "mdc", "web-dom", "flavor:paid", "iOS", ""] {
            assert!(!gate_names(token, ios, "flavor:none"), "{token}");
        }
        assert!(gate_names("web", web, "flavor:none"));
        assert!(gate_names("harmony", harmony, "flavor:none"));
        assert!(gate_names("flavor:paid", ios, "flavor:paid"));
    }

    /// WinUI is the XAML backend: a gate naming XAML reaches it, a gate naming WinUI reaches
    /// only it.
    #[test]
    fn a_xaml_gate_names_the_winui_build_too() {
        let winui = crate::targets::find("windows-winui").expect("windows-winui");
        let xaml = crate::targets::find("windows-xaml").expect("windows-xaml");
        for token in ["xaml", "windows-xaml", "winui", "windows-winui", "windows"] {
            assert!(gate_names(token, winui, "flavor:none"), "{token}");
        }
        for token in ["winui", "windows-winui"] {
            assert!(!gate_names(token, xaml, "flavor:none"), "{token}");
        }
    }

    #[test]
    fn a_token_naming_nothing_is_a_typo() {
        for token in [
            "ios",
            "android",
            "harmony",
            "web",
            "arkui",
            "macos-appkit",
            "flavor:x",
        ] {
            assert!(gate_is_known(token), "{token}");
        }
        for token in ["iso", "iOS", "phone", "ohos", "flavr:x", ""] {
            assert!(!gate_is_known(token), "{token}");
        }
    }
}

#[cfg(test)]
mod terminate_tests {
    use super::desktop_pattern;
    use std::path::Path;

    /// The pattern is anchored at the build root the launch used, so it matches both desktop
    /// layouts under it and nothing under another root. A flavor builds under
    /// `build/day/flavors/<flavor>/`; when the pattern missed that, the old app survived
    /// `terminate`, kept the engine port, and answered the next variant's steps out of the
    /// previous run's state.
    #[test]
    fn the_pattern_is_anchored_at_the_root_the_launch_built_into() {
        assert_eq!(
            desktop_pattern(Path::new("/w/App/build/day"), "macos-appkit"),
            "^/w/App/build/day/(cargo/)?macos-appkit/"
        );
        assert_eq!(
            desktop_pattern(
                Path::new("/w/App/build/day/flavors/appfair"),
                "macos-appkit"
            ),
            "^/w/App/build/day/flavors/appfair/(cargo/)?macos-appkit/"
        );
        assert_eq!(
            desktop_pattern(
                Path::new("/w/App/build/day/day-src/main-2d77edbf"),
                "linux-gtk"
            ),
            "^/w/App/build/day/day-src/main-2d77edbf/(cargo/)?linux-gtk/"
        );
    }

    /// A checkout path with regex punctuation in it stays a literal.
    #[test]
    fn a_path_with_regex_punctuation_matches_itself() {
        assert_eq!(
            desktop_pattern(Path::new("/w/App (2)/build/day"), "linux-gtk"),
            "^/w/App \\(2\\)/build/day/(cargo/)?linux-gtk/"
        );
    }
}

#[cfg(test)]
mod port_tests {
    use super::pick_port;

    /// The port handed to a launch must be bindable right now, which is what the probe checks.
    /// Holding the first pick open proves the next pick walks past it instead of colliding.
    #[test]
    fn pick_port_returns_a_bindable_port_and_walks_past_a_taken_one() {
        let port = pick_port(0);
        let held = std::net::TcpListener::bind(("127.0.0.1", port))
            .expect("pick_port said this port was free");
        let next = pick_port(0);
        assert_ne!(next, port, "the probe must skip the port we hold");
        let _also_free = std::net::TcpListener::bind(("127.0.0.1", next))
            .expect("the second pick must be free too");
        drop(held);
    }

    /// Every port this can hand out must sit below Linux's ephemeral floor (32768). Inside that
    /// range the kernel gives the number to outbound connections on its own, and the engine's
    /// `bind` inside an emulator then fails with EADDRINUSE for the whole session, which the
    /// host-side probe cannot predict, because it is a different machine. The `+ 100` is the walk
    /// `pick_port` may do past busy ports, and `index` is one per target in a multi-target launch.
    #[test]
    fn every_pick_stays_below_the_ephemeral_range() {
        const EPHEMERAL_FLOOR: u16 = 32768;
        // The widest pid and index this can see, beyond today's process.
        let worst = super::ENGINE_PORT_BASE + 8999 + 64 + 100;
        assert!(
            worst < EPHEMERAL_FLOOR,
            "pick_port can reach {worst}, which is inside the ephemeral range ({EPHEMERAL_FLOOR}+)"
        );
        assert!(pick_port(0) < EPHEMERAL_FLOOR);
    }
}

/// A minimal standalone base64 decoder: dayscript replies (screenshots, a11y dumps) come back
/// base64-encoded. Inlined here so the CLI needn't pull in `day-script` (and its whole runtime
/// graph: day-core/reactive/pieces/fluent/l10n) for one small function; `day-script` keeps its
/// own copy for the app side.
mod day_script_b64 {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn b64encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
            out.push(B64[(n >> 18) as usize & 63] as char);
            out.push(B64[(n >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 {
                B64[(n >> 6) as usize & 63] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                B64[n as usize & 63] as char
            } else {
                '='
            });
        }
        out
    }

    pub fn b64decode(s: &str) -> Vec<u8> {
        let val = |c: u8| B64.iter().position(|&x| x == c).unwrap_or(0) as u32;
        let bytes: Vec<u8> = s.bytes().filter(|&c| c != b'\n' && c != b'\r').collect();
        let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
        for chunk in bytes.chunks(4) {
            if chunk.len() < 4 {
                break;
            }
            let pad = chunk.iter().filter(|&&c| c == b'=').count();
            let n = (val(chunk[0]) << 18)
                | (val(chunk[1]) << 12)
                | (val(if chunk[2] == b'=' { b'A' } else { chunk[2] }) << 6)
                | val(if chunk[3] == b'=' { b'A' } else { chunk[3] });
            out.push((n >> 16) as u8);
            if pad < 2 {
                out.push((n >> 8) as u8);
            }
            if pad < 1 {
                out.push(n as u8);
            }
        }
        out
    }
}

#[cfg(test)]
mod window_tests {
    use super::{STARTUP_SECS, first_read_window, reply_window};
    use std::time::Duration;

    /// The first reply also waits out the app's startup; later replies keep the step's window,
    /// and a longer window (a slow device, a long step budget) is never shortened.
    #[test]
    fn first_read_window_covers_startup_without_shortening_a_longer_window() {
        // Pass the override explicitly so the runner's environment cannot alter this test.
        let normal = reply_window(20, 5.0, None);
        assert_eq!(normal, Duration::from_secs(45));
        assert_eq!(first_read_window(normal), Duration::from_secs(STARTUP_SECS));
        for (connect_secs, budget_secs, main_override, expected_secs) in [
            (120, 5.0, None, 120),
            (20, 90.0, None, 190),
            (20, 5.0, Some("90"), 105),
        ] {
            let window = reply_window(connect_secs, budget_secs, main_override);
            assert_eq!(window, Duration::from_secs(expected_secs));
            assert_eq!(first_read_window(window), window);
        }
    }
}

#[cfg(all(test, unix))]
mod harmony_capture_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn acknowledged_identical_captures_do_not_sleep_or_retry() {
        const CHILD: &str = "DAY_TEST_HARMONY_CAPTURE";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = PathBuf::from(root);
            let target = crate::targets::find("harmony-arkui").unwrap();
            for name in ["first.png", "identical.png"] {
                device_screenshot(target, &root.join(name), true).unwrap();
            }
            assert_eq!(
                std::fs::read(root.join("first.png")).unwrap(),
                std::fs::read(root.join("identical.png")).unwrap()
            );
            assert_eq!(
                std::fs::read_to_string(root.join("captures")).unwrap(),
                "capture\ncapture\n"
            );
            return;
        }
        let root =
            std::env::temp_dir().join(format!("day-harmony-capture-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let hdc = root.join("hdc");
        std::fs::write(
            &hdc,
            r#"#!/bin/sh
case "$3 $4" in
  "shell uitest") echo capture >> "$DAY_TEST_HARMONY_CAPTURE/captures" ;;
  "file recv") printf 'identical screenshot' > "$6" ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&hdc, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "script::harmony_capture_tests::acknowledged_identical_captures_do_not_sleep_or_retry"])
            .env(CHILD, &root).env("PATH", &root).env("DAY_OHOS_TARGET", "fake-device")
            // An acknowledged app must bypass even a very large legacy override.
            .env("DAY_OHOS_SHOT_SETTLE_MS", "60000")
            .spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("acknowledged captures waited for the legacy delay");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

// ---------------------------------------------------------------------------
// Tests (docs/testing.md): the `tests` listing and the `run_tests` report
// ---------------------------------------------------------------------------

/// Print the registry the `tests` step answered, one test per line.
fn print_test_listing(reply: &serde_json::Value) {
    let Some(list) = reply.get("data").and_then(|d| d.as_array()) else {
        return;
    };
    for t in list {
        let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let kind = t.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let proves: Vec<&str> = t
            .get("proves")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|p| p.as_str()).collect())
            .unwrap_or_default();
        eprintln!("      {name:<32} {kind:<9} {}", proves.join(" "));
    }
}

/// Print one line per test from a `run_tests` report; returns how many failed.
fn print_test_report(report: &serde_json::Value) -> usize {
    let Some(tests) = report.get("tests").and_then(|t| t.as_array()) else {
        return 0;
    };
    let (mut passed, mut failed, mut skipped) = (0usize, 0usize, 0usize);
    for t in tests {
        let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let verdict = t.get("verdict").and_then(|v| v.as_str()).unwrap_or("?");
        let ms = t.get("ms").and_then(|v| v.as_u64()).unwrap_or(0);
        let dots = ".".repeat(40usize.saturating_sub(name.len()));
        match verdict {
            "pass" => {
                passed += 1;
                eprintln!("      {name} {dots} {SUCCESS}ok{SUCCESS:#}        ({ms} ms)");
            }
            "skip" => {
                skipped += 1;
                let reason = t.get("reason").and_then(|v| v.as_str()).unwrap_or("");
                eprintln!("      {name} {dots} {WARN}skipped{WARN:#}   {reason}");
            }
            _ => {
                failed += 1;
                let message = t.get("message").and_then(|v| v.as_str()).unwrap_or("");
                eprintln!("      {name} {dots} {ERROR}FAILED{ERROR:#}    {message}");
            }
        }
    }
    eprintln!(
        "      {} tests: {passed} passed, {skipped} skipped, {failed} failed",
        tests.len()
    );
    failed
}

/// Write `evidence.json` beside the captures and each test's shots under `tests/<test>/`
/// (docs/testing.md). The file is the contract the website reads; its shape is documented
/// there, and the shots' PNGs are stripped from it (they are the files).
fn write_evidence(
    dir: &Path,
    target: &Target,
    variant: Option<&str>,
    device: Option<&str>,
    report: &serde_json::Value,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut tests = serde_json::Map::new();
    for t in report
        .get("tests")
        .and_then(|t| t.as_array())
        .into_iter()
        .flatten()
    {
        let Some(name) = t.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let mut entry = t.clone();
        if let Some(m) = entry.as_object_mut() {
            m.remove("name");
        }
        tests.insert(name.to_owned(), entry);
    }
    for shot in report
        .get("shots")
        .and_then(|s| s.as_array())
        .into_iter()
        .flatten()
    {
        let (Some(test), Some(name), Some(png)) = (
            shot.get("test").and_then(|v| v.as_str()),
            shot.get("name").and_then(|v| v.as_str()),
            shot.get("png_base64").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        let shot_dir = dir.join("tests").join(test);
        std::fs::create_dir_all(&shot_dir).map_err(|e| e.to_string())?;
        let path = shot_dir.join(format!("{name}.png"));
        std::fs::write(&path, day_script_b64::b64decode(png)).map_err(|e| e.to_string())?;
        if std::env::var_os("DAY_SCREENSHOT_RAW").is_none() {
            let _ = crate::screenshot::normalize_capture(&path);
        }
    }
    // What the run was: the commit and run id come from the CI environment when there is
    // one, and are absent on a laptop rather than guessed.
    let env = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
    let evidence = serde_json::json!({
        "schema": 1,
        "target": target.name,
        "device": device,
        "variant": variant.unwrap_or("default"),
        "day": env!("DAY_VERSION_LONG"),
        "commit": env("GITHUB_SHA"),
        "run": env("GITHUB_RUN_ID"),
        "at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "tests": tests,
    });
    let path = dir.join("evidence.json");
    let text = serde_json::to_string_pretty(&evidence).map_err(|e| e.to_string())?;
    std::fs::write(&path, text + "\n").map_err(|e| e.to_string())?;
    Ok(path)
}
