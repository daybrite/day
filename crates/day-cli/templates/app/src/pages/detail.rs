use crate::model::{ItemFields, Scene};
use crate::res;
use day::prelude::*;

/// Bind native form controls directly to the selected item's observable fields.
pub(crate) fn editor_pane(scene: Scene) -> impl Piece {
    column((
        when(
            move || scene.selected.get().is_none(),
            || label(res::str::item_none()).secondary().padding(24.0),
        ),
        // `each` holds zero or one editor: a new item key rebuilds its field bindings,
        // so edits cannot keep targeting the previous item.
        // Keyed children: https://daybrite.dev/docs/api-tour/#conditionals-and-collections
        each(
            items(
                move || scene.selected.get().into_iter().collect::<Vec<_>>(),
                |id: &u64| *id,
            ),
            move |slot: ItemSlot<u64, u64>| {
                let item = scene.items.elem(slot.key());
                // Controls write back through field bindings: https://daybrite.dev/docs/reactivity/
                form((section((
                    labeled(
                        res::str::field_name(),
                        text_field(item.name())
                            .placeholder(res::str::field_name_hint())
                            .a11y(|a| a.label(res::str::field_name()))
                            .id("field-name"),
                    ),
                    labeled(
                        res::str::field_done(),
                        toggle(item.done())
                            .a11y(|a| a.label(res::str::field_done()))
                            .id("field-done"),
                    ),
                )),))
                .padding(16.0)
            },
        ),
    ))
    .grow()
}
