// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Opt-in fixture for scripts/harmony-long-press.py. Only the driver sends touch input;
//! no synthetic Event::LongPress or public modifier is used to prove the native hook.

use day::prelude::*;
use day_spec::{Event, GestureKind};

struct HoldTarget<P> {
    child: P,
    holds: Signal<u32>,
    point: Signal<String>,
    mounted: Signal<bool>,
    remove_on_hold: Signal<bool>,
}

impl<P: Piece> Piece for HoldTarget<P> {
    fn build(self, cx: &mut day::BuildCx) -> day::RNode {
        let node = self.child.build(cx);
        day_core::with_tree(|tree| {
            tree.enable_gesture(node, GestureKind::LongPress);
            // Repeated installation on the same target must still deliver exactly once.
            tree.enable_gesture(node, GestureKind::LongPress);
        });
        cx.on(node, move |event| {
            if let Event::LongPress(p) = event {
                self.holds.update(|n| *n += 1);
                self.point.set(format!("Point: {:.1},{:.1}", p.x, p.y));
                if self.remove_on_hold.get() {
                    self.remove_on_hold.set(false);
                    self.mounted.set(false);
                }
            }
        });
        node
    }
}

pub fn page() -> impl Piece {
    let holds = Signal::new(0u32);
    let taps = Signal::new(0u32);
    let point = Signal::new(String::from("Point: none"));
    let mounted = Signal::new(true);
    let remove_on_hold = Signal::new(false);
    let list_mode = Signal::new(false);
    let rows = Signal::new(vec!["Alpha", "Beta", "Gamma", "Delta"]);
    column((
        label("Native long press"),
        label(move || format!("Holds: {}", holds.get())).id("holds"),
        label(move || format!("Taps: {}", taps.get())).id("taps"),
        label(move || point.get()).id("point"),
        label(move || format!("Order: {}", rows.get().join(","))).id("order"),
        row((
            button("Reset")
                .action(move || {
                    holds.set(0);
                    taps.set(0);
                    point.set("Point: none".into());
                })
                .id("reset"),
            button("Mount/unmount")
                .action(move || mounted.update(|v| *v = !*v))
                .id("mount"),
            button("Pad/list")
                .action(move || list_mode.update(|v| *v = !*v))
                .id("mode"),
        )),
        button("Remove on next hold")
            .action(move || remove_on_hold.set(true))
            .id("remove-on-hold"),
        when(
            move || mounted.get(),
            move || {
                when(
                    move || list_mode.get(),
                    move || {
                        list(
                            items(move || rows.get(), |name: &&str| *name),
                            move |row: ItemSlot<&str, &str>| HoldTarget {
                                child: label(move || row.get()).height(56.0),
                                holds,
                                point,
                                mounted,
                                remove_on_hold,
                            },
                        )
                        .row_height(day_spec::props::RowHeight::Uniform(56.0))
                        .reorderable(true)
                        .on_reorder(move |from, to| {
                            rows.update(|rows| {
                                let item = rows.remove(from);
                                rows.insert(to.min(rows.len()), item);
                            })
                        })
                        .height(240.0)
                        .id("list")
                    },
                )
                .otherwise(move || {
                    scroll(column((
                        HoldTarget {
                            // The frame is layout-only: exercise native-view descent and delivery
                            // back to the original handler node as well as a bare native target.
                            child: label("Hold here")
                                .frame(220.0, 100.0)
                                .on_tap(move || taps.update(|n| *n += 1))
                                .id("pad"),
                            holds,
                            point,
                            mounted,
                            remove_on_hold,
                        },
                        label("Scroll content").height(800.0),
                    )))
                    .height(240.0)
                    .id("scroll")
                })
            },
        ),
    ))
    .padding(12.0)
}
