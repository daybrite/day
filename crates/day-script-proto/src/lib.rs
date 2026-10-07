// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Dayscript's shared wire protocol and on-disk format.
//!
//! This leaf crate depends only on serde, JSON and YAML. The CLI and embedded engine share
//! request/reply types and document validation without linking any UI, localization or transport.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod format;
pub use format::{FailurePolicy, FormatError, Script, ScriptStep, steps_from_yaml, steps_to_yaml};

/// Default implicit-wait budget, in seconds.
pub const DEFAULT_TIMEOUT_SECS: f64 = 5.0;

/// Which captures a run keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShotPolicy {
    Never,
    #[default]
    OnFailure,
    Always,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Request {
    pub token: String,
    pub step: Step,
}

/// Native UI integration or deterministic Day request/response testing.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DialogMode {
    Native,
    Scripted,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Step {
    WaitFor {
        id: String,
        /// Upper bound in seconds for the implicit retry wait (§14.3), for elements that
        /// appear only after slow work (a login round-trip, a first sync). Defaults to the
        /// shared step timeout.
        #[serde(default)]
        timeout_secs: Option<f64>,
    },
    WaitIdle,
    /// Programmatic scroll (docs/scroll.md §dayscript). With `edge`/`x`+`y`, `id` must name a
    /// `scroll` piece; with neither, `id` names any element and its nearest enclosing scroll
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
        /// Skip an absent element after UI transitions settle. Intended for idempotent
        /// fixture cleanup; existing elements still pass the visibility/enabled gates.
        #[serde(default)]
        if_present: bool,
        #[serde(default)]
        repeat: Option<u32>,
        /// Tap at this point in the element's coordinate space instead of its center, which is
        /// what canvas hit-testing needs (`- tap: { id: canvas, at: [40, 60] }`).
        #[serde(default)]
        at: Option<[f64; 2]>,
        /// Modifier keys "held" for the tap (`modifiers: [shift]` / `[primary]` / `[alt]`):
        /// the executor stands them in through `day::modifiers()` while dispatching, so
        /// modifier-dependent taps (shift-click multi-select) are drivable synthetically.
        #[serde(default)]
        modifiers: Vec<String>,
    },
    /// A non-text key press, delivered the way the platform delivers one: to whatever holds
    /// focus (`- focus: { id: canvas }` then `- key: { key: ArrowRight, modifiers: [shift] }`,
    /// docs/menus.md). Names follow the web `KeyboardEvent.key` vocabulary.
    Key {
        key: String,
        /// The piece to deliver to. Omit to send it wherever focus is, which is what a real
        /// key press does (docs/menus.md); pair it with a `focus:` step to drive the whole
        /// route. Name an `id` to address one piece's handler regardless of focus.
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        modifiers: Vec<String>,
    },
    /// A synthetic pointer drag over one element, in its own coordinate space: `Began` at
    /// `from`, a few `Changed` samples along the segment, `Ended` at `to`, the same
    /// `Event::Drag` stream a native recognizer delivers, so `.on_drag` state machines
    /// (canvas move/resize) run their whole preview→commit path. Injected, like every
    /// dayscript step: green here says the app logic holds, not that the platform recognizer
    /// fires; verify that with real input (docs/agent.md).
    Drag {
        id: String,
        from: [f64; 2],
        to: [f64; 2],
        /// Intermediate `Changed` samples between the endpoints (default 4).
        #[serde(default)]
        steps: Option<u32>,
        /// Modifier keys "held" for the whole drag, named as in [`Step::Tap`], for gestures
        /// whose meaning they change (a shift-drag that adds to a selection rather than
        /// replacing it). They stand in for every phase, press through release, because that
        /// is how a real drag reads them: once, when it starts.
        #[serde(default)]
        modifiers: Vec<String>,
    },
    /// Deliver `Event::Submitted` to the element: the scripted stand-in for the platform's
    /// submit gesture (Enter in a `text_area` with `.on_submit`, a field's return key).
    Submit {
        id: String,
    },
    Input {
        id: String,
        #[serde(default)]
        text: Option<String>,
        /// Localized alternative to `text`: resolve this Fluent key (with `args`) in the run's
        /// locale and type the result, for locale-portable queries (e.g. a localized fruit name).
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
    /// Invoke a list row without changing selection. Out-of-range indices are ignored.
    Activate {
        id: String,
        index: usize,
    },
    Select {
        id: String,
        index: i64,
    },
    /// Drag-reorder a list row programmatically: row `from` drops at row `to` through the same
    /// guard → commit path a native drag takes (docs/list.md); the app's `reorder_guard` may
    /// deny or retarget it. Fails (non-retryably) when the list isn't `.reorderable()` or the
    /// guard denies the move.
    Reorder {
        id: String,
        from: usize,
        to: usize,
    },
    /// Undo one unit of the app's history, through the installed undo bridge, the same
    /// handler ⌘Z and the Edit menu reach (docs/model.md). Portable: it needs no undo
    /// button and no scriptable menu item on the target. Fails (non-retryably) when the
    /// app never installed an undo stack.
    Undo,
    /// Redo one unit, [`Step::Undo`]'s mirror.
    Redo,
    /// Disclose or collapse a tree row programmatically (docs/tree.md). The row is resolved
    /// by its `.row_id` string; the step emits the same `Event::TreeExpanded` a native
    /// disclosure does, so the piece's expansion signal (and through it the native row)
    /// follows. Retryable while the row id is unknown (a pending reload may still produce it).
    Expand {
        id: String,
        row: String,
        #[serde(default = "default_true")]
        expanded: bool,
    },
    /// Move a tree row programmatically: `row` lands under `parent` (absent = the root) at
    /// `index` (absent = dropped onto the parent, i.e. append), through the same guard → commit
    /// path a native drag takes (docs/tree.md). Fails (non-retryably) when the tree isn't
    /// `.movable()` or a guard (structural or the app's) denies the move.
    TreeMove {
        id: String,
        row: String,
        #[serde(default)]
        parent: Option<String>,
        #[serde(default)]
        index: Option<usize>,
    },
    /// Delete a list row programmatically: row `row` goes through the same guard → commit path
    /// a native swipe takes (docs/list.md); the app's `delete_guard` may refuse it. Fails
    /// (non-retryably) when the list isn't `.deletable()` or the guard refuses.
    ///
    /// This is how a walkthrough asserts deletion on every target, including the desktops whose
    /// toolkits answer `Cap::ListDelete = Unsupported` and have no gesture to simulate: the step
    /// drives the guard → commit path, not the platform's gesture recognizer.
    DeleteRow {
        id: String,
        row: usize,
    },
    /// Activate a list row's swipe action programmatically: pull row `row`'s offer for `edge`
    /// (`trailing`, the default, or `leading`) and run action `action` (an index into the
    /// offer, default 0, the one a native full swipe activates), through the same
    /// offer → commit path a native gesture takes (docs/list.md). `label:` (literal) or
    /// `key:` (a Fluent key resolved in the run's locale) pins which button may be pressed.
    /// Offers are state-dependent ("Mark as Read" vs "Mark as Unread"), and the pin is
    /// checked before the press: a mismatched offer refuses the activation and fails the
    /// step with the row's state untouched, so a stale pin (leftover state from an aborted
    /// earlier run) fails once instead of flipping state and poisoning every later run.
    /// Fails (non-retryably) when the list offers no swipe actions or the row's offer has
    /// no such action.
    ///
    /// This is how a walkthrough exercises swipe actions on every target, including the
    /// toolkits that answer `Cap::ListSwipeActions = Unsupported` and show no affordance:
    /// the step drives the offer → commit path, not the platform's gesture recognizer.
    SwipeRow {
        id: String,
        row: usize,
        #[serde(default)]
        edge: Option<String>,
        #[serde(default)]
        action: usize,
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        key: Option<String>,
    },
    /// Evaluate JavaScript in a web view node and assert on the result
    /// (docs/webview-eval.md), the step that proves a page rendered, where
    /// `assert_visible` only proves the native view exists. `script` runs in the page;
    /// its value (a string compares as itself, anything else as its JSON) must contain
    /// `contains:` and/or equal `text:`; with neither, a successful evaluation alone
    /// passes. Retryable while the reply is pending, the script throws, or the assertion
    /// mismatches (a page mid-load settles within the wait); fails non-retryably when the
    /// id names no web view or no webview piece is linked. Only meaningful where the
    /// backend's eval arm exists (`eval_support()`; docs/webview-eval.md keeps the list);
    /// elsewhere the step fails after the wait, so gate it with `only_on:`.
    WebEval {
        id: String,
        script: String,
        #[serde(default)]
        contains: Option<String>,
        #[serde(default)]
        text: Option<String>,
        /// Override the shared wait for a cold browser engine or a slow page load.
        #[serde(default)]
        timeout_secs: Option<f64>,
    },
    /// Invoke an app-menu item programmatically (docs/menus.md): match a unique `Action`
    /// leaf in the installed app-menu model by exact `item` label, or by `key`, a Fluent
    /// key resolved in the run's locale (locale-portable; a standard-role item also
    /// matches its role's core-catalog key, so the auto Preferences item is
    /// `key: day-preferences`). `path` disambiguates with ancestor submenu labels (suffix
    /// match). Items that run a native selector instead of a day action (role items with
    /// id 0) are not invokable this way.
    /// Choose an item from an element's context menu (`.context_menu`, `.context_menu_fn`;
    /// docs/menus.md), as a right click or long press followed by a choice would. `id` names the
    /// element; the item is addressed as [`Step::Menu`] addresses one: `item_id:` (the name the
    /// app gave it), `item:` (the literal label) or `key:`, with `path` for submenus. A
    /// provider menu is asked at the element's origin.
    ContextMenu {
        id: String,
        #[serde(default)]
        item_id: Option<String>,
        #[serde(default)]
        item: Option<String>,
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        path: Option<Vec<String>>,
    },
    /// Choose an app-menu item. Address it by `id:` (the name the app gave it with
    /// `MenuEntry::id`) in preference to anything else: a label is localized, and an item that
    /// shows a check mark rewrites its label as the state moves, so neither is a stable
    /// address. `item:` matches the literal label, `key:` a built-in role key (`day-copy`) or a
    /// Fluent key resolved in the run's locale.
    Menu {
        #[serde(default)]
        id: Option<String>,
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
    /// the app's wiring; it does not prove the native widget drew (a screenshot does).
    Toolbar {
        item: String,
        #[serde(default)]
        text: Option<String>,
        /// Localized alternative to `text`, exactly as [`Step::Input`] takes one: resolve this
        /// Fluent key (with `args`) in the run's locale and type the result. A toolbar search
        /// field that filters on localized text needs this: a literal query written in English
        /// matches nothing once the run switches locale.
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        args: Option<BTreeMap<String, serde_json::Value>>,
        #[serde(default)]
        on: Option<bool>,
        /// A segmented item's choice, by index (docs/toolbars.md). The step fails rather than
        /// guessing if the item is not segmented or the index is out of range.
        #[serde(default)]
        index: Option<usize>,
    },
    AssertVisible {
        id: String,
    },
    /// Fail if the id is in the tree: the assertion for a subtree a `when` has not mounted
    /// (a property row that does not apply, a page's absent chrome). `assert_visible` cannot
    /// say this: a missing id is an error there, and an error is not a pass.
    AssertMissing {
        id: String,
    },
    /// Fail if the element is on screen: pass when it is missing, has an empty frame, or its
    /// native widget reports itself hidden. For what a toolkit hides rather than removes (a
    /// collapsed pane, a closed popover), where `assert_missing` would see it present.
    AssertHidden {
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
        /// Upper bound in seconds for the implicit retry wait (§14.3), for text that changes only
        /// after slow work (a download finishing, an upload's receipt). Defaults to the shared
        /// step timeout.
        #[serde(default)]
        timeout_secs: Option<f64>,
    },
    AssertValue {
        id: String,
        value: serde_json::Value,
    },
    /// Check an element's frame (docs/testing.md): its size, and its origin relative to
    /// `relative_to`'s (or to the window content when absent), in points, within `tolerance`
    /// (default 1). Day's layout must match, and so must the native widget's frame where the
    /// toolkit reads it back; where it cannot, the reply's `data` lists the frame as unread.
    AssertFrame {
        id: String,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        height: Option<f64>,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
        #[serde(default)]
        relative_to: Option<String>,
        #[serde(default)]
        tolerance: Option<f64>,
    },
    /// Check the color of one point of an element in an in-process capture (docs/testing.md).
    /// `x` and `y` are fractions of the element's frame (0.5, 0.5 is its center); `color` is
    /// `#rrggbb`, matched per channel within `tolerance` (default 64 of 255, which absorbs color
    /// management, such as a Display P3 capture reading sRGB red as `#ea3323`, and antialiasing,
    /// but not a different color). A toolkit with no capture
    /// (`Cap::Snapshot` unsupported) passes it as unread.
    SamplePixel {
        id: String,
        x: f64,
        y: f64,
        color: String,
        #[serde(default)]
        tolerance: Option<u8>,
    },
    /// Check that the app asked to open `url` during this test run (docs/testing.md). While
    /// `run_tests` runs, `open_url` records instead of opening anything.
    AssertOpenedUrl {
        url: String,
    },
    /// Compare the NATIVE widget with what is expected (docs/testing.md): the state the
    /// platform reports through `Toolkit::read_native`, not Day's tree. Only the fields given
    /// are checked. A field the toolkit cannot read passes and is listed as unread in the
    /// reply's `data`, so a gap is recorded rather than hidden. `enabled` is also checked
    /// against Day's own state, so a disagreement between the two fails either way.
    AssertNative {
        id: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        number: Option<f64>,
        #[serde(default)]
        checked: Option<bool>,
        #[serde(default)]
        enabled: Option<bool>,
        #[serde(default)]
        visible: Option<bool>,
    },
    /// Fail if any piece kind rendered a `⟨kind⟩` placeholder, i.e. the backend had no renderer
    /// for it. Placeholders are invisible to every other assertion (the app still renders, the
    /// screenshot still looks plausible), so this is the only step that catches a missing or
    /// silently-dropped renderer. `allow` lists the kinds a target is expected to lack, which
    /// makes the script itself the per-target gap ledger; anything outside it is a failure.
    AssertNoPlaceholders {
        #[serde(default)]
        allow: Vec<String>,
    },
    /// Close the secondary window opened under `window` (`day::open_window`'s key;
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
        /// resolves to the primary window, whose fullscreen cover is the content: the same
        /// pixels, no special case. A missing key fails retryably (the window may still be
        /// opening).
        #[serde(default)]
        window: Option<String>,
        /// Whether the reply carries the engine's capture (`png_base64`).
        ///
        /// On a device target the runner captures through `simctl`/`adb` and that image
        /// (whole screen, system chrome and all) is what the gallery publishes, so a payload
        /// rendered here would be encoded, shipped over the socket and dropped on the floor.
        /// Measured at 819ms per shot on the iOS simulator (33.6s across one walkthrough
        /// variant, ~4.5 minutes across a CI job's eight), so the runner asks for it only
        /// where it will be used, and re-asks with this set if the device capture fails.
        ///
        /// Defaults to `true`: an older runner, `day drive`, or a hand-written step says
        /// nothing and still gets the image. The idle wait above happens either way; it is
        /// what makes a capture land on a settled frame, not an artifact of the encoding.
        #[serde(default = "default_true")]
        in_process: bool,
    },
    Pause {
        secs: f64,
    },
    /// Deliver a deep-link URL in-process (docs/deep-links.md): the URL maps to its route
    /// through the same `day_spec::route_of_url` every platform intake uses, then navigates,
    /// identical to a warm OS delivery. Proves routing, params, and back-stack seeding on
    /// every backend, including mock; OS registration and intake are the runner tier's job.
    DeepLink {
        url: String,
    },
    /// Deliver an original external URL to on_open_url (with normal routing fallback).
    OpenUrl {
        url: String,
    },
    /// Navigate to a registered route (reset-to semantics; "" = root). docs/navigation.md.
    Navigate {
        route: String,
    },
    /// Pop one navigation level. Bare, it is Day's own rail (`day_core::nav_back`), which pops
    /// the model and lets the backend follow. `native: true` presses the platform's back
    /// affordance instead (`Toolkit::native_back`): the bar's `shouldPop`, the dispatcher's
    /// callbacks, the native pop, then the backend's report of it to Day as a user back, which
    /// is the code a real tap runs and the bare step never reaches (docs/navigation.md).
    NavBack {
        // Skipped when false so a recorded bare back still writes as `- nav_back:`.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        native: bool,
    },
    /// Assert the current route path ("" = root).
    AssertRoute {
        route: String,
    },
    /// Select presentation behavior for subsequent Day dialogs and file pickers.
    /// Existing requests must be answered before switching.
    DialogMode {
        mode: DialogMode,
    },
    /// Assert that no Day presentation requests remain unanswered.
    AssertNotPresented,
    /// Assert a presentation is pending (native or scripted), optionally checking its title.
    AssertPresented {
        #[serde(default)]
        title: Option<String>,
    },
    /// Answer the open modal: a button `index`, a prompt `text`, a file `path` (open/save
    /// pickers; relative paths resolve against the app temp dir, writable on every target), or
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
    /// Diff the native accessibility tree against Day's expectations (role/label/value/identifier)
    /// for every id'd node, or just `id` (§13, §14.2). Backends that can't read their native tree
    /// (`found = false`) are skipped; role is only compared when both sides map to a known `Role`.
    A11yAudit {
        #[serde(default)]
        id: Option<String>,
    },
    /// List the `#[day::test]` cases the binary registers (docs/testing.md); the reply's
    /// `data` is the listing.
    Tests,
    /// Run the registered tests whose names match `filter` (globs; empty = all), keeping
    /// captures per `shots`. Retryable while the run is in progress, so the runner's wait loop
    /// polls it; `timeout_secs` bounds the whole run, `case_timeout_secs` each case. The
    /// reply's `data` is the report.
    RunTests {
        #[serde(default)]
        filter: Vec<String>,
        #[serde(default)]
        shots: ShotPolicy,
        #[serde(default)]
        timeout_secs: Option<f64>,
        /// Each case's time limit where the case sets none (default 30 s); `0` turns every
        /// limit off, for a run held at a debugger's breakpoint.
        #[serde(default)]
        case_timeout_secs: Option<f64>,
    },
    /// Move native focus to the control: the real Toolkit duty, not a synthetic event, so
    /// keyboards and end-editing flows engage (docs/focus.md). `focused: false` resigns it.
    Focus {
        id: String,
        #[serde(default)]
        focused: Option<bool>,
    },
    /// Assert the control's focus state as Day resolved it (`NodeProbe.focused`; retryable, since
    /// focus lands a turn after the request). `focused` defaults to `true`.
    AssertFocused {
        id: String,
        #[serde(default)]
        focused: Option<bool>,
    },
    /// Expect the app to terminate: the only step that tolerates the app dying (docs/break.md's
    /// crash-reporting flow, docs/agent.md). Must be the last step: a preceding step triggered an
    /// exit or crash, and `expect_exit` treats the connection dropping within `within`
    /// seconds (default 15) as success; the app surviving the window is the failure. Handled
    /// runner-side (`day-cli`), so the in-app engine never executes it; this arm is defensive.
    ExpectExit {
        #[serde(default)]
        within: Option<f64>,
    },
    /// Force the window's size class (docs/size-classes.md) without resizing anything: the
    /// cheap way to drive a responsive layout on a backend whose window cannot be resized from a
    /// script (the phones, an emulator). `width` is `compact` | `medium` | `expanded` | `large` |
    /// `extra-large`; `height` is `compact` | `medium` | `expanded` and defaults to `expanded`.
    ///
    /// This reports a class the way a backend would, so everything downstream (an automatic
    /// nav host re-presenting, a piece that lays out from `day::size_class()`) runs its real
    /// path. What it does not do is change the window's actual pixels: a screenshot after this
    /// step shows the new layout at the old size. Drive a real resize from the runner instead
    /// (Playwright's `setViewportSize` on web, the simulator's rotation on iOS) when the
    /// geometry itself is what's under test.
    /// `width: auto` releases the override and restores the class the window itself reports, so
    /// the steps after an adaptive sweep run at the device's real geometry again. A script that
    /// forces `expanded` and never lets go leaves a phone laying out a split whose detail pane
    /// falls off the screen: the layout is correct, the window just isn't that wide.
    SizeClass {
        width: String,
        #[serde(default)]
        height: Option<String>,
    },
    /// Change the window's actual geometry, and wait until the app has reported the new size
    /// (docs/size-classes.md). The runner performs the resize (a device's window belongs to the
    /// system, not to the app) and this half is the barrier: without it the next step races the
    /// platform's resize animation.
    ///
    /// `size_class:`'s complement, and the difference matters: that step reports a class the
    /// window is not actually at, so a screenshot after it shows the new layout at the old size.
    /// This one moves the pixels.
    Resize {
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        height: Option<f64>,
        /// `resize: auto`: back to the device's geometry.
        #[serde(default)]
        restore: bool,
    },
}

impl Step {
    /// The implicit-wait budget for this step, seconds: its own `timeout_secs` when declared
    /// (and positive), else the shared [`DEFAULT_TIMEOUT_SECS`].
    pub fn wait_budget_secs(&self) -> f64 {
        match self {
            Step::WaitFor {
                timeout_secs: Some(t),
                ..
            }
            | Step::AssertText {
                timeout_secs: Some(t),
                ..
            }
            | Step::WebEval {
                timeout_secs: Some(t),
                ..
            }
            | Step::RunTests {
                timeout_secs: Some(t),
                ..
            } if *t > 0.0 => *t,
            _ => DEFAULT_TIMEOUT_SECS,
        }
    }
}

impl Step {
    /// The stable snake-case operation name used on the wire and in script files.
    pub const fn op(&self) -> &'static str {
        match self {
            Self::WaitFor { .. } => "wait_for",
            Self::WaitIdle => "wait_idle",
            Self::ScrollTo { .. } => "scroll_to",
            Self::Tap { .. } => "tap",
            Self::Key { .. } => "key",
            Self::Drag { .. } => "drag",
            Self::Submit { .. } => "submit",
            Self::Input { .. } => "input",
            Self::SetValue { .. } => "set_value",
            Self::Toggle { .. } => "toggle",
            Self::Activate { .. } => "activate",
            Self::Select { .. } => "select",
            Self::Reorder { .. } => "reorder",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Expand { .. } => "expand",
            Self::TreeMove { .. } => "tree_move",
            Self::DeleteRow { .. } => "delete_row",
            Self::SwipeRow { .. } => "swipe_row",
            Self::WebEval { .. } => "web_eval",
            Self::ContextMenu { .. } => "context_menu",
            Self::Menu { .. } => "menu",
            Self::Toolbar { .. } => "toolbar",
            Self::AssertVisible { .. } => "assert_visible",
            Self::AssertMissing { .. } => "assert_missing",
            Self::AssertHidden { .. } => "assert_hidden",
            Self::AssertText { .. } => "assert_text",
            Self::AssertValue { .. } => "assert_value",
            Self::AssertFrame { .. } => "assert_frame",
            Self::SamplePixel { .. } => "sample_pixel",
            Self::AssertOpenedUrl { .. } => "assert_opened_url",
            Self::AssertNative { .. } => "assert_native",
            Self::AssertNoPlaceholders { .. } => "assert_no_placeholders",
            Self::CloseWindow { .. } => "close_window",
            Self::Screenshot { .. } => "screenshot",
            Self::Pause { .. } => "pause",
            Self::DeepLink { .. } => "deep_link",
            Self::OpenUrl { .. } => "open_url",
            Self::Navigate { .. } => "navigate",
            Self::NavBack { .. } => "nav_back",
            Self::AssertRoute { .. } => "assert_route",
            Self::DialogMode { .. } => "dialog_mode",
            Self::AssertNotPresented => "assert_not_presented",
            Self::AssertPresented { .. } => "assert_presented",
            Self::Respond { .. } => "respond",
            Self::A11yAudit { .. } => "a11y_audit",
            Self::Tests => "tests",
            Self::RunTests { .. } => "run_tests",
            Self::Focus { .. } => "focus",
            Self::AssertFocused { .. } => "assert_focused",
            Self::ExpectExit { .. } => "expect_exit",
            Self::SizeClass { .. } => "size_class",
            Self::Resize { .. } => "resize",
        }
    }
}

/// Defaults to on for fields introduced after the first protocol version.
fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Default, PartialEq)]
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
    /// Acknowledged render checkpoint. Absent when talking to an older app or when
    /// freshness can only be established by an in-process snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_revision: Option<u32>,
    /// App-side policy acknowledgment. Older apps omit it; runners must retain
    /// animation pauses until the app confirms that fast motion is active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast_animations: Option<bool>,
    /// Internal retry cadence: frame checkpoints can complete on the next display tick.
    #[serde(skip)]
    pub capture_pending: bool,
    /// A step's structured answer (`tests`, `run_tests`; docs/testing.md).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl Reply {
    pub fn ok() -> Self {
        Reply {
            ok: true,
            ..Default::default()
        }
    }
    pub fn fail(msg: impl Into<String>, retryable: bool) -> Self {
        Reply {
            ok: false,
            error: Some(msg.into()),
            retryable,
            ..Default::default()
        }
    }
}
