---
title: "Search"
description: "searchable(): a declared search surface each platform presents natively: toolbar field, navigation search, or list filter."
---

# Search (`searchable`)

Search is declared on the **surface**, not on the toolbar.

```rust
nav(section)
    .style(NavStyle::Sidebar)
    .searchable(query)                   // Signal<String>, two-way
    .search_prompt(tr("search-sections"))
    .items(move || destinations_matching(query.get()), row)
```

That decision makes the rest work. Because the surface owns the declaration and the
app owns the `Signal`, the toolkit is free to draw the field wherever its platform puts search (in
the window toolbar on a wide window, attached to the navigation list on a narrow one), and
moving it never moves the state. The app filters its own rows from its own signal either way, and
never branches on where the field went.

This is the same move SwiftUI made with `.searchable()`, for the same reason.

> [!NOTE]
> **Implementation status (2026-08-09).** `.search_scopes()` lowers correctly but **no backend
> draws a scope bar yet**, because a scope bar belongs below the field, and the toolbar placement has
> nowhere to put one (see below). Everything else on this page is implemented.
>
> The following were removed rather than shipped inert: `is_searching()`, `dismiss_search()`, and the
> `Cap::Search`/`SearchScopes`/`SearchSuggestions` capabilities. No backend emitted the activity
> event or reported those caps, so the first two were always false and the caps answered
> `Unsupported` on platforms where search works. They will come back with the
> backends that implement them.
>
> A sidebar `.header()` on iOS defeats the search-bar auto-hide, because the header is a
> static view above the list and UIKit's collapse tracks the page's top-anchored scroll view.
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

## Find commands

Bind an application's localized Find command to `day::focus_search()` (usually Cmd-F on
macOS and Ctrl-F elsewhere). It resolves the focused window's searchable navigation surface,
reveals/focuses the native field, and keeps its existing query. Desktop toolbars place search
at the trailing edge in the Detail column. On AppKit the native search field sends its
immediate action for user edits; application text synchronization remains echo-guarded.
Focus requests use transient toolbar/inline patches, without rebuilding the field. A synthetic
`toolbar: { item: day.search, text: ... }` verifies model wiring, but native keyboard/input
validation is also needed to verify actual AppKit event delivery.

## Placement is a preference

`SearchPlacement::{Automatic, Toolbar, Inline}` states a preference. A backend that cannot
honor the request falls back to its platform's convention, as SwiftUI documents
("depending on the containing view hierarchy and platform, the requested placement may not be able
to be fulfilled"). `Automatic` is almost always the right answer: it lets the field live in
the toolbar on a desktop window and move into the navigation list on a phone.

`Automatic` resolves to the window toolbar wherever that toolbar can hold a search field
(`Cap::Toolbar` and `Cap::ToolbarSearch`), and to **`Inline`** where it cannot. The phones answer
`Cap::ToolbarSearch` unsupported: an Android app-bar menu, iOS bar buttons and ArkUI's title-bar
menus are rows of actions, so their field goes on the navigation surface. That needs no size class,
because "this toolkit's bar holds no field" is a static fact about the backend rather than a
question about the window's width. An explicit `Toolbar` falls back to `Inline` on the same
backends: a field the bar cannot draw would otherwise not be drawn at all, which is what happened
on iOS, Android and ArkUI before the capability existed.

On **iOS the two placements name one surface** anyway: `UINavigationItem` owns the navigation bar's
buttons AND its search controller, and the window toolbar rides that same item
([docs/toolbars.md](toolbars.md)), so day-uikit installs the field whichever way it was asked
for.

> [!NOTE]
> **The case in between still waits on size classes**: a narrow window on a toolkit that
> *does* have a toolbar. Until that lands, a narrow desktop window keeps its field in the toolbar
> ([docs/navigation.md](navigation.md)).

### Inline, per platform

Each platform keeps its own convention.

| backend | where | resting state | how it is revealed |
|---|---|---|---|
| ios-uikit | `UISearchController` on the root page's `navigationItem` | pinned by default | opt-in: search and large title collapse while scrolling; initially-hidden mode uses a compact title and pull-down drawer |
| android-mdc | Material `TextInputLayout` over the nav list | pinned by default | opt-in: scroll/enter-always app bar, returns on downward scroll |
| harmony-arkui | ArkUI `Search` atop the nav list | visible | always there |

Call `.search_hides_on_scroll(true)` after `.searchable(query)` to opt into scrolling chrome.
Desktop toolbar search is unchanged. UIKit explicitly associates the root page's scroll view
with its top edge, so a header or a pinned footer does not prevent native large-title/search
collapse. This applies to both the compact stack and the iPad sidebar.

For a Mail-style iOS drawer, use `.search_initially_hidden(true)` after `.searchable(query)`:

```rust,ignore
nav(selection)
    .searchable(query)
    .search_hides_on_scroll(true) // optional Android scrolling search too
    .search_initially_hidden(true)
```

The new flag defaults to `false`. On UIKit it implies scrolling search, keeps the title in
one compact toolbar row, and starts the native search drawer collapsed. Pull down slightly
on the list to reveal search. On iOS 16+ the drawer explicitly uses `Stacked` placement so
an iPad does not substitute an always-visible inline field. iOS 15 uses its native stacked
placement. UIKit owns subsequent scrolling, editing and navigation transitions: Day never
resets the content offset or clears the query on layout, return navigation or rotation.
Passing `false` preserves the previous large-title presentation and the independently chosen
`search_hides_on_scroll` policy. Other backends ignore `search_initially_hidden`; their
existing search placement and scrolling behavior are unchanged.

Android uses `CoordinatorLayout` and Material `AppBarLayout` scroll/enter-always/snap flags.
Only nested scrolls originating in the root navigation list move this chrome. The search
field is kept expanded during editing. On compact destinations the search field is hidden
and the same MaterialToolbar is moved outside the scrolling CoordinatorLayout, where Up,
title and actions stay pinned. On wide layouts the toolbar is also pinned, while the sidebar
search can scroll independently. Only the compact root places the toolbar in the scroll-away
app bar. Selection, push, pop and layout changes reconcile this together with the title and
Up action, including section selections that do not enter FragmentManager history. A late
root fling cannot move the destination toolbar because it no longer belongs to the app bar. The list content is sized to the visible area so pinned
footers remain on screen. Immersive hosts retain their existing floating chrome. The field
continues to filter the existing list; it does not open a separate full-screen SearchView.

## One model, one writer

`SearchProps` on the nav host holds the state for every placement. The toolbar item a
desktop backend draws is a rendering of it and carries no state of its own:
both inbound transports (a toolbar value callback, or `Event::SearchChanged` from an inline field)
write the app's `Signal`, and one outbound binding patches whichever target the resolved placement
renders into.

That will make a placement change tractable. The state does not live in the widget, so
re-rendering into the other target is a patch rather than a rebuild, the same property that makes
the navigation host re-presentable. The remaining step for the size-class work is a
`SearchPatch::Placement` that swaps the render target on a live host; until it exists, placement is
resolved once at realize and cannot change ([docs/navigation.md](navigation.md)).

## How the toolbar placement is realized

Day hands the resolved field to day-core as an ordinary `ToolbarItemKind::Search` item, which is
merged into the window's bar under the reserved id **`day.search`**, trailing. This has two
consequences:

- every backend that already drew a toolbar search field draws this one, with no per-backend code;
- the merge happens inside day-core's window-bar composition, not at the app's call site, because the app
  installs its bar *before* the tree builds and a derived contribution re-lowers on
  any reactive change. An item injected once would be dropped by the next rebuild.

dayscript addresses the field by that id wherever the platform put it: `toolbar: { item:
day.search, text: "…" }`. On the bar that is the item above; inline, where no item exists, day-core
resolves the id to the host showing the field (`inline_search_host`) and delivers
`Event::SearchChanged`, the event the native field emits, so one step types the same query on a
desktop and on a phone. (A bare `toolbar: { item: day.search }` has nothing to press inline, and
says so.)

## Scopes are not drawn yet

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
| windows-winui | a `ToggleButton` row (written for system XAML, which has no `Segmented`; the WinUI build keeps it) | `Emulated` |

`Emulated` covers two different situations there: a real native component doing this job (the
chips, `SegmentButtonV2`, `NSSegmentedControl`) and a bar composed from primitives (web, XAML).
Neither claims to be the platform's scope bar.

## Suggestions complete the field

On a navigation surface the list **is** the result set. So `.search_suggestions()` offers
completions that fill the field; it does not open an overlay of results, which would cover the
list it is narrowing.

Day uses the search widget's own completion affordance, and only that: a completion popup Day drew
itself would not match the platform's keyboard handling or styling, and a search field that behaves
differently from every other one on the system is worse than no completions.

| backend | completions |
|---|---|
| linux-qt | `QCompleter` (native popup, case-insensitive) |
| web-dom | `<datalist>` (the browser's own popup) |
| windows-winui | `AutoSuggestBox.ItemsSource` |
| ios-uikit | `UISearchResultsUpdating`, at any placement — see below |
| macos-appkit | **none.** `NSSearchField`'s menu is a recents list, not completions for the current text, so Day does not present it as one |
| linux-gtk | **none.** GTK4 deprecated `GtkEntryCompletion` and `GtkSearchEntry` has no replacement |
| android-mdc, harmony-arkui | with the Inline placement |

Choosing a suggestion puts it in the field and emits the same change any keystroke would, so an app
that only reads `query` needs no extra handling. `Event::SearchSuggestionChosen` says *which* one,
for an app that wants to act on the choice itself.

## What is searchable

`Nav` today. A `nav_stack` gains the same surface when the placement resolver lands, since it is the
same lowering. Search on arbitrary page content is out of scope, because every backend
would need a placement answer for content that is not navigation chrome.

## Two-way, through the signal

Both directions go through the app's `Signal`, never through the widget:

- the user typing emits `Event::SearchChanged`, which writes the signal;
- the app writing the signal patches the live field (`SearchPatch::Text`) rather than rebuilding
  it, so a sync never steals focus or resets the insertion point mid-word.

That discipline will let the field change placement later without the query noticing.
