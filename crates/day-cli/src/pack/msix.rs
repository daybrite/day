// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! windows-xaml → .msix (makeappx + signtool). The xaml backend hosts system XAML Islands
//! (Windows.UI.Xaml ships with the OS — no WinAppSDK runtime dependency to declare or bootstrap).
//! Signing providers (Day.toml signing.windows.provider): self-signed-dev (default; generated
//! per-publisher cert in CurrentUser\My — installable locally after trusting it, dev tier) |
//! signtool-cert-store (thumbprint) | azure-artifact-signing (signtool /dlib; 72 h certs make the
//! /tr timestamp mandatory). The MSIX Identity Publisher must byte-match the cert subject.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::settings::{PackOptions, resolve_degradable};
use super::{Artifact, PackError, SignTier, run_tool};
use crate::meta::Project;
use crate::ops::{self, status};
use crate::targets::Target;

const DEV_PUBLISHER: &str = "CN=Day Development";

/// Build the exe and stage the shared installer payload (exe + assets + images + icon) once,
/// for both msix and nsis to package.
pub fn stage_payload(
    project: &Project,
    target: &'static Target,
    opts: &PackOptions,
) -> Result<PathBuf, PackError> {
    let outcome = ops::build(project, target, &opts.profile).map_err(PackError::Other)?;
    let stage = project.root.join("build/day/pack/windows-payload");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| PackError::Other(e.to_string()))?;
    let name = &project.manifest.app.name;
    // SBOM into the shared payload, which feeds both the .msix and the NSIS installer (§20.4).
    if project.manifest.sbom.mode == crate::meta::SbomMode::Embed {
        crate::provenance::embed_into(&project.root.join("build/day/sbom"), &stage)
            .map_err(PackError::Other)?;
    }
    std::fs::copy(&outcome.artifact, stage.join(format!("{name}.exe")))
        .map_err(|e| PackError::Other(e.to_string()))?;
    // The xaml runtime resolves assets/images/fonts relative to the exe when DAY_* env is
    // absent (resources/xaml.rs is a launch-env no-op — pack ships the trees beside the binary).
    for dir in ["assets", "images", "fonts"] {
        let src = project.root.join("resource").join(dir);
        if src.is_dir() {
            super::copy_tree(&src, &stage.join(dir)).map_err(PackError::Other)?;
        }
    }
    // Vector glyphs (docs/vectors.md): the raster FALLBACKS merge into `images/` — one
    // exe-relative probe then serves both the `vector(…)` piece and the nav rows'
    // `ms-appx:///images/<file>` loads (the name namespace is shared, so a stem can't collide
    // with a real image). Only glyphs XAML cannot draw as geometry are here; a convertible glyph
    // ships once, as geometry, and a failure to draw it shows as missing art rather than as a
    // silently substituted PNG.
    let rasters = crate::resources::vector_fallback_dir(project, target.toolkit);
    if rasters.is_dir() {
        let images = stage.join("images");
        std::fs::create_dir_all(&images).map_err(|e| PackError::Other(e.to_string()))?;
        for entry in std::fs::read_dir(&rasters)
            .map_err(|e| PackError::Other(e.to_string()))?
            .flatten()
        {
            let p = entry.path();
            if p.extension().and_then(|x| x.to_str()) == Some("png")
                && let Some(file) = p.file_name()
            {
                std::fs::copy(&p, images.join(file))
                    .map_err(|e| PackError::Other(e.to_string()))?;
            }
        }
    }
    // The XAML geometry (docs/vectors.md) rides as its own `vectors/xaml` tree, which is where
    // `resolve_vector_xaml`'s exe-relative probe looks. It cannot merge into `images/` the way
    // the raster fallbacks do — these are geometry specs, not loadable images — and without it a
    // packed app would have no vector form at all. The glyph SVGs deliberately do NOT ship
    // beside it: this backend renders the geometry, nothing on Windows reads an SVG, and a
    // second copy of every glyph is exactly the weight the raster cache used to carry.
    let geometry = crate::resources::vector_xaml_dir(project);
    if geometry.is_dir() {
        super::copy_tree(&geometry, &stage.join("vectors/xaml")).map_err(PackError::Other)?;
    }
    if let Some(ico) = crate::resources::app_icon(project, "xaml") {
        let _ = std::fs::copy(&ico, stage.join(format!("{name}.ico")));
    }
    // Both containers built from this tree — makeappx's .msix and makensis's -setup.exe — copy
    // each file's mtime into their archive, so the tree is stamped once here rather than in each
    // packer (§20.3).
    super::normalize_mtimes(&stage).map_err(PackError::Other)?;
    Ok(stage)
}

pub fn pack(
    project: &Project,
    target: &'static Target,
    opts: &PackOptions,
    payload: &Path,
    dist: &Path,
) -> Result<Artifact, PackError> {
    let name = &project.manifest.app.name;
    let title = project
        .manifest
        .app
        .title
        .clone()
        .unwrap_or_else(|| name.clone());
    let version = &project.manifest.app.version;

    let makeappx = windows_kit_tool("makeappx.exe").ok_or_else(|| {
        PackError::Other(
            "makeappx.exe not found — install the Windows 10/11 SDK (or set DAY_WINDOWS_KIT)"
                .into(),
        )
    })?;

    let signing = resolve_signing(project)?;
    let publisher = signing.publisher.clone();

    // --- stage msix-specific bits on top of a copy of the payload -------------
    let stage = project.root.join("build/day/pack/windows-msix");
    let _ = std::fs::remove_dir_all(&stage);
    super::copy_tree(payload, &stage).map_err(PackError::Other)?;
    let assets_dir = stage.join("Assets");
    std::fs::create_dir_all(&assets_dir).map_err(|e| PackError::Other(e.to_string()))?;
    // MSIX logo slots, from the largest available PNG (Store lints sizes; sideload does not).
    // The generated AppxManifest.xml references all three slots unconditionally, and makeappx
    // fails on a manifest entry with no file — so an icon-less project gets the built-in
    // default rather than no slots.
    let logo = project
        .root
        .join("resource/icons/windows/day-icon-256.png")
        .exists()
        .then(|| project.root.join("resource/icons/windows/day-icon-256.png"))
        .or_else(|| {
            crate::resources::app_icon(project, "gtk") // any png via the linux/png lookup
        });
    const LOGO_SLOTS: [&str; 3] = [
        "StoreLogo.png",
        "Square150x150Logo.png",
        "Square44x44Logo.png",
    ];
    match logo {
        Some(png) => {
            for slot in LOGO_SLOTS {
                std::fs::copy(&png, assets_dir.join(slot))
                    .map_err(|e| PackError::Other(format!("staging {slot}: {e}")))?;
            }
        }
        None => {
            status(
                "Packing",
                "no resource/icons/*.png — using the default Day icon for the MSIX logo slots (add resource/icons/windows/day-icon-256.png to brand the app)",
            );
            // The largest built-in default (sideload doesn't lint slot sizes; the Store does).
            let (_, bytes) =
                crate::resources::DEFAULT_ICONS[crate::resources::DEFAULT_ICONS.len() - 1];
            for slot in LOGO_SLOTS {
                std::fs::write(assets_dir.join(slot), bytes)
                    .map_err(|e| PackError::Other(format!("staging {slot}: {e}")))?;
            }
        }
    }
    // A normalized identity is not the app id any more, and the difference matters when the
    // package is registered with the Microsoft Store — so say it rather than let it be discovered.
    let identity = identity_name(&project.manifest.app.id);
    if identity != project.manifest.app.id {
        status(
            "Packing",
            &format!(
                "MSIX Identity Name {identity:?} (the manifest schema allows only [-.A-Za-z0-9], \
                 so the app id {:?} cannot be used verbatim)",
                project.manifest.app.id
            ),
        );
    }
    std::fs::write(
        stage.join("AppxManifest.xml"),
        appx_manifest(&project.manifest.app.id, &title, version, name, &publisher),
    )
    .map_err(|e| PackError::Other(e.to_string()))?;

    // --- makeappx pack ---------------------------------------------------------
    let msix = dist.join(super::naming::artifact_file(
        project,
        target,
        opts,
        &[],
        "msix",
    ));
    let _ = std::fs::remove_file(&msix);
    status("Packing", "makeappx pack");
    run_tool(
        Command::new(&makeappx)
            .args(["pack", "/o", "/d"])
            .arg(&stage)
            .arg("/p")
            .arg(&msix),
        "makeappx",
    )
    .map_err(PackError::Other)?;

    // --- sign --------------------------------------------------------------------
    let tier = if opts.no_sign {
        status("Signing", "skipped (--no-sign)");
        SignTier::Unsigned
    } else {
        sign_file(&signing, &msix).map_err(PackError::Sign)?
    };

    Ok(Artifact {
        path: msix,
        kind: "msix",
        sha256: String::new(),
        tier,
    })
}

/// The MSIX `Identity/Name`, which the manifest schema constrains to `[-.A-Za-z0-9]+`.
///
/// An app id is reverse-DNS, but the platforms disagree on what may separate words inside a
/// segment: an underscore is legal in a Java package name, so Android app ids carry them, and it
/// is rejected here — `makeappx` fails the whole pack with a schema violation rather than a
/// warning. Every disallowed character maps to `-`, which keeps the name readable and, being a
/// pure function of the id, stable across rebuilds (`day rebuild` compares the manifest).
pub(crate) fn identity_name(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// AppxManifest for a full-trust Win32 app. Identity Name is normalized by [`identity_name`];
/// MSIX versions are four-part — pad the semver.
pub(crate) fn appx_manifest(
    id: &str,
    title: &str,
    version: &str,
    exe: &str,
    publisher: &str,
) -> String {
    let id = identity_name(id);
    let four_part = {
        let mut parts: Vec<&str> = version.split(['.', '-', '+']).take(3).collect();
        while parts.len() < 3 {
            parts.push("0");
        }
        format!("{}.0", parts.join("."))
    };
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
         xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
         xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities">
  <Identity Name="{id}" Publisher="{publisher}" Version="{four_part}" ProcessorArchitecture="x64"/>
  <Properties>
    <DisplayName>{title}</DisplayName>
    <PublisherDisplayName>{publisher_display}</PublisherDisplayName>
    <Logo>Assets\StoreLogo.png</Logo>
  </Properties>
  <Dependencies>
    <!-- MinVersion 18362 (Windows 10 1903): the canvas radial gradient uses XAML's
         RadialGradientBrush, which does not exist on 1809 (long end-of-life). -->
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.18362.0" MaxVersionTested="10.0.22621.0"/>
  </Dependencies>
  <Resources><Resource Language="en-us"/></Resources>
  <Applications>
    <Application Id="App" Executable="{exe}.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="{title}" Description="{title}"
        BackgroundColor="transparent"
        Square150x150Logo="Assets\Square150x150Logo.png" Square44x44Logo="Assets\Square44x44Logo.png"/>
    </Application>
  </Applications>
  <Capabilities>
    <rescap:Capability Name="runFullTrust"/>
  </Capabilities>
</Package>
"#,
        publisher_display = publisher.trim_start_matches("CN=")
    )
}

pub(crate) struct WindowsSignSettings {
    pub provider: Provider,
    pub publisher: String,
    pub timestamp_url: String,
}

pub(crate) enum Provider {
    SelfSignedDev,
    CertStore {
        thumbprint: String,
    },
    AzureArtifactSigning {
        endpoint: String,
        account: String,
        profile: String,
        dlib: String,
    },
}

pub(crate) fn resolve_signing(project: &Project) -> Result<WindowsSignSettings, PackError> {
    let win = project
        .manifest
        .signing
        .as_ref()
        .and_then(|s| s.windows.as_ref());
    let Some(win) = win else {
        return Ok(WindowsSignSettings {
            provider: Provider::SelfSignedDev,
            publisher: DEV_PUBLISHER.into(),
            timestamp_url: String::new(),
        });
    };
    let publisher = resolve_field(win.publisher.as_ref(), "signing.windows.publisher")?
        .unwrap_or_else(|| DEV_PUBLISHER.into());
    let timestamp = resolve_field(win.timestamp_url.as_ref(), "signing.windows.timestamp-url")?;
    // A release provider whose secrets don't resolve degrades to the dev cert (§20) — the
    // requested provider stays configured, the tier just drops loudly on this run.
    let provider = match win.provider.as_str() {
        "self-signed-dev" => Provider::SelfSignedDev,
        "signtool-cert-store" => {
            match resolve_field(win.thumbprint.as_ref(), "signing.windows.thumbprint")? {
                Some(thumbprint) => Provider::CertStore { thumbprint },
                None => {
                    status(
                        "Warning",
                        "signtool-cert-store unresolved — self-signed dev cert",
                    );
                    Provider::SelfSignedDev
                }
            }
        }
        "azure-artifact-signing" => {
            let fields = (
                resolve_field(win.endpoint.as_ref(), "signing.windows.endpoint")?,
                resolve_field(win.account.as_ref(), "signing.windows.account")?,
                resolve_field(win.profile.as_ref(), "signing.windows.profile")?,
                resolve_field(win.dlib.as_ref(), "signing.windows.dlib")?,
            );
            match fields {
                (Some(endpoint), Some(account), Some(profile), Some(dlib)) => {
                    Provider::AzureArtifactSigning {
                        endpoint,
                        account,
                        profile,
                        dlib,
                    }
                }
                _ => {
                    status(
                        "Warning",
                        "azure-artifact-signing needs endpoint/account/profile/dlib — self-signed dev cert",
                    );
                    Provider::SelfSignedDev
                }
            }
        }
        other => {
            return Err(PackError::Sign(format!(
                "signing.windows.provider {other:?} — expected self-signed-dev | signtool-cert-store | azure-artifact-signing"
            )));
        }
    };
    let timestamp_url = timestamp.unwrap_or_else(|| match &provider {
        Provider::AzureArtifactSigning { .. } => "http://timestamp.acs.microsoft.com".into(),
        _ => "http://timestamp.digicert.com".into(),
    });
    Ok(WindowsSignSettings {
        provider,
        publisher,
        timestamp_url,
    })
}

/// A configured-but-optional signing field: absent stays None; an unset secret degrades (§20).
fn resolve_field(v: Option<&String>, what: &str) -> Result<Option<String>, PackError> {
    match v {
        None => Ok(None),
        Some(raw) => resolve_degradable(raw, what).map_err(PackError::Sign),
    }
}

/// Sign one file with the resolved provider; returns the achieved tier.
pub(crate) fn sign_file(settings: &WindowsSignSettings, file: &Path) -> Result<SignTier, String> {
    let signtool = windows_kit_tool("signtool.exe")
        .ok_or("signtool.exe not found — install the Windows SDK (or set DAY_WINDOWS_KIT)")?;
    match &settings.provider {
        Provider::SelfSignedDev => {
            let thumbprint = ensure_dev_cert(&settings.publisher)?;
            status("Signing", "signtool (self-signed dev cert)");
            run_tool(
                Command::new(&signtool)
                    .args(["sign", "/fd", "SHA256", "/sha1", &thumbprint])
                    .arg(file),
                "signtool (dev)",
            )?;
            Ok(SignTier::DevSigned)
        }
        Provider::CertStore { thumbprint } => {
            status("Signing", "signtool (certificate store)");
            run_tool(
                Command::new(&signtool)
                    .args(["sign", "/fd", "SHA256", "/sha1", thumbprint])
                    .args(["/tr", &settings.timestamp_url, "/td", "SHA256"])
                    .arg(file),
                "signtool",
            )?;
            Ok(SignTier::Release)
        }
        Provider::AzureArtifactSigning {
            endpoint,
            account,
            profile,
            dlib,
        } => {
            // Auth rides DefaultAzureCredential (azure/login OIDC in CI, az login locally).
            let metadata = file.with_extension("signing-metadata.json");
            std::fs::write(
                &metadata,
                serde_json::json!({
                    "Endpoint": endpoint,
                    "CodeSigningAccountName": account,
                    "CertificateProfileName": profile,
                })
                .to_string(),
            )
            .map_err(|e| e.to_string())?;
            status("Signing", "signtool (Azure Artifact Signing)");
            let result = run_tool(
                Command::new(&signtool)
                    .args(["sign", "/v", "/fd", "SHA256"])
                    .args(["/tr", &settings.timestamp_url, "/td", "SHA256"])
                    .arg("/dlib")
                    .arg(dlib)
                    .arg("/dmdf")
                    .arg(&metadata)
                    .arg(file),
                "signtool (azure)",
            );
            let _ = std::fs::remove_file(&metadata);
            result?;
            Ok(SignTier::Release)
        }
    }
}

/// Find-or-create the self-signed dev cert for `publisher` in CurrentUser\My; returns its thumbprint.
fn ensure_dev_cert(publisher: &str) -> Result<String, String> {
    let script = format!(
        "$c = Get-ChildItem Cert:\\CurrentUser\\My | Where-Object {{ $_.Subject -eq '{publisher}' }} | Select-Object -First 1; \
         if (-not $c) {{ $c = New-SelfSignedCertificate -Type Custom -Subject '{publisher}' -KeyUsage DigitalSignature \
         -FriendlyName 'Day dev signing' -CertStoreLocation Cert:\\CurrentUser\\My \
         -TextExtension @('2.5.29.37={{text}}1.3.6.1.5.5.7.3.3','2.5.29.19={{text}}') }}; \
         $c.Thumbprint"
    );
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| format!("powershell: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "dev cert creation failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let thumb = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if thumb.len() != 40 {
        return Err(format!("unexpected thumbprint output: {thumb:?}"));
    }
    Ok(thumb)
}

/// A Windows-Kits bin tool — delegated to the shared, env-overridable lookup
/// (`DAY_WINDOWS_KIT` bin-dir override, then PATH, then `DAY_WINDOWS_KITS_ROOT` /
/// `WindowsSdkDir` / `%ProgramFiles%`-derived roots — docs/environment.md).
pub(crate) fn windows_kit_tool(tool: &str) -> Option<PathBuf> {
    day_toolchain::windows_kit_tool(tool)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The manifest schema's `[-.A-Za-z0-9]+` is narrower than what an app id may contain, and
    /// `makeappx` fails the whole pack rather than warning — so the normalization is what keeps a
    /// legal Android id (underscores) packable on Windows. Regression: `dev.example.ci_sample`
    /// scaffolded by `day new` failed CI with "violates pattern constraint".
    #[test]
    fn identity_name_satisfies_the_manifest_schema() {
        assert_eq!(
            identity_name("dev.example.ci_sample"),
            "dev.example.ci-sample"
        );
        assert_eq!(
            identity_name("dev.daybrite.showcase"),
            "dev.daybrite.showcase"
        );
        assert_eq!(identity_name("dev.example.a b"), "dev.example.a-b");
        for id in [
            "dev.example.ci_sample",
            "dev.example.a b",
            "dev.example.wei\u{df}",
        ] {
            let name = identity_name(id);
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'),
                "{name:?} still violates the schema pattern"
            );
        }
        // The manifest carries the normalized name, not the raw id.
        let m = appx_manifest("dev.example.ci_sample", "T", "1.2.3", "t", "CN=x");
        assert!(m.contains(r#"Name="dev.example.ci-sample""#), "{m}");
        assert!(!m.contains("ci_sample"), "{m}");
    }

    #[test]
    fn appx_manifest_shape() {
        let m = appx_manifest(
            "dev.daybrite.showcase",
            "Day Showcase",
            "0.1.0",
            "showcase",
            "CN=Day Development",
        );
        assert!(m.contains(r#"Version="0.1.0.0""#));
        assert!(m.contains(r#"Publisher="CN=Day Development""#));
        assert!(m.contains(r#"Executable="showcase.exe""#));
        assert!(m.contains("runFullTrust"));
        assert!(m.contains("<PublisherDisplayName>Day Development</PublisherDisplayName>"));
    }
}
