//! Desktop Linux and Windows: no consent database, so nothing to ask.
//!
//! Both platforms let a normal desktop process open the camera, the microphone and the network
//! without asking anyone, so every capability that EXISTS here reports [`Gate::Ungated`] +
//! [`Status::Granted`] — an app should proceed, and a missing device should fail at the device, not
//! at a permission check that has nothing to check. Capabilities with no desktop equivalent at all
//! (a photo library, motion/fitness activity) report [`Gate::Absent`] + [`Status::Unsupported`], so
//! an app can hide the feature instead of offering a dead button.
//!
//! Two future splits will turn this shared module into per-OS files, and neither changes the
//! answers above for an unpackaged app:
//! - **Linux**: xdg-desktop-portal (`org.freedesktop.portal.Camera`, `.Location`) is how a
//!   flatpak-confined app actually asks. It needs a D-Bus dependency this tree does not have.
//! - **Windows**: `Windows.Security.Authorization.AppCapabilityAccess` applies to packaged
//!   (MSIX) apps; Day's MSIX declares only `runFullTrust`, which is not capability-gated.

use super::{Gate, Permission, Status};

fn absent(perm: Permission) -> bool {
    matches!(perm, Permission::Photos | Permission::Motion) || matches!(perm, Permission::Raw(_))
}

pub fn gate(perm: Permission) -> Gate {
    if absent(perm) {
        Gate::Absent
    } else {
        Gate::Ungated
    }
}

pub fn status(perm: Permission) -> Status {
    if absent(perm) {
        Status::Unsupported
    } else {
        Status::Granted
    }
}

pub fn status_async(perm: Permission, on_done: Box<dyn FnOnce(Status) + Send>) {
    on_done(status(perm));
}

pub fn can_prompt(_perm: Permission) -> bool {
    false
}

pub fn should_show_rationale(_perm: Permission) -> bool {
    false
}

pub fn request(perm: Permission, on_done: Box<dyn FnOnce(Status) + Send>) {
    on_done(status(perm));
}

pub fn open_settings(_perm: Permission) -> bool {
    false
}
