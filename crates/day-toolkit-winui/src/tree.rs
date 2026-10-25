// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Native WinUI/XAML TreeView disclosure with Day-owned Canvas row content.
use super::*;
use day_spec::TreeSource;
use ffi::{
    day_xaml_tree_add as tree_add, day_xaml_tree_begin as tree_begin,
    day_xaml_tree_cell as tree_cell, day_xaml_tree_end as tree_end,
    day_xaml_tree_expand as tree_expand, day_xaml_tree_frame as tree_frame,
    day_xaml_tree_new as tree_new, day_xaml_tree_reveal as tree_reveal,
    day_xaml_tree_select as tree_select,
};
use std::collections::HashSet;

type TreePlatform = Xaml;
fn emit_deferred(id: NodeId, event: Event) {
    Xaml::post(Box::new(move || super::emit(id, event)));
}

struct Entry {
    node: NodeId,
    source: Option<TreeSource>,
    cells: HashMap<u64, usize>,
    active: HashSet<u64>,
    selected: Vec<u64>,
    pending: bool,
}
thread_local! { static TREES: RefCell<HashMap<usize, Entry>> = RefCell::new(HashMap::new()); }

extern "C" fn expanded(node: u64, token: u64, on: c_int) {
    ffi_guard::contain((), || {
        emit_deferred(
            NodeId(node),
            Event::TreeExpanded {
                token,
                expanded: on != 0,
            },
        )
    });
}
extern "C" fn selected(node: u64, tokens: *const u64, len: c_int) {
    ffi_guard::contain((), || {
        let tokens = if len > 0 && !tokens.is_null() {
            unsafe { std::slice::from_raw_parts(tokens, len as usize) }.to_vec()
        } else {
            Vec::new()
        };
        TREES.with(|m| {
            for e in m
                .borrow_mut()
                .values_mut()
                .filter(|e| e.node == NodeId(node))
            {
                e.selected = tokens.clone();
            }
        });
        emit_deferred(NodeId(node), Event::TreeSelection(tokens));
    });
}
extern "C" fn viewport(host: *mut c_void) {
    ffi_guard::contain((), || schedule(host as usize, false));
}

pub(super) fn new(id: NodeId, p: &TreeProps) -> *mut c_void {
    let height = match p.row_height {
        RowHeight::Uniform(h) => h,
        RowHeight::Automatic => 36.0,
    };
    let host = unsafe {
        tree_new(
            id.0,
            height,
            p.indent.unwrap_or(16.0),
            i32::from(p.selectable),
            i32::from(p.multi_select),
            expanded,
            selected,
            viewport,
        )
    };
    TREES.with(|m| {
        m.borrow_mut().insert(
            host as usize,
            Entry {
                node: id,
                source: None,
                cells: HashMap::new(),
                active: HashSet::new(),
                selected: vec![],
                pending: false,
            },
        )
    });
    host
}
pub(super) fn release(host: usize) {
    TREES.with(|m| m.borrow_mut().remove(&host));
    unsafe {
        ffi::day_xaml_tree_forget(host as *mut c_void);
    }
}
pub(super) fn attach(host: usize, source: TreeSource) {
    TREES.with(|m| {
        if let Some(e) = m.borrow_mut().get_mut(&host) {
            e.source = Some(source);
        }
    });
    schedule(host, true);
}
pub(super) fn patch(host: usize, patch: &TreePatch) {
    match patch {
        TreePatch::Reload => schedule(host, true),
        TreePatch::Expand(token, on) => {
            unsafe {
                tree_expand(host as *mut c_void, *token, i32::from(*on));
            }
            schedule(host, false);
        }
        TreePatch::Selected(tokens) => {
            TREES.with(|m| {
                if let Some(e) = m.borrow_mut().get_mut(&host) {
                    e.selected = tokens.clone();
                }
            });
            unsafe {
                tree_select(host as *mut c_void, tokens.as_ptr(), tokens.len() as c_int);
            }
        }
        TreePatch::Reveal(token) => {
            unsafe {
                tree_reveal(host as *mut c_void, *token);
            }
            schedule(host, false);
        }
    }
}
fn schedule(host: usize, reload: bool) {
    // Reloads are posted separately: a scroll fill must never swallow a structural change.
    if !reload {
        let post = TREES.with(|m| {
            let mut m = m.borrow_mut();
            let Some(e) = m.get_mut(&host) else {
                return false;
            };
            if e.pending {
                false
            } else {
                e.pending = true;
                true
            }
        });
        if !post {
            return;
        }
    }
    let Some(node) = TREES.with(|m| m.borrow().get(&host).map(|e| e.node)) else {
        return;
    };
    TreePlatform::post(Box::new(move || {
        let source = TREES.with(|m| {
            let mut m = m.borrow_mut();
            let e = m.get_mut(&host)?;
            if e.node != node {
                return None;
            }
            e.pending = false;
            e.source.clone()
        });
        let Some(source) = source else {
            return;
        };
        if reload {
            unsafe {
                tree_begin(host as *mut c_void);
            }
            fn append(
                host: usize,
                source: &TreeSource,
                parent: Option<u64>,
                seen: &mut HashSet<u64>,
            ) {
                for i in 0..(source.children_len)(parent) {
                    let t = (source.child_token)(parent, i);
                    if !seen.insert(t) {
                        continue;
                    }
                    unsafe {
                        tree_add(
                            host as *mut c_void,
                            t,
                            parent.unwrap_or(0),
                            i32::from(parent.is_some()),
                            cstr(&(source.type_select_text)(t)).as_ptr(),
                            i32::from((source.expandable)(t)),
                            i32::from((source.section_header)(t)),
                            i32::from((source.expanded)(t)),
                        );
                    }
                    append(host, source, Some(t), seen);
                }
            }
            let mut active = HashSet::new();
            append(host, &source, None, &mut active);
            let (stale, selected) = TREES.with(|m| {
                let mut m = m.borrow_mut();
                let Some(e) = m.get_mut(&host) else {
                    return (vec![], vec![]);
                };
                let stale = e
                    .cells
                    .iter()
                    .filter(|(t, _)| !active.contains(t))
                    .map(|(_, c)| *c)
                    .collect::<Vec<_>>();
                e.active = active;
                (stale, e.selected.clone())
            });
            for cell in stale {
                (source.recycle)(cell as *mut c_void);
            }
            unsafe {
                tree_end(host as *mut c_void);
                tree_select(
                    host as *mut c_void,
                    selected.as_ptr(),
                    selected.len() as c_int,
                );
            }
        }
        let active = TREES.with(|m| {
            m.borrow()
                .get(&host)
                .map(|e| e.active.clone())
                .unwrap_or_default()
        });
        for token in active {
            let mut width = 0.0;
            let visible = unsafe { tree_frame(host as *mut c_void, token, &mut width) } != 0;
            let old = TREES.with(|m| {
                m.borrow()
                    .get(&host)
                    .and_then(|e| e.cells.get(&token).copied())
            });
            if !visible {
                if let Some(cell) = old {
                    (source.recycle)(cell as *mut c_void);
                }
                continue;
            }
            let cell = old.unwrap_or_else(|| {
                let cell = unsafe { tree_cell(host as *mut c_void, token) } as usize;
                TREES.with(|m| {
                    if let Some(e) = m.borrow_mut().get_mut(&host) {
                        e.cells.insert(token, cell);
                    }
                });
                cell
            });
            if cell != 0 {
                (source.bind_row)(token, cell as *mut c_void);
                (source.layout_cell)(cell as *mut c_void, width);
            }
        }
    }));
}
