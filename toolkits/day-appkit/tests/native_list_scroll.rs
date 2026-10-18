// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    native::run();
}

#[cfg(target_os = "macos")]
mod native {
    use day_appkit::AppKit;
    use day_pieces::prelude::*;
    use day_spec::{Size, WindowOptions};
    use objc2::{
        DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
        rc::Retained,
    };
    use objc2_app_kit::{
        NSApplication, NSPasteboard, NSTableView, NSTableViewDropOperation, NSTextField, NSView,
    };
    use objc2_foundation::{NSIndexSet, NSObject, NSObjectProtocol, NSSize, NSString};

    // A synthetic local drag supplies only the two NSDraggingInfo selectors used by the
    // backend. Call the actual native acceptDrop delegate, not dayscript's reload-based path.
    struct DragState {
        table: Retained<NSTableView>,
        board: Retained<NSPasteboard>,
    }
    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[ivars = DragState]
        struct FixtureListDrag;
        unsafe impl NSObjectProtocol for FixtureListDrag {}
        impl FixtureListDrag {
            #[unsafe(method_id(draggingSource))]
            fn source(&self) -> Retained<NSTableView> { self.ivars().table.clone() }
            #[unsafe(method_id(draggingPasteboard))]
            fn board(&self) -> Retained<NSPasteboard> { self.ivars().board.clone() }
        }
    );

    fn table_in(view: &NSView) -> Option<Retained<NSTableView>> {
        if let Some(table) = view.downcast_ref::<NSTableView>() {
            return Some(table.retain());
        }
        view.subviews().iter().find_map(|v| table_in(&v))
    }

    fn texts(view: &NSView) -> Vec<String> {
        if let Some(label) = view.downcast_ref::<NSTextField>() {
            return vec![label.stringValue().to_string()];
        }
        view.subviews().iter().flat_map(|v| texts(&v)).collect()
    }

    fn native_drop(table: &NSTableView, from: usize, insertion: usize) {
        table.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(from), false);
        day_core::pump_events();
        table.setDropRow_dropOperation(insertion as isize, NSTableViewDropOperation::Above);
        let board = NSPasteboard::pasteboardWithUniqueName();
        board.setString_forType(
            &NSString::from_str(&from.to_string()),
            &NSString::from_str("dev.daybrite.day.row"),
        );
        let drag = FixtureListDrag::alloc(MainThreadMarker::new().unwrap()).set_ivars(DragState {
            table: table.retain(),
            board,
        });
        let drag: Retained<FixtureListDrag> = unsafe { msg_send![super(drag), init] };
        let delegate = unsafe { table.dataSource() }.unwrap();
        let accepted: bool = unsafe {
            msg_send![&*delegate, tableView: table, acceptDrop: &*drag, row: insertion as isize, dropOperation: NSTableViewDropOperation::Above]
        };
        assert!(accepted);
        table.setDropRow_dropOperation(-1, NSTableViewDropOperation::Above);
    }

    fn check_scrolling(table: &NSTableView, rows: &[u64]) {
        let scroll = table.enclosingScrollView().unwrap();
        assert_eq!(table.numberOfRows(), rows.len() as isize);
        assert!(
            (table.frame().size.height - rows.len() as f64 * 44.).abs() < 1.,
            "table height {} for {} rows",
            table.frame().size.height,
            rows.len()
        );
        for row in [rows.len() - 1, 0, rows.len() / 2, 0] {
            table.scrollRowToVisible(row as isize);
            table.layoutSubtreeIfNeeded();
            let visible = table.rowsInRect(table.visibleRect());
            assert!((visible.location..visible.location + visible.length).contains(&row));
            let cell = table
                .viewAtColumn_row_makeIfNecessary(0, row as isize, true)
                .unwrap();
            assert_eq!(texts(&cell), [format!("Fixture row {}", rows[row])]);
            if row == rows.len() - 1 {
                assert!(
                    scroll.contentView().bounds().origin.y > 2000.,
                    "can reach the last row after a drop"
                );
            } else if row == 0 {
                assert!(
                    scroll.contentView().bounds().origin.y < 1.,
                    "can return to the first row"
                );
            }
        }
    }

    pub fn run() {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(20));
            eprintln!("native list scrolling test timed out");
            std::process::exit(2);
        });
        day_core::launch_with(
            AppKit::new(),
            WindowOptions {
                title: "Fixture reorder scrolling".into(),
                size: Size::new(500., 500.),
                ..Default::default()
            },
            || {
                let rows = Signal::new((0..60u64).collect::<Vec<_>>());
                let selected = Signal::new(None::<u64>);
                day_core::task(async move {
                    day_core::sleep(200).await;
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let app =
                            NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
                        let table =
                            table_in(&app.windows().objectAtIndex(0).contentView().unwrap())
                                .unwrap();
                        check_scrolling(&table, &rows.get());
                        native_drop(&table, 0, 4);
                        // Fault injection: a drag's temporary geometry must not become the final
                        // scroll extent. The model still has 60 rows, but the document fits only
                        // the viewport, reproducing the stuck-at-an-edge scrolling state.
                        table.setFrameSize(NSSize::new(table.frame().size.width, 280.));
                        table
                    }));
                    let table = match result {
                        Ok(table) => table,
                        Err(_) => std::process::exit(1),
                    };
                    day_core::sleep(200).await;
                    // Exercise a long viewport history. Native retirement must remove pooled
                    // cells from Day's resize work set even though their row scopes survive.
                    for row in (0..60).step_by(5) {
                        table.scrollRowToVisible(row);
                        table.layoutSubtreeIfNeeded();
                        day_core::sleep(10).await;
                    }
                    let active = day_core::with_tree(|t| {
                        let mut node = t.find_by_id("fixture-list").unwrap();
                        while t.node_kind(node) != Some(day_spec::kinds::LIST) {
                            node = t.first_child(node).unwrap();
                        }
                        t.list_cell_keys(node).len()
                    });
                    assert!(
                        (1..30).contains(&active),
                        "retired cells still participate in layout: {active}"
                    );
                    println!("native list active layout cells after scrolling: {active}");
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        assert_eq!(&rows.get()[..4], &[1, 2, 3, 0]);
                        assert_eq!(selected.get(), Some(0));
                        assert_eq!(table.selectedRow(), 3);
                        check_scrolling(&table, &rows.get());
                        // A second drop at a scrolled position must preserve the viewport.
                        table.scrollRowToVisible(30);
                        let origin = table
                            .enclosingScrollView()
                            .unwrap()
                            .contentView()
                            .bounds()
                            .origin;
                        native_drop(&table, 30, 33);
                        origin
                    }));
                    let origin = match result {
                        Ok(origin) => origin,
                        Err(_) => std::process::exit(1),
                    };
                    day_core::sleep(200).await;
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        assert_eq!(
                            table
                                .enclosingScrollView()
                                .unwrap()
                                .contentView()
                                .bounds()
                                .origin,
                            origin
                        );
                        assert_eq!(selected.get(), Some(30));
                        assert_eq!(table.selectedRow(), 32);
                        check_scrolling(&table, &rows.get());
                        // A queued settle must tolerate the list being disposed by the app.
                        native_drop(&table, 0, 2);
                        day_core::initial_window().unwrap().close();
                    }));
                    if result.is_err() {
                        std::process::exit(1);
                    }
                    day_core::sleep(200).await;
                    println!(
                        "native list drop: scroll extent recovery, row content, selection and viewport preservation passed"
                    );
                    std::process::exit(0);
                });
                // Synthetic UI shaped like settings: a fixed-height native list inside a scroll.
                // Uniform rows: the extent arithmetic below (44 pt a row, a last-row offset past
                // 2000 pt) is the fixed-pitch table's; an Automatic list sizes each row to its
                // label (docs/list.md "Row heights") and has its own conformance cases.
                scroll(
                    column((
                        label("Synthetic list fixture"),
                        list(items(move || rows.get(), |r: &u64| *r), move |slot| {
                            label(move || format!("Fixture row {}", slot.get()))
                                .padding(10.)
                                .grow_w()
                        })
                        .on_selection(move |ids| selected.set(ids.first().copied()))
                        .selected_rows(move || {
                            rows.with(|r| {
                                r.iter()
                                    .position(|v| Some(*v) == selected.get())
                                    .into_iter()
                                    .collect()
                            })
                        })
                        .reorderable(true)
                        .on_reorder(move |from, to| {
                            rows.update(|r| {
                                let v = r.remove(from);
                                r.insert(to, v);
                            })
                        })
                        .row_height(RowHeight::Uniform(44.))
                        .height(280.)
                        .grow_w()
                        .id("fixture-list"),
                        label(move || format!("{} fixture rows", rows.with(|r| r.len()))),
                    ))
                    .spacing(10.)
                    .padding(20.),
                )
                .grow()
                .any()
            },
        );
    }
}
