//! The dayscript runner (DESIGN.md §14, §16.5): launches the app with the engine invited
//! (token + runner-chosen port — the port-0 handshake-file refinement is post-MVP), connects
//! over TCP (adb-forwarded on Android), executes the YAML flow, saves screenshots, prints
//! per-step results, and returns exit code 5 on assertion failure.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::meta::Project;
use crate::targets::{Target, TargetKind};

pub struct ScriptRun {
    pub steps_total: usize,
    pub steps_failed: usize,
    pub screenshots: Vec<PathBuf>,
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

fn connect(port: u16) -> Result<TcpStream, String> {
    for _ in 0..60 {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            s.set_read_timeout(Some(Duration::from_secs(20))).ok();
            return Ok(s);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!(
        "could not connect to the dayscript engine on 127.0.0.1:{port}"
    ))
}

fn shot_dir(project: &Project, target: &Target, locale: Option<&str>) -> PathBuf {
    project
        .root
        .join("build/day/screenshots")
        .join(target.name)
        .join(locale.unwrap_or("default"))
}

/// Device-level capture fallback for targets whose in-process snapshot is unsupported.
fn device_screenshot(target: &Target, path: &Path) -> Result<(), String> {
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
        TargetKind::Desktop => Err("desktop snapshot returned unsupported".into()),
        TargetKind::HarmonyOs => {
            // `uitest screenCap` writes a real PNG; `snapshot_display` writes JPEG (so its bytes in a
            // .png file are wrong) — prefer uitest, fall back to snapshot_display. Then `hdc file recv`.
            let dev = "/data/local/tmp/day-shot.png";
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
            crate::ohos::hdc()
                .args(["file", "recv", dev])
                .arg(path)
                .status()
                .map_err(|e| e.to_string())
                .and_then(|s| {
                    if s.success() {
                        Ok(())
                    } else {
                        Err("hdc file recv failed".into())
                    }
                })
        }
    }
}

pub fn run_scripts(
    project: &Project,
    target: &'static Target,
    port: u16,
    token: &str,
    scripts: &[PathBuf],
    locale: Option<&str>,
) -> Result<ScriptRun, String> {
    if target.kind == TargetKind::Android {
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
    if target.kind == TargetKind::HarmonyOs {
        // hdc's `adb forward` equivalent: host tcp:port → the app's tcp:port on the emulator/device,
        // so `connect(port)` below reaches the in-app dayscript engine over the networked target.
        let _ = crate::ohos::hdc()
            .args(["fport", &format!("tcp:{port}"), &format!("tcp:{port}")])
            .status();
    }
    let mut stream = connect(port)?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);

    // adb-forwarded ports accept host connections BEFORE the device listener exists; a
    // request/reply that hits EOF reconnects and retries within a bounded window.
    let roundtrip = |stream: &mut TcpStream,
                     reader: &mut BufReader<TcpStream>,
                     line: &str|
     -> Result<String, String> {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
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
                        s.set_read_timeout(Some(Duration::from_secs(20))).ok();
                        *reader = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
                        *stream = s;
                    }
                }
                Err(e) => return Err(format!("engine connection lost: {e}")),
            }
        }
    };

    let dir = shot_dir(project, target, locale);
    let _ = std::fs::create_dir_all(&dir);

    let mut run = ScriptRun {
        steps_total: 0,
        steps_failed: 0,
        screenshots: Vec::new(),
    };
    for script in scripts {
        let steps = parse_flow(script)?;
        eprintln!(
            "\x1b[1m     Script\x1b[0m {} on {} ({} steps)",
            script.display(),
            target.name,
            steps.len()
        );
        for (op, step) in steps {
            run.steps_total += 1;
            // `pause` sleeps runner-side (the engine must not block the UI thread).
            if op == "pause" {
                let secs = step.get("secs").and_then(|v| v.as_f64()).unwrap_or(0.5);
                std::thread::sleep(Duration::from_secs_f64(secs));
                eprintln!("  \x1b[32m✓\x1b[0m pause {secs}s");
                continue;
            }
            let req = serde_json::json!({"token": token, "step": step});
            let mut line = serde_json::to_string(&req).unwrap();
            line.push('\n');
            let reply_line = roundtrip(&mut stream, &mut reader, &line)?;
            let reply: serde_json::Value =
                serde_json::from_str(reply_line.trim()).map_err(|e| e.to_string())?;
            let ok = reply.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let detail = step
                .get("id")
                .and_then(|v| v.as_str())
                .or_else(|| step.get("name").and_then(|v| v.as_str()))
                .unwrap_or("");
            if ok {
                eprintln!("  \x1b[32m✓\x1b[0m {op} {detail}");
            } else {
                run.steps_failed += 1;
                let err = reply
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("failed");
                eprintln!("  \x1b[31m✗\x1b[0m {op} {detail} — {err}");
            }
            if op == "screenshot" && ok {
                let name = step.get("name").and_then(|v| v.as_str()).unwrap_or("shot");
                let path = dir.join(format!("{name}.png"));
                if let Some(b64) = reply.get("png_base64").and_then(|v| v.as_str()) {
                    let bytes = day_script_b64::b64decode(b64);
                    let _ = std::fs::write(&path, bytes);
                    run.screenshots.push(path);
                } else if reply
                    .get("screenshot_unsupported")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    match device_screenshot(target, &path) {
                        Ok(()) => run.screenshots.push(path),
                        Err(e) => eprintln!("    (device screenshot failed: {e})"),
                    }
                }
            }
        }
    }
    // Terminate the app now that the run is over.
    terminate(project, target);
    Ok(run)
}

fn terminate(project: &Project, target: &Target) {
    match target.kind {
        TargetKind::Desktop if cfg!(windows) => {
            // No pkill on Windows; kill the app by image name (taskkill is on every runner).
            let _ = Command::new("taskkill")
                .args(["/F", "/IM", &format!("{}.exe", project.manifest.app.name)])
                .status();
        }
        TargetKind::Desktop => {
            let _ = Command::new("pkill")
                .args([
                    "-f",
                    &format!("cargo/{}.*{}", target.name, project.manifest.app.name),
                ])
                .status();
        }
        TargetKind::IosSim => {
            let _ = Command::new("xcrun")
                .args(["simctl", "terminate", "booted", &project.manifest.app.id])
                .status();
        }
        TargetKind::Android => {
            let _ = Command::new("adb")
                .args(["shell", "am", "force-stop", &project.manifest.app.id])
                .status();
        }
        TargetKind::HarmonyOs => {
            let _ = crate::ohos::hdc()
                .args(["shell", "aa", "force-stop", &project.manifest.app.id])
                .status();
        }
    }
}

pub fn pick_port(index: usize) -> u16 {
    34100 + (std::process::id() % 900) as u16 + index as u16
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

/// Local re-export point so this module owns no base64 logic.
mod day_script_b64 {
    pub use day_script::b64decode;
}
