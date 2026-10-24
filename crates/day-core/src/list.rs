// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Recycling-list driver (docs/list.md, §10). The native list host owns scrolling + cell reuse;
//! Day owns row *content*. day-core injects a [`day_spec::ListSource`] into the backend; when the
//! native data-source pulls a cell, `bind_row` builds it once (per physical cell) and thereafter
//! *rebinds* it (one slot-write) as the cell recycles.
//!
//! Re-entrancy (the crux): building a row uses `BuildCx`, and reactive bindings patch native
//! widgets; both acquire `with_tree` per operation. So `bind_row` phases the tree borrow:
//! `with_tree` (adopt cell) → build/rebind + `flush_sync` **outside** any borrow → `with_tree`
//! (lay the row out in its cell). Holding the borrow across the build would deadlock the RefCell.

use crate::tree::{RNode, try_with_tree, with_tree};
use day_reactive::Scope;
use day_spec::{ListSource, RawHandle, props::RowHeight};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// Supplied by the `list()` piece, type-erased over the item type. day-core invokes these to
/// answer the native data-source and to build/rebind rows.
pub struct ListDriver {
    pub row_height: RowHeight,
    pub track_viewport: bool,
    /// Current row count (reads the piece's snapshot; no tree access).
    pub len: Box<dyn Fn() -> usize>,
    /// Stable identity token for row `index` (for native diffing).
    pub token_at: Box<dyn Fn(usize) -> u64>,
    /// Build row `index` into `anchor`. Uses `BuildCx` internally, so it must be called with no
    /// `with_tree` borrow held. Returns the row's scope + a rebind writer.
    pub build: Box<dyn Fn(usize, RNode) -> BuiltRow>,
    /// Drag-to-reorder half, present when the piece is `.reorderable()` (docs/list.md).
    pub reorder: Option<ListReorderDriver>,
    /// Swipe-to-delete half, present when the piece is `.deletable()` (docs/list.md).
    pub delete: Option<ListDeleteDriver>,
    /// Swipe-ACTIONS half, present when the piece declared edge actions (docs/list.md).
    pub swipe: Option<ListSwipeDriver>,
    /// Section headers, present when the piece declared `.sections` (docs/list.md
    /// "Sections"). Every closure above still speaks item indices; day-core inserts the
    /// header rows and translates at the toolkit boundary (see [`SectionMap`]).
    pub sections: Option<ListSectionsDriver>,
}

/// The section half the `list()` piece supplies (docs/list.md "Sections"): a key per item and
/// a header builder. A header row precedes every item whose key differs from the item before.
pub struct ListSectionsDriver {
    /// The section key of item `index` in the current snapshot (the piece snapshots keys with
    /// its rows, and rotates them with a committed move, so this needs no app access).
    pub key_at: Box<dyn Fn(usize) -> u64>,
    /// Build the header of the section that starts at item `first` into `anchor`: the same
    /// contract as [`ListDriver::build`], and its rebind writer takes the section's first item.
    pub build: Box<dyn Fn(usize, RNode) -> BuiltRow>,
}

/// The reorder closures the `list()` piece supplies (docs/list.md). Exposed to the backend as
/// [`day_spec::ListReorder`] by the list source adapter; also driven directly by
/// [`list_try_reorder`] (the dayscript `reorder` step and the mock probe).
pub struct ListReorderDriver {
    /// Consult the app's guard: returns the accepted target index, or -1 to deny.
    pub can_move: Box<dyn Fn(usize, usize) -> i64>,
    /// Commit an accepted move: rotates the piece's snapshot + tokens synchronously and defers
    /// the app's `on_reorder` callback to the next event drain.
    pub moved: Box<dyn Fn(usize, usize)>,
}

/// The delete closures the `list()` piece supplies (docs/list.md). Exposed to the backend as
/// [`day_spec::ListDelete`] by the list source adapter; also driven directly by [`list_try_delete`]
/// (the dayscript `delete_row` step and the mock probe).
pub struct ListDeleteDriver {
    /// Consult the app's guard: may this row be deleted?
    pub can_delete: Box<dyn Fn(usize) -> bool>,
    /// Commit an accepted delete: drops the row from the piece's snapshot + tokens synchronously
    /// and defers the app's `on_delete` callback to the next event drain.
    pub deleted: Box<dyn Fn(usize)>,
}

/// The swipe-action closures the `list()` piece supplies (docs/list.md). Exposed to the
/// backend as [`day_spec::ListSwipe`] by the list source adapter; also driven directly by
/// [`list_try_swipe`] (the dayscript `swipe_row` step and the mock probe).
pub struct ListSwipeDriver {
    /// The offer: the actions for row `index` on `edge`. Pure; runs inside native swipe
    /// callbacks.
    pub actions_at: Box<dyn Fn(usize, day_spec::SwipeEdge) -> Vec<day_spec::ListSwipeAction>>,
    /// Commit an activated action (an index into the offer): defers the app's handler to the
    /// next event drain.
    pub performed: Box<dyn Fn(usize, day_spec::SwipeEdge, usize)>,
}

/// A freshly built row: its `Scope` (owns the row's reactive graph) and a rebind writer that
/// slot-writes item `index` into the row's `ItemSlot` when the cell is recycled.
pub struct BuiltRow {
    pub scope: Scope,
    pub rebind: Rc<dyn Fn(usize)>,
}

pub(crate) struct BoundCell {
    pub anchor: RNode,
    /// Between `bind_row` and `recycle`: only displayed cells participate in layout.
    pub active: bool,
    /// The row subtree's reactive scope, disposed by `remove_subtree` when the list node
    /// goes away (the cells live in the list machinery, not the node tree, so nothing else
    /// would dispose their bindings).
    pub scope: Scope,
    pub rebind: Rc<dyn Fn(usize)>,
    /// The width the backend last laid this cell at through `ListSource::layout_cell`: the
    /// width the native row granted, which on GTK is the list's less the row's own
    /// padding. Every later layout of the row (a rebind, the dirty-cell sweep, a data change)
    /// uses it; `None` until the backend has reported one, when the list's width stands in.
    pub native_width: Option<f64>,
    /// The width this cell's row was last laid at, by either path. Only a width that actually
    /// changed invalidates the row's measurement cache: the dirty-cell sweep re-lays every bound
    /// cell on every pass, and re-measuring each row's text each time made a 500-row list's
    /// binds take whole seconds.
    pub laid_width: Option<f64>,
    /// The height the row was last laid at (`ListSource::cell_height`).
    pub laid_height: Option<f64>,
    /// The cell holds a section header (built by [`ListSectionsDriver::build`]). A host that
    /// pools headers and rows together hands a header's cell to an item row now and then;
    /// the kinds differ, so `bind_row` rebuilds the cell instead of rebinding it.
    pub header: bool,
    /// The host's pinned copy of a header (`ListSource::bind_pinned`): its ids stay parked, so
    /// lookups find the header row in the list instead.
    pub pinned: bool,
}

pub(crate) struct ListState {
    pub driver: Rc<ListDriver>,
    /// Physical cell (native handle as usize) → its built row.
    pub cells: HashMap<usize, BoundCell>,
    /// The display rows of a sectioned list; `None` without sections, where display row and
    /// item index coincide.
    pub sections: Option<Rc<SectionMap>>,
}

/// One row as the native host sees it in a sectioned list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DisplayRow {
    /// The header of the section whose first item is this index.
    Header(usize),
    Item(usize),
}

/// A sectioned list's display rows (docs/list.md "Sections"): the items with a header row in
/// front of each section. The native host counts, binds, selects and scrolls display rows; the
/// app, the piece and dayscript speak item indices. Everything that crosses between them
/// translates here: the `ListSource` closures, the patches the tree sends, and the selection
/// events the toolkit raises (through the event sink, which may run inside a tree borrow, so
/// the map lives outside the tree, in `SECTION_MAPS`).
pub(crate) struct SectionMap {
    driver: Rc<ListDriver>,
    rows: RefCell<Vec<DisplayRow>>,
    /// Item index → its display row.
    display: RefCell<Vec<usize>>,
    /// The display row of each header, in order, with the header's identity token.
    heads: RefCell<Vec<(usize, u64)>>,
    /// The snapshot changed since the rows were computed (a reload, a splice, a commit).
    stale: Cell<bool>,
}

impl SectionMap {
    fn new(driver: Rc<ListDriver>) -> Self {
        SectionMap {
            driver,
            rows: RefCell::new(Vec::new()),
            display: RefCell::new(Vec::new()),
            heads: RefCell::new(Vec::new()),
            stale: Cell::new(true),
        }
    }

    /// The snapshot changed: recompute on the next read.
    pub(crate) fn invalidate(&self) {
        self.stale.set(true);
    }

    fn ensure(&self) {
        let driver = &self.driver;
        let n = (driver.len)();
        // A changed item count is a change nobody announced yet (the piece refreshes its
        // snapshot a moment before it reloads the host): never serve rows for the old one.
        if !self.stale.get() && self.display.borrow().len() == n {
            return;
        }
        let Some(sections) = driver.sections.as_ref() else {
            return;
        };
        let mut rows = Vec::with_capacity(n + n / 4 + 1);
        let mut display = Vec::with_capacity(n);
        let mut heads = Vec::new();
        // How often each key has opened a section so far: a key that recurs further down
        // (a feed listed twice) opens a header of its own identity.
        let mut seen: HashMap<u64, u64> = HashMap::new();
        let mut previous = None;
        for i in 0..n {
            let key = (sections.key_at)(i);
            if previous != Some(key) {
                let nth = seen.entry(key).or_default();
                heads.push((rows.len(), Self::header_token(key, *nth)));
                *nth += 1;
                rows.push(DisplayRow::Header(i));
                previous = Some(key);
            }
            display.push(rows.len());
            rows.push(DisplayRow::Item(i));
        }
        *self.rows.borrow_mut() = rows;
        *self.display.borrow_mut() = display;
        *self.heads.borrow_mut() = heads;
        self.stale.set(false);
    }

    pub(crate) fn len(&self) -> usize {
        self.ensure();
        self.rows.borrow().len()
    }

    pub(crate) fn row(&self, display: usize) -> Option<DisplayRow> {
        self.ensure();
        self.rows.borrow().get(display).copied()
    }

    /// The item at display row `display`; `None` for a header or past the end.
    pub(crate) fn item_of(&self, display: usize) -> Option<usize> {
        match self.row(display)? {
            DisplayRow::Item(i) => Some(i),
            DisplayRow::Header(_) => None,
        }
    }

    /// The item a display row stands for: a header stands for its section's first item.
    pub(crate) fn nearest_item(&self, display: usize) -> Option<usize> {
        match self.row(display)? {
            DisplayRow::Item(i) | DisplayRow::Header(i) => Some(i),
        }
    }

    pub(crate) fn display_of(&self, item: usize) -> Option<usize> {
        self.ensure();
        self.display.borrow().get(item).copied()
    }

    /// The section around display row `display`, when that row is an item: the display
    /// rows of its first and last items, and the number of headers up to and including its
    /// own, which is what separates a display row from its item index inside the section.
    pub(crate) fn span(&self, display: usize) -> Option<SectionSpan> {
        self.item_of(display)?;
        let heads = self.heads.borrow();
        let section = heads
            .partition_point(|(h, _)| *h <= display)
            .checked_sub(1)?;
        let last = match heads.get(section + 1) {
            Some((next, _)) => next - 1,
            None => self.rows.borrow().len() - 1,
        };
        Some(SectionSpan {
            first: heads[section].0 + 1,
            last,
            headers: section + 1,
        })
    }

    /// The identity token of the header at display row `display`.
    pub(crate) fn header_token_at(&self, display: usize) -> Option<u64> {
        self.ensure();
        let heads = self.heads.borrow();
        let at = heads.binary_search_by_key(&display, |(h, _)| *h).ok()?;
        Some(heads[at].1)
    }

    /// A header's identity token: its section key and how many sections with that key came
    /// before it, so a section keeps its header while rows move inside it, and two sections
    /// with the same key (a feed that recurs further down) never share one.
    fn header_token(key: u64, nth: u64) -> u64 {
        (key ^ nth.wrapping_mul(0x9E37_79B9_7F4A_7C15)).rotate_left(29) ^ 0xD1B5_4A32_D192_ED03
    }

    /// A drag of display row `from` toward display row `to`, in item indices: a header never
    /// moves, and a row stays in its section, so a target past either end lands at that end
    /// (docs/list.md "Sections"). Answers the span with the clamped item move.
    fn move_within(&self, from: usize, to: usize) -> Option<(SectionSpan, usize, usize)> {
        let span = self.span(from)?;
        let to = to.clamp(span.first, span.last);
        Some((span, from - span.headers, to - span.headers))
    }
}

/// The rows of one section of a sectioned list, as [`SectionMap::span`] answers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SectionSpan {
    /// The display row of the section's first item.
    pub first: usize,
    /// The display row of the section's last item.
    pub last: usize,
    /// The headers at or above this section: an item's display row less this is its index.
    pub headers: usize,
}

impl SectionSpan {
    /// The display row of item `item`, when it falls inside this section.
    fn display_of(&self, item: usize) -> Option<usize> {
        let display = item + self.headers;
        (self.first..=self.last)
            .contains(&display)
            .then_some(display)
    }
}

thread_local! {
    /// Each sectioned list's map by node, for the event sink (which cannot borrow the tree).
    /// Weak: the list's state owns the map, and an entry outlives its list only until the next
    /// registration prunes it.
    static SECTION_MAPS: RefCell<HashMap<day_spec::NodeId, std::rc::Weak<SectionMap>>> =
        RefCell::new(HashMap::new());
}

/// The map of the sectioned list at `node`; `None` for a list without sections.
fn section_map(node: RNode) -> Option<Rc<SectionMap>> {
    let id = crate::tree::rnode_to_id(node);
    SECTION_MAPS.with(|m| m.borrow().get(&id).and_then(std::rc::Weak::upgrade))
}

/// The map a sectioned list at `node` shares with its source closures and the event sink.
pub(crate) fn register_sections(node: RNode, driver: Rc<ListDriver>) -> Rc<SectionMap> {
    let map = Rc::new(SectionMap::new(driver));
    SECTION_MAPS.with(|m| {
        let mut m = m.borrow_mut();
        m.retain(|_, w| w.strong_count() > 0);
        m.insert(crate::tree::rnode_to_id(node), Rc::downgrade(&map));
    });
    map
}

/// A toolkit's row event on a sectioned list, in display rows, as the item-index event the
/// piece expects. A header is not an item, so a selection or activation of one (a host that
/// failed to refuse it) is dropped; every other event passes unchanged.
pub(crate) fn translate_native_event(
    id: day_spec::NodeId,
    ev: day_spec::Event,
) -> Option<day_spec::Event> {
    use day_spec::Event;
    if !matches!(
        ev,
        Event::SelectionChanged(_) | Event::SelectionSet(_) | Event::ListActivated(_)
    ) {
        return Some(ev);
    }
    let Some(map) = SECTION_MAPS.with(|m| m.borrow().get(&id).and_then(std::rc::Weak::upgrade))
    else {
        return Some(ev);
    };
    match ev {
        Event::SelectionChanged(i) if i >= 0 => map
            .item_of(i as usize)
            .map(|i| Event::SelectionChanged(i as i64)),
        Event::SelectionSet(rows) => Some(Event::SelectionSet(
            rows.iter()
                .filter(|r| **r >= 0)
                .filter_map(|r| map.item_of(*r as usize))
                .map(|i| i as i64)
                .collect(),
        )),
        Event::ListActivated(i) => map.item_of(i).map(Event::ListActivated),
        other => Some(other),
    }
}

/// Whether a `bind_row` must build a new row (fresh anchor) or rebind a recycled cell.
pub enum CellStep {
    Build {
        anchor: RNode,
    },
    Rebind {
        rebind: Rc<dyn Fn(usize)>,
        anchor: RNode,
    },
}

/// Register a list's driver and wire its native host's data-source. Call after the list node and
/// its native handle exist (from within the piece build; `with_tree` is acquired per op).
pub fn install_list(node: RNode, driver: ListDriver) {
    with_tree(|t| t.install_list(node, driver));
}

/// Tell the native list its data changed (re-query the source). Call with no borrow held.
pub fn list_reload(node: RNode) {
    with_tree(|t| t.list_reload(node));
}

/// Imperatively scroll the native list so its last row is fully visible (chat "stick to bottom").
/// A no-op while the list is empty. Call with no borrow held.
/// Row-level deltas to the native host: the animatable alternative to [`list_reload`], for
/// sources that know exactly how their set changed (a live query's maintainer does).
pub fn list_splice(node: RNode, deltas: Vec<day_spec::props::RowDelta>) {
    with_tree(|t| t.list_splice(node, deltas));
}

pub fn list_scroll_to_end(node: RNode) {
    with_tree(|t| t.list_scroll_to_end(node));
}

/// Imperatively scroll the native list so row `row` is visible (docs/list.md), realizing it if
/// needed. Clamped to the row count; a no-op while the list is empty. Call with no borrow held.
pub fn list_scroll_to_row(node: RNode, row: usize) {
    with_tree(|t| t.list_scroll_to_row(node, row));
}

/// Programmatically sync the list's selected rows (empty = clear selection). Applied by the
/// toolkit without re-emitting a selection event. Call with no borrow held.
pub fn list_set_selected(node: RNode, rows: Vec<usize>) {
    with_tree(|t| t.list_set_selected(node, rows));
}

/// Enter or leave the list's native editing mode (`ListPatch::Editing`, docs/list.md).
pub fn list_set_editing(node: RNode, on: bool) {
    with_tree(|t| t.list_set_editing(node, on));
}

/// Programmatically reorder a list row through the same guard → commit path a native drag takes
/// (docs/list.md): consult the piece's guard (which may retarget), rotate the snapshot, defer the
/// app's `on_reorder`, and tell the native host to re-query. This is how the dayscript `reorder`
/// step and the mock probe drive reordering without a native gesture. Returns the index the row
/// actually landed at. Call with no borrow held.
pub fn list_try_reorder(node: RNode, from: usize, to: usize) -> Result<usize, &'static str> {
    let hook = SCRIPTED_REORDERS.with(|m| m.borrow().get(&crate::tree::rnode_to_id(node)).cloned());
    if let Some(hook) = hook {
        return hook(from, to);
    }
    let driver = with_tree(|t| t.list_driver(node)).ok_or("no list at this node")?;
    let Some(re) = driver.reorder.as_ref() else {
        return Err("list is not reorderable");
    };
    let len = (driver.len)();
    if from >= len || to >= len {
        return Err("row index out of bounds");
    }
    // A sectioned list keeps the row in its section, as the native drag does.
    let span = section_map(node).and_then(|map| {
        let span = map.span(map.display_of(from)?)?;
        Some((map, span))
    });
    let to = match &span {
        Some((map, span)) => {
            let display = map.display_of(to).unwrap_or(span.last);
            display.clamp(span.first, span.last) - span.headers
        }
        None => to,
    };
    let accepted = (re.can_move)(from, to);
    if accepted < 0 {
        return Err("reorder denied by the guard");
    }
    let accepted = (accepted as usize).min(len - 1);
    if let Some((_, span)) = &span
        && span.display_of(accepted).is_none()
    {
        return Err("reorder would leave the row's section");
    }
    if accepted != from {
        (re.moved)(from, accepted);
    }
    // No native animation on this path; a reload re-binds the visible rows in the new order.
    list_reload(node);
    Ok(accepted)
}

/// A scripted reorder for a node that reorders without a list driver: a toolkit's own sidebar
/// (`Cap::NavReorder` Native), whose rows the nav moves itself.
type ScriptedReorder = Rc<dyn Fn(usize, usize) -> Result<usize, &'static str>>;

thread_local! {
    static SCRIPTED_REORDERS: RefCell<HashMap<day_spec::NodeId, ScriptedReorder>> =
        RefCell::new(HashMap::new());
}

/// Route [`list_try_reorder`] at `node` (the dayscript `reorder` step) to `reorder`, for a piece
/// that reorders rows with no list driver of its own. `reorder` gets the step's `from` and `to`
/// and answers where the row landed, or why it did not move. Replaces any earlier route.
pub fn set_scripted_reorder(
    node: RNode,
    reorder: impl Fn(usize, usize) -> Result<usize, &'static str> + 'static,
) {
    let id = crate::tree::rnode_to_id(node);
    SCRIPTED_REORDERS.with(|m| m.borrow_mut().insert(id, Rc::new(reorder)));
}

/// Programmatically delete a list row through the same guard → commit path a native swipe takes
/// (docs/list.md): consult the piece's guard, drop the row from the snapshot, defer the app's
/// `on_delete`, and tell the native host to re-query. This is how the dayscript `delete_row` step
/// and the mock probe drive deletion without a native gesture. Call with no borrow held.
pub fn list_try_delete(node: RNode, index: usize) -> Result<(), &'static str> {
    let driver = with_tree(|t| t.list_driver(node)).ok_or("no list at this node")?;
    let Some(del) = driver.delete.as_ref() else {
        return Err("list is not deletable");
    };
    if index >= (driver.len)() {
        return Err("row index out of bounds");
    }
    if !(del.can_delete)(index) {
        return Err("delete denied by the guard");
    }
    (del.deleted)(index);
    // No native animation on this path; a reload re-binds the visible rows without the deleted
    // one.
    list_reload(node);
    Ok(())
}

/// Programmatically run a swipe action through the same offer → commit path a native gesture
/// takes (docs/list.md): pull the row's offer for the edge, refuse when the action is not in
/// it, and defer the commit. This is how the dayscript `swipe_row` step and the mock probe
/// drive the swipe path without a native gesture. `expect` pins the button by its label: offers
/// are state-dependent, and a pinned step must refuse to press a different button rather
/// than press it and complain (pressing anyway flips state the script did not mean to flip,
/// which is how one aborted run poisons every later one). Answers the activated label.
/// Call with no borrow held.
pub fn list_try_swipe(
    node: RNode,
    index: usize,
    edge: day_spec::SwipeEdge,
    action: usize,
    expect: Option<&str>,
) -> Result<String, String> {
    let driver = with_tree(|t| t.list_driver(node)).ok_or("no list at this node")?;
    let Some(sw) = driver.swipe.as_ref() else {
        return Err("list has no swipe actions".into());
    };
    if index >= (driver.len)() {
        return Err("row index out of bounds".into());
    }
    let offer = (sw.actions_at)(index, edge);
    let Some(a) = offer.get(action) else {
        return Err("no such action in this row's offer".into());
    };
    if let Some(want) = expect
        && a.label != want
    {
        return Err(format!(
            "the offer holds {:?}, expected {want:?} — not pressed",
            a.label
        ));
    }
    let label = a.label.clone();
    (sw.performed)(index, edge, action);
    Ok(label)
}

/// The opt-in first-visible report: enqueue-only, deduplicated, in item indices (a sectioned
/// list's host reports a display row; a header stands for its section's first item).
fn driver_viewport(
    node: RNode,
    driver: &ListDriver,
    sections: Option<Rc<SectionMap>>,
) -> Option<Rc<dyn Fn(usize)>> {
    if !driver.track_viewport {
        return None;
    }
    let previous = std::cell::Cell::new(None);
    Some(Rc::new(move |row| {
        let row = match &sections {
            Some(map) => match map.nearest_item(row) {
                Some(item) => item,
                None => return,
            },
            None => row,
        };
        if previous.replace(Some(row)) != Some(row) {
            crate::tree::enqueue_event(
                crate::tree::rnode_to_id(node),
                day_spec::Event::Custom {
                    tag: "day-list-first-visible",
                    num: row as f64,
                    text: String::new(),
                },
            );
        }
    }))
}

/// Build the `ListSource` the backend calls from its data-source. `len`/`token_at` read the driver
/// directly (no tree). `bind_row` phases the tree borrow around the build + flush (see module doc).
/// With `sections`, every closure speaks display rows and translates to the driver's items.
pub(crate) fn make_source(
    node: RNode,
    driver: Rc<ListDriver>,
    sections: Option<Rc<SectionMap>>,
) -> ListSource {
    // The row the host means, in the driver's terms: a plain list's rows are its items.
    let row_of = {
        let sections = sections.clone();
        move |display: usize| match &sections {
            Some(map) => map.row(display),
            None => Some(DisplayRow::Item(display)),
        }
    };
    let item_of = {
        let row_of = row_of.clone();
        move |display: usize| match row_of(display) {
            Some(DisplayRow::Item(i)) => Some(i),
            _ => None,
        }
    };
    let first_visible = driver_viewport(node, &driver, sections.clone());
    let sectioned = sections.is_some();
    ListSource {
        first_visible,
        len: {
            let (d, sections) = (driver.clone(), sections.clone());
            Rc::new(move || match &sections {
                Some(map) => map.len(),
                None => (d.len)(),
            })
        },
        token_at: {
            let (d, row_of, sections) = (driver.clone(), row_of.clone(), sections.clone());
            Rc::new(move |i| match row_of(i) {
                Some(DisplayRow::Item(i)) => (d.token_at)(i),
                Some(DisplayRow::Header(first)) => sections
                    .as_ref()
                    .and_then(|map| map.header_token_at(i))
                    .unwrap_or_else(|| (d.token_at)(first)),
                // Past the end (a stale pull mid-animation): any token, the row is empty.
                None => (d.token_at)(i),
            })
        },
        is_header: sections.as_ref().map(|map| {
            let map = map.clone();
            Rc::new(move |i| matches!(map.row(i), Some(DisplayRow::Header(_))))
                as Rc<dyn Fn(usize) -> bool>
        }),
        bind_row: bind_with(node, driver.clone(), row_of.clone(), false),
        bind_pinned: bind_with(node, driver.clone(), row_of.clone(), true),
        // The cell stays pooled; only its dayscript ids clear, so a hidden row past a shrunk
        // source stops answering lookups (`try_with_tree`: a backend may recycle while a
        // snapshot draw holds the borrow; the ids clear on the next pass then).
        layout_cell: Rc::new(move |cell, width| {
            // From the native cell's own layout pass: skip inside a snapshot borrow, the next
            // real pass corrects it (same rule as bind_row).
            let _ = try_with_tree(|t| t.list_layout_cell_width(node, cell as usize, width));
        }),
        cell_height: Rc::new(move |cell| {
            try_with_tree(|t| t.list_cell_height(node, cell as usize)).unwrap_or(0.0)
        }),
        recycle: Rc::new(move |cell| {
            let _ = try_with_tree(|t| t.list_recycle_cell(node, cell as usize));
        }),
        // The host's goodbye to a cell: dispose the row built in it. Never dropped under a
        // borrow, unlike the park above: a host tears cells down from inside a reload or a
        // removal Day itself started, and a release lost there is the leak this seam closes.
        release: Rc::new(move |cell| {
            let key = cell as usize;
            crate::tree::run_posted_or_defer(Box::new(move || {
                let _ = try_with_tree(|t| t.list_release_cell(node, key));
            }));
        }),
        // A sectioned list offers no native delete: the gesture animates one row out, and
        // deleting a section's last row takes its header with it, which the host's row count
        // would not expect (docs/list.md "Sections"). The scripted path still drives it.
        delete: driver
            .delete
            .as_ref()
            .filter(|_| !sectioned)
            .map(|_| day_spec::ListDelete {
                can_delete: {
                    let d = driver.clone();
                    Rc::new(move |i| (d.delete.as_ref().expect("mapped from Some").can_delete)(i))
                },
                delete_row: {
                    let d = driver.clone();
                    Rc::new(move |i| (d.delete.as_ref().expect("mapped from Some").deleted)(i))
                },
            }),
        // A sectioned list reorders within a section, the platforms' own rule for grouped
        // tables: a header never lifts, a drop past either end of the row's section lands at
        // that end, and the guard sees item indices (docs/list.md "Sections").
        reorder: driver.reorder.as_ref().map(|_| day_spec::ListReorder {
            can_move: {
                let (d, sections) = (driver.clone(), sections.clone());
                Rc::new(move |from, to| {
                    let re = d.reorder.as_ref().expect("mapped from Some");
                    let Some(map) = &sections else {
                        return (re.can_move)(from, to);
                    };
                    let Some((span, from, to)) = map.move_within(from, to) else {
                        return -1;
                    };
                    let accepted = (re.can_move)(from, to);
                    usize::try_from(accepted)
                        .ok()
                        .and_then(|i| span.display_of(i))
                        .map_or(-1, |display| display as i64)
                })
            },
            move_row: {
                let (d, sections) = (driver.clone(), sections.clone());
                Rc::new(move |from, to| {
                    let re = d.reorder.as_ref().expect("mapped from Some");
                    let Some(map) = &sections else {
                        return (re.moved)(from, to);
                    };
                    // Hosts commit only a target `can_move` accepted, which is in the section.
                    if let Some((_, from, to)) = map.move_within(from, to) {
                        (re.moved)(from, to);
                        map.invalidate();
                    }
                })
            },
        }),
        // Swipe actions belong to items: a header offers none.
        swipe: driver.swipe.as_ref().map(|_| day_spec::ListSwipe {
            actions_at: {
                let (d, item_of) = (driver.clone(), item_of.clone());
                Rc::new(move |i, e| match item_of(i) {
                    Some(i) => (d.swipe.as_ref().expect("mapped from Some").actions_at)(i, e),
                    None => Vec::new(),
                })
            },
            perform: {
                let (d, item_of) = (driver.clone(), item_of.clone());
                Rc::new(move |i, e, a| {
                    if let Some(i) = item_of(i) {
                        (d.swipe.as_ref().expect("mapped from Some").performed)(i, e, a)
                    }
                })
            },
        }),
    }
}

/// `bind_row`, and with `pinned` its `bind_pinned` twin: the same build or rebind, after which a
/// pinned copy's ids are parked again (`ListSource::bind_pinned`).
fn bind_with(
    node: RNode,
    d: Rc<ListDriver>,
    row_of: impl Fn(usize) -> Option<DisplayRow> + 'static,
    pinned: bool,
) -> Rc<dyn Fn(usize, RawHandle)> {
    Rc::new(move |index, cell| {
        let key = cell as usize;
        // A stale index past the end binds as an item: the driver answers it with an empty row,
        // as it always has.
        let (header, at) = match row_of(index) {
            Some(DisplayRow::Header(first)) => (true, first),
            Some(DisplayRow::Item(i)) => (false, i),
            None => (false, index),
        };
        // A backend snapshot draws the window while holding the tree borrow; if that draw
        // re-enters here (a lazy list realizing a row mid-`cacheDisplayInRect`), skip rather than
        // double-borrow; the row binds on the next real layout pass (tree.rs::try_with_tree).
        let Some(step) = try_with_tree(|t| t.list_prepare_cell(node, key, cell, header)) else {
            return;
        };
        match step {
            CellStep::Build { anchor } => {
                // Build outside the borrow; BuildCx re-acquires with_tree per op.
                let built = match (header, d.sections.as_ref()) {
                    (true, Some(s)) => (s.build)(at, anchor),
                    _ => (d.build)(at, anchor),
                };
                with_tree(|t| t.list_store_cell(node, key, anchor, built, header));
            }
            CellStep::Rebind { rebind, .. } => rebind(at),
        }
        // Apply the slot-write (or first bindings); reactive effects patch natives via their own
        // with_tree, so this too runs with no borrow held. Then lay the row out.
        day_reactive::flush_sync();
        with_tree(|t| {
            t.list_layout_cell(node, key);
            if pinned {
                t.list_pin_cell(node, key);
            }
        });
    })
}
