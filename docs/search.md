# Search (`searchable`)

Search is declared on the **surface**, not on the toolbar.

```rust
selector(section)
    .style(SelectorStyle::Sidebar)
    .searchable(query)                   // Signal<String>, two-way
    .search_prompt(tr("search-sections"))
    .items(move || destinations_matching(query.get()), row)
```

That one decision is what makes the rest work. Because the surface owns the declaration and the
app owns the `Signal`, the toolkit is free to draw the field wherever its platform puts search —
in the window toolbar on a wide window, attached to the navigation list on a narrow one — and
moving it never moves the state. The app filters its own rows from its own signal either way, and
never branches on where the field went.

This is the same move SwiftUI made with `.searchable()`, for the same reason.

## The modifiers

| modifier | what it does |
|---|---|
| `.searchable(query)` | makes the surface searchable, bound two-way to `query` |
| `.search_prompt(text)` | the empty-state prompt |
| `.search_placement(p)` | a placement **preference** (see below) |
| `.search_scopes(scope, titles)` | a one-of-N scope bar, bound to `scope` |
| `.search_suggestions(f)` | completions for the current text |

Reading the state back:

```rust
day::is_searching()   // tracked — the field is active (SwiftUI's isSearching)
day::dismiss_search() // put the field away (SwiftUI's dismissSearch)
```

`dismiss_search()` does **not** clear the query. The app owns that signal; clearing it is the
app's call, and plenty of apps want the filter to survive the field closing.

## Placement is a preference, not an instruction

`SearchPlacement::{Automatic, Toolbar, Inline}` asks; it does not command. A backend that cannot
honour the request falls back to its platform's own convention — exactly as SwiftUI documents
("depending on the containing view hierarchy and platform, the requested placement may not be able
to be fulfilled"). `Automatic` is almost always the right answer: it is what lets the field live in
the toolbar on a desktop window and move into the navigation list on a phone.

> [!NOTE]
> **Today `Automatic` always resolves to `Toolbar`.** Resolving it against the window's size class
> — so the field moves into the list when a sidebar collapses — arrives with the size-class work
> (docs/navigation.md). Until then a narrow window keeps the field in the toolbar.

## Scopes

A scope bar narrows what the search covers ("All / Unread / Flagged"). Only UIKit has a
purpose-built API for it; everywhere else Day uses the platform's own one-of-N control, and on two
backends it is composed from primitives. `Cap::SearchScopes` reports which you are getting.

| backend | scope bar | `Cap::SearchScopes` |
|---|---|---|
| ios-uikit | `UISearchBar.scopeButtonTitles` | `Native` |
| android-mdc | `ChipGroup` of single-selection filter chips | `Emulated` |
| harmony-arkui | `SegmentButtonV2` | `Emulated` |
| macos-appkit | `NSSegmentedControl` | `Emulated` |
| linux-gtk | `.linked` toggle buttons | `Emulated` |
| linux-qt | a button row | `Emulated` |
| web-dom | a radio group | `Emulated` |
| windows-xaml | a `ToggleButton` row (system XAML has no `Segmented`) | `Emulated` |

`Emulated` covers two different situations here and the difference is worth knowing: a real native
component doing exactly this job (the chips, `SegmentButtonV2`, `NSSegmentedControl`) versus a bar
built from primitives (web, XAML). Neither claims to be the platform's own scope bar, which is why
neither reports `Native`.

## Suggestions complete the field

On a navigation surface the list **is** the result set. So `.search_suggestions()` offers
completions that fill the field — it does not open an overlay of results, which would cover the
very list it is narrowing. Backends whose search widget already does completions use it
(`AutoSuggestBox`, `QCompleter`, `<datalist>`, `UISearchResultsUpdating`); the rest draw a popover.

Choosing a suggestion puts it in the field and emits the same change any keystroke would, so an app
that only reads `query` needs no extra handling. `Event::SearchSuggestionChosen` says *which* one,
for an app that wants to act on the choice itself.

## What is searchable

`Selector` today. A `stack` gains the same surface when the placement resolver lands — it is the
same lowering — and search on arbitrary page content is deliberately out of scope: every backend
would need a placement answer for content that is not navigation chrome.

## Two-way, through the signal

Both directions go through the app's `Signal`, never through the widget:

- the user typing emits `Event::SearchChanged`, which writes the signal;
- the app writing the signal patches the live field (`SearchPatch::Text`) rather than rebuilding
  it, so a sync never steals focus or resets the insertion point mid-word.

That discipline is what will let the field change placement later without the query noticing.
