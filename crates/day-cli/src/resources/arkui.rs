//! ArkUI (HarmonyOS) resource staging (§18.3).
//!
//! Both images and data go into `platform/ohos/entry/src/main/resources/rawfile/day/` (hvigor packages
//! rawfile uncompressed, and the OpenHarmony NDK can only reach `rawfile` — not `media` — from native
//! code). `day-arkui` sets an image node's src to `resource://RAWFILE/day/<name>.png` and its rawfile
//! opener mmaps `day/<name>` for random-access data.

use std::fs;

use super::{FontFile, ResourceSet, sanitize_ident};
use crate::meta::Project;

pub fn stage(project: &Project, set: &ResourceSet, fonts: &[FontFile]) -> Result<(), String> {
    let harmony = project.root.join("platform/ohos");
    if !harmony.exists() {
        return Ok(());
    }
    let dir = harmony.join("entry/src/main/resources/rawfile/day");
    // Regenerate fresh so removed resources don't linger in the packaged rawfile tree.
    let _ = fs::remove_dir_all(&dir);
    if set.images.is_empty() && set.data.is_empty() && fonts.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    // Fonts (§18.4): rawfile `day/fonts/<ident>.<ext>` plus a `day/fonts.json` manifest
    // ([{family, file}]) that the platform/ohos scaffold's EntryAbility feeds to ArkTS
    // `font.registerFont` before the native UI loads — NODE_FONT_FAMILY then resolves the
    // family by name.
    if !fonts.is_empty() {
        let fdir = dir.join("fonts");
        fs::create_dir_all(&fdir).map_err(|e| format!("mkdir {}: {e}", fdir.display()))?;
        let mut manifest = Vec::new();
        for f in fonts {
            let name = f.staged_name();
            let dest = fdir.join(&name);
            fs::copy(&f.path, &dest).map_err(|e| format!("stage {}: {e}", dest.display()))?;
            manifest.push(serde_json::json!({ "family": f.family, "file": name }));
        }
        let json = serde_json::to_string_pretty(&manifest).expect("font manifest");
        fs::write(dir.join("fonts.json"), json).map_err(|e| format!("stage fonts.json: {e}"))?;
    }
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
