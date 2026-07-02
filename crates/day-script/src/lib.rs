//! day-script — the embedded dayscript engine (DESIGN.md §14). Bind-only-when-invited: the
//! server starts ONLY when DAYSCRIPT_PORT + DAYSCRIPT_TOKEN are present in the environment
//! (never otherwise), listens on 127.0.0.1, and accepts only the step catalog. Steps execute
//! as synthesized day events on the main thread between flushes — deterministic and
//! toolkit-uniform. Locator steps get an implicit bounded wait (default 5s).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use day_core::{NodeProbe, rnode_to_id, with_tree};
use serde::{Deserialize, Serialize};

pub const DEFAULT_TIMEOUT_SECS: f64 = 5.0;

// ---------------------------------------------------------------------------
// Wire protocol (shared with the day CLI runner)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Request {
    pub token: String,
    pub step: Step,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Step {
    WaitFor {
        id: String,
    },
    WaitIdle,
    Tap {
        id: String,
        #[serde(default)]
        repeat: Option<u32>,
    },
    Input {
        id: String,
        text: String,
    },
    SetValue {
        id: String,
        value: f64,
    },
    Toggle {
        id: String,
        #[serde(default)]
        value: Option<bool>,
    },
    Select {
        id: String,
        index: i64,
    },
    AssertVisible {
        id: String,
    },
    AssertText {
        id: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        args: Option<BTreeMap<String, serde_json::Value>>,
    },
    AssertValue {
        id: String,
        value: serde_json::Value,
    },
    Screenshot {
        name: String,
    },
    Pause {
        secs: f64,
    },
    /// Navigate to a registered route (reset-to semantics; "" = root). docs/navigation.md.
    Navigate {
        route: String,
    },
    /// Pop one navigation level (the native back path, day-initiated).
    NavBack,
    /// Assert the current route path ("" = root).
    AssertRoute {
        route: String,
    },
    /// Assert a modal is presented, optionally checking its title (docs/dialogs.md).
    AssertPresented {
        #[serde(default)]
        title: Option<String>,
    },
    /// Answer the open modal: a button `index`, a prompt `text`, or `dismiss`.
    Respond {
        #[serde(default)]
        button: Option<i64>,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        dismiss: bool,
    },
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Set on failures that may succeed after a wait (element not found yet, assert pending).
    #[serde(default)]
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub png_base64: Option<String>,
    #[serde(default)]
    pub screenshot_unsupported: bool,
}

impl Reply {
    fn ok() -> Self {
        Reply {
            ok: true,
            ..Default::default()
        }
    }
    fn fail(msg: impl Into<String>, retryable: bool) -> Self {
        Reply {
            ok: false,
            error: Some(msg.into()),
            retryable,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Engine activation + server
// ---------------------------------------------------------------------------

/// Start the engine iff invited via env (call before `launch_with`; inert otherwise).
pub fn init() {
    let (Ok(port), Ok(token)) = (
        std::env::var("DAYSCRIPT_PORT"),
        std::env::var("DAYSCRIPT_TOKEN"),
    ) else {
        return;
    };
    let Ok(port) = port.parse::<u16>() else {
        return;
    };
    std::thread::spawn(move || serve(port, token));
}

fn serve(port: u16, token: String) {
    // Give the app time to mount before binding traffic arrives.
    std::thread::sleep(Duration::from_millis(300));
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("day-script: bind 127.0.0.1:{port} failed: {e}");
            return;
        }
    };
    for stream in listener.incoming().flatten() {
        handle_conn(stream, &token);
    }
}

fn handle_conn(stream: TcpStream, token: &str) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut stream = stream;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let reply = match serde_json::from_str::<Request>(line.trim()) {
            Ok(req) if req.token == token => run_step_with_wait(req.step),
            Ok(_) => Reply::fail("bad token", false),
            Err(e) => Reply::fail(format!("bad request: {e}"), false),
        };
        let mut out = serde_json::to_string(&reply).unwrap_or_else(|_| "{\"ok\":false}".into());
        out.push('\n');
        if stream.write_all(out.as_bytes()).is_err() {
            return;
        }
    }
}

/// Implicit bounded wait (§14.3): retryable failures poll on the main thread until timeout.
fn run_step_with_wait(step: Step) -> Reply {
    let deadline = Instant::now() + Duration::from_secs_f64(DEFAULT_TIMEOUT_SECS);
    loop {
        let reply = run_on_main(step.clone());
        if reply.ok || !reply.retryable || Instant::now() > deadline {
            return reply;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn run_on_main(step: Step) -> Reply {
    let (tx, rx) = mpsc::sync_channel::<Reply>(1);
    day_reactive::on_main(move || {
        let _ = tx.send(exec(step));
    });
    rx.recv_timeout(Duration::from_secs(10))
        .unwrap_or_else(|_| Reply::fail("main thread did not respond", false))
}

// ---------------------------------------------------------------------------
// Step execution (main thread; events go through the normal day path)
// ---------------------------------------------------------------------------

fn find(id: &str) -> Result<day_core::RNode, Reply> {
    with_tree(|t| t.find_by_id(id)).ok_or_else(|| Reply::fail(format!("no element {id:?}"), true))
}

fn probe(id: &str) -> Result<NodeProbe, Reply> {
    let node = find(id)?;
    with_tree(|t| t.node_probe(node)).ok_or_else(|| Reply::fail("element vanished", true))
}

fn emit(id: &str, ev: day_spec::Event) -> Result<(), Reply> {
    let node = find(id)?;
    day_core::enqueue_event(rnode_to_id(node), ev);
    day_reactive::flush_sync();
    Ok(())
}

fn visible(id: &str) -> Result<(), Reply> {
    let node = find(id)?;
    let frame = with_tree(|t| t.node_frame(node));
    match frame {
        Some(f) if f.size.width > 0.0 && f.size.height > 0.0 => Ok(()),
        _ => Err(Reply::fail(format!("{id:?} has no visible frame"), true)),
    }
}

fn norm(s: &str) -> String {
    day_fluent::strip_isolates(s)
}

fn exec(step: Step) -> Reply {
    use day_spec::Event;
    use day_spec::present::PresentResult;
    let result: Result<Reply, Reply> = (|| {
        match step {
            Step::WaitFor { id } => {
                visible(&id)?;
                Ok(Reply::ok())
            }
            Step::WaitIdle => {
                day_reactive::flush_sync();
                Ok(Reply::ok())
            }
            Step::Tap { id, repeat } => {
                for _ in 0..repeat.unwrap_or(1).max(1) {
                    emit(&id, Event::Pressed)?;
                }
                Ok(Reply::ok())
            }
            Step::Input { id, text } => {
                emit(&id, Event::TextChanged(text))?;
                Ok(Reply::ok())
            }
            Step::SetValue { id, value } => {
                emit(&id, Event::ValueChanged(value))?;
                Ok(Reply::ok())
            }
            Step::Toggle { id, value } => {
                let target = match value {
                    Some(v) => v,
                    None => !probe(&id)?.flag,
                };
                emit(&id, Event::ToggleChanged(target))?;
                Ok(Reply::ok())
            }
            Step::Select { id, index } => {
                emit(&id, Event::SelectionChanged(index))?;
                Ok(Reply::ok())
            }
            Step::AssertVisible { id } => {
                visible(&id)?;
                Ok(Reply::ok())
            }
            Step::AssertText {
                id,
                text,
                key,
                args,
            } => {
                let actual = norm(&probe(&id)?.text);
                let expected = if let Some(k) = key {
                    let mut lt = day_fluent::tr(&k);
                    for (name, v) in args.unwrap_or_default() {
                        lt = match v {
                            serde_json::Value::Number(n) => {
                                lt.arg(&name, n.as_f64().unwrap_or(0.0))
                            }
                            serde_json::Value::String(s) => lt.arg(&name, s),
                            other => lt.arg(&name, other.to_string()),
                        };
                    }
                    norm(&lt.format())
                } else {
                    norm(&text.unwrap_or_default())
                };
                if actual == expected {
                    Ok(Reply::ok())
                } else {
                    Err(Reply::fail(
                        format!("{id:?}: expected {expected:?}, found {actual:?}"),
                        true,
                    ))
                }
            }
            Step::AssertValue { id, value } => {
                let p = probe(&id)?;
                let ok = match &value {
                    serde_json::Value::Bool(b) => p.flag == *b,
                    serde_json::Value::Number(n) => {
                        (p.value - n.as_f64().unwrap_or(f64::NAN)).abs() < 0.5
                    }
                    serde_json::Value::String(s) => norm(&p.text) == norm(s),
                    _ => false,
                };
                if ok {
                    Ok(Reply::ok())
                } else {
                    Err(Reply::fail(
                        format!(
                            "{id:?}: expected {value}, probe text={:?} value={} flag={}",
                            p.text, p.value, p.flag
                        ),
                        true,
                    ))
                }
            }
            Step::Screenshot { .. } => {
                let png = with_tree(|t| t.snapshot());
                match png {
                    Ok(bytes) => Ok(Reply {
                        ok: true,
                        png_base64: Some(b64encode(&bytes)),
                        ..Default::default()
                    }),
                    Err(_) => Ok(Reply {
                        ok: true,
                        screenshot_unsupported: true,
                        ..Default::default()
                    }),
                }
            }
            Step::Pause { secs } => {
                // Pausing the MAIN thread would freeze the UI; the runner sleeps instead.
                let _ = secs;
                Ok(Reply::ok())
            }
            Step::Navigate { route } => {
                day_reactive::flush_sync();
                if day_core::navigate(&route) {
                    day_reactive::flush_sync();
                    Ok(Reply::ok())
                } else {
                    // Retryable: the nav host may not have mounted yet.
                    Err(Reply::fail(format!("no route {route:?}"), true))
                }
            }
            Step::NavBack => {
                if day_core::nav_back() {
                    day_reactive::flush_sync();
                    Ok(Reply::ok())
                } else {
                    Err(Reply::fail("nothing to pop", true))
                }
            }
            Step::AssertRoute { route } => {
                let current = day_core::current_route();
                if current.as_deref() == Some(route.as_str()) {
                    Ok(Reply::ok())
                } else {
                    Err(Reply::fail(
                        format!("expected route {route:?}, current {current:?}"),
                        true,
                    ))
                }
            }
            Step::AssertPresented { title } => match day_core::pending_presentation() {
                Some((_, spec)) => {
                    let actual = norm(spec.title());
                    match title {
                        Some(want) if norm(&want) != actual => Err(Reply::fail(
                            format!("modal title {actual:?} != expected {want:?}"),
                            true,
                        )),
                        _ => Ok(Reply::ok()),
                    }
                }
                None => Err(Reply::fail("no modal presented", true)),
            },
            Step::Respond {
                button,
                text,
                dismiss,
            } => {
                let Some((req, _)) = day_core::pending_presentation() else {
                    return Err(Reply::fail("no modal to respond to", true));
                };
                let result = if dismiss {
                    PresentResult::Dismissed
                } else if let Some(t) = text {
                    PresentResult::Text(t)
                } else if let Some(i) = button {
                    PresentResult::Button(i)
                } else {
                    PresentResult::Dismissed
                };
                day_core::respond_presentation(req, result);
                day_reactive::flush_sync();
                Ok(Reply::ok())
            }
        }
    })();
    result.unwrap_or_else(|r| r)
}

// ---------------------------------------------------------------------------
// Minimal base64 (no dependency; screenshots cross as one JSON line)
// ---------------------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
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
