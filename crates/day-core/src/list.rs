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
//! (lay the row out in its cell). Holding the borrow across the build would deadlock the `RefCell`.

use crate::tree::{RNode, try_with_tree, with_tree};
use day_reactive::Scope;
use day_spec::{
    ListSource, RawHandle,
    props::{RowDelta, RowHeight},
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// Supplied by the `list()` piece, type-erased over the item type. day-core invokes these to
/// answer the native data-source and to build/rebind rows.
pub struct ListDriver {
    pub row_height: RowHeight,
    pub track_viewport: bool,
    /// Ask the host to report the scrolls the reader starts (`follow_end`, docs/list.md
    /// "Following the end"), delivered as [`day_spec::Event::ListReaderScrolled`].
    pub track_reader: bool,
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
    /// header rows and translates item indices at the toolkit boundary.
    pub sections: Option<ListSectionsDriver>,
    /// The list's header, footer, empty state and content padding (docs/list.md "Header,
    /// footer and empty state"). day-core shows them as rows of their own around the items,
    /// so every toolkit scrolls them with the list.
    pub supplements: ListSupplements,
}

/// Builds one supplementary row into `anchor`: the contract of [`ListDriver::build`] without
/// an item.
pub type SupplementBuild = Box<dyn Fn(RNode) -> BuiltRow>;

/// The supplementary rows the `list()` piece supplies (docs/list.md "Header, footer and empty
/// state"). Each is a display row the host binds like any other; none is an item, so none is
/// selected, activated, swiped, dragged or counted by a row index.
#[derive(Default)]
pub struct ListSupplements {
    /// Content above the first row, scrolling with the rows.
    pub header: Option<SupplementBuild>,
    /// Content below the last row.
    pub footer: Option<SupplementBuild>,
    /// Content shown in place of the rows while the list has none; it fills the viewport.
    pub empty: Option<SupplementBuild>,
    /// Space around the list's content, inside the scrolling area: the top and bottom scroll
    /// with the rows, the sides inset every row.
    pub padding: day_spec::Insets,
}

impl ListSupplements {
    /// Which supplementary rows a list with these has.
    fn shape(&self) -> SupplementShape {
        SupplementShape {
            pad_top: self.padding.top > 0.0,
            header: self.header.is_some(),
            empty: self.empty.is_some(),
            footer: self.footer.is_some(),
            pad_bottom: self.padding.bottom > 0.0,
        }
    }

    /// Whether the list has any supplementary row at all.
    pub(crate) fn any(&self) -> bool {
        let shape = self.shape();
        shape.pad_top || shape.header || shape.empty || shape.footer || shape.pad_bottom
    }

    /// Build supplementary row `which` into `anchor`. A padding row has no content.
    fn build(&self, which: Supplement, anchor: RNode) -> BuiltRow {
        let build = match which {
            Supplement::Header => self.header.as_ref(),
            Supplement::Footer => self.footer.as_ref(),
            Supplement::Empty => self.empty.as_ref(),
            Supplement::PadTop | Supplement::PadBottom => None,
        };
        match build {
            Some(build) => build(anchor),
            None => BuiltRow {
                scope: Scope::child(),
                rebind: Rc::new(|_| {}),
            },
        }
    }
}

/// A supplementary row of a list: a row that is neither an item nor a section header.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Supplement {
    /// The content padding above everything.
    PadTop,
    /// The list's header.
    Header,
    /// The empty state, present only while the list has no items.
    Empty,
    /// The list's footer.
    Footer,
    /// The content padding below everything.
    PadBottom,
}

impl Supplement {
    /// The row's identity token: fixed, so a host keeps the row across reloads and splices.
    fn token(self) -> u64 {
        0x5DA7_5C0F_F5E7_0000
            | match self {
                Supplement::PadTop => 1,
                Supplement::Header => 2,
                Supplement::Empty => 3,
                Supplement::Footer => 4,
                Supplement::PadBottom => 5,
            }
    }
}

/// Which supplementary rows a list has ([`ListSupplements::shape`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SupplementShape {
    pad_top: bool,
    header: bool,
    empty: bool,
    footer: bool,
    pad_bottom: bool,
}

/// What a bound cell holds. A host that pools every row together hands a cell of one kind to
/// a row of another now and then; the kinds are different pieces, so `bind_row` rebuilds the
/// cell instead of rebinding it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellKind {
    /// An item's row (built by [`ListDriver::build`]).
    Item,
    /// A section header (built by [`ListSectionsDriver::build`]).
    Header,
    /// A supplementary row (built from [`ListSupplements`]).
    Extra(Supplement),
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
    /// What the cell holds: an item's row, a section header, or a supplementary row.
    pub kind: CellKind,
    /// The identity token of the row the cell was last bound to ([`ListSource::token_at`]):
    /// how [`list_refresh_rows`] finds the row again after the rows around it moved.
    pub token: u64,
    /// The host's pinned copy of a header (`ListSource::bind_pinned`): its ids stay parked, so
    /// lookups find the header row in the list instead.
    pub pinned: bool,
}

pub(crate) struct ListState {
    pub driver: Rc<ListDriver>,
    /// Physical cell (native handle as usize) → its built row.
    pub cells: HashMap<usize, BoundCell>,
    /// The display rows of a list with section headers or supplementary rows; `None` without
    /// either, where display row and item index coincide.
    pub sections: Option<Rc<SectionMap>>,
    /// The host was last told the list has no items, so it shows the empty state (when the
    /// list has one). Starts true: the host attaches against an empty snapshot.
    pub showed_empty: bool,
}

/// One row as the native host sees it in a list with headers or supplementary rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DisplayRow {
    /// The header of the section whose first item is this index.
    Header(usize),
    Item(usize),
    /// A supplementary row: the list's header, footer, empty state or content padding.
    Extra(Supplement),
}

/// A list's display rows, solved from its item count, section keys and supplementary rows:
/// what [`SectionMap`] serves, and what [`list_sync`] diffs.
struct DisplayLayout {
    rows: Vec<DisplayRow>,
    /// Item index → its display row.
    display: Vec<usize>,
    /// The display row of each section header, in order, with the header's identity token.
    heads: Vec<(usize, u64)>,
}

impl DisplayLayout {
    /// Lay `n` items out: the supplementary rows around them, and a header row in front of
    /// every item whose section key differs from the item before.
    fn solve(n: usize, key_at: Option<&dyn Fn(usize) -> u64>, shape: SupplementShape) -> Self {
        let mut rows = Vec::with_capacity(n + n / 4 + 5);
        let mut display = Vec::with_capacity(n);
        let mut heads = Vec::new();
        if shape.pad_top {
            rows.push(DisplayRow::Extra(Supplement::PadTop));
        }
        if shape.header {
            rows.push(DisplayRow::Extra(Supplement::Header));
        }
        if n == 0 && shape.empty {
            rows.push(DisplayRow::Extra(Supplement::Empty));
        }
        // How often each key has opened a section so far: a key that recurs further down
        // (a feed listed twice) opens a header of its own identity.
        let mut seen: HashMap<u64, u64> = HashMap::new();
        let mut previous = None;
        for i in 0..n {
            if let Some(key_at) = key_at {
                let key = key_at(i);
                if previous != Some(key) {
                    let nth = seen.entry(key).or_default();
                    heads.push((rows.len(), SectionMap::header_token(key, *nth)));
                    *nth += 1;
                    rows.push(DisplayRow::Header(i));
                    previous = Some(key);
                }
            }
            display.push(rows.len());
            rows.push(DisplayRow::Item(i));
        }
        if shape.footer {
            rows.push(DisplayRow::Extra(Supplement::Footer));
        }
        if shape.pad_bottom {
            rows.push(DisplayRow::Extra(Supplement::PadBottom));
        }
        DisplayLayout {
            rows,
            display,
            heads,
        }
    }

    /// Every display row's identity token, given the items' tokens.
    fn tokens(&self, token_at: &dyn Fn(usize) -> u64) -> Vec<u64> {
        let mut heads = self.heads.iter();
        self.rows
            .iter()
            .map(|row| match row {
                DisplayRow::Item(i) => token_at(*i),
                DisplayRow::Header(_) => heads.next().map_or(0, |(_, token)| *token),
                DisplayRow::Extra(which) => which.token(),
            })
            .collect()
    }
}

/// The most row deltas [`diff_rows`] answers with. A change that needs more is a different
/// list more than an edited one: the host reloads, which costs it one pass instead of one
/// animated step per row.
pub const SPLICE_MAX_DELTAS: usize = 512;

/// The row deltas that turn `old` into `new`, two lists of row identity tokens, or `None`
/// when a reload serves better (docs/list.md "Keyed updates").
///
/// The deltas are sequential, as [`RowDelta`] defines them: removals first, from the bottom
/// up, then one insert per new row and one move per row that changed its place among the
/// rows that stayed, placed from the bottom up so each lands directly above a row already in
/// place. The moves are the fewest possible: the rows on a longest increasing run keep still.
///
/// `None` when either list is empty, when the two share no row, when a token repeats (a
/// repeated key cannot say which row moved), or when the change needs more than
/// [`SPLICE_MAX_DELTAS`] deltas. `Some(vec![])` when the lists are equal.
#[must_use]
pub fn diff_rows(old: &[u64], new: &[u64]) -> Option<Vec<RowDelta>> {
    if old == new {
        return Some(Vec::new());
    }
    if old.is_empty() || new.is_empty() {
        return None;
    }
    let mut new_at: HashMap<u64, usize> = HashMap::with_capacity(new.len());
    for (i, token) in new.iter().enumerate() {
        if new_at.insert(*token, i).is_some() {
            return None;
        }
    }
    // The survivors, in the old order, by their place in the new list.
    let mut seen = std::collections::HashSet::with_capacity(old.len());
    let mut survivors: Vec<usize> = Vec::with_capacity(old.len().min(new.len()));
    let mut deltas = Vec::new();
    for (i, token) in old.iter().enumerate().rev() {
        if !seen.insert(*token) {
            return None;
        }
        match new_at.get(token) {
            Some(at) => survivors.push(*at),
            None => deltas.push(RowDelta::Remove(i)),
        }
    }
    survivors.reverse();
    if survivors.is_empty() {
        return None;
    }
    let still = longest_increasing(&survivors);
    let changes = deltas.len() + (new.len() - survivors.len()) + (survivors.len() - still.len());
    if changes > SPLICE_MAX_DELTAS {
        return None;
    }
    let mut keeps = vec![false; new.len()];
    for at in &still {
        keeps[*at] = true;
    }
    let mut survives = vec![false; new.len()];
    for at in &survivors {
        survives[*at] = true;
    }
    // The working list: the survivors as the removals left them, as places in `new`.
    let mut work = survivors;
    for i in (0..new.len()).rev() {
        if keeps[i] {
            continue;
        }
        // Row `i` lands directly above the row that follows it in `new`, which is in place.
        let before = |work: &[usize]| {
            work.iter()
                .position(|at| *at == i + 1)
                .unwrap_or(work.len())
        };
        if survives[i] {
            let from = work.iter().position(|at| *at == i)?;
            work.remove(from);
            let to = before(&work);
            work.insert(to, i);
            if from != to {
                deltas.push(RowDelta::Move(from, to));
            }
        } else {
            let to = before(&work);
            work.insert(to, i);
            deltas.push(RowDelta::Insert(to));
        }
    }
    Some(deltas)
}

/// The values of one longest strictly increasing subsequence of `seq` (patience sorting).
fn longest_increasing(seq: &[usize]) -> Vec<usize> {
    // `tails[k]`: the index in `seq` of the smallest value ending a run of length `k + 1`.
    let mut tails: Vec<usize> = Vec::new();
    let mut previous: Vec<Option<usize>> = vec![None; seq.len()];
    for (i, value) in seq.iter().enumerate() {
        let k = tails.partition_point(|t| seq[*t] < *value);
        previous[i] = k.checked_sub(1).map(|k| tails[k]);
        if k == tails.len() {
            tails.push(i);
        } else {
            tails[k] = i;
        }
    }
    let mut run = Vec::with_capacity(tails.len());
    let mut at = tails.last().copied();
    while let Some(i) = at {
        run.push(seq[i]);
        at = previous[i];
    }
    run.reverse();
    run
}

/// A list's display rows (docs/list.md "Sections" and "Header, footer and empty state"): the
/// items with a header row in front of each section and the supplementary rows around them.
/// The native host counts, binds, selects and scrolls display rows; the
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
        let key_at = driver.sections.as_ref().map(|s| &*s.key_at);
        let layout = DisplayLayout::solve(n, key_at, driver.supplements.shape());
        *self.rows.borrow_mut() = layout.rows;
        *self.display.borrow_mut() = layout.display;
        *self.heads.borrow_mut() = layout.heads;
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
            DisplayRow::Header(_) | DisplayRow::Extra(_) => None,
        }
    }

    /// The items among display rows `first..=last`, as a half-open range of item indices:
    /// empty when the rows hold headers and supplementary rows only.
    pub(crate) fn items_in(&self, first: usize, last: usize) -> std::ops::Range<usize> {
        self.ensure();
        let display = self.display.borrow();
        let start = display.partition_point(|d| *d < first);
        let end = display.partition_point(|d| *d <= last);
        start..end.max(start)
    }

    /// Whether display row `display` is a supplementary row.
    pub(crate) fn is_supplement(&self, display: usize) -> bool {
        matches!(self.row(display), Some(DisplayRow::Extra(_)))
    }

    /// The display row of supplementary row `which`, when the list shows it.
    pub(crate) fn supplement_row(&self, which: Supplement) -> Option<usize> {
        self.ensure();
        self.rows
            .borrow()
            .iter()
            .position(|row| *row == DisplayRow::Extra(which))
    }

    /// How many display rows sit above the first item when no section header does: what
    /// separates an item index from its display row in a list without sections.
    pub(crate) fn lead(&self) -> usize {
        self.ensure();
        self.display.borrow().first().copied().unwrap_or_else(|| {
            self.rows
                .borrow()
                .iter()
                .take_while(|row| {
                    !matches!(
                        row,
                        DisplayRow::Extra(Supplement::Footer | Supplement::PadBottom)
                    )
                })
                .count()
        })
    }

    pub(crate) fn display_of(&self, item: usize) -> Option<usize> {
        self.ensure();
        self.display.borrow().get(item).copied()
    }

    /// The section around display row `display`, when that row is an item: the display
    /// rows of its first and last items, and the number of rows above it that are not items,
    /// which is what separates a display row from its item index inside the section. A list
    /// without sections is one section of all its items.
    pub(crate) fn span(&self, display: usize) -> Option<SectionSpan> {
        let item = self.item_of(display)?;
        let heads = self.heads.borrow();
        let items = self.display.borrow();
        let (first, last) = if heads.is_empty() {
            (*items.first()?, *items.last()?)
        } else {
            let section = heads
                .partition_point(|(h, _)| *h <= display)
                .checked_sub(1)?;
            let last = match heads.get(section + 1) {
                Some((next, _)) => next - 1,
                None => *items.last()?,
            };
            (heads[section].0 + 1, last)
        };
        Some(SectionSpan {
            first,
            last,
            headers: display - item,
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
    /// The rows above this section's items that are not items (headers and supplementary
    /// rows): an item's display row less this is its index.
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

day_reactive::tls_slots! {
    list;
    /// Each sectioned list's map by node, for the event sink (which cannot borrow the tree).
    /// Weak: the list's state owns the map, and an entry outlives its list only until the next
    /// registration prunes it.
    static SECTION_MAPS: RefCell<HashMap<day_spec::NodeId, std::rc::Weak<SectionMap>>> =
        RefCell::new(HashMap::new());
    /// [`set_scripted_reorder`]'s routes by node.
    static SCRIPTED_REORDERS: RefCell<HashMap<day_spec::NodeId, ScriptedReorder>> =
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

/// Row-level deltas to the native host, in item indices: the animatable alternative to
/// [`list_reload`], for sources that know exactly how their set changed (a live query's
/// maintainer does).
pub fn list_splice(node: RNode, deltas: Vec<RowDelta>) {
    with_tree(|t| t.list_splice(node, deltas));
}

/// One snapshot of a list's rows, for [`list_sync`]: the items' identity tokens, and their
/// section keys when the list is sectioned.
#[derive(Clone, Copy, Debug)]
pub struct ListRows<'a> {
    pub tokens: &'a [u64],
    pub sections: Option<&'a [u64]>,
}

/// What [`list_sync`] told the native host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListSync {
    /// The rows are the same rows in the same order: the host heard nothing.
    Unchanged,
    /// The host got this many row deltas ([`day_spec::props::ListPatch::Splice`]).
    Spliced(usize),
    /// The change was one a splice cannot express ([`diff_rows`]): the host re-queries.
    Reloaded,
}

/// Tell the native host how the list's rows changed, from the snapshot it shows (`old`) to
/// the one the driver now serves (`new`), with the smallest patch that says it
/// (docs/list.md "Keyed updates"): nothing, a splice, or a reload. Section headers and
/// supplementary rows are diffed with the items, so a sectioned list splices too. Rows that
/// stay bound are then rebound to their items ([`list_refresh_rows`]), which is how a
/// changed value reaches a row whose slot holds a copy. Call with no borrow held.
pub fn list_sync(node: RNode, old: ListRows<'_>, new: ListRows<'_>) -> ListSync {
    let Some(driver) = with_tree(|t| t.list_driver(node)) else {
        return ListSync::Unchanged;
    };
    let shape = driver.supplements.shape();
    let display = |rows: ListRows<'_>| {
        let keys = rows.sections;
        let key_at = |i: usize| keys.and_then(|k| k.get(i)).copied().unwrap_or_default();
        let key_at: Option<&dyn Fn(usize) -> u64> = keys.is_some().then_some(&key_at);
        DisplayLayout::solve(rows.tokens.len(), key_at, shape)
            .tokens(&|i| rows.tokens.get(i).copied().unwrap_or_default())
    };
    let outcome = match diff_rows(&display(old), &display(new)) {
        Some(deltas) if deltas.is_empty() => ListSync::Unchanged,
        Some(deltas) => {
            let count = deltas.len();
            with_tree(|t| t.list_splice_rows(node, deltas));
            ListSync::Spliced(count)
        }
        None => {
            list_reload(node);
            ListSync::Reloaded
        }
    };
    // A reload makes the host rebind what it shows; anything less leaves the bound rows to us.
    if outcome != ListSync::Reloaded {
        list_refresh_rows(node);
    }
    outcome
}

/// A bound cell as [`list_refresh_rows`] sees it.
pub struct LiveCell {
    /// The physical cell (the native handle as `usize`).
    pub key: usize,
    /// The identity token of the row the cell was last bound to.
    pub token: u64,
    /// What the cell holds.
    pub kind: CellKind,
    /// The row's rebind writer ([`BuiltRow::rebind`]).
    pub rebind: Rc<dyn Fn(usize)>,
}

/// Rebind every row the list has bound to the item that now carries its identity token, and
/// lay it out again: the visible rows only, with no native reload. A row whose token left the
/// list is skipped (the host recycles its cell). Call with no borrow held.
pub fn list_refresh_rows(node: RNode) {
    let Some((driver, cells)) = with_tree(|t| {
        let driver = t.list_driver(node)?;
        Some((driver, t.list_live_cells(node)))
    }) else {
        return;
    };
    if cells.is_empty() {
        return;
    }
    let map = section_map(node);
    // Where each bound row now sits: one pass over the rows, looking only for bound tokens.
    let mut wanted: HashMap<u64, Vec<usize>> = HashMap::with_capacity(cells.len());
    for (at, cell) in cells.iter().enumerate() {
        wanted.entry(cell.token).or_default().push(at);
    }
    let mut found: Vec<Option<usize>> = vec![None; cells.len()];
    match &map {
        Some(map) => {
            for display in 0..map.len() {
                let (token, item) = match map.row(display) {
                    Some(DisplayRow::Item(i)) => ((driver.token_at)(i), i),
                    Some(DisplayRow::Header(first)) => {
                        (map.header_token_at(display).unwrap_or_default(), first)
                    }
                    Some(DisplayRow::Extra(which)) => (which.token(), 0),
                    None => continue,
                };
                for at in wanted.get(&token).into_iter().flatten() {
                    found[*at] = Some(item);
                }
            }
        }
        None => {
            for i in 0..(driver.len)() {
                for at in wanted.get(&(driver.token_at)(i)).into_iter().flatten() {
                    found[*at] = Some(i);
                }
            }
        }
    }
    let mut touched = Vec::with_capacity(cells.len());
    for (cell, item) in cells.iter().zip(found) {
        if let Some(item) = item {
            // A supplementary row has no item: its bindings follow their own signals.
            if !matches!(cell.kind, CellKind::Extra(_)) {
                (cell.rebind)(item);
            }
            touched.push(cell.key);
        }
    }
    // The slot writes patch natives through their own `with_tree`; then lay the rows out.
    day_reactive::flush_sync();
    with_tree(|t| {
        for key in touched {
            t.list_layout_cell(node, key);
        }
    });
}

/// Imperatively scroll the native list to the very end of its content (a chat timeline
/// following its newest message). A no-op while the list is empty. `animated` asks for the
/// platform's scroll animation, which the reduce-motion setting turns off. Call with no
/// borrow held.
pub fn list_scroll_to_end(node: RNode, animated: bool) {
    with_tree(|t| t.list_scroll(node, None, day_spec::RowAlign::Nearest, animated));
}

/// Imperatively scroll the native list so row `row` is visible (docs/list.md), realizing it if
/// needed, landing where `align` says. Clamped to the row count; a no-op while the list is
/// empty. Call with no borrow held.
pub fn list_scroll_to_row(node: RNode, row: usize, align: day_spec::RowAlign, animated: bool) {
    with_tree(|t| t.list_scroll(node, Some(row), align, animated));
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
///
/// # Errors
///
/// Returns the reason when `node` has no list, the list is not reorderable, an index is out of
/// bounds, the guard denies the move, or the move would leave the row's section.
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
///
/// # Errors
///
/// Returns the reason when `node` has no list, the list is not deletable, `index` is out of
/// bounds, or the guard denies the delete.
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
///
/// # Errors
///
/// Returns the reason when `node` has no list, the list has no swipe actions, `index` is out of
/// bounds, the row's offer has no such action, or the action's label differs from `expect`.
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

/// The host's two viewport reports, a full one and the first-visible-only one, both
/// enqueue-only, deduplicated, and translated to item indices: the opt-in source of
/// [`day_spec::Event::ListViewport`].
type ViewportReports = (
    Option<Rc<dyn Fn(usize)>>,
    Option<Rc<dyn Fn(day_spec::ListViewport)>>,
    Rc<Cell<Option<ViewportKey>>>,
);

/// What makes one viewport report differ from the next: the visible items' range, the
/// distance to the end by its bits, and the row count.
type ViewportKey = (usize, usize, Option<u64>, usize);

fn driver_viewport(
    node: RNode,
    driver: &Rc<ListDriver>,
    sections: Option<Rc<SectionMap>>,
) -> ViewportReports {
    // The last report, by its bits: a distance is compared exactly, and `None` is its own value.
    let previous: Rc<Cell<Option<ViewportKey>>> = Rc::new(Cell::new(None));
    if !driver.track_viewport {
        return (None, None, previous);
    }
    let report = {
        let (driver, previous) = (driver.clone(), previous.clone());
        // `last` is `None` from a host that reports its first visible row only: the range
        // then holds that one row.
        Rc::new(
            move |first: usize, last: Option<usize>, end_distance: Option<f64>| {
                let n = (driver.len)();
                let rows = match (&sections, last) {
                    (Some(map), Some(last)) => map.items_in(first, last),
                    // A header stands for its section's first item.
                    (Some(map), None) => {
                        let start = map.items_in(first, first).start;
                        start..(start + 1).min(n)
                    }
                    (None, Some(last)) => first.min(n)..(last + 1).min(n),
                    (None, None) => first.min(n)..(first + 1).min(n),
                };
                let rows = rows.start..rows.end.max(rows.start);
                // The row count is part of the key: a report after the rows changed always
                // goes through, which is what re-arms `on_end_reached`.
                let key = (rows.start, rows.end, end_distance.map(f64::to_bits), n);
                if previous.replace(Some(key)) != Some(key) {
                    crate::tree::enqueue_event(
                        crate::tree::rnode_to_id(node),
                        day_spec::Event::ListViewport { rows, end_distance },
                    );
                }
            },
        )
    };
    let first_only = {
        let report = report.clone();
        Rc::new(move |row: usize| report(row, None, None)) as Rc<dyn Fn(usize)>
    };
    let full = Rc::new(move |v: day_spec::ListViewport| {
        report(
            v.first,
            Some(v.last.max(v.first)),
            Some(v.end_distance.max(0.0)),
        );
    }) as Rc<dyn Fn(day_spec::ListViewport)>;
    (Some(first_only), Some(full), previous)
}

/// Build the `ListSource` the backend calls from its data-source. `len`/`token_at` read the driver
/// directly (no tree). `bind_row` phases the tree borrow around the build + flush (see module doc).
/// With `sections`, every closure speaks display rows and translates to the driver's items.
pub(crate) fn make_source(
    node: RNode,
    driver: &Rc<ListDriver>,
    sections: Option<&Rc<SectionMap>>,
) -> ListSource {
    // The row the host means, in the driver's terms: a plain list's rows are its items.
    let row_of = {
        let sections = sections.cloned();
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
    let (first_visible, viewport, previous_report) =
        driver_viewport(node, driver, sections.cloned());
    let sectioned = driver.sections.is_some();
    let token_at: Rc<dyn Fn(usize) -> u64> = {
        let (d, row_of, sections) = (driver.clone(), row_of.clone(), sections.cloned());
        Rc::new(move |i| match row_of(i) {
            Some(DisplayRow::Item(i)) => (d.token_at)(i),
            Some(DisplayRow::Header(first)) => sections
                .as_ref()
                .and_then(|map| map.header_token_at(i))
                .unwrap_or_else(|| (d.token_at)(first)),
            Some(DisplayRow::Extra(which)) => which.token(),
            // Past the end (a stale pull mid-animation): any token, the row is empty.
            None => (d.token_at)(i),
        })
    };
    let reader_scroll = driver.track_reader.then(|| {
        let id = crate::tree::rnode_to_id(node);
        Rc::new(move || {
            // The piece counts the reader as away until the list next reports, so that report
            // must reach it even when it repeats the last one (a drag against the end).
            previous_report.set(None);
            crate::tree::enqueue_event(id, day_spec::Event::ListReaderScrolled);
        }) as Rc<dyn Fn()>
    });
    ListSource {
        first_visible,
        viewport,
        reader_scroll,
        is_supplement: sections.filter(|_| driver.supplements.any()).map(|map| {
            let map = map.clone();
            Rc::new(move |i| map.is_supplement(i)) as Rc<dyn Fn(usize) -> bool>
        }),
        len: {
            let (d, sections) = (driver.clone(), sections.cloned());
            Rc::new(move || match &sections {
                Some(map) => map.len(),
                None => (d.len)(),
            })
        },
        token_at: token_at.clone(),
        is_header: sections.filter(|_| sectioned).map(|map| {
            let map = map.clone();
            Rc::new(move |i| matches!(map.row(i), Some(DisplayRow::Header(_))))
                as Rc<dyn Fn(usize) -> bool>
        }),
        bind_row: bind_with(
            node,
            driver.clone(),
            row_of.clone(),
            token_at.clone(),
            false,
        ),
        bind_pinned: bind_with(node, driver.clone(), row_of.clone(), token_at, true),
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
                // Only an item deletes: a supplementary row answers no.
                can_delete: {
                    let (d, item_of) = (driver.clone(), item_of.clone());
                    Rc::new(move |i| {
                        item_of(i).is_some_and(|i| {
                            (d.delete.as_ref().expect("mapped from Some").can_delete)(i)
                        })
                    })
                },
                delete_row: {
                    let (d, item_of) = (driver.clone(), item_of.clone());
                    Rc::new(move |i| {
                        if let Some(i) = item_of(i) {
                            (d.delete.as_ref().expect("mapped from Some").deleted)(i);
                        }
                    })
                },
            }),
        // A sectioned list reorders within a section, the platforms' own rule for grouped
        // tables: a header never lifts, a drop past either end of the row's section lands at
        // that end, and the guard sees item indices (docs/list.md "Sections").
        reorder: driver.reorder.as_ref().map(|_| day_spec::ListReorder {
            can_move: {
                let (d, sections) = (driver.clone(), sections.cloned());
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
                let (d, sections) = (driver.clone(), sections.cloned());
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
                        (d.swipe.as_ref().expect("mapped from Some").performed)(i, e, a);
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
    token_of: Rc<dyn Fn(usize) -> u64>,
    pinned: bool,
) -> Rc<dyn Fn(usize, RawHandle)> {
    Rc::new(move |index, cell| {
        let key = cell as usize;
        // A stale index past the end binds as an item: the driver answers it with an empty row,
        // as it always has.
        let (kind, at) = match row_of(index) {
            Some(DisplayRow::Header(first)) => (CellKind::Header, first),
            Some(DisplayRow::Item(i)) => (CellKind::Item, i),
            Some(DisplayRow::Extra(which)) => (CellKind::Extra(which), 0),
            None => (CellKind::Item, index),
        };
        let token = token_of(index);
        // A backend snapshot draws the window while holding the tree borrow; if that draw
        // re-enters here (a lazy list realizing a row mid-`cacheDisplayInRect`), skip rather than
        // double-borrow; the row binds on the next real layout pass (tree.rs::try_with_tree).
        let Some(step) = try_with_tree(|t| t.list_prepare_cell(node, key, cell, kind, token))
        else {
            return;
        };
        match step {
            CellStep::Build { anchor } => {
                // Build outside the borrow; BuildCx re-acquires with_tree per op.
                let built = match (kind, d.sections.as_ref()) {
                    (CellKind::Header, Some(s)) => (s.build)(at, anchor),
                    (CellKind::Extra(which), _) => d.supplements.build(which, anchor),
                    _ => (d.build)(at, anchor),
                };
                with_tree(|t| t.list_store_cell(node, key, anchor, built, kind, token));
            }
            // A supplementary row has no item to rebind to.
            CellStep::Rebind { rebind, .. } => {
                if !matches!(kind, CellKind::Extra(_)) {
                    rebind(at);
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply sequential deltas to `rows`, taking each inserted row from `new`.
    fn apply(old: &[u64], new: &[u64], deltas: &[RowDelta]) -> Vec<u64> {
        // An insert names an index; the row that goes there is whichever of `new` is missing,
        // found once every delta has run. Mark the holes, then fill them in order.
        let mut rows: Vec<Option<u64>> = old.iter().copied().map(Some).collect();
        for delta in deltas {
            match *delta {
                RowDelta::Insert(at) => rows.insert(at, None),
                RowDelta::Remove(at) => {
                    rows.remove(at);
                }
                RowDelta::Move(from, to) => {
                    let row = rows.remove(from);
                    rows.insert(to, row);
                }
            }
        }
        assert_eq!(rows.len(), new.len(), "row count after {deltas:?}");
        rows.iter()
            .zip(new)
            .map(|(row, want)| row.unwrap_or(*want))
            .collect()
    }

    fn check(old: &[u64], new: &[u64]) -> Vec<RowDelta> {
        let deltas = diff_rows(old, new).expect("a splice");
        assert_eq!(
            apply(old, new, &deltas),
            new,
            "{old:?} -> {new:?} by {deltas:?}"
        );
        deltas
    }

    #[test]
    fn equal_lists_need_no_delta() {
        assert_eq!(diff_rows(&[1, 2, 3], &[1, 2, 3]), Some(Vec::new()));
        assert_eq!(diff_rows(&[], &[]), Some(Vec::new()));
    }

    #[test]
    fn an_append_is_one_insert_at_the_end() {
        assert_eq!(check(&[1, 2, 3], &[1, 2, 3, 4]), vec![RowDelta::Insert(3)]);
    }

    #[test]
    fn a_prepend_is_one_insert_at_the_top() {
        assert_eq!(check(&[1, 2, 3], &[9, 1, 2, 3]), vec![RowDelta::Insert(0)]);
    }

    #[test]
    fn inserts_in_the_middle_are_sequential() {
        let deltas = check(&[1, 2, 3], &[1, 7, 8, 2, 3]);
        assert_eq!(deltas.len(), 2);
        assert!(deltas.iter().all(|d| matches!(d, RowDelta::Insert(_))));
    }

    #[test]
    fn removals_run_from_the_bottom_up() {
        assert_eq!(
            check(&[1, 2, 3, 4, 5], &[1, 3, 5]),
            vec![RowDelta::Remove(3), RowDelta::Remove(1)]
        );
    }

    #[test]
    fn one_row_moved_is_one_move() {
        assert_eq!(
            check(&[1, 2, 3, 4], &[2, 3, 4, 1]),
            vec![RowDelta::Move(0, 3)]
        );
        assert_eq!(
            check(&[1, 2, 3, 4], &[4, 1, 2, 3]),
            vec![RowDelta::Move(3, 0)]
        );
        assert_eq!(check(&[1, 2, 3, 4, 5], &[1, 4, 2, 3, 5]).len(), 1);
    }

    #[test]
    fn a_swap_of_neighbors_is_one_move() {
        assert_eq!(check(&[1, 2, 3], &[1, 3, 2]).len(), 1);
    }

    #[test]
    fn a_reversal_moves_all_but_one_row() {
        let deltas = check(&[1, 2, 3, 4, 5], &[5, 4, 3, 2, 1]);
        assert_eq!(deltas.len(), 4);
        assert!(deltas.iter().all(|d| matches!(d, RowDelta::Move(..))));
    }

    #[test]
    fn mixed_changes_apply_in_order() {
        check(&[1, 2, 3, 4, 5, 6], &[6, 9, 2, 8, 4, 1]);
        check(&[10, 20, 30], &[30, 15, 10, 5]);
        check(&[1, 2, 3, 4, 5, 6, 7, 8], &[8, 1, 3, 2, 11, 7, 12]);
    }

    #[test]
    fn every_small_permutation_with_edits_round_trips() {
        // A deterministic shuffle (a multiplicative generator), over many shapes of change.
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % bound as u64) as usize
        };
        for round in 0..400 {
            let n = 1 + next(12);
            let old: Vec<u64> = (0..n as u64).collect();
            let mut new = old.clone();
            for _ in 0..next(6) {
                if !new.is_empty() {
                    let at = next(new.len());
                    new.remove(at);
                }
            }
            for k in 0..next(6) {
                let at = next(new.len() + 1);
                new.insert(at, 1000 + round * 10 + k as u64);
            }
            for _ in 0..next(5) {
                if new.len() > 1 {
                    let (a, b) = (next(new.len()), next(new.len()));
                    new.swap(a, b);
                }
            }
            match diff_rows(&old, &new) {
                Some(deltas) => assert_eq!(apply(&old, &new, &deltas), new, "{old:?} -> {new:?}"),
                // Only a list with nothing in common, or an empty one, reloads here.
                None => assert!(
                    new.is_empty() || !new.iter().any(|t| old.contains(t)),
                    "{old:?} -> {new:?} should splice"
                ),
            }
        }
    }

    #[test]
    fn what_a_splice_cannot_say_reloads() {
        // From or to nothing, nothing in common, a repeated token on either side.
        assert_eq!(diff_rows(&[], &[1]), None);
        assert_eq!(diff_rows(&[1], &[]), None);
        assert_eq!(diff_rows(&[1, 2], &[3, 4]), None);
        assert_eq!(diff_rows(&[1, 1, 2], &[1, 2]), None);
        assert_eq!(diff_rows(&[1, 2], &[1, 2, 2]), None);
    }

    #[test]
    fn a_change_past_the_limit_reloads() {
        let old: Vec<u64> = (0..2000).collect();
        let mut grown = old.clone();
        grown.extend(5000..5000 + SPLICE_MAX_DELTAS as u64);
        assert_eq!(check(&old, &grown).len(), SPLICE_MAX_DELTAS);
        grown.push(9999);
        assert_eq!(diff_rows(&old, &grown), None);
        let reversed: Vec<u64> = old.iter().rev().copied().collect();
        assert_eq!(diff_rows(&old, &reversed), None);
    }

    #[test]
    fn the_longest_run_keeps_still() {
        assert_eq!(longest_increasing(&[3, 0, 1, 2]), vec![0, 1, 2]);
        assert_eq!(longest_increasing(&[0, 1, 2]), vec![0, 1, 2]);
        assert_eq!(longest_increasing(&[2, 1, 0]).len(), 1);
        assert_eq!(longest_increasing(&[]), [] as [usize; 0]);
    }

    fn shape(header: bool, empty: bool, footer: bool) -> SupplementShape {
        SupplementShape {
            header,
            empty,
            footer,
            ..SupplementShape::default()
        }
    }

    #[test]
    fn sections_put_a_header_before_each_run_of_one_key() {
        let keys = [7_u64, 7, 8, 7];
        let key_at = |i: usize| keys[i];
        let layout = DisplayLayout::solve(4, Some(&key_at), SupplementShape::default());
        assert_eq!(
            layout.rows,
            vec![
                DisplayRow::Header(0),
                DisplayRow::Item(0),
                DisplayRow::Item(1),
                DisplayRow::Header(2),
                DisplayRow::Item(2),
                DisplayRow::Header(3),
                DisplayRow::Item(3),
            ]
        );
        assert_eq!(layout.display, vec![1, 2, 4, 6]);
        // The key that recurs opens a header of its own identity.
        assert_ne!(layout.heads[0].1, layout.heads[2].1);
    }

    #[test]
    fn supplements_surround_the_items_and_the_empty_state_stands_in_for_them() {
        let layout = DisplayLayout::solve(2, None, shape(true, true, true));
        assert_eq!(
            layout.rows,
            vec![
                DisplayRow::Extra(Supplement::Header),
                DisplayRow::Item(0),
                DisplayRow::Item(1),
                DisplayRow::Extra(Supplement::Footer),
            ]
        );
        let empty = DisplayLayout::solve(0, None, shape(true, true, true));
        assert_eq!(
            empty.rows,
            vec![
                DisplayRow::Extra(Supplement::Header),
                DisplayRow::Extra(Supplement::Empty),
                DisplayRow::Extra(Supplement::Footer),
            ]
        );
    }

    #[test]
    fn a_sectioned_change_diffs_headers_with_the_rows() {
        // Two sections; a row joins the second. The headers keep their tokens, so the
        // change is the one insert.
        let tokens = |layout: &DisplayLayout, items: &[u64]| layout.tokens(&|i| items[i]);
        let (old_items, old_keys) = ([1_u64, 2, 3], [10_u64, 10, 20]);
        let (new_items, new_keys) = ([1_u64, 2, 3, 4], [10_u64, 10, 20, 20]);
        let old = DisplayLayout::solve(3, Some(&|i| old_keys[i]), SupplementShape::default());
        let new = DisplayLayout::solve(4, Some(&|i| new_keys[i]), SupplementShape::default());
        let (old, new) = (tokens(&old, &old_items), tokens(&new, &new_items));
        assert_eq!(check(&old, &new), vec![RowDelta::Insert(5)]);

        // A row that opens a new section brings its header: two inserts.
        let newer_keys = [10_u64, 10, 20, 30];
        let newer = DisplayLayout::solve(4, Some(&|i| newer_keys[i]), SupplementShape::default());
        let newer = tokens(&newer, &new_items);
        assert_eq!(check(&old, &newer).len(), 2);

        // The last row of a section leaves and takes its header with it.
        let (fewer_items, fewer_keys) = ([1_u64, 2], [10_u64, 10]);
        let fewer = DisplayLayout::solve(2, Some(&|i| fewer_keys[i]), SupplementShape::default());
        let fewer = tokens(&fewer, &fewer_items);
        assert_eq!(check(&old, &fewer).len(), 2);
    }

    #[test]
    fn the_first_row_replaces_the_empty_state_by_a_splice() {
        let empty = DisplayLayout::solve(0, None, shape(true, true, false));
        let one = DisplayLayout::solve(1, None, shape(true, true, false));
        let (old, new) = (empty.tokens(&|_| 0), one.tokens(&|_| 42));
        // The header stays; the empty state goes and the row comes.
        let deltas = check(&old, &new);
        assert_eq!(deltas.len(), 2);
    }
}
