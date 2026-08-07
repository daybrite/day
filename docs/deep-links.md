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
| harmony-arkui | home skill only; no `uris` yet | env delivery only | — | Planned |
| macos-appkit | `CFBundleURLTypes` (platform/macos scaffold) | — | — | Planned |
| windows-xaml | none | — | — | Planned |
| linux-gtk / linux-qt | none | — | — | Planned |

### iOS — Shipped, two concerns

Intake is `application:openURL:options:` in day-uikit; cold and warm both arrive there and
follow the contract. Concerns:

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

### HarmonyOS — Planned

No deep-link machinery exists yet. The module's `skills` declare only the home entity (no
`uris` entry, so the system cannot deliver a scheme URL), and nothing sets the launch route —
the only path today is the generic env delivery a `day launch --env DAY_DEEPLINK=…` ride-along
provides, which is a dev tool, not OS integration. (A stale comment in the scaffold's
Index.ets still names `DAY_DEMO_ROUTE`, a carrier that no longer exists; it goes when this
lands.) Planned: a `uris` skill with the scheme, `set_launch_deeplink` from the cold `want`,
and `onNewWant` forwarding as the warm path. The `want` parameter machinery is already how the
host passes launch data, so this is wiring, not architecture.

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

## Shortcuts are saved deep links

The persistent icon-menu surfaces (jump lists, home-screen quick actions, `.desktop` actions —
docs/menus.md "Future surfaces") each hold a label, an icon, and a URL of exactly this form.
They add no new delivery machinery; they are declarations that emit these URLs. That is why
this spec comes first.

## Testing — Shipped pieces, plus a dayscript plan

What works today: `DAY_DEEPLINK=route day launch …` exercises the cold path on every desktop
backend and the simulator; `xcrun simctl openurl booted <url>` and
`adb shell am start -a android.intent.action.VIEW -d <url>` exercise real OS delivery on the
mobile targets; on web-dom the URL hash is the whole story and Playwright drives it.

*Planned:* two dayscript tiers, split by what they prove.

1. **`deep_link: { url: "scheme://route?x=1" }`** — an in-process step: the engine parses the
   URL and injects it through the same entry warm delivery uses (`Custom("deeplink")`). It
   proves the app's routing, param handling, and back-stack seeding, identically on every
   backend including mock — cheap enough for every app's walkthrough. It deliberately does
   NOT prove OS registration.
2. **Runner-level OS delivery** — the launch runner sends the URL from outside
   (`simctl openurl`, `am start`, `open <url>`, `hdc … aa start --uri`) and the script then
   asserts with `assert_route`. This proves registration and intake, costs a per-platform
   runner arm, and belongs in the per-target CI jobs rather than every walkthrough.

The split matters because tier 1 failures are app bugs and tier 2 failures are packaging
bugs; a single step that did both would leave every failure ambiguous. `day lint` already
validates literal routes in dayscript files, and the same check extends to `deep_link:` URLs.
