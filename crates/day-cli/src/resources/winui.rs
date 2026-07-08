//! WinUI resource staging (§18.3).
//!
//! Day builds WinUI apps **unpackaged** (no MSIX), where the native + recommended path is loose
//! files loaded through WIC: day-winui resolves images via `resolve_image_file` → `BitmapImage`
//! (`Image.Source`) and data via the default mmap file opener. MRT / `.pri` (`ms-appx:///`,
//! automatic DPI selection) only applies to MSIX-packaged apps, so there is nothing to compile into a
//! resource index here. At `day launch` the loader points `DAY_IMAGE_ROOT`/`DAY_ASSET_ROOT` at the
//! project, so dev runs need no pre-build staging; a portable/packaged layout copies `images/` +
//! `assets/` next to the `.exe` (a `day pack` concern). Embedding as Win32 `.rc` RCDATA for a
//! single-file `.exe` is a possible future option.
use super::ResourceSet;
use crate::meta::Project;

pub fn stage(_project: &Project, _set: &ResourceSet) -> Result<(), String> {
    Ok(())
}
