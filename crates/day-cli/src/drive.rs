// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

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

use crate::cli::{CliError, ErrKind};
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

/// The Ok value is the run's verdict code: 0, or the script-failure code when steps failed —
/// the per-step JSON report on stdout already carries the detail.
pub fn run(project: &Project, target: &Target, steps_json: &str) -> Result<i32, CliError> {
    let steps: Vec<serde_json::Value> = serde_json::from_str(steps_json)
        .map_err(|e| CliError::usage(format!("--steps-json must be a JSON array of steps: {e}")))?;
    let Some(session) = sessions::find(&project.root, target.name) else {
        return Err(CliError::env(format!(
            "no live session for {} — `day launch -p {}` first (sessions: build/day/sessions.json)",
            target.name, target.name
        )));
    };

    script::forward_engine(target.kind, session.engine_port);
    let stream = script::connect(
        session.engine_port,
        script::connect_window_secs(target.kind),
    )
    .map_err(|e| {
        sessions::remove(&project.root, target.name);
        CliError::env(format!(
            "cannot reach the {} engine on port {}: {e} (is the app still running?)",
            target.name, session.engine_port
        ))
    })?;
    let mut stream = stream;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|e| CliError::failure(e.to_string()))?,
    );

    let shot_dir = project.root.join("build/day/screenshots/_drive");
    let _ = std::fs::create_dir_all(&shot_dir);

    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut failed = 0usize;
    for raw in &steps {
        let step = normalize(raw).map_err(CliError::usage)?;
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
            // Same precedence as a scripted run (script.rs): the device capture is the real
            // picture on a device or simulator, the in-process one on desktop. Splitting the rule
            // between the two entry points would frame the same screen two different ways.
            let in_process = reply
                .get("png_base64")
                .and_then(|v| v.as_str())
                .filter(|_| target.kind == crate::targets::TargetKind::Desktop);
            if let Some(b64) = in_process {
                let path = shot_dir.join(format!("{name}.png"));
                let bytes = script::b64decode_public(b64);
                let _ = std::fs::write(&path, bytes);
                result["screenshot"] = serde_json::json!({
                    "path": path.display().to_string(),
                    "pngBase64": b64,
                });
            } else {
                // Device-side capture, inlined for parity with the in-process path. When it
                // refuses, an in-process capture the backend did supply stands in.
                let path = shot_dir.join(format!("{name}.png"));
                let fallback = reply.get("png_base64").and_then(|v| v.as_str());
                if script::device_screenshot_public(target, &path).is_ok() {
                    let b64 = std::fs::read(&path)
                        .map(|b| script::b64encode_public(&b))
                        .unwrap_or_default();
                    result["screenshot"] = serde_json::json!({
                        "path": path.display().to_string(),
                        "pngBase64": b64,
                    });
                } else if let Some(b64) = fallback {
                    let bytes = script::b64decode_public(b64);
                    let _ = std::fs::write(&path, bytes);
                    result["screenshot"] = serde_json::json!({
                        "path": path.display().to_string(),
                        "pngBase64": b64,
                        "framing": "content",
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
    Ok(if failed > 0 {
        ErrKind::Script.exit_code()
    } else {
        0
    })
}
