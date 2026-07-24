//! The package store (docs/lite.md §8): manifests, install/update plans, permission grants,
//! and the on-disk cache. An origin is either `https://…` (any static host — a raw git
//! branch URL works) or a local directory path (the dev loop: files re-read every launch,
//! nothing cached).
//!
//! Disk layout under the store root:
//! ```text
//! <root>/<app_id>/pkg/…            fetched files (manifest.json + day.files)
//! <root>/<app_id>/install.json     InstallRecord (origin, version, grants, hashes)
//! <root>/<app_id>/app.sqlite       day.db (db.rs)
//! <root>/<app_id>/fs/…             the day.fs sandbox (fsx.rs)
//! ```

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// W3C MiniApp-shaped manifest with the `day` extension block (docs/lite.md §3).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub app_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icons: Vec<Icon>,
    pub version: Version,
    #[serde(default)]
    pub platform_version: Option<PlatformVersion>,
    #[serde(default)]
    pub pages: Vec<String>,
    #[serde(default)]
    pub req_permissions: Vec<Permission>,
    #[serde(default)]
    pub window: Window,
    #[serde(default)]
    pub day: DayExt,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Icon {
    pub src: String,
    #[serde(default)]
    pub sizes: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Version {
    pub code: u64,
    #[serde(default)]
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlatformVersion {
    pub min_code: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Permission {
    pub name: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Window {
    #[serde(default)]
    pub background_color: Option<String>,
    #[serde(default)]
    pub orientation: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DayExt {
    /// Entry module; `app.ts` then `app.js` when absent.
    #[serde(default)]
    pub entry: Option<String>,
    /// Complete fetch list (manifest + entry are implicit).
    #[serde(default)]
    pub files: Vec<String>,
    /// URL prefixes `day.net.fetch` may touch (empty = fetch always rejects).
    #[serde(default)]
    pub net_origins: Vec<String>,
}

impl Manifest {
    pub fn entry(&self) -> &str {
        self.day.entry.as_deref().unwrap_or("app.ts")
    }

    pub fn parse(bytes: &[u8]) -> Result<Manifest, StoreError> {
        serde_json::from_slice(bytes).map_err(|e| StoreError::Manifest(e.to_string()))
    }
}

/// What's persisted per installed app.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InstallRecord {
    pub origin: String,
    pub version_code: u64,
    pub granted: Vec<String>,
    /// Path → content hash of the installed files (the update diff base).
    pub hashes: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub enum StoreError {
    Fetch {
        path: String,
        detail: String,
    },
    Manifest(String),
    Io(String),
    /// Origin scheme is neither https nor a local directory.
    BadOrigin(String),
    NotInstalled(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Fetch { path, detail } => write!(f, "fetching {path}: {detail}"),
            StoreError::Manifest(e) => write!(f, "manifest: {e}"),
            StoreError::Io(e) => write!(f, "store io: {e}"),
            StoreError::BadOrigin(o) => {
                write!(f, "origin `{o}` must be https:// or a local directory")
            }
            StoreError::NotInstalled(id) => write!(f, "`{id}` is not installed"),
        }
    }
}

impl std::error::Error for StoreError {}

/// An origin: where a miniapp's files come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Https(String),
    Local(PathBuf),
}

impl Origin {
    pub fn parse(s: &str) -> Result<Origin, StoreError> {
        let s = s.trim().trim_end_matches('/');
        if s.starts_with("https://") {
            Ok(Origin::Https(s.to_string()))
        } else if s.starts_with('/') || s.starts_with("file://") {
            let p = s.strip_prefix("file://").unwrap_or(s);
            Ok(Origin::Local(PathBuf::from(p)))
        } else {
            Err(StoreError::BadOrigin(s.into()))
        }
    }

    pub fn as_str(&self) -> String {
        match self {
            Origin::Https(u) => u.clone(),
            Origin::Local(p) => p.display().to_string(),
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Origin::Local(_))
    }

    /// Read one file from the origin. Blocking on https (call from a background task).
    pub fn read(&self, rel: &str) -> Result<Vec<u8>, StoreError> {
        match self {
            Origin::Local(root) => {
                let p = root.join(rel);
                std::fs::read(&p).map_err(|e| StoreError::Fetch {
                    path: p.display().to_string(),
                    detail: e.to_string(),
                })
            }
            Origin::Https(base) => {
                let url = format!("{base}/{rel}");
                let req = day_part_http::Request::get(&url);
                let resp = day_part_http::fetch(&req).map_err(|e| StoreError::Fetch {
                    path: url.clone(),
                    detail: format!("{e:?}"),
                })?;
                if resp.status != 200 {
                    return Err(StoreError::Fetch {
                        path: url,
                        detail: format!("HTTP {}", resp.status),
                    });
                }
                Ok(resp.body)
            }
        }
    }
}

/// The store root plus operations. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

/// Step 1 of install (docs/lite.md §8): the fetched manifest, held until the embedding UI
/// passes the user-granted permission set to [`InstallPlan::confirm`].
pub struct InstallPlan {
    pub origin: Origin,
    pub manifest: Manifest,
    store: Store,
}

pub struct UpdatePlan {
    pub app_id: String,
    pub from_code: u64,
    pub manifest: Manifest,
    /// Permissions in the new manifest that the current grant set does not cover — the UI
    /// must re-disclose when non-empty.
    pub new_permissions: Vec<Permission>,
    origin: Origin,
    store: Store,
}

fn hash(bytes: &[u8]) -> String {
    // Content identity only (update diffing), not security: FNV-1a 64, hex.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1_0000_01b3);
    }
    format!("{h:016x}")
}

fn io<T>(r: std::io::Result<T>) -> Result<T, StoreError> {
    r.map_err(|e| StoreError::Io(e.to_string()))
}

impl Store {
    pub fn at(root: impl Into<PathBuf>) -> Store {
        Store { root: root.into() }
    }

    pub fn app_dir(&self, app_id: &str) -> PathBuf {
        // App ids are reverse-domain; keep the path safe regardless.
        let safe: String = app_id
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.root.join(safe)
    }

    pub fn pkg_dir(&self, app_id: &str) -> PathBuf {
        self.app_dir(app_id).join("pkg")
    }

    fn record_path(&self, app_id: &str) -> PathBuf {
        self.app_dir(app_id).join("install.json")
    }

    pub fn record(&self, app_id: &str) -> Option<InstallRecord> {
        let bytes = std::fs::read(self.record_path(app_id)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn manifest(&self, app_id: &str) -> Result<Manifest, StoreError> {
        let rec = self
            .record(app_id)
            .ok_or_else(|| StoreError::NotInstalled(app_id.into()))?;
        let origin = Origin::parse(&rec.origin)?;
        if origin.is_local() {
            // Dev loop: always the working tree's current manifest.
            return Manifest::parse(&origin.read("manifest.json")?);
        }
        Manifest::parse(&io(std::fs::read(
            self.pkg_dir(app_id).join("manifest.json"),
        ))?)
    }

    pub fn installed(&self) -> Vec<(String, Manifest)> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return out;
        };
        for e in entries.flatten() {
            let id = e.file_name().to_string_lossy().to_string();
            if self.record(&id).is_some()
                && let Ok(m) = self.manifest(&id)
            {
                out.push((id, m));
            }
        }
        out.sort_by(|a, b| a.1.name.cmp(&b.1.name));
        out
    }

    /// Read a package file for a running app (local origins read the working tree live).
    pub fn read_file(&self, app_id: &str, rel: &str) -> Result<Vec<u8>, StoreError> {
        let rec = self
            .record(app_id)
            .ok_or_else(|| StoreError::NotInstalled(app_id.into()))?;
        let origin = Origin::parse(&rec.origin)?;
        if origin.is_local() {
            return origin.read(rel);
        }
        io(std::fs::read(self.pkg_dir(app_id).join(rel)))
    }

    /// Fetch the manifest and produce the plan the UI confirms. Blocking on https origins.
    pub fn install(&self, origin: &str) -> Result<InstallPlan, StoreError> {
        let origin = Origin::parse(origin)?;
        let manifest = Manifest::parse(&origin.read("manifest.json")?)?;
        Ok(InstallPlan {
            origin,
            manifest,
            store: self.clone(),
        })
    }

    pub fn check_update(&self, app_id: &str) -> Result<Option<UpdatePlan>, StoreError> {
        let rec = self
            .record(app_id)
            .ok_or_else(|| StoreError::NotInstalled(app_id.into()))?;
        let origin = Origin::parse(&rec.origin)?;
        let manifest = Manifest::parse(&origin.read("manifest.json")?)?;
        if manifest.version.code <= rec.version_code {
            return Ok(None);
        }
        let new_permissions = manifest
            .req_permissions
            .iter()
            .filter(|p| !rec.granted.contains(&p.name))
            .cloned()
            .collect();
        Ok(Some(UpdatePlan {
            app_id: app_id.into(),
            from_code: rec.version_code,
            manifest,
            new_permissions,
            origin,
            store: self.clone(),
        }))
    }

    pub fn remove(&self, app_id: &str) -> Result<(), StoreError> {
        io(std::fs::remove_dir_all(self.app_dir(app_id)))
    }

    pub fn set_granted(&self, app_id: &str, granted: Vec<String>) -> Result<(), StoreError> {
        let mut rec = self
            .record(app_id)
            .ok_or_else(|| StoreError::NotInstalled(app_id.into()))?;
        rec.granted = granted;
        self.write_record(app_id, &rec)
    }

    fn write_record(&self, app_id: &str, rec: &InstallRecord) -> Result<(), StoreError> {
        io(std::fs::create_dir_all(self.app_dir(app_id)))?;
        io(std::fs::write(
            self.record_path(app_id),
            serde_json::to_vec_pretty(rec).map_err(|e| StoreError::Io(e.to_string()))?,
        ))
    }

    fn fetch_all(
        &self,
        origin: &Origin,
        manifest: &Manifest,
        old_hashes: Option<&HashMap<String, String>>,
    ) -> Result<HashMap<String, String>, StoreError> {
        let pkg = self.pkg_dir(&manifest.app_id);
        io(std::fs::create_dir_all(&pkg))?;
        let mut files: Vec<String> = manifest.day.files.clone();
        let entry = manifest.entry().to_string();
        if !files.contains(&entry) {
            files.push(entry);
        }
        let mut hashes = HashMap::new();
        // The manifest itself is written last: an interrupted install/update never leaves a
        // manifest whose files are missing.
        for rel in &files {
            let bytes = origin.read(rel)?;
            let h = hash(&bytes);
            let unchanged = old_hashes.is_some_and(|m| m.get(rel) == Some(&h));
            if !unchanged {
                let dest = pkg.join(rel);
                if let Some(parent) = dest.parent() {
                    io(std::fs::create_dir_all(parent))?;
                }
                io(std::fs::write(dest, &bytes))?;
            }
            hashes.insert(rel.clone(), h);
        }
        let mbytes =
            serde_json::to_vec_pretty(manifest).map_err(|e| StoreError::Io(e.to_string()))?;
        hashes.insert("manifest.json".into(), hash(&mbytes));
        io(std::fs::write(pkg.join("manifest.json"), mbytes))?;
        Ok(hashes)
    }
}

impl InstallPlan {
    /// Fetch everything and persist. `granted` is the disclosure outcome the UI collected.
    /// Blocking on https origins.
    pub fn confirm(self, granted: &[String]) -> Result<Manifest, StoreError> {
        let hashes = if self.origin.is_local() {
            HashMap::new() // local files are read live, never cached
        } else {
            self.store.fetch_all(&self.origin, &self.manifest, None)?
        };
        self.store.write_record(
            &self.manifest.app_id,
            &InstallRecord {
                origin: self.origin.as_str(),
                version_code: self.manifest.version.code,
                granted: granted.to_vec(),
                hashes,
            },
        )?;
        Ok(self.manifest)
    }
}

impl UpdatePlan {
    /// Fetch changed files (hash diff) and swap the record. Blocking on https origins.
    /// `extra_granted` covers [`UpdatePlan::new_permissions`] the UI re-disclosed.
    pub fn apply(self, extra_granted: &[String]) -> Result<Manifest, StoreError> {
        let rec = self
            .store
            .record(&self.app_id)
            .ok_or_else(|| StoreError::NotInstalled(self.app_id.clone()))?;
        let hashes = if self.origin.is_local() {
            HashMap::new()
        } else {
            self.store
                .fetch_all(&self.origin, &self.manifest, Some(&rec.hashes))?
        };
        let mut granted = rec.granted.clone();
        for g in extra_granted {
            if !granted.contains(g) {
                granted.push(g.clone());
            }
        }
        self.store.write_record(
            &self.app_id,
            &InstallRecord {
                origin: self.origin.as_str(),
                version_code: self.manifest.version.code,
                granted,
                hashes,
            },
        )?;
        Ok(self.manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn write_miniapp(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            r#"{ "app_id": "org.example.t", "name": "T",
                 "version": { "code": 2, "name": "0.2" },
                 "pages": ["home"],
                 "req_permissions": [{ "name": "day.permission.NETWORK", "reason": "r" }],
                 "day": { "files": ["app.ts"] } }"#,
        )
        .unwrap();
        std::fs::write(dir.join("app.ts"), "page('home', () => label('hi'))").unwrap();
    }

    #[test]
    fn local_install_update_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("day-lite-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let app = tmp.join("src");
        write_miniapp(&app);
        let store = Store::at(tmp.join("store"));

        let plan = store.install(app.to_str().unwrap()).expect("plan");
        assert_eq!(plan.manifest.app_id, "org.example.t");
        assert_eq!(plan.manifest.req_permissions.len(), 1);
        let m = plan
            .confirm(&["day.permission.NETWORK".into()])
            .expect("confirm");
        assert_eq!(m.version.code, 2);

        // Installed + readable through the store (local = live).
        assert_eq!(store.installed().len(), 1);
        let src = store.read_file("org.example.t", "app.ts").expect("read");
        assert!(String::from_utf8_lossy(&src).contains("page("));

        // No update at the same version; bumping the code produces a plan.
        assert!(
            store
                .check_update("org.example.t")
                .expect("check")
                .is_none()
        );
        let manifest2 = std::fs::read_to_string(app.join("manifest.json"))
            .unwrap()
            .replace("\"code\": 2", "\"code\": 3");
        std::fs::write(app.join("manifest.json"), manifest2).unwrap();
        let up = store
            .check_update("org.example.t")
            .expect("check")
            .expect("plan");
        assert_eq!(up.from_code, 2);
        assert!(up.new_permissions.is_empty());
        up.apply(&[]).expect("apply");
        assert_eq!(store.record("org.example.t").unwrap().version_code, 3);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
