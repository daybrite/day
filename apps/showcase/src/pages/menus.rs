use day::prelude::*;
use std::cell::OnceCell;

use crate::widgets::page;

thread_local! {
    /// The last menu action fired — shared between the app menu (installed in `root`) and this
    /// page so both demonstrate action dispatch. Created lazily inside the reactive runtime.
    static MENU_LOG: OnceCell<Signal<String>> = const { OnceCell::new() };
}

fn menu_log() -> Signal<String> {
    MENU_LOG.with(|c| *c.get_or_init(|| Signal::new("—".into())))
}

/// The application menu bar (native NSMenu / GtkPopoverMenuBar / QMenuBar; app-bar overflow on Android;
/// UIMenuBuilder on iPadOS). Custom items carry keyboard shortcuts and update the shared `menu_log`;
/// the Edit menu uses standard roles so Cut/Copy/Paste target the focused control natively.
pub(crate) fn install_app_menu() {
    let log = |what: &'static str| move || menu_log().set(what.into());
    app_menu(vec![
        sub_menu(
            "File",
            vec![
                menu_item("New").key("n").action(log("File ▸ New")),
                menu_item("Open…").key("o").action(log("File ▸ Open")),
                // A nested submenu with keyboard shortcuts.
                sub_menu(
                    "Open Recent",
                    vec![
                        menu_item("report.pdf").action(log("Recent ▸ report.pdf")),
                        menu_item("budget.xlsx").action(log("Recent ▸ budget.xlsx")),
                        menu_separator(),
                        menu_item("Clear Menu").action(log("Recent ▸ Clear")),
                    ],
                ),
                menu_separator(),
                menu_item("Save").key("s").action(log("File ▸ Save")),
                menu_item("Save As…")
                    .shortcut(Shortcut::new("s").shift())
                    .action(log("File ▸ Save As")),
                menu_separator(),
                menu_role(MenuRole::CloseWindow),
                // Quit is a standard role: ⌘Q / Ctrl+Q, it exits the app and fires the
                // `WillTerminate` lifecycle phase (docs/lifecycle.md). macOS also keeps the
                // conventional Quit in the App menu.
                menu_role(MenuRole::Quit),
            ],
        ),
        // Standard edit commands — native items that target the focused control (default shortcuts).
        sub_menu(
            "Edit",
            vec![
                menu_role(MenuRole::Undo),
                menu_role(MenuRole::Redo),
                menu_separator(),
                menu_role(MenuRole::Cut),
                menu_role(MenuRole::Copy),
                menu_role(MenuRole::Paste),
                menu_role(MenuRole::SelectAll),
            ],
        ),
        sub_menu(
            "View",
            vec![
                menu_item("Reload").key("r").action(log("View ▸ Reload")),
                menu_item("Actual Size")
                    .key("0")
                    .action(log("View ▸ Actual Size")),
                menu_separator(),
                menu_role(MenuRole::Fullscreen),
            ],
        ),
    ]);
}

/// Menus & dialogs — the app's transient native surfaces in one place: the menu bar and
/// context menus (docs/menus.md), and the imperative dialogs (docs/dialogs.md), each in its own
/// themed section with a live result readout.
pub(crate) fn menus_page() -> AnyPiece {
    page(
        crate::res::str::nav_menus(),
        "menus-title",
        Some(crate::res::str::menus_caption()),
        form((app_menu_section(), context_section(), dialogs_section())).any(),
    )
}

/// The app-menu section: a live readout of the last action fired from the menu bar (or the
/// context menu below), plus the keyboard-shortcut hint.
fn app_menu_section() -> impl Piece {
    section((
        labeled(
            crate::res::str::menus_last(),
            label(move || menu_log().get()).id("menus-last"),
        ),
        label(crate::res::str::menus_shortcut_hint()).font(Font::Footnote),
    ))
    .title(crate::res::str::menus_appmenu_section())
}

/// The context-menu section: a visually delineated target the user secondary-clicks
/// (long-presses on mobile) — nested submenu, separator, and a standard role.
fn context_section() -> impl Piece {
    section((label(crate::res::str::menus_target())
        .padding(Insets::symmetric(24.0, 24.0))
        .id("menus-context-target")
        .context_menu(vec![
            menu_item("Rename").action(move || menu_log().set("Context ▸ Rename".into())),
            menu_item("Duplicate")
                .key("d")
                .action(move || menu_log().set("Context ▸ Duplicate".into())),
            menu_separator(),
            sub_menu(
                "Move To",
                vec![
                    menu_item("Inbox")
                        .action(move || menu_log().set("Context ▸ Move ▸ Inbox".into())),
                    menu_item("Archive")
                        .action(move || menu_log().set("Context ▸ Move ▸ Archive".into())),
                ],
            ),
            menu_separator(),
            menu_role(MenuRole::Copy),
            menu_item("Delete")
                .shortcut(Shortcut::plain("Delete"))
                .action(move || menu_log().set("Context ▸ Delete".into())),
        ])
        .background(Color::rgba(0.5, 0.5, 0.55, 0.16))
        .corner_radius(10.0),))
    .title(crate::res::str::menus_context_section())
}

/// Imperative dialogs (docs/dialogs.md): each button opens a native dialog from within an
/// async task and writes a fixed result token to `modal-result` (locale-independent so the
/// walkthrough can assert it).
fn dialogs_section() -> impl Piece {
    let last = Signal::new(String::new());
    section((
        row((
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
                    });
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
                    });
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
                    });
                })
                .id("btn-delete"),
        ))
        .spacing(8.0),
        row((
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
                    });
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
                    });
                })
                .id("btn-prompt"),
        ))
        .spacing(8.0),
        labeled(
            crate::res::str::modal_result_label(),
            label(move || {
                let v = last.get();
                if v.is_empty() { "—".into() } else { v }
            })
            .id("modal-result"),
        ),
    ))
    .title(crate::res::str::menus_dialogs_section())
}
