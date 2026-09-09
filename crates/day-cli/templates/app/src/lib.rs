//! {{title}}, a [Day](https://daybrite.dev) app. `root()` runs once and opens the first window;
//! `window_shell` builds one window's UI, and each section lives under `pages/`.

use day::prelude::*;

mod model;
mod pages;
use crate::model::Scene;
use crate::pages::*;

// Entry point for the mobile hosts; a desktop build enters through src/main.rs.
day::day_start!(options: window(), root);

/// Options for every window. The catalog and title go to `launch`, which installs them
/// (https://daybrite.dev/docs/localization).
pub fn window() -> day::WindowOptions {
    day::WindowOptions {
        locales: Some((res::locales::DEFAULT, res::locales::CATALOG)),
        title_fn: Some(|| res::str::app_title().format()),
        // Desktop only; phones fill the screen.
        size: day::prelude::Size::new(960.0, 640.0),
        ..Default::default()
    }
}

// Typed names for everything under `resource/` (https://daybrite.dev/docs/resources).
day::resources!();

/// The `day::prefs` keys the Settings page writes and startup reads.
const THEME_KEY: &str = "app.theme";
const LOCALE_KEY: &str = "app.locale";

day::routes! {
    /// The app's sections, as typed routes (https://daybrite.dev/docs/navigation).
    pub(crate) enum Section {
        Welcome => "welcome",
        Navigate => "navigate",
        Settings => "settings",
    }
}

/// True where there is a menu bar, so Settings lives in the App menu instead of the nav.
pub(crate) fn has_menu_bar() -> bool {
    capability(Cap::AppMenu) != Support::Unsupported
}

/// One-time app setup, then the first window's content.
pub fn root() -> impl Piece {
    // Day installs a logger at launch, so `info!` works as is.
    info!("{{title}} starting");
    // Reapply the saved theme and language before anything is built.
    day_piece_settings::apply_startup(THEME_KEY, LOCALE_KEY);

    // A Settings window and App ▸ Settings… on desktop; a fullscreen cover elsewhere.
    day::register_preferences(settings_body);
    // File ▸ New Window builds the same shell again, which is why the state is a `Scene`
    // per window rather than a global.
    day::register_new_window(|| window_shell(false));
    app_menu(menus());

    window_shell(true)
}

/// One window's UI, for the first window and every File ▸ New Window.
///
/// `Scene::scoped` gives the window its own state, so two windows share no selection or
/// document. The primary window also owns the persisted state and the route namespace.
fn window_shell(primary: bool) -> impl Piece {
    Scene::scoped(move |scene| {
        if primary {
            scene.persist();
        }
        // Title the window after the open item: the Window menu, the tab bar, and the app
        // switchers all label windows by it.
        day::window_title(
            move || match scene.selected.get().and_then(|id| scene.find(id)) {
                Some(item) if !item.name.is_empty() => item.name,
                _ => res::str::app_title().format(),
            },
        );
        // Tabs on a phone, a rail on a tablet, a sidebar on a desktop
        // (https://daybrite.dev/docs/navigation).
        let nav = nav(scene.section)
            .title(res::str::app_title())
            // The list is a content-list pane: its own column where there is room, a pushed
            // layer on a phone.
            .content_list(item_list_pane)
            .content_list_width(320.0)
            // Only Navigate has a list.
            .content_list_for(|s: &Section| matches!(s, Section::Navigate))
            // Whether the editor is up, where only one pane shows at a time.
            .detail_visible(scene.detail_open)
            // The pushed editor's bar title, kept live.
            .detail_title(move || detail_title(scene))
            .item_icon(
                Section::Welcome,
                res::str::nav_welcome(),
                res::vectors::tab_welcome,
                welcome_page,
            )
            // One tint per section, so the icons read apart.
            .icon_tint(Color::hex(0xF59E0B))
            .item_icon(
                Section::Navigate,
                res::str::nav_navigate(),
                res::vectors::tab_navigate,
                navigate_page,
            )
            .icon_tint(Color::hex(0x3B82F6))
            // Settings is a nav row only where there is no menu bar.
            .items(
                move || {
                    if has_menu_bar() {
                        Vec::new()
                    } else {
                        vec![Section::Settings]
                    }
                },
                |s: &Section| {
                    item(*s, res::str::nav_settings())
                        .icon(res::vectors::tab_settings)
                        .icon_tint(Color::hex(0x10B981))
                },
            )
            .destination(|_: &Section| settings_page())
            .id("nav");
        // Only the first window joins the route namespace and restores its place; two routed
        // navs would make deep links ambiguous.
        if primary {
            nav.restore("app.section")
        } else {
            nav.local()
        }
    })
}

/// Run a command against the focused window. Menu bar items belong to no window, so they
/// look the front one up when they run.
fn front(f: impl Fn(Scene) + 'static) -> impl Fn() + 'static {
    move || {
        if let Some(scene) = Scene::focused() {
            f(scene)
        }
    }
}

/// The desktop menu bar; the mobile toolkits ignore it (https://daybrite.dev/docs/guide-desktop).
fn menus() -> Vec<MenuEntry> {
    vec![
        sub_menu(
            res::str::menu_file().format(),
            vec![
                // The platform's own New Window item and ⌘N; disabled until a builder exists.
                menu_role(MenuRole::NewWindow),
                menu_item(res::str::cmd_add().format())
                    // ⌘N is taken by New Window, so this gets ⌘⇧N.
                    .shortcut(Shortcut::new("n").shift())
                    .action(front(|scene| scene.new_item())),
                menu_separator(),
                menu_role(MenuRole::CloseWindow),
            ],
        ),
        // The desktop counterpart to the list's swipe actions.
        sub_menu(
            res::str::menu_edit().format(),
            vec![
                menu_item(res::str::cmd_delete().format())
                    .shortcut(Shortcut::new("Delete"))
                    .action(front(|scene| scene.delete_selected())),
                menu_item(res::str::cmd_done().format())
                    .shortcut(Shortcut::new("d"))
                    .action(front(|scene| scene.done_selected())),
                menu_item(res::str::cmd_show_done().format())
                    .shortcut(Shortcut::new("h"))
                    .action(front(|scene| scene.show_done.update(|v| *v = !*v))),
                menu_separator(),
                menu_role(MenuRole::Cut),
                menu_role(MenuRole::Copy),
                menu_role(MenuRole::Paste),
                menu_role(MenuRole::SelectAll),
            ],
        ),
    ]
}
