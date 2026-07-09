use day::prelude::*;
use day_piece_picker::picker;

/// Picker pieces (docs/picker.md): one `picker` bound two-way to a `Signal<usize>`, in all three
/// SwiftUI-style stylings — each a distinct NATIVE control. A live label mirrors each selection.
pub(crate) fn pickers_page() -> AnyPiece {
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
