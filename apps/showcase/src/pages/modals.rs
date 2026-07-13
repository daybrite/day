use day::prelude::*;

use crate::widgets::heading;

/// Imperative modals (docs/dialogs.md): each button opens a native dialog from within an
/// async task and writes a fixed result token to `modal-result` (locale-independent so the
/// walkthrough can assert it).
pub(crate) fn modals_page() -> AnyPiece {
    let last = Signal::new(String::new());
    column((
        heading(tr("nav-modals"), "modals-title", None),
        button(tr("modal-alert"))
            .bordered()
            .action(move || {
                day::task(async move {
                    alert(tr("alert-title"))
                        .message(tr("alert-body"))
                        .button(tr("ok"), ())
                        .present()
                        .await;
                    last.set("alert-ok".into());
                })
            })
            .id("btn-alert"),
        button(tr("modal-confirm"))
            .bordered()
            .action(move || {
                day::task(async move {
                    let ok = confirm(tr("confirm-title"))
                        .message(tr("confirm-body"))
                        .await;
                    last.set(if ok { "confirm-yes" } else { "confirm-no" }.into());
                })
            })
            .id("btn-confirm"),
        button(tr("modal-delete"))
            .bordered()
            .action(move || {
                day::task(async move {
                    let ok = confirm(tr("delete-title"))
                        .message(tr("delete-body"))
                        .confirm_label(tr("delete"))
                        .destructive()
                        .await;
                    last.set(if ok { "delete-yes" } else { "delete-no" }.into());
                })
            })
            .id("btn-delete"),
        button(tr("modal-sheet"))
            .bordered()
            .action(move || {
                day::task(async move {
                    let choice = Alert::new(tr("flavor-title"))
                        .sheet()
                        .button(tr("vanilla"), 0i64)
                        .button(tr("pistachio"), 1i64)
                        .cancel(tr("cancel"))
                        .present()
                        .await;
                    last.set(match choice {
                        Some(i) => format!("sheet-{i}"),
                        None => "sheet-cancel".into(),
                    });
                })
            })
            .id("btn-sheet"),
        button(tr("modal-prompt"))
            .bordered()
            .action(move || {
                day::task(async move {
                    let name = prompt(tr("name-placeholder")).await;
                    last.set(match name {
                        Some(t) => format!("prompt-{t}"),
                        None => "prompt-none".into(),
                    });
                })
            })
            .id("btn-prompt"),
        divider(),
        label(move || last.get()).id("modal-result"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
