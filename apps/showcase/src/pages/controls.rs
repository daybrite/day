use day::prelude::*;
use day_piece_combobox::combo_box;

use crate::widgets::history;

/// Every interactive control, with stable ids for the walkthrough (§14).
pub(crate) fn controls_page() -> AnyPiece {
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
                label(move || format!("{:.0}", volume.get())).id("volume-value"),
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
