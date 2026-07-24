# Fullscreen cover (`cover`) & the system-gesture shield

> **Status: implemented** on ios-uikit (native fullscreen modal), android-mdc (window
> overlay + slide transition), ohos-arkui (topmost full-window child), and mock (probe-visible
> patches). Desktop backends have no realization yet — `Cap::Cover` answers `Unsupported`
> there and the content never shows. Exercised end-to-end by Day-Games (a grid home page
> whose tiles present each game fullscreen) and `mock_e2e::cover_presents_lays_out_and_dismisses`.

A `cover` presents a Day subtree over the whole window — edge-to-edge, above every other
surface — the SwiftUI `fullScreenCover(item:)` shape. Like `selector` and `stack`
(docs/navigation.md), it is a projection of an app-owned signal, not an imperative controller:

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
- `.background(f)` paints the surface color edge-to-edge — under the status bar and home
  indicator — while the content itself is laid out inside the safe area. Without it the
  platform's default surface color shows in the unsafe regions.
- The builder runs inside the presented content's scope: state it restores, signals it
  creates, and cleanups it registers (e.g. a save-on-exit) live exactly as long as the
  presentation.
- The content is disposed only after the backend reports the hide transition finished
  (`Event::custom("cover-hidden", "")` on the cover node), so the surface never blanks
  mid-animation. Scope cleanups — the natural save-on-exit hook — run at that moment.

### The `cover-hidden` delivery contract

App teardown hangs off this one event, so its delivery is a hard guarantee, not a
best-effort animation callback:

- **Backends MUST deliver it after every dismissal**, even when the platform loses the
  animation completion (UIKit drops transition completions under scripted bursts; Android
  cancels `withEndAction` when an animator is superseded). Both backends pair the normal
  completion with a delayed backstop that reports once the surface has verifiably left the
  screen.
- **The piece treats it as idempotent and orderable**: duplicates are no-ops, and a belated
  report from a PREVIOUS dismissal cannot dispose content presented since (the closing
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
  `isModalInPresentation`, inert under the fullscreen style). Programmatic writes —
  `open.set(None)`, dayscript `nav_back` — still close it; ship an explicit close control.
  The state is queryable via `day_core::shield::dismiss_disabled()` (reactive: reads track
  a change counter).

## Routes, dayscript, deep links

A mounted cover registers a string-route adapter (docs/navigation.md): `navigate("<key>")`
presents the parsed route, `nav_back()` dismisses, and the presented key is the cover's
contribution to `current_route()`. Day-Games' walkthrough drives games with plain
`- navigate: { route: breakout }` / `- nav_back:` steps.

## How it works

- `kinds::COVER` is realized DETACHED from the visible hierarchy (its `set_frame` is a
  toolkit no-op — the frame is native-owned). `CoverPatch::Present { background,
  dismiss_disabled }` shows it; `CoverPatch::Dismiss` hides it; `CoverPatch::
  DismissDisabled` tracks the shield while presented.
- Layout follows the nav-page contract: the backend reports the content container's
  safe-area size via `Event::FrameChanged`, and `CoverLayout` (day-core) lays the children
  out at that size. In the tree the cover node measures 0×0, so it never disturbs the
  layout it sits in.
- A native dismissal *request* (Android back) arrives as `Event::NavBack` on the cover
  node; the piece writes `None` into the signal unless dismissal is disabled — the same
  origin-tagged write-back discipline as every control.

## Per-toolkit realization

| Toolkit | Present | Dismiss request | Hidden report |
|---|---|---|---|
| uikit | `DayCoverVC` (fullscreen modal) over a `DayNavPageView`, through the dialog FIFO | none (fullscreen has no sheet gesture) | dismiss completion block |
| android | `DayCover` shell re-homed onto the activity content root, slide-up `ViewPropertyAnimator` | `OnBackPressedCallback` → `NavBack` | slide-out end action |
| arkui | Stack re-homed onto the window root at full bounds (no transition) | none | posted immediately on dismiss |
| mock | patch recorded (`flag` = presented) | tests emit it | tests emit it |
| appkit / gtk / qt / winui | not realized (`Cap::Cover` = `Unsupported`) | — | — |
