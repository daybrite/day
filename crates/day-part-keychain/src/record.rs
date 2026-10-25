// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The file an item is kept in, where the platform's store holds keys rather than items.
//!
//! Windows (DPAPI) and HarmonyOS (the Universal Keystore) encrypt bytes but keep no items of
//! their own, so this crate keeps one file per item in the app's private data directory and asks
//! the platform to protect its payload. The Android arm lays its files out the same way, in Java.
//!
//! # The record
//!
//! ```text
//! "DAYKEYCH"  8 bytes   magic
//! version     1 byte    1
//! protection  1 byte    Protection::code
//! key length  2 bytes   little-endian, the key's UTF-8 length
//! key         n bytes
//! payload               whatever the platform arm produced (a DPAPI blob, nonce ‖ ciphertext ‖ tag)
//! ```
//!
//! The key is stored in clear because it is not a secret (the app chose it) and because a read has
//! to verify that the file it found by hash is the item it wants. The version byte is what a later
//! layout would check; a record of a version this build does not know reads as
//! [`KeychainError::Unreadable`], never as garbage.
//!
//! # Files
//!
//! An item's file is `<hex sha256 of the key>.dayk` under the store directory; the hash gives a
//! fixed-length, case-insensitive-safe name from any key. Writes go to a temporary file in the
//! same directory and are renamed over the old one, so a read never sees half an item and a
//! crash mid-write leaves the previous item intact. A process-wide lock serializes the writes and
//! deletes; concurrent writers of the same key leave one of their values, whole.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::{Entry, KeychainError, Protection};

const MAGIC: &[u8; 8] = b"DAYKEYCH";
const VERSION: u8 = 1;
const EXTENSION: &str = "dayk";

/// A decoded record.
pub(crate) struct Record {
    pub protection: Protection,
    pub key: String,
    pub payload: Vec<u8>,
}

/// Encode one record.
pub(crate) fn encode(key: &str, protection: Protection, payload: &[u8]) -> Vec<u8> {
    let key_len = u16::try_from(key.len()).unwrap_or(u16::MAX);
    let mut out = Vec::with_capacity(12 + key.len() + payload.len());
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.push(protection.code());
    out.extend_from_slice(&key_len.to_le_bytes());
    out.extend_from_slice(&key.as_bytes()[..usize::from(key_len)]);
    out.extend_from_slice(payload);
    out
}

/// Decode one record. Anything that is not exactly a record of this version is `Unreadable`.
pub(crate) fn decode(bytes: &[u8]) -> Result<Record, KeychainError> {
    if bytes.len() < 12 || &bytes[..8] != MAGIC || bytes[8] != VERSION {
        return Err(KeychainError::Unreadable);
    }
    let protection = Protection::from_code(bytes[9]).ok_or(KeychainError::Unreadable)?;
    let key_len = usize::from(u16::from_le_bytes([bytes[10], bytes[11]]));
    let key_end = 12 + key_len;
    if bytes.len() < key_end {
        return Err(KeychainError::Unreadable);
    }
    let key = std::str::from_utf8(&bytes[12..key_end])
        .map_err(|_| KeychainError::Unreadable)?
        .to_string();
    Ok(Record {
        protection,
        key,
        payload: bytes[key_end..].to_vec(),
    })
}

/// The file an item lives in, under `dir`.
pub(crate) fn path_for(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!(
        "{}.{EXTENSION}",
        crate::sha256::hex_digest(key.as_bytes())
    ))
}

/// Serializes writes and deletes within this process.
static LOCK: Mutex<()> = Mutex::new(());

fn locked() -> std::sync::MutexGuard<'static, ()> {
    match LOCK.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn io_error(e: std::io::Error) -> KeychainError {
    KeychainError::Platform(e.to_string())
}

/// Write a record for `key` under `dir`, atomically, creating the directory.
pub(crate) fn write(
    dir: &Path,
    key: &str,
    protection: Protection,
    payload: &[u8],
) -> Result<(), KeychainError> {
    let _guard = locked();
    std::fs::create_dir_all(dir).map_err(io_error)?;
    let final_path = path_for(dir, key);
    let temp_path = dir.join(format!(
        "{}.{}.tmp",
        crate::sha256::hex_digest(key.as_bytes()),
        std::process::id()
    ));
    let bytes = encode(key, protection, payload);
    let written =
        std::fs::write(&temp_path, &bytes).and_then(|()| std::fs::rename(&temp_path, &final_path));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp_path);
        return Err(io_error(e));
    }
    Ok(())
}

/// The record for `key` under `dir`, verified to be that key's.
pub(crate) fn read(dir: &Path, key: &str) -> Result<Record, KeychainError> {
    let bytes = match std::fs::read(path_for(dir, key)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(KeychainError::NotFound),
        Err(e) => return Err(io_error(e)),
    };
    let record = decode(&bytes)?;
    if record.key != key {
        return Err(KeychainError::Unreadable);
    }
    Ok(record)
}

/// Remove `key`'s file under `dir`. `NotFound` when there is none.
pub(crate) fn remove(dir: &Path, key: &str) -> Result<(), KeychainError> {
    let _guard = locked();
    match std::fs::remove_file(path_for(dir, key)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(KeychainError::NotFound),
        Err(e) => Err(io_error(e)),
    }
}

/// Whether `key` has a file under `dir`.
pub(crate) fn exists(dir: &Path, key: &str) -> bool {
    path_for(dir, key).is_file()
}

/// Every readable record's key and level under `dir`. A damaged file is skipped here (it is
/// reported by the read that asks for it); a missing directory is an empty store.
pub(crate) fn entries(dir: &Path) -> Result<Vec<Entry>, KeychainError> {
    let listing = match std::fs::read_dir(dir) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_error(e)),
    };
    let mut out = Vec::new();
    for entry in listing.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(EXTENSION) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        // Only the header is needed; the payload stays in the file.
        if let Ok(record) = decode(&bytes) {
            out.push(Entry {
                key: record.key,
                protection: record.protection,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_round_trips() {
        let bytes = encode("api/token", Protection::UserPresence, &[1, 2, 3]);
        assert_eq!(&bytes[..8], b"DAYKEYCH");
        let r = decode(&bytes).expect("decodes");
        assert_eq!(r.key, "api/token");
        assert_eq!(r.protection, Protection::UserPresence);
        assert_eq!(r.payload, vec![1, 2, 3]);
        let empty = decode(&encode("k", Protection::WhenUnlocked, &[])).expect("decodes");
        assert!(empty.payload.is_empty());
    }

    #[test]
    fn damaged_records_are_unreadable_not_garbage() {
        assert_eq!(decode(b"").err(), Some(KeychainError::Unreadable));
        assert_eq!(decode(b"DAYKEYCH").err(), Some(KeychainError::Unreadable));
        let mut wrong_version = encode("k", Protection::WhenUnlocked, b"x");
        wrong_version[8] = 2;
        assert_eq!(
            decode(&wrong_version).err(),
            Some(KeychainError::Unreadable)
        );
        let mut bad_protection = encode("k", Protection::WhenUnlocked, b"x");
        bad_protection[9] = 7;
        assert_eq!(
            decode(&bad_protection).err(),
            Some(KeychainError::Unreadable)
        );
        let mut truncated_key = encode("key", Protection::WhenUnlocked, b"");
        truncated_key.truncate(13);
        assert_eq!(
            decode(&truncated_key).err(),
            Some(KeychainError::Unreadable)
        );
    }

    #[test]
    fn files_round_trip_in_a_scratch_directory() {
        let dir =
            std::env::temp_dir().join(format!("day-part-keychain-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(entries(&dir), Ok(Vec::new()));
        assert_eq!(read(&dir, "a").err(), Some(KeychainError::NotFound));
        assert_eq!(remove(&dir, "a").err(), Some(KeychainError::NotFound));
        write(&dir, "a", Protection::WhenUnlocked, b"one").expect("write a");
        write(&dir, "b", Protection::AfterFirstUnlock, b"two").expect("write b");
        assert!(exists(&dir, "a") && exists(&dir, "b") && !exists(&dir, "c"));
        let a = read(&dir, "a").expect("read a");
        assert_eq!(
            (a.key.as_str(), a.protection, a.payload.as_slice()),
            ("a", Protection::WhenUnlocked, &b"one"[..])
        );
        write(&dir, "a", Protection::UserPresence, b"uno").expect("replace a");
        let a = read(&dir, "a").expect("read a again");
        assert_eq!(
            (a.protection, a.payload.as_slice()),
            (Protection::UserPresence, &b"uno"[..])
        );
        let mut listed = entries(&dir).expect("entries");
        listed.sort_by(|x, y| x.key.cmp(&y.key));
        assert_eq!(
            listed,
            vec![
                Entry {
                    key: "a".into(),
                    protection: Protection::UserPresence
                },
                Entry {
                    key: "b".into(),
                    protection: Protection::AfterFirstUnlock
                },
            ]
        );
        // A file whose record names another key is not that key's item.
        std::fs::write(
            path_for(&dir, "c"),
            encode("d", Protection::WhenUnlocked, b"x"),
        )
        .expect("plant");
        assert_eq!(read(&dir, "c").err(), Some(KeychainError::Unreadable));
        assert_eq!(remove(&dir, "a"), Ok(()));
        assert!(!exists(&dir, "a"));
        // No temporary files are left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .expect("dir")
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
