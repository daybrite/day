use std::collections::BTreeSet;

use day::prelude::*;
use day_piece_pullrefresh::pull_to_refresh;

use crate::widgets::heading;

/// A native recycling list (docs/list.md): 500 rows, but only the visible cells are ever built —
/// the platform's NSTableView / RecyclerView / GtkListView / QListView owns scrolling + reuse.
/// The list is wrapped in `pull_to_refresh` (day-piece-pullrefresh) — the recycling-list example:
/// a pull (or dayscript `toggle: {id: list-refresh}`) adds 100 rows, same as the button.
///
/// Selection (docs/list.md): the rows are multi-selectable where the toolkit supports it, the
/// full selection lives in an app signal summarized above the list (with ranges compressed,
/// "4-10"), and Clear Selection syncs an empty selection back into the native list.
pub(crate) fn list_page() -> AnyPiece {
    // A real Vec of row numbers (not a derived range): drag-to-reorder rotates it, shuffle
    // permutes it, and the refresh paths append to it — the order is app-owned state.
    let rows: Signal<Vec<i64>> = Signal::new((1..=500).collect());
    let refreshing = Signal::new(false);
    // Programmatic scrolling (docs/list.md): a row-jump signal + an end trigger drive the
    // native list — the row rail's counterpart to `scroll(...).scroll_target(...)`.
    let jump_row: Signal<Option<usize>> = Signal::new(None);
    let jump_end = Trigger::new();
    // The selected ROW NUMBERS (1-based, matching the row labels), fed from the native list's
    // selection reports; single-selection toolkits contribute one-element sets.
    let selected: Signal<BTreeSet<i64>> = Signal::new(BTreeSet::new());
    // The one reload path for every begin (pull, toggle, programmatic): a timed task on the
    // main-loop executor (`day::sleep`, docs/async.md) stands in for the network — the same
    // code on every platform, including the single-threaded web backend.
    // The target row total. The async refresh below bumps it through a `Setter` (Send), and this
    // watch appends the new rows — reorder-safe, because appending never disturbs the order.
    let total = Signal::new(500i64);
    watch(
        move || total.get(),
        move |t, _| {
            rows.update(|v| {
                let next = v.iter().copied().max().unwrap_or(0);
                if *t > next {
                    v.extend(next + 1..=*t);
                }
            });
        },
    );
    watch(
        move || refreshing.get(),
        move |now, _| {
            if *now {
                let next = total.get_untracked() + 100;
                let grow = total.setter();
                let done = refreshing.setter();
                day::task(async move {
                    day::sleep(900).await;
                    grow.set(next);
                    done.set(false);
                });
            }
        },
    );
    column((
        row((
            heading(crate::res::str::nav_list(), "list-title", None),
            spacer(),
            button(crate::res::str::list_clear_selection())
                .bordered()
                .action(move || selected.set(BTreeSet::new()))
                .id("list-clear"),
            button(crate::res::str::list_add())
                .prominent()
                .action(move || total.update(|c| *c += 100))
                .id("list-add"),
        ))
        .spacing(8.0),
        label(move || crate::res::str::list_caption(rows.get().len() as i64).format())
            .id("list-caption"),
        // Programmatic scrolling + order controls, merged from the old Scrolling page: the
        // buttons drive the RECYCLING list (scroll-to-row realizes virtualized rows), and
        // Shuffle/Reset permute the backing Vec — animated as native row moves where the
        // toolkit supports it (docs/list.md).
        row((
            button(crate::res::str::scroll_to_top())
                .bordered()
                .action(move || jump_row.set(Some(0)))
                .id("list-scroll-top"),
            button(crate::res::str::scroll_to_item())
                .bordered()
                .action(move || jump_row.set(Some(99)))
                .id("list-scroll-item"),
            button(crate::res::str::scroll_to_bottom())
                .bordered()
                .action(move || jump_end.notify())
                .id("list-scroll-bottom"),
            button(crate::res::str::list_shuffle())
                .bordered()
                .action(move || rows.update(|v| shuffle(v)))
                .id("list-shuffle"),
            button(crate::res::str::list_reset())
                .bordered()
                .action(move || rows.update(|v| v.sort_unstable()))
                .id("list-reset"),
        ))
        .spacing(8.0),
        // Drag-to-reorder (docs/list.md): the hint names the pinned-row guard, and the order
        // readout makes the committed order assertable (dayscript `reorder` steps check it).
        label(crate::res::str::list_reorder_hint())
            .font(Font::Footnote)
            .id("list-reorder-hint"),
        label(move || {
            let first: Vec<String> = rows.get().iter().take(5).map(|n| n.to_string()).collect();
            crate::res::str::list_order(first.join(",")).format()
        })
        .font(Font::Footnote)
        .id("list-order"),
        // The live selection summary: pluralized per locale, runs compressed ("4-10").
        label(move || {
            let sel = selected.get();
            if sel.is_empty() {
                crate::res::str::list_selection_none().format()
            } else {
                crate::res::str::list_selection(sel.len() as i64, format_rows(&sel)).format()
            }
        })
        .font(Font::Footnote)
        .id("list-selection"),
        pull_to_refresh(
            refreshing,
            list(
                move || rows.get(),
                |n: &i64| *n,
                |row: ItemSlot<i64, i64>| {
                    // Row 100 wears the warm accent so "Scroll to item 100" visibly lands on
                    // it; the rest keep the theme-neutral slate the old Scrolling page used
                    // (reactive, so recycled cells restyle as they rebind).
                    label(move || crate::res::str::list_row(row.get()).format())
                        .color(move || {
                            if row.get() == 100 {
                                crate::palette::CORAL
                            } else {
                                crate::palette::SLATE
                            }
                        })
                        .padding(Insets::symmetric(12.0, 8.0))
                        .id_keyed("list-row", row.key())
                },
            )
            .row_height(RowHeight::Uniform(36.0))
            .multi_select(true)
            // Keys ARE the row numbers, so the selection set is the report itself.
            .on_selection(move |keys: Vec<i64>| selected.set(keys.into_iter().collect()))
            // Two-way: app-state changes (Clear Selection) sync into the native list —
            // indices are the rows' CURRENT positions (reorder can move them).
            .selected_rows(move || {
                let v = rows.get();
                selected
                    .get()
                    .iter()
                    .filter_map(|n| v.iter().position(|r| r == n))
                    .collect()
            })
            .scroll_to_row(jump_row)
            .scroll_to_end(jump_end)
            // Native drag-to-reorder, with a guard demo: the first row is pinned — it cannot be
            // dragged, and a drop aimed at its slot lands just below it instead (Retarget).
            .reorderable(true)
            .reorder_guard(|from, to| {
                if from == 0 {
                    Reorder::Deny
                } else if to == 0 {
                    Reorder::Retarget(1)
                } else {
                    Reorder::Allow
                }
            })
            .on_reorder(move |from, to| {
                rows.update(|v| {
                    let it = v.remove(from);
                    v.insert(to, it);
                });
            })
            .id("demo-list"),
        )
        .id("list-refresh"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Fisher–Yates over a hand-rolled xorshift64 (no rand dependency; the seed is fixed and
/// advances per call, so the sequence is deterministic per launch — CI-friendly — while
/// successive shuffles differ).
fn shuffle(v: &mut [i64]) {
    thread_local! {
        static SEED: std::cell::Cell<u64> = const { std::cell::Cell::new(0x9E37_79B9_7F4A_7C15) };
    }
    let mut s = SEED.with(|c| c.get());
    for i in (1..v.len()).rev() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        v.swap(i, (s % (i as u64 + 1)) as usize);
    }
    SEED.with(|c| c.set(s));
}

/// Compress sorted row numbers into the display value: runs of three or more become a range
/// ("4-10"), everything else lists out ("1,2,8"). A value, not prose — identical per locale.
fn format_rows(rows: &BTreeSet<i64>) -> String {
    let mut out = String::new();
    let mut iter = rows.iter().copied().peekable();
    while let Some(start) = iter.next() {
        let mut end = start;
        while iter.peek() == Some(&(end + 1)) {
            end = iter.next().unwrap_or(end);
        }
        if !out.is_empty() {
            out.push(',');
        }
        match end - start {
            0 => out.push_str(&start.to_string()),
            1 => out.push_str(&format!("{start},{end}")),
            _ => out.push_str(&format!("{start}-{end}")),
        }
    }
    out
}
