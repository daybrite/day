//! linux-gtk / linux-qt → single-file .flatpak bundle. The runtime supplies the toolkit
//! (GTK4 ⇒ org.gnome.Platform, Qt6 ⇒ org.kde.Platform — no toolkit bundling, which also keeps
//! Qt-LGPL obligations satisfied by the runtime's relinkable shared libs). Day stages the prebuilt
//! release binary + resources into /app (the Tauri/Spotube repack pattern — no build-from-source),
//! generates the app-id-named exports (.desktop, metainfo.xml, hicolor icons), then
//! flatpak-builder → repo → `flatpak build-bundle` with --runtime-repo so the runtime resolves
//! from Flathub at install time. Flathub-ready offline manifests are a later mode.
//!
//! One dependency is NOT in a runtime: QtWebEngine. A flatpak `base:` is copied INTO the app at
//! build time (a `runtime:` is resolved at install), so naming the Qt WebEngine BaseApp adds
//! ~87 MB of Chromium to the bundle. Day names it only when the packed binary actually links
//! WebEngine — see [`links_qt_webengine`].

use std::io::{Read, Seek, SeekFrom};
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
    // SBOM into share/<name>/sbom so the packaged app can read it (§20.4).
    if project.manifest.sbom.mode == crate::meta::SbomMode::Embed {
        crate::provenance::embed_into(&project.root.join("build/day/sbom"), &share_app)
            .map_err(PackError::Other)?;
    }
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
    // The WebEngine BaseApp is dead weight for a Qt app that never opens a webview, so ask the
    // binary. An unreadable/unexpected ELF answers "don't know" — take the base, since a bundle
    // that is too big still runs and one missing QtWebEngine does not.
    let webengine = target.toolkit == "qt" && links_qt_webengine(&outcome.artifact).unwrap_or(true);
    if target.toolkit == "qt" {
        status(
            "Packing",
            if webengine {
                "linked against QtWebEngine — bundling the Qt WebEngine BaseApp"
            } else {
                "no QtWebEngine link — packing without the Qt WebEngine BaseApp"
            },
        );
    }
    let manifest_path = work.join(format!("{id}.yml"));
    std::fs::write(&manifest_path, manifest_yaml(target, &id, &name, webengine))
        .map_err(|e| PackError::Other(e.to_string()))?;

    // --- flatpak-builder → repo → bundle ---------------------------------------
    status("Packing", "flatpak-builder");
    let mut fb = Command::new("flatpak-builder");
    crate::ops::apply_determinism(&mut fb);
    run_tool(
        fb.current_dir(&work)
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
    let bundle = dist.join(format!(
        "{name}{}-{toolkit}-{arch}.flatpak",
        opts.version_tag(version)
    ));
    let _ = std::fs::remove_file(&bundle);
    status("Packing", "flatpak build-bundle");
    let mut bundle_cmd = Command::new("flatpak");
    crate::ops::apply_determinism(&mut bundle_cmd);
    run_tool(
        bundle_cmd
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
/// `webengine` adds the Qt WebEngine BaseApp — only for a Qt app that links it (§16.5).
/// The Flathub runtime a target links against, as `(id, version)`.
///
/// Recorded in the Debian `.buildinfo` (§20.4): `Installed-Build-Depends` describes the machine that
/// ran the build, but a flatpak app runs against this runtime, not against the build host's
/// packages. Without it the buildinfo would describe only half of what a rebuild needs.
pub(crate) fn runtime_for(target: &Target) -> (&'static str, String) {
    match target.toolkit {
        "qt" => (
            "org.kde.Platform",
            std::env::var("DAY_KDE_RUNTIME").unwrap_or_else(|_| KDE_RUNTIME_VERSION.into()),
        ),
        _ => (
            "org.gnome.Platform",
            std::env::var("DAY_GNOME_RUNTIME").unwrap_or_else(|_| GNOME_RUNTIME_VERSION.into()),
        ),
    }
}

pub(crate) fn manifest_yaml(target: &Target, id: &str, name: &str, webengine: bool) -> String {
    let (runtime, runtime_version) = runtime_for(target);
    let sdk = runtime.replace(".Platform", ".Sdk");
    // Qt apps that link WebEngine need the BaseApp (QtWebEngine is not in org.kde.Platform).
    let base = if webengine {
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

/// Does this ELF binary link QtWebEngine? Reads the shared-library names the dynamic linker will
/// load (`DT_NEEDED`) and looks for a `libQt6WebEngine*` among them — the piece links
/// `Qt6WebEngineWidgets` directly (pieces/day-piece-webview/build.rs), so the link is recorded
/// here whenever a webview is actually compiled in.
///
/// `None` = "can't tell": not the ELF64 little-endian shape Day packs flatpaks for (x86_64,
/// aarch64), unreadable, or statically linked. Callers treat that as "assume yes".
///
/// Header offsets are the ELF64 spec's; the file is read through a handful of small seeks rather
/// than slurped, since a release binary with debug info can be hundreds of megabytes.
fn links_qt_webengine(binary: &Path) -> Option<bool> {
    const SHT_DYNAMIC: u32 = 6;
    const DT_NULL: u64 = 0;
    const DT_NEEDED: u64 = 1;
    const SHDR_LEN: usize = 64;

    let mut f = std::fs::File::open(binary).ok()?;
    let mut ehdr = [0u8; 64];
    f.read_exact(&mut ehdr).ok()?;
    // \x7fELF, ELFCLASS64, ELFDATA2LSB.
    if ehdr[..4] != *b"\x7fELF" || ehdr[4] != 2 || ehdr[5] != 1 {
        return None;
    }
    fn u16_at(b: &[u8], o: usize) -> Option<u16> {
        Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
    }
    fn u32_at(b: &[u8], o: usize) -> Option<u32> {
        Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
    }
    fn u64_at(b: &[u8], o: usize) -> Option<u64> {
        Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
    }
    let e_shoff = u64_at(&ehdr, 0x28)?;
    let e_shentsize = u16_at(&ehdr, 0x3a)? as usize;
    let e_shnum = u16_at(&ehdr, 0x3c)? as usize;
    if e_shentsize < SHDR_LEN || e_shnum == 0 {
        return None;
    }

    // One section header: (type, offset, size, link).
    let mut read_shdr = |i: usize| -> Option<(u32, u64, u64, u32)> {
        let at = e_shoff.checked_add((i * e_shentsize) as u64)?;
        f.seek(SeekFrom::Start(at)).ok()?;
        let mut sh = [0u8; SHDR_LEN];
        f.read_exact(&mut sh).ok()?;
        Some((
            u32_at(&sh, 4)?,
            u64_at(&sh, 24)?,
            u64_at(&sh, 32)?,
            u32_at(&sh, 40)?,
        ))
    };
    let (dyn_off, dyn_size, strtab_idx) = (0..e_shnum)
        .filter_map(&mut read_shdr)
        .find(|(kind, ..)| *kind == SHT_DYNAMIC)
        .map(|(_, off, size, link)| (off, size, link as usize))?;
    let (_, str_off, str_size, _) = read_shdr(strtab_idx)?;

    // Walk the dynamic array (16-byte entries: tag, value) collecting DT_NEEDED string offsets.
    let mut needed = Vec::new();
    for i in 0..(dyn_size / 16) {
        f.seek(SeekFrom::Start(dyn_off.checked_add(i * 16)?)).ok()?;
        let mut ent = [0u8; 16];
        f.read_exact(&mut ent).ok()?;
        match u64_at(&ent, 0)? {
            DT_NULL => break,
            DT_NEEDED => needed.push(u64_at(&ent, 8)?),
            _ => {}
        }
    }
    // Each value indexes .dynstr; read the NUL-terminated name there. 256 bytes covers any
    // soname (a longer one simply won't match the prefix we're looking for).
    for off in needed {
        if off >= str_size {
            continue;
        }
        f.seek(SeekFrom::Start(str_off.checked_add(off)?)).ok()?;
        let mut buf = [0u8; 256];
        let n = f.read(&mut buf).ok()?;
        let name = buf[..n].split(|b| *b == 0).next().unwrap_or_default();
        if name.starts_with(b"libQt6WebEngine") {
            return Some(true);
        }
    }
    Some(false)
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
        let gtk = manifest_yaml(
            targets::find("linux-gtk").unwrap(),
            "dev.x.app",
            "app",
            false,
        );
        assert!(gtk.contains("runtime: org.gnome.Platform"));
        assert!(gtk.contains("sdk: org.gnome.Sdk"));
        assert!(!gtk.contains("base:"));
        let qt = manifest_yaml(targets::find("linux-qt").unwrap(), "dev.x.app", "app", true);
        assert!(qt.contains("runtime: org.kde.Platform"));
        assert!(qt.contains("base: io.qt.qtwebengine.BaseApp"));
        assert!(qt.contains("command: dev.x.app"));
        // A Qt app with no WebEngine link keeps the runtime but drops the ~87 MB BaseApp.
        let lean = manifest_yaml(
            targets::find("linux-qt").unwrap(),
            "dev.x.app",
            "app",
            false,
        );
        assert!(lean.contains("runtime: org.kde.Platform"));
        assert!(!lean.contains("base:"));
        // Both manifests must be valid YAML and skip the debuginfo split (its eu-strip
        // dependency isn't installed on CI runners).
        for manifest in [&gtk, &qt, &lean] {
            let parsed: serde_json::Value = serde_norway::from_str(manifest).unwrap();
            assert_eq!(parsed["build-options"]["no-debuginfo"], true);
            assert_eq!(parsed["build-options"]["strip"], false);
        }
    }

    /// A minimal ELF64 LE file whose dynamic section lists `names` as DT_NEEDED — enough shape
    /// for [`links_qt_webengine`], so the probe is testable on every host, not just Linux.
    fn elf_needing(names: &[&str]) -> Vec<u8> {
        const STR_OFF: usize = 0x100;
        const DYN_OFF: usize = 0x400;
        const SH_OFF: usize = 0x800;
        let mut dynstr = vec![0u8];
        let mut dynamic = Vec::new();
        for n in names {
            dynamic.extend_from_slice(&1u64.to_le_bytes()); // DT_NEEDED
            dynamic.extend_from_slice(&(dynstr.len() as u64).to_le_bytes());
            dynstr.extend_from_slice(n.as_bytes());
            dynstr.push(0);
        }
        dynamic.extend_from_slice(&[0u8; 16]); // DT_NULL

        let mut f = vec![0u8; SH_OFF + 3 * 64];
        f[..4].copy_from_slice(b"\x7fELF");
        (f[4], f[5]) = (2, 1); // ELFCLASS64, ELFDATA2LSB
        f[0x28..0x30].copy_from_slice(&(SH_OFF as u64).to_le_bytes()); // e_shoff
        f[0x3a..0x3c].copy_from_slice(&64u16.to_le_bytes()); // e_shentsize
        f[0x3c..0x3e].copy_from_slice(&3u16.to_le_bytes()); // e_shnum
        f[STR_OFF..STR_OFF + dynstr.len()].copy_from_slice(&dynstr);
        f[DYN_OFF..DYN_OFF + dynamic.len()].copy_from_slice(&dynamic);
        // Section headers: [0] null, [1] .dynstr, [2] .dynamic (sh_link → .dynstr).
        let mut shdr = |i: usize, kind: u32, off: usize, size: usize, link: u32| {
            let at = SH_OFF + i * 64;
            f[at + 4..at + 8].copy_from_slice(&kind.to_le_bytes());
            f[at + 24..at + 32].copy_from_slice(&(off as u64).to_le_bytes());
            f[at + 32..at + 40].copy_from_slice(&(size as u64).to_le_bytes());
            f[at + 40..at + 44].copy_from_slice(&link.to_le_bytes());
        };
        shdr(1, 3, STR_OFF, dynstr.len(), 0); // SHT_STRTAB
        shdr(2, 6, DYN_OFF, dynamic.len(), 1); // SHT_DYNAMIC
        f
    }

    #[test]
    fn webengine_probe_reads_dt_needed() {
        let dir = std::env::temp_dir().join(format!("day-flatpak-elf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |stem: &str, bytes: &[u8]| {
            let p = dir.join(stem);
            std::fs::write(&p, bytes).unwrap();
            p
        };

        let with = write(
            "with",
            &elf_needing(&[
                "libQt6Widgets.so.6",
                "libQt6WebEngineWidgets.so.6",
                "libc.so.6",
            ]),
        );
        assert_eq!(links_qt_webengine(&with), Some(true));

        let without = write(
            "without",
            &elf_needing(&["libQt6Widgets.so.6", "libQt6Gui.so.6", "libc.so.6"]),
        );
        assert_eq!(links_qt_webengine(&without), Some(false));

        // Not an ELF (e.g. a Mach-O host build): "can't tell" — the caller keeps the BaseApp.
        let alien = write("alien", b"\xcf\xfa\xed\xfe not an elf at all");
        assert_eq!(links_qt_webengine(&alien), None);
        assert_eq!(links_qt_webengine(&dir.join("nonexistent")), None);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
