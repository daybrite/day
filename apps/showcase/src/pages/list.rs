use day::prelude::*;

use crate::widgets::heading;

/// A native recycling list (docs/list.md): 500 rows, but only the visible cells are ever built —
/// the platform's NSTableView / RecyclerView / GtkListView / QListView owns scrolling + reuse.
pub(crate) fn list_page() -> AnyPiece {
    let count = Signal::new(500i64);
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
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
