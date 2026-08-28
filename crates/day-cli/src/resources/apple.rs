// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

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
//! stays on the bundle-file path too: the `platform/macos/` Xcode host stages images through the
//! `day xcode-backend stage-resources` script phase into `Contents/Resources` rather than through
//! an asset catalog, so no `actool` runs for them there either (the appicon is the exception).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::ResourceFile;

const CATALOG_ROOT: &str = "{ \"info\" : { \"author\" : \"day\", \"version\" : 1 } }\n";

/// Generate `Media.xcassets` under `sources_dir` — one `<name>.imageset` per image (grouping `@Nx`
/// scale variants), each with a `Contents.json`. Returns `true` if any imageset was written (so the
/// caller adds the `.process` resource to the target). SwiftPM/xcodebuild then runs `actool`.
/// `vectors` are `(name, glyph-svg path)` pairs from `resource/vectors/` (docs/vectors.md): each
/// becomes an SVG imageset with `"preserves-vector-representation": true` (the Xcode 12+ vector
/// asset), so `UIImage(named:)` renders the outline at display size instead of resampling a
/// bitmap — the raster-cache PNG of the same name is excluded here in its favor.
pub fn write_media_xcassets(
    sources_dir: &Path,
    images: &[ResourceFile],
    vectors: &[(String, std::path::PathBuf)],
) -> Result<bool, String> {
    if images.is_empty() && vectors.is_empty() {
        return Ok(false);
    }
    let catalog = sources_dir.join("Media.xcassets");
    fs::create_dir_all(&catalog).map_err(|e| e.to_string())?;
    // Touch only what changed and prune the rest: a removed image must not linger in the catalog,
    // but rewriting it wholesale restamps every file and re-runs `actool` on every build.
    let mut expected: Vec<std::path::PathBuf> = Vec::new();
    let root_contents = catalog.join("Contents.json");
    crate::pieces::write_if_changed(&root_contents, CATALOG_ROOT)?;
    expected.push(root_contents);

    // Group scale variants by image name — skipping the raster twins of names shipped below as
    // preserve-vector SVG imagesets (same name, two imagesets would collide in the catalog).
    let vector_names: std::collections::BTreeSet<&str> =
        vectors.iter().map(|(n, _)| n.as_str()).collect();
    let mut by_name: BTreeMap<&str, Vec<&ResourceFile>> = BTreeMap::new();
    for img in images {
        if vector_names.contains(img.name.as_str()) {
            continue;
        }
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
            let staged = imageset.join(&fname);
            crate::pieces::copy_if_changed(&v.path, &staged)?;
            expected.push(staged);
            entries.push(format!(
                "    {{ \"idiom\" : \"universal\", \"filename\" : \"{fname}\", \"scale\" : \"{}x\" }}",
                v.scale
            ));
        }
        let contents = format!(
            "{{\n  \"images\" : [\n{}\n  ],\n  \"info\" : {{ \"author\" : \"day\", \"version\" : 1 }}\n}}\n",
            entries.join(",\n")
        );
        let contents_path = imageset.join("Contents.json");
        crate::pieces::write_if_changed(&contents_path, &contents)?;
        expected.push(contents_path);
    }
    for (name, svg) in vectors {
        let imageset = catalog.join(format!("{name}.imageset"));
        fs::create_dir_all(&imageset).map_err(|e| e.to_string())?;
        let staged = imageset.join(format!("{name}.svg"));
        crate::pieces::copy_if_changed(svg, &staged)?;
        expected.push(staged);
        let contents = format!(
            "{{\n  \"images\" : [\n    {{ \"idiom\" : \"universal\", \"filename\" : \"{name}.svg\" }}\n  ],\n  \"info\" : {{ \"author\" : \"day\", \"version\" : 1 }},\n  \"properties\" : {{ \"preserves-vector-representation\" : true }}\n}}\n"
        );
        let contents_path = imageset.join("Contents.json");
        crate::pieces::write_if_changed(&contents_path, &contents)?;
        expected.push(contents_path);
    }
    crate::pieces::prune_except(&catalog, &expected.into_iter().collect());
    Ok(true)
}
