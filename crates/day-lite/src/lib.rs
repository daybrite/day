//! day-lite: dynamic JS/TS miniapps on day (docs/lite.md).
//!
//! A **superapp** (a compiled day app) embeds a [`Host`], which owns the package [`Store`]
//! and the set of [`Bridge`]s — native capabilities exposed to scripts, each behind a
//! permission id. [`Host::launch`] boots a miniapp's QuickJS runtime and returns its UI as
//! a plain `AnyPiece`. See `apps/daylite` for the reference superapp and docs/lite.md for
//! the whole model.

mod bridges;
mod db;
mod engine;
mod fsx;
mod i18n;
mod sleep;
mod store;
mod testrun;
mod ts;
mod value;

pub use bridges::{Bridge, net, prefs};
pub use db::{Cell, Db, DbError};
pub use engine::{LiteApp, NavEntry};
pub use fsx::{EntryKind, FsError, Sandbox};
pub use sleep::sleep_ms;
pub use store::{
    InstallPlan, InstallRecord, Manifest, Origin, Permission, Store, StoreError, UpdatePlan,
};
pub use testrun::{TestOutcome, run_tests};

/// Well-known permission ids (hosts may add their own).
pub mod permission {
    pub const NETWORK: &str = "day.permission.NETWORK";
    pub const STORAGE: &str = "day.permission.STORAGE";
    pub const PREFS: &str = "day.permission.PREFS";
    pub const FS: &str = "day.permission.FS";
    pub const SENSORS: &str = "day.permission.SENSORS";
}

/// The effective grant set for one running app: what the user granted at install (plus the
/// host's implicit grants).
#[derive(Clone, Debug, Default)]
pub struct PermissionSet {
    granted: Vec<String>,
}

impl PermissionSet {
    pub fn new(granted: Vec<String>) -> PermissionSet {
        PermissionSet { granted }
    }

    pub fn granted(&self, permission: &str) -> bool {
        self.granted.iter().any(|g| g == permission)
    }
}

/// The embedding surface (docs/lite.md §10).
pub struct Host {
    store: Store,
    bridges: Vec<Bridge>,
    implicit: Vec<String>,
}

pub struct HostBuilder {
    store: Option<Store>,
    bridges: Vec<Bridge>,
    implicit: Vec<String>,
}

impl Host {
    pub fn builder() -> HostBuilder {
        HostBuilder {
            store: None,
            bridges: Vec::new(),
            // Storage-class permissions touch only app-private data; granted implicitly
            // (still declared in the manifest for disclosure). Hosts can override.
            implicit: vec![
                permission::STORAGE.to_string(),
                permission::PREFS.to_string(),
                permission::FS.to_string(),
            ],
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Boot an installed miniapp. The returned [`LiteApp`] owns the JS runtime; its
    /// [`LiteApp::surface`] is the UI. Drop the app to tear the runtime down.
    pub fn launch(&self, app_id: &str) -> Result<LiteApp, String> {
        let record = self
            .store
            .record(app_id)
            .ok_or_else(|| format!("`{app_id}` is not installed"))?;
        let mut granted = record.granted.clone();
        for p in &self.implicit {
            if !granted.contains(p) {
                granted.push(p.clone());
            }
        }
        LiteApp::boot(
            self.store.clone(),
            app_id,
            &self.bridges,
            PermissionSet::new(granted),
        )
    }
}

impl HostBuilder {
    pub fn store(mut self, store: Store) -> HostBuilder {
        self.store = Some(store);
        self
    }

    pub fn bridge(mut self, bridge: Bridge) -> HostBuilder {
        self.bridges.push(bridge);
        self
    }

    /// Replace the implicit-grant list (permissions every app gets without asking).
    pub fn implicit(mut self, permissions: &[&str]) -> HostBuilder {
        self.implicit = permissions.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn build(self) -> Host {
        Host {
            store: self
                .store
                .unwrap_or_else(|| Store::at(std::env::temp_dir().join("day-lite-store"))),
            bridges: self.bridges,
            implicit: self.implicit,
        }
    }
}
