// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-mock — the headless toolkit (DESIGN.md §3.2, §21.2 M0–M1).
//!
//! Records every toolkit call into a compact op log (golden-diffable), performs deterministic
//! measurement (8pt/char × 16pt line labels, fixed control sizes), and lets tests inject
//! native events through the real sink. The op log is the contract for the fine-grained
//! guarantees: "exactly one op per state change" and "bounded measure calls" are assertions
//! over this log.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use day_spec::props::*;
use day_spec::*;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MockHandle(pub u64);

#[derive(Default, Debug, Clone)]
pub struct MockWidget {
    pub kind: &'static str,
    pub node: u64,
    pub text: String,
    pub placeholder: String,
    pub value: f64,
    pub flag: bool,
    pub enabled: bool,
    /// `text_area` attributes (probe-visible for tests): editable/read-only, selectable, spell-check.
    pub editable: bool,
    pub selectable: bool,
    pub spellcheck: bool,
    pub children: Vec<u64>,
    pub frame: Rect,
    pub a11y: A11yProps,
    pub scroll_content: Size,
    /// The scroll offset after the last `scroll_to` (docs/scroll.md), computed with the same
    /// minimal-reveal clamp every real backend applies — probe-visible for tests.
    pub scroll_offset: Point,
    pub ops: Vec<DrawOp>,
    /// Surface style from a `background`/`corner_radius` decorator (probe-visible for tests).
    pub background: Option<Color>,
    pub corner_radius: f64,
    pub clips: bool,
    /// Semantic theme-adaptive surface (a form section card) — probe-visible for tests.
    pub surface_role: Option<day_spec::SurfaceRole>,
    /// A label's resolved font spec (probe-visible so tests can assert e.g. `Font::Custom` flow).
    pub font: Option<day_spec::FontSpec>,
    /// A label's styled spans (docs/text-runs.md). Probe-visible so a test can assert that the
    /// SECOND word is bold — which no screenshot comparison can state and no `assert_text` can
    /// see, since the plain text is identical either way.
    pub runs: Vec<day_spec::TextRun>,
    /// Last focus state driven through the `focus` duty (docs/focus.md) — probe-visible.
    pub focused: bool,
    /// Last opacity applied via `set_opacity` (§8.4) — `None` until touched (probe-visible).
    pub opacity: Option<f64>,
    /// Last transform applied via `set_transform` (§8.4) — probe-visible.
    pub transform: Option<day_spec::Transform>,
    /// The most recent animation intent seen on ANY seam for this widget (`update`/`set_frame`/
    /// `set_opacity`/`set_transform`). Lets tests assert `with_animation` threaded the intent.
    pub last_anim: Option<AnimSpec>,
    /// A NAV host's current presentation (docs/size-classes.md) — probe-visible so a test can
    /// assert WHICH of the four a morph landed on. `flag` carries only split-ness, which cannot
    /// tell `Tabs` from `Rail` from `Stack`; both are kept because the older tests read `flag`.
    pub presentation: Option<day_spec::props::NavPresentation>,
    /// The resident detail page index a `NavPatch::Select` last chose (docs/navigation.md).
    /// `None` until the host is asked to select one, which only happens in a presentation whose
    /// rows are chrome — a stacked host pushes and pops instead.
    pub selected_page: Option<usize>,
}

/// A secondary window opened through the `open_window` duty (docs/windows.md) —
/// probe-visible for the seam tests.
#[derive(Clone, Debug)]
pub struct MockWindow {
    /// The content-container widget handle.
    pub handle: u64,
    /// The window root's spec-boundary id (events emit to it).
    pub node: NodeId,
    pub title: String,
    pub size: Size,
    /// `"normal"` | `"preferences"`.
    pub kind: String,
    pub open: bool,
    pub focused: bool,
    /// The content size the window was FITTED to (`WindowOptions::size_to_fit`), or `None` if it
    /// kept the size it was opened at. Probe-visible so a test can assert that a preferences
    /// panel shrank to its rows rather than keeping the caller's ceiling.
    pub fit_size: Option<Size>,
}

#[derive(Default)]
pub struct MockState {
    next: u64,
    pub widgets: HashMap<u64, MockWidget>,
    pub log: Vec<String>,
    pub sink: Option<EventSink>,
    /// (kind, proposal) measure-call counter for the M1 bounded-measure tests.
    pub measure_calls: usize,
    /// Recycling-list row-pull sources, keyed by LIST host handle (docs/list.md). A test drives
    /// the "viewport" through [`MockProbe::list_bind`], simulating what a native list would do.
    pub list_sources: HashMap<u64, ListSource>,
    /// Hierarchical-tree row-pull sources, keyed by TREE host handle (docs/tree.md). A test
    /// drives the "native tree" through the `MockProbe::tree_*` probes.
    pub tree_sources: HashMap<u64, day_spec::TreeSource>,
    /// The app menu as last applied (docs/menus.md) — item titles, probe-visible.
    pub app_menu: Vec<String>,
    /// Context menus by widget handle (docs/menus.md) — item titles per handle.
    pub context_menus: HashMap<u64, Vec<String>>,
    /// Secondary windows (docs/windows.md), in open order — probe-visible.
    pub windows: Vec<MockWindow>,
    /// `open_window` answers `Unsupported` (the cover-fallback test harness).
    pub no_multi_window: bool,
    /// `Cap::NavSplit` answers `Native` — the harness for split and re-presenting nav hosts
    /// (docs/size-classes.md). Off by default, so the mock keeps modeling a phone.
    pub nav_split: bool,
    /// `Cap::NavTabs` answers `Unsupported` — the harness for the DEGRADATION path, where an
    /// `Automatic` selector falls back to the sidebar resolver (docs/navigation.md). Inverted
    /// like `no_multi_window` because the capability is ON by default: a phone has a tab bar,
    /// so a mock that models a phone must have one too, or the default resolution is a fiction.
    pub no_nav_tabs: bool,
    /// `Cap::NavTabsAdaptive` answers `Unsupported` — the harness for a DESKTOP idiom, where a
    /// narrow window collapses to a stack instead of growing a tab bar. Also inverted: the mock
    /// models a phone by default, and a phone adapts.
    pub desktop_idiom: bool,
    /// What `Cap::NavContentList` answers (docs/navigation.md) — `Unsupported` by default (the
    /// composed path); a test opts into `Native` (persistent pane) or `Emulated` (merges into
    /// the stack) with [`MockProbe::set_nav_content_list`]. Read during the BUILD.
    pub nav_content_list: Support,
    /// `open_window` answers `Pending` (the async-completion test harness); the test
    /// finishes the open through [`MockProbe::complete_window`].
    pub pending_windows: bool,
    /// Parked `Pending` opens: (node, title, kind).
    pub pending_opens: Vec<(NodeId, String, String)>,
    /// Formatters for patch types the mock does not know — see [`MockProbe::describe_patch`].
    #[allow(clippy::type_complexity)]
    pub describers: Vec<Box<dyn Fn(&dyn Any) -> Option<String>>>,
}

impl MockState {
    fn log(&mut self, s: String) {
        self.log.push(s);
    }
}

/// The mock backend. Cloneable observer half: construct with [`MockToolkit::new`] and keep the
/// returned [`MockProbe`] to inspect state after day-core takes ownership of the toolkit.
pub struct MockToolkit {
    pub state: Rc<RefCell<MockState>>,
}

#[derive(Clone)]
pub struct MockProbe {
    pub state: Rc<RefCell<MockState>>,
}

impl MockToolkit {
    pub fn new() -> (Self, MockProbe) {
        let state = Rc::new(RefCell::new(MockState::default()));
        (
            MockToolkit {
                state: state.clone(),
            },
            MockProbe { state },
        )
    }
}

impl MockProbe {
    pub fn log(&self) -> Vec<String> {
        self.state.borrow().log.clone()
    }
    pub fn clear_log(&self) {
        let mut s = self.state.borrow_mut();
        s.log.clear();
        s.measure_calls = 0;
    }
    pub fn measure_calls(&self) -> usize {
        self.state.borrow().measure_calls
    }
    /// Teach the op log to name a patch type the mock doesn't know.
    ///
    /// A standalone piece (docs/extending.md) defines its patch enum in its own crate, so the
    /// toolkit seam sees `&dyn Any` and logs `update <kind> #n ?`. Install a describer and the
    /// log carries the variant instead, which is what makes "this write patched attributes and
    /// did NOT replace the document" an assertion a test can make:
    ///
    /// ```ignore
    /// probe.describe_patch::<EditorPatch>(|p| format!("{p:?}"));
    /// ```
    ///
    /// Describers are tried in the order installed; the first that matches the type wins.
    pub fn describe_patch<T: 'static>(&self, f: impl Fn(&T) -> String + 'static) {
        self.state
            .borrow_mut()
            .describers
            .push(Box::new(move |any| any.downcast_ref::<T>().map(&f)));
    }
    /// Ops excluding measures (mutation ops only).
    pub fn mutations(&self) -> Vec<String> {
        self.state
            .borrow()
            .log
            .iter()
            .filter(|l| !l.starts_with("measure"))
            .cloned()
            .collect()
    }
    pub fn widget(&self, h: MockHandle) -> MockWidget {
        self.state
            .borrow()
            .widgets
            .get(&h.0)
            .cloned()
            .unwrap_or_default()
    }
    pub fn find_by_kind(&self, kind: &str) -> Vec<(MockHandle, MockWidget)> {
        let mut v: Vec<_> = self
            .state
            .borrow()
            .widgets
            .iter()
            .filter(|(_, w)| w.kind == kind)
            .map(|(k, w)| (MockHandle(*k), w.clone()))
            .collect();
        v.sort_by_key(|(h, _)| h.0);
        v
    }
    /// Row count a `LIST` host would query from its data-source.
    pub fn list_len(&self, host: MockHandle) -> usize {
        let f = self
            .state
            .borrow()
            .list_sources
            .get(&host.0)
            .map(|s| s.len.clone());
        f.map(|f| f()).unwrap_or(0)
    }

    /// Simulate the native list binding row `index` into a physical `cell` — Day builds the row
    /// the first time a cell is used and rebinds (slot-write) when it is recycled. Drives the real
    /// day-core driver, so tests exercise the whole recycling path. (The source Rc is cloned out
    /// before the call so the re-entrant `with_tree`/toolkit work holds no MockState borrow.)
    pub fn list_bind(&self, host: MockHandle, index: usize, cell: MockHandle) {
        let f = self
            .state
            .borrow()
            .list_sources
            .get(&host.0)
            .map(|s| s.bind_row.clone());
        if let Some(f) = f {
            f(index, cell.0 as RawHandle);
        }
    }

    /// Consult the list's reorder guard the way a native validate hook would: the accepted
    /// target index, or -1 when the guard denies. `i64::MIN` when the list has no reorder seam
    /// (not `.reorderable()`).
    pub fn list_can_move(&self, host: MockHandle, from: usize, to: usize) -> i64 {
        let f = self
            .state
            .borrow()
            .list_sources
            .get(&host.0)
            .and_then(|s| s.reorder.as_ref().map(|r| r.can_move.clone()));
        f.map(|f| f(from, to)).unwrap_or(i64::MIN)
    }

    /// Consult the list's delete guard the way a native swipe would before offering its action:
    /// `Some(true)` to offer, `Some(false)` when the guard protects the row, `None` when the
    /// list has no delete seam (not `.deletable()`).
    pub fn list_can_delete(&self, host: MockHandle, index: usize) -> Option<bool> {
        let f = self
            .state
            .borrow()
            .list_sources
            .get(&host.0)
            .and_then(|s| s.delete.as_ref().map(|d| d.can_delete.clone()));
        f.map(|f| f(index))
    }

    /// Simulate a native swipe-to-delete: consult the guard, commit on accept (shortening Day's
    /// snapshot and deferring the app's `on_delete`, exactly as a native backend would). Returns
    /// whether the delete committed.
    pub fn list_delete(&self, host: MockHandle, index: usize) -> bool {
        let d = self
            .state
            .borrow()
            .list_sources
            .get(&host.0)
            .and_then(|s| s.delete.clone());
        let Some(d) = d else {
            self.state
                .borrow_mut()
                .log(format!("list delete unsupported {index}"));
            return false;
        };
        if !(d.can_delete)(index) {
            self.state
                .borrow_mut()
                .log(format!("list delete denied {index}"));
            return false;
        }
        (d.delete_row)(index);
        true
    }

    /// Pull a row's swipe-action offer the way a native gesture would as the row starts to
    /// slide (docs/list.md): `Some(actions)` — possibly empty — when the list has a swipe
    /// seam, `None` when it has none (no `.swipe_leading()`/`.swipe_trailing()`).
    pub fn list_swipe_actions(
        &self,
        host: MockHandle,
        index: usize,
        edge: SwipeEdge,
    ) -> Option<Vec<ListSwipeAction>> {
        let f = self
            .state
            .borrow()
            .list_sources
            .get(&host.0)
            .and_then(|s| s.swipe.as_ref().map(|sw| sw.actions_at.clone()));
        f.map(|f| f(index, edge))
    }

    /// Simulate a native swipe activation: pull the offer and press button `action` (an index
    /// into it), committing through the seam — which defers the app's handler to the event
    /// drain, exactly as a native backend would. Returns whether an action was activated.
    pub fn list_swipe(
        &self,
        host: MockHandle,
        index: usize,
        edge: SwipeEdge,
        action: usize,
    ) -> bool {
        let sw = self
            .state
            .borrow()
            .list_sources
            .get(&host.0)
            .and_then(|s| s.swipe.clone());
        let Some(sw) = sw else {
            self.state
                .borrow_mut()
                .log(format!("list swipe unsupported {index}"));
            return false;
        };
        if (sw.actions_at)(index, edge).len() <= action {
            self.state
                .borrow_mut()
                .log(format!("list swipe no action {index}"));
            return false;
        }
        (sw.perform)(index, edge, action);
        true
    }

    /// Simulate a native drop: consult the guard, commit on accept (rotating Day's snapshot and
    /// deferring the app's `on_reorder`, exactly as a native backend would). Returns whether the
    /// move committed. (Reorder Rcs are cloned out before the call, like [`Self::list_bind`].)
    pub fn list_move(&self, host: MockHandle, from: usize, to: usize) -> bool {
        let r = self
            .state
            .borrow()
            .list_sources
            .get(&host.0)
            .and_then(|s| s.reorder.clone());
        let Some(r) = r else {
            self.state
                .borrow_mut()
                .log(format!("list move unsupported {from}->{to}"));
            return false;
        };
        let accepted = (r.can_move)(from, to);
        if accepted < 0 {
            self.state
                .borrow_mut()
                .log(format!("list move denied {from}->{to}"));
            return false;
        }
        (r.move_row)(from, accepted as usize);
        self.state
            .borrow_mut()
            .log(format!("list move {from}->{accepted}"));
        true
    }

    /// The tree's direct children of `parent` (`None` = the root level), as tokens — read
    /// straight from the injected `TreeSource` (docs/tree.md).
    pub fn tree_children(&self, host: MockHandle, parent: Option<u64>) -> Vec<u64> {
        let fns = self
            .state
            .borrow()
            .tree_sources
            .get(&host.0)
            .map(|s| (s.children_len.clone(), s.child_token.clone()));
        let Some((len, tok)) = fns else {
            return Vec::new();
        };
        (0..len(parent)).map(|i| tok(parent, i)).collect()
    }

    /// Whether this token can hold children (draws a disclosure), per the injected source.
    pub fn tree_expandable(&self, host: MockHandle, token: u64) -> bool {
        let f = self
            .state
            .borrow()
            .tree_sources
            .get(&host.0)
            .map(|s| s.expandable.clone());
        f.map(|f| f(token)).unwrap_or(false)
    }

    /// The row's type-ahead string, per the injected source.
    pub fn tree_type_text(&self, host: MockHandle, token: u64) -> String {
        let f = self
            .state
            .borrow()
            .tree_sources
            .get(&host.0)
            .map(|s| s.type_select_text.clone());
        f.map(|f| f(token)).unwrap_or_default()
    }

    /// Simulate the native tree binding `token`'s row into a physical `cell` — build on first
    /// use, rebind (slot-write) on recycle, exactly like [`Self::list_bind`]. (The source Rc
    /// is cloned out before the call so the re-entrant work holds no MockState borrow.)
    pub fn tree_bind(&self, host: MockHandle, token: u64, cell: MockHandle) {
        let f = self
            .state
            .borrow()
            .tree_sources
            .get(&host.0)
            .map(|s| s.bind_row.clone());
        if let Some(f) = f {
            f(token, cell.0 as RawHandle);
        }
    }

    /// Consult the tree's move guard the way a native drag-validate hook would. `None` when
    /// the tree has no move seam (not `.movable()`).
    pub fn tree_can_move(
        &self,
        host: MockHandle,
        token: u64,
        parent: Option<u64>,
        index: Option<usize>,
    ) -> Option<day_spec::MoveVerdict> {
        let f = self
            .state
            .borrow()
            .tree_sources
            .get(&host.0)
            .and_then(|s| s.moves.as_ref().map(|m| m.can_move.clone()));
        f.map(|f| f(token, parent, index))
    }

    /// Simulate a native tree drop: consult the guard, commit on accept (deferring the app's
    /// `on_move`, exactly as a native backend would). Returns whether the move committed.
    pub fn tree_move(
        &self,
        host: MockHandle,
        token: u64,
        parent: Option<u64>,
        index: Option<usize>,
    ) -> bool {
        let m = self
            .state
            .borrow()
            .tree_sources
            .get(&host.0)
            .and_then(|s| s.moves.clone());
        let Some(m) = m else {
            self.state
                .borrow_mut()
                .log(format!("tree move unsupported {token}"));
            return false;
        };
        if (m.can_move)(token, parent, index) == day_spec::MoveVerdict::Deny {
            self.state
                .borrow_mut()
                .log(format!("tree move denied {token}"));
            return false;
        }
        (m.move_node)(token, parent, index);
        self.state
            .borrow_mut()
            .log(format!("tree move {token} -> {parent:?}@{index:?}"));
        true
    }

    /// Inject a native event through the real sink (as the toolkit trampoline would).
    pub fn emit(&self, node: NodeId, event: Event) {
        let sink = self.state.borrow_mut().sink.take();
        if let Some(sink) = sink {
            sink(node, event);
            self.state.borrow_mut().sink.get_or_insert(sink);
        } else {
            panic!("day-mock: no event sink installed");
        }
    }

    // --- secondary windows (docs/windows.md) -------------------------------------------

    /// The secondary windows opened so far (closed ones stay listed with `open: false`).
    pub fn windows(&self) -> Vec<MockWindow> {
        self.state.borrow().windows.clone()
    }

    /// Make `open_window` answer `Unsupported` — the cover-fallback test harness.
    pub fn set_no_multi_window(&self, v: bool) {
        self.state.borrow_mut().no_multi_window = v;
    }

    /// Make `Cap::NavSplit` answer `Native` — the harness for a selector that presents as split
    /// panes and re-presents on a size-class change (docs/size-classes.md). Unlike the window
    /// toggles this is read during the BUILD, so set it before launching.
    pub fn set_nav_split(&self, v: bool) {
        self.state.borrow_mut().nav_split = v;
    }

    /// Make `Cap::NavTabs` answer `Unsupported` — the harness for an `Automatic` selector on a
    /// toolkit that cannot draw a tab bar, which must degrade to the sidebar resolver rather
    /// than to a hole (docs/navigation.md). Read during the BUILD, so set it before launching.
    pub fn set_no_nav_tabs(&self, v: bool) {
        self.state.borrow_mut().no_nav_tabs = v;
    }

    /// Model a DESKTOP toolkit: `Cap::NavTabs` stays on (a pinned tab bar still draws) but
    /// `Cap::NavTabsAdaptive` answers `Unsupported`, so an `Automatic` selector collapses a
    /// narrow window to a stack rather than growing a tab bar (docs/navigation.md). Read during
    /// the BUILD, so set it before launching.
    pub fn set_desktop_idiom(&self, v: bool) {
        self.state.borrow_mut().desktop_idiom = v;
    }

    /// What `Cap::NavContentList` answers (docs/navigation.md) — the content-list pane
    /// harness. `Native` = persistent pane, `Emulated` = merges into the collapsed stack,
    /// `Unsupported` (the default) = the selector composes. Read during the BUILD.
    pub fn set_nav_content_list(&self, v: day_spec::Support) {
        self.state.borrow_mut().nav_content_list = v;
    }

    /// Make `open_window` answer `Pending` — the async-completion test harness. Finish an
    /// open with [`Self::complete_window`] + `day_core::finish_window_open`.
    pub fn set_pending_windows(&self, v: bool) {
        self.state.borrow_mut().pending_windows = v;
    }

    /// Materialize the parked `Pending` open for `node`: registers the content-container
    /// widget + window record (as the native side finishing creation would) and returns
    /// the raw handle to pass to `day_core::finish_window_open`. `None` = no such pending
    /// open.
    pub fn complete_window(&self, node: NodeId, size: Size) -> Option<day_spec::RawHandle> {
        let mut s = self.state.borrow_mut();
        let i = s.pending_opens.iter().position(|(n, _, _)| *n == node)?;
        let (node, title, kind) = s.pending_opens.remove(i);
        s.next += 1;
        let h = s.next;
        s.widgets.insert(
            h,
            MockWidget {
                kind: kinds::CONTAINER,
                node: node.0,
                enabled: true,
                ..Default::default()
            },
        );
        s.windows.push(MockWindow {
            handle: h,
            node,
            title,
            size,
            kind,
            open: true,
            focused: false,
            fit_size: None,
        });
        s.log(format!("window_ready #{h} {}", fmt_size(size)));
        Some(h as day_spec::RawHandle)
    }

    /// Resize a secondary window (as a native drag would): updates the record and reports
    /// `WindowResized` to the window's root.
    pub fn resize_window(&self, node: NodeId, size: Size) {
        {
            let mut s = self.state.borrow_mut();
            if let Some(w) = s.windows.iter_mut().find(|w| w.node == node) {
                w.size = size;
            }
        }
        self.emit(node, Event::WindowResized(size));
    }

    /// Close a secondary window from the native side (the title-bar path): marks it closed
    /// and reports `WindowClosed` to the window's root.
    pub fn close_window_natively(&self, node: NodeId) {
        {
            let mut s = self.state.borrow_mut();
            if let Some(w) = s.windows.iter_mut().find(|w| w.node == node) {
                w.open = false;
            }
        }
        self.emit(node, Event::WindowClosed);
    }

    /// The current op-log length — pair with [`Self::log_since`] to scope assertions.
    pub fn log_len(&self) -> usize {
        self.state.borrow().log.len()
    }

    /// The op-log entries recorded after `mark` (from [`Self::log_len`]).
    pub fn log_since(&self, mark: usize) -> Vec<String> {
        self.state.borrow().log[mark..].to_vec()
    }
}

fn fmt_size(s: Size) -> String {
    format!("{}x{}", s.width, s.height)
}
fn fmt_rect(r: Rect) -> String {
    format!(
        "({},{} {}x{})",
        r.origin.x, r.origin.y, r.size.width, r.size.height
    )
}

/// Deterministic text metrics: 8pt per char, 16pt line height, greedy wrap.
/// The mock's line box: one line of text is this tall, whatever it says.
pub const MOCK_LINE_H: f64 = 16.0;

pub fn text_size(text: &str, proposal: Proposal, wraps: bool) -> Size {
    let needed = 8.0 * text.chars().count() as f64;
    match (proposal.width, wraps) {
        (Some(w), true) if needed > w && w > 0.0 => {
            let lines = (needed / w).ceil();
            Size::new(w, MOCK_LINE_H * lines)
        }
        _ => Size::new(needed, MOCK_LINE_H),
    }
}

impl Toolkit for MockToolkit {
    type Handle = MockHandle;

    fn capability(&self, cap: Cap) -> Support {
        match cap {
            Cap::Snapshot => Support::Native,
            // The mock answers `first_baseline` from its synthetic metrics (see below).
            Cap::BaselineAlignment => Support::Native,
            // The mock records the text-area attributes (probe-visible), so it "supports" all three.
            Cap::TextEditable | Cap::TextSelectable | Cap::TextSpellCheck => Support::Native,
            // Styled runs land in `WidgetProbe::runs`, which is what a test asserts on
            // (docs/text-runs.md). ACTIVATING a link is not modeled — nothing in the mock
            // hit-tests text — so `Cap::TextLinks` stays Unsupported in the default arm; a test
            // that wants the rail emits `Event::LinkActivated` itself.
            Cap::TextRuns => Support::Native,
            // The mock "runs" backend-executed animation by recording the intent (probe-visible).
            Cap::Animation => Support::Native,
            // Covers "present" by recording the patch (probe-visible); tests emit the
            // FrameChanged size report themselves, as the native surface would.
            Cap::Cover => Support::Native,
            // The probe drives the whole guard → commit reorder seam (`list_can_move`/`list_move`).
            Cap::ListReorder => Support::Native,
            // The probe drives the whole tree seam (`tree_children`/`tree_bind`/`tree_move`).
            Cap::Tree | Cap::TreeMove => Support::Native,
            // Off by default: the mock models a phone, so a selector stacks unless a test opts in.
            // A mock that can split can also re-present — it records the patch, which is exactly
            // what the morph tests assert against.
            Cap::NavSplit | Cap::NavRepresent => {
                if self.state.borrow().nav_split {
                    Support::Native
                } else {
                    Support::Unsupported
                }
            }
            // Unsupported by default (the composed path); a test opts into the pane shapes
            // (docs/navigation.md) with [`MockProbe::set_nav_content_list`].
            Cap::NavContentList => self.state.borrow().nav_content_list,
            // ON by default (docs/navigation.md): the mock models a phone, and a phone has a tab
            // bar. A test opts OUT to exercise the degradation path, where `Automatic` falls back
            // to the sidebar resolver.
            Cap::NavTabs => {
                if self.state.borrow().no_nav_tabs {
                    Support::Unsupported
                } else {
                    Support::Native
                }
            }
            // Separate from `NavTabs`: a desktop draws a pinned tab bar but never adapts into one.
            Cap::NavTabsAdaptive => {
                let st = self.state.borrow();
                if st.no_nav_tabs || st.desktop_idiom {
                    Support::Unsupported
                } else {
                    Support::Native
                }
            }
            // Real (recorded) windows unless the test opted into the cover-fallback tier.
            Cap::MultiWindow => {
                if self.state.borrow().no_multi_window {
                    Support::Unsupported
                } else {
                    Support::Native
                }
            }
            _ => Support::Unsupported,
        }
    }

    fn realize(&mut self, kind: PieceKind, props: &dyn Any, id: NodeId) -> MockHandle {
        let mut s = self.state.borrow_mut();
        s.next += 1;
        let h = s.next;
        let mut w = MockWidget {
            kind,
            node: id.0,
            enabled: true,
            ..Default::default()
        };
        let mut detail = String::new();
        if let Some(p) = props.downcast_ref::<LabelProps>() {
            w.text = p.text.clone();
            w.font = Some(p.font);
            detail = format!(" text={:?}", p.text);
        } else if let Some(p) = props.downcast_ref::<ButtonProps>() {
            w.text = p.title.clone();
            w.enabled = p.enabled;
            detail = format!(" title={:?}", p.title);
        } else if let Some(p) = props.downcast_ref::<ToggleProps>() {
            w.flag = p.on;
            w.enabled = p.enabled;
        } else if let Some(p) = props.downcast_ref::<SliderProps>() {
            w.value = p.value;
        } else if let Some(p) = props.downcast_ref::<TextFieldProps>() {
            w.text = p.text.clone();
            w.placeholder = p.placeholder.clone();
        } else if let Some(p) = props.downcast_ref::<CanvasProps>() {
            w.ops = p.ops.clone();
        } else if let Some(p) = props.downcast_ref::<ContainerProps>() {
            w.background = p.background;
            w.corner_radius = p.corner_radius;
            w.clips = p.clips;
            w.surface_role = p.role;
            if p.background.is_some() || p.corner_radius > 0.0 || p.clips {
                detail = format!(
                    " bg={:?} radius={} clips={}",
                    p.background, p.corner_radius, p.clips
                );
            }
        } else if let Some(p) = props.downcast_ref::<ProgressProps>() {
            // `flag` records indeterminate-ness; `value` the determinate fraction.
            w.flag = p.value.is_none();
            w.value = p.value.unwrap_or(0.0);
            detail = format!(" value={:?}", p.value);
        } else if let Some(p) = props.downcast_ref::<NavProps>() {
            w.text = p.title.clone();
            w.flag = p.presentation.is_split();
            w.presentation = Some(p.presentation);
            // The content-list pane's realize-time state, so a test can see the shape the host
            // was BUILT with — the pane's initial visibility is settled here, not by a patch.
            detail = match p.list_width {
                Some(w) => format!(
                    " title={:?} presentation={:?} list_width={w} list_visible={}",
                    p.title, p.presentation, p.list_visible
                ),
                None => format!(" title={:?} presentation={:?}", p.title, p.presentation),
            };
        } else if let Some(p) = props.downcast_ref::<NavPageProps>() {
            w.text = p.title.clone();
            // The page's PANE, not the presentation drawing it — a selector's list page reads
            // `sidebar` whether the host is split or stacked (docs/size-classes.md).
            w.flag = p.pane == day_spec::props::Pane::Sidebar;
            detail = format!(" title={:?} pane={:?}", p.title, p.pane);
        } else if let Some(p) = props.downcast_ref::<NavMenuProps>() {
            w.text = p.items.join("|");
            w.value = p.selected.map(|i| i as f64).unwrap_or(-1.0);
            detail = format!(" items={:?} selected={:?}", p.items, p.selected);
        } else if let Some(p) = props.downcast_ref::<PickerProps>() {
            w.text = p.options.join("|");
            w.value = p.selected as f64;
            detail = format!(" options={:?} selected={}", p.options, p.selected);
        } else if let Some(p) = props.downcast_ref::<TextAreaProps>() {
            w.text = p.text.clone();
            w.placeholder = p.placeholder.clone();
            w.editable = p.editable;
            w.selectable = p.selectable;
            w.spellcheck = p.spellcheck;
            detail = format!(
                " lines={}..{} editable={} selectable={} spellcheck={}",
                p.min_lines, p.max_lines, p.editable, p.selectable, p.spellcheck
            );
        }
        s.log(format!("realize {kind} #{h}{detail}"));
        s.widgets.insert(h, w);
        MockHandle(h)
    }

    fn update(
        &mut self,
        h: &MockHandle,
        kind: PieceKind,
        patch: &dyn Any,
        anim: Option<&AnimSpec>,
    ) {
        let mut s = self.state.borrow_mut();
        // Ask the installed describers first — the borrow has to end before `widgets` is taken
        // mutably below.
        let described = s.describers.iter().find_map(|d| d(patch));
        let detail;
        {
            let w = s.widgets.get_mut(&h.0).expect("update on unknown widget");
            if anim.is_some() {
                w.last_anim = anim.copied();
            }
            detail = if let Some(p) = patch.downcast_ref::<LabelPatch>() {
                match p {
                    LabelPatch::Text(t) => {
                        w.text = t.clone();
                        format!("text={t:?}")
                    }
                    LabelPatch::Color(_) => "color".into(),
                    LabelPatch::Font(f) => {
                        w.font = Some(*f);
                        "font".into()
                    }
                    LabelPatch::Runs(text, runs) => {
                        w.text = text.clone();
                        w.runs = runs.clone();
                        format!("runs={}", runs.len())
                    }
                }
            } else if let Some(p) = patch.downcast_ref::<ButtonPatch>() {
                match p {
                    ButtonPatch::Title(t) => {
                        w.text = t.clone();
                        format!("title={t:?}")
                    }
                    ButtonPatch::Enabled(e) => {
                        w.enabled = *e;
                        format!("enabled={e}")
                    }
                    // Recorded so a walkthrough can see a reactive tint change, but the mock
                    // has no pixels to apply it to.
                    ButtonPatch::Style(s) => format!("style={s:?}"),
                }
            } else if let Some(p) = patch.downcast_ref::<TogglePatch>() {
                match p {
                    TogglePatch::On(v) => {
                        w.flag = *v;
                        format!("on={v}")
                    }
                    TogglePatch::Enabled(e) => {
                        w.enabled = *e;
                        format!("enabled={e}")
                    }
                }
            } else if let Some(p) = patch.downcast_ref::<SliderPatch>() {
                match p {
                    SliderPatch::Value(v) => {
                        w.value = *v;
                        format!("value={v}")
                    }
                    SliderPatch::Enabled(e) => {
                        w.enabled = *e;
                        format!("enabled={e}")
                    }
                }
            } else if let Some(p) = patch.downcast_ref::<TextFieldPatch>() {
                match p {
                    TextFieldPatch::Text { text, from_native } => {
                        if !*from_native {
                            w.text = text.clone();
                        }
                        format!("text={text:?} from_native={from_native}")
                    }
                    TextFieldPatch::Placeholder(t) => {
                        w.placeholder = t.clone();
                        format!("placeholder={t:?}")
                    }
                    TextFieldPatch::Enabled(e) => {
                        w.enabled = *e;
                        format!("enabled={e}")
                    }
                }
            } else if let Some(p) = patch.downcast_ref::<CanvasProps>() {
                w.ops = p.ops.clone();
                format!("canvas ops={}", w.ops.len())
            } else if let Some(ProgressPatch::Value(v)) = patch.downcast_ref::<ProgressPatch>() {
                w.flag = v.is_none();
                w.value = v.unwrap_or(0.0);
                format!("value={v:?}")
            } else if let Some(PickerPatch::Selected(i)) = patch.downcast_ref::<PickerPatch>() {
                if let Some(w) = s.widgets.get_mut(&h.0) {
                    w.value = *i as f64;
                }
                format!("picker.selected {i}")
            } else if let Some(tp) = patch.downcast_ref::<TextAreaPatch>() {
                if let Some(w) = s.widgets.get_mut(&h.0) {
                    match tp {
                        TextAreaPatch::SetText(t) => w.text = t.clone(),
                        TextAreaPatch::SetEditable(v) => w.editable = *v,
                        TextAreaPatch::SetSelectable(v) => w.selectable = *v,
                        TextAreaPatch::SetSpellCheck(v) => w.spellcheck = *v,
                    }
                }
                format!("textarea.patch {tp:?}")
            } else if let Some(p) = patch.downcast_ref::<NavMenuPatch>() {
                match p {
                    NavMenuPatch::Selected(sel) => {
                        w.value = sel.map(|i| i as f64).unwrap_or(-1.0);
                        format!("menu selected={sel:?}")
                    }
                    // Data-driven rows: `text` mirrors the joined labels for tests to assert.
                    NavMenuPatch::Items {
                        items, selected, ..
                    } => {
                        w.text = items.join("|");
                        w.value = selected.map(|i| i as f64).unwrap_or(-1.0);
                        format!("menu items={items:?} selected={selected:?}")
                    }
                }
            } else if let Some(p) = patch.downcast_ref::<NavPatch>() {
                match p {
                    NavPatch::Pushed { title, .. } => {
                        w.text = title.clone();
                        format!("nav pushed title={title:?}")
                    }
                    NavPatch::Popped => "nav popped".into(),
                    NavPatch::Title(t) => {
                        w.text = t.clone();
                        format!("nav title={t:?}")
                    }
                    // Probe-visible: tests assert the armed flag round-trips.
                    NavPatch::GuardTop(on) => {
                        w.flag = *on;
                        format!("nav guard={on}")
                    }
                    // The host's presentation after a size-class change. `flag` tracks it the
                    // same way the initial `NavProps` did, so a walkthrough asserting the morph
                    // reads one field either side of it; `presentation` carries which of the
                    // four it actually is, which `flag` cannot say.
                    NavPatch::Presentation(p) => {
                        w.flag = p.is_split();
                        w.presentation = Some(*p);
                        format!("nav presentation={p:?}")
                    }
                    // Resident-page switch (docs/navigation.md). A stacked host never receives
                    // this — it gets `Pushed`/`Popped` instead — so recording it unconditionally
                    // is also what lets a test prove the pieces layer sent the right one.
                    NavPatch::Select(i) => {
                        w.selected_page = Some(*i);
                        w.value = *i as f64;
                        format!("nav select={i}")
                    }
                    // Content-list pane visibility / stack membership (docs/navigation.md).
                    // Logged only: mock answers `Cap::NavContentList` Unsupported, so the pieces
                    // layer composes and these arrive solely in tests that force the cap.
                    NavPatch::ListVisible(v) => format!("nav list visible={v}"),
                    NavPatch::ListInStack(v) => format!("nav list in-stack={v}"),
                }
            } else if let Some(p) = patch.downcast_ref::<CoverPatch>() {
                // `flag` records presented-ness (probe-visible). Tests emit the FrameChanged
                // size report and the `CoverHidden` event, as the native surface would.
                match p {
                    CoverPatch::Present {
                        background,
                        dismiss_disabled,
                    } => {
                        w.flag = true;
                        w.background = *background;
                        format!(
                            "cover present bg={background:?} dismiss_disabled={dismiss_disabled}"
                        )
                    }
                    CoverPatch::DismissDisabled(d) => format!("cover dismiss_disabled={d}"),
                    CoverPatch::Dismiss => {
                        w.flag = false;
                        "cover dismiss".into()
                    }
                }
            } else if let Some(ContainerPatch::Background(c)) =
                patch.downcast_ref::<ContainerPatch>()
            {
                w.background = *c;
                format!("bg={c:?}")
            } else if let Some(p) = patch.downcast_ref::<ListPatch>() {
                match p {
                    ListPatch::Reload => "list reload".into(),
                    ListPatch::Splice(deltas) => format!("list splice {deltas:?}"),
                    ListPatch::RowSizeInvalidated(i) => format!("list row-size-invalidated {i}"),
                    ListPatch::ScrollToEnd => {
                        // Record that the host was asked to follow its last row (probe-visible).
                        w.flag = true;
                        "list scroll-to-end".into()
                    }
                    ListPatch::ScrollToRow(row) => format!("list scroll-to-row {row}"),
                    ListPatch::Selected(rows) => format!("list selected {rows:?}"),
                }
            } else if let Some(p) = patch.downcast_ref::<TreePatch>() {
                match p {
                    TreePatch::Reload => "tree reload".into(),
                    TreePatch::Expand(tok, on) => format!("tree expand {tok} {on}"),
                    TreePatch::Selected(toks) => format!("tree selected {toks:?}"),
                    TreePatch::Reveal(tok) => format!("tree reveal {tok}"),
                }
            } else {
                described.unwrap_or_else(|| "?".into())
            };
        }
        s.log(format!("update {kind} #{} {detail}", h.0));
    }

    fn release(&mut self, h: MockHandle) {
        let mut s = self.state.borrow_mut();
        s.widgets.remove(&h.0);
        s.log(format!("release #{}", h.0));
    }

    fn insert(&mut self, parent: &MockHandle, child: &MockHandle, index: usize) {
        let mut s = self.state.borrow_mut();
        {
            let p = s
                .widgets
                .get_mut(&parent.0)
                .expect("insert into unknown parent");
            let idx = index.min(p.children.len());
            p.children.insert(idx, child.0);
        }
        s.log(format!(
            "insert #{} into #{} at {}",
            child.0, parent.0, index
        ));
    }

    fn remove(&mut self, parent: &MockHandle, child: &MockHandle) {
        let mut s = self.state.borrow_mut();
        {
            let p = s
                .widgets
                .get_mut(&parent.0)
                .expect("remove from unknown parent");
            p.children.retain(|&c| c != child.0);
        }
        s.log(format!("remove #{} from #{}", child.0, parent.0));
    }

    fn move_child(&mut self, parent: &MockHandle, child: &MockHandle, to: usize) {
        let mut s = self.state.borrow_mut();
        {
            let p = s
                .widgets
                .get_mut(&parent.0)
                .expect("move in unknown parent");
            p.children.retain(|&c| c != child.0);
            let idx = to.min(p.children.len());
            p.children.insert(idx, child.0);
        }
        s.log(format!("move #{} in #{} to {}", child.0, parent.0, to));
    }

    fn measure(&mut self, h: &MockHandle, kind: PieceKind, p: Proposal) -> Size {
        let mut s = self.state.borrow_mut();
        s.measure_calls += 1;
        let w = s.widgets.get(&h.0).cloned().unwrap_or_default();
        let size = match kind {
            kinds::LABEL => text_size(&w.text, p, true),
            kinds::BUTTON => {
                let t = text_size(&w.text, Proposal::UNCONSTRAINED, false);
                Size::new(t.width + 16.0, 24.0)
            }
            kinds::TOGGLE => Size::new(51.0, 31.0),
            kinds::SLIDER => Size::new(p.width.unwrap_or(200.0), 24.0),
            kinds::TEXT_FIELD => Size::new(p.width.unwrap_or(200.0), 24.0),
            kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
            kinds::IMAGE => Size::new(32.0, 32.0),
            // Indeterminate spinner is a fixed square; determinate bar fills width.
            kinds::PROGRESS if w.flag => Size::new(20.0, 20.0),
            kinds::PROGRESS => Size::new(p.width.unwrap_or(200.0), 4.0),
            _ => Size::new(p.width.unwrap_or(10.0), p.height.unwrap_or(10.0)),
        };
        s.log(format!(
            "measure {kind} #{} {:?} -> {}",
            h.0,
            p.cache_key(),
            fmt_size(size)
        ));
        size
    }

    /// A synthetic first baseline (docs/baseline.md), modeling the one fact that matters for
    /// the layout math: text sits at different heights inside different widgets. The mock's
    /// line box is 16pt with a 12pt ascent, and a widget that frames its text (a field, a
    /// button) insets it — so a label beside a text field must drop by exactly that inset for
    /// the two to share a line. Widgets with no text report `None`.
    fn first_baseline(&mut self, h: &MockHandle, kind: PieceKind, size: Size) -> Option<f64> {
        const ASCENT: f64 = 12.0;
        let framed_inset = |box_h: f64| (box_h - MOCK_LINE_H) / 2.0 + ASCENT;
        let s = self.state.borrow();
        let w = s.widgets.get(&h.0).cloned().unwrap_or_default();
        match kind {
            kinds::LABEL => (!w.text.is_empty() || size.height > 0.0).then_some(ASCENT),
            kinds::BUTTON | kinds::TEXT_FIELD | kinds::TEXT_AREA => Some(framed_inset(size.height)),
            // No text, so no baseline — these are the children that keep centering.
            _ => None,
        }
    }

    fn set_frame(&mut self, h: &MockHandle, frame: Rect, anim: Option<&AnimSpec>) {
        let mut s = self.state.borrow_mut();
        if let Some(w) = s.widgets.get_mut(&h.0) {
            w.frame = frame;
            if anim.is_some() {
                w.last_anim = anim.copied();
            }
        }
        let a = if anim.is_some() { " animated" } else { "" };
        s.log(format!("set_frame #{} {}{}", h.0, fmt_rect(frame), a));
    }

    fn set_opacity(&mut self, h: &MockHandle, opacity: f64, anim: Option<&AnimSpec>) {
        let mut s = self.state.borrow_mut();
        if let Some(w) = s.widgets.get_mut(&h.0) {
            w.opacity = Some(opacity);
            if anim.is_some() {
                w.last_anim = anim.copied();
            }
        }
        let a = if anim.is_some() { " animated" } else { "" };
        s.log(format!("set_opacity #{} {:.3}{}", h.0, opacity, a));
    }

    fn set_transform(
        &mut self,
        h: &MockHandle,
        t: day_spec::Transform,
        _size: day_spec::Size,
        anim: Option<&AnimSpec>,
    ) {
        let mut s = self.state.borrow_mut();
        if let Some(w) = s.widgets.get_mut(&h.0) {
            w.transform = Some(t);
            if anim.is_some() {
                w.last_anim = anim.copied();
            }
        }
        let a = if anim.is_some() { " animated" } else { "" };
        s.log(format!(
            "set_transform #{} tx={:.1},ty={:.1},sx={:.2},sy={:.2},rot={:.1}{}",
            h.0, t.tx, t.ty, t.sx, t.sy, t.rotate_deg, a
        ));
    }

    fn set_selectable(&mut self, h: &MockHandle, selectable: bool) -> Option<MockHandle> {
        let mut s = self.state.borrow_mut();
        if let Some(w) = s.widgets.get_mut(&h.0) {
            w.selectable = selectable;
        }
        s.log(format!("set_selectable #{} {}", h.0, selectable));
        None
    }

    fn set_scroll_content(&mut self, h: &MockHandle, content: Size) {
        let mut s = self.state.borrow_mut();
        if let Some(w) = s.widgets.get_mut(&h.0) {
            w.scroll_content = content;
        }
        s.log(format!("set_scroll_content #{} {}", h.0, fmt_size(content)));
    }

    fn scroll_to(&mut self, h: &MockHandle, target: Rect, _animated: bool) {
        let mut s = self.state.borrow_mut();
        if let Some(w) = s.widgets.get_mut(&h.0) {
            // Minimal scroll that makes `target` (content space) visible, clamped to range.
            let (vw, vh) = (w.frame.size.width, w.frame.size.height);
            let (cw, ch) = (w.scroll_content.width, w.scroll_content.height);
            let clamp = |cur: f64, lo: f64, hi_edge: f64, view: f64, content: f64| -> f64 {
                let mut o = cur;
                if hi_edge > o + view {
                    o = hi_edge - view;
                }
                if lo < o {
                    o = lo;
                }
                o.clamp(0.0, (content - view).max(0.0))
            };
            let o = w.scroll_offset;
            w.scroll_offset = Point::new(
                clamp(
                    o.x,
                    target.origin.x,
                    target.origin.x + target.size.width,
                    vw,
                    cw,
                ),
                clamp(
                    o.y,
                    target.origin.y,
                    target.origin.y + target.size.height,
                    vh,
                    ch,
                ),
            );
        }
        s.log(format!("scroll_to #{} {}", h.0, fmt_rect(target)));
    }

    fn scroll_offset(&mut self, h: &MockHandle) -> Point {
        self.state
            .borrow()
            .widgets
            .get(&h.0)
            .map(|w| w.scroll_offset)
            .unwrap_or(Point::ZERO)
    }

    fn enable_gesture(&mut self, h: &MockHandle, _node: NodeId, kind: GestureKind) {
        self.state
            .borrow_mut()
            .log(format!("enable_gesture #{} {:?}", h.0, kind));
    }

    fn focus(&mut self, h: &MockHandle, _node: NodeId, focused: bool) {
        let mut st = self.state.borrow_mut();
        st.log(format!("focus #{} {}", h.0, focused));
        if let Some(w) = st.widgets.get_mut(&h.0) {
            w.focused = focused;
        }
    }

    fn set_event_sink(&mut self, sink: EventSink) {
        self.state.borrow_mut().sink = Some(sink);
    }

    fn attach_tree(&mut self, host: &MockHandle, source: day_spec::TreeSource) {
        let mut s = self.state.borrow_mut();
        s.tree_sources.insert(host.0, source);
        s.log(format!("attach_tree #{}", host.0));
    }

    fn attach_list(&mut self, host: &MockHandle, source: ListSource) {
        let mut s = self.state.borrow_mut();
        s.list_sources.insert(host.0, source);
        s.log(format!("attach_list #{}", host.0));
    }

    fn adopt(&mut self, raw: RawHandle) -> MockHandle {
        // A recycling list's cell: register a container widget so row content can attach to it.
        let h = raw as u64;
        let mut s = self.state.borrow_mut();
        s.widgets.entry(h).or_insert_with(|| MockWidget {
            kind: kinds::LIST_CELL,
            node: h,
            enabled: true,
            ..Default::default()
        });
        MockHandle(h)
    }

    fn set_a11y(&mut self, h: &MockHandle, a11y: &A11yProps) {
        let mut s = self.state.borrow_mut();
        if let Some(w) = s.widgets.get_mut(&h.0) {
            w.a11y = a11y.clone();
        }
        s.log(format!("a11y #{} id={:?}", h.0, a11y.identifier));
    }

    fn replay(&mut self, h: &MockHandle, ops: &[DrawOp], size: Size) {
        let mut s = self.state.borrow_mut();
        if let Some(w) = s.widgets.get_mut(&h.0) {
            w.ops = ops.to_vec();
        }
        s.log(format!(
            "replay #{} {} ops in {}",
            h.0,
            ops.len(),
            fmt_size(size)
        ));
    }

    fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
        Ok(vec![0x89, b'P', b'N', b'G'])
    }

    fn fit_window(&mut self, host: &MockHandle, size: Size) {
        let mut st = self.state.borrow_mut();
        if let Some(w) = st.windows.iter_mut().find(|w| w.handle == host.0) {
            w.fit_size = Some(size);
            w.size = size;
        }
        st.log(format!("fit_window {} {size:?}", host.0));
    }

    fn open_window(
        &mut self,
        id: NodeId,
        options: &day_spec::WindowOptions,
        kind: day_spec::WindowKind,
    ) -> day_spec::WindowOpenReply<MockHandle> {
        let kind_s = match kind {
            day_spec::WindowKind::Normal => "normal",
            day_spec::WindowKind::Preferences => "preferences",
        };
        let mut s = self.state.borrow_mut();
        if s.no_multi_window {
            s.log(format!("open_window unsupported kind={kind_s}"));
            return day_spec::WindowOpenReply::Unsupported;
        }
        if s.pending_windows {
            s.log(format!("open_window pending {:?} kind={kind_s}", id.0));
            s.pending_opens
                .push((id, options.title.clone(), kind_s.into()));
            return day_spec::WindowOpenReply::Pending;
        }
        s.next += 1;
        let h = s.next;
        s.widgets.insert(
            h,
            MockWidget {
                kind: kinds::CONTAINER,
                node: id.0,
                enabled: true,
                ..Default::default()
            },
        );
        s.windows.push(MockWindow {
            handle: h,
            node: id,
            title: options.title.clone(),
            size: options.size,
            kind: kind_s.into(),
            open: true,
            focused: false,
            fit_size: None,
        });
        s.log(format!(
            "open_window #{h} {:?} {} kind={kind_s}",
            options.title,
            fmt_size(options.size)
        ));
        day_spec::WindowOpenReply::Open(MockHandle(h))
    }

    fn quit_app(&mut self) {
        // Recorded rather than acted on: a test asserts the close policy reached the platform
        // exit, and the harness has no process to end (docs/windows.md).
        self.state.borrow_mut().log("quit_app".to_string());
    }

    fn close_window(&mut self, host: &MockHandle) {
        // Model the native round-trip: mark closed, then confirm through the sink with
        // `WindowClosed` — day-core tears down when the (queued) event drains.
        let mut s = self.state.borrow_mut();
        let Some(w) = s.windows.iter_mut().find(|w| w.handle == host.0 && w.open) else {
            return;
        };
        w.open = false;
        let node = w.node;
        s.log(format!("close_window #{}", host.0));
        let sink = s.sink.take();
        drop(s);
        if let Some(sink) = sink {
            sink(node, Event::WindowClosed);
            self.state.borrow_mut().sink.get_or_insert(sink);
        }
    }

    fn focus_window(&mut self, host: &MockHandle) {
        let mut s = self.state.borrow_mut();
        let Some(node) = s
            .windows
            .iter()
            .find(|w| w.handle == host.0 && w.open)
            .map(|w| w.node)
        else {
            return;
        };
        let prev = s.windows.iter().find(|w| w.focused).map(|w| w.node);
        for w in s.windows.iter_mut() {
            w.focused = w.handle == host.0;
        }
        s.log(format!("focus_window #{}", host.0));
        let sink = s.sink.take();
        drop(s);
        if let Some(sink) = sink {
            if let Some(p) = prev
                && p != node
            {
                sink(p, Event::WindowFocused(false));
            }
            sink(node, Event::WindowFocused(true));
            self.state.borrow_mut().sink.get_or_insert(sink);
        }
    }

    fn set_window_title(&mut self, host: &MockHandle, title: &str) {
        let mut s = self.state.borrow_mut();
        if let Some(w) = s.windows.iter_mut().find(|w| w.handle == host.0) {
            w.title = title.to_string();
        }
        s.log(format!("set_window_title #{} {title:?}", host.0));
    }

    fn snapshot_window_of(&mut self, host: &MockHandle) -> Result<Vec<u8>, String> {
        // Distinguishable from the primary snapshot so the dayscript `window:` routing is
        // assertable.
        Ok(vec![0x89, b'P', b'N', b'G', host.0 as u8])
    }

    fn present(&mut self, req: u64, spec: &day_spec::present::PresentSpec) {
        // No native UI; day-core's PENDING registry holds the spec. Log for op-log asserts;
        // tests answer via day_core::respond_presentation / pending_presentation.
        self.state
            .borrow_mut()
            .log(format!("present req={req} title={:?}", spec.title()));
    }

    fn dismiss(&mut self, req: u64) {
        self.state.borrow_mut().log(format!("dismiss req={req}"));
    }

    fn open_url(&mut self, url: &str) {
        // No browser to launch; record it so op-log assertions can verify a `link` fired.
        self.state.borrow_mut().log(format!("open_url {url}"));
    }

    fn defer_system_gestures(&mut self, edges: day_spec::Edges) {
        // No system gestures to defer; record the union (docs/cover.md) for op-log asserts.
        self.state
            .borrow_mut()
            .log(format!("defer_system_gestures edges={:#06b}", edges.0));
    }

    // The remaining duties, implemented observably so mock stays a COMPLETE conformance probe
    // (a duty a piece exercises must never vanish into a trait default here).

    fn set_app_menu(&mut self, items: &[day_spec::MenuItem]) {
        let mut s = self.state.borrow_mut();
        s.app_menu = items.iter().map(menu_title).collect();
        s.log(format!("set_app_menu [{} items]", items.len()));
    }

    fn set_context_menu(&mut self, h: &MockHandle, _node: NodeId, items: &[day_spec::MenuItem]) {
        let mut s = self.state.borrow_mut();
        if items.is_empty() {
            s.context_menus.remove(&h.0);
        } else {
            s.context_menus
                .insert(h.0, items.iter().map(menu_title).collect());
        }
        s.log(format!("set_context_menu #{} [{} items]", h.0, items.len()));
    }

    fn supports_lifecycle(&self, _phase: day_spec::Lifecycle) -> bool {
        // Headless CI stands in for every platform: claim the full lifecycle so tests can
        // exercise mobile-only phases (day-core synthesizes delivery).
        true
    }

    fn read_a11y(&self, h: &MockHandle) -> day_spec::A11ySnapshot {
        // Echo what set_a11y recorded, so `a11y_audit` diffs cleanly against expectations.
        let s = self.state.borrow();
        let Some(w) = s.widgets.get(&h.0) else {
            return day_spec::A11ySnapshot::default();
        };
        day_spec::A11ySnapshot {
            found: true,
            role: w.a11y.role,
            label: w.a11y.label.clone(),
            value: w.a11y.value.clone(),
            identifier: w.a11y.identifier.clone(),
        }
    }

    fn ui_idle(&mut self) -> bool {
        // No native transitions exist; idle is immediate — but log the poll so scripted runs
        // can assert dayscript's settle path touched it.
        self.state.borrow_mut().log("ui_idle".into());
        true
    }

    fn on_suspend(&mut self) {
        self.state.borrow_mut().log("on_suspend".into());
    }

    fn on_resume(&mut self) {
        self.state.borrow_mut().log("on_resume".into());
    }

    fn on_memory_warning(&mut self) {
        self.state.borrow_mut().log("on_memory_warning".into());
    }
}

/// A menu item's display title (submenus render as their title; separators as "—").
fn menu_title(item: &day_spec::MenuItem) -> String {
    match item {
        day_spec::MenuItem::Action { label, .. } => label.clone(),
        day_spec::MenuItem::Submenu { label, .. } => label.clone(),
        day_spec::MenuItem::Separator => "—".into(),
    }
}

impl Platform for MockToolkit {
    const TARGET: &'static str = "mock-mock";
    const TOOLKIT: &'static str = "mock";

    fn run(mut self, options: WindowOptions, ready: Box<dyn FnOnce(Self, MockHandle, Size)>) {
        // No native loop: create the root container, hand off, return. Tests drive via
        // MockProbe::emit + day_reactive::flush_sync.
        let root = self.realize(kinds::CONTAINER, &ContainerProps::default(), NodeId(0));
        ready(self, root, options.size);
    }

    fn post(f: Box<dyn FnOnce() + Send>) {
        // No loop to defer to: run immediately (tests are synchronous).
        f();
    }
}
