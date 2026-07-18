use day::prelude::*;
use day_piece_pullrefresh::pull_to_refresh;

use crate::widgets::heading;

/// A native recycling list (docs/list.md): 500 rows, but only the visible cells are ever built —
/// the platform's NSTableView / RecyclerView / GtkListView / QListView owns scrolling + reuse.
/// The list is wrapped in `pull_to_refresh` (day-piece-pullrefresh) — the recycling-list example:
/// a pull (or dayscript `toggle: {id: list-refresh}`) adds 100 rows, same as the button.
pub(crate) fn list_page() -> AnyPiece {
    let count = Signal::new(500i64);
    let refreshing = Signal::new(false);
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
            button(crate::res::str::list_add())
                .prominent()
                .action(move || count.update(|c| *c += 100))
                .id("list-add"),
        )),
        label(crate::res::str::list_caption(count)).id("list-caption"),
        pull_to_refresh(
            refreshing,
            list(
                move || {
                    (1..=count.get())
                        .map(|i| crate::res::str::list_row(i).format())
                        .collect::<Vec<_>>()
                },
                |s: &String| s.clone(),
                |row: ItemSlot<String, String>| {
                    label(move || row.get())
                        .padding(Insets::symmetric(12.0, 8.0))
                        .id_keyed("list-row", row.key())
                },
            )
            .row_height(RowHeight::Uniform(36.0))
            .on_select(|k| println!("selected {k}"))
            .id("demo-list"),
        )
        .id("list-refresh"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
