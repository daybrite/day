use day::prelude::*;
use day_piece_datetime::{DayDate, DayTime, date_picker, time_picker};

use crate::widgets::page;

/// Native date & time pickers (day-piece-datetime, an EXTERNAL standalone piece — the first with
/// renderers on all seven toolkits; docs/datepicker.md), grouped as a form with stable ids for
/// the walkthrough (§14): the date stylings (compact chooser, embedded calendar, bounded), the
/// time stylings (compact + seconds), and combined date+time as composition.
pub(crate) fn dates_page() -> AnyPiece {
    page(
        crate::res::str::nav_dates(),
        "dates-title",
        Some(crate::res::str::dates_caption()),
        form((date_section(), time_section(), composed_section())).any(),
    )
}

/// ONE civil date signal behind every date picker on the page (docs/datepicker.md): each native
/// control is a two-way projection of the same state — pick in the calendar and the compact field
/// follows, and vice versa. Seeded deterministically so the per-locale screenshot grid is
/// reproducible (the walkthrough drives changes via `input:` with ISO values).
fn seed_date() -> DayDate {
    DayDate::new(2026, 7, 18).expect("valid seed date")
}

fn date_section() -> impl Piece {
    let date = Signal::new(seed_date());
    // A separately-bounded picker: picks outside 2026 clamp (natively where the control supports
    // min/max, and always in the piece).
    let bounded = Signal::new(seed_date());
    section((
        // Compact — a field/button that summons the platform's transient chooser.
        labeled(
            crate::res::str::date_compact(),
            date_picker(date).id("date-compact"),
        ),
        // Inline — the embedded calendar (wheels on toolkits without a calendar grid).
        labeled(
            crate::res::str::date_inline(),
            date_picker(date).inline().id("date-inline"),
        ),
        // Bounded: min/max clamp every pick into 2026 (readout beside it, the volume pattern).
        labeled(
            crate::res::str::date_bounded(),
            row((
                date_picker(bounded)
                    .min(DayDate::new(2026, 1, 1).expect("valid min"))
                    .max(DayDate::new(2026, 12, 31).expect("valid max"))
                    .id("date-bounded"),
                label(move || bounded.get().to_string()).id("date-bounded-value"),
            ))
            .spacing(8.0),
        ),
        // Locale-independent ISO readout — what the walkthrough asserts on every backend.
        labeled(
            crate::res::str::date_picked(),
            label(move || date.get().to_string()).id("date-value"),
        ),
    ))
    .title(crate::res::str::dates_date_section())
}

fn time_section() -> impl Piece {
    // ONE time signal behind both time pickers: the seconds variant (honored natively on
    // AppKit/Qt; docs/datepicker.md) mirrors the compact one.
    let time = Signal::new(DayTime::new(9, 30, 0).expect("valid seed time"));
    section((
        labeled(
            crate::res::str::time_compact(),
            time_picker(time).id("time-compact"),
        ),
        labeled(
            crate::res::str::time_seconds(),
            time_picker(time).seconds(true).id("time-seconds"),
        ),
        labeled(
            crate::res::str::time_picked(),
            label(move || time.get().to_string()).id("time-value"),
        ),
    ))
    .title(crate::res::str::dates_time_section())
}

fn composed_section() -> impl Piece {
    // Combined date+time = COMPOSITION (a single combined control exists on only 3 of the 7
    // toolkits, so Day doesn't paper over it — docs/datepicker.md).
    let date = Signal::new(seed_date());
    let time = Signal::new(DayTime::new(18, 0, 0).expect("valid seed time"));
    section((labeled(
        crate::res::str::dates_composed(),
        row((date_picker(date), time_picker(time))).spacing(8.0),
    ),))
    .title(crate::res::str::dates_composed_section())
}
