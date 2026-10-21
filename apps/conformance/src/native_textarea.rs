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
        label(move || format!("Submits: {}", submits.get())).id(day::id!(textarea::submits)),
        label(move || format!("Sent: {:?}", sent.get())).id(day::id!(textarea::sent)),
        label(move || format!("Model: {:?}", text.get())).id(day::id!(textarea::model)),
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
                            .id(day::id!(textarea::area))
                    },
                )
                .otherwise(move || {
                    text_area(text)
                        .min_lines(3)
                        .max_lines(3)
                        .id(day::id!(textarea::area))
                })
            },
        ),
        row((
            button("Editable")
                .id(day::id!(textarea::editable))
                .action(move || editable.update(|v| *v = !*v)),
            button("Plain/submit")
                .id(day::id!(textarea::mode))
                .action(move || submit_mode.update(|v| *v = !*v)),
        )),
        row((
            button("Clear on send")
                .id(day::id!(textarea::clear))
                .action(move || clear.update(|v| *v = !*v)),
            button("Remove on send")
                .id(day::id!(textarea::remove))
                .action(move || remove.update(|v| *v = !*v)),
        )),
        row((
            button("Mount")
                .id(day::id!(textarea::mount))
                .action(move || mounted.set(true)),
            button("Seed paste")
                .id(day::id!(textarea::seed))
                .action(|| assert!(day::clipboard::set_text("first\nsecond"))),
            button("Unicode")
                .id(day::id!(textarea::unicode))
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
        label(move || format!("Model: {}", text.get())).id(day::id!(textarea::model)),
        text_field(clipboard)
            .placeholder("Native paste check")
            .id(day::id!(textarea::clipboard)),
        when(
            move || mounted.get(),
            move || {
                text_area(text)
                    .editable(editable)
                    .selectable(selectable)
                    .min_lines(3)
                    .max_lines(3)
                    .id(day::id!(textarea::area))
            },
        ),
        row((
            button("Edit")
                .id(day::id!(textarea::edit))
                .action(move || editable.set(true)),
            button("Read only")
                .id(day::id!(textarea::readonly))
                .action(move || editable.set(false)),
        )),
        row((
            button("Select")
                .id(day::id!(textarea::select))
                .action(move || selectable.set(true)),
            button("No selection")
                .id(day::id!(textarea::noselect))
                .action(move || selectable.set(false)),
        )),
        row((
            button("Replace")
                .id(day::id!(textarea::replace))
                .action(move || text.set("Gamma Delta".into())),
            button("Reset")
                .id(day::id!(textarea::reset))
                .action(move || text.set("Alpha Beta".into())),
        )),
        row((
            button("Seed clipboard")
                .id(day::id!(textarea::seed))
                .action(|| {
                    assert!(day::clipboard::set_text("PasteMarker"));
                }),
            button("Clear paste check")
                .id(day::id!(textarea::clear_clipboard))
                .action(move || clipboard.set(String::new())),
        )),
        button("Mount/unmount")
            .id(day::id!(textarea::mount))
            .action(move || mounted.update(|v| *v = !*v)),
    ))
    .spacing(8.0)
    .padding(12.0)
}
