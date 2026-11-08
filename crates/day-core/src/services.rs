// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Process services that must initialize before a window or reactive tree exists.
//!
//! Optional parts register initializers at link time. Initializers run on the native startup
//! thread once per process and must return promptly. They may register OS callbacks; they must
//! not access UI signals or mount widgets. Android services can initialize this registry after
//! attaching the JVM without starting an Activity.

use std::sync::Once;

/// A process initializer supplied by an optional part.
pub struct Service {
    pub name: &'static str,
    pub initialize: fn(),
}

#[cfg(not(target_arch = "wasm32"))]
#[linkme::distributed_slice]
pub static SERVICES: [Service];

/// Initialize linked process services. A native host calls this after its runtime is ready.
pub fn initialize() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        #[cfg(not(target_arch = "wasm32"))]
        for service in SERVICES {
            (service.initialize)();
        }
    });
}
