//! linux-gtk / linux-qt → single-file .flatpak bundle. The runtime supplies the toolkit
//! (GTK4 ⇒ org.gnome.Platform, Qt6 ⇒ org.kde.Platform — no toolkit bundling, which also keeps
//! Qt-LGPL obligations satisfied by the runtime's relinkable shared libs). Day stages the prebuilt
//! release binary + resources into /app (the Tauri/Spotube repack pattern — no build-from-source),
//! generates the app-id-named exports (.desktop, metainfo.xml, hicolor icons), then
//! flatpak-builder → repo → `flatpak build-bundle` with --runtime-repo so the runtime resolves
//! from Flathub at install time. Flathub-ready offline manifests are a later mode.

use std::path::Path;
use std::process::Command;

use super::settings::PackOptions;
use super::{Artifact, PackError, SignTier, run_tool};
use crate::meta::Project;
use crate::ops::{self, status};
use crate::targets::Target;

// Overridable runtime pins (DAY_GNOME_RUNTIME / DAY_KDE_RUNTIME) so CI can bump without a release.
const GNOME_RUNTIME_VERSION: &str = "48";
const KDE_RUNTIME_VERSION: &str = "6.9";
/// Qt WebEngine is NOT part of org.kde.Platform — apps that link it need the Qt BaseApp.
const QT_WEBENGINE_BASEAPP: &str = "io.qt.qtwebengine.BaseApp";

pub fn pack(
    project: &Project,
    target: &'static Target,
    opts: &PackOptions,
    dist: &Path,
) -> Result<Artifact, PackError> {
    for tool in ["flatpak", "flatpak-builder"] {
        if !on_path(tool) {
            return Err(PackError::Other(format!(
                "{tool} not found — install flatpak + flatpak-builder and add the flathub remote:\n  \
                 flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo"
            )));
        }
    }

    let outcome = ops::build(project, target, &opts.profile).map_err(PackError::Other)?;
    let name = project.manifest.app.name.clone();
    let id = project.manifest.app.id.clone();
    let title = project
        .manifest
        .app
        .title
        .clone()
        .unwrap_or_else(|| name.clone());
    let version = &project.manifest.app.version;

    let work = project.root.join("build/day/flatpak").join(target.name);
    let stage = work.join("stage");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&stage).map_err(|e| PackError::Other(e.to_string()))?;

    // --- stage the /app payload --------------------------------------------
    // Real binary at bin/<name>-bin; the exported command is a wrapper exporting the DAY_* env the
    // desktop launch path would otherwise pass (ops.rs): resource blobs, asset root, icon name.
    let bin_dir = stage.join("bin");
    let share_app = stage.join("share").join(&name);
    std::fs::create_dir_all(&bin_dir).map_err(|e| PackError::Other(e.to_string()))?;
    std::fs::create_dir_all(&share_app).map_err(|e| PackError::Other(e.to_string()))?;
    std::fs::copy(&outcome.artifact, bin_dir.join(format!("{name}-bin")))
        .map_err(|e| PackError::Other(e.to_string()))?;
    let assets = project.root.join("resource/assets");
    if assets.is_dir() {
        super::copy_tree(&assets, &share_app.join("assets")).map_err(PackError::Other)?;
    }
    let images = project.root.join("resource/images");
    if images.is_dir() {
        super::copy_tree(&images, &share_app.join("images")).map_err(PackError::Other)?;
    }
    // Bundled fonts (§18.4): the backend registers every file under DAY_FONT_ROOT at startup.
    let fonts = project.root.join("resource/fonts");
    if fonts.is_dir() {
        super::copy_tree(&fonts, &share_app.join("fonts")).map_err(PackError::Other)?;
    }
    // Compiled resource blobs, when the toolkit's resource compiler produced them (§18.3).
    let mut wrapper_env = vec![
        format!("export DAY_ASSET_ROOT=/app/share/{name}/assets"),
        format!("export DAY_IMAGE_ROOT=/app/share/{name}/images"),
        format!("export DAY_FONT_ROOT=/app/share/{name}/fonts"),
        format!("export DAY_ICON_NAME={id}"),
    ];
    let gresource = project
        .root
        .join("build/day/gtk")
        .join(format!("{name}.gresource"));
    if target.toolkit == "gtk" && gresource.exists() {
        std::fs::copy(&gresource, share_app.join(format!("{name}.gresource")))
            .map_err(|e| PackError::Other(e.to_string()))?;
        wrapper_env.push(format!(
            "export DAY_GRESOURCE=/app/share/{name}/{name}.gresource"
        ));
    }
    let qresource = project
        .root
        .join("build/day/qt")
        .join(format!("{name}.rcc"));
    if target.toolkit == "qt" && qresource.exists() {
        std::fs::copy(&qresource, share_app.join(format!("{name}.rcc")))
            .map_err(|e| PackError::Other(e.to_string()))?;
        wrapper_env.push(format!("export DAY_QRESOURCE=/app/share/{name}/{name}.rcc"));
    }
    let wrapper = format!(
        "#!/bin/sh\n{}\nexec /app/bin/{name}-bin \"$@\"\n",
        wrapper_env.join("\n")
    );
    let wrapper_path = bin_dir.join(&id);
    std::fs::write(&wrapper_path, wrapper).map_err(|e| PackError::Other(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| PackError::Other(e.to_string()))?;
        std::fs::set_permissions(
            bin_dir.join(format!("{name}-bin")),
            std::fs::Permissions::from_mode(0o755),
        )
        .map_err(|e| PackError::Other(e.to_string()))?;
    }

    // --- exports: icons, .desktop, metainfo (all app-id-named) ---------------
    stage_icons(project, &stage, &id);
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName={title}\nExec={id}\nIcon={id}\nTerminal=false\nCategories=Utility;\n"
    );
    let desktop_dir = stage.join("share/applications");
    std::fs::create_dir_all(&desktop_dir).map_err(|e| PackError::Other(e.to_string()))?;
    std::fs::write(desktop_dir.join(format!("{id}.desktop")), desktop)
        .map_err(|e| PackError::Other(e.to_string()))?;
    let metainfo = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>{id}</id>
  <name>{title}</name>
  <summary>{title}</summary>
  <metadata_license>CC0-1.0</metadata_license>
  <description><p>{title}, built with Day.</p></description>
  <launchable type="desktop-id">{id}.desktop</launchable>
  <releases><release version="{version}"/></releases>
</component>
"#
    );
    let metainfo_dir = stage.join("share/metainfo");
    std::fs::create_dir_all(&metainfo_dir).map_err(|e| PackError::Other(e.to_string()))?;
    std::fs::write(metainfo_dir.join(format!("{id}.metainfo.xml")), metainfo)
        .map_err(|e| PackError::Other(e.to_string()))?;

    // --- manifest -------------------------------------------------------------
    let manifest_path = work.join(format!("{id}.yml"));
    std::fs::write(&manifest_path, manifest_yaml(target, &id, &name))
        .map_err(|e| PackError::Other(e.to_string()))?;

    // --- flatpak-builder → repo → bundle ---------------------------------------
    status("Packing", "flatpak-builder");
    run_tool(
        Command::new("flatpak-builder")
            .current_dir(&work)
            .args(["--force-clean", "--user", "--install-deps-from=flathub"])
            .arg("--repo=repo")
            .arg("builddir")
            .arg(&manifest_path),
        "flatpak-builder",
    )
    .map_err(PackError::Other)?;

    let arch = flatpak_arch();
    // The toolkit is part of the name: linux-gtk and linux-qt both pack this format, and
    // release CI merges every target's dist/ into one directory — identical names collide.
    let toolkit = target.toolkit;
    let bundle = dist.join(format!("{name}-{version}-{toolkit}-{arch}.flatpak"));
    let _ = std::fs::remove_file(&bundle);
    status("Packing", "flatpak build-bundle");
    run_tool(
        Command::new("flatpak")
            .current_dir(&work)
            .arg("build-bundle")
            .arg("repo")
            .arg(&bundle)
            .arg(&id)
            .arg("--runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo"),
        "flatpak build-bundle",
    )
    .map_err(PackError::Other)?;

    // Bundle signing is repo/commit-level GPG (deferred); the bundle itself carries no signature.
    Ok(Artifact {
        path: bundle,
        kind: "flatpak",
        sha256: String::new(),
        tier: SignTier::Unsigned,
    })
}

/// The generated flatpak-builder manifest: runtime per toolkit, module = dump the staged tree.
pub(crate) fn manifest_yaml(target: &Target, id: &str, name: &str) -> String {
    let (runtime, runtime_version) = match target.toolkit {
        "qt" => (
            "org.kde.Platform",
            std::env::var("DAY_KDE_RUNTIME").unwrap_or_else(|_| KDE_RUNTIME_VERSION.into()),
        ),
        _ => (
            "org.gnome.Platform",
            std::env::var("DAY_GNOME_RUNTIME").unwrap_or_else(|_| GNOME_RUNTIME_VERSION.into()),
        ),
    };
    let sdk = runtime.replace(".Platform", ".Sdk");
    // Qt apps that link WebEngine need the BaseApp (QtWebEngine is not in org.kde.Platform).
    let base = if target.toolkit == "qt" {
        format!("base: {QT_WEBENGINE_BASEAPP}\nbase-version: '{runtime_version}'\n")
    } else {
        String::new()
    };
    format!(
        r#"id: {id}
runtime: {runtime}
runtime-version: '{runtime_version}'
sdk: {sdk}
{base}command: {id}
# The payload is a prebuilt release binary with no debug info — skip flatpak-builder's
# debuginfo split (it shells out to elfutils' eu-strip, which isn't installed everywhere,
# e.g. ubuntu-24.04 CI runners) and its strip pass.
build-options:
  no-debuginfo: true
  strip: false
finish-args:
  - --share=ipc
  - --socket=fallback-x11
  - --socket=wayland
  - --device=dri
  - --share=network
modules:
  - name: {name}
    buildsystem: simple
    build-commands:
      - cp -a . /app
    sources:
      - type: dir
        path: stage
"#
    )
}

fn stage_icons(project: &Project, stage: &Path, id: &str) {
    if stage_project_icons(project, stage, id) == 0 {
        // No project icons: stage the built-in defaults. The .desktop says `Icon={id}` and the
        // appstream catalog REQUIRES a resolvable icon for a desktop-application component —
        // flatpak-builder's `appstreamcli compose` fails the whole bundle with `icon-not-found`
        // otherwise, so an icon-less project must still export one. All the policy sizes are
        // staged (48/64/128): compose only probes those, so a single off-policy size stays
        // invisible to it (see resources::DEFAULT_ICONS).
        status(
            "Packing",
            "no resource/icons/*.png — using the default Day icon (add resource/icons/linux/<name>-<size>.png to brand the app)",
        );
        for (size, bytes) in crate::resources::DEFAULT_ICONS {
            let dest_dir = stage
                .join("share/icons/hicolor")
                .join(format!("{size}x{size}"))
                .join("apps");
            if std::fs::create_dir_all(&dest_dir).is_ok() {
                let _ = std::fs::write(dest_dir.join(format!("{id}.png")), bytes);
            }
        }
    }
}

/// Stage the project's own hicolor icons (app-id-named, from icons/linux/*-<N>.png, falling back
/// to any png). Returns how many were staged.
fn stage_project_icons(project: &Project, stage: &Path, id: &str) -> usize {
    let icons_dir = project.root.join("resource/icons/linux");
    let entries = std::fs::read_dir(&icons_dir)
        .or_else(|_| std::fs::read_dir(project.root.join("resource/icons/png")))
        .or_else(|_| std::fs::read_dir(project.root.join("resource/icons")));
    let Ok(entries) = entries else { return 0 };
    let mut staged = 0;
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("png") {
            continue;
        }
        // Size from a trailing -<N> in the stem (day-icon-128.png → 128); skip unsized files.
        let Some(size) = p
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.rsplit('-').next())
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        let dest_dir = stage
            .join("share/icons/hicolor")
            .join(format!("{size}x{size}"))
            .join("apps");
        if std::fs::create_dir_all(&dest_dir).is_ok()
            && std::fs::copy(&p, dest_dir.join(format!("{id}.png"))).is_ok()
        {
            staged += 1;
        }
    }
    staged
}

fn on_path(tool: &str) -> bool {
    std::env::var("PATH").is_ok_and(|p| std::env::split_paths(&p).any(|d| d.join(tool).is_file()))
}

fn flatpak_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::targets;

    #[test]
    fn manifest_runtime_per_toolkit() {
        let gtk = manifest_yaml(targets::find("linux-gtk").unwrap(), "dev.x.app", "app");
        assert!(gtk.contains("runtime: org.gnome.Platform"));
        assert!(gtk.contains("sdk: org.gnome.Sdk"));
        assert!(!gtk.contains("base:"));
        let qt = manifest_yaml(targets::find("linux-qt").unwrap(), "dev.x.app", "app");
        assert!(qt.contains("runtime: org.kde.Platform"));
        assert!(qt.contains("base: io.qt.qtwebengine.BaseApp"));
        assert!(qt.contains("command: dev.x.app"));
        // Both manifests must be valid YAML and skip the debuginfo split (its eu-strip
        // dependency isn't installed on CI runners).
        for manifest in [&gtk, &qt] {
            let parsed: serde_json::Value = serde_norway::from_str(manifest).unwrap();
            assert_eq!(parsed["build-options"]["no-debuginfo"], true);
            assert_eq!(parsed["build-options"]["strip"], false);
        }
    }
}
