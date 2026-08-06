//! linux-gtk / linux-qt → a single-file `.appimage`: one executable that runs on any glibc Linux
//! with no installer, no package manager, and no root.
//!
//! It is the sibling of the `.flatpak`, not a replacement, and the split is about where the
//! toolkit comes from. A flatpak gets GTK/Qt from a runtime the user's flatpak installation
//! resolves — correct, sandboxed, and several hundred megabytes on first install. An AppImage
//! carries what it needs itself, so `curl … && chmod +x && ./app` really is the whole procedure.
//! Releases ship both, and the one-line launcher (daybrite/actions) reaches for the AppImage.
//!
//! Day does not implement the bundling: `linuxdeploy` walks the binary's `DT_NEEDED` closure and
//! copies it in, its `gtk`/`qt` plugin adds the parts a naive `ldd` walk misses (GdkPixbuf
//! loaders, GIO modules, GSettings schemas; Qt's platform plugins), and `--output appimage` seals
//! the result. Reimplementing that here would be reimplementing it badly — the failure modes are
//! all in the parts an `ldd` closure does not see.
//!
//! Without the toolkit plugin the AppImage is still produced and still runs, on a machine that
//! already has that toolkit. That is a real degradation, so it is reported LOUDLY rather than
//! discovered by a user whose desktop happens to differ (§20).
//!
//! `day rebuild` cannot open an AppImage (an ELF with a squashfs appended), so its payload verdict
//! comes from the recorded digests — of the FLATPAK stage, which the same pack produced from the
//! same compiled binary (`pack::payload_root`, §20.3). One recorded payload therefore covers both
//! Linux artifacts, and neither has to be extractable for the code to be verified.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::settings::PackOptions;
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
    let Some(linuxdeploy) = tool("linuxdeploy") else {
        return Err(PackError::Other(
            "linuxdeploy not found — it is what bundles the toolkit into the AppImage.\n  \
             Download it (and the plugin for your toolkit) from \
             https://github.com/linuxdeploy/linuxdeploy/releases, chmod +x, and put it on PATH \
             as `linuxdeploy`; or set DAY_LINUXDEPLOY to its path (docs/environment.md).\n  \
             `day pack --formats flatpak` packs without it."
                .into(),
        ));
    };

    let outcome = ops::build(project, target, &opts.profile).map_err(PackError::Other)?;
    let id = project.manifest.app.id.clone();
    let title = project
        .manifest
        .app
        .title
        .clone()
        .unwrap_or_else(|| project.manifest.app.name.clone());

    // AppDir layout: AppRun, <id>.desktop and <id>.png at the ROOT, everything else under usr/.
    // linuxdeploy fills in usr/lib; Day stages the rest.
    let work = project.root.join("build/day/appimage").join(target.name);
    let appdir = work.join("AppDir");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&appdir).map_err(|e| PackError::Other(e.to_string()))?;
    let prefix = appdir.join("usr");

    let staged = super::linux::stage_tree(project, target, &outcome.artifact, &prefix)
        .map_err(PackError::Other)?;
    // Exec=AppRun, not the app id: an AppImage's .desktop is read by the host after the file is
    // integrated, and AppRun is the only entry point that exists before that.
    super::linux::stage_exports(project, &prefix, &id, &title, "AppRun")
        .map_err(PackError::Other)?;

    // `$0` is AppRun itself, and `readlink -f` resolves the symlink the host may have made — so
    // HERE is the AppDir root wherever the image mounted this run.
    let apprun = appdir.join("AppRun");
    std::fs::write(
        &apprun,
        super::linux::launcher(
            "$HERE/usr",
            "HERE=\"$(dirname \"$(readlink -f \"$0\")\")\"\n",
            &staged,
        ),
    )
    .map_err(|e| PackError::Other(e.to_string()))?;
    super::linux::set_executable(&apprun).map_err(PackError::Other)?;

    // The format wants both at the AppDir root, beside AppRun. Copies rather than symlinks: a
    // dangling link here fails the pack inside appimagetool, with a message about neither file.
    std::fs::copy(
        prefix
            .join("share/applications")
            .join(format!("{id}.desktop")),
        appdir.join(format!("{id}.desktop")),
    )
    .map_err(|e| PackError::Other(format!("staging the root .desktop: {e}")))?;
    let icon = super::linux::largest_icon(&prefix, &id).ok_or_else(|| {
        PackError::Other(
            "no icon staged for the AppDir root — the AppImage format requires one".into(),
        )
    })?;
    std::fs::copy(&icon, appdir.join(format!("{id}.png")))
        .map_err(|e| PackError::Other(format!("staging the root icon: {e}")))?;

    // --- linuxdeploy: bundle, then seal -----------------------------------------
    let plugin = toolkit_plugin(target.toolkit);
    match plugin {
        Some(p) if tool(&format!("linuxdeploy-plugin-{p}")).is_some() => {
            status("Packing", &format!("linuxdeploy --plugin {p}"));
        }
        Some(p) => {
            status(
                "Warning",
                &format!(
                    "linuxdeploy-plugin-{p} not found — the AppImage will carry the library \
                     closure but NOT {}'s modules, loaders or schemas, so it needs a machine that \
                     already has {}. Install the plugin for a self-contained image.",
                    target.toolkit, target.toolkit
                ),
            );
        }
        None => {}
    }

    let out = dist.join(super::naming::artifact_file(
        project,
        target,
        opts,
        &[arch()],
        "appimage",
    ));
    let _ = std::fs::remove_file(&out);

    let mut cmd = Command::new(&linuxdeploy);
    crate::ops::apply_determinism(&mut cmd);
    cmd.current_dir(&work)
        .arg("--appdir")
        .arg(&appdir)
        .arg("--desktop-file")
        .arg(appdir.join(format!("{id}.desktop")))
        .arg("--icon-file")
        .arg(appdir.join(format!("{id}.png")))
        .arg("--executable")
        .arg(prefix.join("bin").join(format!("{}-bin", staged.name)))
        .args(["--output", "appimage"])
        // linuxdeploy names its output from the .desktop; point it at the name pack chose so the
        // release asset needs no rename (pack/naming.rs).
        .env("OUTPUT", &out)
        // An AppImage embeds a build timestamp unless told otherwise; the reproducible epoch is
        // the same clock every other container in this pack uses (§20.3).
        .env("SOURCE_DATE_EPOCH", super::reproducible_epoch().to_string())
        // linuxdeploy runs appimagetool, which is itself an AppImage — and a CI container has no
        // FUSE. This is the documented escape hatch.
        .env("APPIMAGE_EXTRACT_AND_RUN", "1");
    if let Some(p) = plugin
        && tool(&format!("linuxdeploy-plugin-{p}")).is_some()
    {
        cmd.args(["--plugin", p]);
    }
    run_tool(&mut cmd, "linuxdeploy").map_err(PackError::Other)?;

    if !out.is_file() {
        return Err(PackError::Other(format!(
            "linuxdeploy reported success but produced no {}",
            out.display()
        )));
    }
    super::linux::set_executable(&out).map_err(PackError::Other)?;

    Ok(Artifact {
        path: out,
        kind: "appimage",
        // An AppImage carries no signature of its own. Detached signing (the `.AppImage.zsync`
        // / GPG convention) is not wired up, so the tier is honest about that rather than
        // inheriting the flatpak's.
        sha256: String::new(),
        tier: SignTier::Unsigned,
    })
}

/// linuxdeploy's plugin for a toolkit, by the name it is invoked under.
fn toolkit_plugin(toolkit: &str) -> Option<&'static str> {
    match toolkit {
        "gtk" => Some("gtk"),
        "qt" => Some("qt"),
        _ => None,
    }
}

/// Locate a tool, honouring a `DAY_<TOOL>` override before PATH — the linuxdeploy releases are
/// downloaded AppImages rather than packaged, so they often live outside PATH.
fn tool(name: &str) -> Option<PathBuf> {
    let var = format!("DAY_{}", name.replace('-', "_").to_uppercase());
    if let Ok(p) = std::env::var(&var) {
        let path = PathBuf::from(p);
        return path.is_file().then_some(path);
    }
    std::env::var("PATH").ok().and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join(name))
            .find(|c| c.is_file())
    })
}

/// The CPU architecture in the artifact name. AppImages are per-arch, and a release may carry
/// several — the same reason the flatpak bundle names one.
fn arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_linux_toolkit_names_its_bundling_plugin() {
        assert_eq!(toolkit_plugin("gtk"), Some("gtk"));
        assert_eq!(toolkit_plugin("qt"), Some("qt"));
        assert_eq!(toolkit_plugin("appkit"), None);
    }

    /// The override exists because linuxdeploy ships as a downloaded AppImage, not a package.
    #[test]
    fn a_tool_can_be_pointed_at_explicitly() {
        let this = std::env::current_exe().expect("current exe");
        // SAFETY: test-local env mutation, on a variable no other test touches.
        unsafe { std::env::set_var("DAY_LINUXDEPLOY_PLUGIN_GTK", &this) };
        assert_eq!(tool("linuxdeploy-plugin-gtk"), Some(this));
        unsafe { std::env::set_var("DAY_LINUXDEPLOY_PLUGIN_GTK", "/nope/not/here") };
        assert_eq!(tool("linuxdeploy-plugin-gtk"), None);
        unsafe { std::env::remove_var("DAY_LINUXDEPLOY_PLUGIN_GTK") };
    }
}
