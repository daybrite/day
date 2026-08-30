// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// macOS + iOS (one shared file): NSUserDefaults.standard — the system's per-application preferences
// store (a plist under ~/Library/Preferences on macOS, the app container on iOS). It is
// toolkit-independent (no NSApplication / UIApplication, run loop, or window needed), so this works
// in day-qt binaries and plain `cargo test` processes just as well as under day-appkit / day-uikit.
// objc2 Foundation FFI; no Day runtime, no Java shim.
//
// `setObject:forKey:` writes synchronously to the in-memory store, which the system then flushes
// to disk on its OWN schedule — so every write here is followed by `synchronize`, which pushes it
// out now. Apple calls that method unnecessary, and for an app the system suspends politely it is:
// the suspension flushes. A Day app is regularly not that app. A scripted run exits the moment its
// last step finishes, a device can kill a backgrounded app outright, and either one drops whatever
// the periodic flush had not reached yet — silently, since the write itself succeeded. That cost
// a whole CI matrix once (Day-Tradr's iOS walkthrough: the setting written near the end of one
// run was gone by the next, and the three later locale variants each failed six assertions
// against the stale value). A preferences write is rare, small, and made because the user asked
// for it; paying a daemon round-trip to make it real is the right trade.
//
// Only `setObject:forKey:` is `unsafe` in objc2 (the value must be a property-list type) — we
// always pass a real NSString, which is correct.

use objc2::runtime::AnyObject;
use objc2_foundation::{NSString, NSUserDefaults};

pub fn set(key: &str, value: &str) -> bool {
    let defaults = NSUserDefaults::standardUserDefaults();
    let k = NSString::from_str(key);
    let v = NSString::from_str(value);
    // Deref-coerce the concrete NSString to the `&AnyObject` the setter expects.
    let obj: &AnyObject = &v;
    // SAFETY: `obj` is an NSString — a valid property-list value for a string default.
    unsafe { defaults.setObject_forKey(Some(obj), &k) };
    defaults.synchronize()
}

pub fn get(key: &str) -> Option<String> {
    let defaults = NSUserDefaults::standardUserDefaults();
    let k = NSString::from_str(key);
    // stringForKey: coerces numbers to strings and returns nil for absent / non-stringable keys.
    defaults.stringForKey(&k).map(|s| s.to_string())
}

pub fn remove(key: &str) -> bool {
    let defaults = NSUserDefaults::standardUserDefaults();
    let k = NSString::from_str(key);
    let existed = defaults.objectForKey(&k).is_some();
    defaults.removeObjectForKey(&k);
    // Flushed for the same reason a set is: a removal the user asked for must not come back
    // because the process ended before the system got around to writing it.
    defaults.synchronize();
    existed
}

pub fn contains(key: &str) -> bool {
    let defaults = NSUserDefaults::standardUserDefaults();
    let k = NSString::from_str(key);
    // objectForKey: (not stringForKey:) so a stored non-string value still counts as present.
    defaults.objectForKey(&k).is_some()
}
