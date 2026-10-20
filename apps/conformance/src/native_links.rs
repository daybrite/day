// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Native span hit testing; driven by scripts/harmony-links.py with OS taps.
use day::prelude::*;

pub fn page() -> impl Piece {
    let count = Signal::new(0u32);
    let target = Signal::new(String::new());
    let source = Signal::new(
        "😀 [Alpha](#alpha) / [Beta](https://example.invalid/β?x=1&y=2) / plain".to_string(),
    );
    let mounted = Signal::new(true);
    let remove = Signal::new(false);
    let route = Signal::new("home".to_string());
    let record = move |url: &str| {
        count.update(|n| *n += 1);
        target.set(url.into());
    };
    column((
        label(move || format!("Clicks: {}", count.get())).id(day::id!(count)),
        label(target).id(day::id!(target)),
        when(
            move || mounted.get(),
            move || {
                label(source)
                    .markdown()
                    .font(Font::System(20.0))
                    .on_link(move |url| {
                        record(url);
                        if remove.get_untracked() {
                            mounted.set(false);
                        }
                    })
                    .id(day::id!(mixed))
            },
        ),
        label("[link link link link link link link link](#wrapped)")
            .markdown()
            .font(Font::System(20.0))
            .on_link(record)
            .width(130.0)
            .id(day::id!(wrapped)),
        label("")
            .runs_from(TextBuilder::new().link("Builder 東京", "custom:東京?x=1&y=2"))
            .on_link(record)
            .id(day::id!(builder)),
        label("[Selectable](#selectable)")
            .markdown()
            .on_link(record)
            .selectable()
            .id(day::id!(selectable)),
        link("Go to destination", "#destination").id(day::id!(route_link)),
        row((
            button("Retarget")
                .id(day::id!(retarget))
                .action(move || source.set("😀 [Alpha](#changed) / [Beta](#other) / plain".into())),
            button("Plain")
                .id(day::id!(plain))
                .action(move || source.set("😀 Alpha / Beta / plain".into())),
            button("Restore").id(day::id!(restore)).action(move || {
                source.set(
                    "😀 [Alpha](#alpha) / [Beta](https://example.invalid/β?x=1&y=2) / plain".into(),
                )
            }),
        )),
        row((
            button("Remove on click")
                .id(day::id!(remove))
                .action(move || remove.update(|v| *v = !*v)),
            button("Mount")
                .id(day::id!(mount))
                .action(move || mounted.set(true)),
        )),
        nav(route)
            .item("home", "Home", || label("Home page"))
            .item("destination", "Destination", || {
                label("Destination page").id(day::id!(destination))
            })
            .height(90.0),
    ))
    .spacing(8.0)
    .padding(12.0)
}
