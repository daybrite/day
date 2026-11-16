// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use std::fmt::Write;
use std::path::Path;

// All embedded includes are generated here, never hand-maintained in application code.
pub(super) fn generate(resources: &Path) -> Result<String, String> {
    let mut code = String::new();
    // Raw image bytes are useful to decoders on every backend. Keep this separate from
    // ImageName, which denotes the platform's processed image, not its original encoding.
    code.push_str("/// Original image encodings, for byte-oriented decoders.\n#[allow(dead_code, non_upper_case_globals)]\npub mod image_bytes {\n");
    for image in super::plan_images(&resources.join("images"))? {
        let symbol = super::ident_token(&image.symbol);
        let source = std::path::Path::new(&image.source);
        let file = resources
            .join("images")
            .join(source.file_name().ok_or("image has no file name")?);
        let file = file.to_str().ok_or("non-UTF8 image path")?;
        writeln!(
            code,
            "pub const {symbol}: &[u8] = include_bytes!({file:?});"
        )
        .unwrap();
    }
    code.push_str("}\n");
    println!("cargo:rerun-if-env-changed=DAY_WEB_RESOURCE_DIR");
    println!("cargo:rerun-if-env-changed=DAY_WEB_APP_PACKAGE");
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("wasm32")
        || std::env::var("DAY_WEB_APP_PACKAGE").ok() != std::env::var("CARGO_PKG_NAME").ok()
    {
        return Ok(code);
    }
    let dir = std::env::var("DAY_WEB_RESOURCE_DIR").map_err(|e| e.to_string())?;
    let root = Path::new(&dir);
    // Track additions/removals as well as bytes in files already included in the table.
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    collect(root, root, &mut files)?;
    files.sort();
    code.push_str(&render_embedded(&files)?);
    Ok(code)
}

fn render_embedded(files: &[(String, std::path::PathBuf)]) -> Result<String, String> {
    let mut code = String::new();
    code.push_str(
        r#"#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn day_dom_resources_init() {
 day::web::install_resources(&[
"#,
    );
    for (name, file) in files {
        println!("cargo:rerun-if-changed={}", file.display());
        let file = file.to_str().ok_or("non-UTF8 resource path")?;
        writeln!(code, "({name:?}, include_bytes!({file:?})),").unwrap();
    }
    code.push_str("]);\n}\n");
    Ok(code)
}

fn collect(
    root: &Path,
    dir: &Path,
    files: &mut Vec<(String, std::path::PathBuf)>,
) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect(root, &path, files)?;
        } else {
            let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
            files.push((
                format!("assets/{}", relative.to_string_lossy().replace('\\', "/")),
                path,
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{collect, render_embedded};
    use std::path::PathBuf;

    #[test]
    fn bootstrap_includes_literal_paths_and_an_empty_bundle() {
        let path = PathBuf::from("/fixture/quoted\"name.bin");
        let generated = render_embedded(&[("assets/data/nested/name.bin".into(), path)]).unwrap();
        assert!(generated.contains(r#"include_bytes!("/fixture/quoted\"name.bin")"#));
        assert!(generated.contains("assets/data/nested/name.bin"));
        assert!(
            render_embedded(&[])
                .unwrap()
                .contains("day::web::install_resources(&[")
        );
    }

    #[test]
    fn staging_collection_keeps_nested_and_contributed_names() {
        let root =
            std::env::temp_dir().join(format!("day-embedded-resources-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("data/day-piece-fixture/nested")).unwrap();
        std::fs::create_dir_all(root.join("fonts")).unwrap();
        std::fs::write(root.join("data/day-piece-fixture/nested/empty.bin"), []).unwrap();
        std::fs::write(root.join("fonts/fixture.ttf"), b"synthetic font fixture").unwrap();
        let mut files = Vec::new();
        collect(&root, &root, &mut files).unwrap();
        files.sort();
        assert_eq!(
            files
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            [
                "assets/data/day-piece-fixture/nested/empty.bin",
                "assets/fonts/fixture.ttf"
            ]
        );
        std::fs::remove_file(root.join("fonts/fixture.ttf")).unwrap();
        let mut next = Vec::new();
        collect(&root, &root, &mut next).unwrap();
        assert_eq!(next.len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
