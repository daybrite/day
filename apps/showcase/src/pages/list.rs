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
    let count = Signal::new(500i64);
    let refreshing = Signal::new(false);
    // The selected ROW NUMBERS (1-based, matching the row labels), fed from the native list's
    // selection reports; single-selection toolkits contribute one-element sets.
    let selected: Signal<BTreeSet<i64>> = Signal::new(BTreeSet::new());
    // The one reload path for every begin (pull, toggle, programmatic): off the UI thread,
    // completion hops back through the Setters (docs/pullrefresh.md).
    watch(
        move || refreshing.get(),
        move |now, _| {
            if *now {
                let next = count.get_untracked() + 100;
                let grow = count.setter();
                let done = refreshing.setter();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(900));
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
                .action(move || count.update(|c| *c += 100))
                .id("list-add"),
        ))
        .spacing(8.0),
        label(crate::res::str::list_caption(count)).id("list-caption"),
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
                move || (1..=count.get()).collect::<Vec<i64>>(),
                |n: &i64| *n,
                |row: ItemSlot<i64, i64>| {
                    label(move || crate::res::str::list_row(row.get()).format())
                        .padding(Insets::symmetric(12.0, 8.0))
                        .id_keyed("list-row", row.key())
                },
            )
            .row_height(RowHeight::Uniform(36.0))
            .multi_select(true)
            // Keys ARE the row numbers, so the selection set is the report itself.
            .on_selection(move |keys: Vec<i64>| selected.set(keys.into_iter().collect()))
            // Two-way: app-state changes (Clear Selection) sync into the native list —
            // indices are 0-based rows.
            .selected_rows(move || {
                selected
                    .get()
                    .iter()
                    .map(|n| (*n - 1).max(0) as usize)
                    .collect()
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
