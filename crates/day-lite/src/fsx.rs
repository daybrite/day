//! The sandboxed per-app filesystem (docs/lite.md §7): an OPFS-shaped surface rooted at the
//! app's `fs/` directory. Path validation is defense-in-depth: names may not be absolute,
//! contain `..`, backslashes, or NULs, and every resolved path must stay under the root.
//! Errors use OPFS DOMException names (`NotFoundError`, `TypeMismatchError`,
//! `InvalidModificationError`, `SecurityError`) so the JS contract is the familiar one.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsError {
    /// A DOMException-style name (`NotFoundError`, `SecurityError`, …).
    pub name: &'static str,
    pub detail: String,
}

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.name, self.detail)
    }
}

impl std::error::Error for FsError {}

fn fail(name: &'static str, detail: impl Into<String>) -> FsError {
    FsError {
        name,
        detail: detail.into(),
    }
}

/// The app's sandbox. Cheap to clone; all paths below are RELATIVE to the root.
#[derive(Clone, Debug)]
pub struct Sandbox {
    root: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
}

impl Sandbox {
    pub fn at(root: impl Into<PathBuf>) -> Sandbox {
        Sandbox { root: root.into() }
    }

    /// Validate + resolve a relative path. `SecurityError` on any escape attempt.
    fn resolve(&self, rel: &str) -> Result<PathBuf, FsError> {
        if rel.contains('\\') || rel.contains('\0') {
            return Err(fail("SecurityError", "invalid characters in path"));
        }
        let p = Path::new(rel);
        if p.is_absolute() {
            return Err(fail("SecurityError", "absolute paths are not allowed"));
        }
        let mut out = self.root.clone();
        for c in p.components() {
            match c {
                Component::Normal(seg) => out.push(seg),
                Component::CurDir => {}
                _ => return Err(fail("SecurityError", "path escapes the sandbox")),
            }
        }
        Ok(out)
    }

    pub fn read(&self, rel: &str) -> Result<Vec<u8>, FsError> {
        let p = self.resolve(rel)?;
        if p.is_dir() {
            return Err(fail("TypeMismatchError", format!("{rel} is a directory")));
        }
        std::fs::read(&p).map_err(|_| fail("NotFoundError", rel))
    }

    pub fn write(&self, rel: &str, bytes: &[u8]) -> Result<(), FsError> {
        let p = self.resolve(rel)?;
        if p.is_dir() {
            return Err(fail("TypeMismatchError", format!("{rel} is a directory")));
        }
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| fail("InvalidModificationError", e.to_string()))?;
        }
        std::fs::write(&p, bytes).map_err(|e| fail("InvalidModificationError", e.to_string()))
    }

    pub fn mkdir(&self, rel: &str) -> Result<(), FsError> {
        let p = self.resolve(rel)?;
        std::fs::create_dir_all(&p).map_err(|e| fail("InvalidModificationError", e.to_string()))
    }

    pub fn size(&self, rel: &str) -> Result<u64, FsError> {
        let p = self.resolve(rel)?;
        let md = std::fs::metadata(&p).map_err(|_| fail("NotFoundError", rel))?;
        if md.is_dir() {
            return Err(fail("TypeMismatchError", format!("{rel} is a directory")));
        }
        Ok(md.len())
    }

    pub fn entries(&self, rel: &str) -> Result<Vec<(String, EntryKind)>, FsError> {
        let p = self.resolve(rel)?;
        if !p.exists() {
            // The OPFS root always exists conceptually; other dirs must.
            if rel.is_empty() || rel == "." {
                return Ok(Vec::new());
            }
            return Err(fail("NotFoundError", rel));
        }
        if !p.is_dir() {
            return Err(fail("TypeMismatchError", format!("{rel} is a file")));
        }
        let mut out = Vec::new();
        let rd =
            std::fs::read_dir(&p).map_err(|e| fail("InvalidModificationError", e.to_string()))?;
        for e in rd.flatten() {
            let kind = if e.path().is_dir() {
                EntryKind::Directory
            } else {
                EntryKind::File
            };
            out.push((e.file_name().to_string_lossy().into_owned(), kind));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    pub fn remove(&self, rel: &str, recursive: bool) -> Result<(), FsError> {
        let p = self.resolve(rel)?;
        if !p.exists() {
            return Err(fail("NotFoundError", rel));
        }
        let r = if p.is_dir() {
            if recursive {
                std::fs::remove_dir_all(&p)
            } else {
                std::fs::remove_dir(&p)
            }
        } else {
            std::fs::remove_file(&p)
        };
        r.map_err(|e| fail("InvalidModificationError", e.to_string()))
    }

    pub fn exists(&self, rel: &str) -> Result<Option<EntryKind>, FsError> {
        let p = self.resolve(rel)?;
        if !p.exists() {
            return Ok(None);
        }
        Ok(Some(if p.is_dir() {
            EntryKind::Directory
        } else {
            EntryKind::File
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox() -> (PathBuf, Sandbox) {
        let tmp = std::env::temp_dir().join(format!(
            "day-lite-fsx-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        (tmp.clone(), Sandbox::at(tmp))
    }

    #[test]
    fn roundtrip_and_entries() {
        let (tmp, fs) = sandbox();
        fs.write("notes/a.txt", b"hello").unwrap();
        assert_eq!(fs.read("notes/a.txt").unwrap(), b"hello");
        assert_eq!(fs.size("notes/a.txt").unwrap(), 5);
        let entries = fs.entries("notes").unwrap();
        assert_eq!(entries, vec![("a.txt".to_string(), EntryKind::File)]);
        fs.remove("notes", true).unwrap();
        assert!(fs.read("notes/a.txt").is_err());
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[test]
    fn traversal_is_refused() {
        let (tmp, fs) = sandbox();
        for bad in ["../x", "a/../../x", "/etc/passwd", "a\\b", "a\0b"] {
            let e = fs.write(bad, b"x").unwrap_err();
            assert_eq!(e.name, "SecurityError", "{bad}");
        }
        let _ = std::fs::remove_dir_all(tmp);
    }
}
