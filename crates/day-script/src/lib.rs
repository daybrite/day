//! day-script — the embedded dayscript engine (DESIGN.md §14). Bind-only-when-invited: the
//! server starts ONLY when DAYSCRIPT_PORT + DAYSCRIPT_TOKEN are present in the environment
//! (never otherwise), listens on 127.0.0.1, and accepts only the step catalog. Steps execute
//! as synthesized Day events on the main thread between flushes — deterministic and
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
// Wire protocol (shared with the Day CLI runner)
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
        /// Upper bound in seconds for the implicit retry wait (§14.3) — for elements that
        /// appear only after slow work (a login round-trip, a first sync). Defaults to the
        /// shared step timeout.
        #[serde(default)]
        timeout_secs: Option<f64>,
    },
    WaitIdle,
    /// Programmatic scroll (docs/scroll.md §dayscript). With `edge`/`x`+`y`, `id` must name a
    /// `scroll` piece; with neither, `id` names ANY element and its nearest enclosing scroll
    /// reveals it. Unanimated, so the next step sees the settled position.
    ScrollTo {
        id: String,
        /// `"top"` | `"bottom"` | `"leading"` | `"trailing"`.
        #[serde(default)]
        edge: Option<String>,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
    },
    Tap {
        id: String,
        #[serde(default)]
        repeat: Option<u32>,
    },
    /// Deliver `Event::Submitted` to the element — the scripted stand-in for the platform's
    /// submit gesture (Enter in a `text_area` with `.on_submit`, a field's return key).
    Submit {
        id: String,
    },
    Input {
        id: String,
        #[serde(default)]
        text: Option<String>,
        /// Localized alternative to `text`: resolve this Fluent key (with `args`) in the RUN'S
        /// locale and type the result — locale-portable queries (e.g. a localized fruit name).
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        args: Option<BTreeMap<String, serde_json::Value>>,
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
    /// Drag-reorder a list row programmatically: row `from` drops at row `to` through the same
    /// guard → commit path a native drag takes (docs/list.md) — the app's `reorder_guard` may
    /// deny or retarget it. Fails (non-retryably) when the list isn't `.reorderable()` or the
    /// guard denies the move.
    Reorder {
        id: String,
        from: usize,
        to: usize,
    },
    /// Invoke an app-menu item programmatically (docs/menus.md): match a unique `Action`
    /// leaf in the installed app-menu model by exact `item` label, or by `key` — a Fluent
    /// key resolved in the run's locale (locale-portable; a standard-role item also
    /// matches its role's core-catalog key, so the auto Preferences item is
    /// `key: day-preferences`). `path` disambiguates with ancestor submenu labels (suffix
    /// match). Items that run a native selector instead of a day action (role items with
    /// id 0) are not invokable this way.
    Menu {
        #[serde(default)]
        item: Option<String>,
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        path: Option<Vec<String>>,
    },
    /// Drive a window-toolbar item by its id (docs/toolbars.md). With neither `text:` nor
    /// `on:` this runs a button's command; `text:` types into a search item; `on:` sets a
    /// toggle. Each goes through the same dispatch the native control fires, so it exercises
    /// the app's wiring — it does not prove the native widget drew (a screenshot does).
    Toolbar {
        item: String,
        #[serde(default)]
        text: Option<String>,
        /// Localized alternative to `text`, exactly as [`Step::Input`] takes one: resolve this
        /// Fluent key (with `args`) in the RUN'S locale and type the result. A toolbar search
        /// field that filters on localized text needs this — a literal query written in English
        /// matches nothing once the run switches locale.
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        args: Option<BTreeMap<String, serde_json::Value>>,
        #[serde(default)]
        on: Option<bool>,
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
    /// Fail if any piece kind rendered a `⟨kind⟩` placeholder — i.e. the backend had no renderer
    /// for it. Placeholders are invisible to every other assertion (the app still renders, the
    /// screenshot still looks plausible), so this is the only step that catches a missing or
    /// silently-dropped renderer. `allow` lists the kinds a target is expected to lack, which
    /// makes the script itself the per-target gap ledger; anything outside it is a failure.
    AssertNoPlaceholders {
        #[serde(default)]
        allow: Vec<String>,
    },
    /// Close the secondary window opened under `window` (`day::open_window`'s key —
    /// the preferences window is `day.preferences`), through the same async confirm →
    /// teardown path a title-bar close takes (docs/windows.md; on the cover-fallback tier
    /// this dismisses the cover). An already-closed window is a success (closing is
    /// idempotent).
    CloseWindow {
        window: String,
    },
    Screenshot {
        name: String,
        /// Capture the secondary window opened under this key (`day::open_window`'s `key`)
        /// instead of the primary (docs/windows.md). On the cover-fallback tier the key
        /// resolves to the primary window, whose fullscreen cover IS the content — same
        /// pixels, no special case. A missing key fails retryably (the window may still be
        /// opening).
        #[serde(default)]
        window: Option<String>,
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
    /// Answer the open modal: a button `index`, a prompt `text`, a file `path` (open/save
    /// pickers — relative paths resolve against the app temp dir, writable on every target), or
    /// `dismiss`.
    Respond {
        #[serde(default)]
        button: Option<i64>,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        dismiss: bool,
    },
    /// Diff the NATIVE accessibility tree against Day's expectations (role/label/value/identifier)
    /// for every id'd node, or just `id` (§13, §14.2). Backends that can't read their native tree
    /// (`found = false`) are skipped; role is only compared when both sides map to a known `Role`.
    A11yAudit {
        #[serde(default)]
        id: Option<String>,
    },
    /// Move native focus to the control — the real Toolkit duty, not a synthetic event, so
    /// keyboards and end-editing flows engage (docs/focus.md). `focused: false` resigns it.
    Focus {
        id: String,
        #[serde(default)]
        focused: Option<bool>,
    },
    /// Assert the control's focus state as Day resolved it (`NodeProbe.focused`; retryable —
    /// focus lands a turn after the request). `focused` defaults to `true`.
    AssertFocused {
        id: String,
        #[serde(default)]
        focused: Option<bool>,
    },
    /// Expect the app to TERMINATE — the only step that tolerates the app dying (docs/break.md's
    /// crash-reporting flow, docs/agent.md). MUST be the last step: a preceding step triggered an
    /// intentional exit/crash, and `expect_exit` treats the connection dropping within `within`
    /// seconds (default 15) as success; the app surviving the window is the failure. Handled
    /// runner-side (`day-cli`), so the in-app engine never executes it — this arm is defensive.
    ExpectExit {
        #[serde(default)]
        within: Option<f64>,
    },
}

impl Step {
    /// The implicit-wait budget for this step, seconds: its own `timeout_secs` when declared
    /// (and positive), else the shared [`DEFAULT_TIMEOUT_SECS`].
    fn wait_budget_secs(&self) -> f64 {
        match self {
            Step::WaitFor {
                timeout_secs: Some(t),
                ..
            } if *t > 0.0 => *t,
            _ => DEFAULT_TIMEOUT_SECS,
        }
    }
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

// ---------------------------------------------------------------------------
// Web transport (docs/web.md): the host page pipes newline-JSON request lines from a
// WebSocket into [`web_line`], and replies leave through the sender given to [`web_init`]
// (the day-cli dev server bridges that WebSocket to the same TCP protocol the runner
// already speaks, so `day drive`/`--script` are unchanged). Everything runs on the one
// wasm thread: the implicit bounded wait reschedules through the delayed poster instead
// of sleeping, and there is no `Instant` (it traps on wasm) — attempts are counted.
// ---------------------------------------------------------------------------

/// Retry cadence of the implicit bounded wait, shared by both transports.
const RETRY_MS: u32 = 100;

#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use std::cell::RefCell;

    /// One reply line out to the page (installed by [`web_init`]).
    type WebSender = Box<dyn Fn(&str)>;

    thread_local! {
        /// (token, reply sender) once [`web_init`] ran; requests before/without it are dropped.
        static WEB: RefCell<Option<(String, WebSender)>> = const { RefCell::new(None) };
    }

    /// Arm the web transport: `token` authenticates each request (the query-parameter
    /// spelling of `DAYSCRIPT_TOKEN`), `send` carries one reply line back to the page.
    pub fn web_init(token: String, send: impl Fn(&str) + 'static) {
        WEB.with(|w| *w.borrow_mut() = Some((token, Box::new(send))));
    }

    /// One request line from the page. Executes on the main thread now; a retryable failure
    /// reschedules itself until the shared step timeout is spent, then the reply goes out
    /// through the sender. Requests are answered in order because the runner awaits each
    /// reply before sending the next step.
    pub fn web_line(line: &str) {
        let reply = match serde_json::from_str::<Request>(line.trim()) {
            Ok(req) => {
                let authed = WEB.with(|w| {
                    w.borrow()
                        .as_ref()
                        .map(|(t, _)| *t == req.token)
                        .unwrap_or(false)
                });
                if authed {
                    let attempts = (req.step.wait_budget_secs() * 1000.0 / RETRY_MS as f64) as u32;
                    attempt(req.step, attempts);
                    return;
                }
                Reply::fail("bad token", false)
            }
            Err(e) => Reply::fail(format!("bad request: {e}"), false),
        };
        send_reply(reply);
    }

    fn attempt(step: Step, attempts_left: u32) {
        let reply = exec(step.clone());
        if reply.ok || !reply.retryable || attempts_left == 0 {
            send_reply(reply);
            return;
        }
        day_reactive::on_main_delayed(RETRY_MS, move || attempt(step, attempts_left - 1));
    }

    fn send_reply(reply: Reply) {
        let mut out = serde_json::to_string(&reply).unwrap_or_else(|_| "{\"ok\":false}".into());
        out.push('\n');
        WEB.with(|w| {
            if let Some((_, send)) = w.borrow().as_ref() {
                send(&out);
            }
        });
    }
}
#[cfg(target_arch = "wasm32")]
pub use web::{web_init, web_line};

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

/// Implicit bounded wait (§14.3): retryable failures poll on the main thread until timeout —
/// the shared default, or the step's own `timeout_secs` where it declares one.
fn run_step_with_wait(step: Step) -> Reply {
    let deadline = Instant::now() + Duration::from_secs_f64(step.wait_budget_secs());
    loop {
        let reply = run_on_main(step.clone());
        if reply.ok || !reply.retryable || Instant::now() > deadline {
            return reply;
        }
        std::thread::sleep(Duration::from_millis(u64::from(RETRY_MS)));
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
// Step execution (main thread; events go through the normal Day path)
// ---------------------------------------------------------------------------

/// Resolve a Fluent key (+ JSON args) in the current locale — the shared engine for the
/// `key:`-flavored script fields (assert_text, input).
/// Walk the app-menu model for `Action` leaves matching the `menu:` step's target: by
/// exact label, or — when the step used `key:` — by the role's core-catalog key (the
/// injected Preferences item carries an empty label; its role is its identity). `path`
/// filters by ancestor submenu labels (suffix match). Returns `(id, enabled, trail)`.
fn find_menu_actions(
    items: &[day_spec::MenuItem],
    target_label: &str,
    target_key: Option<&str>,
    path: &[(String, String)],
) -> Vec<(u64, bool, Vec<String>)> {
    // Mirrors day-pieces' role_catalog_key (docs/menus.md) — the stable `day-*` key set.
    fn role_key(role: day_spec::MenuRole) -> &'static str {
        use day_spec::MenuRole as R;
        match role {
            R::Cut => "day-cut",
            R::Copy => "day-copy",
            R::Paste => "day-paste",
            R::SelectAll => "day-select-all",
            R::Undo => "day-undo",
            R::Redo => "day-redo",
            R::Delete => "day-delete",
            R::About => "day-about",
            R::Quit => "day-quit",
            R::Preferences => "day-preferences",
            R::Minimize => "day-minimize",
            R::CloseWindow => "day-close",
            R::Fullscreen => "day-fullscreen",
            R::NewWindow => "day-new-window",
        }
    }
    fn walk(
        items: &[day_spec::MenuItem],
        trail: &mut Vec<String>,
        target_label: &str,
        target_key: Option<&str>,
        path: &[(String, String)],
        out: &mut Vec<(u64, bool, Vec<String>)>,
    ) {
        for it in items {
            match it {
                day_spec::MenuItem::Action {
                    id,
                    label,
                    enabled,
                    role,
                    ..
                } => {
                    let by_label = !label.is_empty() && label == target_label;
                    let by_key = target_key.is_some_and(|k| role.is_some_and(|r| role_key(r) == k));
                    let path_ok = path.is_empty()
                        || (path.len() <= trail.len()
                            && trail[trail.len() - path.len()..]
                                .iter()
                                .zip(path)
                                .all(|(seen, want)| seen == &want.0 || seen == &want.1));
                    if (by_label || by_key) && path_ok {
                        out.push((*id, *enabled, trail.clone()));
                    }
                }
                day_spec::MenuItem::Submenu { label, items, .. } => {
                    trail.push(label.clone());
                    walk(items, trail, target_label, target_key, path, out);
                    trail.pop();
                }
                day_spec::MenuItem::Separator => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(
        items,
        &mut Vec::new(),
        target_label,
        target_key,
        path,
        &mut out,
    );
    out
}

fn format_key(key: &str, args: Option<BTreeMap<String, serde_json::Value>>) -> String {
    let mut lt = day_fluent::tr(key);
    for (name, v) in args.unwrap_or_default() {
        lt = match v {
            serde_json::Value::Number(n) => lt.arg(&name, n.as_f64().unwrap_or(0.0)),
            serde_json::Value::String(s) => lt.arg(&name, s),
            other => lt.arg(&name, other.to_string()),
        };
    }
    lt.format()
}

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
            Step::WaitFor { id, .. } => {
                visible(&id)?;
                Ok(Reply::ok())
            }
            Step::WaitIdle => {
                day_reactive::flush_sync();
                Ok(Reply::ok())
            }
            Step::Tap { id, repeat } => {
                // Deliver a button `Pressed` AND a gesture `Tap` at the node's local centre, so one
                // step exercises buttons (which ignore `Tap`) and shape/`.on_tap` pieces (which
                // ignore `Pressed`) alike — the native recognizers deliver the same `Tap`.
                let node = find(&id)?;
                let center = with_tree(|t| t.node_frame(node))
                    .map(|f| day_spec::Point::new(f.size.width / 2.0, f.size.height / 2.0))
                    .unwrap_or(day_spec::Point::ZERO);
                for _ in 0..repeat.unwrap_or(1).max(1) {
                    day_core::enqueue_event(rnode_to_id(node), Event::Pressed);
                    day_core::enqueue_event(rnode_to_id(node), Event::Tap(center));
                }
                day_reactive::flush_sync();
                Ok(Reply::ok())
            }
            Step::Submit { id } => emit(&id, Event::Submitted).map(|()| Reply::ok()),
            Step::Input {
                id,
                text,
                key,
                args,
            } => {
                let value = match key {
                    Some(k) => format_key(&k, args),
                    None => text.unwrap_or_default(),
                };
                // Paint-then-event via the shared synthesizer (day-core): the widget must
                // SHOW the typed text, not only deliver it to the app's signal.
                let node = find(&id)?;
                day_core::synthesize_text(node, value);
                day_reactive::flush_sync();
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
            Step::Reorder { id, from, to } => {
                let node = find(&id)?;
                match day_core::list_try_reorder(node, from, to) {
                    Ok(_) => Ok(Reply::ok()),
                    // Not retryable: a guard denial or a non-reorderable list won't change by
                    // waiting — surface it to the runner immediately.
                    Err(e) => Err(Reply::fail(
                        format!("reorder {id:?} {from}->{to}: {e}"),
                        false,
                    )),
                }
            }
            Step::Menu { item, key, path } => {
                let target_label = match (&item, &key) {
                    (Some(l), _) => l.clone(),
                    (None, Some(k)) => format_key(k, None),
                    (None, None) => {
                        return Err(Reply::fail("menu: needs `item:` or `key:`", false));
                    }
                };
                // Each `path:` entry matches an ancestor submenu by its literal label OR by
                // its Fluent key resolved in the run's locale, so `path: [menu_file]` works
                // wherever `key: menu_file` does.
                let path: Vec<(String, String)> = path
                    .unwrap_or_default()
                    .into_iter()
                    .map(|p| {
                        let resolved = format_key(&p, None);
                        (p, resolved)
                    })
                    .collect();
                let matches = find_menu_actions(
                    &day_core::menu::app_menu_model(),
                    &target_label,
                    key.as_deref(),
                    &path,
                );
                // The AUTO items (docs/windows.md) exist even when the app never installed a
                // menu (the backend's default menu carries them): resolve their keys straight
                // to the registered dispatch actions when the model has no entry.
                let auto_id = match (matches.is_empty(), key.as_deref()) {
                    (true, Some("day-preferences")) => {
                        Some(day_core::windows::preferences_action_id())
                    }
                    (true, Some("day-new-window")) => {
                        Some(day_core::windows::new_window_action_id())
                    }
                    _ => None,
                };
                if let Some(id) = auto_id
                    && id != 0
                {
                    day_core::dispatch_menu_action(id);
                    day_reactive::flush_sync();
                    return Ok(Reply::ok());
                }
                match matches.as_slice() {
                    [] => Err(Reply::fail(
                        // Retryable: the (reactive) app menu may not have installed yet.
                        format!("menu: no item {target_label:?}"),
                        true,
                    )),
                    [(id, enabled, _)] => {
                        if *id == 0 {
                            Err(Reply::fail(
                                format!(
                                    "menu: {target_label:?} runs a native selector (no day \
                                     action) — not invokable from dayscript"
                                ),
                                false,
                            ))
                        } else if !enabled {
                            Err(Reply::fail(
                                format!("menu: {target_label:?} is disabled"),
                                false,
                            ))
                        } else {
                            day_core::dispatch_menu_action(*id);
                            day_reactive::flush_sync();
                            Ok(Reply::ok())
                        }
                    }
                    many => Err(Reply::fail(
                        format!(
                            "menu: {target_label:?} is ambiguous — disambiguate with path: {:?}",
                            many.iter()
                                .map(|(_, _, p)| p.join(" ▸ "))
                                .collect::<Vec<_>>()
                        ),
                        false,
                    )),
                }
            }
            Step::Toolbar {
                item,
                text,
                key,
                args,
                on,
            } => {
                // `key` resolves through the run's locale, like `input`'s does; `text` stays a
                // literal. Resolved BEFORE the item lookup so a bad key fails as a bad key.
                let text = match key {
                    Some(k) => Some(format_key(&k, args)),
                    None => text,
                };
                let model = day_core::toolbar::primary_toolbar_model();
                let Some(found) = model.iter().find(|i| i.id == item) else {
                    // Retryable: a reactive toolbar may not have installed yet.
                    return Err(Reply::fail(format!("toolbar: no item {item:?}"), true));
                };
                if !found.enabled {
                    return Err(Reply::fail(format!("toolbar: {item:?} is disabled"), false));
                }
                if found.action == 0 {
                    return Err(Reply::fail(
                        format!("toolbar: {item:?} has no command"),
                        false,
                    ));
                }
                match (text, on) {
                    (Some(t), _) => day_core::toolbar::dispatch_toolbar_value(
                        found.action,
                        &day_spec::ToolbarValue::Text(t),
                    ),
                    (None, Some(v)) => day_core::toolbar::dispatch_toolbar_value(
                        found.action,
                        &day_spec::ToolbarValue::On(v),
                    ),
                    (None, None) => day_core::dispatch_menu_action(found.action),
                }
                day_reactive::flush_sync();
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
                    norm(&format_key(&k, args))
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
            Step::AssertNoPlaceholders { allow } => {
                let unexpected: Vec<&str> = day_spec::placeholder::seen()
                    .into_iter()
                    .filter(|k| !allow.iter().any(|a| a == k))
                    .collect();
                if unexpected.is_empty() {
                    Ok(Reply::ok())
                } else {
                    Err(Reply::fail(
                        format!(
                            "rendered a placeholder for {} — no renderer on this backend. Enable \
                             the piece's feature for this toolkit, or add the kind to `allow:` if \
                             the gap is intended.",
                            unexpected.join(", ")
                        ),
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
            Step::ScrollTo { id, edge, x, y } => {
                let node = find(&id)?;
                let target = match (edge.as_deref(), x, y) {
                    (Some("top"), _, _) => day_core::ScrollTarget::Top,
                    (Some("bottom"), _, _) => day_core::ScrollTarget::Bottom,
                    (Some("leading"), _, _) => day_core::ScrollTarget::Leading,
                    (Some("trailing"), _, _) => day_core::ScrollTarget::Trailing,
                    (Some(other), _, _) => {
                        return Err(Reply::fail(
                            format!(
                                "scroll_to: unknown edge {other:?} (top|bottom|leading|trailing)"
                            ),
                            false,
                        ));
                    }
                    (None, None, None) => {
                        // Reveal the element in its nearest enclosing scroll.
                        let ok = with_tree(|t| t.scroll_reveal(node, false));
                        if !ok {
                            return Err(Reply::fail(
                                format!("scroll_to: {id:?} has no enclosing scroll"),
                                true,
                            ));
                        }
                        day_reactive::flush_sync();
                        return Ok(Reply::ok());
                    }
                    (None, x, y) => day_core::ScrollTarget::Offset(day_spec::Point::new(
                        x.unwrap_or(0.0),
                        y.unwrap_or(0.0),
                    )),
                };
                let ok = with_tree(|t| t.scroll_to_target(node, &target, false));
                if !ok {
                    return Err(Reply::fail(
                        format!("scroll_to: {id:?} is not a realized scroll piece"),
                        true,
                    ));
                }
                day_reactive::flush_sync();
                Ok(Reply::ok())
            }
            Step::Focus { id, focused } => {
                let node = find(&id)?;
                with_tree(|t| t.focus_node(node, focused.unwrap_or(true)));
                day_reactive::flush_sync();
                Ok(Reply::ok())
            }
            Step::AssertFocused { id, focused } => {
                let want = focused.unwrap_or(true);
                let got = probe(&id)?.focused;
                if got == want {
                    Ok(Reply::ok())
                } else {
                    Err(Reply::fail(
                        format!("{id:?}: expected focused={want}, found focused={got}"),
                        true,
                    ))
                }
            }
            Step::CloseWindow { window } => {
                if let Some(handle) = day_core::window_by_key(&window) {
                    handle.close();
                }
                day_reactive::flush_sync();
                Ok(Reply::ok())
            }
            Step::Screenshot { window, .. } => {
                // Wait (retryable, bounded by the step timeout) for native transitions to
                // settle so the capture never shows a half-dismissed dialog or mid-push page.
                if !with_tree(|t| t.ui_idle()) {
                    return Err(Reply::fail("ui transitions still settling", true));
                }
                let png = match window.as_deref() {
                    Some(key) => match day_core::windows::window_root_by_key(key) {
                        Some(root) => with_tree(|t| t.snapshot_of(root)),
                        // Retryable: the window may still be opening (a Pending native
                        // creation, or the opening tap's turn not yet drained).
                        None => return Err(Reply::fail(format!("no window {key:?}"), true)),
                    },
                    None => with_tree(|t| t.snapshot()),
                };
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
            Step::ExpectExit { within } => {
                // Runner-side (day-cli watches for the connection to drop); the engine only reaches
                // this arm if the step was somehow delivered — treat it as a no-op success.
                let _ = within;
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
                path,
                dismiss,
            } => {
                let Some((req, _)) = day_core::pending_presentation() else {
                    return Err(Reply::fail("no modal to respond to", true));
                };
                let result = if dismiss {
                    PresentResult::Dismissed
                } else if let Some(t) = text {
                    PresentResult::Text(t)
                } else if let Some(p) = path {
                    // Relative paths resolve against the app-writable temp dir so a scripted
                    // open/save round-trip works on desktop, iOS, and Android alike.
                    let pb = std::path::PathBuf::from(&p);
                    let full = if pb.is_absolute() {
                        pb
                    } else {
                        day_core::app_temp_dir().join(pb)
                    };
                    PresentResult::Files(vec![full.to_string_lossy().into_owned()])
                } else if let Some(i) = button {
                    PresentResult::Button(i)
                } else {
                    PresentResult::Dismissed
                };
                day_core::respond_presentation(req, result);
                day_reactive::flush_sync();
                Ok(Reply::ok())
            }
            Step::A11yAudit { id } => {
                let rows = with_tree(|t| t.a11y_nodes());
                let rows: Vec<_> = match &id {
                    Some(want) => rows.into_iter().filter(|(nid, ..)| nid == want).collect(),
                    None => rows,
                };
                if let Some(want) = &id
                    && rows.is_empty()
                {
                    return Err(Reply::fail(
                        format!("a11y_audit: no element {want:?}"),
                        true,
                    ));
                }
                let mut fails = Vec::new();
                let mut checked = 0usize;
                for (nid, _kind, expected, actual) in &rows {
                    if !actual.found {
                        continue; // backend can't read its native a11y tree (non-apple) — skip
                    }
                    checked += 1;
                    if actual.identifier.as_deref() != Some(nid.as_str()) {
                        fails.push(format!(
                            "{nid}: native identifier {:?} ≠ {nid:?}",
                            actual.identifier
                        ));
                    }
                    // Role: audit only EXPLICIT (user-set) roles — the canvas/custom cases where
                    // Day actually applies a role. Native controls own their own roles (Day's job
                    // is to not break them, §13), and those vary per platform, so we don't diff
                    // the kind-default. Compare only when the native role also maps to a known Role.
                    if expected.role != day_spec::Role::None
                        && actual.role != day_spec::Role::None
                        && !role_eq(expected.role, actual.role)
                    {
                        fails.push(format!(
                            "{nid}: role {:?} ≠ expected {:?}",
                            actual.role, expected.role
                        ));
                    }
                    if let Some(lbl) = &expected.label
                        && actual.label.as_deref() != Some(lbl.as_str())
                    {
                        fails.push(format!(
                            "{nid}: label {:?} ≠ expected {lbl:?}",
                            actual.label
                        ));
                    }
                    if let Some(val) = &expected.value
                        && actual.value.as_deref() != Some(val.as_str())
                    {
                        fails.push(format!(
                            "{nid}: value {:?} ≠ expected {val:?}",
                            actual.value
                        ));
                    }
                }
                if !fails.is_empty() {
                    return Err(Reply::fail(
                        format!("a11y_audit: {}", fails.join("; ")),
                        false,
                    ));
                }
                if checked == 0 {
                    // No node could be read natively — treat as unsupported, not a pass/fail.
                    return Ok(Reply::ok());
                }
                Ok(Reply::ok())
            }
        }
    })();
    result.unwrap_or_else(|r| r)
}

/// Two roles match for audit purposes — `Heading` levels are ignored (the native role carries no level).
fn role_eq(a: day_spec::Role, b: day_spec::Role) -> bool {
    use day_spec::Role::Heading;
    matches!((a, b), (Heading(_), Heading(_))) || a == b
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
