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
    let log = |what: String| move || menu_log().set(what.clone());
    // Localized menu names, resolved once at install (the locale is fixed for the run). The
    // MENU_LOG readout composes from the SAME strings so it always matches the visible menus;
    // `menu_role` items are localized by the OS itself. `report.pdf`/`budget.xlsx` are fixture
    // FILENAMES (data, not prose) and stay raw.
    let file = crate::res::str::menu_file().format();
    let new_item = crate::res::str::menu_new().format();
    let open = crate::res::str::menu_open().format();
    let recent = crate::res::str::menu_open_recent().format();
    let clear = crate::res::str::menu_clear_menu().format();
    let save = crate::res::str::menu_save().format();
    let save_as = crate::res::str::menu_save_as().format();
    let view = crate::res::str::menu_view().format();
    let reload = crate::res::str::menu_reload().format();
    let actual_size = crate::res::str::menu_actual_size().format();
    app_menu(vec![
        sub_menu(
            file.clone(),
            vec![
                menu_item(new_item.clone())
                    .key("n")
                    .action(log(format!("{file} ▸ {new_item}"))),
                menu_item(open.clone())
                    .key("o")
                    .action(log(format!("{file} ▸ {open}"))),
                // A nested submenu with keyboard shortcuts.
                sub_menu(
                    recent.clone(),
                    vec![
                        menu_item("report.pdf").action(log(format!("{recent} ▸ report.pdf"))),
                        menu_item("budget.xlsx").action(log(format!("{recent} ▸ budget.xlsx"))),
                        menu_separator(),
                        menu_item(clear.clone()).action(log(format!("{recent} ▸ {clear}"))),
                    ],
                ),
                menu_separator(),
                menu_item(save.clone())
                    .key("s")
                    .action(log(format!("{file} ▸ {save}"))),
                menu_item(save_as.clone())
                    .shortcut(Shortcut::new("s").shift())
                    .action(log(format!("{file} ▸ {save_as}"))),
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
            crate::res::str::menu_edit().format(),
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
            view.clone(),
            vec![
                menu_item(reload.clone())
                    .key("r")
                    .action(log(format!("{view} ▸ {reload}"))),
                menu_item(actual_size.clone())
                    .key("0")
                    .action(log(format!("{view} ▸ {actual_size}"))),
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
    // Localized like the app menu above; the log readout composes from the same strings.
    let log = |what: String| move || menu_log().set(what.clone());
    let context = crate::res::str::menu_context().format();
    let rename = crate::res::str::menu_rename().format();
    let duplicate = crate::res::str::menu_duplicate().format();
    let move_to = crate::res::str::menu_move_to().format();
    let inbox = crate::res::str::menu_inbox().format();
    let archive = crate::res::str::menu_archive().format();
    let delete = crate::res::str::delete().format();
    section((label(crate::res::str::menus_target())
        .padding(Insets::symmetric(24.0, 24.0))
        .id("menus-context-target")
        .context_menu(vec![
            menu_item(rename.clone()).action(log(format!("{context} ▸ {rename}"))),
            menu_item(duplicate.clone())
                .key("d")
                .action(log(format!("{context} ▸ {duplicate}"))),
            menu_separator(),
            sub_menu(
                move_to.clone(),
                vec![
                    menu_item(inbox.clone())
                        .action(log(format!("{context} ▸ {move_to} ▸ {inbox}"))),
                    menu_item(archive.clone())
                        .action(log(format!("{context} ▸ {move_to} ▸ {archive}"))),
                ],
            ),
            menu_separator(),
            menu_role(MenuRole::Copy),
            menu_item(delete.clone())
                .shortcut(Shortcut::plain("Delete"))
                .action(log(format!("{context} ▸ {delete}"))),
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
