---
title: Navigation
description: "Sidebar and tab selection, push/pop stacks, routes, and deep links — with native containers underneath."
order: 20
section: Guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day's navigation model is two Pieces and a route registry. `selector` handles "one of several
top-level sections" (a sidebar on desktop, tabs where that's the platform idiom); `stack` handles
"drill in, come back" (push/pop with the platform's own transitions and back gestures). Both are
driven by plain signals, so navigation state is app state: inspectable, settable, testable.

## Sections: `selector`

```rust
let section = Signal::new("home".to_string());

selector(section)
    .style(SelectorStyle::Sidebar)
    .title("My App")
    .item("home",     tr("nav_home"),     || home_page())
    .item("library",  tr("nav_library"),  || library_page())
    .item("settings", tr("nav_settings"), || settings_page())
```

The selection signal holds the active item's key. Set it from anywhere
(`section.set("settings".into())`) and the selector switches; the UI and programmatic navigation
can't disagree because they're the same state. Residency differs by style: `Tabs` builds every page at mount and keeps them alive, so tab
pages retain their state (field contents, scroll position) across switches; `Sidebar` builds
the selected page on demand and disposes it when the selection changes, so sidebar page state
lives in your signals, not in the widgets. The cost is that a selector with many heavy pages pays for all
of them up front; keep expensive content behind a `when` inside the page if that
matters.

## Drill-down: `stack`

```rust
let path = Signal::new(Vec::<String>::new());

stack(path, library_page())
    .title(tr("nav_library"))
    .destination(|key| detail_page(key))
```

The path signal is the navigation stack: `["album:42"]` means one page pushed above the root.
`.destination` maps a pushed key to its page. Pushing is a vector edit
(`path.update(|p| p.push(key))`) or, more commonly, the helpers:

```rust
nav_link(tr("open_album"), "album:42")   // a button that pushes
navigate("album:42");                    // push from code; returns false if no surface (selector, tabs, or stack) recognizes it
nav_back();                              // pop
current_route();                         // Option<String>
```

Underneath, `stack` uses the platform's navigation machinery (`UINavigationController` on
iOS, the androidx Fragment back stack on Android), so you get the iOS edge-swipe back gesture
and Android's back button without writing either. On Android 14+ the back gesture is fully
**predictive**: the system seeks the actual pop transition under your finger; the page
tracks it and either completes the pop or springs back when you release (on Android 13/14 the OS gates
this behind Developer options → "Predictive back animations"; Android 15 enables it by
default). On desktop, pushed pages get an in-window back header: a chevron and title above
the page on macOS and Qt, libadwaita's own header on GTK.

## Data-driven items

A `selector`'s items can come from a signal, so a sidebar or tab set follows your data:

```rust
selector(current)
    .style(SelectorStyle::Tabs)
    .items(move || rooms.get(), |r: &Room| item(r.id.clone(), r.name.clone()))
    .destination(|k| room_page(k))
```

Rows are added and removed on the native widget as the signal changes; if the selected item
disappears the selection resets. A data-driven item is a label plus an optional icon (the native
row). For a rich master list (avatar, preview, badge) use a [`list`](/docs/internal/list).

Mark a selector `.local()` when it is a *second* one-of-N control inside an already-routing page (a
filter strip beside the main tabs). Two routed selectors at the same level both feed
`current_route()`, so you'd get `section/main/filter` and `navigate` would be ambiguous; `.local()`
keeps the secondary one out of the route. A selector nested one level deeper (a `Tabs` inside a
`Sidebar` section) should stay routed; that cascade is intended. Debug builds warn when two routed
one-of-N surfaces share a level.

## Intercepting back

Guard the user's back (a native gesture, the back button, or `nav_back()`) with `on_back`, for
the unsaved-changes-confirm case. A programmatic `path.set` is never guarded (a write is not a
back); this mirrors Jetpack Compose's `BackHandler`.

```rust
stack(path, editor())
    .destination(|k| detail(k))
    .on_back(move |req| {
        if dirty.get() {
            day::task(async move {
                if confirm("Discard changes?").await {
                    dirty.set(false);
                    req.proceed();          // perform the pop the guard held
                }
            });
            BackResponse::Handled           // consume the back for now
        } else {
            BackResponse::Proceed           // pop normally
        }
    });
```

Return `Proceed` to pop now or `Handled` to consume it; a `Handled` guard can hold the
`BackRequest` and call `proceed()` later. While a guard is armed Day stops the toolkit from
auto-popping on a native gesture and routes the back through your guard instead: on iOS the
swipe is disabled and the back button is intercepted, on Android a back callback takes priority
(the predictive-back preview is unavailable while armed), on GTK the page's swipe is disabled.

## Routes and deep links

A route is `segments/joined/by/slashes` with an optional `?name=value` query. A **single key is
relative**: the innermost surface that knows it wins, falling through outward, right for a
button deep inside a page. A **multi-segment path is absolute**: it anchors at the outermost
surface that knows the first segment, resets everything inside, and descends. One string
reaches a stack several levels deep, even on a cold start where the inner surfaces haven't
mounted yet:

```rust
navigate("library/album-42?hint=shared");   // section, then push, with params

// in the destination builder:
stack(path, root).destination(|key| {
    let hint = route_param("hint");         // Some("shared") when opened via that route
    album_page(key, hint)
})
```

`current_route()` returns the **full** path (`"library/album-42"`), and it round-trips through
`navigate`, so persisting the whole route across launches is: save `current_route()` on the way
out, `navigate(&saved)` on the way back in.

For a single surface, `.restore(key)` does that for you (no `current_route()` plumbing):

```rust
selector(section).restore("nav.section")   // reopens on the last-viewed section
stack(path, home).restore("mail.path")     // rebuilds the pushed path
```

It saves the selected key (or the stack's path) on every change and reads it back at build. A
launch deep link outranks it, and a saved value that no longer fits is ignored. Persistence runs
through a store you install once (`day_part_prefs::install_nav_store()` in `main`), which is
disk-backed, so restore also survives an Android process death. With no store installed, `.restore`
is a no-op, so you can persist on one target and start fresh on another with the same code.

The same mechanism is what [dayscript](/docs/dayscript) uses: `navigate: { route: controls }` in
a script performs the write your UI would, and `assert_route` compares the full
`current_route()`. Testing a navigation flow is asserting on strings, and `day lint` checks
that every literal route in your sources and scripts starts with a declared item key, so a typo
is a lint warning instead of a silently-ignored tap.

## Typed routes

Strings are the wire format; your code doesn't have to speak it. Declare the keys as an enum
and both `selector` and `stack` accept it directly; every `.item`, destination match, and
navigation call site is then compile-checked:

```rust
day::routes! {
    pub enum Section { Home => "home", Library => "library", Settings => "settings" }
}

let section = Signal::new(None::<Section>);      // None = nothing selected (mobile list)
selector(section)
    .item(Section::Home,     tr("nav_home"),     || home_page())
    .item(Section::Library,  tr("nav_library"),  || library_page())
    .item(Section::Settings, tr("nav_settings"), || settings_page())
```

A sidebar keys on `Option<Section>` (`None` is the collapsed mobile list); tabs key on the bare
enum since a tab is always selected. Under the hood each variant maps to its declared string, so
deep links, dayscript, and `current_route()` are unchanged.

Where this earns its keep is **routes that carry data**. Implement the `Route` trait by hand
(`key()` encodes, `from_key()` parses), and stack destinations receive the typed value:

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
stack(path, library_page()).destination(|m: &Media| match m {
    Media::Album { id } => album_page(*id),           // parsed, not string-split
    Media::Track { id } => track_page(*id),
})
```

The encode/parse pair lives in one place instead of being scattered across every push and
destination, and a typed stack validates incoming deep links: a segment `from_key` rejects
stops the navigation instead of pushing a garbage page. Typed navigation helpers mirror the
string ones:

```rust
navigate_to(&Section::Library);                       // relative, ≙ navigate("library")
route(&Section::Library).then(&Media::Album { id: 42 })
    .param("hint", "shared")
    .navigate();                                      // absolute, with params
nav_link_to(tr("open_album"), route(&Section::Library).then(&Media::Album { id: 42 }))
```

`String` implements `Route` too, so the untyped examples above are the same API: start
stringly, move to an enum when the app grows, mix the two freely (a typed selector over a
`String` stack is fine).

## Patterns and limits

- **Desktop split layouts:** `SelectorStyle::Sidebar` gives the two-pane desktop shape. Give the
  detail pane `.grow()` or it collapses to its content width, the most common layout mistake in
  navigation code.
- **State restoration:** `.restore(key)` persists and restores selection and path through the
  installed nav store, as described above. Persist your own signals (see
  [parts: prefs](/docs/parts)) only for custom state beyond navigation.
- **More windows:** `day::open_window` opens secondary windows on every backend
  (`WindowKind::Normal` or `Preferences`, plus `register_preferences_with` for the settings
  window): native windows on the desktop backends, iPad scenes, Android activities, and OHOS
  abilities; iPhone and the web present them as a fullscreen cover. See
  [windows](/docs/internal/windows). Dialogs and alerts are separate
  ([dialogs reference](/docs/internal/dialogs)).
- **Android process death:** if Android kills a backgrounded process, relaunch is a cold start,
  but `.restore` reads back through the disk-backed prefs store, so navigation state survives
  it.

The [navigation reference](/docs/internal/navigation) has the per-platform mapping details.
