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

Underneath, `stack` uses the platform's navigation container where one exists — you get the iOS
edge-swipe back gesture and Android's back button/predictive back without writing either. On
desktop, back affordances render in-window.

## Routes and deep links

Keys are plain strings with whatever structure you give them (`"album:42"` is a convention, not
syntax). Because selection and path are signals, a deep link is a couple of writes:

```rust
pub fn open_deep_link(link: &str) {
    // e.g. "library/album:42"
    if let Some((section_key, item)) = link.split_once('/') {
        section.set(section_key.to_string());
        navigate(item);
    }
}
```

The same mechanism is what [dayscript](/docs/dayscript) uses: `navigate: { route: controls }` in
a script performs the write your UI would, and `assert_route` reads `current_route()`. Testing a
navigation flow is asserting on strings.

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
