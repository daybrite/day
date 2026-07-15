use day::prelude::*;
use day_part_haptics::Haptic;

use crate::widgets::page;

/// Platform services (docs/clipboard.md, docs/prefs.md, docs/haptics.md, docs/files.md): the
/// headless "do something with the OS" parts, one grouped form section each — clipboard
/// round-trip, persisted preferences, haptic feedback, and the native file pickers.
pub(crate) fn services_page() -> AnyPiece {
    page(
        crate::res::str::nav_services(),
        "services-title",
        Some(crate::res::str::services_caption()),
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
    let status = Signal::new(crate::res::str::clipboard_idle().format());
    section((
        label(crate::res::str::clipboard_caption()).font(Font::Footnote),
        text_field(draft)
            .placeholder(crate::res::str::clipboard_placeholder())
            .id("clipboard-field"),
        row((
            button(crate::res::str::clipboard_copy())
                .bordered()
                .action(move || {
                    let ok = draft.with(|t| day_part_clipboard::set_text(t));
                    let msg = if ok {
                        crate::res::str::clipboard_copied()
                    } else {
                        crate::res::str::clipboard_copy_failed()
                    };
                    status.set(msg.format());
                })
                .id("clipboard-copy"),
            button(crate::res::str::clipboard_paste())
                .bordered()
                .action(move || match day_part_clipboard::get_text() {
                    Some(text) => {
                        pasted.set(text);
                        status.set(crate::res::str::clipboard_pasted().format());
                    }
                    None => status.set(crate::res::str::clipboard_empty().format()),
                })
                .id("clipboard-paste"),
            label(move || status.get()).id("clipboard-status"),
        ))
        .spacing(8.0),
        label(move || pasted.get()).id("clipboard-pasted"),
    ))
    .title(crate::res::str::nav_clipboard())
}

fn prefs_section() -> impl Piece {
    const KEY: &str = "showcase.remembered";
    let field = Signal::new(String::new());
    let value = Signal::new(crate::res::str::prefs_empty().format());
    let status = Signal::new(crate::res::str::prefs_idle().format());
    section((
        label(crate::res::str::prefs_caption()).font(Font::Footnote),
        text_field(field)
            .placeholder(crate::res::str::prefs_placeholder())
            .id("prefs-field"),
        row((
            button(crate::res::str::prefs_save())
                .bordered()
                .action(move || {
                    let ok = field.with(|t| day_part_prefs::set(KEY, t));
                    let msg = if ok {
                        crate::res::str::prefs_saved()
                    } else {
                        crate::res::str::prefs_save_failed()
                    };
                    status.set(msg.format());
                })
                .id("prefs-save"),
            button(crate::res::str::prefs_load())
                .bordered()
                .action(move || match day_part_prefs::get(KEY) {
                    Some(v) => {
                        value.set(v);
                        status.set(crate::res::str::prefs_loaded().format());
                    }
                    None => {
                        value.set(crate::res::str::prefs_empty().format());
                        status.set(crate::res::str::prefs_missing().format());
                    }
                })
                .id("prefs-load"),
            button(crate::res::str::prefs_clear())
                .bordered()
                .action(move || {
                    day_part_prefs::remove(KEY);
                    value.set(crate::res::str::prefs_empty().format());
                    status.set(crate::res::str::prefs_cleared().format());
                })
                .id("prefs-clear"),
            label(move || status.get()).id("prefs-status"),
        ))
        .spacing(8.0),
        labeled(
            crate::res::str::prefs_value_label(),
            label(move || value.get()).id("prefs-value"),
        ),
    ))
    .title(crate::res::str::nav_prefs())
}

/// One button that plays a haptic and records the style name into `#haptics-last-played`.
fn haptic_button(
    id: &'static str,
    title: LocalizedText,
    h: Haptic,
    last: Signal<String>,
) -> AnyPiece {
    button(title)
        .bordered()
        .action(move || {
            day_part_haptics::play(h);
            last.set(crate::res::str::haptics_last_played(format!("{h:?}")).format());
        })
        .id(id)
        .any()
}

fn haptics_section() -> impl Piece {
    let last = Signal::new(crate::res::str::haptics_none().format());
    // Report whether this platform has a haptic engine (each branch a full `tr(...)` for `day lint`).
    let supported = if day_part_haptics::is_supported() {
        crate::res::str::haptics_supported_yes()
    } else {
        crate::res::str::haptics_supported_no()
    };
    section((
        label(supported)
            .font(Font::Footnote)
            .id("haptics-supported"),
        row((
            haptic_button(
                "haptics-light",
                crate::res::str::haptics_light(),
                Haptic::Light,
                last,
            ),
            haptic_button(
                "haptics-medium",
                crate::res::str::haptics_medium(),
                Haptic::Medium,
                last,
            ),
            haptic_button(
                "haptics-heavy",
                crate::res::str::haptics_heavy(),
                Haptic::Heavy,
                last,
            ),
        ))
        .spacing(8.0),
        row((
            haptic_button(
                "haptics-success",
                crate::res::str::haptics_success(),
                Haptic::Success,
                last,
            ),
            haptic_button(
                "haptics-warning",
                crate::res::str::haptics_warning(),
                Haptic::Warning,
                last,
            ),
            haptic_button(
                "haptics-error",
                crate::res::str::haptics_error(),
                Haptic::Error,
                last,
            ),
            haptic_button(
                "haptics-selection",
                crate::res::str::haptics_selection(),
                Haptic::Selection,
                last,
            ),
        ))
        .spacing(8.0),
        labeled(
            crate::res::str::haptics_last(),
            label(move || last.get()).id("haptics-last-played"),
        ),
    ))
    .title(crate::res::str::nav_haptics())
}

fn files_section() -> impl Piece {
    // The editor text: what "Save" writes and what "Open" loads into.
    let content = Signal::new(String::from("Hello from Day!\nEdit me, then Save."));
    let status = Signal::new(String::new());
    let opened = Signal::new(String::new());
    section((
        label(crate::res::str::files_caption()).font(Font::Footnote),
        text_field(content)
            .placeholder(crate::res::str::files_placeholder())
            .id("files-content"),
        row((
            button(crate::res::str::files_open())
                .bordered()
                .action(move || {
                    day::task(async move {
                        match open_file()
                            .title(crate::res::str::files_open())
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
            button(crate::res::str::files_save())
                .bordered()
                .action(move || {
                    day::task(async move {
                        let data = content.get_untracked().into_bytes();
                        match save_file(data)
                            .title(crate::res::str::files_save())
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
            move || label(crate::res::str::files_opened(opened)).id("files-opened-name"),
        ),
    ))
    .title(crate::res::str::nav_files())
}
