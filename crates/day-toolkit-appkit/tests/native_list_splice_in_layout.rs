// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    native::run();
}

/// Rows change and, before day-core's posted flush runs, AppKit lays a list out at a new place:
/// `NSTableView` binds the rows that came into view, and the bind flushes the reactive work the
/// change left pending, which splices the list from inside the table's own update of its
/// visible rows. AppKit refuses a row insert there ("can not happen while updating visible
/// rows!") with an exception that aborts the process. The host holds such a change until the
/// pass is over, then applies it, and every row shows the item it stands for. A plain list and
/// a sectioned list of uniform rows (a reader's feed) share the rows.
#[cfg(target_os = "macos")]
mod native {
    use day_pieces::prelude::*;
    use day_spec::{Size, WindowOptions};
    use day_toolkit_appkit::AppKit;
    use objc2::{MainThreadMarker, Message, rc::Retained};
    use objc2_app_kit::{NSApplication, NSTableView, NSTextField, NSView, NSWindow};
    use objc2_foundation::NSPoint;

    /// Every item row is this tall.
    const ROW: f64 = 30.0;
    /// Items per section in the sectioned list.
    const GROUP: u64 = 10;

    fn tables(view: &NSView, found: &mut Vec<Retained<NSTableView>>) {
        if let Some(table) = view.downcast_ref::<NSTableView>() {
            found.push(table.retain());
            return;
        }
        for sub in &view.subviews() {
            tables(&sub, found);
        }
    }

    fn texts(view: &NSView) -> Vec<String> {
        if let Some(label) = view.downcast_ref::<NSTextField>() {
            return vec![label.stringValue().to_string()];
        }
        view.subviews().iter().flat_map(|v| texts(&v)).collect()
    }

    fn window() -> Retained<NSWindow> {
        let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
        app.windows().objectAtIndex(0)
    }

    fn check(ok: bool, what: &str) {
        if !ok {
            eprintln!("native list splice in layout: FAILED {what}");
            std::process::exit(1);
        }
        println!("native list splice in layout: {what}");
    }

    /// What each row of a list over `rows` says, top to bottom: a header row before each run of
    /// one section's items when `sectioned`.
    fn expected(rows: &[u64], sectioned: bool) -> Vec<String> {
        let mut out = Vec::new();
        let mut section = None;
        for row in rows {
            if sectioned && section != Some(row / GROUP) {
                section = Some(row / GROUP);
                out.push(format!("Section {}", row / GROUP));
            }
            out.push(format!("Row {row}"));
        }
        out
    }

    /// The table's row count, and what each row in view says, from the first row in view on.
    fn shown(table: &NSTableView) -> (usize, usize, Vec<String>) {
        let range = table.rowsInRect(table.visibleRect());
        let said = (range.location..range.location + range.length)
            .map(|row| {
                table
                    .viewAtColumn_row_makeIfNecessary(0, row as isize, true)
                    .map_or_else(String::new, |cell| texts(&cell).join(" "))
            })
            .collect();
        (table.numberOfRows().max(0) as usize, range.location, said)
    }

    /// Change the rows, then, before the change's posted flush can run, move `table` to `y`
    /// and have AppKit lay the window out: the binds of that layout run the pending splice.
    fn change_then_jump(table: &NSTableView, change: impl FnOnce(), y: f64) {
        change();
        let scroll = table.enclosingScrollView().unwrap();
        let clip = scroll.contentView();
        clip.scrollToPoint(NSPoint::new(0., y));
        scroll.reflectScrolledClipView(&clip);
        window().layoutIfNeeded();
    }

    /// Once the held change has landed, `table` shows every row of `rows`, each saying what
    /// its item says.
    fn settled(table: &NSTableView, rows: &[u64], sectioned: bool, what: &str) {
        let want = expected(rows, sectioned);
        let (count, first, said) = shown(table);
        if count != want.len() || said.is_empty() || said[..] != want[first..first + said.len()] {
            eprintln!(
                "  {count} rows shown, {} wanted; from row {first}: {said:?}, wanted {:?}",
                want.len(),
                &want[first.min(want.len())..(first + said.len()).min(want.len())]
            );
        }
        check(
            count == want.len()
                && !said.is_empty()
                && first + said.len() <= want.len()
                && said[..] == want[first..first + said.len()],
            what,
        );
    }

    pub fn run() {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(30));
            eprintln!("native list splice in layout test timed out");
            std::process::exit(2);
        });
        let rows = Signal::new((0..300_u64).collect::<Vec<_>>());
        // The item whose row asks for more once shown, and the item it adds.
        let more = Signal::new((u64::MAX, 0_u64));
        day_core::launch_with(
            AppKit::new(),
            WindowOptions {
                title: "Fixture splice in layout".into(),
                size: Size::new(600., 300.),
                ..Default::default()
            },
            move || {
                day_core::task(async move {
                    day_core::sleep(300).await;
                    let mut found = Vec::new();
                    tables(&window().contentView().unwrap(), &mut found);
                    check(found.len() == 2, "both lists are on screen");
                    for (at, (table, sectioned)) in found.iter().zip([false, true]).enumerate() {
                        let kind = if sectioned { "sectioned" } else { "plain" };
                        // Fresh items each round: a row's item is its identity.
                        let base = 10_000 * (at as u64 + 1);
                        // Rows arrive at the end while the list jumps far down: the reported
                        // case (Home, the scroller's knob, then rows before the next layout).
                        change_then_jump(
                            table,
                            || rows.update(|v| v.extend(base..base + 3)),
                            150. * ROW,
                        );
                        check(
                            true,
                            &format!("the {kind} list survives an append mid-layout"),
                        );
                        day_core::sleep(150).await;
                        settled(
                            table,
                            &rows.get(),
                            sectioned,
                            &format!("the {kind} list shows the appended rows in order"),
                        );
                        // Rows arrive above and leave below the viewport while the list jumps
                        // back to its top: rows bound during the held change show their item.
                        change_then_jump(
                            table,
                            || {
                                rows.update(|v| {
                                    v.insert(0, base + 10);
                                    v.insert(0, base + 11);
                                    v.remove(40);
                                    v.swap(3, 4);
                                });
                            },
                            0.,
                        );
                        check(
                            true,
                            &format!(
                                "the {kind} list survives inserts, a removal and a move mid-layout"
                            ),
                        );
                        day_core::sleep(150).await;
                        settled(
                            table,
                            &rows.get(),
                            sectioned,
                            &format!("the {kind} list shows the changed rows in order"),
                        );
                        // Two changes in one pass: the first pending when the pass starts, the
                        // second made by a row the pass binds (it asks for more once shown).
                        // Both are held, and land in order.
                        let now = rows.with(|v| {
                            let mut v = v.clone();
                            v.insert(5, base + 20);
                            v
                        });
                        let sentinel = expected(&now, sectioned)
                            .iter()
                            .skip(84)
                            .find_map(|row| row.strip_prefix("Row ")?.parse::<u64>().ok());
                        more.set((sentinel.unwrap_or(u64::MAX), base + 21));
                        change_then_jump(
                            table,
                            || rows.update(|v| v.insert(5, base + 20)),
                            80. * ROW,
                        );
                        day_core::sleep(150).await;
                        check(
                            rows.with(|v| v.contains(&(base + 21))),
                            &format!("the {kind} list's row asked for more"),
                        );
                        settled(
                            table,
                            &rows.get(),
                            sectioned,
                            &format!("the {kind} list applies changes held together in order"),
                        );
                    }
                    std::process::exit(0);
                });
                // A row that, once it shows the item `more` names, has the app add another (a
                // "load more" row).
                let item = move |slot: ItemSlot<u64, u64>| {
                    day_reactive::Effect::new(move || {
                        let (shown, (wanted, add)) = (slot.get(), more.get());
                        if shown == wanted {
                            more.set((u64::MAX, 0));
                            rows.update(|v| {
                                v.remove(9);
                                v.push(add);
                            });
                        }
                    });
                    label(move || format!("Row {}", slot.get())).grow(Dimension::Width)
                };
                row((
                    list(items(move || rows.get(), |r: &u64| *r), item)
                        .row_height(RowHeight::Uniform(ROW))
                        .grow(Dimension::Both),
                    list(items(move || rows.get(), |r: &u64| *r), item)
                        .sections(
                            move |i| rows.with(|v| v.get(i).map_or(0, |r| r / GROUP)),
                            |slot| label(move || format!("Section {}", slot.get() / GROUP)),
                        )
                        .row_height(RowHeight::Uniform(ROW))
                        .grow(Dimension::Both),
                ))
                .grow(Dimension::Both)
                .any()
            },
        );
    }
}
