//! The day showcase (DESIGN.md Appendix A, staged): every implemented piece behind a
//! native navigation host (docs/navigation.md) — stack presentation on mobile, sidebar +
//! detail split on desktop. Three destinations: controls, gauge, about.

use day::prelude::*;
use day_piece_combobox::combo_box;
use day_piece_picker::picker;
use day_piece_webview::web_view;

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
        .item("shapes", tr("nav-shapes"), shapes_page)
        .item("pickers", tr("nav-pickers"), pickers_page)
        .item("modals", tr("nav-modals"), modals_page)
        .item("tabs", tr("nav-tabs"), tabs_page)
        .item("stack", tr("nav-stack"), stack_page)
        .item("list", tr("nav-list"), list_page)
        .item("webview", tr("nav-webview"), webview_page)
        .item("about", tr("nav-about"), about_page)
        .id("nav")
}

/// A native recycling list (docs/list.md): 500 rows, but only the visible cells are ever built —
/// the platform's NSTableView / RecyclerView / GtkListView / QListView owns scrolling + reuse.
fn list_page() -> AnyPiece {
    let count = Signal::new(500i64);
    column((
        row((
            label(tr("nav-list")).font(Font::Title).id("list-title"),
            spacer(),
            button(tr("list-add"))
                .action(move || count.update(|c| *c += 100))
                .id("list-add"),
        )),
        label(tr("list-caption").arg("count", count)).id("list-caption"),
        list(
            move || {
                (1..=count.get())
                    .map(|i| format!("Row {i}"))
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

/// Shape pieces (docs/shapes.md): the unified `shape` piece rendered atop the canvas — every
/// kind, fill/stroke, a slider-bound rotation, tap-to-recolor, and drag-to-move. Shapes bind to
/// signals and transform through the canvas CTM, so all of this is free reactivity + zero backend
/// geometry code.
fn shapes_page() -> AnyPiece {
    let angle = Signal::new(0.0f64);
    let tapped = Signal::new(false);
    let pos = Signal::new((0.0f64, 0.0f64));
    let base = Signal::new((0.0f64, 0.0f64));
    column((
        label(tr("nav-shapes")).font(Font::Title).id("shapes-title"),
        // Every shape kind: fills and strokes (two rows so all fit the split-detail pane).
        label(tr("shapes-kinds")).font(Font::Headline),
        row((
            rectangle()
                .fill(Color::hex(0x2F6FDE))
                .frame(56.0, 56.0)
                .id("shape-rect"),
            rounded_rectangle(12.0)
                .fill(Color::hex(0x8E44AD))
                .frame(56.0, 56.0)
                .id("shape-rrect"),
            circle()
                .fill(Color::hex(0x27AE60))
                .frame(56.0, 56.0)
                .id("shape-circle"),
        ))
        .spacing(12.0),
        row((
            capsule()
                .fill(Color::hex(0xE67E22))
                .frame(76.0, 40.0)
                .id("shape-capsule"),
            ellipse()
                .stroke(Color::hex(0xC0392B), 4.0)
                .frame(76.0, 48.0)
                .id("shape-ellipse"),
            arc(135.0, 270.0)
                .stroke(Color::hex(0x16A085), 6.0)
                .frame(56.0, 56.0)
                .id("shape-arc"),
        ))
        .spacing(12.0),
        // A rounded rectangle rotated live by a slider (canvas CTM transform).
        label(tr("shapes-transform")).font(Font::Headline),
        row((
            label(tr("shapes-angle")),
            slider(angle).range(0.0..=360.0).id("shapes-angle-slider"),
        ))
        .spacing(8.0),
        rounded_rectangle(10.0)
            .fill(Color::hex(0x2F6FDE))
            .rotate(move || angle.get())
            // Inset so the rotated square's corners stay within the canvas frame (backends that
            // clip children to bounds — e.g. Qt — would otherwise shave the corners at an angle).
            .inset(20.0)
            .frame(120.0, 120.0)
            .id("shapes-rotator"),
        // Tap to recolor (path-precise hit-testing).
        label(tr("shapes-tap")).font(Font::Headline),
        circle()
            .fill(move || {
                if tapped.get() {
                    Color::hex(0xE74C3C)
                } else {
                    Color::hex(0x3498DB)
                }
            })
            .on_tap(move || tapped.update(|t| *t = !*t))
            // `.id` before `.frame` so the identifier lands on the shape leaf (the gesture target),
            // not the frame wrapper — lets dayscript/autodrive address the tap directly.
            .id("shapes-tap-circle")
            .frame(90.0, 90.0),
        // Drag to move (offset bound to the drag translation).
        label(tr("shapes-drag")).font(Font::Headline),
        rectangle()
            .fill(Color::hex(0x9B59B6))
            .offset(move || pos.get().0, move || pos.get().1)
            .on_drag(move |dr| match dr.phase {
                DragPhase::Began => base.set(pos.get_untracked()),
                _ => {
                    let b = base.get_untracked();
                    pos.set((b.0 + dr.translation.x, b.1 + dr.translation.y));
                }
            })
            .id("shapes-drag-rect")
            .frame(90.0, 90.0),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Picker pieces (docs/picker.md): one `picker` bound two-way to a `Signal<usize>`, in all three
/// SwiftUI-style stylings — each a distinct NATIVE control. A live label mirrors each selection.
fn pickers_page() -> AnyPiece {
    let size = Signal::new(1usize);
    let color = Signal::new(0usize);
    let plan = Signal::new(0usize);
    let sizes = ["Small", "Medium", "Large"];
    let colors = ["Red", "Green", "Blue"];
    let plans = ["Free", "Pro", "Team"];
    column((
        label(tr("nav-pickers"))
            .font(Font::Title)
            .id("pickers-title"),
        // Segmented — a horizontal one-of-N control.
        label(tr("picker-segmented")).font(Font::Headline),
        picker(sizes, size).segmented().id("picker-segmented"),
        label(move || sizes[size.get().min(2)].to_string()).id("picker-segmented-value"),
        // Menu — a pop-up / dropdown.
        label(tr("picker-menu")).font(Font::Headline),
        picker(colors, color).menu().id("picker-menu"),
        label(move || colors[color.get().min(2)].to_string()).id("picker-menu-value"),
        // Inline — a vertical radio group.
        label(tr("picker-inline")).font(Font::Headline),
        picker(plans, plan).inline().id("picker-inline"),
        label(move || plans[plan.get().min(2)].to_string()).id("picker-inline-value"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// A native web view (day-piece-webview, an EXTERNAL standalone piece): WKWebView / QWebEngineView /
/// android.webkit.WebView. The URL bar is bound two-way to the view — type + Go loads it, and
/// navigation reports the URL back so the field follows. Back/Forward/Stop/Reload drive history via
/// `Trigger`s the piece watches. The view fills the remaining space (a growing leaf).
fn webview_page() -> AnyPiece {
    let url = Signal::new("https://daybrite.dev".to_string());
    let go = Trigger::new();
    let back = Trigger::new();
    let forward = Trigger::new();
    let stop = Trigger::new();
    let reload = Trigger::new();
    column((
        label(tr("nav-webview"))
            .font(Font::Title)
            .id("webview-title"),
        // URL bar: the field is bound to the view's URL; Go loads whatever it holds.
        row((
            text_field(url)
                .placeholder(tr("webview-url-hint"))
                .id("webview-url"),
            button(tr("webview-go"))
                .action(move || go.notify())
                .id("webview-go"),
        ))
        .spacing(8.0),
        // History controls. "Stop" is the demo's cancel.
        row((
            button(tr("webview-back"))
                .action(move || back.notify())
                .id("webview-back"),
            button(tr("webview-forward"))
                .action(move || forward.notify())
                .id("webview-forward"),
            button(tr("webview-stop"))
                .action(move || stop.notify())
                .id("webview-stop"),
            button(tr("webview-reload"))
                .action(move || reload.notify())
                .id("webview-reload"),
        ))
        .spacing(8.0),
        web_view(url)
            .go(go)
            .back(back)
            .forward(forward)
            .stop(stop)
            .reload(reload)
            .id("webview"),
    ))
    .spacing(10.0)
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
    // Accessibility (§13): a canvas has no inherent role, so day applies `Meter` + a spoken value
    // and label. `.id`/`.a11y` go on the canvas leaf (before `.frame`, a handle-less layout node),
    // so they reach the native widget. Value is a build-time snapshot (reactive a11y is a follow-up).
    .a11y(move |a| {
        a.role(Role::Meter)
            .label(tr("volume-label").format())
            .value(format!("{:.0}", value.get_untracked()))
    })
    .id("gauge")
    .frame(110.0, 110.0)
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
