//! ArkUI (HarmonyOS) resource staging (§18.3).
//!
//! Both images and data go into `harmony/entry/src/main/resources/rawfile/day/` (hvigor packages
//! rawfile uncompressed, and the OpenHarmony NDK can only reach `rawfile` — not `media` — from native
//! code). `day-arkui` sets an image node's src to `resource://RAWFILE/day/<name>.png` and its rawfile
//! opener mmaps `day/<name>` for random-access data.

use std::fs;

use super::{ResourceSet, sanitize_ident};
use crate::meta::Project;

pub fn stage(project: &Project, set: &ResourceSet) -> Result<(), String> {
    let harmony = project.root.join("harmony");
    if !harmony.exists() {
        return Ok(());
    }
    let dir = harmony.join("entry/src/main/resources/rawfile/day");
    // Regenerate fresh so removed resources don't linger in the packaged rawfile tree.
    let _ = fs::remove_dir_all(&dir);
    if set.images.is_empty() && set.data.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    // Images: day-arkui references `resource://RAWFILE/day/<name>.png`, so normalize the file name to
    // `<name>.png` (ArkUI's Image decodes by content, not extension).
    for img in &set.images {
        let dest = dir.join(format!("{}.png", sanitize_ident(&img.name)));
        fs::copy(&img.path, &dest).map_err(|e| format!("stage {}: {e}", dest.display()))?;
    }
    // Data: the rawfile opener reads `day/<name>`.
    for d in &set.data {
        let dest = dir.join(&d.name);
        fs::copy(&d.path, &dest).map_err(|e| format!("stage {}: {e}", dest.display()))?;
    }
    Ok(())
}
