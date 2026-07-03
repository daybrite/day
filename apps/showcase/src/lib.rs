//! The day showcase (DESIGN.md Appendix A, staged): every implemented piece behind a
//! native navigation host (docs/navigation.md) — stack presentation on mobile, sidebar +
//! detail split on desktop. Three destinations: controls, gauge, about.

use day::prelude::*;
use day_piece_combobox::combo_box;

pub fn root() -> AnyPiece {
    install_locales(
        "en",
        &[
            ("en", include_str!("../locales/en/app.ftl")),
            ("fr", include_str!("../locales/fr/app.ftl")),
        ],
    );
    // Top-level navigation is a NavigationSplitView (docs/navigation.md): a `selector` bound
    // to an app-owned `Signal<String>` of the active section. Desktop shows sidebar + detail
    // (an AdwNavigationSplitView on GTK); mobile collapses to a list that pushes the detail.
    let section = Signal::new(String::new());
    selector(section)
        .style(SelectorStyle::Sidebar)
        .title(tr("app-title"))
        .header(sidebar_header)
        .item("controls", tr("nav-controls"), controls_page)
        .item("gauge", tr("nav-gauge"), gauge_page)
        .item("modals", tr("nav-modals"), modals_page)
        .item("tabs", tr("nav-tabs"), tabs_page)
        .item("stack", tr("nav-stack"), stack_page)
        .item("about", tr("nav-about"), about_page)
        .id("nav")
}

/// The sidebar's header (logo + app name); the `selector` renders the item list below it as
/// the native source list / navigation-sidebar / bottom list.
fn sidebar_header() -> AnyPiece {
    row((
        image("day-logo.png").frame(28.0, 28.0),
        label(tr("app-title")).font(Font::Headline).id("home-title"),
    ))
    .spacing(8.0)
    .padding(12.0)
    .any()
}

/// Every interactive control, with stable ids for the walkthrough (§14).
fn controls_page() -> AnyPiece {
    let count = Signal::new(0i64);
    let name = Signal::new(String::new());
    let volume = Signal::new(40.0f64);
    let subscribed = Signal::new(false);
    let flavors = Signal::new(vec![
        "vanilla".to_string(),
        "chocolate".into(),
        "pistachio".into(),
    ]);
    let flavor = Signal::new(Some(0usize));

    scroll(
        column((
            label(tr("nav-controls"))
                .font(Font::Title)
                .id("controls-title"),
            // — state: counter —
            row((
                // The buttons log to the two standard streams (stderr / stdout) so
                // `day launch` can demonstrate forwarding both, per platform.
                button(tr("decrement"))
                    .action(move || {
                        count.update(|c| *c -= 1);
                        eprintln!("counter decremented to {}", count.get_untracked());
                    })
                    .id("decrement-button"),
                label(tr("counter-value").arg("count", count)).id("counter-label"),
                button(tr("increment"))
                    .action(move || {
                        count.update(|c| *c += 1);
                        println!("counter incremented to {}", count.get_untracked());
                    })
                    .id("increment-button"),
            ))
            .spacing(8.0),
            divider(),
            // — text input + conditional —
            text_field(name)
                .placeholder(tr("name-placeholder"))
                .id("name-field"),
            when(
                move || !name.with(|s| s.is_empty()),
                move || label(tr("greeting").arg("name", name)).id("greeting-label"),
            ),
            // — slider with live readout —
            row((
                label(tr("volume-label")),
                slider(volume).range(0.0..=100.0).id("volume-slider"),
                label(move || format!("{:.0}", dbg!(volume.get()))).id("volume-value"),
            ))
            .spacing(8.0),
            // — a determinate progress bar tracking the slider live, and a spinner —
            row((
                label(tr("progress-label")),
                progress(move || volume.get() / 100.0)
                    .id("volume-progress")
                    .a11y(|a| a.role(Role::Meter).label("Volume level")),
            ))
            .spacing(8.0),
            row((label(tr("busy-label")), spinner().id("busy-spinner"))).spacing(8.0),
            toggle(subscribed)
                .id("subscribe-toggle")
                .a11y(|a| a.label("Subscribe to updates")), // a11y strings localize at M6.5
            // — an EXTERNAL Day Piece, registered like any built-in (§8.2, DP-21) —
            row((
                label(tr("flavor-label")),
                combo_box(flavors, flavor).id("flavor-combo"),
                label(move || {
                    let names = flavors.get();
                    flavor
                        .get()
                        .and_then(|i| names.get(i).cloned())
                        .unwrap_or_default()
                })
                .id("flavor-value"),
            ))
            .spacing(8.0),
            divider(),
            // — keyed collection (watch + monotonic keys, §5.4 / A.1) —
            history(count),
        ))
        .spacing(12.0)
        .align(HAlign::Leading)
        .padding(16.0),
    )
    .any()
}

/// Canvas gauge (§11) driven by its own slider.
fn gauge_page() -> AnyPiece {
    let level = Signal::new(40.0f64);
    column((
        row((
            label(tr("volume-label")),
            slider(level).range(0.0..=100.0).id("gauge-slider"),
        ))
        .spacing(8.0),
        gauge(level),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Imperative modals (docs/dialogs.md): each button opens a native dialog from within an
/// async task and writes a fixed result token to `modal-result` (locale-independent so the
/// walkthrough can assert it).
fn modals_page() -> AnyPiece {
    let last = Signal::new(String::new());
    column((
        label(tr("nav-modals")).font(Font::Title).id("modals-title"),
        button(tr("modal-alert"))
            .action(move || {
                day::task(async move {
                    alert(tr("alert-title"))
                        .message(tr("alert-body"))
                        .button(tr("ok"), ())
                        .present()
                        .await;
                    last.set("alert-ok".into());
                })
            })
            .id("btn-alert"),
        button(tr("modal-confirm"))
            .action(move || {
                day::task(async move {
                    let ok = confirm(tr("confirm-title"))
                        .message(tr("confirm-body"))
                        .await;
                    last.set(if ok { "confirm-yes" } else { "confirm-no" }.into());
                })
            })
            .id("btn-confirm"),
        button(tr("modal-delete"))
            .action(move || {
                day::task(async move {
                    let ok = confirm(tr("delete-title"))
                        .message(tr("delete-body"))
                        .confirm_label(tr("delete"))
                        .destructive()
                        .await;
                    last.set(if ok { "delete-yes" } else { "delete-no" }.into());
                })
            })
            .id("btn-delete"),
        button(tr("modal-sheet"))
            .action(move || {
                day::task(async move {
                    let choice = Alert::new(tr("flavor-title"))
                        .sheet()
                        .button(tr("vanilla"), 0i64)
                        .button(tr("pistachio"), 1i64)
                        .cancel(tr("cancel"))
                        .present()
                        .await;
                    last.set(match choice {
                        Some(i) => format!("sheet-{i}"),
                        None => "sheet-cancel".into(),
                    });
                })
            })
            .id("btn-sheet"),
        button(tr("modal-prompt"))
            .action(move || {
                day::task(async move {
                    let name = prompt(tr("name-placeholder")).await;
                    last.set(match name {
                        Some(t) => format!("prompt-{t}"),
                        None => "prompt-none".into(),
                    });
                })
            })
            .id("btn-prompt"),
        divider(),
        label(move || last.get()).id("modal-result"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Native tabbed container (docs/tabs.md): a `selector` with `SelectorStyle::Tabs`, bound to a
/// `Signal<String>` of the active tab key. NSTabView / UITabBarController / GtkNotebook /
/// QTabWidget / Android tab strip. Keys are routes, so deep links and dayscript select tabs.
fn tabs_page() -> AnyPiece {
    fn pane(title: LocalizedText, body: LocalizedText, content_id: &'static str) -> AnyPiece {
        column((label(title).font(Font::Title), label(body).id(content_id)))
            .spacing(10.0)
            .align(HAlign::Leading)
            .padding(16.0)
            .any()
    }
    let tab = Signal::new("one".to_string());
    selector(tab)
        .style(SelectorStyle::Tabs)
        .item("one", tr("tab-one"), || {
            pane(tr("tab-one"), tr("tab-one-body"), "tab-one-content")
        })
        .item("two", tr("tab-two"), || {
            pane(tr("tab-two"), tr("tab-two-body"), "tab-two-content")
        })
        .item("three", tr("tab-three"), || {
            pane(tr("tab-three"), tr("tab-three-body"), "tab-three-content")
        })
        .id("demo-tabs")
}

/// Genuine push/pop navigation (docs/navigation.md): `stack` bound to a `Signal<Vec<String>>`
/// path. Pushing a detail appends to the path; day reconciles the native UINavigationController
/// / AdwNavigationView / back-stack; the native back button writes the pop back into the path.
fn stack_page() -> AnyPiece {
    fn push(path: Signal<Vec<String>>) {
        let mut v = path.get_untracked();
        let n = v.len() + 1;
        v.push(format!("{n}"));
        path.set(v);
    }
    let path = Signal::new(Vec::<String>::new());
    let root = column((
        label(tr("stack-root-body")).id("stack-root"),
        button(tr("stack-push"))
            .action(move || push(path))
            .id("stack-push"),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0);
    stack(path, root)
        .destination(move |key| {
            let depth = key.to_string();
            column((
                label(tr("stack-detail-title").arg("depth", depth))
                    .font(Font::Title)
                    .id("stack-detail"),
                label(tr("stack-detail-body")),
                button(tr("stack-push"))
                    .action(move || push(path))
                    .id("stack-deeper"),
            ))
            .spacing(12.0)
            .align(HAlign::Leading)
            .padding(16.0)
        })
        .id("demo-stack")
}

fn about_page() -> AnyPiece {
    column((
        image("day-logo.png").frame(96.0, 96.0),
        label(tr("app-title")).font(Font::Headline),
        label(tr("about-text")).id("about-text"),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

fn gauge(value: Signal<f64>) -> AnyPiece {
    canvas(move |d, size| {
        if size.width <= 0.0 {
            return;
        }
        let r = Rect::from_size(size).inset(8.0);
        let track = Color::rgba(0.5, 0.5, 0.55, 0.35);
        let accent = Color::hex(0x2F6FDE);
        d.stroke(
            Shape::Arc {
                rect: r,
                start_deg: 135.0,
                sweep_deg: 270.0,
            },
            track,
            6.0,
        );
        let frac = (value.get() / 100.0).clamp(0.0, 1.0);
        if frac > 0.0 {
            d.stroke(
                Shape::Arc {
                    rect: r,
                    start_deg: 135.0,
                    sweep_deg: 270.0 * frac,
                },
                accent,
                6.0,
            );
        }
        d.text(
            &format!("{:.0}", value.get()),
            Point::new(size.width / 2.0, size.height / 2.0),
            TextStyle {
                size: 22.0,
                color: accent,
                anchor: TextAnchor::Centered,
            },
        );
    })
    .frame(110.0, 110.0)
    .id("gauge")
}

fn history(count: Signal<i64>) -> AnyPiece {
    let entries = Signal::new(Vec::<(u64, i64)>::new());
    let next_id = Signal::new(0u64);
    watch(
        move || count.get(),
        move |new, _old| {
            let id = next_id.get_untracked();
            next_id.set(id + 1);
            let v = *new;
            entries.update(|e| {
                e.push((id, v));
                if e.len() > 8 {
                    e.remove(0);
                }
            });
        },
    );
    column((
        label(tr("history-title")).font(Font::Headline),
        each(
            move || entries.get(),
            |e| e.0,
            move |slot: ItemSlot<(u64, i64), u64>| {
                label(move || {
                    tr("history-entry")
                        .arg("value", slot.field(|t| t.1))
                        .format()
                })
            },
        ),
    ))
    .spacing(4.0)
    .align(HAlign::Leading)
    .any()
}

// Mobile entries (DESIGN.md §17.4): the iOS Runner binds `day_main`, DayBridge binds the
// `Java_…` natives. Both macros emit nothing off their target OS.
day::ios_main!("Day Showcase", root);
day::android_main!(root);
