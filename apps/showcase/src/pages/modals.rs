use day::prelude::*;

use crate::widgets::heading;

/// Imperative modals (docs/dialogs.md): each button opens a native dialog from within an
/// async task and writes a fixed result token to `modal-result` (locale-independent so the
/// walkthrough can assert it).
pub(crate) fn modals_page() -> AnyPiece {
    let last = Signal::new(String::new());
    column((
        heading(crate::res::str::nav_modals(), "modals-title", None),
        button(crate::res::str::modal_alert())
            .bordered()
            .action(move || {
                day::task(async move {
                    alert(crate::res::str::alert_title())
                        .message(crate::res::str::alert_body())
                        .button(crate::res::str::ok(), ())
                        .present()
                        .await;
                    last.set("alert-ok".into());
                })
            })
            .id("btn-alert"),
        button(crate::res::str::modal_confirm())
            .bordered()
            .action(move || {
                day::task(async move {
                    let ok = confirm(crate::res::str::confirm_title())
                        .message(crate::res::str::confirm_body())
                        .await;
                    last.set(if ok { "confirm-yes" } else { "confirm-no" }.into());
                })
            })
            .id("btn-confirm"),
        button(crate::res::str::modal_delete())
            .bordered()
            .action(move || {
                day::task(async move {
                    let ok = confirm(crate::res::str::delete_title())
                        .message(crate::res::str::delete_body())
                        .confirm_label(crate::res::str::delete())
                        .destructive()
                        .await;
                    last.set(if ok { "delete-yes" } else { "delete-no" }.into());
                })
            })
            .id("btn-delete"),
        button(crate::res::str::modal_sheet())
            .bordered()
            .action(move || {
                day::task(async move {
                    let choice = Alert::new(crate::res::str::flavor_title())
                        .sheet()
                        .button(crate::res::str::vanilla(), 0i64)
                        .button(crate::res::str::pistachio(), 1i64)
                        .cancel(crate::res::str::cancel())
                        .present()
                        .await;
                    last.set(match choice {
                        Some(i) => format!("sheet-{i}"),
                        None => "sheet-cancel".into(),
                    });
                })
            })
            .id("btn-sheet"),
        button(crate::res::str::modal_prompt())
            .bordered()
            .action(move || {
                day::task(async move {
                    let name = prompt(crate::res::str::name_placeholder()).await;
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
