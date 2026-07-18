//! ICU locale-data thinning (docs/localization.md "Locale data"; docs/environment.md).
//!
//! day-l10n's icu4x components default to `compiled_data` — correct for every locale on Earth,
//! but ~1.5 MB of a release binary (M0 measurements). This module is the Flutter-style fix:
//! locale data follows the app's DECLARED locale set. `day build` runs icu4x's datagen as a
//! library over `resource/locales/*` ∪ the day-l10n core-catalog locales, bakes a thinned data
//! directory under `~/.day/icu/baked/<key>/`, and points every cargo invocation at it via the
//! compile-time `ICU4X_DATA_DIR` override — the icu data crates swap their bundled data for ours
//! while the normal constructors (and dead-code elimination of unused markers) keep working.
//!
//! Every failure degrades to the full compiled data with a warning — thinning is a size
//! optimization, NEVER a build blocker. Bare `cargo` builds (no day CLI) simply embed full data.
//!
//! Source data: datagen's `SourceDataProvider` fetches pinned CLDR/ICU-export tags into
//! `ICU4X_SOURCE_CACHE` on FIRST use (~100 MB, one-time; we point it at `~/.day/icu/src` so it
//! survives reboots and is shared across projects). `DAY_NO_ICU_FETCH` — or the existing
//! `DAY_NO_UPDATE_CHECK` offline switch — skips the fetch (and thinning) when nothing is cached.

use std::path::PathBuf;
use std::process::Command;

// In scope for the registry-generated `all_markers` (`<Marker>::INFO` is a trait const).
use icu_provider::DataMarker as _;

use crate::meta::Project;
use crate::ops::status;

/// The day-l10n built-in core catalog's locales — data for these is always included so framework
/// strings (dialog buttons, menu roles) format correctly even in apps with fewer app locales.
/// Must match `crates/day-l10n/catalog/*.ftl` (guarded by `core_locales_match_catalog` below).
const CORE_LOCALES: &[&str] = &["en", "fr", "es", "de", "ja", "zh"];

/// The icu_provider minor version this day-cli's datagen emits data for. The skew guard skips
/// thinning when the app's lockfile resolves a different minor (its data crates couldn't compile
/// our baked output).
const PINNED_ICU_PROVIDER_MINOR: &str = "2.2";

/// Point `ICU4X_SOURCE_CACHE` at the durable day cache. MUST run early in `main`, before any
/// threads spawn (`set_var` is unsafe under concurrency in edition 2024) — the update-check
/// thread starts later.
pub fn init_source_cache() {
    if std::env::var_os("ICU4X_SOURCE_CACHE").is_none()
        && let Some(dir) = day_icu_dir()
    {
        // SAFETY: called from the top of main before any thread is spawned.
        unsafe { std::env::set_var("ICU4X_SOURCE_CACHE", dir.join("src")) };
    }
}

/// Apply the thinned-data override to a cargo invocation (all four backend paths: desktop, iOS,
/// Android, OHOS). No-op — full compiled data — when thinning is unavailable.
pub fn apply(cmd: &mut Command, project: &Project) {
    if let Some(dir) = ensure_thinned_data(project) {
        cmd.env("ICU4X_DATA_DIR", &dir);
    }
}

/// `~/.day/icu`.
fn day_icu_dir() -> Option<PathBuf> {
    #[allow(deprecated)] // undeprecated in Rust 1.85+; correct on every day host platform
    std::env::home_dir().map(|h| h.join(".day/icu"))
}

/// The locale set to bake: `resource/locales/*` dir names (the app's declared set — the same one
/// `day lint` checks) ∪ the core-catalog locales. Sorted + deduped so the cache key is stable.
fn locale_set(project: &Project) -> Vec<String> {
    let mut set: Vec<String> = CORE_LOCALES.iter().map(|s| s.to_string()).collect();
    let dir = project.root.join("resource/locales");
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            if entry.path().is_dir()
                && let Some(name) = entry.file_name().to_str()
            {
                // `en-XA` is Day's pseudolocale — it formats via its base locale's data.
                let base = name.split("-u-").next().unwrap_or(name);
                if base != "en-XA" {
                    set.push(base.to_string());
                }
            }
        }
    }
    set.sort();
    set.dedup();
    set
}

/// The app lockfile's resolved `icu_provider` version, if any ("" = the app doesn't use icu).
fn lock_icu_provider_version(project: &Project) -> Option<String> {
    let lock = std::fs::read_to_string(project.root.join("Cargo.lock")).ok()?;
    let mut lines = lock.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim() == "name = \"icu_provider\""
            && let Some(next) = lines.peek()
            && let Some(v) = next.trim().strip_prefix("version = \"")
        {
            return Some(v.trim_end_matches('"').to_string());
        }
    }
    None
}

/// Ensure a baked, thinned data dir exists for this project's locale set. `None` ⇒ the build
/// proceeds with full compiled data (opt-out, offline without cache, skew, or any error).
pub fn ensure_thinned_data(project: &Project) -> Option<PathBuf> {
    // Explicit opt-out: embed the full all-locale data (useful when debugging locale issues).
    if std::env::var_os("DAY_ICU_FULL_DATA").is_some() {
        return None;
    }

    // Version-skew guard: baked output only compiles against the icu minor it was made for.
    if let Some(v) = lock_icu_provider_version(project)
        && !v.starts_with(PINNED_ICU_PROVIDER_MINOR)
    {
        status(
            "Warning",
            &format!(
                "icu_provider {v} in Cargo.lock doesn't match day's datagen \
                 ({PINNED_ICU_PROVIDER_MINOR}.x) — building with full locale data"
            ),
        );
        return None;
    }

    let locales = locale_set(project);
    let root = day_icu_dir()?;
    let key = cache_key(&locales);
    let dir = root.join("baked").join(&key);
    if dir.join("day-icu.json").is_file() {
        return Some(dir); // cache hit — fully offline
    }

    // First generation for this locale set. datagen may need to fetch its CLDR/ICU-export source
    // archives (one-time, cached durably in ~/.day/icu/src via ICU4X_SOURCE_CACHE).
    let cache_populated = root.join("src").is_dir();
    let offline = std::env::var_os("DAY_NO_ICU_FETCH").is_some()
        || std::env::var_os("DAY_NO_UPDATE_CHECK").is_some();
    if offline && !cache_populated {
        status(
            "Warning",
            "locale-data thinning skipped (DAY_NO_ICU_FETCH/DAY_NO_UPDATE_CHECK set and no \
             cached CLDR source) — the app embeds all-locale data",
        );
        return None;
    }
    if !cache_populated {
        status(
            "Fetching",
            "CLDR + ICU export data (one-time, ~100 MB, cached in ~/.day/icu/src; \
             DAY_NO_ICU_FETCH=1 skips — apps then embed all-locale data)",
        );
    }
    status(
        "Baking",
        &format!(
            "ICU locale data for {} (~/.day/icu/baked)",
            locales.join(", ")
        ),
    );

    match generate(&locales, &root, &dir) {
        Ok(()) => Some(dir),
        Err(e) => {
            status(
                "Warning",
                &format!("locale-data thinning failed ({e}) — building with full locale data"),
            );
            None
        }
    }
}

/// Content key: locale set + the datagen minor (a new icu minor must re-bake).
fn cache_key(locales: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(locales.join(","));
    h.update(":");
    h.update(PINNED_ICU_PROVIDER_MINOR);
    let out = h.finalize();
    out.iter().take(12).map(|b| format!("{b:02x}")).collect()
}

// The full stable data-marker list, generated from icu4x's own registry (the exact recipe the
// icu4x-datagen CLI uses). All markers are baked — dead-code elimination strips the unused ones
// from the app binary, so thinning stays a per-LOCALE concern only.
macro_rules! cb {
    ($($marker_ty:ty:$marker:ident,)+ #[unstable] $($emarker_ty:ty:$emarker:ident,)+) => {
        fn all_markers() -> Vec<icu_provider::DataMarkerInfo> {
            vec![$(<$marker_ty>::INFO,)+]
        }
    };
}
icu_provider_registry::registry!(cb);

/// Run datagen into `dir` (via a temp sibling + rename, so concurrent `day build`s never observe
/// a half-written cache entry).
fn generate(
    locales: &[String],
    root: &std::path::Path,
    dir: &std::path::Path,
) -> Result<(), String> {
    use icu_provider_export::baked_exporter::{self, BakedExporter};
    use icu_provider_export::prelude::*;
    use icu_provider_source::SourceDataProvider;

    let families: Vec<DataLocaleFamily> = locales
        .iter()
        .filter_map(|l| l.parse().ok())
        .map(DataLocaleFamily::with_descendants)
        .collect();
    if families.is_empty() {
        return Err("no parseable locales".into());
    }

    let tmp = root
        .join("baked")
        .join(format!(".tmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;

    let provider = SourceDataProvider::new();
    let fallbacker =
        LocaleFallbacker::try_new_unstable(&provider).map_err(|e| format!("fallbacker: {e}"))?;
    let exporter = {
        let mut options = baked_exporter::Options::default();
        options.overwrite = true;
        options.use_internal_fallback = true;
        BakedExporter::new(tmp.clone(), options).map_err(|e| format!("exporter: {e}"))?
    };
    ExportDriver::new(families, DeduplicationStrategy::Maximal.into(), fallbacker)
        .with_markers(all_markers())
        .export(&provider, exporter)
        .map_err(|e| format!("datagen: {e}"))?;

    std::fs::write(
        tmp.join("day-icu.json"),
        format!(
            "{{\"locales\":\"{}\",\"icu_provider\":\"{}\"}}\n",
            locales.join(","),
            PINNED_ICU_PROVIDER_MINOR
        ),
    )
    .map_err(|e| e.to_string())?;

    let _ = std::fs::remove_dir_all(dir);
    std::fs::rename(&tmp, dir).map_err(|e| format!("cache rename: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_stable_and_locale_sensitive() {
        let a = cache_key(&["en".into(), "fr".into()]);
        assert_eq!(a, cache_key(&["en".into(), "fr".into()]));
        assert_ne!(a, cache_key(&["en".into(), "zh".into()]));
        assert_eq!(a.len(), 24);
    }

    /// CORE_LOCALES must mirror the day-l10n catalog (repo-relative — skipped when day-cli is
    /// built from the registry where the sibling crate isn't present).
    #[test]
    fn core_locales_match_catalog() {
        let catalog = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../day-l10n/catalog");
        if !catalog.is_dir() {
            return;
        }
        let mut found: Vec<String> = std::fs::read_dir(catalog)
            .unwrap()
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()?
                    .strip_suffix(".ftl")
                    .map(str::to_string)
            })
            .collect();
        found.sort();
        let mut ours: Vec<String> = CORE_LOCALES.iter().map(|s| s.to_string()).collect();
        ours.sort();
        assert_eq!(ours, found, "CORE_LOCALES drifted from day-l10n/catalog");
    }
}
