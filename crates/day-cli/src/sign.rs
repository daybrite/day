//! `day sign` v0 (DESIGN.md §16.5): `--check` validates the presence and resolvability of the
//! Day.toml `signing:` configuration (env vars set, referenced files exist) WITHOUT ever printing
//! a secret value; `--notarize-status <id>` polls an async notarytool submission. Actual signing
//! runs inside `day pack` (the per-format modules in pack/).

use std::path::Path;
use std::process::Command;

use crate::meta::Project;
use crate::ops::status;
use crate::pack::settings::{interpolate, interpolate_opt};

/// One section's readiness. `configured=false` is not an error — pack degrades to the dev tier.
struct Check {
    section: &'static str,
    configured: bool,
    problems: Vec<String>,
}

/// `day sign --check`: exit 0 when every configured section resolves; 6 when any fails (§16.3).
pub fn check(project: &Project) -> i32 {
    let signing = project.manifest.signing.as_ref();
    let mut checks = Vec::new();

    // -- macos ---------------------------------------------------------------
    {
        let macos = signing.and_then(|s| s.macos.as_ref());
        let mut c = Check {
            section: "macos",
            configured: macos.is_some(),
            problems: Vec::new(),
        };
        if let Some(m) = macos {
            match interpolate_opt(m.identity.as_ref()) {
                Ok(Some(id)) if id == "-" || id.is_empty() => {
                    c.problems.push("identity resolves to ad-hoc".into())
                }
                Ok(_) => {}
                Err(e) => c.problems.push(e),
            }
            if let Some(e) = &m.entitlements
                && !project.root.join(e).exists()
            {
                c.problems.push(format!("entitlements file missing: {e}"));
            }
            if let Some(n) = &m.notarize {
                for (label, raw) in [("key-id", &n.key_id), ("issuer", &n.issuer)] {
                    if let Err(e) = interpolate(raw) {
                        c.problems.push(format!("notarize.{label}: {e}"));
                    }
                }
                match interpolate(&n.key_path) {
                    Ok(p) if !Path::new(&p).exists() => {
                        c.problems.push(format!("notarize.key-path missing: {p}"))
                    }
                    Ok(_) => {}
                    Err(e) => c.problems.push(format!("notarize.key-path: {e}")),
                }
            } else {
                c.problems
                    .push("no notarize config (dmg will not pass Gatekeeper)".into());
            }
        }
        checks.push(c);
    }

    // -- ios -------------------------------------------------------------------
    {
        let ios = signing.and_then(|s| s.ios.as_ref());
        let mut c = Check {
            section: "ios",
            configured: ios.is_some(),
            problems: Vec::new(),
        };
        if let Some(i) = ios {
            if let Err(e) = interpolate(&i.team) {
                c.problems.push(format!("team: {e}"));
            }
            let triple = [
                ("key-id", &i.key_id),
                ("issuer", &i.issuer),
                ("key-path", &i.key_path),
            ];
            let set = triple.iter().filter(|(_, v)| v.is_some()).count();
            if set != 0 && set != 3 {
                c.problems
                    .push("key-id, issuer and key-path must be set together".into());
            }
            if let Some(raw) = &i.key_path {
                match interpolate(raw) {
                    Ok(p) if !Path::new(&p).exists() => {
                        c.problems.push(format!("key-path missing: {p}"))
                    }
                    Ok(_) => {}
                    Err(e) => c.problems.push(format!("key-path: {e}")),
                }
            }
        }
        checks.push(c);
    }

    // -- android -----------------------------------------------------------------
    {
        let android = signing.and_then(|s| s.android.as_ref());
        let mut c = Check {
            section: "android",
            configured: android.is_some(),
            problems: Vec::new(),
        };
        if let Some(a) = android {
            match interpolate(&a.keystore) {
                Ok(p) if !project.root.join(&p).exists() => {
                    c.problems.push(format!("keystore missing: {p}"))
                }
                Ok(_) => {}
                Err(e) => c.problems.push(format!("keystore: {e}")),
            }
            for (label, raw) in [
                ("key-alias", &a.key_alias),
                ("store-pass", &a.store_pass),
                ("key-pass", &a.key_pass),
            ] {
                if let Err(e) = interpolate(raw) {
                    c.problems.push(format!("{label}: {e}"));
                }
            }
        }
        checks.push(c);
    }

    // -- windows -------------------------------------------------------------------
    {
        let windows = signing.and_then(|s| s.windows.as_ref());
        let mut c = Check {
            section: "windows",
            configured: windows.is_some(),
            problems: Vec::new(),
        };
        if windows.is_some()
            && let Err(e) = crate::pack::msix_check(project)
        {
            c.problems.push(e);
        }
        checks.push(c);
    }

    // -- ohos ----------------------------------------------------------------------
    {
        let ohos = signing.and_then(|s| s.ohos.as_ref());
        let mut c = Check {
            section: "ohos",
            configured: ohos.is_some(),
            problems: Vec::new(),
        };
        if let Some(o) = ohos {
            for (label, raw) in [
                ("keystore", &o.keystore),
                ("cert", &o.cert),
                ("profile", &o.profile),
            ] {
                match interpolate(raw) {
                    Ok(p) if !project.root.join(&p).exists() => {
                        c.problems.push(format!("{label} missing: {p}"))
                    }
                    Ok(_) => {}
                    Err(e) => c.problems.push(format!("{label}: {e}")),
                }
            }
            for (label, raw) in [
                ("key-alias", &o.key_alias),
                ("store-pass", &o.store_pass),
                ("key-pass", &o.key_pass),
            ] {
                if let Err(e) = interpolate(raw) {
                    c.problems.push(format!("{label}: {e}"));
                }
            }
        }
        checks.push(c);
    }

    // self-signed-dev is a resolvable provider but still the dev tier — say so, don't call it ready.
    let windows_dev_provider = signing
        .and_then(|s| s.windows.as_ref())
        .is_some_and(|w| w.provider == "self-signed-dev");

    let mut failing = false;
    for c in &checks {
        if !c.configured {
            status(
                "Sign",
                &format!("{}: not configured (pack uses the dev tier)", c.section),
            );
        } else if c.problems.is_empty() {
            if c.section == "windows" && windows_dev_provider {
                status(
                    "Sign",
                    "windows: self-signed-dev provider (dev tier — not distributable)",
                );
            } else {
                status(
                    "Sign",
                    &format!("{}: ok (release signing ready)", c.section),
                );
            }
        } else {
            failing = true;
            status(
                "Sign",
                &format!("{}: NOT ready — {}", c.section, c.problems.join("; ")),
            );
        }
    }
    if failing { 6 } else { 0 }
}

/// `day sign --notarize-status <id>`: the async-CI half of `pack --no-wait` (§16.5).
pub fn notarize_status(project: &Project, id: &str) -> i32 {
    let Some(n) = project
        .manifest
        .signing
        .as_ref()
        .and_then(|s| s.macos.as_ref())
        .and_then(|m| m.notarize.as_ref())
    else {
        eprintln!("error: no signing.macos.notarize config in Day.toml");
        return 6;
    };
    let (key_id, issuer, key_path) = match (
        interpolate(&n.key_id),
        interpolate(&n.issuer),
        interpolate(&n.key_path),
    ) {
        (Ok(k), Ok(i), Ok(p)) => (k, i, p),
        (k, i, p) => {
            for e in [k.err(), i.err(), p.err()].into_iter().flatten() {
                eprintln!("error: {e}");
            }
            return 6;
        }
    };
    let ok = Command::new("xcrun")
        .args(["notarytool", "info", id])
        .args(["--key", &key_path, "--key-id", &key_id, "--issuer", &issuer])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok { 0 } else { 6 }
}
