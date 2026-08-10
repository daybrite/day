// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `day pack` (DESIGN.md §16.5): build → sign → installable artifact, per target, with the
//! hoppack-lineage stage order (build → assemble → sign → package → notarize → verify). Every
//! artifact lands in `build/day/dist/` with a sha256 and a signing tier; the tier degrades
//! LOUDLY (never silently) when release signing material is absent (§20).
//!
//! Per-target default formats:
//!   macos-appkit → dmg · ios-uikit → ipa (sim-app without ASC creds) · android-mdc → apk+aab
//!   linux-gtk/linux-qt → flatpak+appimage · windows-xaml → msix+nsis · harmony-arkui → hap
//! GTK/Qt on macOS/Windows is DP-7 (deferred) and refuses with a pointer.

pub(crate) mod android;
mod appimage;
mod flatpak;
mod ios;
pub(crate) mod linux;
mod macos;
mod msix;
pub mod naming;
mod nsis;
mod ohos;
pub mod settings;

use std::path::{Path, PathBuf};

use crate::meta::Project;
use crate::ops::status;
use crate::targets::Target;
pub use settings::PackOptions;

/// How an artifact ended up signed. `DevSigned` covers ad-hoc codesign, debug/CI-generated
/// keystores and self-signed certs — installable for development, not distributable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SignTier {
    Unsigned,
    DevSigned,
    Release,
}

impl SignTier {
    pub fn as_str(self) -> &'static str {
        match self {
            SignTier::Unsigned => "unsigned",
            SignTier::DevSigned => "dev-signed",
            SignTier::Release => "release",
        }
    }
}

pub struct Artifact {
    pub path: PathBuf,
    /// Format tag: "dmg" | "ipa" | "sim-app" | "apk" | "aab" | "flatpak" | "msix" | "nsis" | "hap"
    pub kind: &'static str,
    pub sha256: String,
    pub tier: SignTier,
}

pub struct PackOutcome {
    pub target: &'static str,
    pub artifacts: Vec<Artifact>,
    pub seconds: f64,
}

/// Signing failures exit with code 6 (§16.3); everything else is a build failure (4).
pub enum PackError {
    Sign(String),
    Other(String),
}

impl From<String> for PackError {
    fn from(s: String) -> Self {
        PackError::Other(s)
    }
}

impl PackError {
    pub fn message(&self) -> &str {
        match self {
            PackError::Sign(m) | PackError::Other(m) => m,
        }
    }
    pub fn exit_code(&self) -> i32 {
        match self {
            PackError::Sign(_) => 6,
            PackError::Other(_) => 4,
        }
    }
}

/// The formats this target packs into, or day's own explanation of why it packs none — the answer
/// `day checkup` quotes when it reports a combo as build-only.
pub(crate) fn default_formats(target: &Target) -> Result<Vec<&'static str>, String> {
    Ok(match target.name {
        "macos-appkit" => vec!["dmg"],
        "ios-uikit" => vec!["ipa"], // falls back to sim-app without ASC signing config
        "android-mdc" => vec!["apk", "aab"],
        // Both, and in this order: the flatpak is the desktop-integrated install, the AppImage
        // is the one a `curl … && ./app` line can run with nothing installed (§16.5).
        "linux-gtk" | "linux-qt" => vec!["flatpak", "appimage"],
        "windows-xaml" => vec!["msix", "nsis"],
        "harmony-arkui" => vec!["hap"],
        "macos-gtk" | "macos-qt" | "windows-gtk" | "windows-qt" => {
            return Err(format!(
                "pack for {} means bundling the toolkit into the package — deferred (DESIGN.md \
                 DP-7). Pack the platform-native target instead, or `day launch -p {}` for development.",
                target.name, target.name
            ));
        }
        other => return Err(format!("pack does not support {other}")),
    })
}

pub fn run(
    project: &Project,
    target: &'static Target,
    opts: &PackOptions,
) -> Result<PackOutcome, PackError> {
    let start = std::time::Instant::now();
    let defaults = default_formats(target)?;
    let formats: Vec<String> = match &opts.formats {
        Some(list) => {
            for f in list {
                if !defaults.contains(&f.as_str()) {
                    return Err(PackError::Other(format!(
                        "format {f:?} is not available for {} (available: {})",
                        target.name,
                        defaults.join(", ")
                    )));
                }
            }
            list.clone()
        }
        None => defaults.iter().map(|s| s.to_string()).collect(),
    };

    let dist = project.root.join("build/day/dist");
    std::fs::create_dir_all(&dist).map_err(|e| PackError::Other(e.to_string()))?;

    // Provenance (§20.4). The SBOM is written BEFORE the build so it can be staged into the
    // bundle as a resource; it derives only from source, so it is identical on every machine and
    // does not make the artifact environment-specific.
    let sbom_cfg = project.manifest.sbom.clone();
    let sbom = crate::provenance::collect_sbom(project);
    if !sbom_cfg.is_off() && sbom.dirty {
        status(
            "Warning",
            "the working tree has uncommitted changes — the recorded commit does not describe \
             this artifact, and `day rebuild` cannot reproduce it",
        );
    }
    // Generated into build/day/ regardless of destination; `embed` copies from here into the
    // bundle, `sidecar` copies alongside the artifact, `none` skips generation entirely.
    let sbom_dir = project.root.join("build/day/sbom");
    let _ = std::fs::remove_dir_all(&sbom_dir);
    if !sbom_cfg.is_off() {
        crate::provenance::write_sbom(&sbom_dir, &sbom, &sbom_cfg.formats)
            .map_err(PackError::Other)?;
    }

    let mut artifacts: Vec<Artifact> = Vec::new();
    // The staged compiled code, for the formats whose container cannot be opened on an arbitrary
    // host (see `BuildInfo::payload`). Left `None` for the zip/dmg formats, which `day rebuild`
    // extracts and compares file by file.
    let mut payload_root: Option<std::path::PathBuf> = None;
    match target.name {
        "macos-appkit" => {
            // dmg is the only macOS format today; the .app assembly is its input.
            artifacts.push(macos::pack(project, target, opts, &dist)?);
        }
        "ios-uikit" => {
            artifacts.push(ios::pack(project, target, opts, &dist)?);
        }
        "android-mdc" => {
            artifacts.extend(android::pack(project, target, opts, &dist, &formats)?);
        }
        "linux-gtk" | "linux-qt" => {
            if formats.iter().any(|f| f == "flatpak") {
                artifacts.push(flatpak::pack(project, target, opts, &dist)?);
            }
            if formats.iter().any(|f| f == "appimage") {
                artifacts.push(appimage::pack(project, target, opts, &dist)?);
            }
            payload_root = payload_root_for(project, target);
        }
        "windows-xaml" => {
            let staged = msix::stage_payload(project, target, opts)?;
            payload_root = payload_root_for(project, target);
            if formats.iter().any(|f| f == "msix") {
                artifacts.push(msix::pack(project, target, opts, &staged, &dist)?);
            }
            if formats.iter().any(|f| f == "nsis") {
                artifacts.push(nsis::pack(project, target, opts, &staged, &dist)?);
            }
        }
        "harmony-arkui" => {
            artifacts.push(ohos::pack(project, target, opts, &dist)?);
        }
        other => return Err(PackError::Other(format!("pack does not support {other}"))),
    }

    // Checksums + the loud per-artifact summary (§16.3 result contract).
    for a in &mut artifacts {
        a.sha256 = sha256_file(&a.path).map_err(PackError::Other)?;
        status(
            "Packed",
            &format!(
                "{} ({}, {}) sha256:{}…",
                a.path.display(),
                a.kind,
                a.tier.as_str(),
                &a.sha256[..12]
            ),
        );
        if a.tier != SignTier::Release {
            status(
                "Warning",
                &format!(
                    "{} is {} — NOT distributable (configure Day.toml `signing:` for release signing)",
                    a.path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("artifact"),
                    a.tier.as_str()
                ),
            );
        }
    }

    // The buildinfo sidecar records the machine, so it is deliberately NOT embedded: doing so
    // would make the artifact differ whenever a tool version differs (§20.3).
    let mut info = crate::provenance::collect_buildinfo(target, &opts.profile);
    info.inputs = build_inputs(target);
    if let Some(root) = &payload_root {
        info.payload = payload_digests(root);
    }
    info.artifacts = artifacts
        .iter()
        .map(|a| {
            (
                a.path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string(),
                a.sha256.clone(),
            )
        })
        .collect();
    // Store listing (docs/store.md): the fastlane tree a release lane uploads, generated from `store/`.
    // Written on every pack of a store target so the deploy step never has to run a second tool to
    // get it, and skipped silently for a project with no listing.
    if crate::store::is_store_target(target) {
        match crate::store::read(project) {
            Ok(listing) if !listing.is_empty() => {
                let out = crate::store::stage_dir(project, target);
                match crate::store::stage(project, target, &listing, &out) {
                    Ok(files) => status(
                        "Listing",
                        &format!(
                            "{} ({} file(s), {} locale(s))",
                            out.display(),
                            files.len(),
                            listing.locales.len()
                        ),
                    ),
                    Err(e) => status("Warning", &format!("store listing: {e}")),
                }
            }
            Ok(_) => {}
            Err(e) => status("Warning", &format!("store listing: {e}")),
        }
    }

    // Provenance sidecars are named after the artifact they describe, extension included
    // (`day-showcase-macos-appkit.dmg.buildinfo.json`) — one set PER artifact, because a release
    // directory merges every target's and a bare `day-sbom.cdx.json` there says nothing about
    // which download it belongs to (§20.4). A pack that produced both an .apk and an .aab
    // therefore writes both sets, with identical content.
    let mut sidecars: Vec<PathBuf> = Vec::new();
    for a in &artifacts {
        if sbom_cfg.mode == crate::meta::SbomMode::Sidecar {
            for f in &sbom_cfg.formats {
                let from = sbom_dir.join(f.file_name());
                if from.is_file() {
                    let to = naming::sidecar(&a.path, f.sidecar_suffix());
                    std::fs::copy(&from, &to).map_err(|e| PackError::Other(e.to_string()))?;
                    sidecars.push(to);
                }
            }
        }
        let buildinfo = naming::sidecar(&a.path, "buildinfo.json");
        crate::provenance::write_buildinfo(&buildinfo, &info).map_err(PackError::Other)?;
        sidecars.push(buildinfo);

        // Linux targets additionally get a Debian-format buildinfo (deb-buildinfo(5)), because
        // that is what Debian's reproducibility tooling consumes. The JSON above stays: it is
        // cross-platform and is what `day rebuild` reads.
        if matches!(target.toolkit, "gtk" | "qt") {
            let deb = crate::provenance::debian_buildinfo(
                &sbom,
                &info,
                &project.root,
                Some(flatpak::runtime_for(target)),
            );
            let to = naming::sidecar(&a.path, "buildinfo.deb822");
            std::fs::write(&to, deb).map_err(|e| PackError::Other(e.to_string()))?;
            sidecars.push(to);
        }
    }
    status(
        "Provenance",
        &format!(
            "{} file(s), {} tools · sbom: {}",
            sidecars.len(),
            info.tools.len(),
            if sbom_cfg.is_off() {
                "none".to_string()
            } else {
                format!(
                    "{:?} {} component(s), {}",
                    sbom_cfg.mode,
                    sbom.components.len(),
                    sbom_cfg
                        .formats
                        .iter()
                        .map(|f| match sbom_cfg.mode {
                            crate::meta::SbomMode::Sidecar => f.sidecar_suffix(),
                            _ => f.file_name(),
                        })
                        .collect::<Vec<_>>()
                        .join(" + ")
                )
                .to_lowercase()
            }
        ),
    );

    Ok(PackOutcome {
        target: target.name,
        artifacts,
        seconds: start.elapsed().as_secs_f64(),
    })
}

/// Validate that the windows signing config resolves (shared with `day sign --check`).
pub(crate) fn msix_check(project: &Project) -> Result<(), String> {
    msix::resolve_signing(project)
        .map(|_| ())
        .map_err(|e| e.message().to_string())
}

/// Doctor probe: locate a Windows-Kits tool (None off-Windows or when the SDK is absent).
pub(crate) fn windows_kit_tool_probe(tool: &str) -> Option<String> {
    msix::windows_kit_tool(tool).map(|p| p.display().to_string())
}

/// Doctor probe: locate an AppImage tool the SAME way [`appimage`] does — `DAY_<TOOL>` first, then
/// PATH. A bare PATH lookup would report `linuxdeploy` missing on the machines that set the
/// override (CI downloads the AppImage into a scratch dir), contradicting the pack that succeeds.
pub(crate) fn appimage_tool_probe(tool: &str) -> Option<String> {
    appimage::tool(tool).map(|p| p.display().to_string())
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("checksum {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    // Stream the file through the hasher (sha2 0.11 dropped the `io::Write` hasher impl that let
    // `io::copy` write into it directly).
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("checksum {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    // sha2 0.11's digest output no longer implements `LowerHex` — hex-encode by hand.
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Run a command, returning a readable error with the failing tool's tail output. Under
/// `--verbose` the tool's raw output streams live (via [`crate::ops::run_capture`]).
pub(crate) fn run_tool(cmd: &mut std::process::Command, what: &str) -> Result<(), String> {
    let out = crate::ops::run_capture(cmd, what)?;
    if out.status.success() {
        return Ok(());
    }
    if crate::ops::verbose() {
        // Already streamed live by `--verbose`; don't repeat it.
        return Err(format!("{what} failed"));
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let tail: Vec<&str> = text.lines().rev().take(25).collect();
    Err(format!(
        "{what} failed:\n{}",
        tail.into_iter().rev().collect::<Vec<_>>().join("\n")
    ))
}

/// Copy a directory tree (used for staging payloads).
pub(crate) fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("mkdir {}: {e}", dst.display()))?;
    let entries = std::fs::read_dir(src).map_err(|e| format!("read {}: {e}", src.display()))?;
    for e in entries.flatten() {
        let from = e.path();
        let to = dst.join(e.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)
                .map_err(|e| format!("copy {} → {}: {e}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

// --- Reproducible archives (DESIGN.md §20.3) -----------------------------------------------------
// Every container Day ships is built by a tool that copies file modification times into the archive
// and offers no flag to suppress it. Two packs of identical content therefore differ by exactly the
// wall-clock gap between them. There are two levers, depending on whether Day stages the tree that
// the archiver reads:
//
//   * it does (.ipa, .msix, -setup.exe) → `normalize_mtimes` the staging dir before archiving.
//   * it does not (.hap: hvigor assembles and emits the zip itself) → `normalize_zip_mtimes` on the
//     finished archive, rewriting the timestamps in place.

/// The fixed modification time written into packaged artifacts, in seconds since the Unix epoch.
///
/// `SOURCE_DATE_EPOCH` is the reproducible-builds convention and wins when set. The fallback is
/// 2020-01-01T00:00:00Z rather than the Unix epoch because ZIP's DOS timestamp field cannot encode
/// anything before 1980; an out-of-range value would be clamped by the writer and reintroduce the
/// very variance this removes, so a `SOURCE_DATE_EPOCH` below that floor is ignored.
pub(crate) fn reproducible_epoch() -> i64 {
    const ZIP_FLOOR: i64 = 315_532_800; // 1980-01-01T00:00:00Z
    const DEFAULT: i64 = 1_577_836_800; // 2020-01-01T00:00:00Z
    std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|e| *e >= ZIP_FLOOR)
        .unwrap_or(DEFAULT)
}

/// Stamp every entry under `root` with [`reproducible_epoch`] so an archive built from it is
/// byte-identical between runs.
///
/// `filetime::set_symlink_file_times` stamps a symlink itself rather than its target: std has no
/// equivalent (`File::set_times` follows links, and on Windows a directory cannot even be opened
/// without `FILE_FLAG_BACKUP_SEMANTICS`). The walk is hand-rolled rather than pulling in `walkdir`
/// — this is a tree Day just created, so a general walker's loop detection would be unused weight.
pub(crate) fn normalize_mtimes(root: &Path) -> Result<(), String> {
    let stamp = filetime::FileTime::from_unix_time(reproducible_epoch(), 0);
    let mut stack = vec![root.to_path_buf()];
    let mut entries = Vec::new();
    while let Some(dir) = stack.pop() {
        let read =
            std::fs::read_dir(&dir).map_err(|e| format!("reading {}: {e}", dir.display()))?;
        for entry in read {
            let path = entry
                .map_err(|e| format!("reading {}: {e}", dir.display()))?
                .path();
            // symlink_metadata, not metadata: a broken or absolute symlink must not be followed.
            let meta = std::fs::symlink_metadata(&path)
                .map_err(|e| format!("stat {}: {e}", path.display()))?;
            if meta.is_dir() {
                stack.push(path.clone());
            }
            entries.push(path);
        }
    }
    entries.push(root.to_path_buf());
    // Deepest first, so a directory is stamped after everything inside it.
    entries.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for path in entries {
        filetime::set_symlink_file_times(&path, stamp, stamp)
            .map_err(|e| format!("set mtime on {}: {e}", path.display()))?;
    }
    Ok(())
}

/// [`reproducible_epoch`] as a packed MS-DOS (date, time) pair, the form ZIP stores.
///
/// DOS packs a date into 16 bits as `year-1980 << 9 | month << 5 | day`, and a time as
/// `hour << 11 | minute << 5 | second/2` — hence the two-second resolution.
fn dos_datetime() -> (u16, u16) {
    let secs = reproducible_epoch();
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil-from-days (Howard Hinnant's algorithm), shifted to a March-based year so leap days
    // land at the end of the cycle and no month-length table is needed.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u16;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u16;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as u16;
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let date = (year.saturating_sub(1980) << 9) | (month << 5) | day;
    let time = ((hour as u16) << 11) | ((minute as u16) << 5) | (second as u16 / 2);
    (date, time)
}

/// Rewrite every timestamp in a ZIP archive to [`reproducible_epoch`], in place.
///
/// For archives Day does not stage — hvigor emits the `.hap` itself — normalizing the source tree
/// is not an option, so the finished container is patched instead. Entry offsets and compressed
/// data are untouched, so this cannot invalidate the archive; only the DOS date/time words in each
/// local header and central-directory record change. Any run afterwards (hap signing) is unaffected
/// because it appends its own block rather than rewriting these.
pub(crate) fn normalize_zip_mtimes(archive: &Path) -> Result<(), String> {
    const EOCD_SIG: &[u8] = b"PK\x05\x06";
    const CD_SIG: [u8; 4] = *b"PK\x01\x02";
    const LFH_SIG: [u8; 4] = *b"PK\x03\x04";
    let mut buf =
        std::fs::read(archive).map_err(|e| format!("reading {}: {e}", archive.display()))?;
    // The end-of-central-directory record is last, but a trailing comment may follow it, so scan
    // back from the end for its signature.
    let eocd = (0..buf.len().saturating_sub(21))
        .rev()
        .find(|&i| buf[i..i + 4] == *EOCD_SIG)
        .ok_or_else(|| format!("{}: no ZIP end-of-central-directory", archive.display()))?;
    let count = u16::from_le_bytes([buf[eocd + 10], buf[eocd + 11]]) as usize;
    let mut cd = u32::from_le_bytes([
        buf[eocd + 16],
        buf[eocd + 17],
        buf[eocd + 18],
        buf[eocd + 19],
    ]) as usize;
    let (date, time) = dos_datetime();
    let epoch = reproducible_epoch();
    for _ in 0..count {
        if cd + 46 > buf.len() || buf[cd..cd + 4] != CD_SIG {
            return Err(format!(
                "{}: malformed central directory",
                archive.display()
            ));
        }
        buf[cd + 12..cd + 14].copy_from_slice(&time.to_le_bytes());
        buf[cd + 14..cd + 16].copy_from_slice(&date.to_le_bytes());
        let name = u16::from_le_bytes([buf[cd + 28], buf[cd + 29]]) as usize;
        let extra = u16::from_le_bytes([buf[cd + 30], buf[cd + 31]]) as usize;
        let comment = u16::from_le_bytes([buf[cd + 32], buf[cd + 33]]) as usize;
        let lfh =
            u32::from_le_bytes([buf[cd + 42], buf[cd + 43], buf[cd + 44], buf[cd + 45]]) as usize;
        normalize_extra_timestamps(&mut buf, cd + 46 + name, extra, epoch);
        if lfh + 30 <= buf.len() && buf[lfh..lfh + 4] == LFH_SIG {
            buf[lfh + 10..lfh + 12].copy_from_slice(&time.to_le_bytes());
            buf[lfh + 12..lfh + 14].copy_from_slice(&date.to_le_bytes());
            let lname = u16::from_le_bytes([buf[lfh + 26], buf[lfh + 27]]) as usize;
            let lextra = u16::from_le_bytes([buf[lfh + 28], buf[lfh + 29]]) as usize;
            normalize_extra_timestamps(&mut buf, lfh + 30 + lname, lextra, epoch);
        } else {
            return Err(format!(
                "{}: central directory points at {lfh}, which is not a local file header",
                archive.display()
            ));
        }
        cd += 46 + name + extra + comment;
    }
    std::fs::write(archive, &buf).map_err(|e| format!("writing {}: {e}", archive.display()))
}

/// Rewrite the Unix timestamps inside a ZIP extra-field block, leaving every field's length alone.
///
/// The DOS date/time words are not the only clock in a ZIP: the "extended timestamp" field (`0x5455`)
/// carries 32-bit Unix times, and it is what `unzip -l` and diffoscope actually report. Normalizing
/// only the DOS words leaves the archive looking unchanged in every tool that prefers this field.
/// `0x000a` (NTFS) stores 64-bit FILETIMEs and is normalized the same way.
fn normalize_extra_timestamps(buf: &mut [u8], mut at: usize, len: usize, epoch: i64) {
    const EXTENDED: u16 = 0x5455;
    const NTFS: u16 = 0x000a;
    // FILETIME counts 100ns ticks from 1601-01-01; 11644473600 s separates that from the Unix epoch.
    let filetime = ((epoch + 11_644_473_600) as u64).saturating_mul(10_000_000);
    let end = (at + len).min(buf.len());
    while at + 4 <= end {
        let id = u16::from_le_bytes([buf[at], buf[at + 1]]);
        let size = u16::from_le_bytes([buf[at + 2], buf[at + 3]]) as usize;
        let body = at + 4;
        if body + size > end {
            return; // malformed; leave the rest alone rather than corrupt it
        }
        match id {
            // flags byte, then up to three 4-byte times (mtime, atime, ctime) per the flag bits.
            EXTENDED if size >= 5 => {
                let mut p = body + 1;
                while p + 4 <= body + size {
                    buf[p..p + 4].copy_from_slice(&(epoch as u32).to_le_bytes());
                    p += 4;
                }
            }
            // reserved(4) + tag(2) + tagsize(2), then mtime/atime/ctime as 8-byte FILETIMEs.
            NTFS if size >= 32 => {
                let mut p = body + 8;
                while p + 8 <= body + size {
                    buf[p..p + 8].copy_from_slice(&filetime.to_le_bytes());
                    p += 8;
                }
            }
            _ => {}
        }
        at = body + size;
    }
}

#[cfg(test)]
mod repro_tests {
    use super::*;
    use std::sync::Mutex;

    /// `SOURCE_DATE_EPOCH` is process-global and these tests write it, so they must not overlap.
    /// They did: with the harness running them on separate threads, one test's `1234567890` was
    /// visible to the other, which then read 2009 where it asserted 2020. Same lock pattern as
    /// mobile.rs's ABI tests; `into_inner` because a panic in one must not cascade into the other.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn epoch_honours_source_date_epoch_above_the_zip_floor() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::remove_var("SOURCE_DATE_EPOCH") };
        assert_eq!(
            reproducible_epoch(),
            1_577_836_800,
            "default is 2020-01-01Z"
        );
        unsafe { std::env::set_var("SOURCE_DATE_EPOCH", "1234567890") };
        assert_eq!(
            reproducible_epoch(),
            1_234_567_890,
            "an in-range value wins"
        );
        // Below 1980 ZIP cannot represent it, so the floor rejects rather than letting the writer
        // clamp it back into per-run variance.
        unsafe { std::env::set_var("SOURCE_DATE_EPOCH", "100") };
        assert_eq!(reproducible_epoch(), 1_577_836_800);
        unsafe { std::env::set_var("SOURCE_DATE_EPOCH", "not-a-number") };
        assert_eq!(reproducible_epoch(), 1_577_836_800);
        unsafe { std::env::remove_var("SOURCE_DATE_EPOCH") };
    }

    #[test]
    fn dos_datetime_packs_the_default_epoch() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::remove_var("SOURCE_DATE_EPOCH") };
        let (date, time) = dos_datetime();
        // 2020-01-01T00:00:00Z → year 2020, month 1, day 1, midnight.
        assert_eq!(date >> 9, 2020 - 1980, "year");
        assert_eq!((date >> 5) & 0xF, 1, "month");
        assert_eq!(date & 0x1F, 1, "day");
        assert_eq!(time, 0, "midnight packs to zero");
    }

    #[test]
    fn extended_timestamp_extra_field_is_rewritten() {
        // 0x5455 "extended timestamp": id, size, flags, then mtime (and here atime).
        let epoch: i64 = 1_577_836_800;
        let mut buf = Vec::new();
        buf.extend_from_slice(&0x5455u16.to_le_bytes());
        buf.extend_from_slice(&9u16.to_le_bytes()); // flags + two 4-byte times
        buf.push(0x03);
        buf.extend_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        buf.extend_from_slice(&0xFEED_FACEu32.to_le_bytes());
        let len = buf.len();
        normalize_extra_timestamps(&mut buf, 0, len, epoch);
        assert_eq!(
            u32::from_le_bytes(buf[5..9].try_into().unwrap()),
            epoch as u32
        );
        assert_eq!(
            u32::from_le_bytes(buf[9..13].try_into().unwrap()),
            epoch as u32
        );
        assert_eq!(
            u16::from_le_bytes(buf[0..2].try_into().unwrap()),
            0x5455,
            "id preserved"
        );
        assert_eq!(
            u16::from_le_bytes(buf[2..4].try_into().unwrap()),
            9,
            "length preserved"
        );
    }

    #[test]
    fn a_malformed_extra_field_is_left_alone_rather_than_corrupted() {
        // Declared size runs past the block: the walker must bail, not write out of bounds.
        let mut buf = vec![0x55, 0x54, 0xFF, 0xFF, 0x03, 1, 2, 3, 4];
        let before = buf.clone();
        let len = buf.len();
        normalize_extra_timestamps(&mut buf, 0, len, 1_577_836_800);
        assert_eq!(buf, before);
    }
}

/// Environment inputs that decided the SHAPE of this artifact, resolved to what the build used.
///
/// Only variables whose default is machine-dependent belong here. `DAY_ANDROID_ABI` and
/// `DAY_OHOS_ARCH` fall back to "whatever device is attached, else a fixed default", so a rebuild
/// on a runner with nothing plugged in packs a different set of `.so`s than the CI machine that
/// had an emulator running — the artifacts then differ structurally, for a reason no verdict could
/// explain (§20.3).
fn build_inputs(target: &'static Target) -> Vec<(String, String)> {
    match target.toolkit {
        "mdc" => vec![(
            "DAY_ANDROID_ABI".to_string(),
            crate::mobile::android_build_abis().join(","),
        )],
        "arkui" => vec![(
            "DAY_OHOS_ARCH".to_string(),
            crate::ohos::build_abis().join(","),
        )],
        _ => Vec::new(),
    }
}

/// Where a target stages its compiled code before packaging, when it has such a place. `None` for
/// the formats `day rebuild` can simply open (zip family, dmg).
fn payload_root_for(project: &Project, target: &'static Target) -> Option<PathBuf> {
    payload_root(&project.root, target)
}

pub(crate) fn payload_root(project_root: &Path, target: &'static Target) -> Option<PathBuf> {
    match target.name {
        "linux-gtk" | "linux-qt" => Some(
            project_root
                .join("build/day/flatpak")
                .join(target.name)
                .join("stage/bin"),
        ),
        "windows-xaml" => Some(project_root.join("build/day/pack/windows-payload")),
        _ => None,
    }
}

/// sha256 of every file under a staged payload root, keyed by its relative path (sorted, so the
/// record is stable across filesystems).
pub(crate) fn payload_digests(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(d) = sha256_file(&p)
                && let Ok(rel) = p.strip_prefix(root)
            {
                out.push((rel.to_string_lossy().replace('\\', "/"), d));
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod input_tests {
    use super::*;

    /// Which variables each package's shape depends on. Recording the wrong name (or none) is how
    /// android shipped two ABIs and rebuilt with one.
    #[test]
    fn build_inputs_names_the_variables_that_shape_each_package() {
        let keys = |name: &str| {
            build_inputs(crate::targets::find(name).expect("target"))
                .into_iter()
                .map(|(k, _)| k)
                .collect::<Vec<_>>()
        };
        assert_eq!(keys("android-mdc"), ["DAY_ANDROID_ABI"]);
        assert_eq!(keys("harmony-arkui"), ["DAY_OHOS_ARCH"]);
        assert!(keys("macos-appkit").is_empty());
        assert!(keys("linux-gtk").is_empty());
    }

    /// Every target that stages a payload must stage it where `day rebuild` looks.
    #[test]
    fn payload_roots_match_where_pack_stages() {
        let root = Path::new("/proj");
        assert_eq!(
            payload_root(root, crate::targets::find("linux-gtk").unwrap()),
            Some(root.join("build/day/flatpak/linux-gtk/stage/bin"))
        );
        assert_eq!(
            payload_root(root, crate::targets::find("windows-xaml").unwrap()),
            Some(root.join("build/day/pack/windows-payload"))
        );
        assert_eq!(
            payload_root(root, crate::targets::find("macos-appkit").unwrap()),
            None
        );
    }
}
