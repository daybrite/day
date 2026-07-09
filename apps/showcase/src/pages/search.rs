use day::prelude::*;
use day_piece_searchfield::search_field;

pub(crate) fn search_page() -> AnyPiece {
    let query = Signal::new(String::new());
    column((
        label(tr("nav-search")).font(Font::Title).id("search-title"),
        label(tr("search-caption")),
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
            search_fruit_row(query, "Apple"),
            search_fruit_row(query, "Banana"),
            search_fruit_row(query, "Cherry"),
            search_fruit_row(query, "Date"),
            search_fruit_row(query, "Elderberry"),
        ))
        .spacing(4.0)
        .align(HAlign::Leading),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

const SEARCH_FRUITS: [&str; 5] = ["Apple", "Banana", "Cherry", "Date", "Elderberry"];

/// Case-insensitive substring match; an empty query matches everything.
fn search_matches(query: &str, fruit: &str) -> bool {
    query.is_empty() || fruit.to_lowercase().contains(&query.to_lowercase())
}

/// The first fruit matching `query` (a data value), or an em-dash when none match.
fn search_first_match(query: &str) -> String {
    for fruit in SEARCH_FRUITS {
        if search_matches(query, fruit) {
            return fruit.to_string();
        }
    }
    "\u{2014}".to_string()
}

/// One filtered row: a `when`-gated label that appears only while its fruit matches the query.
fn search_fruit_row(query: Signal<String>, fruit: &'static str) -> AnyPiece {
    when(
        move || search_matches(&query.get(), fruit),
        move || label(fruit),
    )
}
