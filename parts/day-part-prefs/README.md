# day-part-prefs

A small persistent key/value store through each platform's native preferences facility —
`set`/`get`/`remove`/`contains` for small strings that survive relaunches.

`NSUserDefaults` on macOS and iOS, `SharedPreferences` on Android, and a plain file store
under the user config directory on Linux, Windows, and OpenHarmony (documented, corruption-
tolerant format — no registry surprises).

Parts are Day's headless extension unit — platform capability crates with no UI. This one
works in **any Rust program**; a Day app is not required.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
