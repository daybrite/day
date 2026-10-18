// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Real TextArea input, driven by scripts/harmony-textarea.py (no synthetic text events).
use day::prelude::*;

pub fn page() -> impl Piece {
    let text = Signal::new(String::from("Alpha Beta"));
    let editable = Signal::new(false);
    let selectable = Signal::new(true);
    let mounted = Signal::new(true);
    let clipboard = Signal::new(String::new());
    column((
        label("Native text area"),
        label(move || format!("Model: {}", text.get())).id("model"),
        text_field(clipboard)
            .placeholder("Native paste check")
            .id("clipboard"),
        when(
            move || mounted.get(),
            move || {
                text_area(text)
                    .editable(editable)
                    .selectable(selectable)
                    .min_lines(3)
                    .max_lines(3)
                    .id("area")
            },
        ),
        row((
            button("Edit").id("edit").action(move || editable.set(true)),
            button("Read only")
                .id("readonly")
                .action(move || editable.set(false)),
        )),
        row((
            button("Select")
                .id("select")
                .action(move || selectable.set(true)),
            button("No selection")
                .id("noselect")
                .action(move || selectable.set(false)),
        )),
        row((
            button("Replace")
                .id("replace")
                .action(move || text.set("Gamma Delta".into())),
            button("Reset")
                .id("reset")
                .action(move || text.set("Alpha Beta".into())),
        )),
        row((
            button("Seed clipboard").id("seed").action(|| {
                assert!(day::clipboard::set_text("PasteMarker"));
            }),
            button("Clear paste check")
                .id("clear-clipboard")
                .action(move || clipboard.set(String::new())),
        )),
        button("Mount/unmount")
            .id("mount")
            .action(move || mounted.update(|v| *v = !*v)),
    ))
    .spacing(8.0)
    .padding(12.0)
}
