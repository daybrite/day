---
title: Navigation
description: "Sidebar and tab selection, push/pop stacks, routes, and deep links — with native containers underneath."
order: 20
section: Guides
---

Day's navigation model is two Pieces and a route registry. `selector` handles "one of several
top-level sections" (a sidebar on desktop, tabs where that's the platform idiom); `stack` handles
"drill in, come back" (push/pop with the platform's own transitions and back gestures). Both are
driven by plain signals, so navigation state is app state — inspectable, settable, testable.

## Sections: `selector`

```rust
let section = Signal::new("home".to_string());

selector(section)
    .style(SelectorStyle::Sidebar)
    .title("My App")
    .item("home",     tr("nav-home"),     || home_page())
    .item("library",  tr("nav-library"),  || library_page())
    .item("settings", tr("nav-settings"), || settings_page())
```

The selection signal holds the active item's key. Set it from anywhere —
`section.set("settings".into())` — and the selector switches; the UI and programmatic navigation
can't disagree because they're the same state. All item pages are built at mount and kept alive —
switching shows and hides rather than rebuilding — so pages retain their state (field contents,
scroll position) across switches. The cost is that a selector with many heavy pages pays for all
of them up front; keep genuinely expensive content behind a `when` inside the page if that
matters.

## Drill-down: `stack`

```rust
let path = Signal::new(Vec::<String>::new());

stack(path, library_page())
    .title(tr("nav-library"))
    .destination(|key| detail_page(key))
```

The path signal is the navigation stack: `["album:42"]` means one page pushed above the root.
`.destination` maps a pushed key to its page. Pushing is a vector edit
(`path.update(|p| p.push(key))`) or, more commonly, the helpers:

```rust
nav_link(tr("open-album"), "album:42")   // a button that pushes
navigate("album:42");                    // push from code; returns false if no stack handles it
nav_back();                              // pop
current_route();                         // Option<String>
```

Underneath, `stack` uses the platform's navigation machinery — `UINavigationController` on
iOS, the androidx Fragment back stack on Android — so you get the iOS edge-swipe back gesture
and Android's back button without writing either. On Android 14+ the back gesture is fully
**predictive**: the system seeks the actual pop transition under your finger — the page
follows, springs back if you let go early, completes on commit (on Android 13/14 the OS gates
this behind Developer options → "Predictive back animations"; Android 15 enables it by
default). On desktop, pushed pages get an in-window back header — a chevron and title above
the page on macOS and Qt, libadwaita's own header on GTK.

## Routes and deep links

A route is `segments/joined/by/slashes` with an optional `?name=value` query. A **single key is
relative**: the innermost surface that knows it wins, falling through outward — right for a
button deep inside a page. A **multi-segment path is absolute**: it anchors at the outermost
surface that knows the first segment, resets everything inside, and descends — one string
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
`navigate` — so persisting navigation across launches is: save `current_route()` on the way
out, `navigate(&saved)` on the way back in.

The same mechanism is what [dayscript](/docs/dayscript) uses: `navigate: { route: controls }` in
a script performs the write your UI would, and `assert_route` compares the full
`current_route()`. Testing a navigation flow is asserting on strings — and `day lint` checks
that every literal route in your sources and scripts starts with a declared item key, so a typo
is a lint warning instead of a silently-ignored tap.

## Typed routes

Strings are the wire format; your code doesn't have to speak it. Declare the keys as an enum
and both `selector` and `stack` accept it directly — every `.item`, destination match, and
navigation call site is then compile-checked:

```rust
day::routes! {
    pub enum Section { Home => "home", Library => "library", Settings => "settings" }
}

let section = Signal::new(None::<Section>);      // None = nothing selected (mobile list)
selector(section)
    .item(Section::Home,     tr("nav-home"),     || home_page())
    .item(Section::Library,  tr("nav-library"),  || library_page())
    .item(Section::Settings, tr("nav-settings"), || settings_page())
```

A sidebar keys on `Option<Section>` (`None` is the collapsed mobile list); tabs key on the bare
enum since a tab is always selected. Under the hood each variant maps to its declared string, so
deep links, dayscript, and `current_route()` are unchanged.

Where this earns its keep is **routes that carry data**. Implement the `Route` trait by hand —
`key()` encodes, `from_key()` parses — and stack destinations receive the typed value:

```rust
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
destination, and a typed stack validates incoming deep links — a segment `from_key` rejects
stops the navigation instead of pushing a garbage page. Typed navigation helpers mirror the
string ones:

```rust
navigate_to(&Section::Library);                       // relative, ≙ navigate("library")
route(&Section::Library).then(&Media::Album { id: 42 })
    .param("hint", "shared")
    .navigate();                                      // absolute, with params
nav_link_to(tr("open-album"), route(&Section::Library).then(&Media::Album { id: 42 }))
```

`String` implements `Route` too, so the untyped examples above are the same API — start
stringly, move to an enum when the app grows, mix the two freely (a typed selector over a
`String` stack is fine).

## Patterns and limits

- **Desktop split layouts.** `SelectorStyle::Sidebar` gives the two-pane desktop shape. Give the
  detail pane `.grow()` or it collapses to its content width — the most common layout mistake in
  navigation code.
- **State restoration.** Persist the two signals (see [parts: prefs](/docs/parts)) and write them
  back at startup, and your app reopens where it closed. Nothing does this automatically.
- **One window.** Day currently drives a single window per process; multi-window is designed but
  not built. Dialogs and alerts are separate ([dialogs reference](/docs/internal/dialogs)).
- **Android process death.** If Android kills a backgrounded process, relaunch is a cold start —
  Day doesn't yet snapshot navigation state into the saved-instance mechanism, so restoring is
  your code (the prefs pattern above).

The [navigation reference](/docs/internal/navigation) has the per-platform mapping details.
