use day::prelude::*;

/// Native file open/save pickers (docs/files.md). Both buttons open a native picker from an async
/// task; the chosen file crosses back as a `FileUrl`. "Open" reads the file into the editor;
/// "Save" writes the editor's text out. Status tokens are locale-independent so the walkthrough
/// can assert them.
pub(crate) fn files_page() -> AnyPiece {
    // The editor text: what "Save" writes and what "Open" loads into.
    let content = Signal::new(String::from("Hello from Day!\nEdit me, then Save."));
    let status = Signal::new(String::new());
    let opened = Signal::new(String::new());
    column((
        label(tr("nav-files")).font(Font::Title).id("files-title"),
        label(tr("files-caption")),
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
        ))
        .spacing(8.0),
        divider(),
        when(
            move || !opened.with(|s| s.is_empty()),
            move || label(tr("files-opened").arg("name", opened)).id("files-opened-name"),
        ),
        label(move || status.get()).id("files-status"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
