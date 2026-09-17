---
title: "macOS App Sandbox"
description: "Configure signed sandboxed AppKit builds, file access, and persistent security-scoped bookmarks."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# macOS App Sandbox

Day can sign `macos-appkit` applications with Apple's App Sandbox during ordinary builds,
launches, and packaging. Day-Showcase enables it. App Sandbox is required for Mac App Store
applications; it is separate from hardened runtime, notarization, and privacy consent (TCC).
See Apple's [App Sandbox documentation](https://developer.apple.com/documentation/security/app-sandbox).

## Configuration

Add this table to `Day.toml`, then rebuild:

```toml
[sandbox.macos-appkit]
enabled = true
user-selected-files = "read-write"
bookmarks = true
network-client = false
network-server = false
development-network-server = true
camera = false
microphone = false
location = false
bluetooth = false
```

These are the defaults except `enabled`, which defaults to false. An absent table preserves
legacy custom entitlements. Set `enabled = false` to disable sandboxing explicitly; changing
metadata does not change an already-built executable. `--skip-build` reuses its existing policy.
This table applies only to AppKit. Qt and GTK's current macOS development executables are not
bundled/sandboxed by this setting.

`user-selected-files` accepts `none`, `read-only`, or `read-write`. The latter allows save dialogs.
`bookmarks` enables app-scoped security-scoped bookmarks. Network client and server grants are
independent: even loopback connections require the corresponding grant. Debug builds add the
server entitlement when `development-network-server = true` so Day's automation listener can
bind to localhost. The entitlement itself permits listening beyond localhost; Day's listener
binds only to loopback. Release builds receive no implicit server grant. Disable the development
grant if you do not use scripting, or explicitly enable `network-server` if your release app
needs a listener. Showcase does so for its Network & HTTP demonstration server.

Device entitlements do not replace privacy usage descriptions or user consent. Configure the
corresponding Day permissions/Info.plist descriptions too. Showcase grants camera and location
for its permission demonstrations, plus outgoing networking for its network examples.

The CLI merges these settings with `[signing.macos] entitlements = "path/to/custom.plist"`.
Unrelated custom values are preserved, including arrays. Conflicting typed settings produce an
error, rather than silently broadening permissions. Generated plists live under
`build/day/entitlements/macos-appkit/`, separately for Debug and Release. Both Xcode and
packaging signatures use this same plan, including ad-hoc signatures. The CLI verifies the
resulting signature and requested entitlement values after building and packaging.

Use `day build -p macos-appkit` before opening the generated Xcode project: this refreshes
Day's generated xcconfig. A hand-edited local Xcode configuration can still change IDE build
settings; inspect the resulting signed artifact when distributing it:

```sh
codesign --verify --strict path/to/MyApp.app
codesign --display --entitlements - --xml path/to/MyApp.app
```

## Open, save, and reopen

Use Day's native [file pickers](files.md). AppKit uses `NSOpenPanel` and `NSSavePanel`, which
ask the system's Powerbox to authorize the user's selection. A path typed into your own text
field, received over automation, or read from preferences grants no access by itself. Normal
file I/O can still fail after selection (for example, a removed volume); handle errors.
Native file drops can also carry a system access grant. Do not assume a custom string MIME
payload containing a path grants access to that path.

A saved path is not a durable permission. To reopen a selected file after relaunch, store the
opaque bytes from `FileUrl::bookmark(read_only)` in app-private preferences or a database.
Resolve them with `FileUrl::resolve_bookmark`:

```rust
// While the user-selected file is accessible:
let bookmark = selected_file.bookmark(true)?;
// Persist `bookmark` as bytes in your application storage.

// In a later launch:
let access = FileUrl::resolve_bookmark(&bookmark)?;
let contents = access.file().read()?;
if access.was_stale() {
    let replacement = access.file().bookmark(true)?;
    // Replace the stored bookmark with `replacement`.
}
// Dropping `access` balances native security-scoped access.
```

Keep `FileAccess` alive for the entire I/O operation, including asynchronous work. Cloning its
`FileUrl` does not extend the grant. Resolve each time rather than assuming the old path remains
valid: a moved file can resolve to a new location. Renew stale bookmarks with the original
read-only/read-write policy while the guard is alive. If resolution or reading fails, offer the
picker again. The API returns `Unsupported` on other operating systems; these bytes are not a
portable file reference or a permission token to share with another app.

The Files Showcase page stores a read-only bookmark after opening a file and exposes **Reopen
last file**, including after relaunch. A resolved URL that needs no new scoped grant (for
example, an app-private file) is usable too; actual I/O determines whether access is permitted.

## Storage and other restrictions

* Keep preferences, SQLite databases and writable application state in app-private storage.
  macOS supplies the sandbox container as the application's home. Day's normal storage defaults
  follow that home; they must not be replaced with a hard-coded user's home path.
* A SQLite database can create journal/WAL/SHM files next to itself. Selecting only a database
  file does not promise access to its siblings. Prefer importing it into private storage;
  document-in-place workflows need an appropriately authorized directory and lifecycle.
* Bundled resources are read-only. Stage exports in the app's temporary directory and use the
  save picker to write the selected destination.
* Environment overrides such as `DAY_DATA_DIR` or recording paths do not bypass sandbox policy.
  Use container paths or explicitly authorized locations.
* Enabling sandboxing changes where existing preferences/databases are found. It does not
  automatically migrate unsandboxed data. Provide an explicit import workflow for existing users.
* Clipboard content and native drag-and-drop remain available, but an arbitrary file path in
  their payloads is not a substitute for an OS-granted file reference or a picker.
* Process execution, external tools, Apple Events and access to other applications' private
  storage may be restricted. Do not add blanket temporary exceptions to make a development-only
  feature work. Review each capability and Apple's distribution requirements individually.

## Distribution and validation

A sandboxed build is not an App Store submission package. Day's existing `day pack` macOS path
creates a Developer ID/ad-hoc DMG and optionally notarizes it. Mac App Store submission still
requires the appropriate Apple distribution identity, provisioning, archive/installer workflow,
and App Store review. Custom entitlements must match the provisioning profile. Debug's
`get-task-allow` belongs in development builds, not submitted apps. For sandboxed Release builds,
Day disables Xcode's base entitlement injection and rejects a signed debugger grant. This follows
Apple's [distribution signing guidance](https://developer.apple.com/documentation/security/resolving-common-notarization-issues).

Test the signed app, not a bare Cargo executable. Exercise native open and save outside the
container, cancel/error paths, reopening bookmarks after quitting, stale bookmarks, and denied
access to unselected files. Test networking, private persistence, resources, and the device
features your app declares. Dayscript `respond` bypasses the native picker; a scripted round-trip
inside app temp storage verifies application behavior but does not verify a Powerbox grant.

Apple's [sandbox entitlement reference](https://developer.apple.com/library/archive/documentation/Miscellaneous/Reference/EntitlementKeyReference/Chapters/EnablingAppSandbox.html)
describes file-selection, bookmark, network, and device entitlements.
