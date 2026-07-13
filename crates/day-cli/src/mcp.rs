//! `day mcp-server` — a Model Context Protocol server over stdio (docs/agent.md).
//!
//! Gives ANY MCP-capable coding agent (VS Code agent mode, Claude Code, Cursor, CI bots) the
//! full Day loop: inspect the project, build, launch/relaunch/stop, and — through the dayscript
//! engine inside every running app — DRIVE the UI and capture screenshots on all seven toolkits,
//! with images returned as MCP image content so vision models can look at the result.
//!
//! Deliberately thin: each tool call shells out to THIS binary (`current_exe`) with the ordinary
//! CLI arguments and relays the (JSON where available) output. The CLI stays the single source
//! of behavior; the server is transport, not logic. Transport: newline-delimited JSON-RPC 2.0.

use std::io::{BufRead, Write};
use std::process::Command;

use crate::meta::Project;

const PROTOCOL_VERSION: &str = "2024-11-05";

fn tool_list() -> serde_json::Value {
    // NOTE: `name`, `description`, `inputSchema` per the MCP tools spec.
    serde_json::json!([
        {
            "name": "day_metadata",
            "description": "Day project identity: app id/title, declared targets, per-target overrides, host-buildable target catalog, and window defaults. Call this first.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "day_doctor",
            "description": "Check the development environment per toolkit (Rust targets, Android SDK, Xcode, GTK/Qt packages, OpenHarmony SDK). Returns human-readable findings.",
            "inputSchema": {"type": "object", "properties": {
                "toolkit": {"type": "string", "description": "Focus one toolkit (appkit|uikit|gtk|qt|winui|android|harmonyos); its findings become errors with setup help."}
            }}
        },
        {
            "name": "day_build",
            "description": "Compile the app for one or more targets. Compile errors come back in the output; fix and re-run.",
            "inputSchema": {"type": "object", "required": ["targets"], "properties": {
                "targets": {"type": "array", "items": {"type": "string"}, "description": "Target names, e.g. [\"macos-appkit\"]"},
                "profile": {"type": "string", "enum": ["debug", "release"], "default": "debug"}
            }}
        },
        {
            "name": "day_launch",
            "description": "Build and launch the app on one or more targets (detached). Each launch records a drivable session (see day_running / day_drive).",
            "inputSchema": {"type": "object", "required": ["targets"], "properties": {
                "targets": {"type": "array", "items": {"type": "string"}},
                "profile": {"type": "string", "enum": ["debug", "release"], "default": "debug"},
                "locale": {"type": "string", "description": "BCP-47 locale override (e.g. fr, zh-CN)"},
                "env": {"type": "object", "additionalProperties": {"type": "string"}, "description": "Extra environment for the app (e.g. DAY_THEME=dark)"}
            }}
        },
        {
            "name": "day_relaunch",
            "description": "Stop, rebuild, and relaunch targets — the verb for 'apply my code changes'. With no targets, relaunches every running session.",
            "inputSchema": {"type": "object", "properties": {
                "targets": {"type": "array", "items": {"type": "string"}, "description": "Omit to relaunch all running sessions"},
                "profile": {"type": "string", "enum": ["debug", "release"], "default": "debug"}
            }}
        },
        {
            "name": "day_stop",
            "description": "Stop running launches. With no targets, stops everything.",
            "inputSchema": {"type": "object", "properties": {
                "targets": {"type": "array", "items": {"type": "string"}}
            }}
        },
        {
            "name": "day_running",
            "description": "List live sessions (target, app id, engine port, reachability).",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "day_drive",
            "description": "Drive a RUNNING app with dayscript steps and see the result. Ops: navigate {route}, nav_back, tap {id, repeat?}, input {id, text|key}, set_value {id, value}, toggle {id, on}, select {id, index}, wait_for {id}, wait_idle, assert_visible {id}, assert_text {id, text|key}, assert_value {id, value}, assert_route {route}, assert_presented {kind}, respond {…}, a11y_audit, pause {secs}, screenshot {name}. Screenshots return as images — take one after navigating to verify what the user sees.",
            "inputSchema": {"type": "object", "required": ["target", "steps"], "properties": {
                "target": {"type": "string"},
                "steps": {"type": "array", "items": {"type": "object"}, "description": "e.g. [{\"navigate\":{\"route\":\"settings\"}},{\"ui_idle\":null},{\"screenshot\":\"settings\"}]"}
            }}
        },
        {
            "name": "day_screenshot",
            "description": "Capture the current screen of a running target (returns an image).",
            "inputSchema": {"type": "object", "required": ["target"], "properties": {
                "target": {"type": "string"}
            }}
        },
        {
            "name": "day_lint",
            "description": "Check the project for common issues: fluent locale coverage, app id validity. Returns findings.",
            "inputSchema": {"type": "object", "properties": {}}
        }
    ])
}

/// Run this binary with `args`, capture stdout+stderr, and normalize to (ok, text).
fn run_self(project: &Project, args: &[&str]) -> (bool, String) {
    let exe = std::env::current_exe().unwrap_or_else(|_| "day".into());
    let out = Command::new(exe)
        .arg("--project")
        .arg(&project.root)
        .args(args)
        .output();
    match out {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
            let err = String::from_utf8_lossy(&o.stderr);
            if !err.trim().is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(err.trim());
            }
            (o.status.success(), text)
        }
        Err(e) => (false, format!("failed to run day: {e}")),
    }
}

/// MCP text content block.
fn text_content(text: &str) -> serde_json::Value {
    serde_json::json!([{ "type": "text", "text": text }])
}

fn call_tool(
    project: &Project,
    name: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let str_arr = |key: &str| -> Vec<String> {
        args.get(key)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let profile = args
        .get("profile")
        .and_then(|v| v.as_str())
        .unwrap_or("debug")
        .to_string();

    match name {
        "day_metadata" => {
            let (_, text) = run_self(project, &["metadata", "--json"]);
            Ok(text_content(&text))
        }
        "day_doctor" => {
            let mut a = vec!["doctor".to_string()];
            if let Some(t) = args.get("toolkit").and_then(|v| v.as_str()) {
                a.push("--toolkit".into());
                a.push(t.into());
            }
            let refs: Vec<&str> = a.iter().map(String::as_str).collect();
            let (_, text) = run_self(project, &refs);
            Ok(text_content(&text))
        }
        "day_lint" => {
            let (_, text) = run_self(project, &["lint"]);
            Ok(text_content(&text))
        }
        "day_build" => {
            let targets = str_arr("targets");
            if targets.is_empty() {
                return Err("targets is required".into());
            }
            let mut a = vec!["build".to_string()];
            for t in &targets {
                a.push("-p".into());
                a.push(t.clone());
            }
            a.push("--profile".into());
            a.push(profile);
            let refs: Vec<&str> = a.iter().map(String::as_str).collect();
            let (ok, text) = run_self(project, &refs);
            Ok(text_content(&format!(
                "{}\n\n{text}",
                if ok { "BUILD OK" } else { "BUILD FAILED" }
            )))
        }
        "day_launch" | "day_relaunch" => {
            let mut targets = str_arr("targets");
            if name == "day_relaunch" && targets.is_empty() {
                targets = crate::sessions::list(&project.root)
                    .into_iter()
                    .map(|s| s.target)
                    .collect();
                if targets.is_empty() {
                    return Err(
                        "no running sessions to relaunch — use day_launch with explicit targets"
                            .into(),
                    );
                }
            }
            if targets.is_empty() {
                return Err("targets is required".into());
            }
            let mut a: Vec<String> = vec![
                if name == "day_relaunch" {
                    "relaunch"
                } else {
                    "launch"
                }
                .into(),
            ];
            for t in &targets {
                a.push("-p".into());
                a.push(t.clone());
            }
            a.push("--profile".into());
            a.push(profile);
            if name == "day_launch" {
                a.push("--detach".into());
                if let Some(loc) = args.get("locale").and_then(|v| v.as_str()) {
                    a.push("--locale".into());
                    a.push(loc.into());
                }
                if let Some(env) = args.get("env").and_then(|v| v.as_object()) {
                    for (k, v) in env {
                        if let Some(v) = v.as_str() {
                            a.push("--env".into());
                            a.push(format!("{k}={v}"));
                        }
                    }
                }
            }
            let refs: Vec<&str> = a.iter().map(String::as_str).collect();
            let (ok, text) = run_self(project, &refs);
            let sessions = serde_json::to_string_pretty(&crate::sessions::list(&project.root))
                .unwrap_or_default();
            Ok(text_content(&format!(
                "{}\n\n{text}\n\nsessions:\n{sessions}",
                if ok { "OK" } else { "FAILED" }
            )))
        }
        "day_stop" => {
            let targets = str_arr("targets");
            let mut a: Vec<String> = vec!["stop".into()];
            if targets.is_empty() {
                a.push("--all".into());
            } else {
                for t in &targets {
                    a.push("-p".into());
                    a.push(t.clone());
                }
            }
            let refs: Vec<&str> = a.iter().map(String::as_str).collect();
            let (ok, text) = run_self(project, &refs);
            Ok(text_content(&format!(
                "{}\n{text}",
                if ok { "OK" } else { "FAILED" }
            )))
        }
        "day_running" => {
            let sessions = crate::sessions::list(&project.root);
            let rows: Vec<serde_json::Value> = sessions
                .iter()
                .map(|s| {
                    let kind_direct = crate::targets::find(&s.target)
                        .map(|t| {
                            matches!(
                                t.kind,
                                crate::targets::TargetKind::Desktop
                                    | crate::targets::TargetKind::IosSim
                            )
                        })
                        .unwrap_or(false);
                    serde_json::json!({
                        "target": s.target,
                        "appId": s.app_id,
                        "profile": s.profile,
                        "enginePort": s.engine_port,
                        "startedAt": s.started_at,
                        "reachable": crate::sessions::reachable(s, kind_direct),
                    })
                })
                .collect();
            Ok(text_content(
                &serde_json::to_string_pretty(&rows).unwrap_or_default(),
            ))
        }
        "day_drive" | "day_screenshot" => {
            let target = args
                .get("target")
                .and_then(|v| v.as_str())
                .ok_or("target is required")?;
            let steps = if name == "day_screenshot" {
                serde_json::json!([{"wait_idle": null}, {"screenshot": "agent"}])
            } else {
                args.get("steps").cloned().ok_or("steps is required")?
            };
            let steps_str = serde_json::to_string(&steps).map_err(|e| e.to_string())?;
            let (ok, text) = run_self(
                project,
                &["drive", "-p", target, "--steps-json", &steps_str],
            );
            // Lift screenshots out of the drive JSON into MCP image blocks; strip the base64
            // from the text so the transcript stays readable.
            let mut blocks: Vec<serde_json::Value> = Vec::new();
            if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(steps) = v.get_mut("steps").and_then(|s| s.as_array_mut()) {
                    for step in steps.iter_mut() {
                        let png = step
                            .get("screenshot")
                            .and_then(|s| s.get("pngBase64"))
                            .and_then(|b| b.as_str())
                            .map(String::from);
                        if let Some(png) = png {
                            if !png.is_empty() {
                                blocks.push(serde_json::json!({
                                    "type": "image", "data": png, "mimeType": "image/png"
                                }));
                            }
                            if let Some(shot) =
                                step.get_mut("screenshot").and_then(|s| s.as_object_mut())
                            {
                                shot.remove("pngBase64");
                            }
                        }
                    }
                }
                blocks.insert(
                    0,
                    serde_json::json!({"type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or(text)}),
                );
            } else {
                blocks.push(serde_json::json!({"type": "text", "text": format!("{}\n{text}", if ok { "OK" } else { "FAILED" })}));
            }
            Ok(serde_json::Value::Array(blocks))
        }
        other => Err(format!("unknown tool {other}")),
    }
}

pub fn run(project: &Project) -> i32 {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        // Notifications (no id) need no reply.
        let Some(id) = id else { continue };

        let result: Result<serde_json::Value, String> = match method {
            "initialize" => Ok(serde_json::json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "day", "version": env!("CARGO_PKG_VERSION") }
            })),
            "ping" => Ok(serde_json::json!({})),
            "tools/list" => Ok(serde_json::json!({ "tools": tool_list() })),
            "tools/call" => {
                let name = msg
                    .pointer("/params/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let empty = serde_json::json!({});
                let args = msg.pointer("/params/arguments").unwrap_or(&empty);
                match call_tool(project, name, args) {
                    Ok(content) => Ok(serde_json::json!({ "content": content })),
                    Err(e) => Ok(serde_json::json!({
                        "content": [{ "type": "text", "text": format!("error: {e}") }],
                        "isError": true
                    })),
                }
            }
            _ => Err(format!("method not found: {method}")),
        };

        let reply = match result {
            Ok(r) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err(e) => serde_json::json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": e }
            }),
        };
        let _ = writeln!(stdout, "{reply}");
        let _ = stdout.flush();
    }
    0
}
