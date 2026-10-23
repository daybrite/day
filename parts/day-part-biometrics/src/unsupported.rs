// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Targets with no user-presence API an application may call: desktop Linux (GTK and Qt) and the
//! web. Every probe says so and every prompt fails with [`BiometricError::Unsupported`], so an app
//! linking this crate there gets a correct "no" and can hide its button.

use crate::{Availability, BiometricError, Biometry, OnDone, Prompt, Support};

pub fn support() -> Support {
    Support::Unsupported
}

pub fn availability() -> Availability {
    Availability::Unsupported
}

pub fn biometry() -> Biometry {
    Biometry::None
}

pub fn authenticate(_prompt: Prompt, on_done: OnDone) {
    on_done(Err(BiometricError::Unsupported));
}

pub fn cancel() {}
