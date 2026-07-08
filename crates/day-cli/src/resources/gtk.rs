//! GTK resource staging (§18.3) — native GResource packing.
//!
//! Generates a `.gresource.xml` and compiles it with `glib-compile-resources` into a binary
//! `app.gresource` blob under `build/day/gtk/`. `day launch` points `DAY_GRESOURCE` at it; day-gtk
//! registers it at startup (`gio::resources_register`) and then loads data via
//! `g_resources_lookup_data` (zero-copy from the mmapped blob) and images via
//! `gtk_picture_new_for_resource`. Data lives at `/day/assets/<name>`, images at `/day/images/<stem>`
//! (aliased without an extension — GdkTexture sniffs the content).

use std::fs;
use std::process::Command;

use super::ResourceSet;
use crate::meta::Project;

/// Path of the compiled GResource blob for a project (also read by `day launch`).
pub fn gresource_path(project: &Project) -> std::path::PathBuf {
    project.root.join("build/day/gtk/app.gresource")
}

pub fn stage(project: &Project, set: &ResourceSet) -> Result<(), String> {
    if set.is_empty() {
        return Ok(());
    }
    let out = project.root.join("build/day/gtk");
    fs::create_dir_all(&out).map_err(|e| format!("mkdir {}: {e}", out.display()))?;

    let mut files = String::new();
    for d in &set.data {
        // Data path under the project (assets/<name>) → resource /day/assets/<name>.
        let rel = d.path.strip_prefix(&project.root).unwrap_or(&d.path);
        files += &format!("    <file>{}</file>\n", rel.display());
    }
    for img in &set.images {
        // Alias images to /day/images/<stem> (drop the extension) so the backend loads by name.
        let rel = img.path.strip_prefix(&project.root).unwrap_or(&img.path);
        files += &format!(
            "    <file alias=\"images/{}\">{}</file>\n",
            img.name,
            rel.display()
        );
    }
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <gresources>\n  <gresource prefix=\"/day\">\n{files}  </gresource>\n</gresources>\n"
    );
    let manifest = out.join("app.gresource.xml");
    fs::write(&manifest, xml).map_err(|e| format!("write {}: {e}", manifest.display()))?;

    let blob = gresource_path(project);
    let status = Command::new("glib-compile-resources")
        .arg("--target")
        .arg(&blob)
        .arg("--sourcedir")
        .arg(&project.root)
        .arg(&manifest)
        .status()
        .map_err(|e| format!("glib-compile-resources ({e}); install glib"))?;
    if !status.success() {
        return Err("glib-compile-resources failed".into());
    }
    Ok(())
}
