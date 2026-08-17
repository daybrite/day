---
title: "Search"
description: "searchable(): a declared search surface each platform presents natively — toolbar field, navigation search, or list filter."
---

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

> [!NOTE]
> **Implementation status (2026-08-09).** `.search_scopes()` lowers correctly but **no backend
> draws a scope bar yet** — a scope bar belongs below the field, and the toolbar placement has
> nowhere to put one (see below). Everything else on this page is implemented.
>
> Removed rather than shipped inert: `is_searching()`, `dismiss_search()`, and the
> `Cap::Search`/`SearchScopes`/`SearchSuggestions` capabilities. No backend emitted the activity
> event or reported those caps, so the first two were always false and the caps answered
> `Unsupported` on platforms where search demonstrably works. They will come back with the
> backends that implement them.
>
> Also: a sidebar `.header()` on iOS defeats the search-bar auto-hide, because the header is a
> static view above the list and UIKit's collapse tracks the page's TOP-ANCHORED scroll view.
> Day-Showcase dropped its header for this reason; the framework fix is to make a sidebar header
> the table's `tableHeaderView` so header and rows scroll as one.

## The modifiers

| modifier | what it does |
|---|---|
| `.searchable(query)` | makes the surface searchable, bound two-way to `query` |
| `.search_prompt(text)` | the empty-state prompt |
| `.search_placement(p)` | a placement **preference** (see below) |
| `.search_scopes(scope, titles)` | a one-of-N scope bar, bound to `scope` |
| `.search_suggestions(f)` | completions for the current text |

## Placement is a preference, not an instruction

`SearchPlacement::{Automatic, Toolbar, Inline}` asks; it does not command. A backend that cannot
honor the request falls back to its platform's own convention — exactly as SwiftUI documents
("depending on the containing view hierarchy and platform, the requested placement may not be able
to be fulfilled"). `Automatic` is almost always the right answer: it is what lets the field live in
the toolbar on a desktop window and move into the navigation list on a phone.

`Automatic` resolves to the window toolbar wherever the toolkit has one, and to **`Inline`** where
it does not — which is the phones. That second case needs no size class: "this toolkit has no
toolbar at all" is a static fact about the backend, not a question about the window's width.

> [!NOTE]
> **What still waits on size classes** is the case in between: a NARROW window on a toolkit that
> *does* have a toolbar. Until that lands, a narrow desktop window keeps its field in the toolbar
> ([docs/navigation.md](navigation.md)).

### Inline, per platform

Each platform's own convention, not one gesture forced onto all of them.

| backend | where | resting state | how it is revealed |
|---|---|---|---|
| ios-uikit | `UISearchController` on the root page's `navigationItem` | hidden above the list | over-scroll down (`hidesSearchBarWhenScrolling`) |
| android-mdc | Material `SearchBar` above the nav list | visible, scroll-away | always there; returns on scroll up |
| harmony-arkui | ArkUI `Search` atop the nav list | visible | always there |

Android has **no** `hidesSearchBarWhenScrolling` equivalent, and that is deliberate on Material's
part: `SearchBar` supports fixed / scroll-away / lift-on-scroll through `CoordinatorLayout`
behaviors, but scroll-away hides on scroll DOWN and returns on scroll UP — there is no
pull-past-the-top reveal. Forcing the iOS gesture onto Android would make a Day app the only one on
the device that behaves that way.

Note also that Day uses `SearchBar` **without** expanding it into Material's full-screen
`SearchView`. `SearchView`'s model is an overlay showing its own results list, and on a searchable
navigation surface the list underneath already IS the result set — the overlay would mean
maintaining a second copy of it.

## One model, one writer

`SearchProps` on the nav host is the source of truth for every placement. The toolbar item a
desktop backend draws is a RENDERING of it, not a second representation carrying its own state:
both inbound transports (a toolbar value callback, or `Event::SearchChanged` from an inline field)
write the app's `Signal`, and one outbound binding patches whichever target the resolved placement
renders into.

That is what will make a placement CHANGE tractable. The state does not live in the widget, so
re-rendering into the other target is a patch rather than a rebuild — the same property that makes
the navigation host re-presentable. The remaining step for the size-class work is a
`SearchPatch::Placement` that swaps the render target on a live host; until it exists, placement is
resolved once at realize and cannot change ([docs/navigation.md](navigation.md)).

## How the toolbar placement is realized

Day hands the resolved field to day-core as an ordinary `ToolbarItemKind::Search` item, which is
merged into the window's bar under the reserved id **`day.search`**, trailing. Two consequences
worth knowing:

- every backend that already drew a toolbar search field draws this one, with no per-backend code;
- the merge happens inside `set_window_toolbar`, not at the app's call site, because the app
  installs its bar *before* the tree builds and `toolbar_reactive` re-installs the whole model on
  any reactive change. An item injected once would be dropped by the next rebuild.

dayscript addresses the field by that id: `toolbar: { item: day.search, text: "…" }`.

## Scopes — not yet, and why

`.search_scopes()` is in the API and lowers correctly, but **no backend draws a scope bar yet**,
because a scope bar has nowhere to live at the Toolbar placement: it belongs *below* the field, and
Day's toolbar model is a flat list of items with no "below the bar" slot. The field currently sits
inside an `NSSearchToolbarItem` / an `AdwHeaderBar` entry / a `QToolBar` widget, none of which can
host one.

Scopes therefore wait on the **Inline** placement, where the field is attached to the navigation
surface and the bar can sit under it. When they land, the per-backend mapping is:

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

`Emulated` covers two different situations there and the difference is worth knowing: a real native
component doing exactly this job (the chips, `SegmentButtonV2`, `NSSegmentedControl`) versus a bar
composed from primitives (web, XAML). Neither claims to be the platform's own scope bar.

## Suggestions complete the field

On a navigation surface the list **is** the result set. So `.search_suggestions()` offers
completions that fill the field — it does not open an overlay of results, which would cover the
very list it is narrowing.

Day uses the search widget's own completion affordance, and only that: a completion popup Day drew
itself would not match the platform's keyboard handling or styling, and a search field that behaves
differently from every other one on the system is worse than no completions.

| backend | completions |
|---|---|
| linux-qt | `QCompleter` (native popup, case-insensitive) |
| web-dom | `<datalist>` (the browser's own popup) |
| windows-xaml | `AutoSuggestBox.ItemsSource` |
| ios-uikit | `UISearchResultsUpdating` — with the Inline placement |
| macos-appkit | **none.** `NSSearchField`'s menu is a RECENTS list, not completions for the current text; offering it as one would misrepresent the control |
| linux-gtk | **none.** GTK4 deprecated `GtkEntryCompletion` and `GtkSearchEntry` has no replacement |
| android-mdc, harmony-arkui | with the Inline placement |

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
