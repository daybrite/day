---
title: "Fullscreen cover"
description: "The cover presentation: a fullscreen surface over everything, the system-gesture shield, hiding the status bar, and how each backend presents it."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Fullscreen cover (`cover`) & the system-gesture shield

> **Status: implemented on every backend** (2026-07). Native fullscreen modal on ios-uikit;
> window overlay + slide transition on android-mdc; a topmost full-window child (`Cap::Cover` =
> `Emulated`: no transition, no interactive dismissal) on harmony-arkui, macos-appkit, gtk, qt,
> xaml, and web-dom; probe-visible patches on mock. The emulated tier defaults to an opaque
> theme-background surface so a cover always occludes the window. Exercised end-to-end by Games Fair (a grid home page
> whose tiles present each game fullscreen) and `mock_e2e::cover_presents_lays_out_and_dismisses`.
>
> **macos-appkit (2026-09).** The cover filled the full-size content view, so its content ran
> under the window's title bar. It now starts above the pinned content origin (so its surface
> color still paints edge to edge under the bar) and lays its content out below the bar, the
> size it reports through `FrameChanged` (DESIGN.md §7.7).
>
> **web-dom (2026-10).** A cover presented as a window (`open_window` on the cover tier) whose
> content declares toolbar items gets its own strip docked over the cover
> ([docs/toolbars.md](toolbars.md)); the cover starts below it and reports the shorter frame.
> A navigation stack inside the cover moves its back button and title onto that strip. Without
> items the cover is untouched (Games Fair's game covers).
>
> **Half sheet (2026-10).** `cover(open, build).half_sheet()` presents the bottom half of the
> window with the content above it on show: a medium-detent sheet on iOS, a Material bottom
> sheet on Android, a bottom-anchored half panel on web-dom, HarmonyOS and the emulated
> desktop tiers (macOS keeps its window sheet, at half the window's height). The composed inspector's
> compact home takes it through `Inspector::half_sheet()` ([docs/inspector.md](inspector.md)).
> Verified on iOS, Android and web-dom (Day-Sketch); HarmonyOS, GTK, Qt and XAML carry the
> geometry unverified. Conformance: `cover-half-sheet-presents-over-content`,
> `cover-half-sheet-back-dismisses` and `inspector-half-sheet-on-compact` measure the sheet's
> content against the page on every toolkit and the mock; the Showcase's Stack page demonstrates
> it.
>
> **Modal sheet (2026-10).** Where the toolkit has no native sheet, `cover(open, build).sheet()`
> on a window wider than compact is now a card centered over the dimmed page instead of the
> full-window cover ([Modal sheet](#modal-sheet)). Verified on web-dom and GTK (Stanza Redux's
> reader settings over the book) and on Qt (the conformance case, the page showing through);
> XAML, Android tablets and HarmonyOS carry it unverified.
>
> **gtk (2026-09).** The emulated cover took the root `GtkFixed`'s allocation for its size,
> and that Fixed sits in an External-policy scroll wrapper that allocates it its children's
> bounding box, so a small home page (Games Fair's tile grid) presented a cover the size of
> the grid. The cover now takes the wrapper's allocation, the window content area.

> **Window resize (2026-09-11).** A presented cover's frame is the backend's alone: the core
> never applies the parent's placement to the COVER node (where it sits in the tree it measures
> zero, so that placement is a zero-size rect at the parent's center). Until this landed, any
> relayout of the parent — a window resize on the emulated tiers — collapsed the presented cover
> to a line and showed the page beneath it; Games Fair lost its game on every macOS resize.
> GTK sizes a presenting cover from the window's content area (its live default size minus
> the header bar), the same arithmetic its resize path uses, rather than from an allocation
> read off the widget tree, which followed the page's natural size.

A `cover` presents a Day subtree over the whole window (edge-to-edge, above every other
surface): the SwiftUI `fullScreenCover(item:)` shape. Like `nav` and `nav_stack`
([docs/navigation.md](navigation.md)), it is a projection of an app-owned signal, not an imperative controller:

```rust
let open = Signal::new(None::<Section>);          // Section: any Route type
zstack((
    home_page(open),
    cover(open, |section| game_page(*section))
        .background(|section| section.surface_color()),
))
// present:  open.set(Some(Section::Breakout));
// dismiss:  open.set(None);                       // e.g. from an in-content close button
```

- `Some(r)` builds `build(&r)` under the cover and presents it (slide-up where the platform
  animates modals). `None` dismisses it. Switching directly from `Some(a)` to `Some(b)`
  swaps the content and re-presents.
- `.background(f)` paints the surface color edge-to-edge (under the status bar and home
  indicator) while the content itself is laid out inside the safe area. Without it the
  platform's default surface color shows in the unsafe regions.
- The builder runs inside the presented content's scope: state it restores, signals it
  creates, and cleanups it registers (e.g. a save-on-exit) live exactly as long as the
  presentation.
- The content is disposed only after the backend reports the hide transition finished
  (`Event::CoverHidden` on the cover node), so the surface never blanks mid-animation.
  Scope cleanups (the natural save-on-exit hook) run at that moment.

## The `CoverHidden` delivery contract

App teardown hangs off this one event, so its delivery is a hard guarantee, not a
best-effort animation callback:

- **Backends must deliver it after every dismissal**, even when the platform loses the
  animation completion (UIKit drops transition completions under scripted bursts; Android
  cancels `withEndAction` when an animator is superseded). Both backends pair the normal
  completion with a delayed backstop that reports once the surface has verifiably left the
  screen.
- **The piece treats it as idempotent and orderable**: duplicates are no-ops, and a belated
  report from a previous dismissal cannot dispose content presented since (the closing
  gate). Backends may therefore over-report freely rather than risk under-reporting.
- The mock-toolkit e2e (`cover_cycle_keeps_siblings_alive_and_represents`) pins present →
  dismiss → re-present across double and late reports.

## The shield modifiers

Two `Decorate` modifiers protect a fullscreen interaction (a game, a drawing canvas) from
accidental exits. Both are registration-scoped: they apply while their subtree is mounted
and lift when it unmounts.

```rust
game_page()
    .defers_system_gestures(Edges::ALL)   // SwiftUI defersSystemGestures(on:)
    .interactive_dismiss_disabled()       // SwiftUI interactiveDismissDisabled()
```

- **`defers_system_gestures(edges)`** asks the OS to require a second swipe for its edge
  gestures on the given `Edges` (TOP/BOTTOM/LEADING/TRAILING/ALL). iOS defers the chosen
  screen edges (`preferredScreenEdgesDeferringSystemGestures` on the root and cover view
  controllers); Android enters swipe-to-reveal immersive mode while any request is live;
  desktop backends no-op. day-core keeps the union of all mounted requests and re-sends it
  through the `Toolkit::defer_system_gestures` duty on every change.
- **`interactive_dismiss_disabled()`** blocks the *user-initiated* dismissal of the
  enclosing cover: Android's system back stops closing it (iOS sets
  `isModalInPresentation`, inert under the fullscreen style). Programmatic writes
  (`open.set(None)`, dayscript `nav_back`) still close it; ship an explicit close control.
  The state is queryable via `day_core::shield::dismiss_disabled()` (reactive: reads track
  a change counter).

## Hiding the status bar

`status_bar_hidden(hidden)` hides the system status bar while its subtree is mounted, the
analogue of SwiftUI's `statusBarHidden(_:)`. `hidden` is a `bool`, a `Signal<bool>`, or a
closure, so a setting can switch it while the page stays up:

```rust
reader_page()
    .status_bar_hidden(move || prefs.get().hide_status_bar)
```

The bar is hidden while any mounted request answers `true`, and it returns when the last one
unmounts or answers `false`. A request on a page lower in a navigation stack stays live while
pages are pushed over it, so put the modifier on the page that wants the whole screen: a cover,
a reader, or a game.

| Backend | Mechanism |
|---|---|
| iOS | `prefersStatusBarHidden` on the window's root view controller and on each cover's view controller, updated with a fade |
| Android | `WindowInsetsControllerCompat.hide(statusBars())`; a swipe from the top edge shows the bar briefly |
| HarmonyOS | the window's `setSpecificSystemBarEnabled('status', …)`, called from the ArkTS host |
| desktop, web | nothing to hide; the modifier has no effect |

Check `day_core::capability(Cap::StatusBarHidden)` before offering a setting for it: the three
backends above answer `Native`, and the rest answer `Unsupported`. day-core sends the current
answer through the `Toolkit::set_status_bar_hidden` duty each time a request mounts, unmounts,
or changes, and `day_core::shield::status_bar_hidden()` reads it.

## Routes, dayscript, deep links

A mounted cover registers a string-route adapter ([docs/navigation.md](navigation.md)): `navigate("<key>")`
presents the parsed route, `nav_back()` dismisses, and the presented key is the cover's
contribution to `current_route()`. Games Fair's walkthrough drives games with plain
`- navigate: { route: breakout }` / `- nav_back:` steps.

### `.unrouted()`: a cover that belongs to a control

`cover(open, build).unrouted()` skips that registration: `navigate("<key>")` does not reach it, it
contributes nothing to `current_route()`, and `nav_back()` walks past it. Interactive dismissal is
unaffected; Android's system back arrives as `Event::NavBack` on the cover's own node, which
never went through the adapter.

Reach for it whenever the cover belongs to a **control** rather than to the app: a color picker's
chooser ([docs/colorpicker.md](colorpicker.md)), a scrubber's fullscreen mode. Two things go wrong
otherwise.

The sharper problem is that a routed cover claims route segments through `R::from_key`, and the
untyped route is `String`, whose `from_key` accepts **anything**. So a routed `cover(Signal<Option<String>>, …)`
claims every segment: mount one, and the app's next `navigate("settings")` presents that cover
keyed `"settings"` instead of going to settings. A piece that mounts a cover would be silently
rewriting its host app's navigation, which is what happened to the showcase's walkthrough
the first time the color picker's panel went in. A typed `Route` enum does not have this problem,
because its `from_key` rejects what is not its own; `.unrouted()` is the fix when the key is a
string, and the right answer regardless when the surface is not a destination.

The quieter problem is that `current_route()` naming a transient chooser makes a restored session
reopen it, and a "link me to this screen" URL point at a modal.

## How it works

- `kinds::COVER` is realized detached from the visible hierarchy (its `set_frame` is a
  toolkit no-op: the frame is native-owned). `CoverPatch::Present { background,
  dismiss_disabled }` shows it; `CoverPatch::Dismiss` hides it; `CoverPatch::
  DismissDisabled` tracks the shield while presented.
- Layout follows the nav-page contract: the backend reports the content container's
  safe-area size via `Event::FrameChanged`, and `CoverLayout` (day-core) lays the children
  out at that size. In the tree the cover node measures 0×0, so it never disturbs the
  layout it sits in.
- A native dismissal *request* (Android back) arrives as `Event::NavBack` on the cover
  node; the piece writes `None` into the signal unless dismissal is disabled, the same
  origin-tagged write-back discipline as every control.
- A presented cover is a touch boundary, including blank content and safe-area margins.
  Android's `DayCover.onTouchEvent` consumes events that descendants do not handle, after
  normal child dispatch. Background opacity alone does not stop taps reaching the page below;
  apps do not need dummy gesture handlers on headers or spacers. Hiding the cover restores
  interaction with the underlying page.

## Per-toolkit realization

| Toolkit | Present | Dismiss request | Hidden report |
|---|---|---|---|
| uikit | `DayCoverVC` (fullscreen modal, a form sheet for `.sheet()`, or a page sheet for `.half_sheet()`) over a `DayNavPageView`, through the dialog FIFO | none fullscreen; a sheet's pull-down arrives as `NavBack` from the presentation controller's delegate | dismiss completion block (immediate when the sheet is already down) |
| android | `DayCover` shell re-homed onto the activity content root, slide-up `ValueAnimator` | `OnBackPressedCallback` → `NavBack` | slide-out completion, shell becomes GONE |
| arkui | Stack re-homed into a dedicated full-window NodeContent layer above Navigation and its NavDestinations (no transition) | native Back targets the top cover, honoring the dismissal shield | posted immediately on dismiss |
| mock | patch recorded (`flag` = presented) | tests emit it | tests emit it |
| appkit / gtk / qt / xaml / dom | topmost full-window child of the window content, opaque theme background by default (`Cap::Cover` = `Emulated`, no transition) | none | posted immediately on dismiss |

## Modal sheet

`cover(open, build).sheet().unrouted()` uses the cover's signal, scoped child tree and dismissal
contract for a modal sheet: a data-entry form, or settings shown over the page they change. The
page stays in view behind the sheet but can't be used until it closes. The presentation is
fixed when the piece is built; omit `.sheet()` to keep the fullscreen cover.

| Toolkit | Presentation |
|---|---|
| appkit | a native sheet attached to the primary window, the window less a margin, at most 640 by 620 |
| uikit | `FormSheet` (a narrow modal panel on iPad, adapting to compact windows); its pull-down arrives as the cover's back request, so the bound signal follows what the user did |
| android, arkui, gtk, qt, xaml, dom | composed (`Cap::CoverSheet` = `Unsupported`): on a window wider than compact, the cover turns clear and holds a dimmed fill with a rounded card in the window's own color (`SurfaceRole::Window`), centered at AppKit's proportions; on a compact window, the full-window cover |

The composed card decides compact or wide when the sheet opens, and keeps its place in the
window's center as the window resizes. Its controls are the content's own: give it a way out
(a Done or Cancel button), since there is no title bar to close it from. Android's system back
and HarmonyOS's back gesture still dismiss it, as they do any cover.
Conformance: `cover-sheet-composes-a-card-on-wide` measures the card's content against the page
on every toolkit that composes the sheet, and the mock.

## Half sheet

`cover(open, build).half_sheet()` presents the bottom half of the window and leaves the content
above it on show: an inspector over the canvas it describes, a transport over the media it
controls. It implies `.sheet()`. The cover reports its own content size through `FrameChanged`
as every cover does, so the content lays out inside the sheet.

| Toolkit | Presentation | Dismissal |
|---|---|---|
| uikit | `PageSheet` with the medium and large detents, medium selected and undimmed, so the content underneath stays interactive; a grabber | the pull-down gesture (reported as the cover's back request), programmatic |
| android | the `DayCover` shell as a Material `BottomSheetBehavior` child of a `CoordinatorLayout` over the content root, sized to the bottom half, rounded with a drag handle; the host consumes no touch outside the sheet | pulled down past the hide threshold (the back request), system back, programmatic |
| dom | the cover element anchored to the bottom at half the viewport (`.day-half`), rounded, lifted, with a grabber strip | programmatic |
| arkui, gtk, qt, xaml | the emulated cover placed over the bottom half of its layer or content area | programmatic (Android-style back on HarmonyOS) |
| appkit | the native-sheet tier: a window sheet as for `.sheet()`, standing half the parent window's height | the sheet's own controls, programmatic |

The `interactive_dismiss_disabled` shield holds on every tier that has a gesture. A half sheet
has no toolbar strip of its own on web-dom; put the sheet's controls in its content.
