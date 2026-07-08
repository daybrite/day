//! Apple (iOS/UIKit) resource staging (§18.3).
//!
//! Images → a generated `Media.xcassets` inside the `DayPieces` SwiftPM package (the local package
//! the `.xcodeproj` already links), declared `resources: [.process(...)]`. xcodebuild's `actool`
//! compiles the catalog into an optimized, deduplicated `Assets.car` in `DayPieces_DayPieces.bundle`;
//! `day-uikit` loads images by name from that bundle. This is invoked from `pieces::write_ios_pieces`
//! (which owns the DayPieces package), not the `stage()` dispatcher.
//!
//! Data (`assets/`) is copied into the app bundle by the xcode-backend copy phase and read back
//! through the default mmap file opener (a plain bundle file — the Apple native path). macOS/AppKit
//! is a cargo binary (no xcodebuild/actool), so it stays on the bundle-file path.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::ResourceFile;

const CATALOG_ROOT: &str = "{ \"info\" : { \"author\" : \"day\", \"version\" : 1 } }\n";

/// Generate `Media.xcassets` under `sources_dir` — one `<name>.imageset` per image (grouping `@Nx`
/// scale variants), each with a `Contents.json`. Returns `true` if any imageset was written (so the
/// caller adds the `.process` resource to the target). SwiftPM/xcodebuild then runs `actool`.
pub fn write_media_xcassets(sources_dir: &Path, images: &[ResourceFile]) -> Result<bool, String> {
    if images.is_empty() {
        return Ok(false);
    }
    let catalog = sources_dir.join("Media.xcassets");
    // Regenerate fresh so a removed image never lingers in the catalog.
    let _ = fs::remove_dir_all(&catalog);
    fs::create_dir_all(&catalog).map_err(|e| e.to_string())?;
    fs::write(catalog.join("Contents.json"), CATALOG_ROOT).map_err(|e| e.to_string())?;

    // Group scale variants by image name.
    let mut by_name: BTreeMap<&str, Vec<&ResourceFile>> = BTreeMap::new();
    for img in images {
        by_name.entry(img.name.as_str()).or_default().push(img);
    }
    for (name, mut variants) in by_name {
        variants.sort_by_key(|v| v.scale);
        let imageset = catalog.join(format!("{name}.imageset"));
        fs::create_dir_all(&imageset).map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        for v in &variants {
            let ext = v.path.extension().and_then(|e| e.to_str()).unwrap_or("png");
            let fname = if v.scale > 1 {
                format!("{name}@{}x.{ext}", v.scale)
            } else {
                format!("{name}.{ext}")
            };
            fs::copy(&v.path, imageset.join(&fname)).map_err(|e| e.to_string())?;
            entries.push(format!(
                "    {{ \"idiom\" : \"universal\", \"filename\" : \"{fname}\", \"scale\" : \"{}x\" }}",
                v.scale
            ));
        }
        let contents = format!(
            "{{\n  \"images\" : [\n{}\n  ],\n  \"info\" : {{ \"author\" : \"day\", \"version\" : 1 }}\n}}\n",
            entries.join(",\n")
        );
        fs::write(imageset.join("Contents.json"), contents).map_err(|e| e.to_string())?;
    }
    Ok(true)
}
