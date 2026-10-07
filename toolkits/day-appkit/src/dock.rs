// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! The Dock menu (docs/menus.md "Dock menu"): the items macOS lists above Show / Hide / Quit
//! when the user right-clicks the app's Dock icon. day-core composes the model (launcher
//! shortcuts, then the app's `dock_menu` entries) and installs it through `set_dock_menu`; the
//! app delegate hands the built menu to AppKit through `applicationDockMenu:`, which AppKit asks
//! every time the menu opens.
use super::*;

thread_local! {
    static DOCK_MENU: RefCell<Option<Retained<NSMenu>>> = const { RefCell::new(None) };
}

/// Build and keep the Dock menu for `items`; an empty slice removes it.
pub(super) fn set_app_items(mtm: MainThreadMarker, items: &[day_spec::MenuItem]) {
    let menu = (!items.is_empty()).then(|| build_ns_menu(mtm, "", items));
    DOCK_MENU.with(|m| *m.borrow_mut() = menu);
}

/// The menu `applicationDockMenu:` answers.
pub(super) fn menu() -> Option<Retained<NSMenu>> {
    DOCK_MENU.with(|m| m.borrow().clone())
}
