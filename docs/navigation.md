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
| `Sidebar` | a NavigationSplitView: macOS `NSSplitView` source-list + detail; GTK `AdwNavigationSplitView` (libadwaita); Qt `QSplitter`; on mobile it collapses to a list that pushes the detail (UINavigationController / Android M3 app bar+pages with shared-axis motion). |
| `Tabs` | a native tab widget: `NSTabView` / `UITabBarController` / `AdwViewStack` + a `.linked` toggle switcher / `QTabWidget` / Android M3 `BottomNavigationView` / WinUI `Pivot` (docs/tabs.md). |

`selector(sel).style(Tabs)` is exactly what used to be `tabs()`; `selector(sel).style(Sidebar)`
is the old `nav()`. They are one primitive, a selection-bound switcher, differing only in
chrome and page lifetime (tabs keep every page resident; the sidebar builds the selected detail).

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
  innermost (`"mail/inbox/msg-42"`). It round-trips through `navigate`, so persisting navigation
  across launches is two lines: save `current_route()` on the way out (day-part-prefs works),
  `navigate(&saved)` after the first mount on the way back. dayscript's `assert_route` compares
  against the same full path.
- Startup deep links (`DAY_DEEPLINK`) and Android warm links (`Custom("deeplink")`) route the
  same way.

Because each surface owns its own signal, nesting needs no extra machinery: a `selector(Tabs)` or
a `stack` inside a `selector(Sidebar)` section just works. There is no global navigation controller
to arbitrate, only this string adapter for addressing.

**Ordering caveat**: relative dispatch and the full route walk the registry in mount order,
which equals nesting depth for a single active chain. Two *sibling* surfaces mounted at once
(two independent stacks visible in one window) are ordered by mount time, not focus — prefer
absolute routes (or drive the signals directly) in such layouts.

`day lint` cross-checks literal `navigate("…")` calls and dayscript `navigate:`/`assert_route:`
routes against the declared keys in your sources — `.item("key", …)` call sites and
`routes! { … => "key" }` blocks: a route whose first segment nothing declares is reported
(`day::lint::unknown-route`) rather than failing silently at runtime.

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
drills down. Each owns its signal; Day reconciles each native container independently.

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
- **Mobile** presents the host as a native stack for both `Sidebar` (collapsed) and `stack`.

## Testing

`crates/day-pieces/tests/mock_e2e.rs`: selector tabs/sidebar two-way binding, stack
push/pop/reconcile, native-back-into-path, deep-link, nested fall-through, and typed routes
(a `Signal<Option<Area>>` sidebar over a data-carrying `Leg(u32)` stack, including segment
validation). The showcase's top-level nav is a typed `selector(Sidebar)` over a `Section`
enum, its Tabs page a typed `selector(Tabs)`, and its Stack page a `stack` over a
data-carrying `Drill` enum, all driven through the walkthrough on all five local targets.
