// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(windows))]
fn main() {}

// Synthetic UI fixtures, run on the process main thread with a real WinUI message loop.
// A selected hierarchical reload used to corrupt WinUI's native reference-tracker heap:
// RootNodes was cleared before SelectedNodes released its subscriptions to the old nodes.
#[cfg(windows)]
fn main() {
    use day_toolkit_xaml_sys::*;
    use std::ffi::c_void;

    const RELOADS: usize = 256;
    struct State {
        tree: *mut c_void,
        reloads: usize,
    }
    extern "C" fn expanded(_: u64, _: u64, _: i32) {}
    extern "C" fn selected(_: u64, _: *const u64, _: i32) {}
    extern "C" fn viewport(_: *mut c_void) {}

    extern "C" fn reload(raw: *mut c_void) {
        // State belongs to main and outlives the message loop. All callbacks run on
        // that same UI thread; the last callback quits without scheduling another.
        unsafe {
            let state = &mut *raw.cast::<State>();
            day_xaml_tree_begin(state.tree);
            for parent in 1..=3 {
                day_xaml_tree_add(state.tree, parent, 0, 0, c"Fixture group".as_ptr(), 1, 1, 1);
                for child in 1..=8 {
                    day_xaml_tree_add(
                        state.tree,
                        parent * 100 + child,
                        parent,
                        1,
                        c"Fixture row".as_ptr(),
                        0,
                        0,
                        0,
                    );
                }
            }
            day_xaml_tree_end(state.tree);
            let token = 101 + state.reloads as u64 % 8;
            day_xaml_tree_select(state.tree, &token, 1);
            state.reloads += 1;
            if state.reloads == RELOADS {
                day_xaml_quit();
            } else {
                // Let native selection/container notifications run between reloads.
                day_xaml_post_delayed(5, reload, raw);
            }
        }
    }

    unsafe {
        let chrome = DayXamlChrome {
            shadow: 1,
            resizable: 1,
            ..Default::default()
        };
        let window = day_xaml_window_new(
            c"Tree reload regression fixture".as_ptr(),
            640,
            560,
            300,
            300,
            &chrome,
        );
        assert!(!window.is_null(), "WinUI window initialization failed");
        let root = day_xaml_window_root(window);
        let tree = day_xaml_tree_new(1, 36.0, 16.0, 1, 0, expanded, selected, viewport);
        assert!(!tree.is_null(), "WinUI TreeView initialization failed");
        day_xaml_add_child(root, tree);
        day_xaml_set_geometry(tree, 0, 0, 600, 500);
        day_xaml_window_show(window);
        let mut state = Box::new(State { tree, reloads: 0 });
        day_xaml_post_delayed(100, reload, (&mut *state as *mut State).cast());
        day_xaml_run(window);
        assert_eq!(
            state.reloads, RELOADS,
            "message loop ended before all reloads"
        );
        println!("PASS {RELOADS} selected native tree reloads");
    }
}
