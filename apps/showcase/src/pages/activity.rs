use day::prelude::*;
use day_piece_activity::activity;

pub(crate) fn activity_page() -> AnyPiece {
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
    column((
        label(tr("nav-activity"))
            .font(Font::Title)
            .id("activity-title"),
        label(tr("activity-caption")),
        row((
            activity().animating(spinning).id("activity-spinner"),
            label(status).id("activity-status"),
        ))
        .spacing(12.0),
        row((
            label(tr("activity-animating")),
            toggle(spinning).id("activity-toggle"),
        ))
        .spacing(8.0),
        divider(),
        // A separate, always-animating large spinner keeps a visible spinning indicator on screen
        // regardless of the toggle (nice for the walkthrough screenshot).
        label(tr("activity-large-label")).font(Font::Headline),
        activity().large(true).id("activity-large"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
