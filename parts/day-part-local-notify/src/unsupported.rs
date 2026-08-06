//! Targets with no local-notification implementation wired: Windows and HarmonyOS.
//!
//! Both platforms HAVE a notification system — Windows `ToastNotification`, HarmonyOS Notification
//! Kit — so this is a gap in this crate, not in the platform (docs/notify.md says what each would
//! need). Answering `Unsupported` keeps an app honest: `capabilities().post` is false, so a UI can
//! disable its own controls instead of posting into a void.

use crate::{Capabilities, Channel, NotifId, Notification, NotifyError};

pub(crate) fn capabilities() -> Capabilities {
    Capabilities::default()
}

pub(crate) fn register_channel(_channel: &Channel) {}

pub(crate) fn post(_n: &Notification) -> Result<(), NotifyError> {
    Err(NotifyError::Unsupported)
}

pub(crate) fn cancel(_id: NotifId) {}

pub(crate) fn cancel_all() {}
