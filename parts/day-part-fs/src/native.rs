// ---------------------------------------------------------------------------
// Every non-wasm target: std::fs under the per-app data root. The root resolves once per call
// (cheap, and env-driven so tests and hosts can redirect it):
//   1. DAY_DATA_DIR — set by the mobile hosts (DayActivity passes the app's files dir on
//      Android; the OpenHarmony host passes its files dir) and by tests.
//   2. The platform's app-data convention: Application Support on Apple (the iOS sandbox HOME
//      makes this the app container), XDG data on Linux, APPDATA on Windows.
// Everything lives under a `day-fs/` leaf so the root never collides with other day state in
// the same directory (day-part-prefs' store lives beside it, not inside it).
// ---------------------------------------------------------------------------

use std::path::PathBuf;

use super::{BytesResult, FsError, ListResult, UnitResult};

fn root_dir() -> Result<PathBuf, FsError> {
    if let Some(dir) = std::env::var_os("DAY_DATA_DIR")
        && !dir.is_empty()
    {
        return Ok(PathBuf::from(dir).join("day-fs"));
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return Ok(PathBuf::from(home)
                .join("Library/Application Support/day")
                .join("day-fs"));
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(app) = std::env::var_os("APPDATA") {
            return Ok(PathBuf::from(app).join("day").join("day-fs"));
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
    {
        // Linux, Android without a host dir, OHOS without a host dir: XDG data, else ~/.local.
        if let Some(dir) = std::env::var_os("XDG_DATA_HOME")
            && !dir.is_empty()
        {
            return Ok(PathBuf::from(dir).join("day").join("day-fs"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            return Ok(PathBuf::from(home).join(".local/share/day").join("day-fs"));
        }
    }
    Err(FsError::Unsupported)
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

// Async twins: a spawned thread per operation, like day-part-http's non-Apple targets. Storage
// operations are short and the callers are UI actions — a pool would be over-machinery.

pub fn read_async(path: String, on_done: Box<dyn FnOnce(BytesResult) + Send>) {
    std::thread::spawn(move || on_done(read(&path)));
}

pub fn write_async(path: String, bytes: Vec<u8>, on_done: Box<dyn FnOnce(UnitResult) + Send>) {
    std::thread::spawn(move || on_done(write(&path, &bytes)));
}

pub fn remove_async(path: String, on_done: Box<dyn FnOnce(UnitResult) + Send>) {
    std::thread::spawn(move || on_done(remove(&path)));
}

pub fn list_async(dir: String, on_done: Box<dyn FnOnce(ListResult) + Send>) {
    std::thread::spawn(move || on_done(list(&dir)));
}
