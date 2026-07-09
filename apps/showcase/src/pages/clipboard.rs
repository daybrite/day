use day::prelude::*;

/// Clipboard playground (docs/clipboard.md): the headless `day-part-clipboard` part round-trips
/// plain text through the system clipboard — type, Copy, then Paste reads it back natively.
pub(crate) fn clipboard_page() -> AnyPiece {
    let draft = Signal::new(String::new());
    let pasted = Signal::new(String::new());
    let status = Signal::new(tr("clipboard-idle").format());
    column((
        label(tr("nav-clipboard"))
            .font(Font::Title)
            .id("clipboard-title"),
        label(tr("clipboard-caption")),
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
        ))
        .spacing(8.0),
        label(move || status.get()).id("clipboard-status"),
        label(move || pasted.get()).id("clipboard-pasted"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
