// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Android resource staging (§18.3).
//!
//! Images → `build/day/android/res/drawable*/<name>.<ext>` (density bucket from any `@Nx` suffix) so
//! aapt2 crunches them and assigns an `R.drawable` id; `DayBridge.makeImage` resolves the name via
//! `Resources.getIdentifier(name,"drawable",pkg)`. The gradle scaffold registers this tree as a
//! `res.srcDir`. Data (`assets/`) is already the APK `assets/` root (the scaffold's `assets.srcDir`)
//! and is read at runtime through the NDK `AAssetManager`; the scaffold marks it `noCompress` so the
//! bytes are stored uncompressed for a zero-copy `AAsset_getBuffer`.

use std::fs;

use super::{FontFile, ResourceSet, VectorAsset, sanitize_ident};
use crate::meta::Project;
use crate::ops::status;

pub fn stage(
    project: &Project,
    set: &ResourceSet,
    fonts: &[FontFile],
    vectors: &[VectorAsset],
) -> Result<(), String> {
    if set.images.is_empty() && fonts.is_empty() && vectors.is_empty() {
        return Ok(());
    }
    let res = project.root.join("build/day/android/res");
    // Regenerate the tree each build so removed images don't linger.
    let _ = fs::remove_dir_all(&res);
    // Fonts (§18.4) → res/font/<ident>.<ext>: aapt2 assigns an `R.font` id, and
    // `DayBridge.bundledFont` re-derives <ident> from the requested family name at runtime.
    if !fonts.is_empty() {
        let dir = res.join("font");
        fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        for f in fonts {
            let dest = dir.join(f.staged_name());
            fs::copy(&f.path, &dest).map_err(|e| format!("stage {}: {e}", dest.display()))?;
        }
    }
    // Vectors (docs/vectors.md): the REAL vector form — a VectorDrawable in `drawable/`, which
    // `Resources.getIdentifier(name, "drawable", …)` resolves exactly like a PNG, resolution-
    // independent and tintable. Art outside VD's subset falls back to the raster cache at
    // xxxhdpi, LOUDLY. Either way the name resolves, so the generic image loop below must skip
    // the raster-cache synthetics for names handled here (a density-qualified PNG would shadow
    // the VD on that density).
    let mut vector_names = std::collections::BTreeSet::new();
    if !vectors.is_empty() {
        let vd_dir = res.join("drawable");
        let px_dir = res.join("drawable-xxxhdpi");
        for v in vectors {
            let ident = sanitize_ident(&v.name);
            vector_names.insert(v.name.clone());
            let tree = day_vector::parse(v.glyph.as_bytes())
                .map_err(|e| format!("vector {}: {e}", v.name))?;
            match day_vector::to_vector_drawable(&tree) {
                Ok(xml) => {
                    fs::create_dir_all(&vd_dir).map_err(|e| format!("mkdir: {e}"))?;
                    fs::write(vd_dir.join(format!("{ident}.xml")), xml)
                        .map_err(|e| format!("stage vector {}: {e}", v.name))?;
                }
                Err(why) => {
                    status(
                        "Packing",
                        &format!(
                            "vector {} → raster on android ({why} is outside the VectorDrawable subset)",
                            v.name
                        ),
                    );
                    let cached = super::vector_raster_dir(project).join(format!("{}.png", v.name));
                    fs::create_dir_all(&px_dir).map_err(|e| format!("mkdir: {e}"))?;
                    fs::copy(&cached, px_dir.join(format!("{ident}.png")))
                        .map_err(|e| format!("stage vector {}: {e}", v.name))?;
                }
            }
        }
    }
    let raster_cache = super::vector_raster_dir(project);
    for img in &set.images {
        if img.path.starts_with(&raster_cache) && vector_names.contains(&img.name) {
            continue; // staged above as a VectorDrawable (or its explicit fallback)
        }
        let bucket = match img.scale {
            2 => "drawable-xhdpi",
            3 => "drawable-xxhdpi",
            4 => "drawable-xxxhdpi",
            _ => "drawable",
        };
        let dir = res.join(bucket);
        fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        let ext = img
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_ascii_lowercase();
        let dest = dir.join(format!("{}.{}", sanitize_ident(&img.name), ext));
        fs::copy(&img.path, &dest).map_err(|e| format!("stage {}: {e}", dest.display()))?;
    }
    Ok(())
}
