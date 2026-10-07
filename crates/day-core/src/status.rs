// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The app's presence outside its windows (docs/status-item.md): status items, progress on the
//! Dock or taskbar icon, the macOS Dock icon itself, and whether the app keeps running when its
//! last window closes.

use std::cell::{Cell, RefCell};

use day_spec::{AppProgress, StatusItemSpec};

/// Whether the app keeps running when its last window closes (docs/windows.md "Keeping the app
/// running"), set with [`set_keep_running`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeepRunning {
    /// Keep running while the app shows a status item, so a tray app's menu stays usable with
    /// every window closed; otherwise follow the platform's rule.
    #[default]
    Automatic,
    /// Keep running with no window open, status item or not: a background helper, an app that
    /// reopens its window from the Dock or a global shortcut. The app ends when it calls
    /// `day::quit()` or the user quits from a menu.
    Always,
    /// End the app when its last window closes, on every platform, macOS included.
    Never,
}

thread_local! {
    static POLICY: Cell<KeepRunning> = const { Cell::new(KeepRunning::Automatic) };
    /// The status items, in the order the app first added them, each with the dispatch ids its
    /// menu and click hold (swept when the item changes or goes).
    static ITEMS: RefCell<Vec<(StatusItemSpec, Vec<u64>)>> = const { RefCell::new(Vec::new()) };
    /// What the toolkit was last told about keeping the process alive.
    static TOLD_KEEP: Cell<Option<bool>> = const { Cell::new(None) };
}

/// Choose whether the app keeps running when its last window closes (docs/windows.md).
pub fn set_keep_running(policy: KeepRunning) {
    POLICY.with(|p| p.set(policy));
    sync_keep_running();
}

/// The policy as set (default [`KeepRunning::Automatic`]).
pub fn keep_running_policy() -> KeepRunning {
    POLICY.with(|p| p.get())
}

/// Whether closing the last window should leave the app running right now.
pub(crate) fn keeps_running() -> bool {
    match keep_running_policy() {
        KeepRunning::Always => true,
        KeepRunning::Never => false,
        KeepRunning::Automatic => ITEMS.with(|i| !i.borrow().is_empty()),
    }
}

/// Tell the toolkit when the answer changes, so it stops (or resumes) ending the app on its own.
fn sync_keep_running() {
    let keep = keeps_running();
    if TOLD_KEEP.with(|t| t.replace(Some(keep))) != Some(keep) && crate::tree::has_tree() {
        crate::with_tree(|t| t.set_keep_running(keep));
    }
}

/// Re-send the keep-running answer: the backend that boots after the app set its policy (a
/// `set_keep_running` in `root()` runs before the tree exists on some paths).
pub(crate) fn boot_sync() {
    TOLD_KEEP.with(|t| t.set(None));
    if keeps_running() {
        sync_keep_running();
    }
}

fn action_ids(spec: &StatusItemSpec) -> Vec<u64> {
    let mut ids = crate::menu::collect_ids(&spec.menu);
    if spec.activate != 0 {
        ids.push(spec.activate);
    }
    ids
}

/// Show `spec`, or update the shown item with the same id (docs/status-item.md). Replacing an
/// item drops the closures its previous menu and click held.
pub fn set_status_item(spec: StatusItemSpec) {
    let ids = action_ids(&spec);
    let stale = ITEMS.with(|items| {
        let mut items = items.borrow_mut();
        match items.iter_mut().find(|(s, _)| s.id == spec.id) {
            Some((s, old)) => {
                let stale: Vec<u64> = old.iter().copied().filter(|i| !ids.contains(i)).collect();
                *s = spec;
                *old = ids;
                stale
            }
            None => {
                items.push((spec, ids));
                Vec::new()
            }
        }
    });
    crate::menu::forget_actions(&stale);
    send_items();
}

/// Remove the status item `id`, if shown.
pub fn remove_status_item(id: &str) {
    let stale = ITEMS.with(|items| {
        let mut items = items.borrow_mut();
        let i = items.iter().position(|(s, _)| s.id == id)?;
        Some(items.remove(i).1)
    });
    if let Some(stale) = stale {
        crate::menu::forget_actions(&stale);
        send_items();
    }
}

/// The status items currently shown, in order.
pub fn status_items() -> Vec<StatusItemSpec> {
    ITEMS.with(|i| i.borrow().iter().map(|(s, _)| s.clone()).collect())
}

fn send_items() {
    let items = status_items();
    crate::with_tree(|t| t.set_status_items(&items));
    sync_keep_running();
}

/// Show `progress` on the app's Dock or taskbar icon (`Cap::AppProgress`, docs/status-item.md).
pub fn set_app_progress(progress: AppProgress) {
    crate::with_tree(|t| t.set_app_progress(progress));
}

/// Show or hide the app's Dock icon while it runs (`Cap::DockVisibility`, macOS). A menu-bar
/// app hides it and lives in its status item; `[app.macos] dock = false` in Day.toml hides it
/// from launch, before the first frame.
pub fn set_dock_visible(visible: bool) {
    crate::with_tree(|t| t.set_dock_visible(visible));
}

/// Forget everything (tests; pairs with `uninstall_tree`).
pub fn reset_status() {
    ITEMS.with(|i| i.borrow_mut().clear());
    POLICY.with(|p| p.set(KeepRunning::Automatic));
    TOLD_KEEP.with(|t| t.set(None));
}
