// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The in-app test runner (docs/testing.md): runs the `#[day::test]` cases the binary
//! registers, each drive op as the dayscript step it names, and answers the `tests` and
//! `run_tests` steps `day test` sends. A run is one main-loop task: between a drive's awaited
//! ops the app's loop turns, so a native transition settles and a capture paints exactly as
//! they do between a script's steps.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use day_core::conformance::{
    Case, Drive, DriveBackend, DriveOp, Fail, NativeExpect, OpFuture, TestKind,
};
use serde::{Deserialize, Serialize};

use crate::{Reply, Step, b64encode, next_capture_revision};

pub use day_script_proto::ShotPolicy;

/// One test's outcome, the shape `evidence.json` records (docs/testing.md).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub name: String,
    pub kind: String,
    /// `pass`, `fail` or `skip`.
    pub verdict: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub ms: u64,
    pub proves: Vec<String>,
    /// The captures taken, by name; the PNG bytes ride beside them in [`RunReport::shots`].
    #[serde(default)]
    pub shots: Vec<String>,
    /// Native fields this case asked about that the toolkit could not read (`<id> <field>`):
    /// the case passed without them, and the evidence says so.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_unread: Vec<String>,
}

/// A capture a test took, for the runner to write beside the evidence.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Shot {
    pub test: String,
    pub name: String,
    pub png_base64: String,
}

/// What a finished run answers.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunReport {
    pub tests: Vec<Outcome>,
    #[serde(default)]
    pub shots: Vec<Shot>,
    /// What the toolkit answered for every capability during this run (`N`, `E` or `-`), keyed
    /// by name, so the evidence can be held against the declared coverage matrix.
    #[serde(default)]
    pub caps: std::collections::BTreeMap<String, String>,
}

struct Run {
    report: RunReport,
    done: bool,
}

thread_local! {
    static RUN: RefCell<Option<Rc<RefCell<Run>>>> = const { RefCell::new(None) };
}

/// What `tests` answers: every registered test, before any runs.
pub fn listing() -> serde_json::Value {
    let cases = day_core::conformance::cases();
    serde_json::Value::Array(
        cases
            .iter()
            .map(|c| {
                serde_json::json!({
                    "name": c.name(),
                    "kind": kind_name(c.kind()),
                    "proves": c.proves_keys(),
                    "requires": c.requirements().iter().map(|r| format!("{r:?}")).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

/// Answer a `run_tests` step: start the run on its first call, then report "running" until
/// the task finishes, when the report is handed over once and the run forgotten. Retryable
/// while running, so the runner's own wait loop polls it.
pub fn run_tests(
    filter: &[String],
    shots: ShotPolicy,
    case_timeout_secs: Option<f64>,
) -> Result<Reply, Reply> {
    let run = RUN.with(|r| r.borrow().clone());
    match run {
        None => {
            start(filter.to_vec(), shots, case_timeout_secs);
            Err(Reply::fail("tests running", true))
        }
        Some(run) => {
            if !run.borrow().done {
                return Err(Reply::fail("tests running", true));
            }
            RUN.with(|r| *r.borrow_mut() = None);
            let report = std::mem::take(&mut run.borrow_mut().report);
            let data = serde_json::to_value(&report)
                .map_err(|e| Reply::fail(format!("test report: {e}"), false))?;
            Ok(Reply {
                data: Some(data),
                ..Reply::ok()
            })
        }
    }
}

fn kind_name(kind: TestKind) -> &'static str {
    match kind {
        TestKind::Gui => "gui",
        TestKind::Headless => "headless",
    }
}

fn matches(filter: &[String], name: &str) -> bool {
    filter.is_empty() || filter.iter().any(|f| glob(f, name))
}

/// `*` matches any run of characters; nothing else is special.
fn glob(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let Some(first) = parts.next() else {
        return pattern == name;
    };
    if !pattern.contains('*') {
        return pattern == name;
    }
    if !name.starts_with(first) {
        return false;
    }
    let mut rest = &name[first.len()..];
    let parts: Vec<&str> = parts.collect();
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        let last = i + 1 == parts.len() && !pattern.ends_with('*');
        if last {
            return rest.ends_with(part);
        }
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    true
}

/// A case's time limit when neither it nor the run names one.
const DEFAULT_CASE_TIMEOUT_SECS: f64 = 30.0;

fn start(filter: Vec<String>, policy: ShotPolicy, case_timeout_secs: Option<f64>) {
    let run = Rc::new(RefCell::new(Run {
        report: RunReport::default(),
        done: false,
    }));
    run.borrow_mut().report.caps = day_spec::Cap::ALL
        .iter()
        .map(|cap| {
            let answer = match day_core::capability(*cap) {
                day_spec::Support::Native => "N",
                day_spec::Support::Emulated => "E",
                day_spec::Support::Unsupported => "-",
            };
            (format!("{cap:?}"), answer.to_owned())
        })
        .collect();
    RUN.with(|r| *r.borrow_mut() = Some(run.clone()));
    // Links open nothing during the run; they are recorded for `assert_opened_url`.
    day_core::conformance::set_intercepting_urls(true);
    day_core::task(async move {
        let cases: Vec<Case> = day_core::conformance::cases()
            .into_iter()
            .filter(|c| matches(&filter, c.name()))
            .collect();
        let mut gui = false;
        for (i, case) in cases.iter().enumerate() {
            let mut shots = Vec::new();
            // Two functions with one name (in two modules, or two crates) would leave the
            // report, the evidence and every tool that reads them unable to tell them apart.
            let twin = cases
                .iter()
                .enumerate()
                .any(|(j, c)| j != i && c.name() == case.name());
            let outcome = if twin {
                let mut o = outcome_for(case);
                o.verdict = "fail".into();
                o.message = Some(format!(
                    "two #[day::test] functions are named {}; rename one",
                    case.name()
                ));
                o
            } else {
                gui |= case.kind() == TestKind::Gui;
                run_case(case, policy, case_timeout_secs, &mut shots).await
            };
            let mut run = run.borrow_mut();
            run.report.shots.extend(shots);
            run.report.tests.push(outcome);
        }
        // The app's own content again, once, rather than between cases.
        if gui {
            day_core::conformance::clear_active();
        }
        day_core::conformance::set_intercepting_urls(false);
        run.borrow_mut().done = true;
    });
}

fn outcome_for(case: &Case) -> Outcome {
    Outcome {
        name: case.name().to_owned(),
        kind: kind_name(case.kind()).to_owned(),
        verdict: "pass".into(),
        message: None,
        reason: None,
        ms: 0,
        proves: case.proves_keys().to_vec(),
        shots: Vec::new(),
        native_unread: Vec::new(),
    }
}

async fn run_case(
    case: &Case,
    policy: ShotPolicy,
    run_timeout_secs: Option<f64>,
    shots: &mut Vec<Shot>,
) -> Outcome {
    let started = Stopwatch::start();
    let mut outcome = outcome_for(case);
    // A requirement the toolkit lacks is a skip with its reason, never a failure: this is the
    // one way a case adapts to a toolkit (docs/testing.md).
    if let Some(cap) = case
        .requirements()
        .iter()
        .find(|c| day_core::capability(**c) == day_spec::Support::Unsupported)
    {
        outcome.verdict = "skip".into();
        outcome.reason = Some(format!("Cap::{cap:?} is Unsupported on this toolkit"));
        return outcome;
    }
    let backend = Rc::new(Engine {
        shots: Rc::new(RefCell::new(Vec::new())),
        unread: Rc::new(RefCell::new(Vec::new())),
    });
    let drive = Drive::new(backend.clone());
    // A run's `case_timeout_secs: 0` turns limits off (a debugger holding a breakpoint);
    // otherwise the case's own limit, then the run's, then the default.
    let limit = match run_timeout_secs {
        Some(t) if t <= 0.0 => None,
        run => Some(
            case.timeout_secs()
                .or(run)
                .unwrap_or(DEFAULT_CASE_TIMEOUT_SECS),
        ),
    };
    let body = {
        let (case, drive) = (case.clone(), drive.clone());
        async move {
            if case.kind() == TestKind::Gui {
                // The test host shows THIS case's page alone, built fresh, sharing its signals
                // with the drive.
                if !day_core::conformance::set_active(case.clone()) {
                    return Err(Fail(
                        "the app shows no test host: root its test build in \
                         `day::test_host(..)` (docs/testing.md)"
                            .into(),
                    ));
                }
                day_reactive::flush_sync();
                drive.wait_idle().await?;
                if let Some(message) = day_core::conformance::take_page_panic() {
                    return Err(Fail(format!("the page panicked: {message}")));
                }
                for name in case.shots() {
                    drive.shot(name).await?;
                }
            }
            case.run_body(drive).await
        }
    };
    let result = Guarded::new(Box::pin(body), limit).await;
    if let Err(Fail(message)) = &result {
        outcome.verdict = "fail".into();
        outcome.message = Some(message.clone());
        if policy != ShotPolicy::Never {
            // What the screen showed when the assertion failed, the first thing anyone asks.
            let _ = drive.shot("failed").await;
        }
    }
    outcome.ms = started.elapsed_ms();
    outcome.native_unread = std::mem::take(&mut *backend.unread.borrow_mut());
    outcome.native_unread.dedup();
    let keep = match policy {
        ShotPolicy::Never => false,
        ShotPolicy::OnFailure => result.is_err(),
        ShotPolicy::Always => true,
    };
    if keep {
        for (name, png) in backend.shots.borrow_mut().drain(..) {
            outcome.shots.push(name.clone());
            shots.push(Shot {
                test: case.name().to_owned(),
                name,
                png_base64: b64encode(&png),
            });
        }
    }
    outcome
}

/// A case's run with its failures contained: a panic in the body becomes the case's failure
/// instead of the app's crash, and a body still pending at the limit fails as timed out. The
/// limit is a `day_core::sleep` raced against the body, so it is counted in the app's own
/// timers and needs no clock (std's `Instant` panics on wasm). Panics are caught where the
/// platform unwinds; a target built to abort on panic still aborts.
struct Guarded {
    body: Pin<Box<dyn Future<Output = Result<(), Fail>>>>,
    limit: Option<(f64, Pin<Box<day_core::Sleep>>)>,
}

impl Guarded {
    fn new(body: Pin<Box<dyn Future<Output = Result<(), Fail>>>>, secs: Option<f64>) -> Self {
        Guarded {
            body,
            limit: secs.map(|s| (s, Box::pin(day_core::sleep((s * 1000.0) as u32)))),
        }
    }
}

impl Future for Guarded {
    type Output = Result<(), Fail>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        let polled =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| this.body.as_mut().poll(cx)));
        match polled {
            Ok(Poll::Ready(result)) => return Poll::Ready(result),
            Err(payload) => {
                let message = day_core::conformance::panic_message(payload.as_ref());
                return Poll::Ready(Err(Fail(format!("panicked: {message}"))));
            }
            Ok(Poll::Pending) => {}
        }
        if let Some((secs, timer)) = &mut this.limit
            && timer.as_mut().poll(cx).is_ready()
        {
            return Poll::Ready(Err(Fail(format!("timed out after {secs} s"))));
        }
        Poll::Pending
    }
}

/// The engine's [`DriveBackend`]: each op is the dayscript step of the same name, executed on
/// the main thread and retried through the step's own wait window with the main loop turning
/// in between, so a drive sees the app settle the way a script does.
///
/// In a conformance run the Day-side assertions also check the native widget: `assert_text`,
/// `assert_value` and `assert_on` are each followed by the `assert_native` of the same fact,
/// so a pass means the platform's widget shows it too, not only Day's tree.
struct Engine {
    shots: Rc<RefCell<Vec<Capture>>>,
    /// `<id> <field>` for each native field the toolkit could not read.
    unread: Rc<RefCell<Vec<String>>>,
}

/// A capture a drive took: its name and the PNG.
type Capture = (String, Vec<u8>);

impl DriveBackend for Engine {
    fn run(&self, op: DriveOp) -> OpFuture {
        let (shots, unread) = (self.shots.clone(), self.unread.clone());
        Box::pin(async move {
            if let DriveOp::Pause(secs) = op {
                day_core::sleep((secs * 1000.0) as u32).await;
                return Ok(());
            }
            run_step(&op, &shots, &unread).await?;
            if let Some(native) = native_follow_up(&op) {
                run_step(&native, &shots, &unread).await?;
            }
            Ok(())
        })
    }
}

/// The native check that follows a Day-side assertion in a conformance run.
fn native_follow_up(op: &DriveOp) -> Option<DriveOp> {
    let (id, expect) = match op {
        DriveOp::AssertText(id, text) => (
            id,
            NativeExpect {
                text: Some(text.clone()),
                ..Default::default()
            },
        ),
        DriveOp::AssertValue(id, value) => (
            id,
            NativeExpect {
                number: Some(*value),
                ..Default::default()
            },
        ),
        DriveOp::AssertOn(id, on) => (
            id,
            NativeExpect {
                checked: Some(*on),
                ..Default::default()
            },
        ),
        _ => return None,
    };
    Some(DriveOp::AssertNative(id.clone(), expect))
}

/// Run one op as its dayscript step through the step's retry window.
async fn run_step(
    op: &DriveOp,
    shots: &Rc<RefCell<Vec<Capture>>>,
    unread: &Rc<RefCell<Vec<String>>>,
) -> Result<(), Fail> {
    let step = step_for(op)?;
    // The wait is the sleeps, or the clock where there is one, whichever says more: an attempt
    // can itself be slow (a capture), and counted in sleeps alone a failing one would retry past
    // the case's limit and report a timeout instead of its own message. The browser has no
    // `Instant` (std's panics on wasm), so there the sleeps are the whole count.
    let clock = Stopwatch::start();
    let mut waited_ms = 0u32;
    let budget_ms = (crate::DEFAULT_TIMEOUT_SECS * 1000.0) as u32;
    // One capture revision for the whole wait, as the socket runner keeps: a capture's
    // checkpoint is armed under it, and a fresh one per retry would never see it land.
    let revision = next_capture_revision();
    loop {
        let reply = crate::exec(step.clone(), revision);
        if reply.ok {
            if let (DriveOp::Shot(name), Some(png)) = (op, &reply.png_base64) {
                shots
                    .borrow_mut()
                    .push((name.clone(), crate::b64decode(png)));
            }
            let checked = match op {
                DriveOp::AssertNative(id, _)
                | DriveOp::AssertFrame(id, _)
                | DriveOp::SamplePixel(id, ..) => Some(id),
                _ => None,
            };
            if let (Some(id), Some(data)) = (checked, &reply.data) {
                let fields = data.get("unread").and_then(|u| u.as_array());
                for f in fields.into_iter().flatten().filter_map(|f| f.as_str()) {
                    unread.borrow_mut().push(format!("{id} {f}"));
                }
            }
            return Ok(());
        }
        let message = reply.error.unwrap_or_else(|| "failed".into());
        let elapsed_ms = u32::try_from(clock.elapsed_ms()).unwrap_or(u32::MAX);
        if !reply.retryable || waited_ms.max(elapsed_ms) >= budget_ms {
            return Err(Fail(format!("{}: {message}", op_name(op))));
        }
        let wait = if reply.capture_pending {
            16
        } else {
            crate::RETRY_MS
        };
        waited_ms += wait;
        day_core::sleep(wait).await;
    }
}

fn op_name(op: &DriveOp) -> String {
    match op {
        DriveOp::Tap(id) => format!("tap {id}"),
        DriveOp::Input(id, _) => format!("input {id}"),
        DriveOp::Toggle(id, _) => format!("toggle {id}"),
        DriveOp::SetValue(id, _) => format!("set_value {id}"),
        DriveOp::Select(id, _) => format!("select {id}"),
        DriveOp::Focus(id) => format!("focus {id}"),
        DriveOp::Submit(id) => format!("submit {id}"),
        DriveOp::Navigate(route) => format!("navigate {route}"),
        DriveOp::WaitIdle => "wait_idle".into(),
        DriveOp::Pause(_) => "pause".into(),
        DriveOp::Shot(name) => format!("shot {name}"),
        DriveOp::AssertText(id, _) => format!("assert_text {id}"),
        DriveOp::AssertVisible(id) => format!("assert_visible {id}"),
        DriveOp::AssertMissing(id) => format!("assert_missing {id}"),
        DriveOp::AssertHidden(id) => format!("assert_hidden {id}"),
        DriveOp::AssertValue(id, _) | DriveOp::AssertOn(id, _) => format!("assert_value {id}"),
        DriveOp::AssertFocused(id, _) => format!("assert_focused {id}"),
        DriveOp::AssertRoute(route) => format!("assert_route {route}"),
        DriveOp::A11yAudit(_) => "a11y_audit".into(),
        DriveOp::AssertNative(id, _) => format!("assert_native {id}"),
        DriveOp::AssertFrame(id, _) => format!("assert_frame {id}"),
        DriveOp::SamplePixel(id, ..) => format!("sample_pixel {id}"),
        DriveOp::AssertOpenedUrl(url) => format!("assert_opened_url {url}"),
        DriveOp::Activate(id, _) => format!("activate {id}"),
        DriveOp::Reorder(id, ..) => format!("reorder {id}"),
        DriveOp::DeleteRow(id, _) => format!("delete_row {id}"),
        DriveOp::SwipeRow(id, ..) => format!("swipe_row {id}"),
        DriveOp::ScrollTo(id, _) => format!("scroll_to {id}"),
        DriveOp::Expand(id, row, _) => format!("expand {id} {row}"),
        DriveOp::NavBack => "nav_back".into(),
        DriveOp::TreeMove(id, row, ..) => format!("tree_move {id} {row}"),
        DriveOp::ContextMenu(id, item) => format!("context_menu {id} {item}"),
        DriveOp::Menu(item) => format!("menu {item}"),
        DriveOp::ToolbarPress(item) | DriveOp::ToolbarToggle(item, _) => format!("toolbar {item}"),
        DriveOp::DialogScripted(_) => "dialog_mode".into(),
        DriveOp::AssertPresented(_) => "assert_presented".into(),
        DriveOp::Respond(_) => "respond".into(),
    }
}

/// The dayscript step an op stands for, built through the script format so the two can
/// never mean different things.
fn step_for(op: &DriveOp) -> Result<Step, Fail> {
    use serde_json::json;
    let v = match op {
        DriveOp::Tap(id) => json!({"op": "tap", "id": id}),
        DriveOp::Input(id, text) => json!({"op": "input", "id": id, "text": text}),
        DriveOp::Toggle(id, on) => json!({"op": "toggle", "id": id, "value": on}),
        DriveOp::SetValue(id, value) => json!({"op": "set_value", "id": id, "value": value}),
        DriveOp::Select(id, index) => json!({"op": "select", "id": id, "index": index}),
        DriveOp::Focus(id) => json!({"op": "focus", "id": id}),
        DriveOp::Submit(id) => json!({"op": "submit", "id": id}),
        DriveOp::Navigate(route) => json!({"op": "navigate", "route": route}),
        DriveOp::WaitIdle => json!({"op": "wait_idle"}),
        DriveOp::Pause(secs) => json!({"op": "pause", "secs": secs}),
        DriveOp::Shot(name) => json!({"op": "screenshot", "name": name, "in_process": true}),
        DriveOp::AssertText(id, text) => json!({"op": "assert_text", "id": id, "text": text}),
        DriveOp::AssertVisible(id) => json!({"op": "assert_visible", "id": id}),
        DriveOp::AssertMissing(id) => json!({"op": "assert_missing", "id": id}),
        DriveOp::AssertHidden(id) => json!({"op": "assert_hidden", "id": id}),
        DriveOp::AssertValue(id, value) => json!({"op": "assert_value", "id": id, "value": value}),
        DriveOp::AssertOn(id, on) => json!({"op": "assert_value", "id": id, "value": on}),
        DriveOp::AssertFocused(id, focused) => {
            json!({"op": "assert_focused", "id": id, "focused": focused})
        }
        DriveOp::AssertRoute(route) => json!({"op": "assert_route", "route": route}),
        DriveOp::A11yAudit(id) => json!({"op": "a11y_audit", "id": id}),
        DriveOp::AssertNative(id, e) => json!({
            "op": "assert_native", "id": id, "text": e.text, "number": e.number,
            "checked": e.checked, "enabled": e.enabled, "visible": e.visible,
        }),
        DriveOp::AssertFrame(id, f) => json!({
            "op": "assert_frame", "id": id, "width": f.width, "height": f.height,
            "x": f.x, "y": f.y, "relative_to": f.relative_to, "tolerance": f.tolerance,
        }),
        DriveOp::SamplePixel(id, x, y, color) => {
            json!({"op": "sample_pixel", "id": id, "x": x, "y": y, "color": color})
        }
        DriveOp::AssertOpenedUrl(url) => json!({"op": "assert_opened_url", "url": url}),
        DriveOp::Activate(id, index) => json!({"op": "activate", "id": id, "index": index}),
        DriveOp::Reorder(id, from, to) => {
            json!({"op": "reorder", "id": id, "from": from, "to": to})
        }
        DriveOp::DeleteRow(id, row) => json!({"op": "delete_row", "id": id, "row": row}),
        DriveOp::SwipeRow(id, row, leading, action) => json!({
            "op": "swipe_row", "id": id, "row": row, "action": action,
            "edge": if *leading { "leading" } else { "trailing" },
        }),
        DriveOp::ScrollTo(id, edge) => json!({"op": "scroll_to", "id": id, "edge": edge}),
        DriveOp::Expand(id, row, expanded) => {
            json!({"op": "expand", "id": id, "row": row, "expanded": expanded})
        }
        DriveOp::NavBack => json!({"op": "nav_back"}),
        DriveOp::TreeMove(id, row, parent, index) => json!({
            "op": "tree_move", "id": id, "row": row, "parent": parent, "index": index,
        }),
        DriveOp::ContextMenu(id, item) => json!({"op": "context_menu", "id": id, "item": item}),
        DriveOp::Menu(item) => json!({"op": "menu", "id": item}),
        DriveOp::ToolbarPress(item) => json!({"op": "toolbar", "item": item}),
        DriveOp::ToolbarToggle(item, on) => json!({"op": "toolbar", "item": item, "on": on}),
        DriveOp::DialogScripted(scripted) => json!({
            "op": "dialog_mode", "mode": if *scripted { "scripted" } else { "native" },
        }),
        DriveOp::AssertPresented(title) => json!({"op": "assert_presented", "title": title}),
        DriveOp::Respond(Some(button)) => json!({"op": "respond", "button": button}),
        DriveOp::Respond(None) => json!({"op": "respond", "dismiss": true}),
    };
    serde_json::from_value(v).map_err(|e| Fail(format!("{}: {e}", op_name(op))))
}

/// A test's wall-clock span, where the platform keeps one. `std::time::Instant` panics on
/// wasm, so the browser reports no timing rather than a fake one.
struct Stopwatch {
    #[cfg(not(target_arch = "wasm32"))]
    started: std::time::Instant,
}

impl Stopwatch {
    fn start() -> Self {
        Stopwatch {
            #[cfg(not(target_arch = "wasm32"))]
            started: std::time::Instant::now(),
        }
    }
    fn elapsed_ms(&self) -> u64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.started.elapsed().as_millis() as u64
        }
        #[cfg(target_arch = "wasm32")]
        {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::glob;

    #[test]
    fn globs_match_prefixes_suffixes_and_middles() {
        assert!(glob("button-*", "button-status"));
        assert!(glob("*-status", "button-status"));
        assert!(glob("button*status", "button-status"));
        assert!(glob("*", "anything"));
        assert!(glob("button-status", "button-status"));
        assert!(!glob("button-*", "slider-range"));
        assert!(!glob("button", "button-status"));
    }
}
