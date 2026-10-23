---
title: "Deep links"
description: "Custom URL schemes end to end: registration, cold and warm delivery, routing, and launcher shortcuts as saved deep links."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Deep links

How a URL outside the app becomes a [`navigate`](https://daybrite.dev/api/day_pieces/fn.navigate.html) inside it. This document covers the OS side:
scheme registration, delivery into the process, per-platform capabilities, and testing. The
route grammar, absolute/relative addressing, query params, and how a pending link interacts
with [`.restore`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.restore) are specified in [docs/navigation](navigation.md#routes-the-string-route-adapter-deep-links--dayscript)
and are not repeated here.

Platform execution claims below are retained records and were not rerun during restructuring.

## The URL model — Shipped

```text
<scheme>://<route>[?<params>]          e.g.  fieldnotes://mail/inbox/msg-42?hint=shared
```

The host + path after `://` is the day route string, verbatim. Query params ride through to
[`route_param(..)`](https://daybrite.dev/api/day_pieces/fn.route_param.html)/[`route_params()`](https://daybrite.dev/api/day_pieces/fn.route_params.html). `day new` derives `<scheme>` from the app name (letters
and digits only, lowercased: `Field Notes` ⇒ `fieldnotes`) and writes it into every host
project that registers schemes.

URL parsers treat the first segment as a host and lowercase it on the component-based
intakes (NSURL, android.net.Uri), so a deep-linked surface's first-segment keys must be
lowercase, which day's route-key convention already requires. Later segments pass through
case-preserved.

Short schemes collide: nothing stops another app from claiming `fieldnotes://`, and the OS
resolves the tie (see the per-platform notes). Apps that care should set an explicit,
longer scheme.

## The delivery contract — Shipped

However a link arrives, the behavior inside the app is the same:

- **Cold start** (the link launched the app): the route is recorded before the tree mounts
  (`DAY_DEEPLINK` in the environment where launch environments exist, or
  [`day_core::set_launch_deeplink`](https://daybrite.dev/api/day_core/fn.set_launch_deeplink.html) from the platform entry where they don't) and navigates one
  turn after the first mount. It wins over [`.restore`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.restore).
- **Warm delivery** (the app was running): the platform layer emits `RouteRequested(route)`
  to the active nav host, which navigates immediately.
- **Handlers stack.** [`day::on_open_url`](https://daybrite.dev/api/day/fn.on_open_url.html) registrations stack, newest first: the newest live
  handler sees a URL first, and a handler that returns `false` passes it on. A part that waits for
  one callback URL for the life of one operation (the sign-in redirect in
  [day-part-auth](/docs/reference/auth)) registers in a scope of its own,
  takes the one URL it expects, and declines everything else, so the app's root handler keeps
  receiving the rest. A registration goes with its scope.
- **Navigation only.** A scheme URL can be sent by any app or web page, unauthenticated. Deep
  links never execute an action; they only address a surface. Anything destructive must sit
  behind the app's own UI once the user arrives.
- An unknown route falls back exactly like any bad [`navigate`](https://daybrite.dev/api/day_pieces/fn.navigate.html) call: the unmatched segments are
  dropped and `day lint`'s `unknown-route` check catches literal mistakes at build time.

## Where each platform stands

Platform guidance: [Apple: custom URL schemes](https://developer.apple.com/documentation/xcode/defining-a-custom-url-scheme-for-your-app); [Android: deep links](https://developer.android.com/training/app-links/deep-linking).

| Platform | Registration | Cold | Warm | Status |
|---|---|---|---|---|
| ios-uikit | [`CFBundleURLTypes`](https://developer.apple.com/documentation/bundleresources/information-property-list/cfbundleurltypes) (scaffold) | ✓ | ✓ `application:openURL:options:` | Shipped |
| android-mdc | `intent-filter` VIEW+BROWSABLE (scaffold), `singleTask` | ✓ | ✓ `onNewIntent` → kind 7 | Shipped |
| web-dom | the page URL is the link | ✓ hash/`?route=` | ✓ `RouteRequested` on hash change | Shipped |
| harmony-arkui | `uris` skill (scaffold) | ✓ `want.uri` → buffered | ✓ `onNewWant` | Shipped |
| macos-appkit | [`CFBundleURLTypes`](https://developer.apple.com/documentation/bundleresources/information-property-list/cfbundleurltypes) (platform/macos scaffold) | ✓ `application:openURLs:` | ✓ the same | Shipped |
| windows-winui | Day.toml `url_schemes` (MSIX, NSIS); `--day-open-url <link>` on the command line | ✓ | ✓ single-instance forwarding | Compile-checked |
| linux-gtk / linux-qt | Day.toml `url_schemes` (`.desktop` `x-scheme-handler`); `--day-open-url <link>` | ✓ | ✓ single-instance forwarding | Shipped |

### iOS — Shipped, two concerns

Under the scene lifecycle the app runs, URLs arrive at the scene delegate, and that is where
day-uikit takes them: cold from the connection options' `URLContexts` (and a quick action's
`shortcutItem`), warm from `scene:openURLContexts:` (and
`windowScene:performActionForShortcutItem:`), every arm one call into
[`day_core::request_route`](https://daybrite.dev/api/day_core/fn.request_route.html). The app-delegate `application:openURL:options:` intake remains for
the pre-scene path. `URLContexts` is declared non-null but a plain launch returns nil, so the
cold arm reads it through a nullable send; the strict binding panicked on every ordinary
launch until the dayscript walkthrough caught it. Concerns:

1. **Scheme exclusivity does not exist.** If two installed apps claim one scheme, iOS picks
   one, silently. The fix at the platform level is Universal Links (below).
2. **Universal Links are a separate tier.** They need an `applinks:` entitlement, a team id in
   signing config, and an `apple-app-site-association` file served from the app's domain. The
   natural host for that file is the daysite deployment the app already publishes; `day pack`
   knows the signing config and daysite knows the domain, so the pieces exist. Not started.

### Android — Shipped, two concerns

The scaffold's manifest registers the scheme with `BROWSABLE` (so links in a browser work) and
`launchMode="singleTask"`, which is what routes a warm link through `onNewIntent` instead of
stacking a second activity. DayActivity forwards both cold and warm intents as deep links.
Concerns:

1. **The chooser dialog.** Two apps claiming one scheme puts a disambiguation sheet in front
   of the user. Verified App Links (an `assetlinks.json` on the app's domain, same daysite
   hosting requirements as iOS) bypass it for `https` links.
2. **Intent extras are not the URL.** Only the `data` URI is treated as a link; anything else
   in the intent is ignored, per the delivery contract above.

### HarmonyOS — Shipped

The module's `skills` declare a `uris` entry with the app scheme, and both temperatures are
one call: the ArkTS host forwards a cold `want.uri` (in `onCreate`, before `start()`) and a
warm `onNewWant` one to the shim's `deepLink(uri)`, which lands in [`day_core::request_route`](https://daybrite.dev/api/day_core/fn.request_route.html)
(buffered until the first mount, applied on the UI thread after it). A want with no URI
carries `want.uri` as an empty string rather than undefined, so the ability also reads the
`parameters["day.uri"]` fallback a `[[shortcuts]]` want uses through `||`, never `??`. Verified on the Oniro
emulator with `aa start -U "<scheme>://<route>"`, cold and warm. One concern is that `aa start
-U` is also the only local delivery tool, since the emulator image has no system browser to
exercise link-from-a-page flows.

## Single-instance forwarding

A second launch of a desktop app hands its command line (`--day-open-url`, `--day-open-file`,
files, URLs) to the copy already running and exits; the running copy receives them as a warm
link or file and brings its front window forward ([`day_core::forward_launch_args`](https://daybrite.dev/api/day_core/documents/fn.forward_launch_args.html)). Apps never
see two processes.

| Toolkit | Mechanism | Status |
|---|---|---|
| AppKit | Launch Services activates the running app and delivers URLs and files to its delegate | Shipped |
| GTK | GApplication's own uniqueness: the second launch's files and URIs reach the first through its `open` signal | Shipped |
| Qt (Linux) | `day_dbus::instance`: the first launch owns the app id on the session bus, a second calls its `Forward` method | Shipped |
| Windows XAML | A named mutex per app id; a second launch sends its arguments to the first with `WM_COPYDATA` | Compile-checked; runtime pending |

Two development builds of one app id forward to each other, whichever started first.

## Launcher shortcuts

The persistent icon-menu surfaces (home-screen quick actions, launcher shortcuts, jump lists,
`.desktop` actions, the macOS Dock menu) each hold a label and a URL of exactly
this form. They add no new delivery machinery; they are declarations that emit these URLs
into the intake above.

Declared in Day.toml, in display order:

```toml
[[shortcuts]]
route = "menus"        # the route the shortcut opens; query params allowed
label = "nav_menus"    # a Fluent message id from resource/locales/
```

`day build` resolves each label in **every** locale (a missing translation, a multi-line
message, or a placeable is a build error, because the native launcher renders the conveyed
string with no formatter behind it) and writes the platform's native declaration:

- **Android** — nothing is committed: `res/xml/day_shortcuts.xml` plus per-locale string
  resources are staged into `build/day/android/res` (already a scaffold res srcDir), and the
  `<meta-data android:name="android.app.shortcuts">` rides the day-pieces overlay manifest,
  merged into the launcher activity by name. The shortcut intent is VIEW + the URL, so
  activation goes through the shipped intent-filter intake. Verified on the emulator: the shortcut
  service parses both demo shortcuts with locale-resolved labels (`dumpsys shortcut`), and
  the declared intent cold-launches onto the right page.
- **iOS** — [`UIApplicationShortcutItems`](https://developer.apple.com/documentation/bundleresources/information-property-list/uiapplicationshortcutitems) is written into the committed `Info.plist` (the same
  managed-key editor as the permission strings), titled with the default-locale text; the
  scaffold's `Stage Day Strings` script phase (`day xcode-backend stage-strings`, injected
  into pre-existing scaffolds on first use) stages `<locale>.lproj/InfoPlist.strings` into
  the built bundle, keyed by that default text so an unlocalized device falls back to
  readable English. The default locale gets its own `.lproj` too: those dirs are the
  bundle's localization list, and iOS 15 resolves a device language outside the list to the
  first entry, not to [`CFBundleDevelopmentRegion`](https://developer.apple.com/documentation/bundleresources/information-property-list/cfbundledevelopmentregion). A quick action's type string is the URL itself; the scene delegate feeds
  it into [`day_core::request_route`](https://daybrite.dev/api/day_core/fn.request_route.html), warm via `performActionForShortcutItem` and cold via the
  connection options. Conveyance is verified in the built bundle; the tap itself cannot be
  automated on a simulator (no touch injection, and `simctl openurl` sits behind a
  confirmation dialog), so OS-delivered activation relies on the same intake path the
  walkthrough and the other platforms exercise.
- **HarmonyOS** — a generated `$profile:shortcuts_config` plus an `ohos.ability.shortcuts`
  metadata entry on the ability, labels merged into each locale's `string.json` in the
  staged host under `build/day/harmony/project/`
  (`day_shortcut_` prefix is the ownership marker, `base/` carries the default locale). The
  want carries the URL in `parameters["day.uri"]`; EntryAbility forwards it through the same
  `deepLink` call a `uris` launch uses. Verified cold and warm on the Oniro emulator with
  the exact want the profile declares (`aa start … --ps day.uri <url>`); the emulator's
  stock launcher renders no shortcut panel for any app, so the panel UI itself needs real
  hardware.
- **macOS** — the Dock menu lists the shortcuts above the app's own [`dock_menu`](https://daybrite.dev/api/day_pieces/fn.dock_menu.html) entries
  ([docs/menus](menus.md#dock-menu)). `day-build` registers the `[[shortcuts]]` into the
  binary at link time, so the menu is built while the app runs, in its current language, and
  choosing an entry opens its route in the running app.
- **Windows** — the jump list's Tasks: each entry runs the app with
  `--day-open-url "<scheme>://<route>"`, which single-instance forwarding hands to the copy
  already running. Built at startup and rebuilt on every [`set_launcher_shortcuts`](https://daybrite.dev/api/day/fn.set_launcher_shortcuts.html).
- **Linux** — `Actions=` in the generated `.desktop` file, one `[Desktop Action …]` group per
  shortcut with a `Name[<locale>]` for every locale and the same `--day-open-url` command line.
- **Web** — the manifest's `shortcuts` member, opening `./#<route>`, in the default locale.

### Shortcuts set while the app runs

[`day::set_launcher_shortcuts(vec![LauncherShortcut { route, label }])`](https://daybrite.dev/api/day/fn.set_launcher_shortcuts.html) replaces the app's
run-time shortcuts: "Resume <last document>", the three most recent chats. They show beside the
declared ones as iOS Home Screen quick actions, Android dynamic shortcuts (API 25+), Windows
jump-list tasks and macOS Dock menu items (`Cap::DynamicShortcuts`). Pass labels already
localized. Linux launchers and the web keep only the declared shortcuts.

Launchers show at most about four entries (`day lint` warns past four), per-shortcut icons
are not conveyed yet (the platforms render their default glyph), and on OpenHarmony
`want.uri` arrives as an empty string on non-URI launches, which is why the ability checks
`want.uri || parameters["day.uri"]` rather than `??`.

## Testing shipped pieces

What works today: the dayscript step below on every toolkit; `DAY_DEEPLINK=route day
launch …` for the cold env path; `xcrun simctl openurl booted <url>`,
`adb shell am start -a android.intent.action.VIEW -d <url>`, and
`hdc shell aa start -U <url>` for real OS delivery on the mobile targets; on web-dom the URL
hash is the entire intake and Playwright drives it. A HarmonyOS launcher-shortcut tap is
simulated exactly by `aa start -b <bundle> -a EntryAbility --ps day.uri <url>`, the same
want the generated profile declares. On an iOS 16/17 simulator `simctl openurl` sits behind
an "Open in …?" confirmation SpringBoard shows headlessly, so automated openurl runs need a
tap the simulator cannot inject; a device, or an XCUITest runner, gets past it.

**`deep_link: { url: "scheme://route?x=1" }` — Shipped.** An in-process step: the URL maps to
its route through the same [`day_spec::route_of_url`](https://daybrite.dev/api/day_spec/fn.route_of_url.html) every platform intake uses, then
navigates: a warm delivery without the OS. It proves the app's routing, param handling, and
back-stack seeding identically on every toolkit including mock (cheap enough for every
app's walkthrough), and it does not prove OS registration. `day lint` validates the URL's
route against the app's declared keys, the same check `navigate:` gets.



## External URL handlers

Declare additional OS associations at the root of Day.toml:

```toml
url_schemes = ["feed", "web+feed"]
```

Register [`day::on_open_url(|url| { /* show validation UI */ true })`](https://daybrite.dev/api/day/fn.on_open_url.html) in the app root.
Handlers stack: the newest registration is asked first, and a URL it declines passes to the
earlier ones, so a part awaiting a callback URL (such as a sign-in redirect)
registers for the duration of its own scope without
displacing the app's handler; each registration leaves the stack with the scope it was made in.
The callback receives the complete original URL on the UI thread, including its scheme and query.
Return false to retain existing route handling. Each registration's scope owns its cleanup. Cold-start delivery occurs after mounting. Use `DAY_OPEN_URL` or the
`open_url: { url: ... }` dayscript step to exercise this rail without OS registration.

Apple bundles, Android/Harmony manifests, Linux desktop entries, Windows MSIX and NSIS packages
advertise protocol candidates. Windows development builds emit an opt-in registration file.
PWA manifests include only valid `web+` names. Registration doesn't force the user's default or
make every HTTPS link leave a browser; MIME handling depends on browser/platform policy.
