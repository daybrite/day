---
title: "App extensions"
description: "Widgets, share and action extensions, notification extensions, Safari web extensions, the MarketplaceKit extension and more: declared in Day.toml, written in Swift, and built and signed with the iOS and macOS app."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# App extensions

An [app extension](https://developer.apple.com/app-extensions/) is a small bundle the system
loads outside your app: a widget on the Home Screen, an entry in the share sheet, a notification
service that rewrites a push before it is shown, a Safari web extension, a custom keyboard. On
iOS and macOS the app carries its extensions inside its own bundle, and the system discovers them
when the app is installed.

Day builds each extension as its own target in the generated Xcode project, embeds it in the
`ios-uikit` or `macos-appkit` app, and signs it with the app. You declare it in `Day.toml` and
write it in Swift, because every extension point is a Swift or Objective-C API.

## Add an extension

```sh
day project add-extension widget
```

This writes a starter under `platform/<os>/<Name>/` for every Apple target the app has, and adds
a declaration to `Day.toml`:

```toml
[[extensions]]
name = "Widgets"
kind = "widget"
```

The next `day build -t ios-uikit` (or `day launch`) builds the widget and embeds it in the app.
Long-press the Home Screen on the simulator to add it.

`day project add-extension --list` lists every kind. `--name` picks the target name (the default
is derived from the kind), and `--platform ios` or `--platform macos` limits it to one platform.

| Kind | Platforms | What it is |
| --- | --- | --- |
| `widget` | iOS, macOS | Home Screen, Lock Screen and desktop widgets, Live Activities and controls ([WidgetKit](https://developer.apple.com/documentation/widgetkit)) |
| `share` | iOS, macOS | an entry in the share sheet that receives links, text or files |
| `action` | iOS, macOS | an action in the share sheet that transforms or shows content in place |
| `notification-service` | iOS, macOS | rewrites a remote notification before it is shown ([UNNotificationServiceExtension](https://developer.apple.com/documentation/usernotifications/unnotificationserviceextension)) |
| `notification-content` | iOS, macOS | a custom interface for an expanded notification |
| `safari-web-extension` | iOS, macOS | a [Safari web extension](https://developer.apple.com/documentation/safariservices/safari-web-extensions): a WebExtensions manifest, scripts and a native handler |
| `content-blocker` | iOS, macOS | a [Safari content blocker](https://developer.apple.com/documentation/safariservices/creating-a-content-blocker): a JSON rule list |
| `keyboard` | iOS | a custom system-wide keyboard |
| `intents` | iOS, macOS | handles SiriKit intents in the background |
| `intents-ui` | iOS | the view Siri shows for a SiriKit intent |
| `app-intents` | iOS, macOS | [App Intents](https://developer.apple.com/documentation/appintents) that Shortcuts, Siri and Spotlight run without launching the app |
| `file-provider` | iOS, macOS | the app's documents in the Files app or Finder ([replicated File Provider](https://developer.apple.com/documentation/fileprovider)) |
| `file-provider-ui` | iOS | custom actions on File Provider items |
| `photo-editing` | iOS, macOS | edits photos and videos inside the Photos app |
| `broadcast-upload` | iOS, macOS | receives a ReplayKit screen broadcast |
| `broadcast-setup-ui` | iOS, macOS | the setup screen before a ReplayKit broadcast |
| `call-directory` | iOS | identifies or blocks callers ([CallKit](https://developer.apple.com/documentation/callkit)) |
| `message-filter` | iOS | files SMS and MMS from unknown senders |
| `credential-provider` | iOS, macOS | the app's passwords and passkeys in AutoFill |
| `packet-tunnel`, `app-proxy` | iOS, macOS | a VPN tunnel or a per-app proxy ([Network Extension](https://developer.apple.com/documentation/networkextension)) |
| `dns-proxy`, `filter-data`, `filter-control` | iOS | a DNS proxy, or the two halves of a content filter |
| `quicklook-preview`, `quicklook-thumbnail` | iOS, macOS | previews and thumbnails of the app's document types |
| `spotlight-import`, `spotlight-index` | iOS, macOS | indexes documents and searchable items for Spotlight |
| `imessage` | iOS | an iMessage app |
| `sticker-pack` | iOS | a sticker pack for Messages, from an asset catalog |
| `location-push` | iOS | answers a location request pushed from a server |
| `finder-sync` | macOS | badges and menu items in Finder |
| `marketplace` | iOS | the [MarketplaceKit](https://developer.apple.com/documentation/marketplacekit) extension of an alternative app marketplace |
| `custom` | iOS, macOS | any other extension point: set `point` yourself |

`app-intents` and `marketplace` are [ExtensionKit](https://developer.apple.com/documentation/extensionkit)
extensions, embedded in the app's `Extensions/` folder. Every other kind is an NSExtension app
extension, embedded in `PlugIns/`. The kind decides this; you never set it.

## The declaration

Every key except `name` and `kind` is optional.

```toml
[[extensions]]
name = "ShareExtension"           # the Xcode target, the .appex, and the Swift module
kind = "share"
platforms = ["ios"]               # default: every Apple target the app has that the kind exists on
sources = "platform/ios/ShareExtension"   # the default; or { ios = "…", macos = "…" }
bundle-id = "share"               # a suffix → <app id>.share; or a full id that starts with the app's
display-name = "Save to Fleet"    # CFBundleDisplayName, what the share sheet shows
deployment-target = "17.0"        # or { ios = "17.0", macos = "14.0" }
principal-class = "ShareViewController"   # the preset's class; qualified with the module for you
attributes = { NSExtensionActivationRule = { NSExtensionActivationSupportsWebURLWithMaxCount = 1 } }
info = { NSExtension = { NSExtensionAttributes = { Custom = true } } }   # any Info.plist keys
entitlements = "platform/ios/ShareExtension/Share.entitlements"
app-groups = ["group.dev.example.fleet"]
frameworks = ["LinkPresentation"]
swift-packages = [{ url = "https://github.com/example/kit", from = "1.0.0", products = ["Kit"] }]
build-settings = { SWIFT_STRICT_CONCURRENCY = "complete" }
```

| Key | Meaning |
| --- | --- |
| `name` | Target, product and Swift module name: ASCII letters, digits and underscores, starting with a letter. It cannot be `Runner`, and it cannot be the name of a system framework such as `Intents` or `WidgetKit`, which a module of the same name hides. |
| `kind` | A kind from the table above. |
| `point` | The extension point identifier, for `kind = "custom"` or to override a preset. |
| `extensionkit` | `custom` only: `true` for an ExtensionKit point (`EXAppExtensionAttributes`, embedded in `Extensions/`). |
| `platforms` | `ios`, `macos`, or both. |
| `sources` | The source folder, relative to the project. |
| `bundle-id` | A suffix appended to the app's id, or a full id that starts with the app's. The default suffix is `name`. A suffix follows the app's id through [flavors](/docs/how-to/flavors). |
| `display-name` | `CFBundleDisplayName`. The default is `name`. |
| `deployment-target` | The oldest OS version it runs on. The default is the later of the kind's floor and the app's deployment target. An extension may require a newer OS than the app; on an older one the system does not load it. |
| `principal-class` | The class the system instantiates. Unqualified names are qualified with the extension's module. |
| `attributes` | Merged into `NSExtensionAttributes`, or into `EXAppExtensionAttributes` for an ExtensionKit extension. |
| `info` | Merged into the top level of the `Info.plist`. |
| `entitlements` | An entitlements plist merged into the generated one. Without this key, `<sources>/<name>.entitlements` is merged when it exists. |
| `app-groups` | App Group identifiers shared with the app. |
| `frameworks` | System frameworks to link beyond those the Swift sources import (imports link themselves). |
| `swift-packages` | Swift packages the extension links: `url` with `from`, `exact`, `branch` or `revision`, or `path` relative to the project; `products` defaults to the package name. |
| `live-activities` | `widget` only: the bundle has Live Activities, so Day adds `NSSupportsLiveActivities` to the app's `Info.plist`. |
| `build-settings` | Target build settings for anything this table does not cover. |

## The source folder

Everything in `platform/<os>/<Name>/` belongs to the extension. The target uses an
Xcode *synchronized folder*, so a file you add there is part of the next build without editing
the project:

- Swift files compile into the extension.
- Asset catalogs and other resources are copied into the bundle. Like a group in Xcode, files in
  subfolders land at the top of the bundle.
- `Resources/` is the exception: it is copied with its folder layout intact. A Safari web
  extension keeps its `manifest.json`, scripts and `_locales/` there, and a content blocker its
  rule list.
- `Info.plist`, when present, is merged over the preset (below), not copied.
- `<Name>.entitlements` is merged into the extension's entitlements.

## Info.plist and entitlements

Day writes each extension's `Info.plist` and entitlements to
`build/day/extensions/<os>/<Name>/` on every build, in layers, each merged over the one before
(dictionaries merge key by key, anything else replaces):

1. The bundle keys every extension needs: `CFBundleIdentifier`, `CFBundleExecutable`, and the
   app's version and build number through `$(MARKETING_VERSION)` and
   `$(CURRENT_PROJECT_VERSION)`. App Store Connect rejects an extension whose version differs
   from its app's, so they always match.
2. The kind's preset: the extension point, the principal class, and the attributes the point
   needs (a share extension's activation rule, a keyboard's language).
3. The source folder's own `Info.plist`, if any.
4. `info` and `attributes` from `Day.toml`.

The entitlements start from what the kind requires (`packet-tunnel` claims its Network Extension
provider type, `credential-provider` the AutoFill capability), add `app-groups`, add the
[App Sandbox](https://developer.apple.com/documentation/security/app-sandbox) on macOS, where
every app extension must run sandboxed, and merge the extension's own entitlements file last.
An extension with nothing to claim gets an empty file.

The files are rewritten only when their content changes, so an unchanged extension does not
rebuild.

## Sharing data with the app

An extension runs in its own process and cannot see the app's files. To share data, both claim
the same [App Group](https://developer.apple.com/documentation/xcode/configuring-app-groups) and
read and write its container:

1. Add the group to the extension: `app-groups = ["group.<app id>"]`.
2. Add the same group to the app's entitlements. On iOS that is the file `[signing.ios]
   entitlements` names, or the `CODE_SIGN_ENTITLEMENTS` in `platform/ios/DayApp.xcconfig`:

   ```xml
   <key>com.apple.security.application-groups</key>
   <array>
       <string>group.dev.example.app</string>
   </array>
   ```

   `day lint` reports an extension group the app does not claim.
3. Write from the app. In Rust, find the container through Foundation (the `objc2-foundation`
   crate, features `NSFileManager`, `NSString` and `NSURL`):

   ```rust
   #[cfg(any(target_os = "ios", target_os = "macos"))]
   fn shared_container(group: &str) -> Option<std::path::PathBuf> {
       use objc2_foundation::{NSFileManager, NSString};
       let url = NSFileManager::defaultManager()
           .containerURLForSecurityApplicationGroupIdentifier(&NSString::from_str(group))?;
       Some(url.path()?.to_string().into())
   }
   ```

4. Read from the extension, in Swift:

   ```swift
   let container = FileManager.default
       .containerURL(forSecurityApplicationGroupIdentifier: "group.dev.example.app")
   ```

A widget shows only what its timeline provider returns. After the app writes new data, ask
WidgetKit to reload: `WidgetCenter` is a Swift API, so call it from Swift the app ships, a
`swift` arm of a `day_bridge::bridge!` block or a file under `[package.metadata.day.ios] swift`.

An extension never links the app's Rust code. The app's library carries the UI toolkit and
`day_main`, and extension points run under tight memory limits (a widget gets about 30 MB) that a
full Day runtime does not fit in. Keep the logic the two share in data, not in code.

## Building

`day build`, `day launch` and `day pack` build every declared extension; there is no separate
step. The Runner target depends on each extension, so Xcode builds them first, and embeds them
with a copy phase that signs each on copy.

Day passes the app's deployment target and macOS entitlements to `xcodebuild` on the command
line, where they would reach every target. When the app has extensions, it passes them as
`SETTING=$(DAY_TARGET_SETTING:default=<app value>)` instead: each extension target defines
`DAY_TARGET_IPHONEOS_DEPLOYMENT_TARGET`, `DAY_TARGET_CODE_SIGN_ENTITLEMENTS` and (on macOS)
`DAY_TARGET_ENABLE_APP_SANDBOX`, and keeps its own value.

The project lives under `build/day/xcode/<os>/` and is regenerated from `Day.toml`. If the app
keeps a checked-in project under `platform/<os>/` (`--persist-native-project`), builds never
change it: `day project add-extension` adds the new target to it once, and `day lint` reports a
declared extension the checked-in project has no target for.

## Signing

**Simulator.** Extensions are signed to run locally, with their entitlements, like the app.

The rest of an app's signing setup is in [Packaging](/docs/how-to/packaging#signing-configuration).

**Device builds** (`day launch -t ios-uikit --ios-device …`). Day builds unsigned and signs
afterwards. Each extension needs an installed development profile for its own bundle id: Day
embeds it, signs the extension with the entitlements Day generated for it (cut down to what the
profile grants), then signs the app.

**App Store builds** (`day pack -t ios-uikit`). Through Xcode's automatic signing, each
extension is provisioned like the app. When Day signs over an installed App Store profile
instead, every extension needs an installed App Store profile for its bundle id too.

**`day sign apply`** signs each extension in a built `.ipa` with its own profile, found in
this order:

1. `--extension-profile <name or bundle id>=<file>` (repeatable).
2. `[signing.ios.extensions.<Name>] profile` in `Day.toml`.
3. The app's profile, when its app id covers the extension's (a wildcard profile).

```toml
[signing.ios.extensions.Widgets]
profile = "${SIGNING_DIR}/Widgets_AppStore.mobileprovision"
# entitlements = "platform/ios/Widgets/Release.entitlements"   # instead of Day's generated file
```

`day sign check` validates these entries.

**macOS.** The Developer ID `.dmg` from `day pack -t macos-appkit` signs each extension with its
sandboxed entitlements and the hardened runtime before the app. A kind that claims a restricted
entitlement (`packet-tunnel`, `app-proxy`, `credential-provider`) cannot be signed ad hoc, so
even a local macOS build needs a team: set `DEVELOPMENT_TEAM` in
`platform/macos/DayApp.local.xcconfig`. `day lint` reports it. `day sign apply` does not sign a
Mac App Store package that embeds extensions yet; archive and export it in Xcode
(`day open -t macos-appkit`).

## What day lint checks

| Code | Finding |
| --- | --- |
| `day::lint::extension` (error) | a declaration that does not resolve, a missing source folder or entitlements file, a deployment target below the kind's floor, an Intents extension with no intent (the system refuses to install the app) |
| `day::lint::extension-incomplete` | an attribute left for you to fill in (the document types a Quick Look or Spotlight extension handles), or a `TRUEPREDICATE` activation rule, which App Review rejects |
| `day::lint::extension-app-group` | an App Group the extension claims and the app does not |
| `day::lint::extension-provisioning` | a macOS extension with a restricted entitlement and no development team |
| `day::lint::extension-unused` | extensions declared in an app with no Apple target |
| `day::lint::xcode-project` (error) | a checked-in project with no target for a declared extension |

## Limits

- **One Messages extension.** An app embeds at most one `imessage` or `sticker-pack` extension;
  the system refuses to install an app with two.
- **Intents need intents.** An `intents` or `intents-ui` extension must list at least one intent
  class in `attributes.IntentsSupported`. `add-extension` writes `INSendMessageIntent` as a
  placeholder.
- **Apps, not pieces.** Only the app's `Day.toml` declares extensions. A piece cannot
  contribute one: project generation reads `Day.toml` alone, so `day lint` and `day prepare
  --check` work offline without resolving the dependency graph, and an extension needs a bundle
  id and a profile per app.
- **iOS and macOS.** watchOS, tvOS and visionOS are not Day targets, so their extensions are not
  either.
- **Swift 6.** Extension targets build in Swift 6 language mode, like the Runner. Set
  `build-settings = { SWIFT_VERSION = "5.0" }` for code that is not ready for strict
  concurrency.
- **MarketplaceKit** runs only in an app that holds the
  [`com.apple.developer.marketplace.app-installation`](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.marketplace.app-installation)
  entitlement, which Apple grants to approved alternative app marketplaces. The extension builds,
  embeds and registers without it, which is enough to develop against.
