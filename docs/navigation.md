# Navigation in day (`nav()`)

Native-first navigation (extends DESIGN.md §9/§10, resolves DP-23's "native navigation
containers" direction). One declarative API; each toolkit maps it onto its own idiom.

## Model

A **route** is a string path (`"controls"`, `"about"`). Routes unify three consumers:

1. **Declarative links** — `nav_link("Controls", "controls")` pushes a registered route.
2. **dayscript** — `- navigate: { route: controls }`, `- nav_back:`, `- assert_route: …`.
3. **Deep links** — platform URLs (`dayshowcase://controls`) resolve to the same route
   strings; every platform also honors a `DAY_DEEPLINK` env var at startup (uniform,
   headless-testable).

The nav host owns a route stack `Signal<Vec<Route>>`. Pushing builds the destination
lazily in its own reactive `Scope` (the `when` pattern); popping disposes it. All
dynamism flows through the ordinary bind/patch path — build-once holds.

```rust
nav("Day Showcase", home_page())            // root title + root/sidebar content
    .route("controls", tr("nav-controls"), || controls_page())
    .route("about",    tr("nav-about"),    || about_page())
    .id("nav")
```

`navigator().push("about")` / `navigator().pop()` for programmatic use.

`nav_menu()` renders the route table as a NATIVE navigation list inside the root
content: NSOutlineView source list (in an `NSVisualEffectView` sidebar) on macOS,
`navigation-sidebar` GtkListBox on GTK, a sidebar-styled QListWidget on Qt,
inset-grouped `UITableView` rows with disclosure chevrons on iOS, and ripple list rows
on Android. Selection navigates; the active route highlights (split presentation), and
split hosts auto-select the first route at launch — desktop sidebars don't present
empty detail panes.
**v1 constraint: `nav()` must be the app root** (matches `NavigationStack` at scene
root; iOS installs a window-level `UINavigationController`). Non-root hosts degrade to
a plain container with a warning.

## Presentation per toolkit

| Toolkit | Host | Idiom |
|---|---|---|
| uikit | `UINavigationController` (window root VC) | native push/pop animation, native back button + swipe, per-VC `title` |
| android | `DayNavHost` (LinearLayout: `android.widget.Toolbar` + page `FrameLayout`) | slide animation, toolbar title, toolbar up-arrow + system back |
| appkit | `NSSplitView` (sidebar + detail panes) | SwiftUI `NavigationSplitView` reading: root content = sidebar, active route renders in detail; day-drawn detail header (title + back at depth > 1) |
| gtk | `GtkPaned` | same split reading |
| qt | `QSplitter` (via shim) | same split reading |
| mock | recorded containers | drives core tests |
| winui | container fallback (`_ =>` arm) | nav UI TODO (needs a Windows host to build) |

Split-pane sizing: pane containers report their allocated size via
`Event::FrameChanged` (the canvas mechanism); `NavLayout` lays sidebar/page subtrees
out inside the last-reported pane sizes. Mobile pages report their usable size
(under bars / safe areas) the same way.

## Wire surface (day-spec)

- Kinds: `kinds::NAV` (host), `kinds::NAV_PAGE` (one destination's native container).
- `NavProps { title, split }` — `split` chosen by the pieces layer from the toolkit's
  capability (`Cap::NavSplit` → `Support::Native` on desktop backends).
- `NavPagePatch::Pushed { title }` / `NavPatch::Popped` — applied to the HOST after the
  page child is attached/before it is removed; toolkits animate accordingly.
- `Event::NavBack` — native back (iOS back button/swipe, Android system back or toolbar
  up). The host piece pops the stack **without re-issuing a native pop** when the
  toolkit reports the pop already happened natively (`Event::NavBack` carries
  `already_popped: bool` — the TextField `from_native` echo pattern).

The host's `NodeProbe.text` carries the current route path (dayscript `assert_route`).

## Back-sync invariant

Exactly one of {day, toolkit} initiates any pop; the other follows:

- day-initiated (`nav_back` step, `navigator().pop()`, desktop header back):
  stack pops → `NavPatch::Popped` → toolkit pops natively.
- native-initiated (iOS back button/gesture, Android system back): toolkit emits
  `NavBack { already_popped }` → host pops the stack, skipping the patch when
  `already_popped` (iOS pops itself; Android reports `already_popped: false` and
  waits for the patch).

## Deep links

- **startup (all platforms):** `DAY_DEEPLINK=<route>` env → after mount + first layout,
  `day_core::navigate(route)`.
- **iOS:** `CFBundleURLTypes` scheme in the Runner Info.plist;
  `application:openURL:options:` → route = URL host + path → `navigate`.
  Test: `xcrun simctl openurl booted dayshowcase://about`.
- **Android:** `intent-filter` (VIEW/BROWSABLE/`dayshowcase` scheme) +
  `launchMode="singleTask"`; cold start passes the route in the env blob as
  `DAY_DEEPLINK`; warm start `onNewIntent` → `nativeOnEvent` kind 5 → `navigate`.
  Test: `adb shell am start -a android.intent.action.VIEW -d dayshowcase://about`.
- **Desktop:** `DAY_DEEPLINK` env (`day launch --env DAY_DEEPLINK=about`); OS-level
  scheme registration (Info.plist in packed .app / `.desktop` files) is a `day pack`
  concern, post-v1.

## Out of scope (v1)

Route arguments/pattern matching, nested nav hosts, `GtkHeaderBar`/native desktop
titlebars, androidx `NavController`, winui nav UI, state restoration.
