---
title: "Search"
description: "searchable(Search): a declared search surface each platform presents natively: toolbar field, navigation search, or list filter."
---

# Search

Search helps people reach content by describing what they want instead of browsing through every
destination. Placing search with the page or navigation it affects makes its scope easier to
understand.

Bind the query to app state with
[`.searchable()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.searchable);
the app decides which results to show.

```rust
nav(section)
    .style(NavStyle::Sidebar)
    .searchable(Search::new(query).prompt(res::str::search_sections())) // query: Signal<String>, two-way
    .items(nav_items(move || destinations_matching(query.get()), row))
```

Typing updates the query signal; writing the signal updates the field without rebuilding it.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Modifiers

| spelling | what it does |
|---|---|
| [`.searchable`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.searchable)`(`[`Search::new`](https://docs.rs/day-pieces/latest/day_pieces/struct.Search.html#method.new)`(query))` | makes the surface searchable, bound two-way to `query` (a `Signal<String>` or any binding) |
| [`Search::prompt(text)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Search.html#method.prompt) | the empty-state prompt; a localized string or closure follows the language and its signals |
| [`Search::placement(p)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Search.html#method.placement) | a placement **preference** (see below) |
| [`Search::reveal(SearchReveal)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Search.html#method.reveal) | how the field comes and goes on a phone (see below) |
| [`Search::scopes(scope, titles)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Search.html#method.scopes) | scope selection metadata; see the rendering limit below |
| [`Search::suggestions(f)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Search.html#method.suggestions) | completions for the current text |

## Find commands

Bind the app's localized Find command to
[`day::focus_search()`](https://docs.rs/day-app/latest/day/fn.focus_search.html), normally Cmd-F on
macOS or Ctrl-F elsewhere. It focuses the active searchable navigation surface and keeps its query.

## Placement and reveal

`SearchPlacement::Automatic` uses the window toolbar where it supports search, otherwise an
inline field on the navigation surface. `Toolbar` and `Inline` request a placement; the toolkit
can fall back when it cannot honor the request. A narrow desktop window can retain toolbar search.

[`Search::reveal`](https://docs.rs/day-pieces/latest/day_pieces/struct.Search.html#method.reveal)
controls a navigation surface's search field:

| Policy | Behavior |
| --- | --- |
| `Always` (default) | Keeps the field visible. |
| `Scrolls` | Lets supporting mobile toolkits collapse the field and title as the list scrolls. |
| `OnPull` | Requests an initially hidden field. Android honors this; UIKit treats it as `Scrolls`. |

Desktop toolbar search is unaffected by reveal policy. On UIKit, scrolling and returning to
the top control the field's visibility. On Android, tapping the scrolling search bar opens
its search view with the navigation list as results; choosing a result or pressing Back
collapses the search view. The query still belongs to the app.

## Search scopes

[`Search::scopes`](https://docs.rs/day-pieces/latest/day_pieces/struct.Search.html#method.scopes) accepts
scope titles and a selection binding, but the toolkits do not currently render a scope bar.
Use a separate [picker](/docs/reference/picker) when the app needs visible search filters.

## Suggestions complete the field

On a navigation surface the list **is** the result set. So [`Search::suggestions`](https://docs.rs/day-pieces/latest/day_pieces/struct.Search.html#method.suggestions) offers
completions that fill the field; it does not open an overlay of results, which would cover the
list it is narrowing.

Completion presentation depends on the toolkit:

| toolkit | completions |
|---|---|
| linux-qt | [`QCompleter`](https://doc.qt.io/qt-6/qcompleter.html) (native popup, case-insensitive) |
| web-dom | `<datalist>` (the browser's own popup) |
| windows-winui | [`AutoSuggestBox.ItemsSource`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.autosuggestbox) |
| ios-uikit | [`UISearchResultsUpdating`](https://developer.apple.com/documentation/uikit/uisearchresultsupdating), at any placement — see below |
| macos-appkit | **none.** [`NSSearchField`](https://developer.apple.com/documentation/appkit/nssearchfield)'s menu is a recents list, not completions for the current text, so Day does not present it as one |
| linux-gtk | **none.** GTK4 deprecated [`GtkEntryCompletion`](https://docs.gtk.org/gtk4/class.EntryCompletion.html) and [`GtkSearchEntry`](https://docs.gtk.org/gtk4/class.SearchEntry.html) has no replacement |
| android-material, harmony-arkui | with the Inline placement |

Choosing a suggestion puts it in the field and emits the same change any keystroke would, so an app
that only reads `query` needs no extra handling. `Event::SearchSuggestionChosen` says *which* one,
for an app that wants to act on the choice itself.

## Searchable content

A navigation surface ([`Nav::searchable`](https://docs.rs/day-pieces/latest/day_pieces/struct.Nav.html#method.searchable)), and a [`nav_stack`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav_stack.html) page ([`Decorate::searchable`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.searchable), below).
For a field within ordinary content, use [Search field](/docs/reference/searchfield).

## Page search

A page that lists something the user searches, such as a catalog or a mailbox, declares the field
on itself, as it declares its [`.toolbar(…)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.toolbar):

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
| android-material | a collapsible [`androidx.appcompat.widget.SearchView`](https://developer.android.com/reference/androidx/appcompat/widget/SearchView) action on the page's app bar: the magnifier expands into a field across the bar; the collapse arrow or system Back closes it and clears the query | the keyboard's Search key |
| harmony-arkui | an ArkUI `Search` at the top of the showing page, under its title bar | `Search.onSubmit` |
| macos-appkit | the [`NSSearchToolbarItem`](https://developer.apple.com/documentation/appkit/nssearchtoolbaritem) at the trailing edge of the window toolbar | Return or keypad Enter, taken by the field (`insertNewline:`), so no default button fires and editing continues |
| linux-gtk | a [`GtkSearchEntry`](https://docs.gtk.org/gtk4/class.SearchEntry.html) in the window's [`AdwHeaderBar`](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.HeaderBar.html) | Enter (`activate`) |
| linux-qt | a [`QLineEdit`](https://doc.qt.io/qt-6/qlineedit.html) on the window's [`QToolBar`](https://doc.qt.io/qt-6/qtoolbar.html), with a clear button and a [`QCompleter`](https://doc.qt.io/qt-6/qcompleter.html) popup | Return (`returnPressed`); Return on a completion applies it first |
| windows-winui | an [`AutoSuggestBox`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.autosuggestbox) in the window toolbar | `QuerySubmitted`: Return, the query glyph, or a picked suggestion |
| web-dom | an `<input type=search enterkeyhint=search>` in the docked toolbar strip, outside any form | Return, outside IME composition, after any `<datalist>` completion it accepted |

The desktops and the web draw one bar per window, so the field rides it while its page is on top
of its stack; Android keeps an open field through repaints of its bar and brings it back open when
its page returns with a query in force.

[`Search::available`](https://docs.rs/day-pieces/latest/day_pieces/struct.Search.html#method.available) offers the field only while it holds, for a page that learns whether it can be
searched after it shows (a feed that loads first). A field withdrawn and offered again keeps its
text. Scopes, reveal and placement belong to a surface's search and do not apply to a page's.

## Submitting

[`Search::on_submit`](https://docs.rs/day-pieces/latest/day_pieces/struct.Search.html#method.on_submit)
runs when the user presses Return or the keyboard's Search action. It receives the submitted
text, and the query binding already contains that text. Use this for an explicit server search;
for a live filter, derive the results from the changing query instead.

## Platform guidance

- [Apple: search fields](https://developer.apple.com/design/human-interface-guidelines/search-fields)
- [Android: search interface](https://developer.android.com/develop/ui/views/search/search-dialog)
