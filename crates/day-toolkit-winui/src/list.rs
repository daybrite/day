// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! WinUI ItemsRepeater owns realization, layout and the physical cell pool.
//! Native layout only schedules work: ListSource pulls run outside day-core borrows.
//! A sectioned list (docs/list.md "Sections") hands the shim its header rows, which it keeps
//! out of selection and keyboard travel, and binds one extra cell: the shim's pinned copy of
//! the current section's header.
use super::*;
use day_spec::{ListSource, SwipeEdge};

#[derive(Clone, Copy)]
struct Binding {
    row: usize,
    generation: u64,
    token: u64,
    width: f64,
}

/// What the pinned header cell holds.
#[derive(Clone, Copy)]
struct Pinned {
    cell: usize,
    row: usize,
    token: u64,
    width: f64,
}

struct Entry {
    node: NodeId,
    props: ListProps,
    source: Option<ListSource>,
    active: HashMap<usize, Binding>,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    posted: bool,
    reload: bool,
    frame: Size,
    pinned: Option<Pinned>,
}

thread_local! {
    static LISTS: RefCell<HashMap<usize, Entry>> = RefCell::new(HashMap::new());
}

fn source(host: usize) -> Option<ListSource> {
    LISTS.with(|m| m.borrow().get(&host)?.source.clone())
}

fn is_header(source: &ListSource, row: usize) -> bool {
    source.is_header.as_ref().is_some_and(|f| f(row))
}

extern "C" fn changed(host: *mut c_void) {
    ffi_guard::contain((), || schedule(host as usize, false));
}

pub(super) fn new(node: NodeId, props: &ListProps) -> *mut c_void {
    let pitch = match props.row_height {
        RowHeight::Uniform(h) => h.max(1.0),
        RowHeight::Automatic => 44.0,
    };
    let host = unsafe {
        ffi::day_winui_list_new(
            pitch,
            props.selectable.into(),
            props.activatable.into(),
            props.reorderable.into(),
            changed,
            selected,
            offer,
            action,
            can_move,
            moved,
        )
    };
    LISTS.with(|m| {
        m.borrow_mut().insert(
            host as usize,
            Entry {
                node,
                props: props.clone(),
                source: None,
                active: HashMap::new(),
                selected: BTreeSet::new(),
                anchor: None,
                posted: false,
                reload: false,
                frame: Size::default(),
                pinned: None,
            },
        );
    });
    host
}

pub(super) fn attach(host: &WinHandle, source: ListSource) {
    LISTS.with(|m| {
        if let Some(e) = m.borrow_mut().get_mut(&(host.0 as usize)) {
            e.source = Some(source);
        }
    });
    schedule(host.0 as usize, true);
}

pub(super) fn release(host: usize) {
    if let Some(e) = LISTS.with(|m| m.borrow_mut().remove(&host)) {
        // day-core disposes adopted row scopes; the native factory owns their handles. The
        // pinned header's cell is the list's own, so say goodbye to it here.
        if let (Some(source), Some(pinned)) = (&e.source, e.pinned) {
            (source.release)(pinned.cell as *mut c_void);
        }
        unsafe { ffi::day_winui_list_forget(host as *mut c_void) };
    }
}

pub(super) fn frame(host: &WinHandle, frame: Rect) {
    let changed = LISTS.with(|m| {
        let mut m = m.borrow_mut();
        let Some(e) = m.get_mut(&(host.0 as usize)) else {
            return false;
        };
        let changed = e.frame != frame.size;
        e.frame = frame.size;
        changed
    });
    if changed {
        unsafe { ffi::day_winui_list_frame(host.0, frame.size.width, frame.size.height) };
        schedule(host.0 as usize, false);
    }
}

pub(super) fn patch(host: &WinHandle, patch: &ListPatch) {
    match patch {
        ListPatch::Reload | ListPatch::Splice(_) => schedule(host.0 as usize, true),
        ListPatch::ScrollToEnd => scroll(host.0 as usize, None),
        ListPatch::ScrollToRow(row) => scroll(host.0 as usize, Some(*row)),
        ListPatch::Selected(rows) => {
            LISTS.with(|m| {
                if let Some(e) = m.borrow_mut().get_mut(&(host.0 as usize)) {
                    e.selected = rows.iter().copied().collect();
                    e.anchor = rows.last().copied();
                }
            });
            paint_selection(host.0 as usize);
        }
        ListPatch::RowSizeInvalidated { .. } => schedule(host.0 as usize, false),
        ListPatch::Editing(_) => {}
    }
}

fn scroll(host: usize, row: Option<usize>) {
    let node = LISTS.with(|m| m.borrow().get(&host).map(|e| e.node));
    Xaml::post(Box::new(move || {
        if LISTS.with(|m| m.borrow().get(&host).map(|e| e.node)) != node || node.is_none() {
            return;
        }
        unsafe {
            ffi::day_winui_list_scroll(
                host as *mut c_void,
                row.map_or(-1, |r| r.min(i32::MAX as usize) as i32),
            )
        };
        schedule(host, false);
    }));
}

fn schedule(host: usize, reload: bool) {
    let node = LISTS.with(|m| {
        let mut m = m.borrow_mut();
        let e = m.get_mut(&host)?;
        e.reload |= reload;
        if e.posted {
            return None;
        }
        e.posted = true;
        Some(e.node)
    });
    if let Some(node) = node {
        Xaml::post(Box::new(move || fill(host, node)));
    }
}

fn fill(host: usize, node: NodeId) {
    let work = LISTS.with(|m| {
        let mut m = m.borrow_mut();
        let e = m.get_mut(&host)?;
        if e.node != node {
            return None;
        }
        e.posted = false;
        Some((
            e.source.clone()?,
            std::mem::take(&mut e.reload),
            e.active.clone(),
            e.props.row_height,
        ))
    });
    let Some((source, reload, previous, height)) = work else {
        return;
    };
    let raw = host as *mut c_void;
    if reload {
        let len = (source.len)().min(i32::MAX as usize);
        if let Some(is_header) = &source.is_header {
            // Before the reload, so the factory pools the new rows by kind.
            let headers: Vec<i32> = (0..len)
                .filter(|r| is_header(*r))
                .map(|r| r as i32)
                .collect();
            unsafe { ffi::day_winui_list_headers(raw, headers.as_ptr(), headers.len() as i32) };
        }
        unsafe { ffi::day_winui_list_reload(raw, len as i32) };
    }
    unsafe { ffi::day_winui_list_layout(raw) };
    let count = unsafe { ffi::day_winui_list_cells(raw, std::ptr::null_mut(), 0) }.max(0) as usize;
    let mut cells = vec![
        ffi::DayWinuiListCell {
            cell: std::ptr::null_mut(),
            row: -1,
            generation: 0,
            width: 0.0,
            visible: 0
        };
        count
    ];
    unsafe { ffi::day_winui_list_cells(raw, cells.as_mut_ptr(), count as i32) };
    let live: HashMap<usize, _> = cells
        .iter()
        .filter(|c| c.row >= 0)
        .map(|c| (c.cell as usize, c))
        .collect();
    for (&cell, old) in &previous {
        if live
            .get(&cell)
            .is_none_or(|c| c.generation != old.generation || c.row as usize != old.row)
        {
            (source.recycle)(cell as *mut c_void);
        }
    }
    let mut active = HashMap::new();
    let mut first = None;
    let len = (source.len)();
    for c in cells.into_iter().filter(|c| c.row >= 0 && c.width > 0.0) {
        let row = c.row as usize;
        if row >= len {
            continue;
        }
        let token = (source.token_at)(row);
        let old = previous.get(&(c.cell as usize));
        let rebind = reload
            || old.is_none_or(|b| b.row != row || b.generation != c.generation || b.token != token);
        if rebind {
            (source.bind_row)(row, c.cell);
        }
        if rebind || old.is_none_or(|b| (b.width - c.width).abs() > 0.25) {
            (source.layout_cell)(c.cell, c.width);
        }
        // A header sizes to its content even among uniform rows.
        let measured = match height {
            RowHeight::Uniform(h) if !is_header(&source, row) => h.max(1.0),
            _ => (source.cell_height)(c.cell).max(1.0),
        };
        unsafe { ffi::day_winui_list_cell_size(raw, c.cell, c.generation, c.width, measured) };
        active.insert(
            c.cell as usize,
            Binding {
                row,
                generation: c.generation,
                token,
                width: c.width,
            },
        );
        if c.visible != 0 {
            first = Some(first.map_or(row, |f: usize| f.min(row)))
        }
        if rebind {
            offer(raw, c.cell, c.row);
        }
    }
    LISTS.with(|m| {
        if let Some(e) = m.borrow_mut().get_mut(&host).filter(|e| e.node == node) {
            e.active = active;
        }
    });
    if source.is_header.is_some() {
        pin(host, node, &source, reload);
    }
    paint_selection(host);
    if let (Some(report), Some(first)) = (&source.first_visible, first) {
        report(first);
    }
}

/// Bind the shim's pinned header cell to the header it asks for, and keep its height current.
fn pin(host: usize, node: NodeId, source: &ListSource, reload: bool) {
    let raw = host as *mut c_void;
    let (mut cell, mut width) = (std::ptr::null_mut(), 0.0);
    let want = unsafe { ffi::day_winui_list_pinned(raw, &mut cell, &mut width) };
    if want < 0 || cell.is_null() || width <= 0.0 || want as usize >= (source.len)() {
        return;
    }
    let row = want as usize;
    if !is_header(source, row) {
        return;
    }
    let token = (source.token_at)(row);
    let old = LISTS.with(|m| m.borrow().get(&host).and_then(|e| e.pinned));
    let rebind = reload || old.is_none_or(|p| p.row != row || p.token != token);
    if rebind {
        // The copy carries no element ids: lookups of a header's id find the real row.
        (source.bind_pinned)(row, cell);
    }
    if rebind || old.is_none_or(|p| (p.width - width).abs() > 0.25) {
        (source.layout_cell)(cell, width);
    }
    let height = (source.cell_height)(cell).max(1.0);
    unsafe { ffi::day_winui_list_pinned_bound(raw, want, width, height) };
    LISTS.with(|m| {
        if let Some(e) = m.borrow_mut().get_mut(&host).filter(|e| e.node == node) {
            e.pinned = Some(Pinned {
                cell: cell as usize,
                row,
                token,
                width,
            });
        }
    });
}

fn paint_selection(host: usize) {
    let rows = LISTS.with(|m| {
        m.borrow()
            .get(&host)
            .map(|e| e.selected.iter().map(|r| *r as i32).collect::<Vec<_>>())
    });
    if let Some(rows) = rows {
        unsafe {
            ffi::day_winui_list_select(host as *mut c_void, rows.as_ptr(), rows.len() as i32)
        };
    }
}

extern "C" fn selected(host: *mut c_void, row: i32, modifiers: i32) {
    ffi_guard::contain((), || {
        let Some(source) = source(host as usize) else {
            return;
        };
        let len = (source.len)();
        // Headers are never selected or activated (docs/list.md "Sections").
        if row >= 0 && is_header(&source, row as usize) {
            return;
        }
        let event = LISTS.with(|m| {
            let mut m = m.borrow_mut();
            let e = m.get_mut(&(host as usize))?;
            if modifiers == 4 {
                return (e.props.activatable && row >= 0 && (row as usize) < len)
                    .then_some((e.node, Event::ListActivated(row as usize)));
            }
            if !e.props.selectable {
                return None;
            }
            if modifiers == 16 && e.props.multi_select {
                e.selected = (0..len).filter(|r| !is_header(&source, *r)).collect();
            } else if row < 0 || row as usize >= len {
                return None;
            } else {
                let row = row as usize;
                if modifiers == 64 {
                    e.selected.remove(&row);
                } else if modifiers == 32 {
                    if !e.props.multi_select {
                        e.selected.clear();
                    }
                    e.selected.insert(row);
                    e.anchor = Some(row);
                } else if e.props.multi_select && modifiers & 2 != 0 {
                    let anchor = e.anchor.unwrap_or(row).min(len.saturating_sub(1));
                    e.selected = (anchor.min(row)..=anchor.max(row))
                        .filter(|r| !is_header(&source, *r))
                        .collect();
                } else if e.props.multi_select && modifiers & 1 != 0 {
                    if !e.selected.remove(&row) {
                        e.selected.insert(row);
                    }
                    e.anchor = Some(row);
                } else {
                    e.selected = BTreeSet::from([row]);
                    e.anchor = Some(row);
                }
            }
            Some((
                e.node,
                if e.props.multi_select {
                    Event::SelectionSet(e.selected.iter().map(|r| *r as i64).collect())
                } else {
                    Event::SelectionChanged(e.selected.first().map_or(-1, |r| *r as i64))
                },
            ))
        });
        paint_selection(host as usize);
        if let Some((node, event)) = event {
            emit(node, event);
        }
    });
}

extern "C" fn offer(host: *mut c_void, cell: *mut c_void, row: i32) {
    ffi_guard::contain((), || {
        let Some(source) = source(host as usize) else {
            return;
        };
        if row < 0 || row as usize >= (source.len)() || is_header(&source, row as usize) {
            return;
        }
        let row = row as usize;
        let token = (source.token_at)(row);
        unsafe { ffi::day_winui_list_actions_begin(host, cell) };
        if let Some(swipe) = &source.swipe {
            for (edge, code) in [(SwipeEdge::Leading, 0), (SwipeEdge::Trailing, 1)] {
                for (index, action) in (swipe.actions_at)(row, edge).into_iter().enumerate() {
                    let color = action.tint.map(|c| [c.r, c.g, c.b, c.a]);
                    unsafe {
                        ffi::day_winui_list_action_add(
                            host,
                            cell,
                            token,
                            code,
                            index as i32,
                            cstr(&action.label).as_ptr(),
                            cstr(action.symbol.map_or("", super::toolbar::glyph)).as_ptr(),
                            action.destructive.into(),
                            color.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                        );
                    }
                }
            }
        }
        if let Some(delete) = &source.delete
            && (delete.can_delete)(row)
        {
            let label = LISTS
                .with(|m| {
                    m.borrow()
                        .get(&(host as usize))
                        .map(|e| e.props.delete_label.clone())
                })
                .unwrap_or_default();
            unsafe {
                ffi::day_winui_list_action_add(
                    host,
                    cell,
                    token,
                    1,
                    -1,
                    cstr(&label).as_ptr(),
                    cstr("E74D").as_ptr(),
                    1,
                    std::ptr::null(),
                )
            };
        }
        unsafe { ffi::day_winui_list_actions_end(host, cell) };
    });
}

extern "C" fn action(host: *mut c_void, row: i32, token: u64, edge: i32, index: i32) {
    ffi_guard::contain((), || {
        let Some(source) = source(host as usize) else {
            return;
        };
        if row < 0 || row as usize >= (source.len)() || (source.token_at)(row as usize) != token {
            return;
        }
        if index < 0 {
            if let Some(delete) = &source.delete
                && (delete.can_delete)(row as usize)
            {
                (delete.delete_row)(row as usize);
            }
        } else if let Some(swipe) = &source.swipe {
            (swipe.perform)(
                row as usize,
                if edge == 0 {
                    SwipeEdge::Leading
                } else {
                    SwipeEdge::Trailing
                },
                index as usize,
            );
        }
        schedule(host as usize, true);
    });
}

extern "C" fn can_move(host: *mut c_void, from: i32, to: i32) -> i32 {
    ffi_guard::contain(-1, || {
        let Some(source) = source(host as usize) else {
            return -1;
        };
        let n = (source.len)();
        if from < 0 || to < 0 || from as usize >= n || to as usize >= n {
            return -1;
        }
        source.reorder.as_ref().map_or(-1, |r| {
            (r.can_move)(from as usize, to as usize).min(n as i64 - 1) as i32
        })
    })
}

extern "C" fn moved(host: *mut c_void, from: i32, to: i32) {
    ffi_guard::contain((), || {
        let Some(source) = source(host as usize) else {
            return;
        };
        if let Some(reorder) = &source.reorder
            && from >= 0
            && to >= 0
            && (from as usize) < (source.len)()
            && (to as usize) < (source.len)()
        {
            (reorder.move_row)(from as usize, to as usize);
            schedule(host as usize, true);
        }
    });
}
