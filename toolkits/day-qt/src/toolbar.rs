// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Qt: a QToolBar under the menu bar (docs/toolbars.md). Items are QActions, so the toolbar
// takes its icon size and its icon/text style from the user's Qt settings the way every other
// Qt app on their desktop does — which is the KDE convention, and why nothing here sets those
// explicitly. Search is a QLineEdit with the clear button and the leading find action; a menu
// item is a QToolButton in InstantPopup mode, which draws Qt's own pull-down chevron.
// ---------------------------------------------------------------------------

use std::ffi::{CStr, c_char, c_int, c_void};

use day_qt_sys as ffi;
use day_spec::{Event, Icon, Symbol, ToolbarItem, ToolbarItemKind, ToolbarPatch, ToolbarValue};

use crate::{Qt, QtHandle, build_qt_menu, cstr, emit};

/// `QStyle::StandardPixmap` values, as the fallback for platforms with no freedesktop icon
/// theme (macOS, Windows). `-1` = no standard icon, so the item shows its text instead.
mod sp {
    pub const ARROW_BACK: i32 = 53;
    pub const ARROW_FORWARD: i32 = 54;
    pub const ARROW_UP: i32 = 49;
    pub const ARROW_DOWN: i32 = 52;
    pub const BROWSER_RELOAD: i32 = 58;
    pub const DIALOG_OPEN: i32 = 20;
    pub const DIALOG_SAVE: i32 = 43;
    pub const DIALOG_APPLY: i32 = 44;
    pub const DIALOG_CANCEL: i32 = 39;
    pub const DIALOG_HELP: i32 = 27;
    pub const DIR_ICON: i32 = 37;
    pub const FILE_ICON: i32 = 24;
    pub const NEW_FOLDER: i32 = 46;
    pub const TRASH: i32 = 14;
    pub const MEDIA_PLAY: i32 = 60;
    pub const MEDIA_STOP: i32 = 61;
    pub const MEDIA_PAUSE: i32 = 62;
    pub const MSG_WARNING: i32 = 10;
    pub const MSG_INFORMATION: i32 = 9;
    pub const NONE: i32 = -1;
}

/// The freedesktop icon name and the QStyle fallback for each standard symbol.
fn icon_for(s: Symbol) -> (&'static str, i32) {
    match s {
        Symbol::Add => ("list-add", sp::NONE),
        Symbol::Remove => ("list-remove", sp::NONE),
        Symbol::Delete => ("edit-delete", sp::TRASH),
        Symbol::Edit => ("document-edit", sp::NONE),
        Symbol::New => ("document-new", sp::NEW_FOLDER),
        Symbol::Open => ("document-open", sp::DIALOG_OPEN),
        Symbol::Save => ("document-save", sp::DIALOG_SAVE),
        Symbol::Print => ("document-print", sp::NONE),
        Symbol::Refresh => ("view-refresh", sp::BROWSER_RELOAD),
        Symbol::Search => ("system-search", sp::NONE),
        Symbol::Share => ("emblem-shared", sp::NONE),
        Symbol::Settings => ("configure", sp::NONE),
        Symbol::Info => ("dialog-information", sp::MSG_INFORMATION),
        Symbol::Star => ("starred", sp::NONE),
        Symbol::Bookmark => ("bookmarks", sp::NONE),
        Symbol::Back => ("go-previous", sp::ARROW_BACK),
        Symbol::Forward => ("go-next", sp::ARROW_FORWARD),
        Symbol::Up => ("go-up", sp::ARROW_UP),
        Symbol::Down => ("go-down", sp::ARROW_DOWN),
        Symbol::Home => ("go-home", sp::NONE),
        Symbol::Sidebar => ("sidebar-show", sp::NONE),
        Symbol::Filter => ("view-filter", sp::NONE),
        Symbol::Sort => ("view-sort-ascending", sp::NONE),
        Symbol::More => ("overflow-menu", sp::NONE),
        Symbol::Play => ("media-playback-start", sp::MEDIA_PLAY),
        Symbol::Pause => ("media-playback-pause", sp::MEDIA_PAUSE),
        Symbol::Stop => ("media-playback-stop", sp::MEDIA_STOP),
        Symbol::ZoomIn => ("zoom-in", sp::NONE),
        Symbol::ZoomOut => ("zoom-out", sp::NONE),
        Symbol::Undo => ("edit-undo", sp::NONE),
        Symbol::Redo => ("edit-redo", sp::NONE),
        Symbol::Copy => ("edit-copy", sp::NONE),
        Symbol::Cut => ("edit-cut", sp::NONE),
        Symbol::Paste => ("edit-paste", sp::NONE),
        Symbol::Mail => ("mail-send", sp::NONE),
        Symbol::Folder => ("folder", sp::DIR_ICON),
        Symbol::Document => ("text-x-generic", sp::FILE_ICON),
        Symbol::Check => ("dialog-ok", sp::DIALOG_APPLY),
        Symbol::Close => ("window-close", sp::DIALOG_CANCEL),
        Symbol::Warning => ("dialog-warning", sp::MSG_WARNING),
        // The vocabulary is `#[non_exhaustive]`: an unmapped symbol shows the label.
        _ => ("", sp::DIALOG_HELP),
    }
}

/// An item's icon as the (theme name, QStyle fallback) pair the shim takes. A bundled image
/// has no theme name; Qt loads it by path through the theme lookup's file branch.
fn icon_args(icon: Option<&Icon>) -> (String, c_int) {
    match icon {
        Some(Icon::Symbol(s)) => {
            let (name, fallback) = icon_for(*s);
            (name.to_string(), fallback as c_int)
        }
        Some(Icon::Image(name)) => (
            day_spec::resource::resolve_image_file(name)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            sp::NONE as c_int,
        ),
        None => (String::new(), sp::NONE as c_int),
    }
}

/// The reserved dispatch id a `SidebarToggle` item carries. Qt routes every toolbar click
/// through an action id, and this item has no app action to route — so it rides a sentinel that
/// `on_toolbar_value` intercepts and never forwards to the app's registry.
pub(crate) const SIDEBAR_TOGGLE_ACTION: u64 = u64::MAX;

/// Values from the shim: kind 0 = a toggle's new state, kind 1 = a search field's text.
pub(crate) extern "C" fn on_toolbar_value(
    action: u64,
    kind: c_int,
    on: c_int,
    text: *const c_char,
) {
    // The sidebar toggle is Day's own, not the app's: drive the split host and stop here.
    if action == SIDEBAR_TOGGLE_ACTION {
        crate::toggle_sidebar();
        return;
    }
    let value = if kind == 0 {
        ToolbarValue::On(on != 0)
    } else {
        let text = unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned();
        ToolbarValue::Text(text)
    };
    emit(
        day_spec::WINDOW_NODE,
        Event::ToolbarChanged { action, value },
    );
}

impl Qt {
    /// Install `items` as this window's toolbar (docs/toolbars.md).
    pub(crate) fn install_toolbar(&mut self, h: &QtHandle, items: &[ToolbarItem]) {
        let Some(win) = self.window_of(h) else { return };
        let bar = unsafe { ffi::day_qt_window_toolbar(win) };
        if bar.is_null() {
            return;
        }
        for item in items {
            let id = cstr(&item.id);
            let label = cstr(&item.label);
            let tip = cstr(item.tooltip.as_deref().unwrap_or(&item.label));
            let (icon, fallback) = icon_args(item.icon.as_ref());
            let icon = cstr(&icon);
            match &item.kind {
                ToolbarItemKind::Button
                | ToolbarItemKind::Toggle { .. }
                | ToolbarItemKind::SidebarToggle => {
                    // A checkable action whose checked state IS the sidebar's visibility, so
                    // the button reads pressed while the pane is open (docs/toolbars.md).
                    let (checkable, checked) = match item.kind {
                        ToolbarItemKind::Toggle { on } => (1, on as c_int),
                        ToolbarItemKind::SidebarToggle => (1, 1),
                        _ => (0, 0),
                    };
                    let action = match item.kind {
                        ToolbarItemKind::SidebarToggle => SIDEBAR_TOGGLE_ACTION,
                        _ => item.action,
                    };
                    unsafe {
                        ffi::day_qt_toolbar_add_action(
                            bar,
                            id.as_ptr(),
                            label.as_ptr(),
                            icon.as_ptr(),
                            fallback,
                            tip.as_ptr(),
                            action,
                            item.enabled as c_int,
                            checkable,
                            checked,
                        )
                    };
                }
                ToolbarItemKind::Menu { items } => {
                    let menu = unsafe {
                        ffi::day_qt_toolbar_add_menu(
                            bar,
                            id.as_ptr(),
                            label.as_ptr(),
                            icon.as_ptr(),
                            fallback,
                            tip.as_ptr(),
                            item.enabled as c_int,
                        )
                    };
                    if !menu.is_null() {
                        build_qt_menu(menu, items);
                    }
                }
                ToolbarItemKind::Search { text, placeholder } => {
                    let text = cstr(text);
                    let ph = cstr(placeholder);
                    unsafe {
                        ffi::day_qt_toolbar_add_search(
                            bar,
                            id.as_ptr(),
                            text.as_ptr(),
                            ph.as_ptr(),
                            item.action,
                            item.enabled as c_int,
                        )
                    };
                }
                ToolbarItemKind::Label => unsafe {
                    ffi::day_qt_toolbar_add_label(bar, id.as_ptr(), label.as_ptr())
                },
                ToolbarItemKind::Separator => unsafe { ffi::day_qt_toolbar_add_separator(bar) },
                ToolbarItemKind::Space => unsafe { ffi::day_qt_toolbar_add_space(bar, 0) },
                ToolbarItemKind::FlexibleSpace => unsafe { ffi::day_qt_toolbar_add_space(bar, 1) },
            }
        }
        unsafe { ffi::day_qt_window_toolbar_done(win) };
    }

    /// Apply a targeted change to one live item.
    pub(crate) fn patch_toolbar(&mut self, _h: &QtHandle, patch: &ToolbarPatch) {
        match patch {
            ToolbarPatch::Text { item, text } => {
                let (id, text) = (cstr(item), cstr(text));
                unsafe { ffi::day_qt_toolbar_set_text(id.as_ptr(), text.as_ptr()) };
            }
            ToolbarPatch::On { item, on } => {
                let id = cstr(item);
                unsafe { ffi::day_qt_toolbar_set_checked(id.as_ptr(), *on as c_int) };
            }
            ToolbarPatch::Enabled { item, on } => {
                let id = cstr(item);
                unsafe { ffi::day_qt_toolbar_set_enabled(id.as_ptr(), *on as c_int) };
            }
        }
    }

    /// The window a day root handle belongs to: the primary window, or the secondary whose
    /// content area is this handle.
    fn window_of(&self, h: &QtHandle) -> Option<*mut c_void> {
        if let Some(w) = self.secondary.iter().find(|w| w.content == h.0) {
            return Some(w.win);
        }
        (!self.window.is_null()).then_some(self.window)
    }
}
