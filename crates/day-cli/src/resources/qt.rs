//! Qt resource staging (§18.3) — native Qt Resource System packing.
//!
//! Generates a `.qrc` and compiles it with `rcc -binary` into `app.rcc` under `build/day/qt/`.
//! `day launch` points `DAY_QRESOURCE` at it; the day-qt shim registers it at startup
//! (`QResource::registerResource`) and then loads data via `QResource::data` (zero-copy from the
//! mmapped, uncompressed blob) and images via `QPixmap(":/day/images/<stem>")`. Data lives at
//! `/day/assets/<name>`, images at `/day/images/<stem>`.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::ResourceSet;
use crate::meta::Project;

/// Path of the compiled Qt resource blob (also read by `day launch`).
pub fn qresource_path(project: &Project) -> PathBuf {
    project.root.join("build/day/qt/app.rcc")
}

/// Locate `rcc` — it lives in Qt's libexec (Qt 6) or host-bins, or on PATH.
fn find_rcc() -> Option<PathBuf> {
    // On Windows the executable carries `.exe`, so the qmake-queried libexec/host-bins dir holds
    // `rcc.exe`; joining a bare "rcc" there fails `exists()` and Qt's icon would silently drop.
    let names: &[&str] = if cfg!(windows) {
        &["rcc.exe", "rcc"]
    } else {
        &["rcc"]
    };
    for qmake in ["qmake6", "qmake"] {
        for var in ["QT_INSTALL_LIBEXECS", "QT_HOST_BINS"] {
            if let Ok(out) = Command::new(qmake).args(["-query", var]).output()
                && out.status.success()
            {
                let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
                for name in names {
                    let p = Path::new(&dir).join(name);
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
    }
    // PATH fallback (Command resolves `rcc`/`rcc.exe` on PATH if present; else stage() degrades).
    Some(PathBuf::from("rcc"))
}

pub fn stage(project: &Project, set: &ResourceSet) -> Result<(), String> {
    if set.is_empty() {
        return Ok(());
    }
    let out = project.root.join("build/day/qt");
    std::fs::create_dir_all(&out).map_err(|e| format!("mkdir {}: {e}", out.display()))?;

    let mut files = String::new();
    for d in &set.data {
        files += &format!(
            "    <file alias=\"assets/{}\">{}</file>\n",
            d.name,
            d.path.display()
        );
    }
    for img in &set.images {
        // Alias images to images/<stem> (drop the extension) so the backend loads by name.
        files += &format!(
            "    <file alias=\"images/{}\">{}</file>\n",
            img.name,
            img.path.display()
        );
    }
    let qrc = format!(
        "<!DOCTYPE RCC><RCC version=\"1.0\">\n  <qresource prefix=\"/day\">\n{files}  </qresource>\n</RCC>\n"
    );
    let manifest = out.join("app.qrc");
    std::fs::write(&manifest, qrc).map_err(|e| format!("write {}: {e}", manifest.display()))?;

    let rcc = find_rcc().ok_or("could not locate Qt's rcc")?;
    let blob = qresource_path(project);
    // `-no-compress` stores entries uncompressed so QResource::data() is a zero-copy pointer.
    let status = Command::new(&rcc)
        .arg("-binary")
        .arg("-no-compress")
        .arg(&manifest)
        .arg("-o")
        .arg(&blob)
        .status()
        .map_err(|e| format!("rcc ({e})"))?;
    if !status.success() {
        return Err("rcc failed".into());
    }
    Ok(())
}
