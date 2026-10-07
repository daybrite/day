// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The app's launcher shortcuts at run time (docs/deep-links.md "Launcher shortcuts").
//!
//! `[[shortcuts]]` in Day.toml is a list of routes with Fluent label keys. `day build` writes the
//! static platform declarations from it (iOS quick actions, Android `shortcuts.xml`, HarmonyOS
//! `shortcuts_config.json`, the web manifest), and `day-build` also registers the same list into
//! the binary through [`LAUNCHER_SHORTCUTS`], so a platform whose shortcut menu is built while the
//! app runs (the macOS Dock menu) can list them too. Choosing one opens its route through the
//! deep-link intake, the same door the static declarations use.

use std::cell::RefCell;
use std::rc::Rc;

/// The `[[shortcuts]]` of every app linked into this binary, as `(route, label key)` pairs,
/// registered at link time by the code `day-build` generates into the app's `res` module.
/// Empty for an app without `[[shortcuts]]`. Not on wasm, where `linkme` has no support and
/// the web manifest carries the shortcuts instead.
#[cfg(not(target_arch = "wasm32"))]
#[linkme::distributed_slice]
pub static LAUNCHER_SHORTCUTS: [&'static [(&'static str, &'static str)]];

/// The app's launcher shortcuts as `(route, label key)` pairs, in declaration order.
pub fn launcher_shortcuts() -> Vec<(&'static str, &'static str)> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        LAUNCHER_SHORTCUTS
            .iter()
            .flat_map(|list| list.iter().copied())
            .collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
        Vec::new()
    }
}

thread_local! {
    /// The shortcuts the app set while running (`set_launcher_shortcuts`).
    static DYNAMIC: RefCell<Vec<day_spec::LauncherShortcut>> = const { RefCell::new(Vec::new()) };
    /// One dispatch id per shortcut route, registered on first use and kept for the life of the
    /// process: the menus that list shortcuts are rebuilt often, the shortcuts never change.
    static ACTIONS: RefCell<Vec<(String, u64)>> = const { RefCell::new(Vec::new()) };
}

fn action_for(route: &str) -> u64 {
    if let Some(id) = ACTIONS.with(|a| {
        a.borrow()
            .iter()
            .find(|(r, _)| r == route)
            .map(|(_, id)| *id)
    }) {
        return id;
    }
    let owned = route.to_owned();
    let id = crate::menu::register_menu_action(Rc::new(move || {
        crate::request_open_url(&owned);
    }));
    ACTIONS.with(|a| a.borrow_mut().push((route.to_owned(), id)));
    id
}

/// Replace the launcher shortcuts the app sets while it runs (docs/deep-links.md "Launcher
/// shortcuts"): "Resume <last document>", the three most recent chats. They show beside the
/// Day.toml `[[shortcuts]]` on the iOS Home Screen, the Android launcher and the macOS Dock menu
/// (`Cap::DynamicShortcuts`), and each opens its route like a deep link. Labels are shown as
/// given, so pass localized text.
pub fn set_launcher_shortcuts(shortcuts: Vec<day_spec::LauncherShortcut>) {
    DYNAMIC.with(|d| *d.borrow_mut() = shortcuts.clone());
    crate::with_tree(|t| t.set_launcher_shortcuts(&shortcuts));
    crate::menu::install_dock_menu();
}

/// The shortcuts as menu items in the current locale, for a menu built while the app runs: the
/// app's run-time ones first (they are the most recent), then the declared ones.
pub fn shortcut_menu_items() -> Vec<day_spec::MenuItem> {
    let dynamic = DYNAMIC.with(|d| d.borrow().clone());
    let declared = launcher_shortcuts()
        .into_iter()
        .map(|(route, label)| (route.to_owned(), day_l10n::t(label)));
    dynamic
        .into_iter()
        .map(|s| (s.route, s.label))
        .chain(declared)
        .map(|(route, label)| day_spec::MenuItem::Action {
            id: Some(format!("shortcut:{route}")),
            action: action_for(&route),
            label,
            shortcut: None,
            enabled: true,
            checked: None,
            role: None,
            icon: None,
        })
        .collect()
}
