# Navigation (`selector`, `stack`)

day models navigation the way it models everything else: as a **projection of an app-owned
`Signal`**. There is no imperative navigation controller in app code — you own the state, and
the native container is reconciled to it. Two orthogonal primitives cover the field, matching
what every native toolkit has converged on:

- **`selector`** — a flat one-of-N choice, bound to a `Signal<String>` of the active key. Its
  `.style` picks the native chrome.
- **`stack`** — a genuine push/pop stack, bound to a `Signal<Vec<String>>` **path**.

A thin string-route adapter (`navigate`, `nav_back`, `current_route`) sits on top so deep links
and dayscript address surfaces by key — but the surfaces themselves run on their signals.

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
| `Sidebar` | a **NavigationSplitView**: macOS `NSSplitView` source-list + detail; **GTK `AdwNavigationSplitView`** (libadwaita); Qt `QSplitter`; on mobile it collapses to a list that pushes the detail (UINavigationController / Android toolbar+pages). |
| `Tabs` | a native tab widget: `NSTabView` / `UITabBarController` / `AdwViewStack` + a `.linked` toggle switcher / `QTabWidget` / Android tab strip / WinUI `Pivot` (docs/tabs.md). |

`selector(sel).style(Tabs)` is exactly what used to be `tabs()`; `selector(sel).style(Sidebar)`
is the old `nav()`. They are one primitive — a selection-bound switcher — differing only in
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

day reconciles the native stack to `path` (keep the common prefix, pop the rest, push the new
suffix — the same diff `NavigationStack`/React-Navigation do). The native containers:
`UINavigationController` (iOS), **`AdwNavigationView`** (GTK), Android back-stack, and a
top-page-only presentation on macOS `NSSplitView` / Qt `QSplitter` in stack mode. The path is
*data*, so deep-linking is "parse the URL into a path and `set` it," and the stack is unit-testable
without the framework.

## The string-route adapter (deep links & dayscript)

Each mounted surface registers a small adapter over its own signal, so the existing
key-addressed API keeps working:

- `navigate("key")` — reaches the **innermost** surface first and falls through outward. For a
  `selector` it sets the active key; for a `stack`, `navigate("")` pops to root while other keys
  fall through to the enclosing surface (a stack is driven by its path, not by magic strings).
- `nav_back()` — pops the innermost surface, falling through when it is already at its root.
- `current_route()` — the innermost surface's active key.
- Startup deep links (`DAY_DEEPLINK`) and Android warm links (`Custom("deeplink")`) route the
  same way.

Because each surface owns its own signal, **nesting is free** — a `selector(Tabs)` or a `stack`
inside a `selector(Sidebar)` section just works; there is no global navigation controller to
arbitrate, only this string adapter for addressing.

## Composition

The Mail.app / Files.app pattern falls out by nesting:

```rust
selector(section).style(SelectorStyle::Sidebar)
    .item("library", tr("library"), || stack(lib_path, library_root).destination(detail))
```

The sidebar selection drives which section shows; the selected section is itself a `stack` that
drills down. Each owns its signal; day reconciles each native container independently.

## Backend notes

- **GTK adopts libadwaita throughout** (`adw::Application` loads the Adwaita stylesheet). The
  window is an `AdwApplicationWindow` whose content is an `AdwToolbarView` (an `AdwHeaderBar`
  supplies the title, window controls, and drag; day's content sits below it). Navigation:
  `Sidebar` → `AdwNavigationSplitView` with `AdwNavigationPage` sidebar/content; `stack` →
  `AdwNavigationView` (push/pop + back gesture; its `popped` signal writes native back into the
  path). Page content is a `GtkFixed` wrapped in an `AdwNavigationPage`; day sizes it from the
  host width (sidebar is a fixed width, detail fills the rest). Tabs use an `AdwViewStack` with a
  `.linked` toggle switcher (docs/tabs.md); dialogs use `AdwAlertDialog` (docs/dialogs.md).
- **macOS `NSSplitView` / Qt `QSplitter`** honor a `split` flag: `Sidebar` shows both panes; a
  `stack` collapses the empty sidebar and stacks every page (top visible) in the detail pane.
- **Mobile** presents the host as a native stack for both `Sidebar` (collapsed) and `stack`.

## Testing

`crates/day-pieces/tests/mock_e2e.rs`: selector tabs/sidebar two-way binding, stack
push/pop/reconcile, native-back-into-path, deep-link, and nested fall-through. The showcase's
top-level nav is a `selector(Sidebar)`, its Tabs page a `selector(Tabs)`, and its Stack page a
genuine `stack` — all driven through the walkthrough on all five local targets.
