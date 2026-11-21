// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    native::run();
}

/// The tree's sibling of `native_list_splice_in_layout`: a tree's nodes change and, before
/// day-core's posted flush runs, AppKit lays the tree out at a new place. `NSOutlineView` binds
/// the rows that came into view, and the bind flushes the reactive work the change left
/// pending, which reloads the tree from inside the outline's own update of its visible rows.
/// Every row must still show the node it stands for once the change has landed, and nothing
/// may throw.
#[cfg(target_os = "macos")]
mod native {
    use day_pieces::prelude::*;
    use day_spec::{Size, WindowOptions};
    use day_toolkit_appkit::AppKit;
    use objc2::{MainThreadMarker, Message, rc::Retained};
    use objc2_app_kit::{NSApplication, NSOutlineView, NSTextField, NSView, NSWindow};
    use objc2_foundation::NSPoint;
    use std::collections::HashSet;

    /// Every row is this tall.
    const ROW: f64 = 30.0;

    /// A node: its key and its parent's.
    type Node = (u64, Option<u64>);

    fn outlines(view: &NSView, found: &mut Vec<Retained<NSOutlineView>>) {
        if let Some(outline) = view.downcast_ref::<NSOutlineView>() {
            found.push(outline.retain());
            return;
        }
        for sub in &view.subviews() {
            outlines(&sub, found);
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
            eprintln!("native tree reload in layout: FAILED {what}");
            std::process::exit(1);
        }
        println!("native tree reload in layout: {what}");
    }

    /// What each row of a tree over `nodes` says, top to bottom, with the nodes in `open`
    /// disclosed.
    fn expected(nodes: &[Node], open: &HashSet<u64>) -> Vec<String> {
        fn walk(nodes: &[Node], open: &HashSet<u64>, parent: Option<u64>, out: &mut Vec<String>) {
            for (key, _) in nodes.iter().filter(|n| n.1 == parent) {
                out.push(format!("Row {key}"));
                if open.contains(key) {
                    walk(nodes, open, Some(*key), out);
                }
            }
        }
        let mut out = Vec::new();
        walk(nodes, open, None, &mut out);
        out
    }

    /// The outline's row count, and what each row in view says, from the first row in view on.
    fn shown(outline: &NSOutlineView) -> (usize, usize, Vec<String>) {
        let range = outline.rowsInRect(outline.visibleRect());
        let said = (range.location..range.location + range.length)
            .map(|row| {
                outline
                    .viewAtColumn_row_makeIfNecessary(0, row as isize, false)
                    .map_or_else(String::new, |cell| texts(&cell).join(" "))
            })
            .collect();
        (outline.numberOfRows().max(0) as usize, range.location, said)
    }

    /// Change the nodes, then, before the change's posted flush can run, move `outline` to `y`
    /// and have AppKit lay the window out: the binds of that layout run the pending reload.
    fn change_then_jump(outline: &NSOutlineView, change: impl FnOnce(), y: f64) {
        change();
        let scroll = outline.enclosingScrollView().unwrap();
        let clip = scroll.contentView();
        clip.scrollToPoint(NSPoint::new(0., y));
        scroll.reflectScrolledClipView(&clip);
        window().layoutIfNeeded();
    }

    /// Once the change has landed, `outline` shows every row of `nodes`, each saying what its
    /// node says.
    fn settled(outline: &NSOutlineView, nodes: &[Node], open: &HashSet<u64>, what: &str) {
        let want = expected(nodes, open);
        let (count, first, said) = shown(outline);
        let ok = count == want.len()
            && !said.is_empty()
            && first + said.len() <= want.len()
            && said[..] == want[first..first + said.len()];
        if !ok {
            eprintln!(
                "  {count} rows shown, {} wanted; from row {first}: {said:?}, wanted {:?}",
                want.len(),
                &want[first.min(want.len())..(first + said.len()).min(want.len())]
            );
        }
        check(ok, what);
    }

    pub fn run() {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(30));
            eprintln!("native tree reload in layout test timed out");
            std::process::exit(2);
        });
        // 300 root rows; the first has four children, disclosed.
        let nodes = Signal::new(
            (0..300_u64)
                .map(|k| (k, None))
                .chain((1000..1004_u64).map(|k| (k, Some(0))))
                .collect::<Vec<Node>>(),
        );
        let open = Signal::new(HashSet::from([0_u64]));
        // The node whose row asks for more once shown, and the root node it adds.
        let more = Signal::new((u64::MAX, 0_u64));
        day_core::launch_with(
            AppKit::new(),
            WindowOptions {
                title: "Fixture tree reload in layout".into(),
                size: Size::new(400., 300.),
                ..Default::default()
            },
            move || {
                day_core::task(async move {
                    day_core::sleep(300).await;
                    let mut found = Vec::new();
                    outlines(&window().contentView().unwrap(), &mut found);
                    check(found.len() == 1, "the tree is on screen");
                    let outline = &found[0];
                    settled(
                        outline,
                        &nodes.get(),
                        &open.get(),
                        "the tree shows its rows",
                    );
                    // Roots arrive at the end while the tree jumps far down.
                    change_then_jump(
                        outline,
                        || nodes.update(|v| v.extend((5000..5003).map(|k| (k, None)))),
                        150. * ROW,
                    );
                    check(true, "the tree survives an append mid-layout");
                    day_core::sleep(150).await;
                    settled(
                        outline,
                        &nodes.get(),
                        &open.get(),
                        "the tree shows the appended rows in order",
                    );
                    // Rows arrive above and leave below the viewport, a child among them,
                    // while the tree jumps back to its top.
                    change_then_jump(
                        outline,
                        || {
                            nodes.update(|v| {
                                v.insert(0, (5010, None));
                                v.insert(0, (5011, None));
                                v.retain(|n| n.0 != 40 && n.0 != 1001);
                                v.push((5012, Some(0)));
                            });
                        },
                        0.,
                    );
                    check(true, "the tree survives inserts and removals mid-layout");
                    day_core::sleep(150).await;
                    settled(
                        outline,
                        &nodes.get(),
                        &open.get(),
                        "the tree shows the changed rows in order",
                    );
                    // Two changes in one pass: the first pending when the pass starts, the
                    // second made by a row the pass binds (it asks for more once shown).
                    let now = nodes.with(|v| {
                        let mut v = v.clone();
                        v.insert(5, (5020, None));
                        v
                    });
                    let sentinel = expected(&now, &open.get())
                        .iter()
                        .skip(84)
                        .find_map(|row| row.strip_prefix("Row ")?.parse::<u64>().ok());
                    more.set((sentinel.unwrap_or(u64::MAX), 5021));
                    change_then_jump(
                        outline,
                        || nodes.update(|v| v.insert(5, (5020, None))),
                        80. * ROW,
                    );
                    day_core::sleep(150).await;
                    check(
                        nodes.with(|v| v.iter().any(|n| n.0 == 5021)),
                        "the tree's row asked for more",
                    );
                    settled(
                        outline,
                        &nodes.get(),
                        &open.get(),
                        "the tree applies changes made together in order",
                    );
                    std::process::exit(0);
                });
                tree(
                    branches(move || nodes.get(), |n: &Node| n.0, |n: &Node| n.1),
                    move |slot: ItemSlot<Node, u64>| {
                        // A row that, once it shows the node `more` names, has the app add
                        // another (a "load more" row).
                        day_reactive::Effect::new(move || {
                            let (shown, (wanted, add)) = (slot.get().0, more.get());
                            if shown == wanted {
                                more.set((u64::MAX, 0));
                                nodes.update(|v| {
                                    v.retain(|n| n.0 != 9);
                                    v.push((add, None));
                                });
                            }
                        });
                        label(move || format!("Row {}", slot.get().0)).grow(Dimension::Width)
                    },
                )
                .row_height(RowHeight::Uniform(ROW))
                .expanded(open)
                .grow(Dimension::Both)
                .any()
            },
        );
    }
}
