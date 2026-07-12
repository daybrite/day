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
        tr("nav-controls"),
        "controls-title",
        Some(tr("controls-caption")),
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
        tr("vanilla").format(),
        tr("chocolate").format(),
        tr("pistachio").format(),
    ]);
    let flavor = Signal::new(Some(0usize));
    section((
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
        // — text input + conditional —
        text_field(name)
            .placeholder(tr("name-placeholder"))
            .id("name-field"),
        when(
            move || !name.with(|s| s.is_empty()),
            move || label(tr("greeting").arg("name", name)).id("greeting-label"),
        ),
        // — slider with live readout —
        labeled(
            tr("volume-label"),
            row((
                slider(volume).range(0.0..=100.0).id("volume-slider"),
                label(move || format!("{:.0}", volume.get())).id("volume-value"),
            ))
            .spacing(8.0),
        ),
        labeled(
            tr("subscribe-label"),
            toggle(subscribed)
                .id("subscribe-toggle")
                .a11y(|a| a.label("Subscribe to updates")), // a11y strings localize at M6.5
        ),
        // — an EXTERNAL Day Piece, registered like any built-in (§8.2, DP-21) —
        labeled(
            tr("flavor-label"),
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
    .title(tr("controls-basics"))
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
        tr("size-small").format(),
        tr("size-medium").format(),
        tr("size-large").format(),
    ]);
    let value = {
        let sizes = sizes.clone();
        move || sizes[choice.get().min(2)].clone()
    };
    section((
        label(tr("picker-shared-caption")).font(Font::Footnote),
        // Segmented — a horizontal one-of-N control.
        labeled(
            tr("picker-segmented"),
            row((
                picker(sizes.iter().cloned(), choice)
                    .segmented()
                    .id("picker-segmented"),
                label(value.clone()).id("picker-segmented-value"),
            ))
            .spacing(8.0),
        ),
        // Menu — a pop-up / dropdown.
        labeled(
            tr("picker-menu"),
            row((
                picker(sizes.iter().cloned(), choice)
                    .menu()
                    .id("picker-menu"),
                label(value.clone()).id("picker-menu-value"),
            ))
            .spacing(8.0),
        ),
        // Inline — a vertical radio group.
        labeled(
            tr("picker-inline"),
            row((
                picker(sizes.iter().cloned(), choice)
                    .inline()
                    .id("picker-inline"),
                label(value).id("picker-inline-value"),
            ))
            .spacing(8.0),
        ),
    ))
    .title(tr("nav-pickers"))
}

fn search_section() -> impl Piece {
    let query = Signal::new(String::new());
    section((
        // A native search field bound two-way to `query` + a Clear button that sets it to ""
        // (proving the reverse binding patches the native control).
        row((
            search_field(query)
                .placeholder(tr("search-placeholder"))
                .id("search-input"),
            button(tr("search-clear"))
                .action(move || query.set(String::new()))
                .id("search-clear"),
        ))
        .spacing(8.0),
        // First match (a value, not prose) or an em-dash when nothing matches.
        label(move || search_first_match(&query.get())).id("search-result"),
        // The filtered fruit list — each row is a reactive `when`-gated label.
        column((
            search_fruit_row(query, tr("fruit-apple").format()),
            search_fruit_row(query, tr("fruit-banana").format()),
            search_fruit_row(query, tr("fruit-cherry").format()),
            search_fruit_row(query, tr("fruit-date").format()),
            search_fruit_row(query, tr("fruit-elderberry").format()),
        ))
        .spacing(4.0)
        .align(HAlign::Leading),
    ))
    .title(tr("nav-search"))
}

fn feedback_section() -> impl Piece {
    let volume = Signal::new(65.0f64);
    // The spinner's running state is a Signal<bool> shared by the piece, the toggle, and a status
    // label that mirrors it reactively (each `tr(...)` branch is a full literal call for `day lint`).
    let spinning = Signal::new(true);
    let status = move || {
        if spinning.get() {
            tr("activity-on")
        } else {
            tr("activity-off")
        }
        .format()
    };
    section((
        labeled(
            tr("progress-label"),
            row((
                slider(volume).range(0.0..=100.0).id("progress-slider"),
                progress(move || volume.get() / 100.0)
                    .id("volume-progress")
                    .a11y(|a| a.role(Role::Meter).label("Volume level")),
            ))
            .spacing(8.0),
        ),
        labeled(tr("busy-label"), spinner().id("busy-spinner")),
        labeled(
            tr("activity-animating"),
            row((
                toggle(spinning).id("activity-toggle"),
                activity().animating(spinning).id("activity-spinner"),
                label(status).id("activity-status"),
            ))
            .spacing(8.0),
        ),
    ))
    .title(tr("controls-feedback"))
}

/// The fruit list, localized (resolved once at build — the locale is fixed for the run).
fn search_fruits() -> Vec<String> {
    [
        "fruit-apple",
        "fruit-banana",
        "fruit-cherry",
        "fruit-date",
        "fruit-elderberry",
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
