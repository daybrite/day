// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Real TextArea input, driven by the harmony-textarea scripts (no synthetic text events).
use day::prelude::*;

/// Submission fixture: OS-input tests distinguish actual Send/Enter from injected Submitted.
pub fn submission_page() -> impl Piece {
    let text = Signal::new(String::from("Draft"));
    let sent = Signal::new(String::new());
    let submits = Signal::new(0u32);
    let editable = Signal::new(true);
    let submit_mode = Signal::new(true);
    let mounted = Signal::new(true);
    let clear = Signal::new(false);
    let remove = Signal::new(false);
    column((
        label(move || format!("Submits: {}", submits.get())).id("submits"),
        label(move || format!("Sent: {:?}", sent.get())).id("sent"),
        label(move || format!("Model: {:?}", text.get())).id("model"),
        when(
            move || mounted.get(),
            move || {
                when(
                    move || submit_mode.get(),
                    move || {
                        text_area(text)
                            .editable(editable)
                            .min_lines(3)
                            .max_lines(3)
                            .on_submit(move || {
                                submits.update(|n| *n += 1);
                                sent.set(text.get_untracked());
                                if clear.get_untracked() {
                                    text.set(String::new());
                                }
                                if remove.get_untracked() {
                                    mounted.set(false);
                                }
                            })
                            .id("area")
                    },
                )
                .otherwise(move || text_area(text).min_lines(3).max_lines(3).id("area"))
            },
        ),
        row((
            button("Editable")
                .id("editable")
                .action(move || editable.update(|v| *v = !*v)),
            button("Plain/submit")
                .id("mode")
                .action(move || submit_mode.update(|v| *v = !*v)),
        )),
        row((
            button("Clear on send")
                .id("clear")
                .action(move || clear.update(|v| *v = !*v)),
            button("Remove on send")
                .id("remove")
                .action(move || remove.update(|v| *v = !*v)),
        )),
        row((
            button("Mount")
                .id("mount")
                .action(move || mounted.set(true)),
            button("Seed paste")
                .id("seed")
                .action(|| assert!(day::clipboard::set_text("first\nsecond"))),
            button("Unicode")
                .id("unicode")
                .action(move || text.set("A😀東京B".into())),
        )),
    ))
    .spacing(4.0)
    .padding(12.0)
}

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
