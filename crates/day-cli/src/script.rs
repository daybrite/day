// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The dayscript runner (DESIGN.md §14, §16.5): launches the app with the engine invited
//! (token + runner-chosen port — the port-0 handshake-file refinement is post-MVP), connects
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
    pub steps_failed: usize,
    /// How many of `steps_failed` the engine marked RETRYABLE — an element not realized yet,
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
    /// never came up). `steps_failed` counts failures seen BEFORE the loss — a loss with ZERO
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
fn parse_flow(path: &Path) -> Result<Vec<(String, serde_json::Value)>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc: serde_json::Value = serde_norway::from_str(&text).map_err(|e| e.to_string())?;
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
            serde_json::Value::Null => {}
            other => {
                return Err(format!("step {op}: unsupported params {other}"));
            }
        }
        steps.push((op.clone(), serde_json::Value::Object(step)));
    }
    Ok(steps)
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
/// `DAYSCRIPT_CONNECT_SECS`; the default is per-target — 20 s for local targets, 120 s for
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

/// How long the runner waits for one step's reply: the connect window (which a slow device
/// bumps — HarmonyOS uses 120 s), but never less than the step's own implicit-wait budget plus
/// headroom. The engine answers a retryable step only after polling for the whole budget, so an
/// equal timeout is already a race; the headroom covers the reply's trip back.
fn read_window(window_secs: u64, budget_secs: f64) -> Duration {
    let floor = Duration::from_secs(window_secs.max(20));
    Duration::from_secs_f64(budget_secs + 10.0).max(floor)
}

pub(crate) fn connect(port: u16, window_secs: u64) -> Result<TcpStream, String> {
    let attempts = window_secs * 4; // 250 ms apart
    for _ in 0..attempts {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            // A floor for the handshake only — `roundtrip` resets this per step from the
            // step's own wait budget.
            s.set_read_timeout(Some(read_window(window_secs, 0.0))).ok();
            return Ok(s);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!(
        "could not connect to the dayscript engine on 127.0.0.1:{port}"
    ))
}

/// Where a run's screenshots land: `build/day/screenshots/<target>/<subdir>/`. The subdir is
/// the `--variant` name when given (themed/localized capture sets: light / dark / fr), else
/// the locale, else "default".
fn shot_dir(
    project: &Project,
    target: &Target,
    locale: Option<&str>,
    variant: Option<&str>,
) -> PathBuf {
    project
        .root
        .join("build/day/screenshots")
        .join(target.name)
        .join(variant.or(locale).unwrap_or("default"))
}

/// Device-level capture fallback for targets whose in-process snapshot is unsupported.
/// `prev` is the run's previous capture, when there is one: on HarmonyOS a shot that comes out
/// byte-identical to it is treated as a stale frame and re-captured (see the arm's comment).
fn device_screenshot(target: &Target, path: &Path, prev: Option<&Path>) -> Result<(), String> {
    match target.kind {
        TargetKind::IosSim => {
            // The scripted run drives one simulator (the first booted); pin it so multiple booted
            // sims don't make `simctl … booted` ambiguous.
            let udid = crate::mobile::booted_sims()
                .into_iter()
                .next()
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
            // Pin the first device (the one the runner forwarded to), else `adb` errors with
            // several attached.
            let mut cmd = Command::new("adb");
            if let Some(dev) = crate::mobile::android_devices().first() {
                cmd.args(["-s", &dev.serial]);
            }
            let out = cmd
                .args(["exec-out", "screencap", "-p"])
                .output()
                .map_err(|e| e.to_string())?;
            std::fs::write(path, &out.stdout).map_err(|e| e.to_string())
        }
        TargetKind::Desktop => {
            // Engine (in-process) snapshot unavailable — on an X11 session (the CI linux legs run
            // under xvfb) capture the root window with ImageMagick's `import`: with the xvfb
            // screen sized to the app window (ci.yml passes `-screen 0 1000x720x24`) the root IS
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
            // `uitest screenCap` writes a real PNG; `snapshot_display` writes JPEG (so its bytes in a
            // .png file are wrong) — prefer uitest, fall back to snapshot_display. Then `hdc file recv`.
            // Re-wake the display first (best-effort): a sleeping screen captures as a black frame.
            let _ = crate::ohos::hdc()
                .args(["shell", "power-shell", "wakeup"])
                .status();
            // The TCG guest's compositor lags the UI thread: `ui_idle` returns once the pushed
            // page has reported its first area (laid out), but screenCap serves the PREVIOUS
            // frame until RenderService composites the new one — measured at 2-3s on the
            // cross-arch emulator (every shot trails one page without this settle). The first
            // push after app start can lag longer still (>6s: first render-tree build), so a
            // capture that comes out byte-identical to the run's previous screenshot is treated
            // as that stale frame and retried; the last attempt is accepted either way, which
            // keeps scripts with genuinely identical consecutive shots slow-but-correct.
            let settle = std::env::var("DAY_OHOS_SHOT_SETTLE_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4000);
            let dev = "/data/local/tmp/day-shot.png";
            for attempt in 0..4u32 {
                std::thread::sleep(Duration::from_millis(if attempt == 0 {
                    settle
                } else {
                    3000
                }));
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
                    return Err(
                        "hdc screenshot failed (uitest screenCap / snapshot_display)".into(),
                    );
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
                let stale = prev.is_some_and(|p| {
                    std::fs::read(p)
                        .ok()
                        .zip(std::fs::read(path).ok())
                        .is_some_and(|(a, b)| a == b)
                });
                if !stale {
                    break;
                }
            }
            Ok(())
        }
    }
}

/// Reach the in-app dayscript engine from the host: device targets need a TCP forward
/// (adb / hdc); desktop and the iOS simulator answer on loopback directly.
/// Public seams for `day drive` (drive.rs): the same primitives run_scripts uses.
pub(crate) fn b64decode_public(s: &str) -> Vec<u8> {
    day_script_b64::b64decode(s)
}
pub(crate) fn b64encode_public(bytes: &[u8]) -> String {
    day_script_b64::b64encode(bytes)
}
pub(crate) fn device_screenshot_public(target: &Target, path: &Path) -> Result<(), String> {
    device_screenshot(target, path, None)
}

pub(crate) fn forward_engine(kind: TargetKind, port: u16) {
    if kind == TargetKind::Android {
        // The dayscript runner drives ONE device; with several attached, `adb forward` (no `-s`)
        // errors ("more than one device"), so pin the first enumerated device.
        let mut cmd = Command::new("adb");
        if let Some(dev) = crate::mobile::android_devices().first() {
            cmd.args(["-s", &dev.serial]);
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
    keep_alive: bool,
    attached: bool,
) -> Result<ScriptRun, ScriptError> {
    forward_engine(target.kind, port);
    let window_secs = connect_window_secs(target.kind);
    // A connect failure IS an engine loss (the app died during startup, or never bound) — the
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

    // adb-forwarded ports accept host connections BEFORE the device listener exists; a
    // request/reply that hits EOF reconnects and retries within a bounded window.
    //
    // `budget` is the step's own implicit-wait budget (§14.3). The engine polls a retryable step
    // on the main thread for that long before answering, so the runner must out-wait it: sizing
    // the socket read from `window_secs` alone made any step declaring a longer `timeout_secs`
    // time out runner-side FIRST and report "engine connection lost" — a healthy, idle app
    // mislabeled as a dead one.
    let roundtrip = |stream: &mut TcpStream,
                     reader: &mut BufReader<TcpStream>,
                     line: &str,
                     budget: f64|
     -> Result<String, String> {
        let window = read_window(window_secs, budget);
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
                Ok(r) => return Ok(r),
                Err(e) if std::time::Instant::now() < deadline => {
                    let _ = e;
                    std::thread::sleep(Duration::from_millis(500));
                    if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
                        s.set_read_timeout(Some(window)).ok();
                        *reader = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
                        *stream = s;
                    }
                }
                Err(e) => return Err(e),
            }
        }
    };

    let dir = shot_dir(project, target, locale, variant);
    let _ = std::fs::create_dir_all(&dir);

    let mut run = ScriptRun {
        steps_total: 0,
        steps_failed: 0,
        retryable_failed: 0,
        screenshots: Vec::new(),
    };
    // Captures this run saved, for the per-target gallery index (screenshot.rs §14.7).
    let mut index_entries: Vec<crate::screenshot::TargetEntry> = Vec::new();
    for script in scripts {
        let steps = parse_flow(script).map_err(ScriptError::Other)?;
        // `expect_exit` tolerates the app dying, so it must be terminal — a step after it could
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
        for (op, step) in steps {
            run.steps_total += 1;
            // The target gates run BEFORE the runner-side steps below (`pause`, `expect_exit`):
            // those `continue` on their own, so evaluating them first made a gated `pause` sleep
            // on every target regardless of its `only_on`/`skip_on` — 10s a variant on Android
            // and 21s on HarmonyOS, spent waiting for blocks those targets never run.
            //
            // `skip_on:` — a per-step target filter: the step is dropped on the named targets
            // or toolkits (`skip_on: [web-dom]`), so ONE walkthrough stays honest across
            // platforms with genuinely absent capabilities (docs/agent.md).
            if let Some(skips) = step.get("skip_on").and_then(|v| v.as_array()) {
                let hit = skips
                    .iter()
                    .filter_map(|v| v.as_str())
                    .any(|s| s == target.name || s == target.toolkit);
                if hit {
                    eprintln!("  {WARN}–{WARN:#} {op} (skipped on {})", target.name);
                    continue;
                }
            }
            // `only_on:` — skip_on's mirror, for a step whose expectations are per-target (an
            // `assert_no_placeholders` allow-list differs sharply between, say, appkit and
            // web-dom, so the script carries one step per target group).
            if let Some(onlys) = step.get("only_on").and_then(|v| v.as_array()) {
                let hit = onlys
                    .iter()
                    .filter_map(|v| v.as_str())
                    .any(|s| s == target.name || s == target.toolkit);
                if !hit {
                    eprintln!("  {WARN}\u{2013}{WARN:#} {op} (not for {})", target.name);
                    continue;
                }
            }
            let mut step = step;
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
            // `expect_exit` is runner-side: a prior step triggered an intentional exit/crash, so
            // here we WANT the connection to drop. Probe until it does (success) or the window
            // elapses (the app survived — failure). Never sent to the engine.
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
                // is the RUNNER's: it feeds the per-target gallery.json below and must not
                // reach the engine — apps predate it and never need it.
                if op == "screenshot" {
                    shot_meta = crate::screenshot::extract_meta(map);
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
                .unwrap_or(0.0);
            // A roundtrip that gives up reconnecting means the app process is gone — carry
            // the failure count seen so far, so the caller can tell a clean-run flake (retry)
            // from a failing run that then died (report).
            let failed_before = run.steps_failed;
            let reply_line =
                roundtrip(&mut stream, &mut reader, &line, budget).map_err(|detail| {
                    ScriptError::EngineLost {
                        steps_failed: failed_before,
                        detail,
                    }
                })?;
            let reply: serde_json::Value = serde_json::from_str(reply_line.trim())
                .map_err(|e| ScriptError::Other(e.to_string()))?;
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
            if op == "screenshot" && ok {
                let name = step.get("name").and_then(|v| v.as_str()).unwrap_or("shot");
                let path = dir.join(format!("{name}.png"));
                let in_process = reply
                    .get("png_base64")
                    .and_then(|v| v.as_str())
                    .map(day_script_b64::b64decode);
                let write_in_process = |bytes: &[u8]| std::fs::write(&path, bytes).is_ok();

                // On a device or simulator the DEVICE capture stays the source of truth. It is the
                // whole screen — status bar, home indicator, system chrome — which is what every
                // published mobile gallery shot shows; the in-process capture frames the app's own
                // view tree alone, so preferring it would silently re-crop every one of them.
                //
                // The mobile backends that grew an in-process capture (docs/window-image.md) are
                // the FALLBACK instead: a refusing device tool used to abandon the shot outright.
                // Desktop is the other way round — the in-process render is the real capture there
                // and `device_screenshot` has no desktop arm at all.
                let prev = run.screenshots.last().cloned();
                let mut saved = false;
                if device_first {
                    match device_screenshot(target, &path, prev.as_deref()) {
                        Ok(()) => saved = true,
                        Err(e) => {
                            // The payload was skipped on purpose, so fetch it now — this arm
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
                        match device_screenshot(target, &path, prev.as_deref()) {
                            Ok(()) => saved = true,
                            Err(e) => eprintln!("    (desktop screenshot failed: {e})"),
                        }
                    }
                }
                if saved {
                    // Record the capture in the target's gallery index (screenshot.rs): the
                    // step's metadata plus the saved file's facts. The subdir name is the
                    // variant key the published index uses.
                    let vname = variant.or(locale).unwrap_or("default");
                    if let Some(entry) = crate::screenshot::target_entry(
                        &path,
                        vname,
                        name,
                        locale,
                        Some(&shot_meta),
                    ) {
                        index_entries.push(entry);
                    }
                    run.screenshots.push(path);
                }
            }
        }
    }
    if keep_alive {
        // Interactive script development (docs/agent.md): leave the app running — its session
        // stays drivable (`day drive`), so scripts can be built and debugged incrementally.
        // Attached: `day` stays in the foreground streaming the app's console output until the
        // app exits or the run is stopped. Detached: `day` exits now and the app lives on.
        //
        // A device app is stopped by a registered command rather than by dying with its parent
        // (signals.rs), and the exit path runs those unconditionally — which would take down the
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
    // run's captures into the target's machine-readable gallery index (screenshot.rs) —
    // `day screenshot index` merges those per-target files into the published gallery.json.
    if !index_entries.is_empty() {
        crate::screenshot::record_target_entries(
            &project.root.join("build/day/screenshots"),
            target.name,
            index_entries,
        );
    }
    if !run.screenshots.is_empty() {
        write_gallery(&project.root.join("build/day/screenshots"));
    }
    Ok(run)
}

/// Regenerate `build/day/screenshots/index.html`: one labeled thumbnail per capture, grouped
/// by `<target>/<variant>`, each linking to the full-size image — a quick browsable index of
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
        for variant in dirs(&target) {
            let vname = variant
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
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

pub(crate) fn terminate(project: &Project, target: &Target) {
    match target.kind {
        TargetKind::Desktop if cfg!(windows) => {
            // No pkill on Windows; kill the app by image name (taskkill is on every runner).
            let _ = Command::new("taskkill")
                .args(["/F", "/IM", &format!("{}.exe", project.manifest.app.name)])
                .status();
        }
        TargetKind::Desktop => {
            // Match the launch DIRECTORY, never the app name. Two layouts have to be covered,
            // because macos-appkit now builds through a scaffolded Xcode host project (§17.4)
            // while every other desktop target is still a bare cargo binary:
            //
            //   <root>/build/day/cargo/<target>/<profile>/<name>                     cargo
            //   <root>/build/day/<target>/<config>/<Name>.app/Contents/MacOS/<Name>  xcodebuild
            //
            // and the executable's NAME is not common ground between them: `app.name` is the
            // crate name (`day-skies`), while an Xcode bundle's binary is named by the pbxproj's
            // PRODUCT_NAME (`DaySkies`). A pattern built from `app.name` matches nothing at all
            // under the second layout. The directory is the one thing both agree on, and it is
            // also what makes this project-specific — two checkouts building the same target
            // would otherwise terminate each other's apps.
            //
            // Getting this wrong is not a leaked process so much as a corrupted run: the
            // survivor holds the dayscript engine's port, the NEXT launch cannot bind, and the
            // runner then drives the OLD app — which shares the run's token and answers every
            // step, so a locale sweep quietly re-photographs the first locale.
            let root = ere_escape(&project.root.to_string_lossy());
            let pattern = format!("^{root}/build/day/(cargo/)?{}/", target.name);
            let _ = Command::new("pkill").args(["-f", &pattern]).status();
            // `pkill` only DELIVERS the signal; the app still has to run its own teardown, and
            // it holds the engine port until it does. Returning here would hand the next launch
            // a port that is still bound — the same corrupted run as above, reached by a race
            // instead of by a bad pattern. So wait for the process table to actually clear, and
            // escalate to SIGKILL for an app that will not go on its own.
            if !await_exit(&pattern, Duration::from_secs(10)) {
                let _ = Command::new("pkill").args(["-9", "-f", &pattern]).status();
                let _ = await_exit(&pattern, Duration::from_secs(5));
            }
        }
        // The three device branches are bounded (`status_within`): each talks to a device over a
        // tool that waits for an unresponsive one indefinitely, and this runs BETWEEN a matrix
        // run's variants — so a wedged emulator here stops the run rather than the app.
        TargetKind::IosSim => {
            let _ = crate::ops::status_within(
                Command::new("xcrun").args([
                    "simctl",
                    "terminate",
                    "booted",
                    &project.manifest.app.id,
                ]),
                DEVICE_CMD,
            );
        }
        TargetKind::Android => {
            let _ = crate::ops::status_within(
                Command::new("adb").args(["shell", "am", "force-stop", &project.manifest.app.id]),
                DEVICE_CMD,
            );
        }
        TargetKind::HarmonyOs => {
            let _ = crate::ops::status_within(
                crate::ohos::hdc().args(["shell", "aa", "force-stop", &project.manifest.app.id]),
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

/// Whether this target's device still answers — asked after the engine is lost, to decide
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
            let mut c = Command::new("adb");
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
/// actually free — the arithmetic alone handed out ports something else already held. Falls
/// back to the base when the whole range is busy (the old behavior: let the launch report it).
pub fn pick_port(index: usize) -> u16 {
    let base = 34100 + (std::process::id() % 900) as u16 + index as u16;
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

#[cfg(test)]
mod port_tests {
    use super::pick_port;

    /// The port handed to a launch must be bindable right now — the probe is the point. Holding
    /// the first pick open proves the next pick walks past it instead of colliding.
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
}

/// A minimal standalone base64 decoder — dayscript replies (screenshots, a11y dumps) come back
/// base64-encoded. Inlined here so the CLI needn't pull in `day-script` (and its whole runtime graph:
/// day-core/reactive/pieces/fluent/l10n) for one small function; `day-script` keeps its own copy for
/// the app side.
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
