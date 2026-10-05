// Standalone emulator fixture. See README.md for generating the throwaway app.
use day::prelude::*;
day::day_start!(options: window(), root);
pub fn window() -> day::WindowOptions {
    day::WindowOptions {
        title: "Navigation isolation probe".into(),
        ..Default::default()
    }
}
pub fn root() -> AnyPiece {
    if std::env::var_os("PROBE_PLAIN").is_some() {
        let clicked = Signal::new(false);
        return label(move || {
            if clicked.get() {
                "WINDOW ACTION WORKED"
            } else {
                "Standalone toolbar"
            }
        })
        .toolbar(
            toolbar_button("plain-action", "Window action")
                .icon(Symbol::Add)
                .action(move || clicked.set(true)),
        )
        .any();
    }
    let mounted = Signal::new(true);
    let reset = Signal::new(0usize);
    column((
        // A root control destroys/rebuilds BOTH hosts without terminating the process.
        button("Mount/unmount tabs")
            .action(move || {
                mounted.update(|v| *v = !*v);
                reset.update(|v| *v += 1);
            })
            .id("mount-tabs"),
        label(move || format!("Generation {}", reset.get())),
        when(move || mounted.get(), tabs).grow(),
    ))
    .any()
}
fn tabs() -> impl Piece {
    let tab = Signal::new("library".to_string());
    let library = Signal::new(Vec::<String>::new());
    let catalogs = Signal::new(Vec::<String>::new());
    nav(tab)
        .style(NavStyle::Tabs)
        .title("Probe")
        .item("library", "Library", move || {
            stack("Library", library, catalogs)
        })
        .item("catalogs", "Catalogs", move || {
            stack("Catalogs", catalogs, library)
        })
        .item("settings", "Settings", || {
            if std::env::var_os("PROBE_SCROLL").is_some() {
                return scroll_probe().any();
            }
            column((
                label("SETTINGS ONLY").id("settings-marker"),
                label("No Catalogs header should be above this page."),
            ))
            .padding(20.)
            .any()
        })
        .id("probe-tabs")
}
fn stack(name: &'static str, path: Signal<Vec<String>>, other: Signal<Vec<String>>) -> impl Piece {
    let guarded = Signal::new(false);
    nav_stack(
        path,
        column((
            label(format!("{name} ROOT")),
            button("Push detail")
                .action(move || path.update(|p| p.push(format!("{name} detail"))))
                .id(format!("push-{name}")),
            button("Push hidden sibling")
                .action(move || other.update(|p| p.push("hidden detail".into())))
                .id(format!("hidden-{name}")),
            button("Push and pop immediately")
                .action(move || {
                    path.update(|p| p.push("transient".into()));
                    path.update(|p| {
                        p.pop();
                    });
                })
                .id(format!("transient-{name}")),
        ))
        .padding(20.),
    )
    .title(name)
    .toolbar(
        toolbar_button(format!("action-{name}"), format!("{name} action"))
            .icon(Symbol::Add)
            .action(|| {}),
    )
    .on_back(move |_| {
        if guarded.get() {
            BackResponse::Handled
        } else {
            BackResponse::Proceed
        }
    })
    .destination(move |key: &String| {
        column((
            label(key.clone()).id(format!("detail-{name}")),
            button("Push deeper")
                .action(move || path.update(|p| p.push(format!("{name} deeper"))))
                .id(format!("deeper-{name}")),
            button("Pop")
                .action(move || {
                    path.update(|p| {
                        p.pop();
                    })
                })
                .id(format!("pop-{name}")),
            button("Toggle Back guard")
                .action(move || guarded.update(|v| *v = !*v))
                .id(format!("guard-{name}")),
        ))
        .padding(20.)
    })
}

fn scroll_probe() -> impl Piece {
    let expanded = Signal::new(false);
    column((
        label("SCROLL EXTENT PROBE"),
        button("Change content height")
            .action(move || expanded.update(|v| *v = !*v))
            .id("resize-scroll-content"),
        scroll(
            column((
                label("Wrapped text must be measured at the actual viewport width. ".repeat(35)),
                when(
                    move || expanded.get(),
                    || label("Dynamically added wrapping content. ".repeat(90)),
                ),
                button("BOTTOM SENTINEL").action(|| {}).id("scroll-end"),
            ))
            .spacing(12.)
            .padding(8.),
        )
        .id("probe-scroll")
        .grow(),
    ))
    .spacing(8.)
    .padding(12.)
}
