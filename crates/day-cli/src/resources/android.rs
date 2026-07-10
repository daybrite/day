//! Android resource staging (§18.3).
//!
//! Images → `build/day/android/res/drawable*/<name>.<ext>` (density bucket from any `@Nx` suffix) so
//! aapt2 crunches them and assigns an `R.drawable` id; `DayBridge.makeImage` resolves the name via
//! `Resources.getIdentifier(name,"drawable",pkg)`. The gradle scaffold registers this tree as a
//! `res.srcDir`. Data (`assets/`) is already the APK `assets/` root (the scaffold's `assets.srcDir`)
//! and is read at runtime through the NDK `AAssetManager`; the scaffold marks it `noCompress` so the
//! bytes are stored uncompressed for a zero-copy `AAsset_getBuffer`.

use std::fs;

use super::{FontFile, ResourceSet, sanitize_ident};
use crate::meta::Project;

pub fn stage(project: &Project, set: &ResourceSet, fonts: &[FontFile]) -> Result<(), String> {
    if set.images.is_empty() && fonts.is_empty() {
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
    for img in &set.images {
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
