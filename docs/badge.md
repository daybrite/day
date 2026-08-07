# App icon badge (proposed)

> [!NOTE]
> **Status: phase 1 shipped.** `AppBadge`, `Cap::AppBadge{Count,Text,Dot}`, the defaulted
> `Toolkit::set_app_badge` duty, the `day::set_app_badge` facade, and the **AppKit, UIKit, and
> web-dom** arms are implemented; docs/duty-matrix.md and docs/coverage-matrix.md carry the rows.
> Every other backend inherits the default no-op and answers `Unsupported`, which is the honest
> answer for Android (it has no API) and a to-do for Linux, Windows, and HarmonyOS.
>
> The Showcase's Platform services page has an "App badge" group — a stepper, Set/Clear, and a
> macOS-only "Set text" button that appears only where `Cap::AppBadgeText` is `Native`.
>
> One naming decision landed differently from the plan below: the surface is `app_badge`
> throughout (`Toolkit::set_app_badge`, `Cap::AppBadgeCount`, `day::set_app_badge`), to keep it
> clear of `SelectorItem::badge`.

## The recommendation: a Toolkit duty, not a part

**Do not make a `day-part-badge` crate.** Add a defaulted `Toolkit::set_badge` duty plus a small
`day::badge` facade, the way `set_appearance` and `set_app_menu` already work.

Three reasons, in order of weight:

**A badge is app chrome, and app chrome is already a duty.** `set_app_menu`, `set_toolbar`,
`set_window_title`, and `set_appearance` are all Toolkit duties today. A Dock badge sits in exactly
that category — it decorates the running application, it is per-toolkit, and it has no meaning
outside a running app. Parts are for headless OS services that work in a plain `main` with no Day
runtime (docs/clipboard.md is explicit about this); a badge has nothing to say in that context.

**The handle a badge needs is the one the toolkit already holds.** Windows attaches an overlay icon
to an `HWND`. macOS needs `NSApplication`'s dock tile. A part reaching those would need a `day-core`
edge for `WindowHandle` and still could not get at the `HWND` cleanly, which is a heavier dependency
than any existing part takes and buys nothing.

**A defaulted duty costs nothing where the platform cannot do it.** `set_appearance` is implemented
by 4 of 9 backends; the other five inherit `fn set_appearance(&mut self, _dark: Option<bool>) {}`
and answer `Cap::Appearance = Unsupported`. Badge support is at least as uneven, so the same shape
carries it without forcing nine implementations.

The counter-argument, stated fairly: on iOS the badge is part of the notification system
(`UNUserNotificationCenter.setBadgeCount`, gated on the `.badge` authorization option), and on
Android it is *only* reachable through notifications. That is an argument for folding it into
`day-part-local-notify`. It loses because on macOS, Linux, and Windows the badge has no relationship
to notifications at all, and a desktop app wanting a Dock count should not compile alarm receivers
and boot re-arm to get one. The platforms that plumb it through notifications are an implementation
detail a duty can hide — which is the job.

## What each platform will actually accept

This is the part that decides the API, because the payload differs more than the availability does.

| target | count | text | dot | native API |
|---|---|---|---|---|
| macos-appkit | ✓ | **✓** | ✓ | `NSApp.dockTile.badgeLabel` — an arbitrary `String` |
| ios-uikit | ✓ | – | – | `UNUserNotificationCenter.setBadgeCount` (iOS 16+); number only |
| linux-gtk / linux-qt | ✓ | – | ✓ | `com.canonical.Unity.LauncherEntry` D-Bus signal (`count`, `count-visible`) |
| web-dom | ✓ | – | ✓ | `navigator.setAppBadge(n?)` / `clearAppBadge()` |
| windows-xaml | ~ | – | ~ | `ITaskbarList3::SetOverlayIcon` — an **image**, not a number |
| android-mdc | – | – | ~ | none: the launcher derives a dot from posted notifications |
| harmony-arkui | ? | ? | ? | `notificationManager.setBadgeNumber` — likely ArkTS-only, needs investigation |

Four findings worth pulling out of that table:

**macOS is the only platform that takes arbitrary text.** `badgeLabel` is a `String`, so `"beta"` or
`"99+"` render literally. Everywhere else the payload is a number or nothing.

**Android cannot set a badge at all.** There is no AOSP API. The launcher dot is derived from active
notifications, and `Notification.setNumber` is honored only by some launchers. The OEM broadcast
hacks (the ShortcutBadger approach) are per-vendor and routinely break. The honest answer is
`Unsupported` on every badge cap, with the docs pointing at `day-part-local-notify`'s
`Notification::badge(n)` — which is the correct Android path and already ships.

**Windows takes a picture, not a number.** The unpackaged Win32 XAML host can only set an overlay
`HICON`, so a count means rendering digits into an icon at runtime. A packaged MSIX build could use
`BadgeUpdateManager` (1–99 plus a fixed glyph set) instead, but `day pack` also produces an
unpackaged NSIS installer, so the backend cannot assume it. Marked `~`: real work, deferred past v1.

**Linux depends on the shell, not the toolkit.** The Unity launcher protocol is a plain D-Bus signal
naming the app's `.desktop` id, so GTK and Qt behave identically — but KDE Plasma, Dash-to-Dock, and
Docky honor it while stock GNOME Shell ignores it. `Cap` cannot see which shell is running, so this
reports `Emulated`: the call is made and may do nothing.

## The API

A `Badge` value, an imperative setter, and per-payload capabilities.

```rust
use day::badge::{self, Badge};

badge::set(Badge::Count(7));         // the portable case
badge::set(Badge::Text("99+"));      // macOS Dock only — ignored elsewhere
badge::set(Badge::Dot);              // "something is waiting", no number
badge::set(Badge::None);             // clear it
```

```rust
pub enum Badge {
    /// Clear the badge.
    None,
    /// A count. Zero clears, matching every platform's own convention.
    Count(u32),
    /// Short arbitrary text. Only macOS renders it; see `Cap::BadgeText` before using it.
    Text(String),
    /// An indicator with no value.
    Dot,
}
```

**Three capabilities, not one**, following the split `Cap::TextEditable` / `TextSelectable` /
`TextSpellCheck` already uses for exactly this reason — one flag cannot express "counts yes, text
no":

```rust
Cap::BadgeCount    // Badge::Count is honored
Cap::BadgeText     // Badge::Text renders as written (macOS only)
Cap::BadgeDot      // Badge::Dot is honored
// Cap::BadgeImage — reserved, see "deferred" below
```

**Setting is fire-and-forget and never invents a fallback.** `set` returns nothing; an unsupported
payload is ignored, and the app probes the cap first. This mirrors `set_appearance` exactly, whose
own doc says "probe before showing a theme picker — on Unsupported backends the call is ignored."
The alternative — silently degrading `Text("beta")` to `Count(1)` — would put a wrong number on a
user's icon, which is worse than nothing. An app that wants a fallback writes it:

```rust
let b = if capability(Cap::BadgeText) == Support::Native {
    Badge::Text(label)
} else {
    Badge::Count(unread)
};
badge::set(b);
```

### Persistence, which differs and will surprise people

An iOS badge is a property of the installed app and **survives termination** — an app that exits
without clearing leaves a stale number on the home screen, so a `WillTerminate` handler
(docs/lifecycle.md) is usually wanted. A macOS Dock badge dies with the process. The web badge
persists for the installed PWA. This belongs in the doc because it is the one behavior that
silently differs and cannot be probed.

### The iOS permission coupling

`setBadgeCount` needs the `.badge` authorization option, which is part of the notification grant. So
on iOS a badge is invisible until the user has allowed notifications, and the duty should declare
`uses = ["notifications"]` through the same permission machinery `day-part-local-notify` uses
(docs/permissions.md). Two subsystems declaring the same permission is fine — the app grants once.

## A naming collision to resolve first

`badge` is already taken in the piece vocabulary: `SelectorItem::badge` is the count on a **sidebar
row** (`crates/day-pieces/src/nav.rs`), and `Decorate::overlay_aligned`'s docs describe corner
badges. Those are in-window annotations and have nothing to do with the app icon.

Recommendation: name the new surface **`app_badge`** at every layer — `Toolkit::set_app_badge`,
`Cap::AppBadgeCount`, `day::app_badge::set` — so a reader grepping `badge` is never left guessing
which one a call site means. The slightly longer name is worth it.

## Deferred, with the reason

**Custom graphics.** macOS can host an arbitrary view on the Dock tile (`NSDockTile.contentView`)
and Windows overlay icons are images by nature, so a `Badge::Image` is real on two targets. It needs
a per-platform native image type and an encode path that Day does not have at this layer —
`day-piece-remote-image` decodes bytes into a *widget*, which is not the same thing. `Cap::BadgeImage`
is reserved so the enum can grow without a breaking change.

**Progress.** The same Unity D-Bus protocol carries a `progress` double, macOS can draw a progress
bar on the Dock tile, and Windows has `ITaskbarList3::SetProgressValue`. That is a coherent second
feature and should not be smuggled into `Badge`.

## Phasing

1. **`Cap::AppBadge{Count,Text,Dot}` + the defaulted duty + the `day::app_badge` facade + the
   AppKit, UIKit, and web-dom arms.** Those three are small and cover the platforms with a real API:
   `badgeLabel`, `setBadgeCount`, `setAppBadge`. Android answers `Unsupported` from the default and
   its doc points at `Notification::badge`.
2. **Linux**, over the Unity D-Bus signal, reusing the std-only D-Bus approach docs/notify.md
   specifies for `org.freedesktop.Notifications` rather than adding a D-Bus crate. Reports
   `Emulated`, because whether it shows depends on the shell.
3. **HarmonyOS**, once the ArkTS-versus-NDK question is settled, and **Windows**, which needs the
   render-digits-to-an-icon path or a packaged-only implementation.

Phase 1 is the one worth doing on its own: it is three small arms, it needs no new crate, and it
makes the capability honest everywhere else through one defaulted method.

## Verification note

A badge is drawn by the Dock, the launcher, or the home screen — **outside the app's own window**.
`snapshot_window` cannot capture it and a dayscript cannot assert it, exactly like a notification
banner (docs/notify.md). Scripts can assert that `set` was called and what the caps report; that a
number actually appeared on the icon needs a person looking at a device, and the CI gallery should
not imply otherwise.
