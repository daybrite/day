use day::prelude::*;
use day_part_haptics::Haptic;

use crate::widgets::page;

/// Platform services (docs/clipboard.md, docs/prefs.md, docs/haptics.md, docs/files.md): the
/// headless "do something with the OS" parts, one grouped form section each — clipboard
/// round-trip, persisted preferences, haptic feedback, and the native file pickers.
pub(crate) fn services_page() -> AnyPiece {
    page(
        tr("nav-services"),
        "services-title",
        Some(tr("services-caption")),
        form((
            clipboard_section(),
            prefs_section(),
            haptics_section(),
            files_section(),
        ))
        .any(),
    )
}

fn clipboard_section() -> impl Piece {
    let draft = Signal::new(String::new());
    let pasted = Signal::new(String::new());
    let status = Signal::new(tr("clipboard-idle").format());
    section((
        label(tr("clipboard-caption")).font(Font::Footnote),
        text_field(draft)
            .placeholder(tr("clipboard-placeholder"))
            .id("clipboard-field"),
        row((
            button(tr("clipboard-copy"))
                .action(move || {
                    let ok = draft.with(|t| day_part_clipboard::set_text(t));
                    let msg = if ok {
                        tr("clipboard-copied")
                    } else {
                        tr("clipboard-copy-failed")
                    };
                    status.set(msg.format());
                })
                .id("clipboard-copy"),
            button(tr("clipboard-paste"))
                .action(move || match day_part_clipboard::get_text() {
                    Some(text) => {
                        pasted.set(text);
                        status.set(tr("clipboard-pasted").format());
                    }
                    None => status.set(tr("clipboard-empty").format()),
                })
                .id("clipboard-paste"),
            label(move || status.get()).id("clipboard-status"),
        ))
        .spacing(8.0),
        label(move || pasted.get()).id("clipboard-pasted"),
    ))
    .title(tr("nav-clipboard"))
}

fn prefs_section() -> impl Piece {
    const KEY: &str = "showcase.remembered";
    let field = Signal::new(String::new());
    let value = Signal::new(tr("prefs-empty").format());
    let status = Signal::new(tr("prefs-idle").format());
    section((
        label(tr("prefs-caption")).font(Font::Footnote),
        text_field(field)
            .placeholder(tr("prefs-placeholder"))
            .id("prefs-field"),
        row((
            button(tr("prefs-save"))
                .action(move || {
                    let ok = field.with(|t| day_part_prefs::set(KEY, t));
                    let msg = if ok {
                        tr("prefs-saved")
                    } else {
                        tr("prefs-save-failed")
                    };
                    status.set(msg.format());
                })
                .id("prefs-save"),
            button(tr("prefs-load"))
                .action(move || match day_part_prefs::get(KEY) {
                    Some(v) => {
                        value.set(v);
                        status.set(tr("prefs-loaded").format());
                    }
                    None => {
                        value.set(tr("prefs-empty").format());
                        status.set(tr("prefs-missing").format());
                    }
                })
                .id("prefs-load"),
            button(tr("prefs-clear"))
                .action(move || {
                    day_part_prefs::remove(KEY);
                    value.set(tr("prefs-empty").format());
                    status.set(tr("prefs-cleared").format());
                })
                .id("prefs-clear"),
            label(move || status.get()).id("prefs-status"),
        ))
        .spacing(8.0),
        labeled(
            tr("prefs-value-label"),
            label(move || value.get()).id("prefs-value"),
        ),
    ))
    .title(tr("nav-prefs"))
}

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

fn haptics_section() -> impl Piece {
    let last = Signal::new(tr("haptics-none").format());
    // Report whether this platform has a haptic engine (each branch a full `tr(...)` for `day lint`).
    let supported = if day_part_haptics::is_supported() {
        tr("haptics-supported-yes")
    } else {
        tr("haptics-supported-no")
    };
    section((
        label(supported)
            .font(Font::Footnote)
            .id("haptics-supported"),
        row((
            haptic_button("haptics-light", tr("haptics-light"), Haptic::Light, last),
            haptic_button("haptics-medium", tr("haptics-medium"), Haptic::Medium, last),
            haptic_button("haptics-heavy", tr("haptics-heavy"), Haptic::Heavy, last),
        ))
        .spacing(8.0),
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
            haptic_button(
                "haptics-selection",
                tr("haptics-selection"),
                Haptic::Selection,
                last,
            ),
        ))
        .spacing(8.0),
        labeled(
            tr("haptics-last"),
            label(move || last.get()).id("haptics-last-played"),
        ),
    ))
    .title(tr("nav-haptics"))
}

fn files_section() -> impl Piece {
    // The editor text: what "Save" writes and what "Open" loads into.
    let content = Signal::new(String::from("Hello from Day!\nEdit me, then Save."));
    let status = Signal::new(String::new());
    let opened = Signal::new(String::new());
    section((
        label(tr("files-caption")).font(Font::Footnote),
        text_field(content)
            .placeholder(tr("files-placeholder"))
            .id("files-content"),
        row((
            button(tr("files-open"))
                .action(move || {
                    day::task(async move {
                        match open_file()
                            .title(tr("files-open"))
                            .filter("Text", &["txt", "md"])
                            .await
                        {
                            Some(file) => match file.read_to_string() {
                                Ok(text) => {
                                    content.set(text);
                                    opened.set(file.file_name().unwrap_or_default());
                                    status.set("opened".into());
                                }
                                Err(_) => status.set("open-error".into()),
                            },
                            None => status.set("open-cancel".into()),
                        }
                    })
                })
                .id("btn-open-file"),
            button(tr("files-save"))
                .action(move || {
                    day::task(async move {
                        let data = content.get_untracked().into_bytes();
                        match save_file(data)
                            .title(tr("files-save"))
                            .suggested_name("day-notes.txt")
                            .filter("Text", &["txt"])
                            .await
                        {
                            Some(dest) => status
                                .set(format!("saved:{}", dest.file_name().unwrap_or_default())),
                            None => status.set("save-cancel".into()),
                        }
                    })
                })
                .id("btn-save-file"),
            label(move || status.get()).id("files-status"),
        ))
        .spacing(8.0),
        when(
            move || !opened.with(|s| s.is_empty()),
            move || label(tr("files-opened").arg("name", opened)).id("files-opened-name"),
        ),
    ))
    .title(tr("nav-files"))
}
