//! The payload tree the Linux packages share.
//!
//! `.flatpak` and `.appimage` differ in how they are sealed and where they get their toolkit, but
//! the tree inside is the same FHS-shaped prefix — the binary under `bin/`, the app's resources
//! under `share/<name>/`, and the app-id-named desktop exports under `share/`. Staging it once
//! here is what keeps a bundle and an AppImage of the same build carrying the same files, which is
//! also what lets `day rebuild` compare either against one recorded payload digest set (§20.3).
//!
//! The only thing that varies is the install prefix: flatpak mounts at `/app`, an AppImage at a
//! `$APPDIR` chosen per run. So the launcher script is generated from a prefix EXPRESSION rather
//! than a path, and each packer supplies its own.

use std::path::Path;

use crate::meta::Project;
use crate::ops::status;
use crate::targets::Target;

/// What staging produced, for the launcher the caller then writes.
pub(crate) struct Staged {
    /// The app name — `bin/<name>-bin`, `share/<name>/…`.
    pub name: String,
    /// The compiled resource blob, when the toolkit's compiler produced one (§18.3): the
    /// environment variable that points at it, and its file name under `share/<name>/`.
    pub resource_blob: Option<(&'static str, String)>,
}

/// Copy the built binary and the app's resources into `prefix`.
///
/// The binary lands at `bin/<name>-bin` rather than `bin/<name>`: the launcher takes the plain
/// name, and it has to export the `DAY_*` roots that a desktop launch would otherwise inherit
/// from `day launch` (ops.rs).
pub(crate) fn stage_tree(
    project: &Project,
    target: &Target,
    binary: &Path,
    prefix: &Path,
) -> Result<Staged, String> {
    let name = project.manifest.app.name.clone();
    let bin_dir = prefix.join("bin");
    let share_app = prefix.join("share").join(&name);
    std::fs::create_dir_all(&bin_dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&share_app).map_err(|e| e.to_string())?;

    // SBOM into share/<name>/sbom so the packaged app can read it (§20.4).
    if project.manifest.sbom.mode == crate::meta::SbomMode::Embed {
        crate::provenance::embed_into(&project.root.join("build/day/sbom"), &share_app)?;
    }
    let staged_bin = bin_dir.join(format!("{name}-bin"));
    std::fs::copy(binary, &staged_bin).map_err(|e| e.to_string())?;
    set_executable(&staged_bin)?;

    for dir in ["assets", "images", "fonts"] {
        let from = project.root.join("resource").join(dir);
        if from.is_dir() {
            super::copy_tree(&from, &share_app.join(dir))?;
        }
    }

    // Vector glyphs (docs/vectors.md): the raster cache plus the staged SVGs, under
    // `share/<name>/vectors/` — the launcher exports the same `DAY_VECTOR_*_ROOT` roots a
    // dev `day launch` would, so packed resolution matches dev exactly.
    for (from, to) in [
        (
            crate::resources::vector_fallback_dir(project, target.toolkit),
            "vectors/raster",
        ),
        (crate::resources::vector_svg_dir(project), "vectors/svg"),
    ] {
        if from.is_dir() {
            super::copy_tree(&from, &share_app.join(to))?;
        }
    }

    // Compiled resource blobs, when the toolkit's resource compiler produced them (§18.3).
    let resource_blob = match target.toolkit {
        "gtk" => Some(("DAY_GRESOURCE", "gtk", format!("{name}.gresource"))),
        "qt" => Some(("DAY_QRESOURCE", "qt", format!("{name}.rcc"))),
        _ => None,
    }
    .and_then(|(var, dir, file)| {
        let from = project.root.join("build/day").join(dir).join(&file);
        if !from.exists() {
            return None;
        }
        std::fs::copy(&from, share_app.join(&file)).ok()?;
        Some((var, file))
    });

    Ok(Staged {
        name,
        resource_blob,
    })
}

/// The launcher script: exports the `DAY_*` roots, then execs the real binary.
///
/// `prefix` is a SHELL EXPRESSION, not a path — `/app` for a flatpak, `"$HERE/usr"` for an
/// AppImage whose mount point is only known at run time — and `preamble` is whatever has to run
/// before it resolves.
pub(crate) fn launcher(prefix: &str, preamble: &str, staged: &Staged) -> String {
    let name = &staged.name;
    let mut lines = vec![
        format!(r#"export DAY_ASSET_ROOT="{prefix}/share/{name}/assets""#),
        format!(r#"export DAY_IMAGE_ROOT="{prefix}/share/{name}/images""#),
        format!(r#"export DAY_FONT_ROOT="{prefix}/share/{name}/fonts""#),
        format!(r#"export DAY_VECTOR_RASTER_ROOT="{prefix}/share/{name}/vectors/raster""#),
        format!(r#"export DAY_VECTOR_SVG_ROOT="{prefix}/share/{name}/vectors/svg""#),
    ];
    if let Some((var, file)) = &staged.resource_blob {
        lines.push(format!(r#"export {var}="{prefix}/share/{name}/{file}""#));
    }
    format!(
        "#!/bin/sh\n{preamble}{}\nexec \"{prefix}/bin/{name}-bin\" \"$@\"\n",
        lines.join("\n")
    )
}

/// Write the app-id-named desktop exports: hicolor icons, `.desktop`, and an AppStream metainfo.
pub(crate) fn stage_exports(
    project: &Project,
    prefix: &Path,
    id: &str,
    title: &str,
    exec: &str,
) -> Result<(), String> {
    stage_icons(project, prefix, id);
    let version = &project.manifest.app.version;
    let desktop_dir = prefix.join("share/applications");
    std::fs::create_dir_all(&desktop_dir).map_err(|e| e.to_string())?;
    std::fs::write(
        desktop_dir.join(format!("{id}.desktop")),
        desktop_entry(title, exec, id),
    )
    .map_err(|e| e.to_string())?;

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
    let metainfo_dir = prefix.join("share/metainfo");
    std::fs::create_dir_all(&metainfo_dir).map_err(|e| e.to_string())?;
    std::fs::write(metainfo_dir.join(format!("{id}.metainfo.xml")), metainfo)
        .map_err(|e| e.to_string())
}

/// The `.desktop` entry. `exec` differs per format — a flatpak exports the app-id command, an
/// AppImage runs its own `AppRun` — so the caller names it.
pub(crate) fn desktop_entry(title: &str, exec: &str, icon: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={title}\nExec={exec}\nIcon={icon}\nTerminal=false\nCategories=Utility;\n"
    )
}

pub(crate) fn set_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod +x {}: {e}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn stage_icons(project: &Project, prefix: &Path, id: &str) {
    if stage_project_icons(project, prefix, id) == 0 {
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
            let dest_dir = hicolor_dir(prefix, size);
            if std::fs::create_dir_all(&dest_dir).is_ok() {
                let _ = std::fs::write(dest_dir.join(format!("{id}.png")), bytes);
            }
        }
    }
}

/// Stage the project's own hicolor icons (app-id-named, from icons/linux/*-<N>.png, falling back
/// to any png). Returns how many were staged.
fn stage_project_icons(project: &Project, prefix: &Path, id: &str) -> usize {
    let entries = std::fs::read_dir(project.root.join("resource/icons/linux"))
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
        let dest_dir = hicolor_dir(prefix, size);
        if std::fs::create_dir_all(&dest_dir).is_ok()
            && std::fs::copy(&p, dest_dir.join(format!("{id}.png"))).is_ok()
        {
            staged += 1;
        }
    }
    staged
}

/// The largest staged icon, which an AppImage also needs at its ROOT (the format looks for
/// `<icon>.png` beside `AppRun`, not only in the hicolor tree).
pub(crate) fn largest_icon(prefix: &Path, id: &str) -> Option<std::path::PathBuf> {
    let mut best: Option<(u32, std::path::PathBuf)> = None;
    let hicolor = prefix.join("share/icons/hicolor");
    for e in std::fs::read_dir(&hicolor).ok()?.flatten() {
        let Some(size) = e
            .file_name()
            .to_str()
            .and_then(|n| n.split('x').next())
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        let png = e.path().join("apps").join(format!("{id}.png"));
        if png.is_file() && best.as_ref().is_none_or(|(b, _)| size > *b) {
            best = Some((size, png));
        }
    }
    best.map(|(_, p)| p)
}

fn hicolor_dir(prefix: &Path, size: u32) -> std::path::PathBuf {
    prefix
        .join("share/icons/hicolor")
        .join(format!("{size}x{size}"))
        .join("apps")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn staged(blob: Option<(&'static str, &str)>) -> Staged {
        Staged {
            name: "demo".into(),
            resource_blob: blob.map(|(v, f)| (v, f.to_string())),
        }
    }

    /// A flatpak mounts at a fixed prefix, so the launcher can name it outright.
    #[test]
    fn the_flatpak_launcher_points_at_the_fixed_prefix() {
        let sh = launcher(
            "/app",
            "",
            &staged(Some(("DAY_GRESOURCE", "demo.gresource"))),
        );
        assert!(sh.starts_with("#!/bin/sh\n"), "{sh}");
        assert!(
            sh.contains(r#"export DAY_ASSET_ROOT="/app/share/demo/assets""#),
            "{sh}"
        );
        assert!(
            sh.contains(r#"export DAY_GRESOURCE="/app/share/demo/demo.gresource""#),
            "{sh}"
        );
        assert!(
            sh.contains(r#"export DAY_VECTOR_RASTER_ROOT="/app/share/demo/vectors/raster""#),
            "{sh}"
        );
        assert!(
            sh.trim_end().ends_with(r#"exec "/app/bin/demo-bin" "$@""#),
            "{sh}"
        );
    }

    /// An AppImage's mount point is picked per run, so every path has to resolve through the
    /// preamble's variable rather than being baked in.
    #[test]
    fn the_appimage_launcher_resolves_every_path_at_run_time() {
        let preamble = "HERE=\"$(dirname \"$(readlink -f \"$0\")\")\"\n";
        let sh = launcher("$HERE/usr", preamble, &staged(None));
        assert!(sh.contains("readlink -f"), "{sh}");
        assert!(
            sh.contains(r#"export DAY_ASSET_ROOT="$HERE/usr/share/demo/assets""#),
            "{sh}"
        );
        assert!(
            sh.trim_end()
                .ends_with(r#"exec "$HERE/usr/bin/demo-bin" "$@""#),
            "{sh}"
        );
        // No blob staged: no variable pointing at one that is not there.
        assert!(!sh.contains("DAY_GRESOURCE"), "{sh}");
        assert!(!sh.contains("DAY_QRESOURCE"), "{sh}");
        // Nothing may reference an absolute build path.
        assert!(!sh.contains("/app/"), "{sh}");
    }

    /// The tree itself, staged from a fixture project. Both Linux formats read this layout, so a
    /// path that moves here silently breaks whichever packer was not being tested at the time.
    #[test]
    fn staging_lays_out_the_prefix_both_formats_expect() {
        let tmp = std::env::temp_dir().join(format!("day-linux-stage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("proj");
        std::fs::create_dir_all(root.join("resource/assets")).expect("assets");
        std::fs::create_dir_all(root.join("resource/images")).expect("images");
        std::fs::create_dir_all(root.join("build/day/gtk")).expect("blob dir");
        // The rasters a target ships come from its own fallback tree, not the shared cache
        // (docs/vectors.md) — gtk has no vector arm, so `write_vector_fallbacks` fills it with
        // every glyph. Staged directly here: this test drives the packer, not the whole build.
        std::fs::create_dir_all(root.join("build/day/vectors/fallback/gtk")).expect("raster dir");
        std::fs::create_dir_all(root.join("build/day/vectors/svg")).expect("svg dir");
        std::fs::write(root.join("resource/assets/data.txt"), "x").expect("asset");
        std::fs::write(root.join("resource/images/logo.png"), "x").expect("image");
        std::fs::write(root.join("build/day/gtk/demo.gresource"), "x").expect("blob");
        std::fs::write(root.join("build/day/vectors/fallback/gtk/home.png"), "x").expect("raster");
        std::fs::write(root.join("build/day/vectors/svg/home.svg"), "x").expect("svg");
        let binary = tmp.join("demo");
        std::fs::write(&binary, "#!/bin/sh\n").expect("binary");

        let manifest = crate::meta::parse_manifest(
            "schema = 1\n[app]\nid = \"dev.example.demo\"\ntitle = \"Demo App\"\n",
            "[package]\nname = \"demo\"\nversion = \"1.0.0\"\n",
            None,
        )
        .expect("manifest");
        let project = Project {
            root: root.clone(),
            manifest,
        };
        let target = crate::targets::find("linux-gtk").expect("target");

        // An AppImage's prefix is the AppDir's usr/; a flatpak's is the bundle root.
        let prefix = tmp.join("AppDir/usr");
        let staged = stage_tree(&project, target, &binary, &prefix).expect("stage");
        stage_exports(&project, &prefix, "dev.example.demo", "Demo App", "AppRun")
            .expect("exports");

        assert!(prefix.join("bin/demo-bin").is_file(), "the real binary");
        assert!(prefix.join("share/demo/assets/data.txt").is_file());
        assert!(prefix.join("share/demo/images/logo.png").is_file());
        assert!(prefix.join("share/demo/vectors/raster/home.png").is_file());
        assert!(prefix.join("share/demo/vectors/svg/home.svg").is_file());
        assert!(prefix.join("share/demo/demo.gresource").is_file());
        assert_eq!(
            staged.resource_blob,
            Some(("DAY_GRESOURCE", "demo.gresource".to_string())),
            "the gtk blob is found and reported so the launcher can point at it"
        );
        assert!(
            prefix
                .join("share/applications/dev.example.demo.desktop")
                .is_file()
        );
        assert!(
            prefix
                .join("share/metainfo/dev.example.demo.metainfo.xml")
                .is_file()
        );
        // No project icons, so the built-in defaults stand in — the format requires one.
        let icon = largest_icon(&prefix, "dev.example.demo").expect("an icon at some size");
        assert!(icon.ends_with("dev.example.demo.png"), "{}", icon.display());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(prefix.join("bin/demo-bin"))
                .expect("stat")
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o111,
                0o111,
                "the staged binary has to be executable"
            );
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The AppImage root icon is the biggest staged size, not whichever the directory walk hit
    /// first — a 48px icon at the root is what makes a launcher render a blurry entry.
    #[test]
    fn the_root_icon_is_the_largest_staged_size() {
        let tmp = std::env::temp_dir().join(format!("day-linux-icon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        for size in [48u32, 256, 128] {
            let dir = hicolor_dir(&tmp, size);
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(dir.join("dev.example.demo.png"), "x").expect("icon");
        }
        let icon = largest_icon(&tmp, "dev.example.demo").expect("icon");
        assert!(
            icon.to_string_lossy().contains("256x256"),
            "{}",
            icon.display()
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn the_desktop_entry_names_the_exec_the_caller_chose() {
        let d = desktop_entry("Demo App", "AppRun", "dev.example.demo");
        assert!(d.contains("Name=Demo App"), "{d}");
        assert!(d.contains("Exec=AppRun"), "{d}");
        assert!(d.contains("Icon=dev.example.demo"), "{d}");
    }
}
