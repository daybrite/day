//! The dayscript **recorder** (DESIGN.md §14.6): the inverse of playback. Where the engine turns a
//! script into synthesized Day events, the recorder turns the events an app actually receives back
//! into a script. It rides one seam — [`day_core::set_event_observer`], the single point EVERY
//! backend funnels its native events through ([`day_core::enqueue_events`]) — so it needs no
//! per-toolkit code, and it emits an ordinary dayscript that replays cross-toolkit through the same
//! executor as any hand-written one.
//!
//! Scope is deliberately narrow: **actions only, and only where the step is portable**. A tap, a
//! text edit, a selection/toggle, a navigation, a back — the id-addressed things a walkthrough is
//! made of. Positional taps, gestures, slider drags, and native OS chrome are dropped (see
//! [`event_to_step`]); the resulting script is a starting point to edit, not a pixel-exact replay.
//!
//! Everything here is main-thread state (the observer only ever runs on the main thread, where
//! day-core dispatches). On wasm there is no in-process playback ([`play`]) — the WebSocket
//! transport drives the page instead (docs/web.md).

use std::cell::{OnceCell, RefCell};
use std::path::{Path, PathBuf};

use day_reactive::Signal;
use day_spec::{Event, NodeId};

use crate::Step;

// ---------------------------------------------------------------------------
// Canonical on-disk form <-> Step (the exact inverse of day-cli's `parse_flow`)
// ---------------------------------------------------------------------------

/// Serialize steps to the canonical on-disk dayscript form — a `flow:` document of
/// `- <op>: { <params> }` entries (§14.1), byte-compatible with the file day-cli's `parse_flow`
/// reads. Each [`Step`] serializes to its internal-`op`-tag map (`{op: tap, id: inc, …}`); this
/// lifts the `op` out to become the entry key and drops null-valued optional params, then
/// serde_norway renders the whole `{flow: […]}` document as YAML.
pub fn steps_to_yaml(steps: &[Step]) -> String {
    let entries: Vec<serde_json::Value> = steps.iter().map(step_to_entry).collect();
    let doc = serde_json::json!({ "flow": entries });
    // The document is a plain map/seq of scalars we just built, so serialization cannot fail; the
    // fallback keeps this total for callers (the observer mirrors on every event).
    serde_norway::to_string(&doc).unwrap_or_else(|_| "flow: []\n".to_string())
}

/// Like [`steps_to_yaml`], but each step's identifying line carries a trailing `# "label"` comment
/// naming the control it came from (§14.6) — its accessibility label, or its visible text. The
/// comment sits on the `id:` line for a tap/input/select and the `route:` line for a navigate, so a
/// reader sees `route: focus # "Focus"`. Comments are ordinary YAML, so an annotated script parses
/// and replays exactly as the bare one does; this is the form the recorder streams and saves.
pub fn annotate_yaml(steps: &[Step], labels: &[Option<String>]) -> String {
    let yaml = steps_to_yaml(steps);
    // serde_norway renders each step as a `- <op>:` list item followed by indented params. Walk the
    // lines, tracking which step we are inside, and append the comment to that step's key line.
    let key_of = |step: &Step| match step {
        Step::Tap { .. } | Step::Input { .. } | Step::Select { .. } | Step::WaitFor { .. } => {
            Some("id:")
        }
        Step::Navigate { .. } => Some("route:"),
        _ => None,
    };
    let mut out = String::with_capacity(yaml.len());
    let mut idx: isize = -1;
    let mut done_this_step = false;
    for line in yaml.lines() {
        if let Some(rest) = line.strip_prefix("- ") {
            let _ = rest;
            idx += 1;
            done_this_step = false;
        }
        out.push_str(line);
        if !done_this_step
            && idx >= 0
            && let Some(Some(label)) = labels.get(idx as usize)
            && let Some(Some(key)) = steps.get(idx as usize).map(key_of)
            && line.trim_start().starts_with(key)
        {
            // A comment cannot span lines, and a quote would confuse the eye — collapse whitespace
            // and drop any embedded quote so the annotation stays a clean single token.
            let clean = label
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .replace('"', "");
            out.push_str(&format!(" # \"{clean}\""));
            done_this_step = true;
        }
        out.push('\n');
    }
    out
}

/// One `Step` as its on-disk `{ <op>: { <params> } }` mapping (params `null` when the op takes
/// none, e.g. `nav_back`).
fn step_to_entry(step: &Step) -> serde_json::Value {
    let mut map = match serde_json::to_value(step) {
        Ok(serde_json::Value::Object(m)) => m,
        // A Step always serializes to an internally-tagged object; anything else is unreachable,
        // but stay total rather than panicking on the event path.
        _ => return serde_json::Value::Null,
    };
    let op = match map.remove("op") {
        Some(serde_json::Value::String(s)) => s,
        _ => return serde_json::Value::Null,
    };
    // Drop null optional params so a recorded `input` reads `{ id, text }`, not
    // `{ id, text, key: null, args: null }` — and so it round-trips (the fields default to None).
    map.retain(|_, v| !v.is_null());
    let params = if map.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::Object(map)
    };
    let mut entry = serde_json::Map::new();
    entry.insert(op, params);
    serde_json::Value::Object(entry)
}

/// Parse the canonical on-disk dayscript form back into steps — the inverse of [`steps_to_yaml`].
///
/// This MIRRORS day-cli's `parse_flow` (crates/day-cli/src/script.rs) on purpose: it accepts the
/// same `- <op>: {…}` entries, the `- screenshot: name` / `- pause: 1.5` scalar shorthands, and a
/// bare `- nav_back:` (null params). The two are kept in step by a test that round-trips the CLI's
/// own `smoke.yaml` template through here. (day-cli does NOT call this — the CLI deliberately does
/// not depend on day-script's runtime graph — so the shared shape is guarded by test, not code.)
pub fn steps_from_yaml(yaml: &str) -> Result<Vec<Step>, String> {
    let doc: serde_json::Value = serde_norway::from_str(yaml).map_err(|e| e.to_string())?;
    let flow = doc
        .get("flow")
        .and_then(|f| f.as_array())
        .ok_or("script has no `flow:` sequence")?;
    let mut steps = Vec::with_capacity(flow.len());
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
            other => return Err(format!("step {op}: unsupported params {other}")),
        }
        let step: Step = serde_json::from_value(serde_json::Value::Object(step))
            .map_err(|e| format!("step {op}: {e}"))?;
        steps.push(step);
    }
    Ok(steps)
}

// ---------------------------------------------------------------------------
// Event -> Step
// ---------------------------------------------------------------------------

/// Map a native event to the dayscript step that reproduces it — **actions only, semantic only**.
/// Returns `None` for everything the recorder drops.
///
/// DROPPED, and why:
/// - the positional `Tap(Point)` twin of `Pressed` (and `LongPress`/`ContextMenu`): a coordinate
///   is not portable — `Pressed` carries the id, so it is the one recorded, and the positional
///   variant is dropped to avoid recording every tap twice;
/// - `Drag`/`ScrollChanged`/`Pointer`/`Key`/`WindowResized`/`FrameChanged`/`Submitted`/
///   `FocusChanged`: gesture and low-level input, no id-addressed step;
/// - `ValueChanged` (a slider drag): no `set_value` step is emitted — a slider re-records as a
///   storm of intermediate values that rarely belongs in a walkthrough (edit one in by hand);
/// - `SelectionSet` (multi-select): no single-index step covers it;
/// - lifecycle / menu / toolbar / present-result / custom / window events: not the user UI actions
///   the recorder targets.
///
/// An id-less `Pressed`/`TextChanged`/`SelectionChanged`/`ToggleChanged` is dropped too: without an
/// id there is no portable step to write.
fn event_to_step(id: Option<&str>, ev: &Event) -> Option<Step> {
    match ev {
        Event::Pressed => id.map(|id| Step::Tap {
            id: id.to_string(),
            repeat: Some(1),
        }),
        Event::TextChanged(text) => id.map(|id| Step::Input {
            id: id.to_string(),
            text: Some(text.clone()),
            key: None,
            args: None,
        }),
        Event::SelectionChanged(index) => id.map(|id| Step::Select {
            id: id.to_string(),
            index: *index,
        }),
        // Playback drives a toggle through `select` (index = the bool as 0/1) — the pattern
        // Day-Skies' weather.yaml uses — so a recorded toggle replays without a `toggle` step.
        Event::ToggleChanged(on) => id.map(|id| Step::Select {
            id: id.to_string(),
            index: *on as i64,
        }),
        // Navigation (RouteRequested, NavBack, and every nav_link/sidebar/stack push that calls
        // `navigate` from an event handler) is captured by the NAV observer via route changes,
        // not here — see `on_nav`. Mapping it here too would double-record RouteRequested.
        _ => None,
    }
}

/// Whether appending `next` should REPLACE the last step rather than push a new one — the
/// coalescing rules that keep a recording readable: every keystroke in a field, or every step of a
/// multi-step selection, collapses to the final value; consecutive navigations collapse to the last
/// destination. Taps never coalesce (each is its own action).
fn coalesces(last: &Step, next: &Step) -> bool {
    match (last, next) {
        (Step::Input { id: a, .. }, Step::Input { id: b, .. }) => a == b,
        (Step::Select { id: a, .. }, Step::Select { id: b, .. }) => a == b,
        (Step::Navigate { .. }, Step::Navigate { .. }) => true,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Recorder state (main-thread thread-local)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Recorder {
    /// Whether events are being captured. A plain bool (not the reactive flag) so the event path's
    /// guard is a cheap thread-local read, not a reactive-runtime access.
    active: bool,
    steps: Vec<Step>,
    /// A signal mirrored with the current script text on every change (the showcase's editable
    /// buffer). `None` when recording headlessly.
    into: Option<Signal<String>>,
    /// A file rewritten with the current script on every change — continuous, so a `--record` run
    /// that is killed still leaves everything captured up to the last event.
    file: Option<PathBuf>,
    /// Events whose id starts with this prefix are skipped, so a UI's own record/stop controls
    /// never record themselves (else playback would re-tap Stop and the replay would never run).
    exclude: String,
    /// The pump generation in which the last `Tap`/`Select` was recorded, or `None` if the last
    /// step is not a foldable input. A navigation folds that input into one portable `Navigate`
    /// (a sidebar row, a stack push) when it happened in the SAME or the immediately following
    /// pump — a signal-bound remount settles one pump late, so "same pump" alone would miss it,
    /// and "any time" would wrongly swallow an unrelated earlier tap. See `on_nav`.
    input_gen: Option<u64>,
    /// Per-step annotation (a control's a11y label or visible text), index-aligned with `steps`.
    /// Rendered as a trailing `# "label"` comment by [`annotate_yaml`].
    labels: Vec<Option<String>>,
}

impl Recorder {
    /// Append `step` with its annotation `label` (the control's a11y label or text — §14.6),
    /// applying the [`coalesces`] rules. `labels` stays index-aligned with `steps`.
    fn push(&mut self, step: Step, label: Option<String>) {
        if let Some(last) = self.steps.last_mut()
            && coalesces(last, &step)
        {
            *last = step;
            *self.labels.last_mut().expect("labels align with steps") = label;
            return;
        }
        self.steps.push(step);
        self.labels.push(label);
    }

    /// Write the current script to the live sinks (buffer signal and/or file). Takes `&self` and
    /// touches no recorder thread-local, so it is safe to call while the recorder is borrowed.
    fn flush(&self) {
        let script = annotate_yaml(&self.steps, &self.labels);
        if let Some(sig) = self.into {
            // A disposed buffer (its page navigated away) is a defined no-op write — the recorder
            // keeps capturing regardless, and a fresh page re-targets it via `start_into`.
            sig.set(script.clone());
        }
        if let Some(path) = &self.file {
            let _ = std::fs::write(path, script.as_bytes());
        }
    }
}

thread_local! {
    static REC: RefCell<Recorder> = RefCell::new(Recorder::default());
    /// The reactive on/off flag a UI binds to (label/style). Lazily created in the ROOT scope so it
    /// survives the page that first reads it (Signal::global, docs/reactivity.md).
    static FLAG: OnceCell<Signal<bool>> = const { OnceCell::new() };
    /// A counter bumped on every recorded/cleared step (see [`version`]).
    static VERSION: OnceCell<Signal<u64>> = const { OnceCell::new() };
}

/// The reactive recording flag: `true` while recording. Bind a control's label/style to it (the
/// showcase's Record↔Stop button does). The same global handle every call, so reads from any page
/// track the one signal.
pub fn recording_signal() -> Signal<bool> {
    FLAG.with(|c| *c.get_or_init(|| Signal::global(false)))
}

/// A counter that increments each time a step is recorded (or the recording is cleared). A UI can
/// watch it to react to recording *progress* — a live step count, say — without owning the mirrored
/// buffer signal. Global-scoped, so it survives page rebuilds.
pub fn version() -> Signal<u64> {
    VERSION.with(|c| *c.get_or_init(|| Signal::global(0)))
}

fn bump_version() {
    version().update(|n| *n += 1);
}

/// Whether an event on `id` is excluded by `prefix` — the rule that keeps a UI's own record/stop
/// controls out of its own recording. An empty prefix excludes nothing; an id-less event is never
/// excluded (it has no id to match, and is dropped later for lack of a portable step anyway).
fn is_excluded(id: Option<&str>, prefix: &str) -> bool {
    !prefix.is_empty() && id.is_some_and(|id| id.starts_with(prefix))
}

/// Echo a recorded action to stdout, so it shows up in the console while the app runs (each line
/// carries the `[target]` prefix `day launch` adds). Runs from the event/nav observer — a native
/// trampoline path — so it uses a NON-panicking write: a raw `println!` on a broken stdout pipe
/// (routine when `day launch` tears the app down) would unwind into non-Rust frames and abort the
/// process (`panic_cannot_unwind`, the reason `day_core::diag` exists). Errors are dropped.
fn echo_action(step: &Step, label: Option<&str>) {
    use std::fmt::Write as _;
    let mut line = String::from("day record ▸ ");
    match step {
        Step::Tap { id, .. } => {
            let _ = write!(line, "tap {id}");
        }
        Step::Input { id, text, .. } => {
            let _ = write!(line, "input {id} = {:?}", text.as_deref().unwrap_or(""));
        }
        Step::Select { id, index } => {
            let _ = write!(line, "select {id} = {index}");
        }
        Step::Navigate { route } => {
            let _ = write!(
                line,
                "navigate → {}",
                if route.is_empty() { "/" } else { route }
            );
        }
        Step::NavBack => line.push_str("nav_back"),
        // The recorder never emits the other step kinds; nothing to echo.
        _ => return,
    }
    if let Some(label) = label {
        let _ = write!(
            line,
            "  \"{}\"",
            label.split_whitespace().collect::<Vec<_>>().join(" ")
        );
    }
    let mut out = std::io::stdout();
    use std::io::Write as _;
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// The event observer: resolve the node's id, drop the recorder's own controls, map to a step, and
/// append. Installed by the `start*` fns; removed by [`stop`].
fn on_event(node: NodeId, ev: &Event) {
    // Cheap guard first — the observer stays installed only while active, but a stop() racing an
    // in-flight dispatch can land here after the flag cleared.
    if !is_recording() {
        return;
    }
    // Resolve the id with NO recorder borrow held: `id_of` reads the tree, and keeping the borrow
    // out means it can never overlap the append below, even if that tree read re-enters dispatch.
    let id = day_core::id_of(node);
    let Some(step) = event_to_step(id.as_deref(), ev) else {
        return;
    };
    // The control's label — a11y label preferred, its own text as the fallback (§14.6). Read with
    // no recorder borrow held, same as `id_of`.
    let label = day_core::label_of(node);
    REC.with(|r| {
        let mut rec = r.borrow_mut();
        if !rec.active || is_excluded(id.as_deref(), &rec.exclude) {
            return;
        }
        let is_input = matches!(step, Step::Tap { .. } | Step::Select { .. });
        echo_action(&step, label.as_deref());
        rec.push(step, label);
        rec.input_gen = is_input.then(day_core::pump_generation);
        rec.flush();
    });
    bump_version();
}

/// The navigation observer: every route change, from any source (§14.6). Records the new FULL
/// route as one absolute `Navigate` — which replays multi-level stacks (`items/item-1`) in a
/// single step — folding in the tap/select that triggered it when there was one.
fn on_nav(route: &str, label: Option<&str>) {
    if !is_recording() {
        return;
    }
    let step = Step::Navigate {
        route: route.to_string(),
    };
    let label = label.map(str::to_string);
    REC.with(|r| {
        let mut rec = r.borrow_mut();
        if !rec.active {
            return;
        }
        let foldable = rec
            .input_gen
            .is_some_and(|g| day_core::pump_generation().saturating_sub(g) <= 1)
            && matches!(
                rec.steps.last(),
                Some(Step::Tap { .. } | Step::Select { .. })
            );
        echo_action(&step, label.as_deref());
        if foldable {
            // The tap/select that caused this navigation IS this navigation — replace the
            // non-portable `Select { id: nav, index }` / stack-push tap with the absolute route,
            // and carry the nav host's own label (the sidebar row title).
            *rec.steps.last_mut().expect("input_gen implies a last step") = step;
            *rec.labels.last_mut().expect("labels align with steps") = label;
        } else {
            rec.push(step, label);
        }
        rec.input_gen = None;
        rec.flush();
    });
    bump_version();
}

fn install_observer() {
    day_core::set_event_observer(Some(Box::new(on_event)));
    // Navigation is delivered by day-core's nav observer (§14.6): it fires on every route change
    // — sidebar, nav_link, imperative `navigate`, native back, and the signal-bound stack/tab
    // pushes (caught at the event-pump tail once their reactive reconcile settles). A reactive
    // watch on current_route() does NOT suffice: navigate()/push mutate controller state that
    // current_route() reads without a tracked dependency, so the watch never re-runs.
    day_core::set_nav_observer(Some(Box::new(on_nav)));
}

/// Shared start path. A **fresh** start (nothing was recording) begins from an empty script and
/// resets the sinks; re-calling while already recording only re-targets the sinks that are given
/// (so returning to a rebuilt page keeps the live stream flowing into the new buffer without losing
/// what was captured while it was gone).
fn start_common(into: Option<Signal<String>>, file: Option<PathBuf>) {
    REC.with(|r| {
        let mut rec = r.borrow_mut();
        if !rec.active {
            rec.steps.clear();
            rec.labels.clear();
            rec.input_gen = None;
            rec.into = None;
            rec.file = None;
            rec.active = true;
        }
        if into.is_some() {
            rec.into = into;
        }
        if file.is_some() {
            rec.file = file;
        }
        rec.flush();
    });
    install_observer();
    recording_signal().set_if_changed(true);
    bump_version();
}

/// Start recording into memory only (read it back with [`script`] / [`steps`] / [`save`]).
pub fn start() {
    start_common(None, None);
}

/// Start recording, mirroring the script text into `sig` on every event — the streaming, editable
/// buffer an app binds a `text_area` to. Called again while already recording, it re-targets the
/// stream at `sig` (keeping the recording so far) rather than restarting — what a page needs after
/// it was disposed and rebuilt mid-recording.
pub fn start_into(sig: Signal<String>) {
    start_common(Some(sig), None);
}

/// Start recording, continuously flushing the script to `path` — how `DAY_RECORD` / `day launch
/// --record` capture headlessly. Crash-resilient: the file holds everything up to the last event
/// even if the app is killed.
pub fn start_to_file(path: impl AsRef<Path>) {
    start_common(None, Some(path.as_ref().to_path_buf()));
}

/// Stop recording and remove the observer. The captured script is kept (read it with [`script`] /
/// [`steps`] / [`save`], or resume with a `start*`).
pub fn stop() {
    REC.with(|r| r.borrow_mut().active = false);
    day_core::set_event_observer(None);
    day_core::set_nav_observer(None);
    recording_signal().set_if_changed(false);
}

/// Whether a recording is currently live.
pub fn is_recording() -> bool {
    REC.with(|r| r.borrow().active)
}

/// The recorded script as canonical dayscript YAML ([`steps_to_yaml`]); `flow: []` when empty.
pub fn script() -> String {
    REC.with(|r| steps_to_yaml(&r.borrow().steps))
}

/// The recorded steps, cloned.
pub fn steps() -> Vec<Step> {
    REC.with(|r| r.borrow().steps.clone())
}

/// Discard the recorded steps (recording, if live, continues from empty). Mirrors the emptied
/// script into any live sink.
pub fn clear() {
    REC.with(|r| {
        let mut rec = r.borrow_mut();
        rec.steps.clear();
        rec.labels.clear();
        rec.input_gen = None;
        rec.flush();
    });
    bump_version();
}

/// Write the recorded script to `path`.
pub fn save(path: &Path) -> std::io::Result<()> {
    std::fs::write(path, script().as_bytes())
}

/// Skip events whose element id starts with `prefix` — set it to the id prefix an app gives its own
/// record/stop/play controls so they never record themselves. Set before `start*`.
pub fn exclude_prefix(prefix: &str) {
    REC.with(|r| r.borrow_mut().exclude = prefix.to_string());
}

// ---------------------------------------------------------------------------
// In-process playback
// ---------------------------------------------------------------------------

/// Play a dayscript in-process (§14.6): parse `yaml` and run each step through the SAME executor
/// the socket runner uses ([`crate::run_step_with_wait`]), on a spawned thread that dispatches each
/// step to the main thread and awaits its reply — mirroring the engine's connection loop. Returns
/// as soon as the run is *dispatched* (the steps then run asynchronously against the live UI).
/// Refuses while a recording is live, so a replay never records itself.
#[cfg(not(target_arch = "wasm32"))]
pub fn play(yaml: &str) -> Result<(), String> {
    play_with_delay(yaml, 0.0)
}

/// Play with an artificial pause (seconds) between each step — a slow-motion replay for watching a
/// script drive the UI. Returns `Err` WITHOUT spawning when the script is empty or does not parse,
/// so a UI can call it to validate (see [`is_playable`]) and to run from one path.
pub fn play_with_delay(yaml: &str, step_delay_secs: f64) -> Result<(), String> {
    if is_recording() {
        return Err("cannot play while recording — stop the recording first".to_string());
    }
    let steps = steps_from_yaml(yaml)?;
    if steps.is_empty() {
        return Err("script has no steps to play".to_string());
    }
    let delay = std::time::Duration::from_secs_f64(step_delay_secs.max(0.0));
    std::thread::spawn(move || {
        for (i, step) in steps.into_iter().enumerate() {
            if i > 0 && !delay.is_zero() {
                std::thread::sleep(delay);
            }
            // Best-effort: driven over a socket a failed step reports through the reply path, but
            // here there is no runner to read it, so carry on to the next step.
            let _ = crate::run_step_with_wait(step);
        }
    });
    Ok(())
}

/// Whether `yaml` is a non-empty, parseable script — what a Play button binds its enabled state to.
pub fn is_playable(yaml: &str) -> bool {
    steps_from_yaml(yaml).is_ok_and(|s| !s.is_empty())
}

/// wasm has no background thread and no `Instant` (the executor's wait loop traps there), so
/// in-process playback isn't available — drive the page over the dayscript WebSocket transport
/// instead (docs/web.md).
#[cfg(target_arch = "wasm32")]
pub fn play(_yaml: &str) -> Result<(), String> {
    play_with_delay(_yaml, 0.0)
}

#[cfg(target_arch = "wasm32")]
pub fn play_with_delay(_yaml: &str, _step_delay_secs: f64) -> Result<(), String> {
    Err(
        "day::play_script is not available on web (no background thread); drive the page over the \
         dayscript WebSocket transport instead"
            .to_string(),
    )
}

#[cfg(target_arch = "wasm32")]
pub fn is_playable(yaml: &str) -> bool {
    steps_from_yaml(yaml).is_ok_and(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_recorder_steps() {
        let steps = vec![
            Step::Navigate {
                route: "controls".into(),
            },
            Step::Tap {
                id: "inc".into(),
                repeat: Some(1),
            },
            Step::Input {
                id: "name-field".into(),
                text: Some("Ada".into()),
                key: None,
                args: None,
            },
            Step::Select {
                id: "size".into(),
                index: 2,
            },
            Step::NavBack,
        ];
        let yaml = steps_to_yaml(&steps);
        let back = steps_from_yaml(&yaml).expect("re-parse");
        // Step-level round trip (not byte-for-byte YAML): parse(emit(x)) == x.
        assert_eq!(
            format!("{steps:?}"),
            format!("{back:?}"),
            "yaml was:\n{yaml}"
        );
        // The emitted form is the on-disk `- <op>: {…}` shape day-cli reads, null params stripped.
        assert!(yaml.contains("tap:"), "yaml was:\n{yaml}");
        assert!(
            !yaml.contains("key:"),
            "null params must be stripped:\n{yaml}"
        );
    }

    #[test]
    fn accepts_cli_smoke_template() {
        // The recorder's parser MUST accept the exact file day-cli's `parse_flow` reads — string
        // `screenshot`, bare `nav_back:`, inline `{ id: … }` mappings and all. If this drifts, the
        // two parsers have diverged (see `steps_from_yaml`'s doc-comment).
        let smoke = include_str!("../../day-cli/templates/app/dayscript/smoke.yaml");
        let steps = steps_from_yaml(smoke).expect("smoke.yaml parses");
        assert!(
            steps.iter().any(|s| matches!(s, Step::NavBack)),
            "smoke.yaml has a nav_back"
        );
        assert!(
            steps
                .iter()
                .any(|s| matches!(s, Step::Screenshot { name, .. } if name == "smoke")),
            "smoke.yaml ends in a `screenshot: smoke` scalar"
        );
        // And it re-emits to a form that parses back to the same steps.
        let reparsed = steps_from_yaml(&steps_to_yaml(&steps)).expect("re-parse");
        assert_eq!(format!("{steps:?}"), format!("{reparsed:?}"));
    }

    #[test]
    fn event_to_step_maps_actions_and_drops_the_rest() {
        use day_spec::Point;
        // Actions with an id become steps.
        assert!(matches!(
            event_to_step(Some("inc"), &Event::Pressed),
            Some(Step::Tap { .. })
        ));
        assert!(matches!(
            event_to_step(Some("field"), &Event::TextChanged("hi".into())),
            Some(Step::Input { .. })
        ));
        assert!(matches!(
            event_to_step(Some("t"), &Event::ToggleChanged(true)),
            Some(Step::Select { index: 1, .. })
        ));
        // Navigation is NOT an event-to-step concern (the nav observer captures route changes) —
        // a RouteRequested must NOT also map to a Navigate here, or it would double-record.
        assert!(event_to_step(None, &Event::RouteRequested("home".into())).is_none());
        // The positional tap twin, a slider value, and an id-less press are all dropped.
        assert!(event_to_step(Some("x"), &Event::Tap(Point::new(1.0, 2.0))).is_none());
        assert!(event_to_step(Some("s"), &Event::ValueChanged(0.5)).is_none());
        assert!(event_to_step(None, &Event::Pressed).is_none());
    }

    #[test]
    fn annotate_yaml_comments_the_identifying_line() {
        let steps = vec![
            Step::Navigate {
                route: "focus".into(),
            },
            Step::Tap {
                id: "focus-next-button".into(),
                repeat: Some(1),
            },
        ];
        let labels = vec![Some("Focus".to_string()), Some("Focus next".to_string())];
        let yaml = annotate_yaml(&steps, &labels);
        assert!(
            yaml.contains("route: focus # \"Focus\""),
            "nav label on route line:\n{yaml}"
        );
        assert!(
            yaml.contains("id: focus-next-button # \"Focus next\""),
            "tap label on id line:\n{yaml}"
        );
        // Comments are ignored on parse — the annotated script still round-trips to the same steps.
        let reparsed = steps_from_yaml(&yaml).expect("annotated yaml parses");
        assert_eq!(format!("{steps:?}"), format!("{reparsed:?}"));
    }

    #[test]
    fn is_playable_rejects_empty_and_garbage() {
        assert!(!is_playable(""));
        assert!(!is_playable("flow: []\n"));
        assert!(!is_playable("not: a script"));
        assert!(is_playable("flow:\n- tap: { id: go }\n"));
    }

    #[test]
    fn coalesces_input_select_and_navigate() {
        let inp = |id: &str, t: &str| Step::Input {
            id: id.into(),
            text: Some(t.into()),
            key: None,
            args: None,
        };
        let mut rec = Recorder::default();
        rec.push(inp("f", "A"), None);
        rec.push(inp("f", "Ab"), None); // same id -> replaces
        rec.push(inp("g", "x"), None); // different id -> new step
        rec.push(Step::Navigate { route: "a".into() }, None);
        rec.push(Step::Navigate { route: "b".into() }, Some("B page".into())); // consecutive nav -> replaces
        assert_eq!(rec.steps.len(), 3);
        assert!(matches!(&rec.steps[0], Step::Input { text: Some(t), .. } if t == "Ab"));
        assert!(matches!(&rec.steps[2], Step::Navigate { route } if route == "b"));
    }

    #[test]
    fn exclude_prefix_drops_control_ids() {
        // The exact predicate the observer applies before recording an event.
        assert!(is_excluded(Some("scripting-record"), "scripting-"));
        assert!(!is_excluded(Some("inc"), "scripting-"));
        assert!(!is_excluded(None, "scripting-")); // id-less: nothing to match
        assert!(!is_excluded(Some("scripting-record"), "")); // empty prefix excludes nothing
    }
}
