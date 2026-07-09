use day::prelude::*;

use crate::widgets::gauge;

/// Canvas gauge (§11) driven by its own slider.
pub(crate) fn gauge_page() -> AnyPiece {
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
