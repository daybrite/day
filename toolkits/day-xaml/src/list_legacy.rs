// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! The deprecated system-XAML list adapter. WinUI uses list.rs.
use super::*;

day_core::tls_group! {
    static LIST_STATE: RefCell<HashMap<usize, ListEntry>> = RefCell::new(HashMap::new());
    static LIST_BY_NODE: RefCell<HashMap<u64, usize>> = RefCell::new(HashMap::new());
}

// ---------------------------------------------------------------------------
// Recycling list (docs/list.md, §10). XAML's ListView virtualizes with a data source, which
// doesn't fit Day's synchronous `bind_row` pull; instead — like the Qt backend (DP-19) — Day
// EMULATES recycling: a ScrollViewer whose content Canvas holds one absolutely-positioned cell
// per row, each filled through the same `bind_row` seam. Cells are pooled append-only.
//
// Only the rows the viewport SHOWS are realized, which is the promise `list` makes over `each`
// (docs/list.md: "builds only the rows the native widget currently shows"). A cell is created
// and bound the first time its row scrolls into the window, and the ScrollViewer's ViewChanged
// brings the next ones in; the content Canvas is sized to the whole extent from the start, so
// the scrollbar tells the truth about a set nothing has built yet. Building every row instead
// is what a ten-thousand-row query cost before: ten thousand WinRT elements and ten thousand
// row layouts, on the UI thread, before the first paint.
//
// Cell index stays == row index for the cell's whole life, so nothing about selection, the
// press handler's row, or a drag's slot changes — this defers realization, it does not recycle
// one cell across rows. What a realized cell can go stale on is its CONTENT, so anything that
// changes the source (a reload, a reorder, a width that re-lays every row) marks the bindings
// dirty and the window rebinds; rows outside it rebind when they next scroll in.
// ---------------------------------------------------------------------------

/// Rows built beyond each edge of the viewport, so a flick has something to show before the
/// scroll callback lands. Cheap: a row costs one populate pass, and this is a handful of them.
const LIST_OVERSCAN: usize = 8;

struct ListEntry {
    host: *mut c_void,
    content: *mut c_void,
    /// The declared pitch (`RowHeight::Uniform`), or the estimate a content-sized list starts
    /// from before it has measured any row.
    row_height: f64,
    /// `RowHeight::Automatic`: each row stands the height Day laid it at (`heights`), placed one
    /// under the other, rather than every row standing `row_height`.
    automatic: bool,
    /// A content-sized list's measured heights, by row (None = not built since the source last
    /// changed, which then stands the average of the measured ones).
    heights: Vec<Option<f64>>,
    /// A measured height changed: the next fill moves every built cell to its new top.
    relayout: bool,
    source: Rc<RefCell<Option<day_spec::ListSource>>>,
    /// One slot per row, NULL until that row has been shown: cell index == row index, so a
    /// slot's identity never changes and a realized cell stays that row's for good.
    cells: Vec<*mut c_void>,
    /// Which rows' content is currently built and current. Cleared wholesale when the source
    /// changes under the cells; individual rows go true as they are bound.
    bound: Vec<bool>,
    /// Last host width a populate ran at, so `set_frame` only repopulates on a real width change
    /// (a populate's own child `set_frame`s must not schedule another, or it loops forever).
    last_width: c_int,
    /// The width day's layout last framed the host at (-1 = never framed). This — not the host's
    /// `ActualWidth`, which lags a posted populate by a layout pass — is what cells are sized from.
    frame_width: c_int,
    /// The height day's layout last framed the host at, read the same way and for the same
    /// reason: `ViewportHeight` is 0 until XAML has laid the ScrollViewer out, and a window of
    /// zero rows would leave the first paint empty.
    frame_height: c_int,
    /// A scroll-driven fill is already posted — ViewChanged fires many times through one flick,
    /// and they would each post a pass that does the same work.
    fill_pending: bool,
    /// A row jump (`ListPatch::ScrollToRow`) for the next fill to repeat: one asked for before
    /// the first fill (a list the page scrolls as it builds) meets content with no extent yet,
    /// and the ScrollViewer clamps it to the top.
    pending_scroll: Option<usize>,
    /// Drag-to-reorder (docs/list.md): whether new cells get the WinRT drag armed.
    reorderable: bool,
    node: u64,
    /// Selection (docs/list.md): whether rows select at all, whether several may be selected at
    /// once, the currently selected rows, and the last plainly-clicked row (the shift anchor).
    selectable: bool,
    multi: bool,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
}

/// Where each row of an emulated list sits on its content Canvas.
enum RowGrid {
    /// Every row the same pitch: row i at `i * pitch`, as the uniform list has always placed it.
    Uniform(f64),
    /// A content-sized list: each row's top, then the extent after the last (`n + 1` entries).
    Measured(Vec<f64>),
}

impl RowGrid {
    fn top(&self, row: usize) -> f64 {
        match self {
            Self::Uniform(pitch) => row as f64 * pitch,
            Self::Measured(tops) => tops[row.min(tops.len() - 1)],
        }
    }

    fn height(&self, row: usize) -> f64 {
        match self {
            Self::Uniform(pitch) => *pitch,
            Self::Measured(tops) => match (tops.get(row), tops.get(row + 1)) {
                (Some(top), Some(next)) => next - top,
                _ => 0.0,
            },
        }
    }

    /// The row at content offset `y` (its top at or above `y`).
    fn row_at(&self, y: f64) -> usize {
        match self {
            Self::Uniform(pitch) => (y / pitch).floor() as usize,
            Self::Measured(tops) => tops[..tops.len() - 1]
                .partition_point(|&top| top <= y)
                .saturating_sub(1),
        }
    }

    /// How many rows start above content offset `end`.
    fn rows_before(&self, end: f64) -> usize {
        match self {
            Self::Uniform(pitch) => (end / pitch).ceil() as usize,
            Self::Measured(tops) => tops[..tops.len() - 1].partition_point(|&top| top < end),
        }
    }
}

impl ListEntry {
    /// The grid for the source's `n` rows.
    fn grid(&self, n: usize) -> RowGrid {
        if !self.automatic {
            return RowGrid::Uniform(self.row_height.max(1.0));
        }
        let (sum, count) = self
            .heights
            .iter()
            .take(n)
            .flatten()
            .fold((0.0, 0_usize), |(sum, count), h| (sum + h, count + 1));
        let estimate = if count > 0 {
            sum / count as f64
        } else {
            self.row_height.max(1.0)
        };
        let mut tops = Vec::with_capacity(n + 1);
        let mut y = 0.0;
        tops.push(y);
        for row in 0..n {
            y += self.heights.get(row).copied().flatten().unwrap_or(estimate);
            tops.push(y);
        }
        RowGrid::Measured(tops)
    }
}

/// Repaint every cell's selected treatment from the entry's selection set.
fn list_paint_selection(entry: &ListEntry) {
    for (i, &cell) in entry.cells.iter().enumerate() {
        // Unrealized rows have no cell to paint; they pick the treatment up when they are built
        // (this runs at the end of every fill, so a row scrolled into a selection lands painted).
        if cell.is_null() {
            continue;
        }
        unsafe { ffi::day_xaml_cell_set_selected(cell, entry.selected.contains(&i) as c_int) };
    }
}

/// Enter activates the selected row without changing the selection.
extern "C" fn on_list_key(node: u64, key: c_int) -> c_int {
    ffi_guard::contain(0, || {
        if key != 4 {
            return 0;
        }
        let row = LIST_BY_NODE
            .with(|m| m.borrow().get(&node).copied())
            .and_then(|host| {
                LIST_STATE.with(|m| {
                    let state = m.borrow();
                    let list = state.get(&host)?;
                    list.selectable
                        .then(|| list.selected.last().copied())
                        .flatten()
                })
            });
        if let Some(row) = row {
            emit(NodeId(node), Event::ListActivated(row));
            return 1;
        }
        0
    })
}

/// A press on an emulated list cell (docs/list.md). Owns the selection semantics: a plain click
/// replaces the selection, ctrl toggles the row, shift extends from the anchor (multi-select
/// only) — then repaints and reports (`SelectionSet` in multi mode, `SelectionChanged` single).
extern "C" fn on_list_row_click(node: u64, row: c_int, mods: c_int) {
    ffi_guard::contain((), || {
        if mods == 4 {
            if row >= 0 {
                emit(NodeId(node), Event::ListActivated(row as usize));
            }
            return;
        }
        let row = row.max(0) as usize;
        let Some(host_key) = LIST_BY_NODE.with(|m| m.borrow().get(&node).copied()) else {
            return;
        };
        let emit_ev = LIST_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let st = m.get_mut(&host_key)?;
            if !st.selectable {
                return None;
            }
            let (ctrl, shift) = (mods & 1 != 0, mods & 2 != 0);
            if st.multi && ctrl {
                if !st.selected.remove(&row) {
                    st.selected.insert(row);
                }
                st.anchor = Some(row);
            } else if st.multi && shift {
                let a = st.anchor.unwrap_or(row);
                st.selected = (a.min(row)..=a.max(row)).collect();
            } else {
                st.selected = std::iter::once(row).collect();
                st.anchor = Some(row);
            }
            list_paint_selection(st);
            Some(if st.multi {
                Event::SelectionSet(st.selected.iter().map(|r| *r as i64).collect())
            } else {
                Event::SelectionChanged(row as i64)
            })
        });
        if let Some(ev) = emit_ev {
            emit(NodeId(node), ev);
        }
    });
}

/// The reorder guard's verdict for a hovered drop (docs/list.md), called synchronously from the
/// shim's DragOver handler: the accepted target index, or -1. The source is cloned out before
/// the app's guard runs — no thread-local borrow held.
extern "C" fn on_list_can_move(node: u64, from: c_int, to: c_int) -> c_int {
    // Runs the app's own guard closure — contained, with "reject the drop" as the default.
    ffi_guard::contain(-1, || {
        let Some(host_key) = LIST_BY_NODE.with(|m| m.borrow().get(&node).copied()) else {
            return -1;
        };
        let r = LIST_STATE.with(|m| {
            m.borrow().get(&host_key).and_then(|st| {
                let s = st.source.borrow().clone()?;
                Some(((s.len)(), s.reorder))
            })
        });
        let Some((len, Some(r))) = r else { return -1 };
        let (from, to) = (from.max(0) as usize, to.max(0) as usize);
        if from >= len {
            return -1;
        }
        let to = to.min(len - 1);
        ((r.can_move)(from, to) as c_int).min(len as c_int - 1)
    })
}

/// Commit a drop the shim accepted: rotate day's snapshot through the sync seam (deferring the
/// app callback) and re-bind the cells in the new order.
extern "C" fn on_list_move(node: u64, from: c_int, to: c_int) {
    ffi_guard::contain((), || {
        let Some(host_key) = LIST_BY_NODE.with(|m| m.borrow().get(&node).copied()) else {
            return;
        };
        let r = LIST_STATE.with(|m| {
            m.borrow()
                .get(&host_key)
                .and_then(|st| st.source.borrow().clone()?.reorder)
        });
        let Some(r) = r else { return };
        let (from, to) = (from.max(0) as usize, to.max(0) as usize);
        if from != to {
            (r.move_row)(from, to);
            schedule_list_populate(host_key);
        }
    });
}

/// The list scrolled: build whatever rows just came into view. Deferred and coalesced —
/// ViewChanged fires repeatedly through one flick (that is what makes it useful), and it fires
/// from inside XAML's own layout, where `bind_row`'s re-entry into `with_tree` cannot run.
extern "C" fn on_list_scrolled(node: u64) {
    ffi_guard::contain((), || {
        if let Some(host_key) = LIST_BY_NODE.with(|m| m.borrow().get(&node).copied()) {
            schedule_list_fill(host_key);
        }
    });
}

/// Fill the window on the next loop turn, at most once per turn however many times it is asked.
fn schedule_list_fill(host_key: usize) {
    let post = LIST_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let Some(st) = m.get_mut(&host_key) else {
            return false;
        };
        let first = !st.fill_pending;
        st.fill_pending = true;
        first
    });
    if !post {
        return;
    }
    let boxed: Box<dyn FnOnce() + Send> = Box::new(move || {
        LIST_STATE.with(|m| {
            if let Some(st) = m.borrow_mut().get_mut(&host_key) {
                st.fill_pending = false;
            }
        });
        list_fill_window(host_key);
    });
    let data = Box::into_raw(Box::new(boxed)) as *mut c_void;
    unsafe { ffi::day_xaml_post(run_posted, data) };
}

/// Populate/refresh a list's cells on the next loop turn — NOT inline: a reload runs inside a
/// `with_tree` borrow, and `bind_row` re-enters `with_tree`, which would panic.
fn schedule_list_populate(host_key: usize) {
    let boxed: Box<dyn FnOnce() + Send> = Box::new(move || list_populate(host_key));
    let data = Box::into_raw(Box::new(boxed)) as *mut c_void;
    unsafe { ffi::day_xaml_post(run_posted, data) };
}

/// Scroll the (emulated) list host to its bottom on the next turn — deferred so any pending
/// `list_populate` has sized the content Canvas first (posts run FIFO). No-op when empty.
fn schedule_list_scroll_end(host_key: usize) {
    let boxed: Box<dyn FnOnce() + Send> = Box::new(move || list_scroll_end(host_key));
    let data = Box::into_raw(Box::new(boxed)) as *mut c_void;
    unsafe { ffi::day_xaml_post(run_posted, data) };
}

/// Scroll the emulated list so row `row` sits at the top of the viewport (docs/list.md), on the
/// next loop turn — the same deferral as `schedule_list_scroll_end`.
fn schedule_list_scroll_row(host_key: usize, row: usize) {
    LIST_STATE.with(|m| {
        if let Some(st) = m.borrow_mut().get_mut(&host_key) {
            st.pending_scroll = Some(row);
        }
    });
    post_list_scroll_row(host_key, row);
}

/// Scroll a list host to `row` on the next loop turn.
fn post_list_scroll_row(host_key: usize, row: usize) {
    let boxed: Box<dyn FnOnce() + Send> = Box::new(move || {
        let target = LIST_STATE.with(|m| {
            let m = m.borrow();
            let st = m.get(&host_key)?;
            let n = st.source.borrow().as_ref().map_or(0, |s| (s.len)());
            let grid = st.grid(n);
            Some((
                st.host,
                grid.top(row).round() as c_int,
                grid.height(row).round() as c_int,
            ))
        });
        if let Some((host, y, rowh)) = target {
            unsafe { ffi::day_xaml_scroll_to(host, y, rowh, 1) };
            // A ScrollViewer XAML has not laid out yet (ViewportHeight 0) drops the ChangeView,
            // and no ViewChanged follows to build the rows it would reveal. Keep the jump
            // pending — the first fill that sees a measured viewport lands it — and fill now,
            // which builds the window around the jump rather than around the unmoved top.
            let (mut offset, mut vh) = (0.0, 0.0);
            unsafe { ffi::day_xaml_list_viewport(host, &mut offset, &mut vh) };
            if vh <= 0.0 {
                LIST_STATE.with(|m| {
                    if let Some(st) = m.borrow_mut().get_mut(&host_key) {
                        st.pending_scroll = Some(row);
                    }
                });
                schedule_list_fill(host_key);
            }
        }
    });
    let data = Box::into_raw(Box::new(boxed)) as *mut c_void;
    unsafe { ffi::day_xaml_post(run_posted, data) };
}

fn list_scroll_end(host_key: usize) {
    // The list host is a ScrollViewer; make the last row's band [y, y+rowh] visible (its content
    // Canvas is `rows * rowh` tall). Reuses the general scroll seam — no new XAML shim.
    let target = LIST_STATE.with(|m| {
        let m = m.borrow();
        let st = m.get(&host_key)?;
        let n = st.source.borrow().as_ref().map(|s| (s.len)()).unwrap_or(0);
        if n == 0 {
            return None;
        }
        let grid = st.grid(n);
        Some((st.host, grid.top(n - 1), grid.height(n - 1)))
    });
    if let Some((host, y, rowh)) = target {
        unsafe { ffi::day_xaml_scroll_to(host, y.round() as c_int, rowh.round() as c_int, 1) };
    }
}

/// A source change under the cells: everything realized is now showing the wrong row's data, so
/// mark it all dirty and refill the window. Reload, reorder, and a width that re-lays every row
/// all come through here — the callers that used to rebind all n rows.
fn list_populate(host_key: usize) {
    let stale = LIST_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let st = m.get_mut(&host_key)?;
        st.bound.iter_mut().for_each(|b| *b = false);
        // Every row is rebuilt, so every measured height is stale: rows re-measure as they are
        // bound, and the cells built before move to the tops the new heights give them.
        if st.automatic {
            st.heights.clear();
            st.relayout = true;
        }
        // A source that SHRANK leaves realized cells past its end. They stay in the pool (index
        // == row, so a source that grows back reuses each for the row it always held) and are
        // simply hidden — the same append-only pool, minus the eager building.
        let n = st.source.borrow().as_ref().map_or(0, |src| (src.len)());
        let recycle = st.source.borrow().as_ref().map(|src| src.recycle.clone())?;
        let stale: Vec<*mut c_void> = st
            .cells
            .iter()
            .skip(n)
            .copied()
            .filter(|c| !c.is_null())
            .collect();
        Some((recycle, stale))
    });
    if let Some((recycle, stale)) = stale {
        for cell in stale {
            unsafe { ffi::day_xaml_set_visible(cell, 0) };
            // A hidden row past the shrunk source keeps its pooled cell but must stop
            // answering dayscript lookups — clear its element ids (day-core's recycle).
            recycle(cell);
        }
    }
    list_fill_window(host_key);
}

/// Build the rows the viewport shows and that are not built already. Idempotent and cheap when
/// nothing moved, which is what lets every scroll event call it.
fn list_fill_window(host_key: usize) {
    // Phase 1 — under the LIST_STATE borrow: realize the window's cells + snapshot what we need.
    let Some((content, grid, source, work, n, width, measured)) = LIST_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let st = m.get_mut(&host_key)?;
        let source = st.source.borrow().clone()?;
        // Cells are sized from the width day's layout FRAMED the host at — NOT from the host's
        // `ActualWidth`. A populate is posted for the next loop turn, which routinely runs before
        // XAML's layout pass has published the size `set_frame` just assigned, so `ActualWidth`
        // reads a stale 0 and every cell gets built 1px wide.
        let width = if st.frame_width > 0 {
            st.frame_width
        } else {
            // `attach_list` populates before the host is ever framed: use whatever is realized.
            let (mut w, mut h) = (0.0_f64, 0.0_f64);
            unsafe { ffi::day_xaml_widget_size(st.host, &mut w, &mut h) };
            w.round() as c_int
        };
        if width <= 0 {
            // No usable width yet. Bail without recording `last_width`, so the host's next
            // `set_frame` still reads as a width change and schedules the real populate.
            return None;
        }
        let n = (source.len)();
        if st.automatic {
            st.heights.resize(n, None);
        }
        let grid = st.grid(n);
        // The rows on screen, plus the overscan. `ViewportHeight` is 0 until XAML has laid the
        // ScrollViewer out — the framed height stands in until then, exactly as the width does.
        let (mut offset, mut vh) = (0.0_f64, 0.0_f64);
        unsafe { ffi::day_xaml_list_viewport(st.host, &mut offset, &mut vh) };
        let measured = vh > 0.0;
        if !measured {
            vh = if st.frame_height > 0 {
                st.frame_height as f64
            } else {
                // Never framed and never laid out: build a screen's worth so the first paint is
                // not blank, and let the frame that follows widen the window.
                600.0
            };
            // A row jump the unmeasured ScrollViewer could not take yet: build the window it
            // will reveal (the minimal reveal from the top, as `day_xaml_scroll_to` scrolls), so
            // the row exists before the scroll lands.
            if let Some(row) = st.pending_scroll {
                let max = (grid.top(n) - vh).max(0.0);
                offset = (grid.top(row.min(n.saturating_sub(1)) + 1) - vh).clamp(0.0, max);
            }
        }
        if let Some(report) = &source.first_visible {
            report(grid.row_at(offset).min(n.saturating_sub(1)));
        }
        let first = grid.row_at(offset).saturating_sub(LIST_OVERSCAN);
        let last = (grid.rows_before(offset + vh) + LIST_OVERSCAN).min(n);
        // Slots exist for every row (a Vec of nulls, not of elements): the cell for row i lives
        // at i for good, which is what keeps the press handler's baked row honest.
        if st.cells.len() < n {
            st.cells.resize(n, std::ptr::null_mut());
            st.bound.resize(n, false);
        }
        let mut work: Vec<(usize, *mut c_void)> = Vec::new();
        for i in first..last {
            if st.cells[i].is_null() {
                let cell = unsafe { ffi::day_xaml_container_new() };
                unsafe { ffi::day_xaml_add_child(st.content, cell) };
                // Cell index == row for the cell's whole life (docs/list.md), so both the press
                // handler's row and the drag's are fixed here, at creation.
                if st.selectable {
                    unsafe {
                        ffi::day_xaml_list_cell_click(cell, st.node, i as c_int, on_list_row_click)
                    };
                }
                if st.reorderable {
                    unsafe { ffi::day_xaml_cell_drag(cell, st.node, i as c_int) };
                }
                st.cells[i] = cell;
            }
            if !st.bound[i] {
                st.bound[i] = true;
                work.push((i, st.cells[i]));
            }
        }
        st.last_width = width;
        Some((st.content, grid, source, work, n, width, measured))
    }) else {
        return;
    };
    // Phase 2 — no borrow held: bind_row re-enters with_tree (lays the row out, set_frames the
    // list host — taking LIST_STATE again).
    let mut measured_rows: Vec<(usize, f64)> = Vec::new();
    for (i, cell) in work {
        unsafe {
            ffi::day_xaml_set_geometry(
                cell,
                0,
                grid.top(i) as c_int,
                width,
                grid.height(i) as c_int,
            );
            ffi::day_xaml_set_visible(cell, 1);
        }
        (source.bind_row)(i, cell);
        if matches!(grid, RowGrid::Measured(_)) {
            // Content-sized: the height Day just laid the row at (0 = the bind skipped).
            let height = (source.cell_height)(cell);
            if height > 0.0 {
                measured_rows.push((i, height));
            }
        }
    }
    // Record what the binds measured; a new height moves the rows below it, so every built cell
    // goes to its new top, and the window refills: rows that came out shorter than the estimate
    // reveal rows that are not built yet.
    let mut grid = grid;
    let moved = LIST_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let st = m.get_mut(&host_key)?;
        if !st.automatic {
            return None;
        }
        let mut changed = false;
        for (row, height) in measured_rows {
            if let Some(slot) = st.heights.get_mut(row)
                && slot.is_none_or(|h| (h - height).abs() > 0.25)
            {
                *slot = Some(height);
                changed = true;
            }
        }
        if !changed && !st.relayout {
            return None;
        }
        st.relayout = false;
        grid = st.grid(n);
        let cells: Vec<(usize, *mut c_void)> = st
            .cells
            .iter()
            .take(n)
            .enumerate()
            .filter(|(_, c)| !c.is_null())
            .map(|(i, c)| (i, *c))
            .collect();
        Some((cells, changed))
    });
    if let Some((cells, changed)) = moved {
        for (i, cell) in cells {
            unsafe {
                ffi::day_xaml_set_geometry(
                    cell,
                    0,
                    grid.top(i) as c_int,
                    width,
                    grid.height(i) as c_int,
                )
            };
        }
        if changed {
            schedule_list_fill(host_key);
        }
    }
    // The extent is the whole source, built or not: the scrollbar is how the user reaches rows
    // that do not exist yet, so it cannot be sized to what happens to be realized.
    unsafe { ffi::day_xaml_list_set_content_size(content, width, grid.top(n) as c_int) };
    // A row jump that arrived before this extent existed lands now, on the next turn so the
    // ScrollViewer has measured the new extent; its ViewChanged builds the rows it reveals. An
    // unmeasured viewport keeps the jump for a later fill: the scroll would only be dropped again.
    let pending = LIST_STATE.with(|m| {
        m.borrow_mut()
            .get_mut(&host_key)
            .filter(|_| measured)
            .and_then(|st| st.pending_scroll.take())
    });
    if let Some(row) = pending {
        post_list_scroll_row(host_key, row);
    }
    // Cells just added to the pool start unpainted, and a reload can move which rows are selected
    // under a selection that hasn't changed — so repaint from the entry's set on every populate.
    LIST_STATE.with(|m| {
        if let Some(st) = m.borrow().get(&host_key) {
            list_paint_selection(st);
        }
    });
}

pub(super) fn new(id: NodeId, p: &ListProps) -> *mut c_void {
    unsafe {
        let mut content: *mut c_void = std::ptr::null_mut();
        let host = ffi::day_xaml_list_new(&mut content);
        let row_height = match p.row_height {
            RowHeight::Uniform(h) => h,
            RowHeight::Automatic => 44.0,
        };
        if p.reorderable && !content.is_null() {
            // WinRT drag reorder (docs/list.md): drops on the content Canvas route
            // through the seam callbacks above.
            ffi::day_xaml_list_enable_reorder(
                content,
                id.0,
                row_height as c_int,
                on_list_can_move,
                on_list_move,
            );
        }
        LIST_STATE.with(|m| {
            m.borrow_mut().insert(
                host as usize,
                ListEntry {
                    host,
                    content,
                    row_height,
                    automatic: matches!(p.row_height, RowHeight::Automatic),
                    heights: Vec::new(),
                    relayout: false,
                    source: Rc::new(RefCell::new(None)),
                    cells: Vec::new(),
                    bound: Vec::new(),
                    last_width: -1,
                    frame_width: -1,
                    frame_height: -1,
                    fill_pending: false,
                    pending_scroll: None,
                    reorderable: p.reorderable,
                    node: id.0,
                    selectable: p.selectable,
                    multi: p.multi_select,
                    selected: BTreeSet::new(),
                    anchor: None,
                },
            )
        });
        LIST_BY_NODE.with(|m| m.borrow_mut().insert(id.0, host as usize));
        // Rows are built as they scroll in, so the list has to hear about scrolling.
        ffi::day_xaml_list_on_scroll(host, id.0, on_list_scrolled);
        if p.selectable && p.activatable {
            ffi::day_xaml_list_keynav(host, id.0, on_list_key);
        }
        host
    }
}

pub(super) fn patch(h: &WinHandle, patch: &ListPatch) {
    unsafe {
        match Some(patch) {
            Some(ListPatch::Reload) | Some(ListPatch::Splice(_)) => {
                schedule_list_populate(h.0 as usize)
            }
            Some(ListPatch::ScrollToEnd) => schedule_list_scroll_end(h.0 as usize),
            Some(ListPatch::ScrollToRow(row)) => schedule_list_scroll_row(h.0 as usize, *row),
            Some(ListPatch::Selected(rows)) => {
                // Programmatic selection sync (empty = clear): repaint, no re-emit.
                LIST_STATE.with(|m| {
                    if let Some(st) = m.borrow_mut().get_mut(&(h.0 as usize)) {
                        st.selected = rows.iter().copied().collect();
                        st.anchor = rows.last().copied();
                        list_paint_selection(st);
                    }
                });
            }
            Some(ListPatch::RowSizeInvalidated { cell, height }) => {
                // A content-sized row re-laid at a new height: record it against the
                // row its cell holds (cell index == row), and let the posted fill move
                // the cells, out of the day-core layout pass this arrives from.
                let resized = LIST_STATE.with(|m| {
                    let mut m = m.borrow_mut();
                    let Some(st) = m.get_mut(&(h.0 as usize)) else {
                        return false;
                    };
                    let Some(row) = st
                        .cells
                        .iter()
                        .position(|&c| c as usize == *cell)
                        .filter(|_| st.automatic)
                    else {
                        return false;
                    };
                    if let Some(slot) = st.heights.get_mut(row) {
                        *slot = Some(*height);
                        st.relayout = true;
                    }
                    st.relayout
                });
                if resized {
                    schedule_list_fill(h.0 as usize);
                }
            }
            // No editing mode (Cap::ListEditing is Unsupported).
            Some(ListPatch::Editing(_)) | None => {}
        }
    }
}

pub(super) fn release(key: usize) {
    // day-core never releases the adopted cell handles (docs/list.md: they are the host's own
    // cells, handed out through `adopt`), so the list host owns cell + content cleanup.
    if let Some(st) = LIST_STATE.with(|m| m.borrow_mut().remove(&key)) {
        // …and the node→host mapping the reorder callbacks resolve through, or a later list
        // that lands on this freed address answers drags aimed at the dead one.
        LIST_BY_NODE.with(|m| m.borrow_mut().remove(&st.node));
        for cell in st.cells {
            unsafe { ffi::day_xaml_delete(cell) };
        }
        unsafe { ffi::day_xaml_delete(st.content) };
    }
}

pub(super) fn frame(h: &WinHandle, frame: Rect) {
    // List host framed: (re)fill its cells — but only when the width actually changed, so the
    // set_frames a populate itself makes (on row content) don't schedule another forever.
    let framed = frame.size.width.round() as c_int;
    let width_changed = LIST_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let Some(st) = m.get_mut(&(h.0 as usize)) else {
            return false;
        };
        // Remember the assigned width for the populate this schedules: the host's own
        // `ActualWidth` is still the PREVIOUS pass's value at that point (see ListEntry).
        st.frame_width = framed;
        st.last_width != framed
    });
    // A taller host shows more rows, and the ones it grew into are not built yet — so a
    // height change refills the window even though every built row is still valid. Width is
    // the one that invalidates content: each row is laid out to it.
    let framed_h = frame.size.height.round() as c_int;
    let height_changed = LIST_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let Some(st) = m.get_mut(&(h.0 as usize)) else {
            return false;
        };
        let changed = st.frame_height != framed_h;
        st.frame_height = framed_h;
        changed
    });
    if width_changed {
        schedule_list_populate(h.0 as usize);
    } else if height_changed {
        schedule_list_fill(h.0 as usize);
    }
}

pub(super) fn attach(host: &WinHandle, source: day_spec::ListSource) {
    LIST_STATE.with(|m| {
        if let Some(st) = m.borrow().get(&(host.0 as usize)) {
            *st.source.borrow_mut() = Some(source);
        }
    });
    // Deferred (see schedule_list_populate): populating re-enters with_tree via bind_row.
    schedule_list_populate(host.0 as usize);
}
