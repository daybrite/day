//! Live launch sessions (`build/day/sessions.json`): target → dayscript-engine coordinates.
//!
//! Every `day launch` records where the app's engine listens (loopback port + token), so a LATER
//! process — `day drive`, `day stop`, `day relaunch --all-running`, `day mcp-server`, and through
//! it any coding agent — can attach to an app the developer already has open. Best-effort JSON:
//! entries are upserted per target on launch, dropped on stop, and replaced wholesale by a new
//! launch of the same target (docs/agent.md).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// Target name (`macos-appkit`, `ios-uikit`, …).
    pub target: String,
    /// The resolved application id the target launched with.
    pub app_id: String,
    /// Build profile (`debug` / `release`).
    pub profile: String,
    /// Host-side TCP port of the dayscript engine (loopback; devices reach it via forward).
    pub engine_port: u16,
    /// The per-launch token every engine request must carry.
    pub engine_token: String,
    /// Unix millis at launch.
    pub started_at: u64,
}

fn file(root: &Path) -> PathBuf {
    root.join("build/day/sessions.json")
}

pub fn list(root: &Path) -> Vec<Session> {
    std::fs::read_to_string(file(root))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write(root: &Path, sessions: &[Session]) {
    let path = file(root);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(sessions) {
        let _ = std::fs::write(path, json);
    }
}

/// Upsert the entry for `session.target` (one live session per target).
pub fn record(root: &Path, session: Session) {
    let mut all = list(root);
    all.retain(|s| s.target != session.target);
    all.push(session);
    write(root, &all);
}

pub fn remove(root: &Path, target: &str) {
    let mut all = list(root);
    all.retain(|s| s.target != target);
    write(root, &all);
}

pub fn find(root: &Path, target: &str) -> Option<Session> {
    list(root).into_iter().find(|s| s.target == target)
}

/// Whether the engine answers on its port right now (direct loopback probe — meaningful for
/// desktop and the iOS simulator; device targets need a forward first, so `None` = unknown).
pub fn reachable(session: &Session, direct: bool) -> Option<bool> {
    if !direct {
        return None;
    }
    Some(
        std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], session.engine_port)),
            std::time::Duration::from_millis(300),
        )
        .is_ok(),
    )
}

pub fn now_millis() -> u64 {
    std::time::UNIX_EPOCH
        .elapsed()
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
