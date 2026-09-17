// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Explicit leases for persistent macOS file access; no process-global URL leak.
use crate::FileUrl;
use std::io;

/// A resolved file and the lifetime of its native access grant. Borrow the file while doing
/// I/O; a cloned path alone does not retain access after this guard is dropped.
pub struct FileAccess {
    file: FileUrl,
    stale: bool,
    #[cfg(target_os = "macos")]
    _access: apple::Access,
}
impl FileAccess {
    pub fn file(&self) -> &FileUrl {
        &self.file
    }
    /// Renew a stale bookmark with `file().bookmark(...)` while this guard is alive and
    /// replace the stored bytes. A moved file may resolve to a different path.
    pub fn was_stale(&self) -> bool {
        self.stale
    }
}

#[cfg(target_os = "macos")]
mod apple {
    use super::*;
    use objc2::{rc::Retained, runtime::Bool};
    use objc2_foundation::{
        NSData, NSString, NSURL, NSURLBookmarkCreationOptions as Creation,
        NSURLBookmarkResolutionOptions as Resolution,
    };

    pub(super) struct Access {
        url: Retained<NSURL>,
        active: bool,
    }
    impl Drop for Access {
        fn drop(&mut self) {
            if self.active {
                unsafe { self.url.stopAccessingSecurityScopedResource() };
            }
        }
    }
    pub fn bookmark(file: &FileUrl, read_only: bool) -> io::Result<Vec<u8>> {
        let path = file.local_path().ok_or(io::ErrorKind::InvalidInput)?;
        if !path.is_absolute() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let path = path.to_str().ok_or(io::ErrorKind::InvalidInput)?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let mut options = Creation::WithSecurityScope;
        if read_only {
            options |= Creation::SecurityScopeAllowOnlyReadAccess;
        }
        let data = url
            .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                options, None, None,
            )
            .map_err(|e| io::Error::other(e.to_string()))?;
        Ok(data.to_vec())
    }
    pub fn resolve(bytes: &[u8]) -> io::Result<FileAccess> {
        // Bookmarks are small metadata, not file contents. Reject accidental/untrusted bulk data.
        if bytes.is_empty() || bytes.len() > 1024 * 1024 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let mut stale = Bool::NO;
        let url = unsafe {
            NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                &NSData::with_bytes(bytes),
                Resolution::WithSecurityScope | Resolution::WithoutUI,
                None,
                &mut stale,
            )
        }
        .map_err(|e| io::Error::other(e.to_string()))?;
        if !url.isFileURL() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let active = unsafe { url.startAccessingSecurityScopedResource() };
        let access = Access { url, active };
        let path = access
            .url
            .path()
            .ok_or(io::ErrorKind::InvalidData)?
            .to_string();
        Ok(FileAccess {
            file: FileUrl::new(path),
            stale: stale.as_bool(),
            _access: access,
        })
    }
}
#[cfg(target_os = "macos")]
pub(crate) use apple::{bookmark, resolve};

#[cfg(not(target_os = "macos"))]
pub(crate) fn bookmark(_: &FileUrl, _: bool) -> io::Result<Vec<u8>> {
    Err(io::ErrorKind::Unsupported.into())
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn resolve(_: &[u8]) -> io::Result<FileAccess> {
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_urls_decode_unicode_and_spaces_and_reject_remote_authorities() {
        assert_eq!(
            FileUrl::new("file:///tmp/a%20b%23%E2%98%80.txt")
                .local_path()
                .unwrap(),
            std::path::PathBuf::from("/tmp/a b#☀.txt")
        );
        assert!(
            FileUrl::new("file://remote/tmp/a.txt")
                .local_path()
                .is_none()
        );
        assert!(FileUrl::new("file:///tmp/%ZZ").local_path().is_none());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn native_bookmark_round_trip_preserves_file_contents() {
        let dir = std::env::temp_dir().join(format!(
            "day-bookmark-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("file café #1.txt");
        std::fs::write(&path, b"bookmark contents").unwrap();
        let file = FileUrl::new(path.to_str().unwrap());
        let bytes = file.bookmark(true).unwrap();
        {
            let access = FileUrl::resolve_bookmark(&bytes).unwrap();
            assert_eq!(access.file().read().unwrap(), b"bookmark contents");
        }
        // Repeated resolution exercises balanced scoped access lifetimes.
        let access = FileUrl::resolve_bookmark(&bytes).unwrap();
        assert_eq!(access.file().read().unwrap(), b"bookmark contents");
        drop(access);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_bookmarks_do_not_create_access() {
        assert!(FileUrl::resolve_bookmark(&[]).is_err());
        assert!(FileUrl::resolve_bookmark(b"not a bookmark").is_err());
    }
}
