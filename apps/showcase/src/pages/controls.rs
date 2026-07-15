use day::prelude::*;
use day_piece_activity::activity;
use day_piece_combobox::combo_box;
use day_piece_picker::picker;
use day_piece_searchfield::search_field;

use crate::widgets::{history, page};

/// Every interactive control, grouped as a form (docs/forms.md) with stable ids for the
/// walkthrough (§14): the basics (counter, text, slider, toggle), the picker stylings, search,
/// and progress/activity feedback — each in its own themed section, labels aligned form-wide.
pub(crate) fn controls_page() -> AnyPiece {
    page(
        crate::res::str::nav_controls(),
        "controls-title",
        Some(crate::res::str::controls_caption()),
        form((
            basics_section(),
            pickers_section(),
            search_section(),
            feedback_section(),
        ))
        .any(),
    )
}

fn basics_section() -> impl Piece {
    let count = Signal::new(0i64);
    let name = Signal::new(String::new());
    let volume = Signal::new(40.0f64);
    let subscribed = Signal::new(false);
    let flavors = Signal::new(vec![
        crate::res::str::vanilla().format(),
        crate::res::str::chocolate().format(),
        crate::res::str::pistachio().format(),
    ]);
    let flavor = Signal::new(Some(0usize));
    section((
        // — state: counter —
        row((
            // The buttons log to the two standard streams (stderr / stdout) so
            // `day launch` can demonstrate forwarding both, per platform.
            button(crate::res::str::decrement())
                .bordered()
                .action(move || {
                    count.update(|c| *c -= 1);
                    eprintln!("counter decremented to {}", count.get_untracked());
                })
                .id("decrement-button"),
            label(crate::res::str::counter_value(count)).id("counter-label"),
            button(crate::res::str::increment())
                .prominent()
                .action(move || {
                    count.update(|c| *c += 1);
                    println!("counter incremented to {}", count.get_untracked());
                })
                .id("increment-button"),
        ))
        .spacing(8.0),
        // — text input + conditional —
        text_field(name)
            .placeholder(crate::res::str::name_placeholder())
            .id("name-field"),
        when(
            move || !name.with(|s| s.is_empty()),
            move || label(crate::res::str::greeting(name)).id("greeting-label"),
        ),
        // — slider with live readout —
        labeled(
            crate::res::str::volume_label(),
            row((
                slider(volume).range(0.0..=100.0).id("volume-slider"),
                label(move || format!("{:.0}", volume.get())).id("volume-value"),
            ))
            .spacing(8.0),
        ),
        labeled(
            crate::res::str::subscribe_label(),
            toggle(subscribed)
                .id("subscribe-toggle")
                .a11y(|a| a.label("Subscribe to updates")), // a11y strings localize at M6.5
        ),
        // — an EXTERNAL Day Piece, registered like any built-in (§8.2, DP-21) —
        labeled(
            crate::res::str::flavor_label(),
            row((
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
        ),
        // — keyed collection (watch + monotonic keys, §5.4 / A.1) —
        history(count),
    ))
    .title(crate::res::str::controls_basics())
}

fn pickers_section() -> impl Piece {
    // ONE selection signal behind all three stylings (docs/picker.md): every native control is a
    // two-way projection of the same state, so changing any one of them moves the other two (and
    // the per-row readouts) natively — the walkthrough selects via each styling in turn and
    // asserts the others follow.
    let choice = Signal::new(1usize);
    // Localized option list (resolved once at build — the locale is fixed for the run) shared
    // by all three picker stylings and the per-row readouts.
    let sizes = std::rc::Rc::new(vec![
        crate::res::str::size_small().format(),
        crate::res::str::size_medium().format(),
        crate::res::str::size_large().format(),
    ]);
    let value = {
        let sizes = sizes.clone();
        move || sizes[choice.get().min(2)].clone()
    };
    section((
        label(crate::res::str::picker_shared_caption()).font(Font::Footnote),
        // Segmented — a horizontal one-of-N control. (No per-row readout: the shared state is
        // already visible in the other two stylings, and one quiet readout below serves all.)
        labeled(
            crate::res::str::picker_segmented(),
            picker(sizes.iter().cloned(), choice)
                .segmented()
                .id("picker-segmented"),
        ),
        // Menu — a pop-up / dropdown.
        labeled(
            crate::res::str::picker_menu(),
            picker(sizes.iter().cloned(), choice)
                .menu()
                .id("picker-menu"),
        ),
        // Inline — a vertical radio group.
        labeled(
            crate::res::str::picker_inline(),
            picker(sizes.iter().cloned(), choice)
                .inline()
                .id("picker-inline"),
        ),
        // The one shared readout the walkthrough asserts after driving each styling.
        labeled(
            crate::res::str::picker_selected(),
            label(value).id("picker-value"),
        ),
    ))
    .title(crate::res::str::nav_pickers())
}

fn search_section() -> impl Piece {
    let query = Signal::new(String::new());
    section((
        // A native search field bound two-way to `query` + a Clear button that sets it to ""
        // (proving the reverse binding patches the native control).
        row((
            search_field(query)
                .placeholder(crate::res::str::search_placeholder())
                .id("search-input"),
            button(crate::res::str::search_clear())
                .bordered()
                .action(move || query.set(String::new()))
                .id("search-clear"),
        ))
        .spacing(8.0),
        // First match (a value, not prose) or an em-dash when nothing matches.
        label(move || search_first_match(&query.get())).id("search-result"),
        // The filtered fruit list — each row is a reactive `when`-gated label.
        column((
            search_fruit_row(query, crate::res::str::fruit_apple().format()),
            search_fruit_row(query, crate::res::str::fruit_banana().format()),
            search_fruit_row(query, crate::res::str::fruit_cherry().format()),
            search_fruit_row(query, crate::res::str::fruit_date().format()),
            search_fruit_row(query, crate::res::str::fruit_elderberry().format()),
        ))
        .spacing(4.0)
        .align(HAlign::Leading),
    ))
    .title(crate::res::str::nav_search())
}

fn feedback_section() -> impl Piece {
    let volume = Signal::new(65.0f64);
    // The spinner's running state is a Signal<bool> shared by the piece, the toggle, and a status
    // label that mirrors it reactively (each `tr(...)` branch is a full literal call for `day lint`).
    let spinning = Signal::new(true);
    let status = move || {
        if spinning.get() {
            crate::res::str::activity_on()
        } else {
            crate::res::str::activity_off()
        }
        .format()
    };
    section((
        labeled(
            crate::res::str::progress_label(),
            row((
                slider(volume).range(0.0..=100.0).id("progress-slider"),
                progress(move || volume.get() / 100.0)
                    .id("volume-progress")
                    .a11y(|a| a.role(Role::Meter).label("Volume level")),
            ))
            .spacing(8.0),
        ),
        labeled(crate::res::str::busy_label(), spinner().id("busy-spinner")),
        labeled(
            crate::res::str::activity_animating(),
            row((
                toggle(spinning).id("activity-toggle"),
                activity().animating(spinning).id("activity-spinner"),
                label(status).id("activity-status"),
            ))
            .spacing(8.0),
        ),
    ))
    .title(crate::res::str::controls_feedback())
}

/// The fruit list, localized (resolved once at build — the locale is fixed for the run).
fn search_fruits() -> Vec<String> {
    [
        "fruit_apple",
        "fruit_banana",
        "fruit_cherry",
        "fruit_date",
        "fruit_elderberry",
    ]
    .iter()
    .map(|k| tr(k).format())
    .collect()
}

/// Case-insensitive substring match; an empty query matches everything.
fn search_matches(query: &str, fruit: &str) -> bool {
    query.is_empty() || fruit.to_lowercase().contains(&query.to_lowercase())
}

/// The first fruit matching `query` (a data value), or an em-dash when none match.
fn search_first_match(query: &str) -> String {
    for fruit in search_fruits() {
        if search_matches(query, &fruit) {
            return fruit;
        }
    }
    "\u{2014}".to_string()
}

/// One filtered row: a `when`-gated label that appears only while its fruit matches the query.
fn search_fruit_row(query: Signal<String>, fruit: String) -> AnyPiece {
    let shown = fruit.clone();
    when(
        move || search_matches(&query.get(), &fruit),
        move || label(shown.clone()),
    )
}
