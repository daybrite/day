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

/// How long to keep (re)trying the engine connection, in seconds. Override with
/// `DAYSCRIPT_CONNECT_SECS`; the default is per-target — 20 s for local targets, 120 s for
/// HarmonyOS, whose software-emulated (TCG) guest can spend minutes between `aa start` and the
/// app-side engine binding its socket (and whose forwarded hdc channel drops with transient
/// connection resets that the roundtrip retry below rides out).
fn connect_window_secs(kind: TargetKind) -> u64 {
    std::env::var("DAYSCRIPT_CONNECT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(match kind {
            TargetKind::HarmonyOs => 120,
            _ => 20,
        })
}

fn connect(port: u16, window_secs: u64) -> Result<TcpStream, String> {
    let attempts = window_secs * 4; // 250 ms apart
    for _ in 0..attempts {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            s.set_read_timeout(Some(Duration::from_secs(window_secs.max(20))))
                .ok();
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
        TargetKind::HarmonyOs => {
            // `uitest screenCap` writes a real PNG; `snapshot_display` writes JPEG (so its bytes in a
            // .png file are wrong) — prefer uitest, fall back to snapshot_display. Then `hdc file recv`.
            // Re-wake the display first (best-effort): a sleeping screen captures as a black frame.
            let _ = crate::ohos::hdc()
                .args(["shell", "power-shell", "wakeup"])
                .status();
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
    variant: Option<&str>,
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
        // hdc's `adb forward` equivalent: host tcp:port → the app's tcp:port on the launched
        // target, so `connect(port)` below reaches the in-app dayscript engine (docs/harmonyos.md;
        // pinned to the discovered device + retried through hdc server recycles in ohos.rs).
        crate::ohos::fport_engine(port);
    }
    let window_secs = connect_window_secs(target.kind);
    let mut stream = connect(port, window_secs)?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);

    // adb-forwarded ports accept host connections BEFORE the device listener exists; a
    // request/reply that hits EOF reconnects and retries within a bounded window.
    let roundtrip = |stream: &mut TcpStream,
                     reader: &mut BufReader<TcpStream>,
                     line: &str|
     -> Result<String, String> {
        let deadline = std::time::Instant::now() + Duration::from_secs(window_secs);
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
                        s.set_read_timeout(Some(Duration::from_secs(window_secs.max(20))))
                            .ok();
                        *reader = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
                        *stream = s;
                    }
                }
                Err(e) => return Err(format!("engine connection lost: {e}")),
            }
        }
    };

    let dir = shot_dir(project, target, locale, variant);
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
    // Refresh the machine-local screenshot gallery (an at-a-glance index of every capture
    // set under build/day/screenshots/) after each run that saved captures.
    if !run.screenshots.is_empty() {
        write_gallery(&project.root.join("build/day/screenshots"));
    }
    Ok(run)
}

/// Regenerate `build/day/screenshots/index.html`: one labelled thumbnail per capture, grouped
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

/// A minimal standalone base64 decoder — dayscript replies (screenshots, a11y dumps) come back
/// base64-encoded. Inlined here so the CLI needn't pull in `day-script` (and its whole runtime graph:
/// day-core/reactive/pieces/fluent/l10n) for one small function; `day-script` keeps its own copy for
/// the app side.
mod day_script_b64 {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

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
