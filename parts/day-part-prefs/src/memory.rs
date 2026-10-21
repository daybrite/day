// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The in-process store behind the `memory` feature: a map per thread that lasts as long as the
//! thread. Each test thread starts empty, and nothing reaches the platform's preferences. With
//! `DAY_PREFS_DIR` set, the file store in that directory serves instead, so a test that restarts
//! its process keeps a private store across the restart.

use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static STORE: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Whether `DAY_PREFS_DIR` hands the store to the file store.
fn in_dir() -> bool {
    std::env::var_os("DAY_PREFS_DIR").is_some_and(|d| !d.is_empty())
}

pub fn set(key: &str, value: &str) -> bool {
    if in_dir() {
        return super::file::set(key, value);
    }
    STORE.with(|s| s.borrow_mut().insert(key.to_owned(), value.to_owned()));
    true
}

pub fn get(key: &str) -> Option<String> {
    if in_dir() {
        return super::file::get(key);
    }
    STORE.with(|s| s.borrow().get(key).cloned())
}

pub fn remove(key: &str) -> bool {
    if in_dir() {
        return super::file::remove(key);
    }
    STORE.with(|s| s.borrow_mut().remove(key).is_some())
}

pub fn contains(key: &str) -> bool {
    if in_dir() {
        return super::file::contains(key);
    }
    STORE.with(|s| s.borrow().contains_key(key))
}
