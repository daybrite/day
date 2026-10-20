// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Public presentation APIs driven through real ArkUI controls by harmony-dialogs.py.
use day::prelude::*;

pub fn page() -> impl Piece {
    let answer = Signal::new(String::from("pending"));
    let first = Signal::new(String::from("pending"));
    let second = Signal::new(String::from("pending"));
    let startup = answer.setter();
    day::task(async move {
        let yes = confirm("Startup confirmation").present().await;
        startup.set(format!("startup {yes}"));
    });
    column((
        label("Native dialogs"),
        label(move || answer.get()).id(day::id!(answer)),
        label(move || first.get()).id(day::id!(first)),
        label(move || second.get()).id(day::id!(second)),
        button("Confirmation").id(day::id!(confirm)).action(move || {
            answer.set("pending".into());
            let set = answer.setter();
            day::task(async move {
                let yes = confirm("Delete \"東京\"?")
                    .message("This cannot be undone.\nKeep a backup.")
                    .confirm_label("Remove")
                    .cancel_label("Keep")
                    .destructive()
                    .present()
                    .await;
                set.set(format!("confirm {yes}"));
            });
        }),
        button("Text prompt").id(day::id!(prompt)).action(move || {
            answer.set("pending".into());
            let set = answer.setter();
            day::task(async move {
                let text = prompt("Your name")
                    .message("Choose a display name")
                    .placeholder("Name")
                    .initial("Ada")
                    .ok_label("Save")
                    .cancel_label("Cancel")
                    .present()
                    .await;
                set.set(format!("prompt {text:?}"));
            });
        }),
        button("Action sheet").id(day::id!(sheet)).action(move || {
            answer.set("pending".into());
            let set = answer.setter();
            day::task(async move {
                let choice = Alert::new("Choose an action")
                    .sheet()
                    .message("Four choices")
                    .button("First", 10)
                    .cancel("Cancel")
                    .destructive("Delete", 30)
                    .button("Last", 40)
                    .present()
                    .await;
                set.set(format!("sheet {choice:?}"));
            });
        }),
        button("No buttons").id(day::id!(empty)).action(move || {
            let set = answer.setter();
            day::task(async move {
                alert("No choices")
                    .message("Dismiss with Back or outside the dialog")
                    .await;
                set.set("empty dismissed".into());
            });
        }),
        button("Two queued dialogs").id(day::id!(pair)).action(move || {
            first.set("pending".into());
            second.set("pending".into());
            let a = first.setter();
            let b = second.setter();
            day::task(async move {
                let yes = confirm("Queue A").present().await;
                a.set(format!("first {yes}"));
            });
            day::task(async move {
                let yes = confirm("Queue B").present().await;
                b.set(format!("second {yes}"));
            });
        }),
        button("Chained dialogs").id(day::id!(chain)).action(move || {
            answer.set("pending".into());
            let set = answer.setter();
            day::task(async move {
                if confirm("Continue?").present().await {
                    let text = prompt("Next dialog").initial("ready").present().await;
                    set.set(format!("chain {text:?}"));
                } else {
                    set.set("chain cancelled".into());
                }
            });
        }),
        row((
            button("Dark")
                .id(day::id!(dark))
                .action(|| day::set_appearance(Some(true))),
            button("Light")
                .id(day::id!(light))
                .action(|| day::set_appearance(Some(false))),
            button("System")
                .id(day::id!(system))
                .action(|| day::set_appearance(None)),
        )),
    ))
    .padding(12.0)
}
