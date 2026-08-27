// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `day mcp-server` really speaks MCP (docs/agent.md), driven over stdio the way a client drives
//! it: the `initialize` handshake, the tool catalog, a tool call that reaches the CLI, and the
//! error paths.
//!
//! This is the seam between the CLI and every MCP client there is — VS Code agent mode, Claude
//! Code, Cursor, a CI bot — and none of it is exercised by compiling day. A renamed tool, a
//! dropped `initialize` field, a `required` naming a property that no longer exists, or a panic on
//! a malformed line all build perfectly and surface only once an agent has already failed to
//! connect, with the failure attributed to the client. So these tests spawn the real binary and
//! talk to it.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The tools `docs/agent.md` promises. Clients bind to these names, so a rename is a breaking
/// change to every configured agent and has to be a deliberate edit here.
const EXPECTED_TOOLS: &[&str] = &[
    "day_metadata",
    "day_doctor",
    "day_build",
    "day_launch",
    "day_relaunch",
    "day_stop",
    "day_running",
    "day_drive",
    "day_screenshot",
    "day_lint",
];

/// Long enough that a cold machine spawning nested `day` processes finishes comfortably; short
/// enough that a wedged server fails as a test rather than as a CI job timeout.
const TIMEOUT: Duration = Duration::from_secs(120);

// ---------------------------------------------------------------------------------------------
// A throwaway Day project

/// The smallest tree `day metadata` accepts: Day.toml for the app table, Cargo.toml for the name
/// and version it deliberately does not restate, and a target file because cargo rejects a package
/// that has none.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    const APP_ID: &'static str = "dev.example.mcpfixture";
    const TARGETS: [&'static str; 2] = ["macos-appkit", "linux-gtk"];

    fn new(tag: &str) -> std::io::Result<Self> {
        // Tag + pid keeps concurrently-running test threads (and concurrent `cargo test` runs) off
        // each other's directories without a rand dependency.
        let dir = std::env::temp_dir().join(format!("day-mcp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src"))?;
        std::fs::write(
            dir.join("Day.toml"),
            format!(
                "schema = 1\n\n[app]\nid = \"{}\"\ntitle = \"MCP Fixture\"\ntargets = [{}]\n",
                Self::APP_ID,
                Self::TARGETS
                    .iter()
                    .map(|t| format!("\"{t}\""))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        )?;
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"mcp-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\
             \n[workspace]\n",
        )?;
        std::fs::write(dir.join("src/lib.rs"), "")?;
        Ok(Self { dir })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ---------------------------------------------------------------------------------------------
// A minimal MCP client

/// A running `day mcp-server`, with the newline-delimited JSON-RPC plumbing an MCP client provides.
struct Server {
    child: Arc<Mutex<Child>>,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    finished: Arc<AtomicBool>,
    next_id: i64,
}

impl Server {
    fn start(fixture: &Fixture) -> Self {
        Self::start_with(fixture, &[])
    }

    /// `env` is applied on top of the server's environment, for the `DAY_SELF_COMMAND` tests.
    fn start_with(fixture: &Fixture, env: &[(&str, &str)]) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_day"));
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd
            .arg("--project")
            .arg(&fixture.dir)
            .arg("mcp-server")
            // A release-profile test run would otherwise reach crates.io on start.
            .env("DAY_NO_UPDATE_CHECK", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn day mcp-server");
        // Taken before the child is shared, so the watchdog can hold the lock without contending
        // with a blocked read.
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));

        let child = Arc::new(Mutex::new(child));
        let finished = Arc::new(AtomicBool::new(false));
        {
            // Killing the child closes its stdout, which turns a hung read into an EOF the reader
            // reports with context — rather than hanging until the CI job is cancelled.
            let (child, finished) = (Arc::clone(&child), Arc::clone(&finished));
            std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + TIMEOUT;
                while std::time::Instant::now() < deadline {
                    if finished.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                if let Ok(mut c) = child.lock() {
                    let _ = c.kill();
                }
            });
        }
        Self {
            child,
            stdin,
            stdout,
            finished,
            next_id: 0,
        }
    }

    /// Write one raw line to the server. Used directly only for the malformed-input tests; every
    /// well-formed exchange goes through `request`.
    fn send_line(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").expect("write to mcp-server");
        self.stdin.flush().expect("flush mcp-server stdin");
    }

    /// Read one reply, failing with context if the server died instead of answering.
    fn read_reply(&mut self) -> serde_json::Value {
        let mut line = String::new();
        let n = self.stdout.read_line(&mut line).expect("read mcp-server");
        assert!(
            n > 0,
            "mcp-server closed stdout without replying — it exited or was killed by the {}s \
             watchdog",
            TIMEOUT.as_secs()
        );
        serde_json::from_str(&line)
            .unwrap_or_else(|e| panic!("mcp-server wrote a non-JSON line ({e}): {line:?}"))
    }

    /// One request/response round trip, asserting the envelope every JSON-RPC reply must carry.
    ///
    /// The id check is doing real work: it is what proves a notification drew no reply. If the
    /// server ever answered one, that stray reply would be the next line on the wire and would
    /// arrive here bearing the wrong id.
    fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send_line(
            &serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
                .to_string(),
        );
        let reply = self.read_reply();
        assert_eq!(
            reply["jsonrpc"], "2.0",
            "reply is not JSON-RPC 2.0: {reply}"
        );
        assert_eq!(reply["id"], id, "reply carries the wrong id: {reply}");
        reply
    }

    /// A successful request's `result`, with the server's error surfaced if there is one.
    fn result(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let reply = self.request(method, params);
        assert!(
            reply.get("error").is_none(),
            "{method} failed: {}",
            reply["error"]
        );
        reply["result"].clone()
    }

    /// The handshake every client performs before anything else.
    fn initialize(&mut self) -> serde_json::Value {
        self.result(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "day-cli tests", "version": "0"}
            }),
        )
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.finished.store(true, Ordering::Relaxed);
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Tests

#[test]
fn initialize_announces_the_server_and_its_tool_capability() {
    let fixture = Fixture::new("init").expect("write fixture");
    let mut server = Server::start(&fixture);
    let result = server.initialize();

    assert!(
        result["protocolVersion"]
            .as_str()
            .is_some_and(|v| !v.is_empty()),
        "initialize must name a protocol version: {result}"
    );
    // Without this, a client has no reason to ask for the tool list at all.
    assert!(
        result["capabilities"].get("tools").is_some(),
        "initialize must advertise the tools capability: {result}"
    );
    // Naming the project, not just the product: a window of several Day apps runs one server per
    // app, and a client that shows this name would otherwise present identical `day` rows with no
    // way to tell which app each one drives.
    let name = result["serverInfo"]["name"].as_str().unwrap_or_default();
    assert!(
        name.starts_with("day") && name.contains(Fixture::APP_ID),
        "serverInfo.name must identify the bound project: {name:?}"
    );
    assert_eq!(
        result["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION"),
        "serverInfo must report the CLI's real version, so an agent transcript names the day that \
         answered it"
    );
}

#[test]
fn tools_list_returns_the_documented_catalog() {
    let fixture = Fixture::new("tools").expect("write fixture");
    let mut server = Server::start(&fixture);
    server.initialize();

    let result = server.result("tools/list", serde_json::json!({}));
    let tools = result["tools"]
        .as_array()
        .expect("tools/list returns a list");
    let names: Vec<&str> = tools
        .iter()
        .map(|t| t["name"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        names, EXPECTED_TOOLS,
        "the MCP tool catalog changed — clients bind to these names, so update EXPECTED_TOOLS and \
         the table in docs/agent.md deliberately"
    );

    for tool in tools {
        let name = tool["name"].as_str().unwrap_or_default();
        assert!(
            tool["description"].as_str().is_some_and(|d| !d.is_empty()),
            "{name} has no description — the description is the only thing a model reads when \
             deciding whether to call a tool"
        );
        let schema = &tool["inputSchema"];
        assert_eq!(
            schema["type"], "object",
            "{name} must take an object per the MCP tools spec: {schema}"
        );
        // A `required` entry with no matching property is a schema a strict client rejects
        // outright, and the kind of typo that only shows up as "the agent never calls that tool".
        let props = schema.get("properties").and_then(|p| p.as_object());
        for required in schema
            .get("required")
            .and_then(|r| r.as_array())
            .into_iter()
            .flatten()
        {
            let key = required.as_str().unwrap_or_default();
            assert!(
                props.is_some_and(|p| p.contains_key(key)),
                "{name} requires {key:?}, which its properties do not define: {schema}"
            );
        }
    }
}

/// The table in `docs/agent.md` is hand-maintained, and an agent pointed at an undocumented tool
/// is the same failure as a documented tool that does not exist.
#[test]
fn the_documented_tool_table_matches_the_server() {
    let doc = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/agent.md");
    let Ok(src) = std::fs::read_to_string(doc) else {
        return; // not a full workspace checkout (e.g. a published crate's own test run)
    };
    let documented: Vec<&str> = src
        .lines()
        .filter_map(|l| l.trim().strip_prefix("| `day_"))
        .filter_map(|l| l.split('`').next())
        .collect();
    let expected: Vec<String> = EXPECTED_TOOLS
        .iter()
        .map(|t| t.trim_start_matches("day_").to_string())
        .collect();
    assert_eq!(
        documented, expected,
        "the tool table in docs/agent.md has drifted from the server's catalog"
    );
}

#[test]
fn a_tool_call_reaches_the_cli_and_answers_about_this_project() {
    let fixture = Fixture::new("call").expect("write fixture");
    let mut server = Server::start(&fixture);
    server.initialize();

    let result = server.result(
        "tools/call",
        serde_json::json!({"name": "day_metadata", "arguments": {}}),
    );
    assert_ne!(
        result["isError"], true,
        "day_metadata reported an error: {result}"
    );
    let content = result["content"]
        .as_array()
        .expect("tools/call returns content blocks");
    assert_eq!(content[0]["type"], "text");

    // Parsing the SECOND block proves the whole path: the server shelled into this binary with
    // `--project` pointing at the fixture, and relayed `metadata --json` intact. A tool that runs
    // but reports on the wrong directory is precisely the bug `--project` exists to prevent. It
    // has to stay a block of its own — an agent parses it, and a header merged into it would make
    // the one tool everything else depends on unparseable.
    let text = content[1]["text"].as_str().unwrap_or_default();
    let meta: serde_json::Value = serde_json::from_str(text)
        .unwrap_or_else(|e| panic!("day_metadata did not return JSON ({e}): {text}"));

    // The header names the bound project. Without it an agent in a multi-project window builds
    // and drives the wrong app with nothing in any result to say so.
    let note = content[0]["text"].as_str().unwrap_or_default();
    let root = meta["project"]["root"].as_str().unwrap_or_default();
    assert!(
        note.contains(Fixture::APP_ID) && !root.is_empty() && note.contains(root),
        "the first block must name the project this server is bound to ({root}): {note:?}"
    );
    assert_eq!(meta["project"]["id"], Fixture::APP_ID);
    let targets: Vec<&str> = meta["project"]["targets"]
        .as_array()
        .map(|a| a.iter().filter_map(|t| t.as_str()).collect())
        .unwrap_or_default();
    assert_eq!(targets, Fixture::TARGETS);
}

/// A failed tool reports in-band, with `isError`, so the model can read what went wrong and try
/// again. Returning a JSON-RPC error instead would hide it from the model and abort the turn.
#[test]
fn a_failing_tool_reports_in_band_rather_than_as_a_protocol_error() {
    let fixture = Fixture::new("toolerr").expect("write fixture");
    let mut server = Server::start(&fixture);
    server.initialize();

    for arguments in [
        // A tool that does not exist.
        serde_json::json!({"name": "day_nonesuch", "arguments": {}}),
        // A real tool missing a required argument. `day_build` without targets refuses before it
        // compiles anything, so this stays a fast call.
        serde_json::json!({"name": "day_build", "arguments": {}}),
    ] {
        let name = arguments["name"].clone();
        let result = server.result("tools/call", arguments);
        assert_eq!(
            result["isError"], true,
            "{name} should have failed: {result}"
        );
        let texts: Vec<&str> = result["content"]
            .as_array()
            .map(|b| b.iter().filter_map(|x| x["text"].as_str()).collect())
            .unwrap_or_default();
        assert!(
            texts.iter().any(|t| t.contains("error")),
            "{name} failed without saying why: {result}"
        );
        // A failure is exactly when knowing which project answered matters most — "no running
        // sessions" about the wrong app is what sent one agent chasing a working launch.
        assert!(
            texts.iter().any(|t| t.contains(Fixture::APP_ID)),
            "{name}'s failure does not name the project it was about: {result}"
        );
    }
}

#[test]
fn an_unknown_method_is_a_protocol_error() {
    let fixture = Fixture::new("method").expect("write fixture");
    let mut server = Server::start(&fixture);
    server.initialize();

    let reply = server.request("no/such/method", serde_json::json!({}));
    assert!(
        reply.get("result").is_none(),
        "an error reply must not also carry a result: {reply}"
    );
    assert_eq!(
        reply["error"]["code"], -32601,
        "unknown methods are JSON-RPC 'method not found': {reply}"
    );
}

/// `DAY_SELF_COMMAND` is how a day-development window keeps the agent's tools on the same CLI as
/// the editor's own Build and Run: the extension resolves `cargo run` against the open checkout
/// and names it here, so a day-cli edit is compiled into the next tool call.
///
/// Proved by pointing it somewhere unrunnable. Were the override ignored, the call would quietly
/// succeed through `current_exe()` — which is exactly the silent staleness this exists to end, and
/// a test that could not tell the difference would be no test at all.
#[test]
fn a_tool_call_re_invokes_the_cli_the_self_command_names() {
    let fixture = Fixture::new("selfcmd").expect("write fixture");
    let mut server = Server::start_with(
        &fixture,
        &[(
            "DAY_SELF_COMMAND",
            r#"["day-cli-no-such-program-exists-here"]"#,
        )],
    );
    server.initialize();

    let result = server.result(
        "tools/call",
        serde_json::json!({"name": "day_metadata", "arguments": {}}),
    );
    let text = result["content"]
        .as_array()
        .map(|b| {
            b.iter()
                .filter_map(|x| x["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    assert!(
        text.contains("failed to run day"),
        "the tool call did not go through DAY_SELF_COMMAND: {text}"
    );
}

/// The override is a convenience channel, so an unusable value must degrade to the default rather
/// than break every tool on the server.
#[test]
fn an_unusable_self_command_falls_back_to_this_binary() {
    let fixture = Fixture::new("selfbad").expect("write fixture");
    for value in ["not json", "[]", "{}", "[1, 2]"] {
        let mut server = Server::start_with(&fixture, &[("DAY_SELF_COMMAND", value)]);
        server.initialize();
        let result = server.result(
            "tools/call",
            serde_json::json!({"name": "day_metadata", "arguments": {}}),
        );
        let text = result["content"][1]["text"].as_str().unwrap_or_default();
        let meta: serde_json::Value = serde_json::from_str(text).unwrap_or_else(|e| {
            panic!("DAY_SELF_COMMAND={value:?} broke day_metadata ({e}): {text}")
        });
        assert_eq!(meta["project"]["id"], Fixture::APP_ID);
    }
}

/// A client sends notifications (`notifications/initialized` right after the handshake) and can
/// send anything at all down a pipe. Neither may end the session: an MCP server that exits on the
/// first surprise looks to the user like an extension that silently stopped working.
#[test]
fn junk_and_notifications_do_not_end_the_session() {
    let fixture = Fixture::new("junk").expect("write fixture");
    let mut server = Server::start(&fixture);
    server.initialize();

    server.send_line("this is not json at all");
    server.send_line("{\"jsonrpc\": \"2.0\", \"method\": \"notifications/initialized\"}");
    server.send_line("");
    server.send_line("{\"jsonrpc\": \"2.0\"}");

    // Answering at all proves the server survived; `request` proves no stray reply was emitted for
    // any of the lines above, since a reply for one of them would arrive here with the wrong id.
    let result = server.result("tools/list", serde_json::json!({}));
    assert_eq!(
        result["tools"].as_array().map(Vec::len),
        Some(EXPECTED_TOOLS.len())
    );
}
