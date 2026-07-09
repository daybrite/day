use day::prelude::*;
use day_part_haptics::Haptic;

/// One button that plays a haptic and records the style name into `#haptics-last-played`.
fn haptic_button(
    id: &'static str,
    title: LocalizedText,
    h: Haptic,
    last: Signal<String>,
) -> AnyPiece {
    button(title)
        .action(move || {
            day_part_haptics::play(h);
            last.set(
                tr("haptics-last-played")
                    .arg("style", format!("{h:?}"))
                    .format(),
            );
        })
        .id(id)
        .any()
}

/// Haptics playground (docs/haptics.md): the headless `day-part-haptics` part fires a native haptic
/// for each style; `#haptics-last-played` echoes the last one so the walkthrough can assert it.
pub(crate) fn haptics_page() -> AnyPiece {
    let last = Signal::new(tr("haptics-none").format());
    // Report whether this platform has a haptic engine (each branch a full `tr(...)` for `day lint`).
    let supported = if day_part_haptics::is_supported() {
        tr("haptics-supported-yes")
    } else {
        tr("haptics-supported-no")
    };
    column((
        label(tr("nav-haptics"))
            .font(Font::Title)
            .id("haptics-title"),
        label(tr("haptics-caption")),
        label(supported).id("haptics-supported"),
        // Impact intensities.
        row((
            haptic_button("haptics-light", tr("haptics-light"), Haptic::Light, last),
            haptic_button("haptics-medium", tr("haptics-medium"), Haptic::Medium, last),
            haptic_button("haptics-heavy", tr("haptics-heavy"), Haptic::Heavy, last),
        ))
        .spacing(8.0),
        // Notification outcomes.
        row((
            haptic_button(
                "haptics-success",
                tr("haptics-success"),
                Haptic::Success,
                last,
            ),
            haptic_button(
                "haptics-warning",
                tr("haptics-warning"),
                Haptic::Warning,
                last,
            ),
            haptic_button("haptics-error", tr("haptics-error"), Haptic::Error, last),
        ))
        .spacing(8.0),
        haptic_button(
            "haptics-selection",
            tr("haptics-selection"),
            Haptic::Selection,
            last,
        ),
        divider(),
        row((
            label(tr("haptics-last")),
            label(move || last.get()).id("haptics-last-played"),
        ))
        .spacing(8.0),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
