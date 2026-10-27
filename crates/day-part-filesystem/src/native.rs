// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Every non-wasm target: std::fs under the app's data directory, `day_spec::storage::data_dir()`
// (docs/fs.md "Where files live"): DAY_DATA_DIR when a mobile host or a test sets it, else the
// platform's per-app folder. It resolves on every call (cheap, and env-driven so tests and hosts
// can redirect it). Everything lives under a `day-fs/` leaf, so the paths this part hands out never
// reach the app's other files there (its database, its settings file).
// ---------------------------------------------------------------------------

use std::path::PathBuf;

use super::{BytesResult, FsError, ListResult, UnitResult};

/// The app's data directory, before the `day-fs/` leaf.
pub(crate) fn data_dir() -> Result<PathBuf, FsError> {
    day_spec::storage::data_dir().ok_or(FsError::Unsupported)
}

/// The platform's application-data directory, the parent every app's data sits under.
pub(crate) fn platform_data_dir() -> Result<PathBuf, FsError> {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return Ok(PathBuf::from(home).join("Library/Application Support"));
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(app) = std::env::var_os("APPDATA") {
            return Ok(PathBuf::from(app));
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
    {
        // Linux, Android without a host dir, OHOS without a host dir: XDG data, else ~/.local.
        if let Some(dir) = std::env::var_os("XDG_DATA_HOME")
            && !dir.is_empty()
        {
            return Ok(PathBuf::from(dir));
        }
        if let Some(home) = std::env::var_os("HOME") {
            return Ok(PathBuf::from(home).join(".local/share"));
        }
    }
    Err(FsError::Unsupported)
}

fn root_dir() -> Result<PathBuf, FsError> {
    data_dir().map(|dir| dir.join("day-fs"))
}

fn io(e: std::io::Error) -> FsError {
    if e.kind() == std::io::ErrorKind::NotFound {
        FsError::NotFound
    } else {
        FsError::Io(e.to_string())
    }
}

pub fn read(path: &str) -> BytesResult {
    std::fs::read(root_dir()?.join(path)).map_err(io)
}

pub fn open_read(path: &str) -> Result<std::fs::File, FsError> {
    std::fs::File::open(root_dir()?.join(path)).map_err(io)
}

pub fn write(path: &str, bytes: &[u8]) -> UnitResult {
    let full = root_dir()?.join(path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    std::fs::write(full, bytes).map_err(io)
}

pub fn remove(path: &str) -> UnitResult {
    let full = root_dir()?.join(path);
    if full.is_dir() {
        std::fs::remove_dir(full).map_err(io)
    } else {
        std::fs::remove_file(full).map_err(io)
    }
}

pub fn list(dir: &str) -> ListResult {
    let full = root_dir()?.join(dir);
    let entries = match std::fs::read_dir(full) {
        Ok(e) => e,
        // A never-written directory lists as empty rather than erroring: "nothing stored yet"
        // is the ordinary first-run state, not a failure.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io(e)),
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let mut name = e.file_name().to_string_lossy().into_owned();
            if e.file_type().ok()?.is_dir() {
                name.push('/');
            }
            Some(name)
        })
        .collect();
    names.sort();
    Ok(names)
}

// Async twins: each operation is a job on day-async's blocking pool (docs/async.md), detached
// because the completion callback is the result. The pool is bounded, so a burst of saves
// queues rather than starting a thread apiece.

pub fn read_async(path: String, on_done: Box<dyn FnOnce(BytesResult) + Send>) {
    day_async::spawn_blocking(move || on_done(read(&path))).detach();
}

pub fn write_async(path: String, bytes: Vec<u8>, on_done: Box<dyn FnOnce(UnitResult) + Send>) {
    day_async::spawn_blocking(move || on_done(write(&path, &bytes))).detach();
}

pub fn remove_async(path: String, on_done: Box<dyn FnOnce(UnitResult) + Send>) {
    day_async::spawn_blocking(move || on_done(remove(&path))).detach();
}

pub fn list_async(dir: String, on_done: Box<dyn FnOnce(ListResult) + Send>) {
    day_async::spawn_blocking(move || on_done(list(&dir))).detach();
}
