# Tabs (`selector` with `SelectorStyle::Tabs`)

> **Note (migration):** tabs are now `selector(sel).style(SelectorStyle::Tabs)` — a one-of-N
> selector bound to a `Signal<String>` of the active tab key (docs/navigation.md). The prose
> below describes the tab semantics and native mapping, which are unchanged; the standalone
> `tabs()` builder was folded into `selector`.

---


A `tabs()` host is a native tabbed container: several keyed destinations, one visible at a
time, switched by a native tab widget. It reuses the same route registry as `nav()`
(docs/navigation.md), so a tab key *is* a route — you select tabs, deep-link to them, and
drive/assert them from dayscript exactly the way you navigate.

```rust
tabs()
    .tab("overview", tr("overview"), overview_page)
    .tab("details",  tr("details"),  details_page)
    .tab("settings", tr("settings"), settings_page)
    .id("main-tabs")
```

`navigate("settings")` selects the settings tab; a deep link to `settings` lands on it;
dayscript `navigate {route: settings}` / `assert_route {route: settings}` drive and check it.

## Semantics

- **Keyed destinations.** Each `.tab(key, title, build)` is addressed by `key`. `title` is the
  tab label; `build` runs **once** at mount.
- **All pages resident.** Every tab's content is built eagerly and kept alive, so each tab
  preserves its own state across switches — the behaviour of every native tab container.
- **`.selected(key)`** picks the initial tab (default: the first). Startup deep links still win.
- **Nesting & fall-through.** Hosts register on a stack (docs/navigation.md). `tabs()` inside a
  `nav()` route registers *on top*: `navigate("<tab-key>")` selects the tab, while
  `navigate("<some-nav-route>")` — a key the tabs host doesn't know — falls through to the
  enclosing `nav()`, which replaces the page (disposing the tabs host, whose scope cleanup
  unregisters its controller). `current_route()` reports the innermost host — the active tab.

## The wire (spec)

`day_spec`:

- `kinds::TABS` (host) and `kinds::TABS_PAGE` (one tab's content container; its frame is
  native-owned, like a nav page).
- `props::TabsProps { titles: Vec<String>, selected: usize }`; `TabsPatch::Selected(usize)`
  (programmatic sync, applied without echoing a `SelectionChanged` back — the from-native rule).
- `props::TabsPageProps { title }` — the page's tab label, read by the host on insert.

The framework side (`day-pieces`) registers a `NavController` whose `push` selects a tab by key,
`current` reports the active tab key, and `pop` is a no-op (tabs have no back-stack). Native tab
selection arrives as `Event::SelectionChanged`; the host lays out each page's content at the size
the tab widget reports via `Event::FrameChanged`.

## Native mapping

The widget owns page content on every backend (the user-visible choice is a native tab widget
per platform; GTK, having adopted libadwaita, uses the Adwaita segmented switcher — a `.linked`
toggle group — over an `AdwViewStack`, since Adwaita has no icon-free tab widget):

| Backend | Widget | Notes |
|---------|--------|-------|
| AppKit  | `NSTabView` (`NSTabViewItem` per page) | `NSTabViewDelegate` reports selection |
| UIKit   | `UITabBarController` | bottom tab bar; each page is a child `UIViewController` |
| GTK 4   | `AdwViewStack` + a `.linked` grouped-toggle switcher | libadwaita; label-only segmented control drives the stack |
| Qt      | `QTabWidget` (shim) | `currentChanged` reports selection |
| Android | custom `DayTabs` strip | top tab bar + content `FrameLayout` (dependency-free; Material's TabLayout/BottomNavigationView aren't required) |
| WinUI 3 | `Pivot` (shim) | `SelectionChanged` reports selection |

Each page reports its allocated content size (`FrameChanged`) so day lays out the tab's content
at native size — the same mechanism nav pages use. Pages with native-owned frames are skipped by
`set_frame`.

## Deep links & dayscript

Because tab keys are routes, everything that already targets routes works unchanged:

- **Deep link:** launching with the deep link `settings` selects the settings tab once the tabs
  host is mounted (warm links arrive as `Custom("deeplink")` and re-`navigate`).
- **dayscript:**

  ```yaml
  - navigate: { route: tabs }        # enter the tabs route (nav pushes it; tabs registers)
  - assert_route: { route: overview } # innermost host = the active tab
  - navigate: { route: details }      # select a tab by key
  - assert_route: { route: details }
  - assert_value: { id: main-tabs, value: 1 } # the tabs host records the active index
  ```

## Testing

- **e2e (`day-mock`, `crates/day-pieces/tests/mock_e2e.rs`):** eager page build, select-by-key +
  index recording, native-selection → `current_route`, no-redundant-patch idempotence, and the
  nested-in-nav fall-through (leaving a tab route disposes the tabs host). Plus a deep-link test.
- **Showcase + walkthrough:** the `tabs` route hosts three keyed tabs; the walkthrough enters it,
  selects each tab by key, asserts the route/index, and screenshots — verified on all five local
  targets (macos-appkit/gtk/qt, ios-uikit, android-widget).
