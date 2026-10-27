// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// File-backed persistent store for platforms with no in-reach system preferences API: desktop Linux,
// Windows, and (best-effort) HarmonyOS/OpenHarmony. The store is a flat String->String map in
// `preferences.store` in the app's configuration directory (`day_spec::storage::config_dir()`,
// a folder named for the app), serialized one entry per line as
// `escaped_key=escaped_value`. escape() removes every raw `=`, newline, and carriage return, so the
// first raw `=` on a line is unambiguously the separator and a value can contain anything.
//
// A process-wide mutex serializes load-modify-save cycles (the file is shared mutable state). Every
// read tolerates a missing, unreadable, or corrupt file by treating the store as empty, so a partial
// write or a hand-edit can never panic a caller. Writes are best-effort atomic: write a sibling temp
// file, then rename it over the target. Pure std, with no extra dependencies.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

/// The store file's name in a `DAY_PREFS_DIR`, named after the app (docs/prefs.md): the
/// `[app].id` from `Day.toml`, which `day build`/`day launch` export as `DAY_APP_ID` (a dev launch
/// sets it in the process environment; a packaged build bakes it at compile time). A binary
/// built outside the CLI, with no id either way, shares one `day-part-preferences.store`.
fn store_file() -> String {
    let id = std::env::var("DAY_APP_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| option_env!("DAY_APP_ID").map(str::to_owned));
    match id {
        Some(id) => format!("{id}.store"),
        None => "day-part-preferences.store".to_string(),
    }
}

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

/// The store file: under `DAY_PREFS_DIR` when it is set (a private store, which is how a test
/// keeps a store across the processes it starts), else `preferences.store` in the app's
/// configuration directory.
fn store_path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("DAY_PREFS_DIR")
        && !dir.is_empty()
    {
        return Some(PathBuf::from(dir).join(store_file()));
    }
    Some(config_dir()?.join("preferences.store"))
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

// --- The app's configuration directory ---

// The platforms whose store is this file: the app's own configuration folder,
// `$XDG_CONFIG_HOME/<app id>` (else `~/.config/<app id>`) on Linux, `%APPDATA%\<app id>` on
// Windows, the host's files directory on HarmonyOS (`DAY_DATA_DIR`), with a fixed sandbox path
// there when the host passed none so a store is always creatable.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn config_dir() -> Option<PathBuf> {
    let dir = day_spec::storage::config_dir();
    #[cfg(target_env = "ohos")]
    let dir = dir.or_else(|| {
        Some(PathBuf::from(
            "/data/storage/el2/base/haps/entry/files/config",
        ))
    });
    dir
}

// Elsewhere this file serves only the `memory` feature's `DAY_PREFS_DIR` store.
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn config_dir() -> Option<PathBuf> {
    None
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
