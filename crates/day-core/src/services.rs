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

/// Native notification delivery before or after a window exists. Handlers must return promptly
/// and use a main-thread setter to update UI state. Payloads are JSON from the platform.
#[derive(Clone)]
pub enum NotificationEvent {
    Registered(Vec<u8>),
    RegistrationFailed(String),
    Received { payload: String, opened: bool },
}

impl std::fmt::Debug for NotificationEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Registered(_) => f.debug_tuple("Registered").finish_non_exhaustive(),
            Self::RegistrationFailed(error) => {
                f.debug_tuple("RegistrationFailed").field(error).finish()
            }
            Self::Received { opened, .. } => f
                .debug_struct("Received")
                .field("opened", opened)
                .finish_non_exhaustive(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[linkme::distributed_slice]
pub static NOTIFICATION_HANDLERS: [fn(&NotificationEvent)];

/// Forward a native delegate callback to linked optional parts.
#[expect(
    clippy::needless_pass_by_value,
    reason = "public signature; toolkits hand over the event they built"
)]
pub fn notification(event: NotificationEvent) {
    #[cfg(not(target_arch = "wasm32"))]
    for handler in NOTIFICATION_HANDLERS {
        handler(&event);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = event;
}

/// Android's headless host has a JVM and application Context but no widget tree.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_daybrite_day_bridge_DayRuntime_initializeServices(
    mut env: day_platform_android::jni::EnvUnowned,
    class: day_platform_android::jni::objects::JClass,
) {
    env.with_env(|env| -> day_platform_android::jni::errors::Result<()> {
        day_platform_android::init(env, &class)?;
        initialize();
        Ok(())
    })
    .resolve::<day_platform_android::jni::errors::ThrowRuntimeExAndDefault>();
}
