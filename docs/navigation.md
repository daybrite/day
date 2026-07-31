# Navigation (`selector`, `stack`)

Day models navigation the way it models everything else: as a projection of an app-owned
`Signal`. There is no imperative navigation controller in app code: you own the state, and
the native container is reconciled to it. Two orthogonal primitives cover the field, matching
what every native toolkit has converged on:

- **`selector`**: a flat one-of-N choice, bound to a `Signal` of the active key. Its
  `.style` picks the native chrome.
- **`stack`**: a push/pop stack, bound to a `Signal<Vec<_>>` **path**.

Both are generic over the key type — any [`Route`](#typed-routes): plain `String`s for
stringly-keyed quick starts, or an app-defined enum for compile-checked navigation whose
variants can carry data. A thin string-route adapter (`navigate`, `nav_back`, `current_route`)
sits underneath so deep links and dayscript address surfaces by key either way, but the
surfaces themselves run on their signals.

## `selector` — one-of-N

```rust
let section = Signal::new("home".to_string());
selector(section)
    .style(SelectorStyle::Sidebar)      // .Sidebar | .Tabs
    .title(tr("app-title"))
    .header(sidebar_header)             // optional piece above the list
    .item("home",     tr("home"),     home_page)
    .item("settings", tr("settings"), settings_page)
```

The active key is a `Signal<String>`, two-way exactly like `Picker`/`Toggle`: set it and the UI
switches; the user picking natively writes it back (origin-tagged, no echo).

| Style | Native container |
|-------|------------------|
| `Sidebar` | a NavigationSplitView: macOS `NSSplitView` sidebar (inset-styled `NSOutlineView`; accent selection, capture-safe — no offscreen-hostile material) + detail; GTK `AdwNavigationSplitView` (libadwaita); Qt `QSplitter`; on mobile it collapses to a list that pushes the detail (UINavigationController / Android M3 app bar+pages with shared-axis motion). |
| `Tabs` | a native tab widget: `NSTabView` / `UITabBarController` / `AdwViewStack` + a `.linked` toggle switcher / `QTabWidget` / Android M3 `BottomNavigationView` / XAML `Pivot` (docs/tabs.md). |

`selector(sel).style(Tabs)` is exactly what used to be `tabs()`; `selector(sel).style(Sidebar)`
is the old `nav()`. They are one primitive, a selection-bound switcher, differing only in
chrome and page lifetime (tabs keep every page resident; the sidebar builds the selected detail).

### Immersive items (`.immersive()`)

`.item(…).immersive()` marks the LAST-added destination as an immersive-chrome page: on
backends with an immersive nav mode (day-android's edge-to-edge opt-in today) its pushed page
keeps the floating transparent bar over full-bleed content, while unmarked pages and the root
get the standard opaque bar. Every other backend ignores the flag. Pair it with
`day::safe_area()` (docs/layout.md): the immersive page paints its background unpadded and pads
its content by the reported insets.

## `stack` — push/pop with a value path

```rust
let path = Signal::new(Vec::<String>::new());
stack(path, home_view)
    .destination(|key| detail_view(key))
// push:  path.update(|p| p.push("item-42".into()));
// pop:   path.update(|p| { p.pop(); });
// the native back button writes the pop back into `path` (origin-tagged).
```

Day reconciles the native stack to `path` (keep the common prefix, pop the rest, push the new
suffix; the same diff `NavigationStack`/React-Navigation do). The native containers:
`UINavigationController` (iOS), `AdwNavigationView` (GTK), Android back-stack, and a
top-page-only presentation on macOS `NSSplitView` / Qt `QSplitter` in stack mode. The path is
data, so deep-linking is "parse the URL into a path and `set` it," and the stack is unit-testable
without the framework.

## Data-driven items (`selector().items`)

`selector` items can come from a signal, so a sidebar or tab set grows and shrinks with your data
(a rooms list, open documents). Static `.item`s and dynamic `.items` blocks mix; pair `.items`
with `.destination` to build the page for a data-driven key (like `stack`).

```rust
let tabs = Signal::new(vec!["general".to_string(), "random".to_string()]);
selector(current)
    .style(SelectorStyle::Tabs)
    .items(move || tabs.get(), |k: &String| item(k.clone(), k.clone()))
    .destination(|k: &String| room_page(k))
```

The row set re-derives whenever a block's signal changes: rows are added/removed on the native
widget, and if the selected key disappears the selection resets (to `None` for an `Option` key).
The same effect resolves every row title tracked, so a runtime `set_locale` retitles the native
rows in place — static `.item`s included.
`item(key, title).icon(name)` is the row spec, and `.immersive()` on it marks that row's pushed
page immersive-chrome, same as the static form above. A selector used as a self-contained widget inside a
page that already routes should call `.local()` so it does not add a segment to `current_route` or
intercept `navigate`.

**A data-driven item is a label + optional icon** — the native sidebar/tab row. It is NOT an
arbitrary rich row (an avatar + preview + badge); a master list that needs those is a `list`, and
combining a rich master list with native master-detail push is a separate, not-yet-built feature.

**Backend support (2026-07):** dynamic add/remove/reselect renders on `macos-appkit`, `linux-gtk`,
`linux-qt`, and `web-dom` (and their host variants) — verified in the showcase walkthrough. The
`ios-uikit` sidebar and tab selection are wired; dynamic rendering on the UIKit/Android/ArkUI/XAML
tab widgets is in progress (those backends ignore the item-set patch until then, so the initial set
still shows). The item logic is backend-independent and covered by
`mock_e2e::selector_data_driven_items_reconcile`.

## Back interception (`on_back`)

`Stack::on_back` intercepts the user's back affordance — a native gesture/button, or `nav_back()`
— to run a policy before the pop. It does NOT run for a programmatic `path.set` (a write is not a
back), matching Jetpack Compose's `BackHandler`.

```rust
let dirty = Signal::new(false);
stack(path, home_view)
    .destination(|k| detail_view(k))
    .on_back(move |req| {
        if dirty.get() {
            // confirm asynchronously, then perform the deferred pop on "yes"
            day::task(async move {
                if confirm("Discard changes?").await {
                    dirty.set(false);
                    req.proceed();          // performs the pop the guard consumed
                }
            });
            BackResponse::Handled           // consume this back
        } else {
            BackResponse::Proceed           // normal pop
        }
    });
```

The guard returns `Proceed` (pop now) or `Handled` (consume; the pop does not happen). A `Handled`
guard may hold the `BackRequest` and call `proceed()` later — the unsaved-changes → confirm → leave
flow. `proceed()` performs exactly the pop `Proceed` would have.

While a guard is armed above the root, Day tells the toolkit to stop auto-popping on a native
gesture and route the back through Day instead (`NavPatch::GuardTop`). What that means per backend:

| backend | native-gesture arming while guarded |
|---|---|
| iOS (UIKit) | swipe disabled; the back **button** is vetoed via a `UINavigationController` subclass's `navigationBar:shouldPopItem:`, which emits the back to Day |
| Android | a higher-priority `OnBackPressedCallback` routes the system/gesture back and the toolbar up-arrow to Day (the predictive-back preview is unavailable while armed) |
| HarmonyOS (ArkUI) | the top `NavDestination`'s `onBackPressed` consumes the native back and defers to Day |
| GTK | the top `AdwNavigationPage` sets `can-pop = false` (swipe/Escape disabled; the app drives back through its own control, which is guarded) |
| macOS / Qt / XAML / web | no-op — the back button already routes through Day, so the guard runs with no native arming needed |

The guard's LOGIC (intercept, defer, proceed, never-on-programmatic-write) is identical everywhere
and covered by `mock_e2e::stack_on_back_guard_intercepts_and_defers`.

## Routes: the string-route adapter (deep links & dayscript)

Each mounted surface registers a small adapter over its own signal, so a string route can
address the whole tree. The grammar:

```text
route    = segment *( "/" segment ) [ "?" query ]     e.g.  mail/inbox/msg-42?hint=shared
segment  = a selector/tabs item key, or a stack destination key
query    = name "=" value *( "&" name "=" value )     (params for the destination builders)
```

Reserved characters inside a segment or param value (`/ ? & = %`) are percent-encoded;
`day_core::nav::{parse_route, encode_route}` do this for you. Two addressing modes:

- **A single key is RELATIVE** — `navigate("inbox")` reaches the innermost surface first and
  falls through outward. For a `selector`/tabs it sets the active key; a `stack` claims only
  `""` (pop to root), so sibling keys fall through to the enclosing surface. This is what a
  button deep inside a page wants: address the nearest thing that knows the key.
- **A `/`-separated path is ABSOLUTE** — `navigate("mail/inbox/msg-42")` anchors at the
  outermost surface that knows the first segment, resets every surface inside the anchor to its
  root, then feeds the remaining segments inward. Segments for surfaces that only mount as the
  outer switch takes effect are queued and consumed as those surfaces register — one string
  reaches a stack three levels deep on a cold start. A stack consumes absolute segments
  unconditionally (its destinations are open-ended); the explicit path IS the stack's state
  (set-semantics: navigating `mail/inbox` while `mail/inbox/msg-42` shows pops the detail).

**Params** ride the query string: `route_param("hint")` / `route_params()` inside a destination
builder return the values of the navigation being applied. They describe the navigation in
flight — a push you perform by writing the path signal directly carries its data in your own
state instead.

- `nav_back()`: pops the innermost surface, falling through when it is already at its root.
- `current_route()`: the **full** path — every mounted surface's contribution, outermost to
  innermost (`"mail/inbox/msg-42"`). It round-trips through `navigate`, so persisting the *whole*
  route by hand is two lines: save `current_route()` on the way out (day-part-prefs works),
  `navigate(&saved)` after the first mount on the way back. For a single surface, `.restore`
  (below) does the same without the plumbing. dayscript's `assert_route` compares against the same
  full path.
- Startup deep links (`DAY_DEEPLINK`) and Android warm links (`Custom("deeplink")`) route the
  same way. On hosts with no process environment the platform entry records the launch route
  with `day_core::set_launch_deeplink` instead — web-dom seeds it from the page's URL hash
  (docs/web.md), so `…/#controls` opens on that section.
- The URL stays live both ways on web-dom: day-core reports every route change to the backend
  (`Toolkit::set_route` — the hash updates as you navigate, one history entry per step), and a
  hash change the app didn't write (browser back/forward, a hand-edited URL) arrives as
  `Event::RouteRequested` and navigates. Other backends inherit the no-op default.

Because each surface owns its own signal, a `selector(Tabs)` or a `stack` nests inside a
`selector(Sidebar)` section with no extra wiring. There is no global navigation controller
to arbitrate, only this string adapter for addressing.

**Sibling one-of-N surfaces need `.local()`.** Every routed surface contributes to the full
route, so *two* `selector`/tabs at the **same level** — a filter tab strip beside a main tab bar —
both feed `current_route()`: you get `section/mainKey/filterKey`, and `navigate("filterKey")` is
ambiguous. Mark all but the primary one `.local()`; it then drives its own signal without touching
the route. A selector nested one level *deeper* (a `Tabs` inside a `Sidebar` section) is the
opposite case and should stay routed — that cascade is the whole point of nesting. In debug builds,
two routed one-of-N surfaces at the same level log a warning naming this fix.

**Ordering caveat**: relative dispatch and the full route walk the registry in mount order,
which equals nesting depth for a single active chain. Two *sibling* surfaces mounted at once
(two independent stacks visible in one window) are ordered by mount time, not focus — prefer
absolute routes (or drive the signals directly) in such layouts.

`day lint` cross-checks literal `navigate("…")` calls and dayscript `navigate:`/`assert_route:`
routes against the declared keys in your sources — `.item("key", …)` call sites and
`routes! { … => "key" }` blocks: a route whose first segment nothing declares is reported
(`day::lint::unknown-route`) rather than failing silently at runtime.

## Restoring state across launches (`.restore`)

When you want a surface to simply reopen where the user left it, mark it with `.restore(key)`
instead of wiring `current_route()` by hand:

```rust
selector(section).restore("nav.section")   // reopens on the last-viewed section
stack(path, home).restore("mail.path")     // rebuilds the pushed path
```

The selected key — or the stack's `/`-joined path — is saved under `key` on every change and read
back at build. A pending launch deep link **wins**: a `DAY_DEEPLINK` (or a `set_launch_deeplink`
hint) routes one turn after mount, so `.restore` steps aside and the link decides where the app
opens. A saved value that no longer fits — a selector key whose item is gone, a stack segment that
no longer parses — is ignored rather than restoring a broken state.

`.restore` reads and writes through a store the app installs once at startup; nothing persists
until you install one:

```rust
fn main() {
    day_part_prefs::install_nav_store();   // before the UI mounts
    // …
}
```

The prefs store is disk-backed, so restore also survives an **Android process death** — the OS
reclaims a backgrounded app and rebuilds it on return, and the value is still on disk. With no
store installed, `.restore` is a silent no-op, so the same code compiles and runs on a target
where you don't want persistence: the Showcase installs the store on web only, where a reload is
routine, and starts fresh on native. To back `.restore` with your own storage, implement
`day_core::NavStore` and hand it to `day_core::set_nav_store`.

## Typed routes

Route keys are data, and strings are just their wire format. The `Route` trait carries the
two-way mapping:

```rust
pub trait Route: Clone + PartialEq + 'static {
    fn key(&self) -> String;                  // typed value → path segment
    fn from_key(key: &str) -> Option<Self>;   // path segment → typed value
    fn title(&self) -> String { self.key() }  // native nav-bar title (defaults to the key)
}
```

`title()` is the label a [stack](#stacks-pushpop-navigation) shows in the native navigation bar
for a pushed page. It defaults to the wire `key`, so override it to display a name when the key
is not presentable (e.g. a route that carries only an id can look the name up from your data).

`String` implements it (the untyped baseline — every segment parses), and for plain enums the
`routes!` macro writes both sides:

```rust
day::routes! {
    pub enum Section { Home => "home", Stack => "stack" }
}

let section = Signal::new(None::<Section>);        // None = the collapsed mobile list
selector(section)
    .item(Section::Home,  tr("home"),  home_page)  // compile-checked, no raw keys
    .item(Section::Stack, tr("stack"), stack_page)
```

A sidebar `selector` keys on `Option<Section>` (`None` ↔ `""`, the no-selection list state);
tabs always have a selection, so they key on the bare enum (`Signal::new(Tab::One)`). Blanket
impls cover both: `Option<R>` is a `Route` whenever `R` is, and `.item` takes the bare variant
either way.

**Variants carry data** — this is the point where typed routes beat string encoding. Implement
`Route` by hand and put the payload in the variant:

```rust
enum Drill { Depth(u32), Item { id: u32 } }        // "3" ↔ Depth(3), "item-42" ↔ Item{id:42}

let path = Signal::new(Vec::<Drill>::new());
stack(path, root).destination(|d: &Drill| match d {
    Drill::Depth(n)    => level_page(*n),          // parsed, not string-split
    Drill::Item { id } => item_page(*id),
})
// push: path.update(|p| p.push(Drill::Item { id: 42 }));
```

The destination builder receives the parsed value; encode/decode lives in exactly one place
(the `Route` impl). A typed stack also **validates** absolute routes: a segment `from_key`
rejects is refused (the navigation stops there) instead of pushing a garbage key — a `String`
stack keeps its open-ended accept-anything behavior.

Typed absolute paths compose with `route(…)`, and `navigate_to` is the typed relative form:

```rust
navigate_to(&Section::Home);                       // ≙ navigate("home")
route(&Section::Stack).then(&Drill::Item { id: 42 })
    .param("hint", "linked")
    .navigate();                                   // ≙ navigate("stack/item-42?hint=linked")
nav_link_to(tr("open-42"), route(&Section::Stack).then(&Drill::Item { id: 42 }))
```

Everything downstream is unchanged: `current_route()` still returns the encoded string (which
is what you persist), deep links and dayscript still speak segments, and the two layers meet
only at `key`/`from_key`. Mixed trees are fine — a typed selector over a `String` stack, or
vice versa.

## Composition

The Mail.app / Files.app pattern falls out by nesting:

```rust
selector(section).style(SelectorStyle::Sidebar)
    .item("library", tr("library"), || stack(lib_path, library_root).destination(detail))
```

The sidebar selection drives which section shows; the selected section is itself a `stack` that
drills down. Each surface owns its signal.

**Nested stacks share one native container on mobile.** When the enclosing host presents as a
push stack (mobile, where `Cap::NavSplit` is unsupported and a `Sidebar` collapses to a
list-that-pushes), a `stack` built inside one of its pages does **not** mint a second native
navigation controller — it pushes its own pages onto the enclosing host, so the whole chain
(list → section → drill-down) is one native stack with a single back button. The inner `stack`
keeps its own path signal and route registration (so `current_route()`, deep links, and
`nav_back()` fall-through are unchanged); only the native container is shared. On desktop the
enclosing host presents as split panes (`split == true`), so a nested `stack` is *not* merged —
it renders in the detail pane with its own back-header, which is the right desktop shape. A
resident container (`selector(Tabs)`) is a merge barrier: a `stack` inside a tab keeps its own
host.

## Backend notes

- **GTK** adopts libadwaita throughout (`adw::Application` loads the Adwaita stylesheet). The
  window is an `AdwApplicationWindow` whose content is an `AdwToolbarView` (an `AdwHeaderBar`
  supplies the title, window controls, and drag; Day's content sits below it). Navigation:
  `Sidebar` → `AdwNavigationSplitView` with `AdwNavigationPage` sidebar/content; `stack` →
  `AdwNavigationView` (push/pop + back gesture; its `popped` signal writes native back into the
  path). Page content is a `GtkFixed` wrapped in an `AdwNavigationPage`; Day sizes it from the
  host width (sidebar is a fixed width, detail fills the rest). Tabs use an `AdwViewStack` with a
  `.linked` toggle switcher (docs/tabs.md); dialogs use `AdwAlertDialog` (docs/dialogs.md).
- **macOS `NSSplitView` / Qt `QSplitter`** honor a `split` flag: `Sidebar` shows both panes; a
  `stack` collapses the empty sidebar and stacks every page (top visible) in the detail pane,
  with a **back header** (chevron + centered title, hidden at the root) above the pages —
  desktop has no system back affordance, so a pushed page carries its own way out. The button
  emits the same `NavBack` event mobile back does, writing the pop into the path signal.
- **Android** hosts each page in an androidx **Fragment** that retains its Day-owned view
  (the react-native-screens pattern — the FragmentManager owns WHEN a page shows, Day owns
  WHAT it shows). A push is a `replace()` back-stack transaction carrying `MaterialSharedAxis`
  transitions, which buys the whole back story from the platform with no hand-rolled gesture
  code: `OnBackPressedDispatcher` dispatches hardware/gesture back on every API level, the
  FragmentManager **seeks the pop transition live under the predictive back gesture** on API
  34+ (progress, cancel, commit), and its back callback is enabled only while the back stack
  is non-empty — so the system's predictive back-to-home animation stays available at the
  root (apps opt in with `android:enableOnBackInvokedCallback="true"`; the scaffold does).
  Native pops are reported to Rust as `NavBack { already_popped: true }`; Rust-initiated pops
  run `popBackStack`. Note for testing: on Android 13/14 (API 33/34) the system gates
  predictive-back animation behind Developer options → "Predictive back animations"
  (`adb shell settings put global enable_back_animation 1`), and gesture navigation must be
  active; Android 15+ enables it by default.
- **Mobile** presents the host as a native stack for both `Sidebar` (collapsed) and `stack`. A
  `stack` nested inside such a host's page merges into it (one `UINavigationController` /
  `DayNavHost`, one back button) rather than nesting a second controller — see Composition. No
  backend change is involved: the shared host receives NAV_PAGE pushes/pops from both the outer
  surface and the inner stack identically to a single-surface stack.

## Testing

`crates/day-pieces/tests/mock_e2e.rs`: selector tabs/sidebar two-way binding, stack
push/pop/reconcile, native-back-into-path, deep-link, nested fall-through, and typed routes
(a `Signal<Option<Area>>` sidebar over a data-carrying `Leg(u32)` stack, including segment
validation). The showcase's top-level nav is a typed `selector(Sidebar)` over a `Section`
enum, its Tabs page a typed `selector(Tabs)`, and its Stack page a `stack` over a
data-carrying `Drill` enum, all driven through the walkthrough on all five local targets.
