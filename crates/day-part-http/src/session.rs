// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Runtime routing. Selection is captured for a logical request; changes affect the next one.
use crate::{Capabilities, simulation::Simulation, statistics::Store, transport::*};
use std::sync::{Arc, Mutex, OnceLock};

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub(crate) struct Resolved {
    pub transport: Arc<dyn Transport>,
    pub clock: Option<Arc<dyn crate::simulation::Clock>>,
}
type Factory = dyn Fn(&TransportConfig) -> Arc<dyn Transport> + Send + Sync;
/// An HTTP/WebSocket provider. Native is the default; custom providers must describe their
/// capabilities honestly. Reqwest is optional and unavailable on wasm and HarmonyOS.
#[derive(Clone, Default)]
pub enum Provider {
    #[default]
    Native,
    Custom(Arc<Factory>),
    #[cfg(all(
        feature = "reqwest",
        not(target_arch = "wasm32"),
        not(target_env = "ohos")
    ))]
    Reqwest,
}
impl Provider {
    pub fn custom(
        factory: impl Fn(&TransportConfig) -> Arc<dyn Transport> + Send + Sync + 'static,
    ) -> Self {
        Self::Custom(Arc::new(factory))
    }
    pub(crate) fn build(&self, config: &TransportConfig) -> Arc<dyn Transport> {
        match self {
            Self::Native => crate::platform_transport(config),
            Self::Custom(f) => f(config),
            #[cfg(all(
                feature = "reqwest",
                not(target_arch = "wasm32"),
                not(target_env = "ohos")
            ))]
            Self::Reqwest => Arc::new(crate::reqwest_provider::Reqwest::new(config)),
        }
    }
}
struct Settings {
    provider: Provider,
    generation: u64,
    simulation: Option<Simulation>,
}
struct Inner {
    settings: Mutex<Settings>,
    stats: Arc<Store>,
}
/// An isolated routing and statistics scope. Clones share settings. A new session uses the
/// native provider and does not inherit global interception. Default clients use `global()`.
#[derive(Clone)]
pub struct Session(Arc<Inner>);
impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}
impl Session {
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            settings: Mutex::new(Settings {
                provider: Provider::Native,
                generation: 0,
                simulation: None,
            }),
            stats: Arc::new(Store::default()),
        }))
    }
    pub fn global() -> Self {
        static GLOBAL: OnceLock<Session> = OnceLock::new();
        GLOBAL.get_or_init(Self::new).clone()
    }
    /// Applies to subsequent requests, including clients that already exist.
    pub fn set_provider(&self, provider: Provider) {
        let mut s = lock(&self.0.settings);
        s.provider = provider;
        s.generation += 1;
    }
    /// Enable/disable interception. Active transfers keep their selected handler and provider.
    pub fn set_simulation(&self, simulation: Option<Simulation>) {
        lock(&self.0.settings).simulation = simulation;
    }
    pub fn statistics(&self) -> crate::Statistics {
        self.0.stats.snapshot()
    }
    /// The latest 256 exchanges/sockets, including in-progress operations; totals are unbounded
    /// counters, but history storage is bounded. These are not physical TCP connection counts.
    pub fn transfers(&self) -> Vec<crate::TransferStatistics> {
        self.0.stats.transfers()
    }
    pub(crate) fn resolve(
        &self,
        config: &TransportConfig,
        cache: &Mutex<Option<(u64, Arc<dyn Transport>)>>,
        url: &str,
        method: &str,
    ) -> Resolved {
        let simulation = lock(&self.0.settings).simulation.clone();
        let simulation = simulation.filter(|s| s.intercepts(method, url));
        let (selected, clock, simulated) = match simulation {
            Some(s) => (s.transport(), Some(s.clock()), true),
            None => (self.provider_transport(config, cache), None, false),
        };
        Resolved {
            transport: crate::statistics::observe(selected, self.0.stats.clone(), simulated),
            clock,
        }
    }

    pub(crate) fn provider_transport(
        &self,
        config: &TransportConfig,
        cache: &Mutex<Option<(u64, Arc<dyn Transport>)>>,
    ) -> Arc<dyn Transport> {
        let (provider, generation) = {
            let s = lock(&self.0.settings);
            (s.provider.clone(), s.generation)
        };
        let cached = lock(cache)
            .as_ref()
            .filter(|(g, _)| *g == generation)
            .map(|(_, t)| t.clone());
        cached.unwrap_or_else(|| {
            let t = provider.build(config);
            *lock(cache) = Some((generation, t.clone()));
            t
        })
    }
    pub(crate) fn observe(&self, t: Arc<dyn Transport>) -> Arc<dyn Transport> {
        crate::statistics::observe(t, self.0.stats.clone(), false)
    }
}

pub(crate) fn simulated_capabilities() -> Capabilities {
    Capabilities {
        streaming: true,
        upload_streaming: true,
        upload_progress: true,
        manual_redirects: true,
        websockets: true,
        websocket_ping: true,
        websocket_headers: true,
        ..Default::default()
    }
}
