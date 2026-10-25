// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! ArkUI (HarmonyOS) resource staging (§18.3).
//!
//! Both images and data go into `build/day/harmony/project/entry/src/main/resources/rawfile/day/` (hvigor packages
//! rawfile uncompressed, and the OpenHarmony NDK can only reach `rawfile`, not `media`, from native
//! code). `day-toolkit-arkui` sets an image node's src to `resource://RAWFILE/day/<name>.png` and its rawfile
//! opener mmaps `day/<name>` for random-access data.

use std::fs;

use super::{FontFile, ResourceSet, sanitize_ident};
use crate::meta::Project;

pub fn stage(project: &Project, set: &ResourceSet, fonts: &[FontFile]) -> Result<(), String> {
    let harmony = crate::ohos::staged_harmony_dir(project);
    if !harmony.exists() {
        return Ok(());
    }
    let dir = harmony.join("entry/src/main/resources/rawfile/day");
    // Regenerate fresh so removed resources don't linger in the packaged rawfile tree.
    let _ = fs::remove_dir_all(&dir);
    // Vector glyph SVGs (docs/vectors.md): staged beside the raster under the same stem.
    // ArkUI's Image renders SVG natively and alpha-preserving SourceIn compositing tints it, so
    // day-toolkit-arkui probes `day/<name>.svg` first and falls back to the png.
    let svgs: Vec<std::path::PathBuf> = super::vector_svg_dir(project)
        .read_dir()
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("svg"))
                .collect()
        })
        .unwrap_or_default();
    if set.images.is_empty() && set.data.is_empty() && fonts.is_empty() && svgs.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    // Fonts (§18.4): rawfile `day/fonts/<ident>.<ext>` plus a `day/fonts.json` manifest
    // ([{family, file}]) that the platform/harmony scaffold's EntryAbility feeds to ArkTS
    // `font.registerFont` before the native UI loads; NODE_FONT_FAMILY then resolves the
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
    // Images: day-toolkit-arkui references `resource://RAWFILE/day/<name>.png`, so normalize the file name to
    // `<name>.png` (ArkUI's Image decodes by content, not extension).
    for img in &set.images {
        let dest = dir.join(format!("{}.png", sanitize_ident(&img.name)));
        fs::copy(&img.path, &dest).map_err(|e| format!("stage {}: {e}", dest.display()))?;
    }
    for svg in &svgs {
        if let Some(name) = svg.file_name() {
            let dest = dir.join(name);
            let source =
                fs::read_to_string(svg).map_err(|e| format!("read {}: {e}", svg.display()))?;
            let source = sized_svg(&source)?;
            fs::write(&dest, source).map_err(|e| format!("stage {}: {e}", dest.display()))?;
        }
    }
    // Data: the rawfile opener reads `day/<name>`; `name` is the `/`-relative tree path
    // (§18.5), so recreate its parents under the rawfile dir.
    for d in &set.data {
        let dest = dir.join(&d.name);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        fs::copy(&d.path, &dest).map_err(|e| format!("stage {}: {e}", dest.display()))?;
    }
    Ok(())
}

/// ArkUI treats a viewBox-only SVG's intrinsic size as the Image node's destination size.
/// Its object-fit transform then becomes the identity, leaving the SVG's internal
/// preserveAspectRatio to contain the artwork even for Cover/Fill. Supply the intrinsic
/// size resolved by our SVG parser without rewriting paths, paint or viewBox semantics.
fn sized_svg(source: &str) -> Result<String, String> {
    let doc = day_vector::roxmltree::Document::parse(source).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    let size = day_vector::parse(source.as_bytes())?.size();
    let mut out = source.to_string();
    let mut edits = Vec::new();
    let mut missing = String::new();
    for (name, value) in [("width", size.width()), ("height", size.height())] {
        if let Some(attr) = root.attribute_node(name) {
            edits.push((attr.range_value(), value.to_string()));
        } else {
            missing.push_str(&format!(" {name}=\"{value}\""));
        }
    }
    // Insert immediately after the root element's qualified name, preserving its namespace.
    let start = root.range().start + 1;
    let end = start
        + source[start..]
            .find([' ', '\t', '\r', '\n', '/', '>'])
            .unwrap();
    edits.push((end..end, missing));
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (range, value) in edits {
        out.replace_range(range, &value);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::sized_svg;

    #[test]
    fn viewbox_only_svg_gets_intrinsic_dimensions_without_rewriting_art() {
        let art = r##"<path fill="none" stroke="#00ff00" d="M10 10H60"/>"##;
        let svg =
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 40">{art}</svg>"#);
        let out = sized_svg(&svg).unwrap();
        let doc = day_vector::roxmltree::Document::parse(&out).unwrap();
        let root = doc.root_element();
        assert_eq!(root.attribute("width"), Some("80"));
        assert_eq!(root.attribute("height"), Some("40"));
        assert_eq!(root.attribute("viewBox"), Some("0 0 80 40"));
        assert!(out.contains(art));
        assert_eq!(sized_svg(&out).unwrap(), out);
    }

    #[test]
    fn authored_dimensions_and_aspect_ratio_survive_staging() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="12" viewBox="0 0 80 40" preserveAspectRatio="xMinYMin slice"><rect width="80" height="40"/></svg>"#;
        assert_eq!(sized_svg(svg).unwrap(), svg);
    }
}
