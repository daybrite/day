use day::prelude::*;

/// One of each basic control, two-way bound to a signal with a live readout
/// (https://daybrite.dev/docs/pieces). Set the signal and the control moves; move the
/// control and the signal (and everything reading it) updates.
pub(crate) fn controls_page() -> AnyPiece {
    let on = Signal::new(true);
    let level = Signal::new(35.0f64);
    let text = Signal::new(String::new());
    column((
        label(tr("controls-title"))
            .font(Font::Title)
            .id("controls-title"),
        row((
            toggle(on).id("toggle"),
            label(move || format!("{}", on.get())).id("toggle-value"),
        ))
        .spacing(8.0),
        row((
            slider(level).range(0.0..=100.0).id("slider"),
            label(move || format!("{:.0}", level.get())).id("slider-value"),
        ))
        .spacing(8.0),
        text_field(text)
            .placeholder(tr("controls-field-placeholder"))
            .id("field"),
        row((
            label(tr("controls-echo")),
            label(move || text.get()).id("echo"),
        ))
        .spacing(8.0),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
