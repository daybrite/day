// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! What a packaged artifact is called (§16.5, §20.4).
//!
//! Every format lands on ONE pattern:
//!
//! ```text
//! <stem>[-<version>]-<target>[-<extra>…].<ext>
//!   day-showcase-macos-appkit.dmg
//!   day-showcase-windows-xaml-setup.exe
//!   day-showcase-linux-gtk-x86_64.flatpak
//!   day-showcase-1.2.0-android-mdc.aab          (without --no-version-in-name)
//! ```
//!
//! The target combo is part of the name the CLI writes, not something release CI splices in
//! afterwards. That matters three ways: `build/day/dist/` holds several targets at once and their
//! names have to be distinct; `day rebuild <downloaded-asset>` looks for a rebuilt file of the
//! SAME name, so the local name must be the published one; and the provenance sidecars are named
//! after the artifact they describe, which is only possible once the artifact's final name is
//! known here.
//!
//! Sidecars hang off the artifact's whole file name, extension included, so a release directory
//! sorts them next to what they describe and never has to guess which `.dmg` a buildinfo belongs
//! to:
//!
//! ```text
//! day-showcase-macos-appkit.dmg.buildinfo.json
//! day-showcase-macos-appkit.dmg.sbom-cdx.json
//! day-showcase-macos-appkit.dmg.sbom-spdx.json
//! ```

use crate::meta::Project;
use crate::targets::Target;

use super::PackOptions;

/// The filename stem every artifact of this project shares, before the target combo.
///
/// Precedence: `day pack --artifact-name` > Day.toml `[app] artifact` (including any
/// `[app.<target>]` override) > a slug of the app title. Always slugged, so an override cannot
/// introduce a space or a capital that GitHub would rewrite on upload.
pub fn stem(project: &Project, target: &Target, opts: &PackOptions) -> String {
    match &opts.artifact_name {
        Some(explicit) => crate::meta::slug(explicit),
        None => project.manifest.resolve(target.name).artifact,
    }
}

/// The full file name for one packaged artifact.
///
/// `extra` are the tokens that distinguish artifacts sharing a target and extension — `setup` for
/// the NSIS installer, the CPU arch for a flatpak, `unsigned` for an ipa packed without signing
/// material. They follow the target so the combo stays a contiguous, greppable token.
pub fn artifact_file(
    project: &Project,
    target: &Target,
    opts: &PackOptions,
    extra: &[&str],
    ext: &str,
) -> String {
    let version = opts.version_tag(&project.manifest.app.version);
    let mut name = format!("{}{version}-{}", stem(project, target, opts), target.name);
    for token in extra {
        name.push('-');
        name.push_str(token);
    }
    name.push('.');
    name.push_str(ext);
    name
}

/// The provenance sidecar for `artifact`, as `<artifact file name>.<suffix>`.
///
/// Takes the artifact path rather than rebuilding the name from parts: a sidecar that does not
/// match its artifact byte for byte is worse than no sidecar, and this way it cannot drift.
pub fn sidecar(artifact: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let name = artifact
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    artifact.with_file_name(format!("{name}.{suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::slug;

    /// A project that exists only in memory — naming is pure string work, so nothing here needs
    /// a directory on disk.
    fn fixture(title: Option<&str>, artifact: Option<&str>) -> Project {
        let mut day = String::from("schema = 1\n[app]\nid = \"dev.example.showcase\"\n");
        if let Some(t) = title {
            day.push_str(&format!("title = {t:?}\n"));
        }
        if let Some(a) = artifact {
            day.push_str(&format!("artifact = {a:?}\n"));
        }
        let manifest = crate::meta::parse_manifest(
            &day,
            "[package]\nname = \"showcase\"\nversion = \"1.2.0\"\n",
            None,
        )
        .expect("manifest");
        Project {
            root: std::path::PathBuf::from("/proj"),
            manifest,
        }
    }

    #[test]
    fn slug_folds_to_a_filename_safe_token() {
        assert_eq!(slug("Day Showcase"), "day-showcase");
        assert_eq!(slug("day-showcase"), "day-showcase");
        // Runs of separators collapse, and the edges are trimmed — never `--` or a leading `-`.
        assert_eq!(slug("  Day   Skies!! "), "day-skies");
        assert_eq!(slug("Tradr 2.0"), "tradr-2-0");
        // Non-ASCII folds to the separator rather than reaching a URL unescaped (and here the
        // separator is then trimmed, being last).
        assert_eq!(slug("Café"), "caf");
        assert_eq!(slug("Café Noir"), "caf-noir");
        assert_eq!(slug("!!!"), "app", "a name that folds away still needs one");
    }

    /// The whole point of the change: one pattern, whatever the format.
    #[test]
    fn every_format_lands_on_the_same_pattern() {
        let project = fixture(Some("Day Showcase"), None);
        let opts = PackOptions {
            version_in_name: false,
            ..PackOptions::default()
        };
        let file = |t: &str, extra: &[&str], ext: &str| {
            artifact_file(
                &project,
                crate::targets::find(t).unwrap(),
                &opts,
                extra,
                ext,
            )
        };
        assert_eq!(
            file("macos-appkit", &[], "dmg"),
            "day-showcase-macos-appkit.dmg"
        );
        assert_eq!(file("ios-uikit", &[], "ipa"), "day-showcase-ios-uikit.ipa");
        assert_eq!(
            file("ios-uikit", &["unsigned"], "ipa"),
            "day-showcase-ios-uikit-unsigned.ipa"
        );
        assert_eq!(
            file("android-mdc", &[], "aab"),
            "day-showcase-android-mdc.aab"
        );
        assert_eq!(
            file("windows-xaml", &["setup"], "exe"),
            "day-showcase-windows-xaml-setup.exe"
        );
        assert_eq!(
            file("linux-gtk", &["x86_64"], "flatpak"),
            "day-showcase-linux-gtk-x86_64.flatpak"
        );
        assert_eq!(
            file("harmony-arkui", &[], "hap"),
            "day-showcase-harmony-arkui.hap"
        );
    }

    /// `--no-version-in-name` is what keeps a `releases/latest/download/<name>` URL stable; with
    /// the version in, it sits between the stem and the combo so the combo stays contiguous.
    #[test]
    fn the_version_infix_precedes_the_target_combo() {
        let project = fixture(Some("Day Showcase"), None);
        let target = crate::targets::find("android-mdc").unwrap();
        assert_eq!(
            artifact_file(&project, target, &PackOptions::default(), &[], "aab"),
            "day-showcase-1.2.0-android-mdc.aab"
        );
    }

    #[test]
    fn the_stem_comes_from_the_flag_then_the_manifest_then_the_title() {
        let target = crate::targets::find("macos-appkit").unwrap();
        let flagged = PackOptions {
            artifact_name: Some("Custom Name".into()),
            ..PackOptions::default()
        };
        // The flag wins, and is slugged like any other source.
        let project = fixture(Some("Day Showcase"), None);
        assert_eq!(stem(&project, target, &flagged), "custom-name");
        assert_eq!(
            stem(&project, target, &PackOptions::default()),
            "day-showcase"
        );
        // No title: the crate name carries it.
        let untitled = fixture(None, None);
        assert_eq!(stem(&untitled, target, &PackOptions::default()), "showcase");
        // `[app] artifact` beats the title, for an app whose display name is not what its
        // downloads should be called.
        let named = fixture(Some("Day Showcase"), Some("showcase"));
        assert_eq!(stem(&named, target, &PackOptions::default()), "showcase");
    }

    #[test]
    fn a_sidecar_carries_the_artifacts_whole_file_name() {
        let dmg = std::path::Path::new("/dist/day-showcase-macos-appkit.dmg");
        assert_eq!(
            sidecar(dmg, "buildinfo.json"),
            std::path::Path::new("/dist/day-showcase-macos-appkit.dmg.buildinfo.json")
        );
        // The extension stays IN the stem, so the .apk and .aab sidecars of one pack differ.
        let apk = std::path::Path::new("/dist/day-showcase-android-mdc.apk");
        let aab = std::path::Path::new("/dist/day-showcase-android-mdc.aab");
        assert_ne!(sidecar(apk, "sbom-cdx.json"), sidecar(aab, "sbom-cdx.json"));
    }
}
