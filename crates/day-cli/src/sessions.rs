// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Live launch sessions (`build/day/sessions.json`): target → dayscript-engine coordinates.
//!
//! Every `day launch` records where the app's engine listens (loopback port + token), so a later
//! process (`day drive`, `day stop`, `day relaunch --all-running`, `day mcp-server`, and through
//! it any coding agent) can attach to an app the developer already has open. Best-effort JSON:
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
    let mut sessions: Vec<Session> = std::fs::read_to_string(file(root))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    for session in &mut sessions {
        session.target = crate::targets::canonical(&session.target).to_string();
    }
    // An old spelling and a new spelling can refer to the same live target. Keep
    // the newest launch, matching record's one-session-per-target contract.
    sessions.sort_by_key(|s| std::cmp::Reverse(s.started_at));
    let mut seen = std::collections::BTreeSet::new();
    sessions.retain(|s| seen.insert(s.target.clone()));
    sessions
}

fn write(root: &Path, sessions: &[Session]) {
    let path = file(root);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(sessions) {
        // Temp file + rename, in the same directory so the rename stays atomic: `record` and
        // `remove` are read-modify-write with no lock, and parallel `day` invocations writing
        // in place could hand each other (and every reader) a torn file. A rename never can;
        // the last writer wins whole. The pid keys the temp name so two writers don't collide
        // on it either.
        let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

/// Upsert the entry for `session.target` (one live session per target).
pub fn record(root: &Path, mut session: Session) {
    session.target = crate::targets::canonical(&session.target).to_string();
    let mut all = list(root);
    all.retain(|s| s.target != session.target);
    all.push(session);
    write(root, &all);
}

pub fn remove(root: &Path, target: &str) {
    let target = crate::targets::canonical(target);
    let mut all = list(root);
    all.retain(|s| s.target != target);
    write(root, &all);
}

pub fn find(root: &Path, target: &str) -> Option<Session> {
    let target = crate::targets::canonical(target);
    list(root).into_iter().find(|s| s.target == target)
}

/// Whether the engine answers on its port right now (direct loopback probe, meaningful for
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_sessions_find_the_latest_launch_and_write_only_canonical_keys() {
        let root = std::env::temp_dir().join(format!(
            "day-session-alias-{}-{}",
            std::process::id(),
            now_millis()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let session = |target: &str, started_at| Session {
            target: target.to_owned(),
            app_id: "dev.example.fixture".into(),
            profile: "debug".into(),
            engine_port: 1234,
            engine_token: "synthetic-test-token".into(),
            started_at,
        };
        write(
            &root,
            &[
                session("android-mdc", 2),
                session("android-material", 1),
                session("web-dom", 3),
            ],
        );
        let found = find(&root, "android-mdc").unwrap();
        assert_eq!(found.started_at, 2);
        assert_eq!(found.target, "android-material");
        record(&root, session("android-mdc", 4));
        let saved = std::fs::read_to_string(file(&root)).unwrap();
        assert!(!saved.contains("android-mdc"));
        assert_eq!(list(&root).len(), 2);
        remove(&root, "android-mdc");
        assert_eq!(list(&root)[0].target, "web-dom");
        std::fs::remove_dir_all(root).unwrap();
    }
}
