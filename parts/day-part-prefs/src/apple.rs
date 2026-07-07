// macOS + iOS (one shared file): NSUserDefaults.standard — the system's per-application preferences
// store (a plist under ~/Library/Preferences on macOS, the app container on iOS). It is
// toolkit-independent (no NSApplication / UIApplication, run loop, or window needed), so this works
// in day-qt binaries and plain `cargo test` processes just as well as under day-appkit / day-uikit.
// objc2 Foundation FFI; no Day runtime, no Java shim.
//
// `setObject:forKey:` writes synchronously to the in-memory store and is flushed to disk
// asynchronously by the system; a value written here is immediately readable and persists across
// launches. Only `setObject:forKey:` is `unsafe` in objc2 (the value must be a property-list type) —
// we always pass a real NSString, which is correct.

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
    true
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
    existed
}

pub fn contains(key: &str) -> bool {
    let defaults = NSUserDefaults::standardUserDefaults();
    let k = NSString::from_str(key);
    // objectForKey: (not stringForKey:) so a stored non-string value still counts as present.
    defaults.objectForKey(&k).is_some()
}
