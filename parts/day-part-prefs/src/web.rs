// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The browser arm: `localStorage` through the day-dom shim's `day_dom_pref_*` imports
//! (`crates/day-cli/resources/web/shim.js`, `day.pref.` key namespace). Persistence is an OS concern,
//! and on the web the OS is the browser — values survive reloads and browser restarts, scoped
//! per origin. Using this crate on wasm outside a day-dom host page fails at instantiation
//! (the imports are unresolved); the web target is `web-dom` (docs/web.md).

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn day_dom_pref_set(k: *const u8, kl: usize, v: *const u8, vl: usize) -> u32;
    /// Returns the value's FULL byte length (shim writes at most `cap` bytes), -1 if absent.
    fn day_dom_pref_get(k: *const u8, kl: usize, out: *mut u8, cap: usize) -> i32;
    fn day_dom_pref_remove(k: *const u8, kl: usize) -> u32;
    fn day_dom_pref_has(k: *const u8, kl: usize) -> u32;
}

pub fn set(key: &str, value: &str) -> bool {
    unsafe { day_dom_pref_set(key.as_ptr(), key.len(), value.as_ptr(), value.len()) != 0 }
}

pub fn get(key: &str) -> Option<String> {
    let mut buf = vec![0u8; 256];
    loop {
        let n = unsafe { day_dom_pref_get(key.as_ptr(), key.len(), buf.as_mut_ptr(), buf.len()) };
        if n < 0 {
            return None;
        }
        let n = n as usize;
        if n <= buf.len() {
            buf.truncate(n);
            return Some(String::from_utf8_lossy(&buf).into_owned());
        }
        buf = vec![0u8; n]; // larger than the guess: retry with the exact size
    }
}

pub fn remove(key: &str) -> bool {
    unsafe { day_dom_pref_remove(key.as_ptr(), key.len()) != 0 }
}

pub fn contains(key: &str) -> bool {
    unsafe { day_dom_pref_has(key.as_ptr(), key.len()) != 0 }
}
