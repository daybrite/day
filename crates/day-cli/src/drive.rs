//! `day drive` — execute dayscript steps against an ALREADY-RUNNING app (docs/agent.md).
//!
//! The session registry (sessions.rs) holds the engine coordinates a previous `day launch`
//! recorded; this command connects, runs the given steps, and reports one JSON object per step
//! on stdout (an array). Steps use the walkthrough vocabulary in either spelling:
//!
//! ```json
//! [{"navigate": {"route": "controls"}}, {"ui_idle": null}, {"screenshot": "controls"}]
//! [{"op": "tap", "id": "increment"}]
//! ```
//!
//! Screenshots are written under `build/day/screenshots/_drive/` and reported as `{path,
//! pngBase64}` so callers (the MCP server, agents) can show the pixels without re-reading disk.

use std::io::{BufRead, BufReader, Write};

use crate::meta::Project;
use crate::script;
use crate::sessions;
use crate::targets::Target;

/// Normalize either step spelling into the engine's flattened `{"op": …, …}` form.
fn normalize(
    step: &serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let obj = step.as_object().ok_or("steps must be JSON objects")?;
    if obj.contains_key("op") {
        return Ok(obj.clone());
    }
    let (op, params) = obj.iter().next().ok_or("empty step")?;
    if obj.len() != 1 {
        return Err(format!(
            "step must be {{\"op\": …}} or a single-key mapping, got {} keys",
            obj.len()
        ));
    }
    let mut out = serde_json::Map::new();
    out.insert("op".into(), serde_json::Value::String(op.clone()));
    match params {
        serde_json::Value::Object(m) => {
            for (k, v) in m {
                out.insert(k.clone(), v.clone());
            }
        }
        serde_json::Value::String(s) if op == "screenshot" => {
            out.insert("name".into(), serde_json::Value::String(s.clone()));
        }
        serde_json::Value::Number(n) if op == "pause" => {
            out.insert("secs".into(), serde_json::Value::Number(n.clone()));
        }
        serde_json::Value::Null => {}
        other => return Err(format!("step {op}: unsupported params {other}")),
    }
    Ok(out)
}

pub fn run(project: &Project, target: &Target, steps_json: &str) -> i32 {
    let steps: Vec<serde_json::Value> = match serde_json::from_str(steps_json) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: --steps-json must be a JSON array of steps: {e}");
            return 2;
        }
    };
    let Some(session) = sessions::find(&project.root, target.name) else {
        eprintln!(
            "error: no live session for {} — `day launch -p {}` first (sessions: build/day/sessions.json)",
            target.name, target.name
        );
        return 3;
    };

    script::forward_engine(target.kind, session.engine_port);
    let stream = match script::connect(
        session.engine_port,
        script::connect_window_secs(target.kind),
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "error: cannot reach the {} engine on port {}: {e} (is the app still running?)",
                target.name, session.engine_port
            );
            sessions::remove(&project.root, target.name);
            return 3;
        }
    };
    let mut stream = stream;
    let mut reader = match stream.try_clone() {
        Ok(s) => BufReader::new(s),
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    let shot_dir = project.root.join("build/day/screenshots/_drive");
    let _ = std::fs::create_dir_all(&shot_dir);

    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut failed = 0usize;
    for raw in &steps {
        let step = match normalize(raw) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        };
        let op = step
            .get("op")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // `pause` sleeps runner-side (the engine must not block the UI thread).
        if op == "pause" {
            let secs = step.get("secs").and_then(|v| v.as_f64()).unwrap_or(0.5);
            std::thread::sleep(std::time::Duration::from_secs_f64(secs));
            results.push(serde_json::json!({"op": "pause", "ok": true}));
            continue;
        }
        let req = serde_json::json!({"token": session.engine_token, "step": step});
        let mut line = serde_json::to_string(&req).unwrap();
        line.push('\n');
        let reply: serde_json::Value = match stream
            .write_all(line.as_bytes())
            .map_err(|e| e.to_string())
            .and_then(|_| {
                let mut reply = String::new();
                let n = reader.read_line(&mut reply).map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("EOF".into());
                }
                serde_json::from_str(reply.trim()).map_err(|e| e.to_string())
            }) {
            Ok(r) => r,
            Err(e) => {
                results.push(serde_json::json!({"op": op, "ok": false, "error": format!("engine connection lost: {e}")}));
                failed += 1;
                break;
            }
        };
        let ok = reply.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if !ok {
            failed += 1;
        }
        let mut result = serde_json::json!({
            "op": op,
            "ok": ok,
        });
        if let Some(err) = reply.get("error").and_then(|v| v.as_str()) {
            result["error"] = serde_json::Value::String(err.into());
        }
        if let Some(id) = step.get("id") {
            result["id"] = id.clone();
        }
        if op == "screenshot" && ok {
            let name = step.get("name").and_then(|v| v.as_str()).unwrap_or("shot");
            if let Some(b64) = reply.get("png_base64").and_then(|v| v.as_str()) {
                let path = shot_dir.join(format!("{name}.png"));
                let bytes = script::b64decode_public(b64);
                let _ = std::fs::write(&path, bytes);
                result["screenshot"] = serde_json::json!({
                    "path": path.display().to_string(),
                    "pngBase64": b64,
                });
            } else if reply
                .get("screenshot_unsupported")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                // In-process capture unsupported on this toolkit: fall back to a device-side
                // capture, then inline the bytes for parity with the in-process path.
                let path = shot_dir.join(format!("{name}.png"));
                if script::device_screenshot_public(target, &path).is_ok() {
                    let b64 = std::fs::read(&path)
                        .map(|b| script::b64encode_public(&b))
                        .unwrap_or_default();
                    result["screenshot"] = serde_json::json!({
                        "path": path.display().to_string(),
                        "pngBase64": b64,
                    });
                } else {
                    result["ok"] = serde_json::Value::Bool(false);
                    result["error"] = serde_json::Value::String(
                        "screenshot unsupported and device capture failed".into(),
                    );
                    failed += 1;
                }
            }
        }
        results.push(result);
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "target": target.name,
            "steps": results,
            "failed": failed,
        }))
        .unwrap()
    );
    if failed > 0 { 5 } else { 0 }
}
