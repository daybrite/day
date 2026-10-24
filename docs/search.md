---
title: "Search"
description: "searchable(Search): a declared search surface each platform presents natively: toolbar field, navigation search, or list filter."
---

# Search

Let people filter a page or find a destination with a search field in the platform’s usual
location. Bind the query to app state with [`.searchable()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.searchable); the app decides which results to show.

```rust
nav(section)
    .style(NavStyle::Sidebar)
    .searchable(Search::new(query).prompt(res::str::search_sections())) // query: Signal<String>, two-way
    .items(nav_items(move || destinations_matching(query.get()), row))
```

The search is one `Search` value: the query binding it is built from, and the prompt,
placement, reveal policy, scopes and suggestions chained onto it. That decision makes the rest
work. Because the surface owns the declaration and the app owns the binding, the toolkit is free to draw the field wherever its platform puts search (in
the window toolbar on a wide window, attached to the navigation list on a narrow one), and
moving it never moves the state. The app filters its own rows from its own signal either way, and
never branches on where the field went.

This is the same move SwiftUI made with [`.searchable()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.searchable), for the same reason.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

## The modifiers

| spelling | what it does |
|---|---|
| [`.searchable`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.searchable)`(`[`Search::new`](https://daybrite.dev/api/day_pieces/struct.Search.html#method.new)`(query))` | makes the surface searchable, bound two-way to `query` (a `Signal<String>` or any binding) |
| [`Search::prompt(text)`](https://daybrite.dev/api/day_pieces/struct.Search.html#method.prompt) | the empty-state prompt; a localized string or closure follows the language and its signals |
| [`Search::placement(p)`](https://daybrite.dev/api/day_pieces/struct.Search.html#method.placement) | a placement **preference** (see below) |
| [`Search::reveal(SearchReveal)`](https://daybrite.dev/api/day_pieces/struct.Search.html#method.reveal) | how the field comes and goes on a phone (see below) |
| [`Search::scopes(scope, titles)`](https://daybrite.dev/api/day_pieces/struct.Search.html#method.scopes) | a one-of-N scope bar, bound to `scope` |
| [`Search::suggestions(f)`](https://daybrite.dev/api/day_pieces/struct.Search.html#method.suggestions) | completions for the current text |

## Find commands

Bind an application's localized Find command to [`day::focus_search()`](https://daybrite.dev/api/day/fn.focus_search.html) (usually Cmd-F on
macOS and Ctrl-F elsewhere). It resolves the focused window's searchable navigation surface,
reveals/focuses the native field, and keeps its existing query. Desktop toolbars place search
at the trailing edge in the Detail column. On AppKit the native search field sends its
action for user edits from the run loop, just after the keystroke; application text
synchronization remains echo-guarded.
Focus requests use transient toolbar/inline patches, without rebuilding the field. A synthetic
`toolbar: { item: day::search, text: ... }` verifies model wiring, but native keyboard/input
validation is also needed to verify actual AppKit event delivery.

## Placement is a preference

`SearchPlacement::{Automatic, Toolbar, Inline}` states a preference. A toolkit that cannot
honor the request falls back to its platform's convention, as SwiftUI documents
("depending on the containing view hierarchy and platform, the requested placement may not be able
to be fulfilled"). `Automatic` is almost always the right answer: it lets the field live in
the toolbar on a desktop window and move into the navigation list on a phone.

`Automatic` resolves to the window toolbar wherever that toolbar can hold a search field
(`Cap::Toolbar` and `Cap::ToolbarSearch`), and to **`Inline`** where it cannot. The phones answer
`Cap::ToolbarSearch` unsupported: an Android app-bar menu, iOS bar buttons and ArkUI's title-bar
menus are rows of actions, so their field goes on the navigation surface. That needs no size class,
because "this toolkit's bar holds no field" is a static fact about the toolkit rather than a
question about the window's width. An explicit `Toolbar` falls back to `Inline` on the same
toolkits: a field the bar cannot draw would otherwise not be drawn at all, which is what happened
on iOS, Android and ArkUI before the capability existed.

On **iOS the two placements name one surface** anyway: [`UINavigationItem`](https://developer.apple.com/documentation/uikit/uinavigationitem) owns the navigation bar's
buttons AND its search controller, and the window toolbar rides that same item
([docs/toolbars](toolbars.md)), so day-uikit installs the field whichever way it was asked
for.

> [!NOTE]
> **The case in between still waits on size classes**: a narrow window on a toolkit that
> *does* have a toolbar. Until that lands, a narrow desktop window keeps its field in the toolbar
> ([docs/navigation](navigation.md)).

On iOS, a sidebar [`.header()`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.header) prevents the search bar from hiding automatically because
the header sits outside the list’s scrolling view.

### Inline, per platform

Each platform keeps its own convention.

| toolkit | where | resting state | how it is revealed |
|---|---|---|---|
| ios-uikit | [`UISearchController`](https://developer.apple.com/documentation/uikit/uisearchcontroller) on the root page's `navigationItem`, under the large title | pinned by default | opt-in: UIKit's `hidesSearchBarWhenScrolling`, which collapses the field and the large title as the list scrolls and brings both back at the top |
| android-mdc | pinned: a Material [`TextInputLayout`](https://developer.android.com/reference/com/google/android/material/textfield/TextInputLayout) over the nav list; scrolling: Material's [`SearchBar`](https://developer.android.com/reference/com/google/android/material/search/SearchBar) under the title, expanding into its full-screen [`SearchView`](https://developer.android.com/reference/com/google/android/material/search/SearchView) with the nav list as its results | pinned by default | opt-in: the search bar and title scroll away with the list and return on the first pull; a tap expands the search view |
| harmony-arkui | ArkUI `Search` atop the nav list | visible | always there |

[`Search::reveal(SearchReveal::Scrolls)`](https://daybrite.dev/api/day_pieces/struct.Search.html#method.reveal) opts into scrolling chrome. Desktop toolbar search
is unchanged whatever the reveal policy says. UIKit explicitly associates the root page's scroll view
with its top edge, so a header or a pinned footer does not prevent native large-title/search
collapse. This applies to both the compact stack and the iPad sidebar.

For a Mail-style iOS drawer, use `SearchReveal::OnPull`:

```rust,ignore
nav(selection)
    .searchable(Search::new(query).reveal(SearchReveal::OnPull))
```

The default is `SearchReveal::Always`. `OnPull` implies scrolling search (so Android's
scrolling chrome comes with it) and asks for the field to start out of sight. Android honors
that: the app bar starts collapsed and the first scroll up brings the search bar back. UIKit
has no such state, and Day no longer manufactures one: on iOS `OnPull` is `Scrolls`, the
field under the large title that UIKit's `hidesSearchBarWhenScrolling` collapses as the list
scrolls and shows again at the top, which is what Apple's own apps do. On iOS 16+ the field
uses `Stacked` placement at every width, so an iPad does not fold it into the bar's trailing
end. UIKit owns every scrolling, editing and navigation transition: Day never resets the
content offset or clears the query on layout, return navigation or rotation, and sets the
hide-on-scroll flag once, at the start (set later, or toggled, UIKit leaves the bar collapsed).
`Always` keeps the large-title presentation with the field pinned under it. The other
toolkits treat `OnPull` as `Scrolls`; their existing search placement is unchanged.

Android uses [`CoordinatorLayout`](https://developer.android.com/reference/androidx/coordinatorlayout/widget/CoordinatorLayout) and Material `AppBarLayout` scroll/enter-always/snap flags.
Only nested scrolls originating in the root navigation list move this chrome. The search
field is kept expanded during editing. On compact destinations the search field is hidden
and the same MaterialToolbar is moved outside the scrolling CoordinatorLayout, where Up,
title and actions stay pinned. On wide layouts the toolbar is also pinned, while the sidebar
search can scroll independently. Only the compact root places the toolbar in the scroll-away
app bar. Selection, push, pop and layout changes reconcile this together with the title and
Up action, including section selections that do not enter FragmentManager history. A late
root fling cannot move the destination toolbar because it no longer belongs to the app bar. The list content is sized to the visible area so pinned
footers remain on screen. Immersive hosts retain their existing floating chrome. With
scrolling chrome the field is Material's search bar: a tap expands it into the full-screen
search view, and the navigation list itself moves into that view as the results while it is
up, so the query filters the same rows in both places. Choosing a row navigates and collapses
the view; Back collapses it; the Search key submits and collapses it onto the filtered list,
whose query the collapsed bar keeps showing until its clear button empties it.

## One model, one writer

`SearchProps` on the nav host holds the state for every placement. The toolbar item a
desktop toolkit draws is a rendering of it and carries no state of its own:
both inbound transports (a toolbar value callback, or `Event::SearchChanged` from an inline field)
write the app's `Signal`, and one outbound binding patches whichever target the resolved placement
renders into.

That will make a placement change tractable. The state does not live in the widget, so
re-rendering into the other target is a patch rather than a rebuild, the same property that makes
the navigation host re-presentable. The remaining step for the size-class work is a
`SearchPatch::Placement` that swaps the render target on a live host; until it exists, placement is
resolved once at realize and cannot change ([docs/navigation](navigation.md)).

## How the toolbar placement is realized

Day hands the resolved field to day-core as an ordinary `ToolbarItemKind::Search` item, which is
merged into the window's bar under the reserved id **`day::search`**, trailing. This has two
consequences:

- every toolkit that already drew a toolbar search field draws this one, with no per-toolkit code;
- the merge happens inside day-core's window-bar composition, not at the app's call site, because the app
  installs its bar *before* the tree builds and a derived contribution re-lowers on
  any reactive change. An item injected once would be dropped by the next rebuild.

dayscript addresses the field by that id wherever the platform put it: `toolbar: { item:
day::search, text: "…" }`. On the bar that is the item above; inline, where no item exists, day-core
resolves the id to the host showing the field ([`inline_search_host`](https://daybrite.dev/api/day_core/fn.inline_search_host.html)) and delivers
`Event::SearchChanged`, the event the native field emits, so one step types the same query on a
desktop and on a phone. (A bare `toolbar: { item: day::search }` has nothing to press inline, and
says so.)

## Scopes are not drawn yet

[`Search::scopes`](https://daybrite.dev/api/day_pieces/struct.Search.html#method.scopes) is in the API and lowers correctly, but **no toolkit draws a scope bar yet**,
because a scope bar has nowhere to live at the Toolbar placement: it belongs *below* the field, and
Day's toolbar model is a flat list of items with no "below the bar" slot. The field currently sits
inside an [`NSSearchToolbarItem`](https://developer.apple.com/documentation/appkit/nssearchtoolbaritem) / an [`AdwHeaderBar`](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.HeaderBar.html) entry / a [`QToolBar`](https://doc.qt.io/qt-6/qtoolbar.html) widget, none of which can
host one.

Scopes therefore wait on the **Inline** placement, where the field is attached to the navigation
surface and the bar can sit under it. When they land, the per-toolkit mapping is:

| toolkit | scope bar | `Cap::SearchScopes` |
|---|---|---|
| ios-uikit | [`UISearchBar.scopeButtonTitles`](https://developer.apple.com/documentation/uikit/uisearchbar) | `Native` |
| android-mdc | `ChipGroup` of single-selection filter chips | `Emulated` |
| harmony-arkui | `SegmentButtonV2` | `Emulated` |
| macos-appkit | [`NSSegmentedControl`](https://developer.apple.com/documentation/appkit/nssegmentedcontrol) | `Emulated` |
| linux-gtk | `.linked` toggle buttons | `Emulated` |
| linux-qt | a button row | `Emulated` |
| web-dom | a radio group | `Emulated` |
| windows-winui | a [`ToggleButton`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.primitives.togglebutton) row | `Emulated` |

`Emulated` covers two different situations there: a real native component doing this job (the
chips, `SegmentButtonV2`, [`NSSegmentedControl`](https://developer.apple.com/documentation/appkit/nssegmentedcontrol)) and a bar composed from primitives (web, WinUI).
Neither claims to be the platform's scope bar.

## Suggestions complete the field

On a navigation surface the list **is** the result set. So [`Search::suggestions`](https://daybrite.dev/api/day_pieces/struct.Search.html#method.suggestions) offers
completions that fill the field; it does not open an overlay of results, which would cover the
list it is narrowing.

Day uses the search widget's own completion affordance, and only that: a completion popup Day drew
itself would not match the platform's keyboard handling or styling, and a search field that behaves
differently from every other one on the system is worse than no completions.

| toolkit | completions |
|---|---|
| linux-qt | [`QCompleter`](https://doc.qt.io/qt-6/qcompleter.html) (native popup, case-insensitive) |
| web-dom | `<datalist>` (the browser's own popup) |
| windows-winui | [`AutoSuggestBox.ItemsSource`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.autosuggestbox) |
| ios-uikit | [`UISearchResultsUpdating`](https://developer.apple.com/documentation/uikit/uisearchresultsupdating), at any placement — see below |
| macos-appkit | **none.** [`NSSearchField`](https://developer.apple.com/documentation/appkit/nssearchfield)'s menu is a recents list, not completions for the current text, so Day does not present it as one |
| linux-gtk | **none.** GTK4 deprecated [`GtkEntryCompletion`](https://docs.gtk.org/gtk4/class.EntryCompletion.html) and [`GtkSearchEntry`](https://docs.gtk.org/gtk4/class.SearchEntry.html) has no replacement |
| android-mdc, harmony-arkui | with the Inline placement |

Choosing a suggestion puts it in the field and emits the same change any keystroke would, so an app
that only reads `query` needs no extra handling. `Event::SearchSuggestionChosen` says *which* one,
for an app that wants to act on the choice itself.

## What is searchable

A navigation surface ([`Nav::searchable`](https://daybrite.dev/api/day_pieces/struct.Nav.html#method.searchable)), and a [`nav_stack`](https://daybrite.dev/api/day_pieces/fn.nav_stack.html) page ([`Decorate::searchable`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.searchable), below).
Search on arbitrary content is out of scope, because every toolkit would need a placement answer for
content that is not navigation chrome.

## Page search

A page that lists something the user searches, such as a catalog or a mailbox, declares the field
on itself, as it declares its [`.toolbar(…)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.toolbar):

```rust
results_page(feed)
    .searchable(
        Search::new(query)
            .prompt(res::str::search())
            .on_submit(move |text| run_search(text))
            .available(move || feed_loaded.get() && feed_can_search.get()),
    )
```

The field is the reserved `day::search` item on the page's own chrome, so it follows the page:
it shows while the page is on top of its stack and leaves when another page is pushed over it or
the page is popped. Each toolkit draws it where its platform puts a page's search:

| toolkit | page search | submit |
|---|---|---|
| ios-uikit | the page's own [`UISearchController`](https://developer.apple.com/documentation/uikit/uisearchcontroller) on its `navigationItem`, stacked under the title as Mail's mailbox does: shown when the field arrives, then hidden on scroll and back on a pull; Cancel clears the query. One controller per page for the page's life, so a page covered by a push keeps its field and text | the keyboard's Search key |
| android-mdc | a collapsible [`androidx.appcompat.widget.SearchView`](https://developer.android.com/reference/androidx/appcompat/widget/SearchView) action on the page's app bar: the magnifier expands into a field across the bar; the collapse arrow or system Back closes it and clears the query | the keyboard's Search key |
| harmony-arkui | an ArkUI `Search` at the top of the showing page, under its title bar | `Search.onSubmit` |
| macos-appkit | the [`NSSearchToolbarItem`](https://developer.apple.com/documentation/appkit/nssearchtoolbaritem) at the trailing edge of the window toolbar | Return or keypad Enter, taken by the field (`insertNewline:`), so no default button fires and editing continues |
| linux-gtk | a [`GtkSearchEntry`](https://docs.gtk.org/gtk4/class.SearchEntry.html) in the window's [`AdwHeaderBar`](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.HeaderBar.html) | Enter (`activate`) |
| linux-qt | a [`QLineEdit`](https://doc.qt.io/qt-6/qlineedit.html) on the window's [`QToolBar`](https://doc.qt.io/qt-6/qtoolbar.html), with a clear button and a [`QCompleter`](https://doc.qt.io/qt-6/qcompleter.html) popup | Return (`returnPressed`); Return on a completion applies it first |
| windows-winui | an [`AutoSuggestBox`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.autosuggestbox) in the window toolbar | `QuerySubmitted`: Return, the query glyph, or a picked suggestion |
| web-dom | an `<input type=search enterkeyhint=search>` in the docked toolbar strip, outside any form | Return, outside IME composition, after any `<datalist>` completion it accepted |

The desktops and the web draw one bar per window, so the field rides it while its page is on top
of its stack; Android keeps an open field through repaints of its bar and brings it back open when
its page returns with a query in force.

[`Search::available`](https://daybrite.dev/api/day_pieces/struct.Search.html#method.available) offers the field only while it holds, for a page that learns whether it can be
searched after it shows (a feed that loads first). A field withdrawn and offered again keeps its
text. Scopes, reveal and placement belong to a surface's search and do not apply to a page's.

## Submitting

[`Search::on_submit`](https://daybrite.dev/api/day_pieces/struct.Search.html#method.on_submit) runs when the user presses the field's Return or Search key, with the text as
submitted; the query already holds it. It is the trigger for a search the server runs: a filter
reads the query as it changes, but a catalog query should not run on every keystroke. A field on
the bar carries it as `ToolbarValue::Submit` after the last `ToolbarValue::Text`; an inline surface
field emits `Event::SearchSubmitted` (the inline fields of ios-uikit, android-mdc and
harmony-arkui, on the keyboard's Search key). A field sends any text the app has not heard yet
before it submits, so the query is current when [`on_submit`](https://daybrite.dev/api/day_pieces/struct.Search.html#method.on_submit) runs.

dayscript submits with `toolbar: { item: day::search, text: "…", submit: true }`. The step paints
the field with the text first, as `input:` does for a text field, so a screenshot shows what the
app is searching for.

## Two-way, through the signal

Both directions go through the app's `Signal`, never through the widget:

- the user typing emits `Event::SearchChanged`, which writes the signal;
- the app writing the signal patches the live field (`SearchPatch::Text`) rather than rebuilding
  it, so a sync never steals focus or resets the insertion point mid-word.

That discipline will let the field change placement later without the query noticing.

## Platform guidance

- [Apple: search fields](https://developer.apple.com/design/human-interface-guidelines/search-fields)
- [Android: search interface](https://developer.android.com/develop/ui/views/search/search-dialog)
