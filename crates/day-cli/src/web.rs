// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! web-dom build + launch (DESIGN.md §9, docs/web.md). Build compiles the app's lib crate as a
//! wasm32 cdylib and assembles a self-contained `dist/` — host page + shim + stylesheet
//! (embedded from `resources/web/` at CLI compile time), the wasm module, bundled
//! images, and fonts with a `fonts.json` manifest the shim pre-loads. Launch serves `dist/`
//! over loopback (browsers won't instantiate wasm from `file:`) and opens the default browser.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::meta::Project;
use crate::ops::{BuildOutcome, LaunchSpec, apply_app_identity, feature_selection, status};
use crate::targets::Target;

// The host page trio, embedded so an installed CLI needs no source checkout. They live INSIDE
// this crate (not next to `toolkits/day-dom`, whose `extern "C"` block shim.js implements)
// because `include_str!` may not reach outside the package: `cargo package` copies only this
// directory, so a path into the workspace vanishes on crates.io and `cargo install day-cli`
// fails to compile — which is exactly what shipped in 0.0.15. Editing shim.js means rebuilding
// the CLI, and `day-dom`'s crate docs point here.
const HOST_INDEX: &str = include_str!("../resources/web/index.html");
const HOST_SHIM: &str = include_str!("../resources/web/shim.js");
const HOST_CSS: &str = include_str!("../resources/web/day.css");

pub fn build_web(
    project: &Project,
    target: &'static Target,
    profile: &str,
    start: std::time::Instant,
) -> Result<BuildOutcome, String> {
    let name = &project.manifest.app.name;
    let features = feature_selection(project, target.toolkit);
    let cargo_dir = crate::ops::cargo_dir(project, target, profile);

    let mut cmd = Command::new("cargo");
    cmd.current_dir(&project.root)
        .env("CARGO_TARGET_DIR", &cargo_dir)
        // The app's lib as a cdylib (the same shape as Android/HarmonyOS): `web_main!` exports
        // `day_dom_main`, which the host page calls after instantiation.
        .args(["rustc", "-p", name, "--lib", "--no-default-features"])
        .args(["--features", &features])
        .args(["--target", "wasm32-unknown-unknown"]);
    apply_app_identity(&mut cmd, project);
    crate::bridge::apply_staged(&mut cmd, project, "web-dom");
    if profile == "release" {
        cmd.arg("--release");
    }
    cmd.args(["--crate-type", "cdylib"]);
    if profile == "release" {
        // Debug symbols are most of a release wasm's bytes and no browser tool reads them
        // from a stripped build; keep the shipped module small.
        cmd.args(["--", "-Cstrip=symbols"]);
    }
    status("Building", &format!("{} ({profile})", target.name));
    let out = cmd.status().map_err(|e| format!("cargo: {e}"))?;
    if !out.success() {
        return Err(format!(
            "cargo build failed for {} (is the target installed? `rustup target add wasm32-unknown-unknown`)",
            target.name
        ));
    }

    // Assemble dist/. The wasm artifact uses the lib name (hyphens become underscores).
    let wasm = cargo_dir
        .join("wasm32-unknown-unknown")
        .join(profile)
        .join(format!("{}.wasm", name.replace('-', "_")));
    let dist = cargo_dir.join("dist");
    std::fs::create_dir_all(&dist).map_err(|e| format!("dist dir: {e}"))?;
    std::fs::write(dist.join("shim.js"), HOST_SHIM).map_err(|e| format!("shim: {e}"))?;
    // daybridge web arms (docs/bridge.md): one ES module per bridged crate, listed for the shim to
    // import and merge into the wasm imports before instantiation.
    let bridges = crate::bridge::write_js(&dist, &crate::bridge::stage(project, "web"))?;
    std::fs::write(dist.join("day.css"), HOST_CSS).map_err(|e| format!("css: {e}"))?;
    std::fs::copy(&wasm, dist.join("app.wasm")).map_err(|e| format!("{}: {e}", wasm.display()))?;

    // Vector glyphs (docs/vectors.md): the SVG is what day-dom asks for and what the browser
    // renders at display size, so only the raster FALLBACKS land beside the images — art the
    // vector pipeline could not express. Every browser that can run a wasm app renders SVG, so
    // a second PNG copy of a convertible glyph is weight that would also hide a broken vector
    // path behind art that still looks right. The page learns which names are vectors via
    // `window.__DAY_VECTORS` (injected into index.html below), read back through the shim's
    // `vector:` env keys.
    let mut vector_names: Vec<String> = Vec::new();
    let vectors_cache = crate::resources::vector_fallback_dir(project, target.toolkit);
    if vectors_cache.is_dir() {
        let images = dist.join("assets/images");
        std::fs::create_dir_all(&images).map_err(|e| format!("images dir: {e}"))?;
        for entry in std::fs::read_dir(&vectors_cache)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let p = entry.path();
            if p.extension().and_then(|x| x.to_str()) == Some("png")
                && let Some(name) = p.file_name()
            {
                std::fs::copy(&p, images.join(name)).map_err(|e| format!("vector copy: {e}"))?;
            }
        }
    }
    let svg_cache = crate::resources::vector_svg_dir(project);
    if svg_cache.is_dir() {
        let images = dist.join("assets/images");
        std::fs::create_dir_all(&images).map_err(|e| format!("images dir: {e}"))?;
        for entry in std::fs::read_dir(&svg_cache)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let p = entry.path();
            if p.extension().and_then(|x| x.to_str()) == Some("svg")
                && let (Some(name), Some(stem)) =
                    (p.file_name(), p.file_stem().and_then(|s| s.to_str()))
            {
                std::fs::copy(&p, images.join(name)).map_err(|e| format!("vector copy: {e}"))?;
                vector_names.push(stem.to_string());
            }
        }
        vector_names.sort();
    }
    // Resource names are `[a-z0-9_]` (day-build enforces this), so plain quoting is JS-safe.
    let vectors_json = vector_names
        .iter()
        .map(|n| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(",");
    let bridges_json = bridges
        .iter()
        .map(|m| format!("\"{m}\""))
        .collect::<Vec<_>>()
        .join(",");
    std::fs::write(
        dist.join("index.html"),
        HOST_INDEX
            .replace("[/*day:vectors*/]", &format!("[{vectors_json}]"))
            .replace("[/*day:bridges*/]", &format!("[{bridges_json}]")),
    )
    .map_err(|e| format!("index: {e}"))?;
    // Bundled images, flat under assets/images/ — the paths day-dom writes into `src` attrs.
    let images_src = project.root.join("resource/images");
    if images_src.is_dir() {
        let images = dist.join("assets/images");
        std::fs::create_dir_all(&images).map_err(|e| format!("images dir: {e}"))?;
        for f in std::fs::read_dir(&images_src)
            .map_err(|e| format!("images: {e}"))?
            .flatten()
        {
            if f.path().is_file() {
                std::fs::copy(f.path(), images.join(f.file_name()))
                    .map_err(|e| format!("{}: {e}", f.path().display()))?;
            }
        }
    }

    // Bundled data assets, the whole TREE (§18.5), under assets/data/ — same-origin URLs for
    // anything that browses them (the inline web view's `assets/data/<site>/…` base above all).
    let data_src = project.root.join("resource/assets");
    if data_src.is_dir() {
        crate::pack::copy_tree(&data_src, &dist.join("assets/data"))?;
    }

    // Bundled fonts + the fonts.json manifest (family name from the font's own name table, the
    // same resolution day-build codegen uses) — the shim registers each FontFace before the
    // first layout so custom families measure correctly.
    let fonts = crate::resources::scan_fonts(project)?;
    if !fonts.is_empty() {
        let dir = dist.join("assets/fonts");
        std::fs::create_dir_all(&dir).map_err(|e| format!("fonts dir: {e}"))?;
        let mut manifest = String::from("[");
        for (i, f) in fonts.iter().enumerate() {
            let staged = f.staged_name();
            std::fs::copy(&f.path, dir.join(&staged))
                .map_err(|e| format!("{}: {e}", f.path.display()))?;
            if i > 0 {
                manifest.push(',');
            }
            manifest.push_str(&format!(
                "{{\"family\":\"{}\",\"url\":\"assets/fonts/{staged}\"}}",
                f.family.replace('"', "\\\"")
            ));
        }
        manifest.push(']');
        std::fs::write(dir.join("fonts.json"), manifest).map_err(|e| format!("fonts.json: {e}"))?;
    }

    Ok(BuildOutcome {
        target: target.name,
        artifact: dist,
        seconds: start.elapsed().as_secs_f64(),
    })
}

/// Percent-encode a query key/value: keep unreserved characters (RFC 3986), escape the rest —
/// `URLSearchParams` on the page decodes them back.
fn query_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Serve the assembled `dist/` on a loopback port and open a browser at it. The returned
/// handle runs the accept loop; `day launch` stays in the foreground (Ctrl-C stops the
/// server). `--locale` and a `DAY_THEME` env ride as query parameters (`?locale=`,
/// `?theme=`), and every other `--env` pair as `?<key>=<value>` for `day::env` to read back;
/// the session's dayscript invitation rides as `?dayscript=<token>`, and the
/// server bridges the page's `/dayscript` WebSocket to the plain TCP protocol the runner
/// speaks on `DAYSCRIPT_PORT` — `--script` and `day drive` work unchanged (docs/web.md).
pub fn launch_web(
    project: &Project,
    outcome: &BuildOutcome,
    spec: &LaunchSpec,
) -> Result<std::thread::JoinHandle<i32>, String> {
    let dist = outcome.artifact.clone();
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("bind: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("addr: {e}"))?
        .port();
    let env_of = |key: &str| {
        spec.envs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    let mut params: Vec<String> = Vec::new();
    if let Some(locale) = &spec.locale {
        params.push(format!("locale={locale}"));
    }
    if let Some(theme) = env_of("DAY_THEME") {
        params.push(format!("theme={theme}"));
    }
    if let Some(token) = env_of("DAYSCRIPT_TOKEN") {
        params.push(format!("dayscript={token}"));
    }
    // Every other `--env` pair travels as its own query parameter: a browser sandbox has no
    // process environment, so the page URL is the delivery channel and `day::env` reads it
    // back through the shim's `day_dom_env` (docs/web.md). DAYSCRIPT_PORT stays host-side,
    // and DAYSCRIPT_TOKEN/DAY_THEME already travel under their reserved names above.
    for (k, v) in &spec.envs {
        if k == "DAYSCRIPT_PORT" || k == "DAYSCRIPT_TOKEN" || k == "DAY_THEME" {
            continue;
        }
        params.push(format!("{}={}", query_escape(k), query_escape(v)));
    }
    let mut url = format!("http://127.0.0.1:{port}/");
    if !params.is_empty() {
        url.push('?');
        url.push_str(&params.join("&"));
    }
    status("Serving", &format!("{} → {url}", dist.display()));
    let _ = &project.root; // identity/env vars are baked into the wasm at build time
    if let Some(script_port) = env_of("DAYSCRIPT_PORT").and_then(|p| p.parse::<u16>().ok()) {
        start_runner_bridge(script_port)?;
    }
    // Drop the PREVIOUS launch's bridge endpoints before opening this page (capture matrix:
    // several launches share one process and one bridge). The old page's socket is closed but
    // still WRITABLE — TCP buffers the first write after a peer close and errors only on the
    // next — so a stale PAGE_WS swallows the new run's first step whole: forwarded "successfully",
    // no reply ever, the runner burns its whole window and reports the engine lost. Cleared
    // slots make forward_to_page genuinely wait for THIS page's registration instead.
    {
        *PAGE_WS.lock().expect("page slot") = None;
        *RUNNER.lock().expect("runner slot") = None;
    }
    open_page(&url)?;
    let handle = std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let dist = dist.clone();
            std::thread::spawn(move || serve_one(stream, &dist));
        }
        0
    });
    Ok(handle)
}

/// Open the page: through the `DAY_WEB_DRIVER` command when set (a scripted/CI browser that
/// also answers screenshot requests — see [`driver_screenshot`]), else the default browser.
/// The driver is spawned as `<cmd…> <url> <control-port>` and serves `GET /screenshot` (PNG)
/// and `GET /quit` on the control port.
fn open_page(url: &str) -> Result<(), String> {
    let Ok(driver) = std::env::var("DAY_WEB_DRIVER") else {
        open_in_browser(url);
        return Ok(());
    };
    // Reserve a loopback port for the driver's control server (bind-then-drop; the driver
    // rebinds it immediately).
    let control = TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map_err(|e| format!("driver control port: {e}"))?
        .port();
    let mut words = driver.split_whitespace();
    let program = words.next().ok_or("DAY_WEB_DRIVER is empty")?;
    // A previous variant's browser (capture matrix) shows the OLD page — retire it first, or
    // its control port would keep answering screenshot requests with stale pixels.
    stop_driver();
    let child = Command::new(program)
        .args(words)
        .arg(url)
        .arg(control.to_string())
        .spawn()
        .map_err(|e| format!("DAY_WEB_DRIVER {driver:?}: {e}"))?;
    crate::signals::register_child(child.id());
    *DRIVER.lock().expect("driver slot") = Some((control, child));
    status("Driver", &format!("{driver} (control port {control})"));
    Ok(())
}

fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let opener = Command::new("open").arg(url).status();
    #[cfg(target_os = "linux")]
    let opener = Command::new("xdg-open").arg(url).status();
    #[cfg(target_os = "windows")]
    let opener = Command::new("cmd").args(["/C", "start", "", url]).status();
    if opener.is_err() {
        status("Open", url);
    }
}

/// Answer one HTTP request: static GETs resolved strictly inside `dist`, plus the two dynamic
/// paths — the `/dayscript` WebSocket and the `/day-http-ok` echo endpoint.
fn serve_one(mut stream: TcpStream, dist: &Path) {
    let mut buf = [0u8; 4096];
    let n = match stream.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return,
    };
    let head = String::from_utf8_lossy(&buf[..n]).into_owned();
    let path = head
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/");
    if path == "/dayscript" {
        // The page's dayscript WebSocket (docs/web.md) — this thread becomes its pump.
        serve_dayscript_ws(stream, &buf[..n]);
        return;
    }
    if path == "/day-http-ok" {
        // day-part-http's same-origin demo endpoint (docs/web.md): a browser tab can host no
        // loopback listener, so apps whose HTTP demo would spin one (the showcase's Platform
        // services page) fetch this path instead. Same bodies as that native one-shot server —
        // GET answers `day-http-ok`, any other method echoes `day-http-ok:<METHOD>` — so
        // walkthrough asserts are byte-identical on web.
        let method = head.split_whitespace().next().unwrap_or("GET");
        let body = if method == "GET" {
            "day-http-ok".to_string()
        } else {
            format!("day-http-ok:{method}")
        };
        let _ = stream.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        );
        return;
    }
    match resolve(dist, path).and_then(|p| std::fs::read(&p).ok().map(|b| (p, b))) {
        Some((p, body)) => {
            let mime = mime_of(&p);
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            );
            let _ = stream.write_all(&body);
        }
        None => {
            let _ = stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    }
}

/// Map a request path to a file inside `dist`, rejecting anything that could escape it.
fn resolve(dist: &Path, path: &str) -> Option<PathBuf> {
    let rel = path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel
        .split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return None;
    }
    let p = dist.join(rel);
    p.is_file().then_some(p)
}

fn mime_of(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        // Required exactly: `WebAssembly.instantiateStreaming` refuses other types.
        "wasm" => "application/wasm",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
}

// ---------------------------------------------------------------------------
// dayscript bridge (docs/web.md, §14.5): the runner speaks its usual newline-JSON TCP
// protocol to DAYSCRIPT_PORT; the page speaks WebSocket to `/dayscript` on the dev server;
// this bridge pipes lines between the two. The engine in the wasm validates the token, so
// the bridge itself is a dumb pipe.
// ---------------------------------------------------------------------------

/// The page half of the bridge: the accepted `/dayscript` WebSocket (write side).
static PAGE_WS: std::sync::Mutex<Option<TcpStream>> = std::sync::Mutex::new(None);
/// The runner half: the accepted DAYSCRIPT_PORT connection (write side, for replies).
static RUNNER: std::sync::Mutex<Option<TcpStream>> = std::sync::Mutex::new(None);
/// The `DAY_WEB_DRIVER` control port + child of the CURRENT launch. A `Mutex<Option<…>>`, not a
/// `OnceLock`: a capture-matrix launch (`--themes`/`--locales`) opens the page once per variant
/// IN ONE PROCESS, and a once-only slot would leave every later screenshot request talking to
/// the FIRST variant's browser — silently capturing the wrong theme and locale.
#[allow(clippy::type_complexity)]
static DRIVER: std::sync::Mutex<Option<(u16, std::process::Child)>> = std::sync::Mutex::new(None);

/// Ports this process already runs a dayscript bridge on. The bridge listener is a forever
/// thread; a second launch on the same port in the same process (again: the capture matrix)
/// must REUSE it — rebinding is EADDRINUSE — and the accept loop already hands each new runner
/// connection and page WebSocket to the current slots.
static BRIDGED: std::sync::Mutex<Option<std::collections::HashSet<u16>>> =
    std::sync::Mutex::new(None);

/// Accept runner connections on the dayscript port and forward each request line to the
/// page's WebSocket (waiting for the page to connect — it is still loading when the runner's
/// first step arrives).
fn start_runner_bridge(port: u16) -> Result<(), String> {
    {
        let mut bridged = BRIDGED.lock().expect("bridged ports");
        let set = bridged.get_or_insert_with(std::collections::HashSet::new);
        if !set.insert(port) {
            // Already ours: the listener thread below is still accepting, and the next runner
            // connection and page WebSocket will replace the RUNNER/PAGE_WS slots.
            return Ok(());
        }
    }
    let listener =
        TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("dayscript bridge: {e}"))?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            *RUNNER.lock().expect("runner slot") = stream.try_clone().ok();
            let mut reader = std::io::BufReader::new(stream);
            let mut line = String::new();
            loop {
                line.clear();
                match std::io::BufRead::read_line(&mut reader, &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                if !forward_to_page(line.trim_end()) {
                    // The page never connected (or died): answer for it so the runner fails
                    // with a diagnosis instead of a timeout.
                    let reply = "{\"ok\":false,\"error\":\"web page not connected (dayscript bridge)\",\"retryable\":false}\n";
                    if let Some(r) = RUNNER.lock().expect("runner slot").as_mut() {
                        let _ = r.write_all(reply.as_bytes());
                    }
                }
            }
        }
    });
    Ok(())
}

/// Send one request line to the page as a WebSocket text frame, waiting up to ~20 s for the
/// page to connect first. False when it never did or the write failed.
fn forward_to_page(line: &str) -> bool {
    for _ in 0..80 {
        {
            let mut slot = PAGE_WS.lock().expect("page slot");
            if let Some(ws) = slot.as_mut() {
                if ws_send_text(ws, line).is_ok() {
                    return true;
                }
                *slot = None; // page went away; wait for a reconnect
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    false
}

/// Upgrade an accepted HTTP connection to the `/dayscript` WebSocket and pump its text
/// frames (engine reply lines) to the runner. `head` is the raw request bytes already read;
/// any bytes past the header end are the start of the frame stream.
fn serve_dayscript_ws(mut stream: TcpStream, head: &[u8]) {
    let text = String::from_utf8_lossy(head);
    let Some(key) = text.lines().find_map(|l| {
        l.split_once(':').and_then(|(name, v)| {
            name.eq_ignore_ascii_case("sec-websocket-key")
                .then(|| v.trim().to_string())
        })
    }) else {
        return;
    };
    let accept = crate::script::b64encode_public(&sha1(
        format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
    ));
    let resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    if stream.write_all(resp.as_bytes()).is_err() {
        return;
    }
    let leftover = head
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| head[i + 4..].to_vec())
        .unwrap_or_default();
    *PAGE_WS.lock().expect("page slot") = stream.try_clone().ok();
    ws_read_loop(stream, leftover);
}

/// Read client frames forever: text frames carry engine reply lines for the runner; ping is
/// answered; close (or a read error) ends the connection.
fn ws_read_loop(mut stream: TcpStream, mut pending: Vec<u8>) {
    let mut message: Vec<u8> = Vec::new();
    loop {
        let mut hdr = [0u8; 2];
        if ws_read_exact(&mut stream, &mut hdr, &mut pending).is_err() {
            return;
        }
        let fin = hdr[0] & 0x80 != 0;
        let opcode = hdr[0] & 0x0f;
        let masked = hdr[1] & 0x80 != 0;
        let mut len = (hdr[1] & 0x7f) as u64;
        if len == 126 {
            let mut ext = [0u8; 2];
            if ws_read_exact(&mut stream, &mut ext, &mut pending).is_err() {
                return;
            }
            len = u64::from(u16::from_be_bytes(ext));
        } else if len == 127 {
            let mut ext = [0u8; 8];
            if ws_read_exact(&mut stream, &mut ext, &mut pending).is_err() {
                return;
            }
            len = u64::from_be_bytes(ext);
        }
        if len > 16 * 1024 * 1024 {
            return; // a reply line should never be this large — refuse
        }
        let mut mask = [0u8; 4];
        if masked && ws_read_exact(&mut stream, &mut mask, &mut pending).is_err() {
            return;
        }
        let mut payload = vec![0u8; len as usize];
        if ws_read_exact(&mut stream, &mut payload, &mut pending).is_err() {
            return;
        }
        if masked {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
        }
        match opcode {
            0x8 => return, // close
            0x9 => {
                // ping → pong (unfragmented control frame)
                let mut pong = vec![0x8a, payload.len() as u8];
                pong.extend_from_slice(&payload);
                if stream.write_all(&pong).is_err() {
                    return;
                }
            }
            0x1 | 0x0 => {
                message.extend_from_slice(&payload);
                if fin {
                    let line = String::from_utf8_lossy(&message).into_owned();
                    message.clear();
                    if let Some(r) = RUNNER.lock().expect("runner slot").as_mut() {
                        let mut out = line.trim_end().to_string();
                        out.push('\n');
                        let _ = r.write_all(out.as_bytes());
                    }
                }
            }
            _ => {} // pong / reserved: ignore
        }
    }
}

/// Fill `buf` from the handshake leftover first, then the socket.
fn ws_read_exact(
    stream: &mut TcpStream,
    buf: &mut [u8],
    pending: &mut Vec<u8>,
) -> std::io::Result<()> {
    let from_pending = pending.len().min(buf.len());
    buf[..from_pending].copy_from_slice(&pending[..from_pending]);
    pending.drain(..from_pending);
    if from_pending < buf.len() {
        std::io::Read::read_exact(stream, &mut buf[from_pending..])?;
    }
    Ok(())
}

/// Send one unmasked server→client text frame.
fn ws_send_text(stream: &mut TcpStream, text: &str) -> std::io::Result<()> {
    let payload = text.as_bytes();
    let mut frame = Vec::with_capacity(payload.len() + 10);
    frame.push(0x81);
    match payload.len() {
        n if n < 126 => frame.push(n as u8),
        n if n < 65536 => {
            frame.push(126);
            frame.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            frame.push(127);
            frame.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(payload);
    stream.write_all(&frame)
}

/// SHA-1, for the WebSocket accept key only (RFC 6455 mandates it; this is not used for any
/// security purpose beyond the handshake's anti-cache echo).
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    let ml = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&ml.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// Driver control (script.rs): screenshots + teardown for the DAY_WEB_DRIVER browser.
// ---------------------------------------------------------------------------

/// Fetch a PNG of the page from the driver's control server and write it to `path`.
pub(crate) fn driver_screenshot(path: &Path) -> Result<(), String> {
    let port = match DRIVER.lock().expect("driver slot").as_ref() {
        Some((port, _)) => *port,
        None => {
            return Err("no web driver (set DAY_WEB_DRIVER to a browser driver command)".into());
        }
    };
    let body = control_get(port, "/screenshot")?;
    if body.is_empty() {
        return Err("web driver returned an empty screenshot".into());
    }
    std::fs::write(path, &body).map_err(|e| format!("{}: {e}", path.display()))
}

/// Stop the driver browser (end of a scripted run): ask it to quit, then reap the child.
pub(crate) fn stop_driver() {
    if let Some((port, mut child)) = DRIVER.lock().expect("driver slot").take() {
        let _ = control_get(port, "/quit");
        let _ = child.wait();
    }
}

/// Minimal HTTP GET against the driver's loopback control server (Connection: close).
fn control_get(port: u16, path: &str) -> Result<Vec<u8>, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| format!("driver: {e}"))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .ok();
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .map_err(|e| format!("driver: {e}"))?;
    let mut all = Vec::new();
    std::io::Read::read_to_end(&mut stream, &mut all).map_err(|e| format!("driver: {e}"))?;
    let split = all
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("driver: malformed response")?;
    Ok(all[split + 4..].to_vec())
}
