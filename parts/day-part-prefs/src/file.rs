// File-backed persistent store for platforms with no in-reach system preferences API: desktop Linux,
// Windows, and (best-effort) HarmonyOS/OpenHarmony. The store is a flat String->String map under
// `<config-dir>/day/day-part-prefs.store`, serialized one entry per line as
// `escaped_key=escaped_value`. escape() removes every raw `=`, newline, and carriage return, so the
// first raw `=` on a line is unambiguously the separator and a value can contain anything.
//
// A process-wide mutex serializes load-modify-save cycles (the file is shared mutable state). Every
// read tolerates a missing, unreadable, or corrupt file by treating the store as empty, so a partial
// write or a hand-edit can never panic a caller. Writes are best-effort atomic: write a sibling temp
// file, then rename it over the target. Pure std — no extra dependencies.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

/// The store file, created under `<config-dir>/day/`.
const STORE_FILE: &str = "day-part-prefs.store";

/// Serializes this process's accesses (two threads writing concurrently could lose an update or read
/// a half-written file). Cross-process concurrency is out of scope for a settings store.
static LOCK: Mutex<()> = Mutex::new(());

pub fn set(key: &str, value: &str) -> bool {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut map = load();
    map.insert(key.to_string(), value.to_string());
    save(&map)
}

pub fn get(key: &str) -> Option<String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load().remove(key)
}

pub fn remove(key: &str) -> bool {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut map = load();
    if map.remove(key).is_none() {
        return false;
    }
    save(&map)
}

pub fn contains(key: &str) -> bool {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load().contains_key(key)
}

/// Read the whole store. A missing, unreadable, or corrupt file yields an empty map.
fn load() -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Some(path) = store_path() else {
        return map;
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return map;
    };
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        // escape() leaves exactly one raw `=` per line: the separator.
        if let Some(sep) = line.find('=') {
            let key = unescape(&line[..sep]);
            let value = unescape(&line[sep + 1..]);
            map.insert(key, value);
        }
    }
    map
}

/// Write the whole store, creating the parent directory as needed. Best-effort atomic: write a
/// sibling temp file then rename it over the target so a crash mid-write can't truncate the store.
fn save(map: &BTreeMap<String, String>) -> bool {
    let Some(path) = store_path() else {
        return false;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut out = String::new();
    for (key, value) in map {
        out.push_str(&escape(key));
        out.push('=');
        out.push_str(&escape(value));
        out.push('\n');
    }
    let tmp = path.with_extension("store.tmp");
    if fs::write(&tmp, out).is_err() {
        return false;
    }
    if fs::rename(&tmp, &path).is_err() {
        let _ = fs::remove_file(&tmp);
        return false;
    }
    true
}

fn store_path() -> Option<PathBuf> {
    Some(config_dir()?.join("day").join(STORE_FILE))
}

/// Escape so the result contains no raw `=`, newline, or carriage return:
/// `\` -> `\\`, newline -> `\n`, CR -> `\r`, `=` -> `\e`.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '=' => out.push_str("\\e"),
            _ => out.push(c),
        }
    }
    out
}

/// Inverse of [`escape`]. Unknown escapes pass the following char through verbatim.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('e') => out.push('='),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

// --- Per-OS config directory ---

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME")
        && !dir.is_empty()
    {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
}

#[cfg(target_os = "windows")]
fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("APPDATA")
        && !dir.is_empty()
    {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join("AppData").join("Roaming"))
}

// HarmonyOS/OpenHarmony: best-effort. Prefer an app-provided files dir from the environment, then the
// usual roots, and finally a fixed sandbox path so a store is always creatable.
#[cfg(all(target_os = "linux", target_env = "ohos"))]
fn config_dir() -> Option<PathBuf> {
    for var in ["OHOS_APP_FILES_DIR", "XDG_CONFIG_HOME", "HOME", "TMPDIR"] {
        if let Some(dir) = std::env::var_os(var)
            && !dir.is_empty()
        {
            return Some(PathBuf::from(dir));
        }
    }
    Some(PathBuf::from("/data/storage/el2/base/haps/entry/files"))
}

#[cfg(test)]
mod tests {
    use super::{escape, unescape};

    #[test]
    fn escape_round_trips_special_chars() {
        for original in [
            "",
            "plain",
            "a=b",
            "line1\nline2",
            "carriage\rreturn",
            "back\\slash",
            "all\\=\n\r together",
        ] {
            let escaped = escape(original);
            assert!(
                !escaped.contains('='),
                "escaped still has raw '=': {escaped:?}"
            );
            assert!(
                !escaped.contains('\n'),
                "escaped still has newline: {escaped:?}"
            );
            assert_eq!(unescape(&escaped), original);
        }
    }
}
