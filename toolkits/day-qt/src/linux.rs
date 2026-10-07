// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! The Linux session-bus pieces Qt does not provide (docs/status-item.md, docs/deep-links.md):
//! badge count and progress through the Unity LauncherEntry signal, and single-instance
//! forwarding, which GTK gets from GApplication and Qt has no equivalent for.
use std::cell::{Cell, RefCell};

use day_dbus::Connection;
use day_dbus::launcher::{self, LauncherProps};
use day_spec::Platform as _;

thread_local! {
    static CONN: RefCell<Option<Connection>> = const { RefCell::new(None) };
    static PROPS: Cell<LauncherProps> = Cell::new(LauncherProps::default());
}

/// The app id: the desktop id docks match a launcher entry by, and the single-instance name.
fn app_id() -> Option<String> {
    std::env::var("DAY_APP_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| option_env!("DAY_APP_ID").map(str::to_owned))
}

/// Send the whole launcher state: every `Update` restates count, progress and urgency. A signal
/// nothing answers, so whether a dock shows it is the desktop's business (Emulated).
fn send(props: LauncherProps) {
    PROPS.with(|p| p.set(props));
    let Some(id) = app_id() else {
        return;
    };
    CONN.with(|c| {
        let mut c = c.borrow_mut();
        if c.as_ref().is_none_or(|conn| conn.is_closed()) {
            *c = Connection::session().ok();
        }
        if let Some(conn) = c.as_ref() {
            match launcher::update(conn, &id, props) {
                Ok(()) => log::debug!("qt: launcher entry {id} {props:?}"),
                Err(e) => log::debug!("qt: launcher entry {id} not sent: {e}"),
            }
        }
    });
}

/// `AppProgress`: a fraction shows the bar; an error also marks the icon urgent. The protocol has
/// no indeterminate or paused bar, so indeterminate shows none and paused shows its value.
pub(crate) fn set_progress(progress: day_spec::AppProgress) {
    use day_spec::AppProgress as P;
    let mut props = PROPS.with(|p| p.get());
    (props.progress, props.urgent) = match progress {
        P::None | P::Indeterminate => (None, None),
        P::Value(v) | P::Paused(v) => (Some(v), None),
        P::Error(v) => (Some(v), Some(true)),
    };
    send(props);
}

/// `AppBadge`: only a count exists in the protocol; text and a dot are not substituted.
pub(crate) fn set_badge(badge: &day_spec::AppBadge) {
    let mut props = PROPS.with(|p| p.get());
    props.count = match badge {
        day_spec::AppBadge::None | day_spec::AppBadge::Count(0) => None,
        day_spec::AppBadge::Count(n) => Some(i64::from(*n)),
        _ => return,
    };
    send(props);
}

/// Claim the app id for this process before anything else starts (docs/deep-links.md
/// "Single-instance forwarding"). A second launch hands its arguments to the first and exits
/// here; the first hears them on the bus's reader thread and delivers them on the Qt main
/// thread through `day_core::forward_launch_args`. Without an app id or a session bus every
/// launch runs on its own, as before. The returned claim lives as long as the app.
pub(crate) fn claim_instance() -> Option<day_dbus::instance::Claim> {
    use day_dbus::instance::{Claimed, claim};
    let id = app_id()?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    match claim(&id, args, |forwarded| {
        crate::Qt::post(Box::new(move || {
            log::debug!("qt: launch forwarded by a second instance: {forwarded:?}");
            day_core::forward_launch_args(forwarded);
        }))
    }) {
        Ok(c) => Some(c),
        Err(Claimed::Forwarded) => {
            log::info!("{id} is already running; handed this launch to it");
            std::process::exit(0);
        }
        Err(Claimed::NoBus(e)) => {
            log::debug!("qt: no single-instance claim: {e}");
            None
        }
    }
}
