//! Best-effort "is there a newer day-cli on crates.io?" check.
//!
//! [`spawn`] kicks off the crates.io query on a background thread the moment the CLI starts, so it
//! runs concurrently with whatever command the user asked for. [`finish`] is called right before the
//! process exits: it polls the result WITHOUT blocking — if the reply already landed (i.e. the command
//! took long enough) and a newer stable release exists, it prints a one-line yellow nudge; if the reply
//! isn't back yet it just returns, and the detached worker thread is torn down by process exit. So the
//! check never delays the CLI: a slow command "pays" for it for free, a fast one simply skips it.
//!
//! Silent (and makes NO network call) on debug builds, when the `DAY_NO_UPDATE_CHECK` env var is set
//! — the opt-out for anyone who wants day to stay fully offline, since this is its only outbound call
//! — and whenever `spawn` is called with `enabled = false` (the build-system plumbing callbacks and
//! `--format json` machine output opt out — see `cli::run`).

use std::sync::mpsc::Receiver;

use crate::term::WARN;

/// Set this env var (to any non-empty value) to skip the update check entirely — and with it day's
/// only outbound network call. Follows the `NO_COLOR` convention: present + non-empty ⇒ disabled.
const DISABLE_ENV: &str = "DAY_NO_UPDATE_CHECK";

/// The crates.io metadata endpoint returns `{ "crate": { "max_stable_version": "x.y.z", … }, … }`.
#[derive(serde::Deserialize)]
struct CratesResp {
    #[serde(rename = "crate")]
    krate: CrateInfo,
}
#[derive(serde::Deserialize)]
struct CrateInfo {
    /// Newest NON-prerelease, non-yanked version — exactly what a `cargo install day-cli` would pick.
    max_stable_version: Option<String>,
}

/// Start the background crates.io check, returning a channel [`finish`] polls at exit. Returns `None`
/// (no check, no thread — so no network call at all) on debug builds, when `DAY_NO_UPDATE_CHECK` is
/// set, or when `enabled` is false.
pub fn spawn(enabled: bool) -> Option<Receiver<String>> {
    // Presence (non-empty) opts out — the NO_COLOR convention. Lets anyone stop day from touching the
    // network at all, since this check is its only outbound call.
    let opted_out = std::env::var_os(DISABLE_ENV).is_some_and(|v| !v.is_empty());
    if cfg!(debug_assertions) || !enabled || opted_out {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        if let Some(latest) = fetch_latest() {
            // Ignore a send error: the receiver being gone means the process is already exiting.
            let _ = tx.send(latest);
        }
    });
    Some(rx)
}

/// Poll the check without blocking. If the crates.io reply already arrived and names a newer stable
/// release than this build, print a yellow update nudge to stderr. Called just before the CLI exits.
pub fn finish(rx: Option<Receiver<String>>) {
    // No check was started, or the reply hasn't arrived yet (`try_recv` never blocks) → nothing to do.
    let Some(rx) = rx else { return };
    let Ok(latest) = rx.try_recv() else { return };

    let current = env!("CARGO_PKG_VERSION");
    if is_newer(&latest, current) {
        eprintln!(
            "{WARN}A new release of day-cli is available: {current} → {latest}. \
             Update with `cargo install day-cli`.{WARN:#}"
        );
    }
}

/// GET the crate metadata from crates.io with a descriptive User-Agent (crates.io rejects generic
/// ones), returning its newest stable version. Any failure (offline, timeout, 404 before the crate is
/// published, malformed JSON) yields `None` — the check is best-effort and never surfaces errors.
fn fetch_latest() -> Option<String> {
    let ua = format!(
        "day-cli/{} ({}; {}; +{})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        env!("CARGO_PKG_REPOSITORY"),
    );
    let body = ureq::get("https://crates.io/api/v1/crates/day-cli")
        .header("User-Agent", &ua)
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(5)))
        .build()
        .call()
        .ok()?
        .into_body()
        .read_to_string()
        .ok()?;
    serde_json::from_str::<CratesResp>(&body)
        .ok()?
        .krate
        .max_stable_version
}

/// `latest > current`, comparing only the numeric `major.minor.patch` core (crates.io's stable
/// version carries no pre-release/build suffix). Unparseable input compares as "not newer".
fn is_newer(latest: &str, current: &str) -> bool {
    match (parse(latest), parse(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.split(['-', '+']).next().unwrap_or(v);
    let mut parts = core.split('.');
    let major = parts.next()?.trim().parse().ok()?;
    let minor = parts.next()?.trim().parse().ok()?;
    let patch = parts.next()?.trim().parse().ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::{is_newer, parse};

    #[test]
    fn parses_semver_core() {
        assert_eq!(parse("0.0.5"), Some((0, 0, 5)));
        assert_eq!(parse("1.2.3-alpha.1"), Some((1, 2, 3)));
        assert_eq!(parse("2.0.0+build.7"), Some((2, 0, 0)));
        assert_eq!(parse("nonsense"), None);
        assert_eq!(parse("1.2"), None);
    }

    #[test]
    fn newer_only_when_strictly_greater() {
        assert!(is_newer("0.1.0", "0.0.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.0.6", "0.0.5"));
        assert!(!is_newer("0.0.5", "0.0.5")); // equal → no nudge
        assert!(!is_newer("0.0.4", "0.0.5")); // older on crates.io (e.g. a git build) → no nudge
        assert!(!is_newer("garbage", "0.0.5")); // unparseable → no nudge
    }
}
