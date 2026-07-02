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
    nav(tr("app-title"), home_page())
        .route("controls", tr("nav-controls"), controls_page)
        .route("gauge", tr("nav-gauge"), gauge_page)
        .route("about", tr("nav-about"), about_page)
        .id("nav")
}

/// Root content: the sidebar on desktop, the first stack page on mobile. The route
/// list renders NATIVELY via nav_menu() — NSOutlineView source list, GtkListBox
/// navigation-sidebar, QListWidget, chevroned UITableView rows, Android ripple rows.
fn home_page() -> AnyPiece {
    column((
        row((
            image("day-logo.png").frame(28.0, 28.0),
            label(tr("app-title")).font(Font::Headline).id("home-title"),
        ))
        .spacing(8.0)
        .padding(12.0),
        nav_menu().id("nav-menu"),
    ))
    .spacing(4.0)
    .align(HAlign::Leading)
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
