# Deep links (OS integration)

How a URL outside the app becomes a `navigate` inside it. This document covers the OS side:
scheme registration, delivery into the process, per-platform capabilities, and testing. The
route grammar, absolute/relative addressing, query params, and how a pending link interacts
with `.restore` are specified in [docs/navigation.md](navigation.md#routes-the-string-route-adapter-deep-links--dayscript)
and are not repeated here.

Each section below is marked **Shipped** or **Planned**. Planned sections are design, not
description; they change with review.

## The URL model — Shipped

```text
<scheme>://<route>[?<params>]          e.g.  fieldnotes://mail/inbox/msg-42?hint=shared
```

The host + path after `://` is the day route string, verbatim. Query params ride through to
`route_param(..)`/`route_params()`. `day new` derives `<scheme>` from the app name (letters
and digits only, lowercased — `Field Notes` ⇒ `fieldnotes`) and writes it into every host
project that registers schemes.

URL parsers treat the first segment as a HOST and lowercase it on the component-based
intakes (NSURL, android.net.Uri), so a deep-linked surface's first-segment keys must be
lowercase — which day's route-key convention already is. Later segments pass through
case-preserved.

Short schemes collide: nothing stops another app from claiming `fieldnotes://`, and the OS
resolves the tie, not day (see the per-platform notes). Apps that care should set an explicit,
longer scheme. *Planned:* a `scheme = "…"` key under `[app]` in Day.toml, conveyed to each
host project the same way the id is; today the scheme is fixed at scaffold time.

## The delivery contract — Shipped

However a link arrives, the behavior inside the app is the same:

- **Cold start** (the link launched the app): the route is recorded before the tree mounts —
  `DAY_DEEPLINK` in the environment where launch environments exist, or
  `day_core::set_launch_deeplink` from the platform entry where they don't — and navigates one
  turn after the first mount. It wins over `.restore`.
- **Warm delivery** (the app was running): the platform layer emits `Custom("deeplink",
  route)` to the active nav host, which navigates immediately.
- **Navigation only.** A scheme URL can be sent by any app or web page, unauthenticated. Deep
  links never execute an action; they only address a surface. Anything destructive must sit
  behind the app's own UI once the user arrives.
- An unknown route falls back exactly like any bad `navigate` call: the unmatched segments are
  dropped and `day lint`'s `unknown-route` check catches literal mistakes at build time.

## Where each platform stands

| Platform | Registration | Cold | Warm | Status |
|---|---|---|---|---|
| ios-uikit | `CFBundleURLTypes` (scaffold) | ✓ | ✓ `application:openURL:options:` | Shipped |
| android-mdc | `intent-filter` VIEW+BROWSABLE (scaffold), `singleTask` | ✓ | ✓ `onNewIntent` → kind 7 | Shipped |
| web-dom | the page URL is the link | ✓ hash/`?route=` | ✓ `RouteRequested` on hash change | Shipped |
| harmony-arkui | `uris` skill (scaffold) | ✓ `want.uri` → buffered | ✓ `onNewWant` | Shipped |
| macos-appkit | `CFBundleURLTypes` (platform/macos scaffold) | — | — | Planned |
| windows-xaml | none | — | — | Planned |
| linux-gtk / linux-qt | none | — | — | Planned |

### iOS — Shipped, two concerns

Under the scene lifecycle the app runs, URLs arrive at the scene delegate, and that is where
day-uikit takes them: cold from the connection options' `URLContexts` (and a quick action's
`shortcutItem`), warm from `scene:openURLContexts:` (and
`windowScene:performActionForShortcutItem:`), every arm one call into
`day_core::request_route`. The app-delegate `application:openURL:options:` intake remains for
the pre-scene path. One lesson is encoded in the code rather than repeated: `URLContexts` is
declared non-null but a plain launch returns nil, so the cold arm reads it through a
nullable send — the strict binding panicked on every ordinary launch until the dayscript
walkthrough caught it. Concerns:

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
   hosting story as iOS) bypass it for `https` links.
2. **Intent extras are not the URL.** Only the `data` URI is treated as a link; anything else
   in the intent is ignored by design — see the security note above.

### HarmonyOS — Shipped

The module's `skills` declare a `uris` entry with the app scheme, and both temperatures are
one call: the ArkTS host forwards a cold `want.uri` (in `onCreate`, before `start()`) and a
warm `onNewWant` one to the shim's `deepLink(uri)`, which lands in `day_core::request_route`
— buffered until the first mount, applied on the UI thread after it. A want with no URI
carries `want.uri` as an EMPTY string, not undefined, so the ability also reads the
`parameters["day.uri"]` fallback a `[[shortcuts]]` want uses through `||`, never `??`. Verified on the Oniro
emulator with `aa start -U "<scheme>://<route>"`, cold and warm. One concern: `aa start -U`
is also the only local delivery tool — there is no system browser in the emulator image to
exercise link-from-a-page flows.

### macOS — Planned, with one structural caveat

Registration already ships in the platform/macos scaffold's Info.plist. Intake does not: the
runtime needs the Apple Event handler (`kAEGetURL`) registered at startup, emitting the same
cold/warm paths as iOS. Two macOS-specific concerns:

1. **Only the Xcode-built `.app` can receive links.** Launch Services registers bundles, not
   bare binaries — the `DAY_MACOS_XCODE=0` cargo path runs an unbundled executable that no
   scheme can reach. Dev-loop link testing therefore requires the bundle path (the default
   where the scaffold exists).
2. **Stale copies shadow each other.** Launch Services indexes every copy of the bundle it
   has seen — a Debug build in `build/day/`, a packed copy in `/Applications` — and picks one
   by its own rules. During development, `open <url>` may target a copy other than the one
   just built. Worth a `day doctor` note when the handler lands.

### Windows — Planned, the largest lift

Nothing ships. The pieces, in order of effort:

1. **Packaged (MSIX):** a `uap:Protocol` extension in the generated manifest — small, and the
   store-grade path.
2. **Unpackaged (dev + NSIS):** protocol registration is an `HKCU` registry ProgId written at
   install or first run; `day launch` dev builds would self-register on start, which is a
   machine-state write day currently never does. This deserves an explicit opt-in.
3. **Single-instance forwarding** (below) is not optional here: without it every link spawns
   another copy of the app. Packaged apps can use `AppInstance` redirection; unpackaged needs
   a named mutex + pipe in the C++/WinRT shim. This is the bulk of the work.

### Linux — Planned, with one honest limit

Registration is two lines in the generated `.desktop` file (`MimeType=x-scheme-handler/<scheme>;`
plus `DBusActivatable=true`), and both packers already generate that file. Delivery concerns:

1. **AppImage integration is opt-in by the user.** An AppImage registers no `.desktop` entry
   unless the user runs an integration tool, so scheme links reaching an AppImage build are
   not dependable. Flatpak installs integrate normally and are the reliable path.
2. **Dev launches cannot receive links** for the same reason as macOS's cargo path: there is
   no installed `.desktop` pointing at the build tree. Testing needs an installed build, or
   the dayscript tier below.
3. Warm delivery should ride DBus activation (`org.freedesktop.Application.Open`), which both
   desktops' launchers use when `DBusActivatable` is set; a fallback single-instance socket
   would cover launchers that exec directly.

## Single-instance forwarding — Planned

Desktop platforms need a policy for a link arriving while the app runs, or arriving twice.
macOS forwards through Launch Services automatically. Linux gets it from DBus activation.
Windows must build it (above). The contract in all three cases is the same: the second
invocation hands its URL to the running instance and exits; the running instance treats it as
a warm link. day should own this in the platform layer so apps never see two processes.

## Shortcuts are saved deep links — Shipped (ios / android / harmony)

The persistent icon-menu surfaces (home-screen quick actions, launcher shortcuts, jump lists,
`.desktop` actions — docs/menus.md "Future surfaces") each hold a label and a URL of exactly
this form. They add no new delivery machinery; they are declarations that emit these URLs
into the intake above.

Declared in Day.toml, in display order:

```toml
[[shortcuts]]
route = "menus"        # the route the shortcut opens; query params allowed
label = "nav_menus"    # a Fluent message id from resource/locales/
```

`day build` resolves each label in **every** locale (a missing translation, a multi-line
message, or a placeable is a build error — the native launcher renders the conveyed string
with no formatter behind it) and writes the platform's native declaration:

- **Android** — nothing is committed: `res/xml/day_shortcuts.xml` plus per-locale string
  resources are staged into `build/day/android/res` (already a scaffold res srcDir), and the
  `<meta-data android:name="android.app.shortcuts">` rides the day-pieces overlay manifest,
  merged into the launcher activity by name. The shortcut intent is VIEW + the URL, so
  activation IS the shipped intent-filter intake. Verified on the emulator: the shortcut
  service parses both demo shortcuts with locale-resolved labels (`dumpsys shortcut`), and
  the declared intent cold-launches onto the right page.
- **iOS** — `UIApplicationShortcutItems` is written into the committed `Info.plist` (the same
  managed-key editor as the permission strings), titled with the default-locale text; the
  scaffold's `Stage Day Strings` script phase (`day xcode-backend stage-strings`, injected
  into pre-existing scaffolds on first use) stages `<locale>.lproj/InfoPlist.strings` into
  the built bundle, keyed by that default text so an unlocalized device falls back to
  readable English. A quick action's type string is the URL itself; the scene delegate feeds
  it — warm via `performActionForShortcutItem`, cold via the connection options — into
  `day_core::request_route`. Conveyance is verified in the built bundle; the tap itself
  cannot be automated on a simulator (no touch injection, and `simctl openurl` sits behind a
  confirmation dialog), so OS-delivered activation rides on the same rail the walkthrough
  and the other platforms prove.
- **HarmonyOS** — a generated `$profile:shortcuts_config` plus an `ohos.ability.shortcuts`
  metadata entry on the ability, labels merged into each locale's `string.json`
  (`day_shortcut_` prefix is the ownership marker, `base/` carries the default locale). The
  want carries the URL in `parameters["day.uri"]`; EntryAbility forwards it through the same
  `deepLink` call a `uris` launch uses. Verified cold and warm on the Oniro emulator with
  the exact want the profile declares (`aa start … --ps day.uri <url>`); the emulator's
  stock launcher renders no shortcut panel for ANY app, so the panel UI itself needs real
  hardware.
- **macOS / Windows / Linux** — not yet: their surfaces (dock menu, jump list, `.desktop`
  Actions) stay gated on the missing intake above. Declaring `[[shortcuts]]` today conveys
  nothing there and breaks nothing.

Limits worth knowing: launchers show at most about four entries (`day lint` warns past
four); per-shortcut icons are not conveyed yet (the platforms render their default glyph);
and on OpenHarmony `want.uri` arrives as an empty string on non-URI launches, which is why
the ability checks `want.uri || parameters["day.uri"]`, not `??`.

## Testing — Shipped pieces, plus a dayscript plan

What works today: the dayscript step below on every backend; `DAY_DEEPLINK=route day
launch …` for the cold env path; `xcrun simctl openurl booted <url>`,
`adb shell am start -a android.intent.action.VIEW -d <url>`, and
`hdc shell aa start -U <url>` for real OS delivery on the mobile targets; on web-dom the URL
hash is the whole story and Playwright drives it. A HarmonyOS launcher-shortcut tap is
simulated exactly by `aa start -b <bundle> -a EntryAbility --ps day.uri <url>` — the same
want the generated profile declares. On an iOS 16/17 simulator `simctl openurl` sits behind
an "Open in …?" confirmation SpringBoard shows headlessly, so automated openurl runs need a
tap the simulator cannot inject; a device, or an XCUITest runner, gets past it.

**`deep_link: { url: "scheme://route?x=1" }` — Shipped.** An in-process step: the URL maps to
its route through the same `day_spec::route_of_url` every platform intake uses, then
navigates — a warm delivery minus the OS. It proves the app's routing, param handling, and
back-stack seeding identically on every backend including mock — cheap enough for every
app's walkthrough — and deliberately does NOT prove OS registration. `day lint` validates
the URL's route against the app's declared keys, the same check `navigate:` gets.

*Planned:* the second tier — the launch runner delivering the URL from outside (the commands
above) with the script asserting via `assert_route`. That proves registration and intake,
costs a per-platform runner arm, and belongs in the per-target CI jobs rather than every
walkthrough. The split matters because tier-1 failures are app bugs and tier-2 failures are
packaging bugs; a single step doing both would leave every failure ambiguous.
