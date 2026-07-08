//! Bundled resources — efficient random read-only access to app-declared data, backed by each
//! toolkit's native resource mechanism (DESIGN §18.3).
//!
//! `day build` stages the files declared under a project's `assets/` (and `images/`) into each
//! platform's native resource store, **uncompressed where possible** so the runtime can hand back a
//! zero-copy view. At runtime `resource("name")` returns a [`Resource`] whose bytes are borrowed
//! directly from that store — an mmap of a bundle file on Apple, the NDK `AAssetManager` buffer on
//! Android, `g_resources_lookup_data` on GTK, `QResource::data` on Qt, the rawfile fd on ArkUI.
//!
//! The active backend registers its opener once via [`set_resource_opener`]; if none is registered
//! (desktop dev, host tests, and the Apple backends, whose native mechanism *is* a bundle file) the
//! [default opener](default_open) mmaps the file resolved from `DAY_ASSET_ROOT` or the app bundle.

use std::any::Any;
use std::path::PathBuf;
use std::sync::OnceLock;

/// A handle to a bundled resource's bytes with efficient random read-only access.
///
/// The bytes stay valid for the lifetime of the `Resource`: it owns a guard (an mmap, a native
/// `GBytes`/`QResource`/`AAsset` handle, or an owned buffer) that keeps the backing store alive.
/// `Resource` is neither `Send` nor `Sync` — like the rest of the day runtime it is used on the
/// main/UI thread.
pub struct Resource {
    ptr: *const u8,
    len: usize,
    /// Keeps the backing store alive while `ptr`/`len` are borrowed from it.
    _guard: Box<dyn Any>,
}

impl Resource {
    /// Build a `Resource` from a raw byte view plus the guard that owns it. Backends call this from
    /// their opener after obtaining a zero-copy pointer from the native resource API.
    ///
    /// # Safety
    /// `ptr` must point to `len` valid, immutable bytes that remain valid for as long as `guard` is
    /// alive, and `guard` must own (and keep alive) that backing store.
    pub unsafe fn from_raw(ptr: *const u8, len: usize, guard: Box<dyn Any>) -> Resource {
        Resource {
            ptr,
            len,
            _guard: guard,
        }
    }

    /// Build a `Resource` from owned bytes — the copy fallback for backends that cannot expose a
    /// stable pointer into their store (or when a native API only offers a read-into-buffer call).
    pub fn from_vec(bytes: Vec<u8>) -> Resource {
        let boxed: Box<[u8]> = bytes.into_boxed_slice();
        let ptr = boxed.as_ptr();
        let len = boxed.len();
        // Moving the box moves the (fat) pointer, not the heap allocation, so `ptr` stays valid.
        Resource {
            ptr,
            len,
            _guard: Box::new(boxed),
        }
    }

    /// Total byte length of the resource.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the resource is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// A zero-copy view of the full contents. Backed directly by the native store (no allocation,
    /// no copy). Wrap in [`std::io::Cursor`] for `Read`/`Seek`-style access.
    pub fn as_slice(&self) -> &[u8] {
        // Safety: `ptr`/`len` describe a valid immutable region kept alive by `_guard`.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// Random-access read: copy up to `buf.len()` bytes starting at `offset` into `buf`, returning
    /// the number of bytes copied (0 if `offset >= len`). A direct `memcpy` from the backing store,
    /// no allocation — the efficient primitive for seeking around a large embedded blob.
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let data = self.as_slice();
        if offset >= data.len() {
            return 0;
        }
        let n = buf.len().min(data.len() - offset);
        buf[..n].copy_from_slice(&data[offset..offset + n]);
        n
    }

    /// Copy the full contents into a freshly allocated `Vec`.
    pub fn to_vec(&self) -> Vec<u8> {
        self.as_slice().to_vec()
    }
}

/// A backend's resource opener: maps a declared name (e.g. `"stations.json"`) to its bytes, or
/// `None` if there is no such resource.
pub type ResourceOpener = fn(&str) -> Option<Resource>;

static OPENER: OnceLock<ResourceOpener> = OnceLock::new();

/// Register the active backend's resource opener (Android `AAssetManager`, GTK `GResource`, Qt
/// `QResource`, ArkUI rawfile, …). Called once during backend init. A second call is ignored, so a
/// backend that shares the default file opener need not call this at all.
pub fn set_resource_opener(opener: ResourceOpener) {
    let _ = OPENER.set(opener);
}

/// Open a bundled resource by name for efficient random read-only access.
///
/// Returns `None` if the backend has no resource with that name. Names match the file names staged
/// by `day build` from the project's `assets/` directory (e.g. `resource("stations.json")`).
pub fn resource(name: &str) -> Option<Resource> {
    match OPENER.get() {
        Some(open) => open(name),
        None => default_open(name),
    }
}

/// The default opener: mmap the file resolved from `DAY_ASSET_ROOT` (dev runs / `day launch`) or the
/// app bundle's `Resources/assets`. This is the Apple backends' native path (a plain bundle file)
/// and the desktop/host-test path.
fn default_open(name: &str) -> Option<Resource> {
    let path = resolve_resource_path(name)?;
    let file = std::fs::File::open(&path).ok()?;
    // Safety: we open read-only and never mutate the mapping; the `Mmap` guard keeps it alive.
    let mmap = unsafe { memmap2::Mmap::map(&file).ok()? };
    let ptr = mmap.as_ptr();
    let len = mmap.len();
    Some(unsafe { Resource::from_raw(ptr, len, Box::new(mmap)) })
}

/// Resolve a data-resource name to an on-disk path: `DAY_ASSET_ROOT` first (dev / CLI launch), then
/// bundle-relative locations next to the executable (macOS `.app`, Linux `share`).
fn resolve_resource_path(name: &str) -> Option<PathBuf> {
    if let Ok(root) = std::env::var("DAY_ASSET_ROOT") {
        let p = PathBuf::from(root).join(name);
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        for rel in ["../Resources/assets", "Resources/assets", "assets"] {
            let p = dir.join(rel).join(name);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

/// Image file extensions probed when an `image("name")` reference omits an extension.
const IMAGE_EXTS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "bmp", "webp", "pdf", "svg"];

/// Resolve an image name to an on-disk file, for the file-loading backends (AppKit/GTK/Qt, and the
/// desktop/dev path). Probes `DAY_IMAGE_ROOT` (the project's `images/` under `day launch`) first,
/// then `DAY_ASSET_ROOT` and the bundle, trying the bare name and, if it has no extension, each
/// known image extension. Native-pipeline backends (iOS `Assets.car`, Android `R`, …) resolve by
/// name through their own store and do not use this.
pub fn resolve_image_file(name: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for var in ["DAY_IMAGE_ROOT", "DAY_ASSET_ROOT"] {
        if let Ok(v) = std::env::var(var) {
            roots.push(PathBuf::from(v));
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        for rel in [
            "../Resources/images",
            "../Resources/assets",
            "images",
            "assets",
        ] {
            roots.push(dir.join(rel));
        }
    }
    let has_ext = std::path::Path::new(name).extension().is_some();
    for root in roots {
        let exact = root.join(name);
        if exact.is_file() {
            return Some(exact);
        }
        if !has_ext {
            for ext in IMAGE_EXTS {
                let p = root.join(format!("{name}.{ext}"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test owns the process env (set_var races across threads), covering both the default
    // data opener and the image-file resolver sequentially.
    #[test]
    fn env_backed_resource_access() {
        let dir = std::env::temp_dir().join(format!("day-res-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let payload: Vec<u8> = (0u8..=255).cycle().take(1000).collect();
        std::fs::write(dir.join("blob.bin"), &payload).unwrap();
        std::fs::write(dir.join("logo.png"), b"x").unwrap();
        // SAFETY: this is the only test that touches the env.
        unsafe {
            std::env::set_var("DAY_ASSET_ROOT", &dir);
            std::env::set_var("DAY_IMAGE_ROOT", &dir);
        }

        // default data opener: whole view + random access + clamping + miss.
        let res = resource("blob.bin").expect("resource present");
        assert_eq!(res.len(), 1000);
        assert!(!res.is_empty());
        assert_eq!(res.as_slice(), &payload[..]);
        let mut buf = [0u8; 16];
        assert_eq!(res.read_at(500, &mut buf), 16);
        assert_eq!(&buf, &payload[500..516]);
        assert_eq!(res.read_at(995, &mut buf), 5);
        assert_eq!(&buf[..5], &payload[995..1000]);
        assert_eq!(res.read_at(2000, &mut buf), 0);
        assert!(resource("does-not-exist.bin").is_none());

        // image-file resolver: extension inference + exact + miss.
        assert_eq!(resolve_image_file("logo"), Some(dir.join("logo.png")));
        assert_eq!(resolve_image_file("logo.png"), Some(dir.join("logo.png")));
        assert_eq!(resolve_image_file("missing"), None);

        // SAFETY: single test owning the env.
        unsafe {
            std::env::remove_var("DAY_ASSET_ROOT");
            std::env::remove_var("DAY_IMAGE_ROOT");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn from_vec_view_is_stable() {
        let r = Resource::from_vec(vec![1, 2, 3, 4]);
        assert_eq!(r.as_slice(), &[1, 2, 3, 4]);
        assert_eq!(r.to_vec(), vec![1, 2, 3, 4]);
        let mut b = [0u8; 2];
        assert_eq!(r.read_at(2, &mut b), 2);
        assert_eq!(b, [3, 4]);
    }
}
