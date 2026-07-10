//! ohos-arkui → .hap. With `signing.ohos` config the hvigor-built UNSIGNED hap is release-signed
//! via the SDK's hap-sign-tool (localSign, user keystore + release cert + provisioning profile);
//! without it the dev path stands (platform/ohos/sign-hap.mjs + the public OpenHarmony cert —
//! emulator installs only, dev tier).

use std::path::{Path, PathBuf};
use std::process::Command;

use super::settings::{PackOptions, resolve_degradable};
use super::{Artifact, PackError, SignTier, run_tool};
use crate::meta::Project;
use crate::ops::{self, status};
use crate::targets::Target;

pub fn pack(
    project: &Project,
    target: &'static Target,
    opts: &PackOptions,
    dist: &Path,
) -> Result<Artifact, PackError> {
    // Build assembles + dev-signs (build_ohos). The unsigned hap stays behind in entry/build —
    // release signing re-signs THAT, never the dev-signed one.
    let outcome = ops::build(project, target, &opts.profile).map_err(PackError::Other)?;
    let name = &project.manifest.app.name;
    let version = &project.manifest.app.version;
    let out = dist.join(format!("{name}-{version}.hap"));
    let _ = std::fs::remove_file(&out);

    let ohos = project
        .manifest
        .signing
        .as_ref()
        .and_then(|s| s.ohos.as_ref());
    // Unresolved release material degrades to the dev-signed hap (§20), loudly.
    let material = match ohos {
        Some(cfg) if !opts.no_sign => resolve_material(project, cfg)?,
        _ => None,
    };
    let tier = match material {
        Some(m) => {
            let unsigned = crate::ohos::find_unsigned_hap(project).ok_or_else(|| {
                PackError::Other("no unsigned .hap found under platform/ohos/entry/build".into())
            })?;
            let signed = project.root.join("build/day/pack/ohos-release.hap");
            std::fs::create_dir_all(signed.parent().unwrap())
                .map_err(|e| PackError::Other(e.to_string()))?;
            release_sign(&m, &unsigned, &signed).map_err(PackError::Sign)?;
            std::fs::copy(&signed, &out).map_err(|e| PackError::Other(e.to_string()))?;
            SignTier::Release
        }
        None => {
            if ohos.is_none() {
                status(
                    "Warning",
                    "no signing.ohos config — packing the dev-signed hap (emulator installs only)",
                );
            }
            std::fs::copy(&outcome.artifact, &out).map_err(|e| PackError::Other(e.to_string()))?;
            SignTier::DevSigned
        }
    };

    Ok(Artifact {
        path: out,
        kind: "hap",
        sha256: String::new(),
        tier,
    })
}

struct OhosMaterial {
    keystore: std::path::PathBuf,
    cert: std::path::PathBuf,
    profile: std::path::PathBuf,
    key_alias: String,
    store_pass: String,
    key_pass: String,
}

/// Resolve the release material; any unresolved secret degrades the whole section (None).
/// A RESOLVED path that doesn't exist is a real misconfiguration and errors.
fn resolve_material(
    project: &Project,
    cfg: &crate::meta::OhosSigning,
) -> Result<Option<OhosMaterial>, PackError> {
    let fields = (
        resolve_degradable(&cfg.keystore, "signing.ohos.keystore").map_err(PackError::Sign)?,
        resolve_degradable(&cfg.cert, "signing.ohos.cert").map_err(PackError::Sign)?,
        resolve_degradable(&cfg.profile, "signing.ohos.profile").map_err(PackError::Sign)?,
        resolve_degradable(&cfg.key_alias, "signing.ohos.key-alias").map_err(PackError::Sign)?,
        resolve_degradable(&cfg.store_pass, "signing.ohos.store-pass").map_err(PackError::Sign)?,
        resolve_degradable(&cfg.key_pass, "signing.ohos.key-pass").map_err(PackError::Sign)?,
    );
    let (
        Some(keystore),
        Some(cert),
        Some(profile),
        Some(key_alias),
        Some(store_pass),
        Some(key_pass),
    ) = fields
    else {
        return Ok(None);
    };
    let m = OhosMaterial {
        keystore: project.root.join(keystore),
        cert: project.root.join(cert),
        profile: project.root.join(profile),
        key_alias,
        store_pass,
        key_pass,
    };
    for (label, p) in [
        ("keystore", &m.keystore),
        ("cert", &m.cert),
        ("profile", &m.profile),
    ] {
        if !p.exists() {
            return Err(PackError::Sign(format!(
                "signing.ohos.{label} not found: {}",
                p.display()
            )));
        }
    }
    Ok(Some(m))
}

fn release_sign(m: &OhosMaterial, unsigned: &Path, signed: &Path) -> Result<(), String> {
    let jar = find_hap_sign_tool().ok_or(
        "hap-sign-tool.jar not found — set OHOS_SDK_HOME/OHOS_NDK_HOME to a full OpenHarmony SDK",
    )?;

    status("Signing", "hap-sign-tool (release)");
    run_tool(
        Command::new("java")
            .arg("-jar")
            .arg(&jar)
            .args([
                "sign-app",
                "-mode",
                "localSign",
                "-signAlg",
                "SHA256withECDSA",
            ])
            .args(["-keyAlias", &m.key_alias])
            .arg("-appCertFile")
            .arg(&m.cert)
            .arg("-profileFile")
            .arg(&m.profile)
            .arg("-keystoreFile")
            .arg(&m.keystore)
            .args(["-keystorePwd", &m.store_pass, "-keyPwd", &m.key_pass])
            .arg("-inFile")
            .arg(unsigned)
            .arg("-outFile")
            .arg(signed)
            .args(["-signCode", "1"]),
        "hap-sign-tool",
    )
}

/// hap-sign-tool.jar lives in the SDK's toolchains/lib — probe from the NDK location upward
/// (OHOS_NDK_HOME points at `<sdk>/native`; toolchains is its sibling).
fn find_hap_sign_tool() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for var in ["OHOS_SDK_HOME", "OHOS_BASE_SDK_HOME", "OHOS_NDK_HOME"] {
        if let Ok(v) = std::env::var(var) {
            let p = PathBuf::from(v);
            roots.push(p.clone());
            if let Some(parent) = p.parent() {
                roots.push(parent.to_path_buf());
            }
        }
    }
    for root in roots {
        for candidate in [
            root.join("toolchains/lib/hap-sign-tool.jar"),
            root.join("lib/hap-sign-tool.jar"),
        ] {
            if candidate.exists() {
                return Some(candidate);
            }
        }
        // Versioned SDK layouts: <root>/<api>/toolchains/lib/…
        if let Ok(entries) = std::fs::read_dir(&root) {
            for e in entries.flatten() {
                let candidate = e.path().join("toolchains/lib/hap-sign-tool.jar");
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}
