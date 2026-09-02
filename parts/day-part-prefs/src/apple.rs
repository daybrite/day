// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// macOS + iOS (one shared file): NSUserDefaults.standard — the system's per-application preferences
// store (a plist under ~/Library/Preferences on macOS, the app container on iOS). It is
// toolkit-independent (no NSApplication / UIApplication, run loop, or window needed), so this works
// in day-qt binaries and plain `cargo test` processes just as well as under day-appkit / day-uikit.
// objc2 Foundation FFI; no Day runtime, no Java shim.
//
// `setObject:forKey:` writes synchronously to the in-memory store, which the system then flushes
// to disk on its OWN schedule — so every write here is followed by `synchronize`, which hands it
// to the preferences daemon now. That is as far as an app can push it: the daemon still writes
// the plist when it chooses, and on the simulator that was measured to be well after a scripted
// run had ended (three writes to one key, the plist held the first). What survives a plain
// relaunch is the daemon's copy, which is current; what loses the late writes is a REINSTALL of
// the app, which migrates its container and rereads the stale plist. That cost Day-Tradr's iOS
// walkthrough matrix twice: once before `synchronize` was here at all, and once more because
// `day launch` reinstalled the app for every locale variant — the CLI now installs a build once
// per simulator and relaunches it. A preferences write is rare, small, and made because the
// user asked for it; the daemon round-trip is the right trade even so.
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
