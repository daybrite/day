---
title: Navigation
description: "Sidebar and tab selection, push/pop stacks, routes, and deep links, with native containers underneath."
order: 20
section: How-to guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day has two navigation pieces: [`nav`](https://daybrite.dev/api/day_pieces/fn.nav.html) selects a section, and [`nav_stack`](https://daybrite.dev/api/day_pieces/fn.nav_stack.html) opens detail pages with
a path back. Both use platform navigation controls and keep their state in
[signals](/docs/reference/glossary#signal). Routes connect that state to deep links and automated tests.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/examples/catalog) and [catalog setup](/docs/how-to/localization).

## Sections: [`nav`](https://daybrite.dev/api/day_pieces/fn.nav.html)

```rust
let section = Signal::new("home".to_string());

nav(section)
    .style(NavStyle::Sidebar)
    .title(res::str::my_app())
    .item("home", res::str::nav_home(), || home_page())
    .item("library", res::str::nav_library(), || library_page())
    .item("settings", res::str::nav_settings(), || settings_page())
```

The selection signal holds the active item's key. Set it from anywhere
([`section.set("settings".into())`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.set)) and the nav host switches; the UI and programmatic navigation
can't disagree because they're the same state. Residency differs by style: `Tabs` builds every page at mount and keeps them alive, so tab
pages retain their state (field contents, scroll position) across switches; `Sidebar` builds
the selected page on demand and disposes it when the selection changes, so state that must survive a switch belongs in signals created above the destination builder.
Signals created inside that builder are disposed with its page. A Tabs host with many heavy pages pays for all
of them up front; keep expensive content behind a [`when`](https://daybrite.dev/api/day_pieces/fn.when.html) inside the page if that
matters.

## Drill-down: [`nav_stack`](https://daybrite.dev/api/day_pieces/fn.nav_stack.html)

```rust
let path = Signal::new(Vec::<String>::new());

nav_stack(path, library_page())
    .title(res::str::nav_library())
    .destination(|key| detail_page(key))
```

The path signal is the navigation stack: `["album:42"]` means one page pushed above the root.
[`.destination`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.destination) maps a pushed key to its page. Pushing is a vector edit
([`path.update(|p| p.push(key))`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.update)) or, more commonly, the helpers:

```rust
nav_link(res::str::open_album(), "album:42");  // a button that pushes
navigate("album:42");                    // push from code; returns false if no surface (nav host, tabs, or stack) recognizes it
nav_back();                              // pop
current_route();                         // Option<String>
```

[`nav_stack`](https://daybrite.dev/api/day_pieces/fn.nav_stack.html) delegates native back handling to each backend, including UIKit navigation
controllers and Android back callbacks. Guarded and unguarded navigation can use different
gesture paths. Check edge swipes, system back, and any predictive preview on the OS versions
you ship; the repository implementation is not evidence of those behaviors passing on devices.
Desktop back headers also depend on the chosen toolkit.

## Data-driven items

A [`nav`](https://daybrite.dev/api/day_pieces/fn.nav.html)'s items can come from a signal, so a sidebar or tab set follows your data:

```rust
nav(current)
    .style(NavStyle::Tabs)
    .items(nav_items(move || rooms.get(), |r: &Room| item(r.id.clone(), r.name.clone())))
    .destination(|k| room_page(k))
```

Rows are added and removed on the native widget as the signal changes; if the selected item
disappears the selection resets. A data-driven item is a label plus an optional icon (the native
row). For a rich master list (avatar, preview, badge) use a [`list`](/docs/reference/list).

Mark a nav host [`.local()`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.local) when it is a *second* one-of-N control inside an already-routing page (a
filter strip beside the main tabs). Two routed nav hosts at the same level both feed
[`current_route()`](https://daybrite.dev/api/day_pieces/fn.current_route.html), so you'd get `section/main/filter` and [`navigate`](https://daybrite.dev/api/day_pieces/fn.navigate.html) would be ambiguous; [`.local()`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.local)
keeps the secondary one out of the route. A nav host nested one level deeper (a `Tabs` inside a
`Sidebar` section) should stay routed. Debug builds warn when two routed one-of-N surfaces share
a level.

## Intercepting back

Guard the user's back (a native gesture, the back button, or [`nav_back()`](https://daybrite.dev/api/day_pieces/fn.nav_back.html)) with [`on_back`](https://daybrite.dev/api/day_pieces/trait.NavStackBuilder.html#tymethod.on_back), for
the unsaved-changes-confirm case. A programmatic `path.set` is never guarded (a write is not a
back); this mirrors Jetpack Compose's `BackHandler`.

```rust
nav_stack(path, editor())
    .destination(|k| detail(k))
    .on_back(move |req| {
        if dirty.get() {
            day::task(async move {
                if confirm(res::str::discard_changes()).await {
                    dirty.set(false);
                    req.proceed(); // perform the pop the guard held
                }
            });
            BackResponse::Handled // consume the back for now
        } else {
            BackResponse::Proceed // pop normally
        }
    });
```

Return `Proceed` to pop now or `Handled` to consume it; a `Handled` guard can hold the
`BackRequest` and call [`proceed()`](https://daybrite.dev/api/day_pieces/struct.BackRequest.html#method.proceed) later. While a guard is armed Day stops the [toolkit](/docs/reference/glossary#toolkit) from
auto-popping on a native gesture and routes the back through your guard instead: on iOS the
swipe is disabled and the back button is intercepted, on Android a back callback takes priority
(the predictive-back preview is unavailable while armed), on GTK the page's swipe is disabled.

[`Route::title()`](https://daybrite.dev/api/day_pieces/trait.Route.html#method.title) may read reactive metadata. The stack updates the current native header when
that title changes, even if the route still compares equal. This lets a destination open
immediately with a loading title and acquire its final title after fetching data without
rebuilding the page.

## Routes and deep links

A route is `segments/joined/by/slashes` with an optional `?name=value` query. A **single key is
relative**: the innermost surface that knows it wins, falling through outward, which suits a button
deep inside a page. A **multi-segment path is absolute**: it anchors at the outermost
surface that knows the first segment, resets everything inside, and descends. One string
reaches a stack several levels deep, even on a cold start where the inner surfaces haven't
mounted yet:

```rust
navigate("library/album-42?hint=shared");   // section, then push, with params

// in the destination builder:
nav_stack(path, root).destination(|key| {
    let hint = route_param("hint");         // Some("shared") when opened via that route
    album_page(key, hint)
})
```

[`current_route()`](https://daybrite.dev/api/day_pieces/fn.current_route.html) returns the **full** path (`"library/album-42"`), and it round-trips through
[`navigate`](https://daybrite.dev/api/day_pieces/fn.navigate.html), so persisting the whole route across launches is: save [`current_route()`](https://daybrite.dev/api/day_pieces/fn.current_route.html) on the way
out, [`navigate(&saved)`](https://daybrite.dev/api/day_pieces/fn.navigate.html) on the way back in.

For a single surface, [`.restore(key)`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.restore) does that for you without any [`current_route()`](https://daybrite.dev/api/day_pieces/fn.current_route.html) calls:

```rust
nav(section).restore("nav.section");  // reopens on the last-viewed section
nav_stack(path, home).restore("mail.path");    // rebuilds the pushed path
```

It saves the selected key (or the stack's path) on every change and reads it back at build. A
launch deep link outranks it, and a saved value that no longer fits is ignored. Persistence runs
through the app's preferences, which [`day::launch`](https://daybrite.dev/api/day/fn.launch.html) sets up, and they are disk-backed, so restore
also survives an Android process death.

[dayscript](/docs/how-to/dayscript) uses the same mechanism: `navigate: { route: controls }` in a script
performs the write your UI would, and `assert_route` compares the full [`current_route()`](https://daybrite.dev/api/day_pieces/fn.current_route.html). A
navigation test asserts on route strings, and `day lint` checks that every literal route in your
sources and scripts starts with a declared item key, so a typo is a lint warning instead of a
silently-ignored tap.

## Typed routes

Routes travel as strings; in code you can declare them as an enum. Declare the keys as an enum and
both [`nav`](https://daybrite.dev/api/day_pieces/fn.nav.html) and [`nav_stack`](https://daybrite.dev/api/day_pieces/fn.nav_stack.html) accept it directly; every [`.item`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.item), destination match, and navigation call
site is then compile-checked:

```rust
day::routes! {
    pub enum Section { Home => "home", Library => "library", Settings => "settings" }
}

let section = Signal::new(None::<Section>); // None = nothing selected (mobile list)
nav(section)
    .item(Section::Home, res::str::nav_home(), || home_page())
    .item(Section::Library, res::str::nav_library(), || library_page())
    .item(Section::Settings, res::str::nav_settings(), || {
        settings_page()
    })
```

A sidebar keys on `Option<Section>` (`None` is the collapsed mobile list); tabs key on the bare
enum since a tab is always selected. Each variant maps to its declared string, so
deep links, [dayscript](/docs/reference/glossary#dayscript), and [`current_route()`](https://daybrite.dev/api/day_pieces/fn.current_route.html) are unchanged.

Typed routes pay off most for **routes that carry data**. Implement the `Route` trait by hand
([`key()`](https://daybrite.dev/api/day_pieces/trait.Route.html#method.key) encodes, [`from_key()`](https://daybrite.dev/api/day_pieces/trait.Route.html#method.from_key) parses), and stack destinations receive the typed value:

```rust
#[derive(Clone, PartialEq)]
enum Media { Album { id: u32 }, Track { id: u32 } }   // "album-42" ↔ Album { id: 42 }

impl Route for Media {
    fn key(&self) -> String {
        match self {
            Media::Album { id } => format!("album-{id}"),
            Media::Track { id } => format!("track-{id}"),
        }
    }
    fn from_key(key: &str) -> Option<Self> {
        if let Some(id) = key.strip_prefix("album-") {
            return id.parse().ok().map(|id| Media::Album { id });
        }
        key.strip_prefix("track-")?.parse().ok().map(|id| Media::Track { id })
    }
}

let path = Signal::new(Vec::<Media>::new());
nav_stack(path, library_page()).destination(|m: &Media| match m {
    Media::Album { id } => album_page(*id),           // parsed, not string-split
    Media::Track { id } => track_page(*id),
})
```

The encode/parse pair lives in one place, and a typed stack validates incoming deep links: a
segment `from_key` rejects stops the navigation instead of pushing a garbage page. Typed
navigation helpers mirror the string ones:

```rust
navigate_to(&Section::Library); // relative, ≙ navigate("library")
route(&Section::Library)
    .then(&Media::Album { id: 42 })
    .param("hint", "shared")
    .navigate(); // absolute, with params
nav_link_to(
    res::str::open_album(),
    route(&Section::Library).then(&Media::Album { id: 42 }),
)
```

`String` implements `Route` too, so the untyped examples above are the same API: start
stringly, move to an enum when the app grows, mix the two freely (a typed nav host over a
`String` stack is fine).

## Patterns and limits

- **Desktop split layouts:** `NavStyle::Sidebar` gives the two-pane desktop shape. Give the
  detail pane [`.grow()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.grow) or it collapses to its content width.
- **State restoration:** [`.restore(key)`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.restore) persists and restores selection and path through the
  installed nav store, as described above. Persist your own signals (see
  [parts: prefs](/docs/reference/parts)) only for custom state beyond navigation.
- **More windows:** [`day::open_window`](https://daybrite.dev/api/day/fn.open_window.html) opens secondary windows on every [backend](/docs/reference/glossary#backend)
  (`WindowKind::Normal`, `Preferences` or `About`; [`day-piece-settings`](/docs/reference/settings) registers the
  Settings and About windows for you): native windows on the desktop backends, iPad scenes, Android activities, and OHOS
  abilities; iPhone and the web present them as a fullscreen cover. See
  [windows](/docs/reference/windows). Dialogs and alerts are separate
  ([dialogs reference](/docs/reference/dialogs)).
- **Android process death:** if Android kills a backgrounded process, relaunch is a cold start,
  but [`.restore`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.restore) reads back through the disk-backed prefs store, so navigation state survives
  it.

The [navigation reference](/docs/reference/navigation) has the per-platform mapping details.

## Verify route and state lifetime

The examples above are integration fragments; app page functions and route enums must be
provided. Use the [navigation lesson](/docs/tutorials/navigation) for complete compiled code.
The current signatures are `nav<K: Route, S: Binding<K>>(selection: S) -> Nav<S, K>` and
`nav_stack<K: Route, S: Binding<Vec<K>>>(path: S, root: impl Piece) -> NavStack<S, K>`.
A route is not limited to `String`; its encode/parse contract is part of the app's data model.

Keep shared state above replaceable destination closures. Persisted navigation restores route
selection/path, not arbitrary page signals. [`navigate(&str) -> bool`](https://daybrite.dev/api/day_pieces/fn.navigate.html) reports whether routing
succeeded; it does not validate application data or grant permission to open a document.
Secondary windows have app-owned lifetimes and do not extend captured page-local signals.

Evidence: [navigation builders](https://github.com/daybrite/day/blob/main/crates/day-pieces/src/nav.rs),
[route dispatch](https://github.com/daybrite/day/blob/main/crates/day-core/src/nav.rs), and
[prefs nav store](https://github.com/daybrite/day/blob/main/crates/day-part-preferences/src/lib.rs).
Test unknown routes, back behavior, restoration, and close/reopen on the target you ship.

## Share state between sections

Create shared signals before the replaceable page builders. Both pages below read the same
count; switching away from the counter does not discard it. Section keys are protocol data,
while the labels are localized. The selection is supplied by the caller for programmatic
navigation and tests.

Complete module for `src/navigation_recipe.rs` in the
[technical recipe harness](/docs/reference/examples/catalog#technical-recipes).
Add the matching Fluent messages below. This module is compiled as part of the harness;
it is not a standalone app.

<!-- technical-example: navigation_recipe -->
```rust
use crate::res;
use day::prelude::*;

day::routes! {
    pub enum Section { Counter => "counter", Summary => "summary" }
}

pub fn sections(section: Signal<Section>, count: Signal<i64>) -> impl Piece {
    nav(section)
        .style(NavStyle::Sidebar)
        .item(Section::Counter, res::str::section_counter(), move || {
            crate::events::counter(count)
        })
        .item(Section::Summary, res::str::section_summary(), move || {
            label(res::str::section_total(count))
                .id(id!(section_total))
                .padding(Space::L)
        })
        .id(id!(recipe_sections))
        .grow()
}
```

<!-- technical-catalog: navigation_recipe -->
```ftl
section_counter = Counter
section_summary = Summary
section_total = Total: { $count }
```

This module also uses the [event module](/docs/explanation/reactivity#handle-an-event-and-update-state).
For the mobile sidebar's unselected list state, use `Signal<Option<Section>>` and set it to
`None`. The bare enum above always names a selected section.
