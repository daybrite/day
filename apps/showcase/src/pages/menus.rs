use day::prelude::*;
use std::cell::OnceCell;

use crate::lifecycle_log;
use crate::widgets::heading;

thread_local! {
    /// The last menu action fired — shared between the app menu (installed in `root`) and this
    /// Menus page so both demonstrate action dispatch. Created lazily inside the reactive runtime.
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

/// Menus playground: a context menu (secondary-click on desktop, long-press on mobile) with nested
/// submenus, standard roles, and shortcuts, plus a live readout of the last menu action fired — from
/// EITHER the app menu bar or this context menu. See docs/menus.md.
pub(crate) fn menus_page() -> AnyPiece {
    column((
        heading(crate::res::str::nav_menus(), "menus-title", Some(crate::res::str::menus_caption())),
        // Live readouts: the last menu action (app menu or context menu), and the last app-lifecycle
        // phase (docs/lifecycle.md) — Quit fires WillTerminate; switching apps fires resign/active.
        column((
            label(move || format!("{}  {}", crate::res::str::menus_last().format(), menu_log().get()))
                .id("menus-last"),
            label(move || {
                format!(
                    "{}  {}",
                    crate::res::str::menus_lifecycle().format(),
                    lifecycle_log().get()
                )
            })
            .id("menus-lifecycle"),
        ))
        .spacing(6.0)
        .align(HAlign::Leading),
        divider(),
        label(crate::res::str::menus_context_hint()).font(Font::Headline),
        // A target for the context menu: nested submenu + a separator + a standard role.
        label(crate::res::str::menus_target())
            .font(Font::Body)
            .padding(Insets::symmetric(20.0, 28.0))
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
            ]),
        divider(),
        label(crate::res::str::menus_shortcut_hint()).font(Font::Footnote),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
