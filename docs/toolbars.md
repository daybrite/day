---
title: "Window toolbars"
description: "Native window toolbars: items, search, overflow, and per-platform presentation."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Toolbars

> **Status: implemented** on every backend. A toolbar is chrome, not a piece: it does not live in
> the tree and day does not lay it out. Each backend realizes the model with its platform's own
> bar — `NSToolbar`, `AdwHeaderBar`, `QToolBar`, `CommandBar`, a `UINavigationItem`, a Material
> app bar's menu, a `Navigation`'s `.menus()`, a drawn strip on the web.

## The rule

**Where you declare an item is where it appears, and how long the declaring piece lives is how
long it stays.** Everything else follows from that one sentence.

```rust
use day::prelude::*;

reader_page(article).toolbar([
    toolbar_button("next-unread", tr("next-unread")).icon(Symbol::Down).action(open_next),
    toolbar_toggle("star", tr("star"), article.is_starred).icon(Symbol::Star),
])
```

| Declared on | Rides |
|---|---|
| the window's root piece | every page of that window |
| a `selector` host (`Selector::toolbar`) | its sidebar column, and the root list when collapsed |
| a content-list pane | the list column, and the middle layer when collapsed |
| a destination page | the detail column, and every page pushed onto it |
| any piece inside a page | that page's chrome |

Nothing declares which bar it belongs to, and nothing declares when to hide: a command leaves the
bar when the content it acts on leaves the screen. A collapsed content-list pane, a destination
the selection has moved off, and a list a detail has been pushed over all take their commands with
them. That is what a window's bar showing a page's commands one level out of step used to be.

[`Decorate::toolbar`] takes one item, a list of them, an array, or a closure that derives the list
and re-runs whenever its reactive reads change — one name for all four, through a disjoint marker
parameter (the same `IntoText` shape §12.2 uses). There is no separate reactive spelling.

```rust
page.toolbar(one_item)
page.toolbar([a, b, c])
page.toolbar(vec![a, b, c])
page.toolbar(move || vec![…])          // re-derived on change
```

An item that should come and go is a piece that comes and goes — `when` adds and withdraws it
through the same scope disposal that tears down any other subtree. Use `.enabled_when(…)` instead
where the command should stay visible but unavailable; that patches the one item rather than
rebuilding the bar.

## Placement

Placement names an item's ROLE on the chrome carrying it, never a surface: which bar it rides
already follows from where it was declared.

| Placement | Desktop toolbar | iOS | Android | web-dom |
|---|---|---|---|---|
| `Automatic` (default) | after the leading group | trailing item group | menu action, `IF_ROOM` | strip, declaration order |
| `Navigation` | leading edge of its column | `leftBarButtonItems`, after the back button | leading | leading |
| `Principal` | centered in its column | `titleView` | the bar's centered slot | centered |
| `Primary` | trailing, before secondaries | trailing, folds last | `SHOW_AS_ACTION_ALWAYS` | trailing |
| `Secondary` | trailing, after primaries | folds into ⋯ first | `SHOW_AS_ACTION_NEVER` | overflow |
| `Bottom` | falls back to `Secondary` | `toolbarItems`, the bottom bar | falls back to `Secondary` | falls back |

There is deliberately no `Cancel`/`Confirm`: a modal's affirmative and dismissive buttons are
dialog buttons with their own roles ([docs/dialogs.md](dialogs.md)), and a second, weaker spelling
of the same idea would leave two right answers for one question.

`.label_style(…)` chooses the title, the icon, or both where a platform can draw more than one; an
item folded into an overflow menu shows its title whatever it asks for, because a menu row with no
words is not a menu row. `.prominent()` asks for the platform's emphasized style.

## Columns

A desktop toolbar spans every column of a split window at once, and a three-pane app expects each
column's commands to sit over that column — Mail, Notes and Finder all do. Day knows which column
an item came from, because the declaration site already said so, and stamps it on the item.
**An app never writes it.**

`macos-appkit` realizes it with a tracking separator at each divider: AppKit vends
`NSToolbarSidebarTrackingSeparatorItemIdentifier` for the sidebar's, and Day builds the second
with `NSTrackingSeparatorToolbarItem` bound to the split at divider 1. Windows carry
`NSWindowStyleMaskFullSizeContentView` so those items can find their dividers. The sidebar's
commands pack against its trailing edge; every other column packs leading roles first, then the
trailing ones at that column's own right edge. `web-dom` does the same with three flex tracks
whose widths follow the panes'. Qt does it inside its one `QToolBar`: three track widgets, the
sidebar's and the list's given the width of their splitter pane every time the splitter lays
out, the detail's taking the rest — the same packing within each. A window with no navigation
splitter (a settings window, a stack-only app) packs one flat bar by placement.

Everywhere else the column is DROPPED, never the item: GTK draws one header bar with no divider
to track, XAML's `CommandBar` spans the window, and ArkUI's `.menus()` is a flat list.
Degradation always removes the specialization and keeps the command.

## The sidebar affordance

A `selector(Sidebar)` supplies its own, so an app declares nothing for it. It reaches the backends
as an ordinary button under the reserved id `day_spec::SIDEBAR_TOGGLE_ID` whose action names the
host it was built for (`Toolkit::toggle_sidebar(host)`), so a second window's button collapses
that window's sidebar and a dayscript `toolbar:` step presses it like any other item. Each
backend does what its platform expects: AppKit swaps in `NSToolbarToggleSidebarItemIdentifier`,
Qt keeps the button on the bar when its pane collapses (the track shrinks to it), XAML drops it because
`NavigationView` draws its own pane button, and **UIKit drops it entirely** — `UISplitViewController`
and `.tabSidebar` each supply one, and Day's copy was both dead on a phone and doubled on an iPad.
Suppress it with `.sidebar_toggle(false)`.

### The items

| constructor | what it is |
|---|---|
| `toolbar_button(id, label)` | a command |
| `toolbar_toggle(id, label, signal)` | a two-state button, bound two-way |
| `toolbar_segmented(id, segments, signal)` | one native segmented control over a `Signal<usize>` |
| `toolbar_menu(id, label, entries)` | a pull-down, from the same `MenuEntry`s the menu bar takes |
| `toolbar_label(id, text)` | static text — a status or a caption |
| `toolbar_separator()` | a divider between neighbors in the same placement bucket |

There are no spacers. Alignment is [placement](#placement), which is a fact about what the command
IS rather than about where it happens to sit in a list, and it survives a bar that has to fold.

**Search is declared elsewhere.** It belongs to the navigation surface it filters
(`Selector::searchable`, [docs/search.md](search.md)), and Day merges the resulting field into this bar under the
reserved id `day.search`. Declaring it on the surface lets the platform move it (into the
navigation list on a window too narrow for a sidebar) without the app re-declaring anything.

Modifiers: `.icon(Symbol)`, `.image(name)`, `.action(f)`, `.tooltip(t)`, `.enabled(bool)`,
`.enabled_when(f)`, `.placement(…)`, `.label_style(…)`, `.prominent()`.

**Use `toolbar_segmented` wherever exactly one of a set is on at a time**, such as a theme
chooser or a view mode. Three toggles instead say "three independent switches" to the eye and to
a screen reader, leave the app to keep them exclusive, and take three times the width:

```rust
toolbar_segmented("theme", vec![
    segment(tr("light")).icon(Symbol::Light),
    segment(tr("system")).icon(Symbol::Auto),
    segment(tr("dark")).icon(Symbol::Dark),
], mode)   // mode: Signal<usize>
```

Each backend draws the control its platform already has: `NSSegmentedControl` in `selectOne`
tracking on AppKit, a `.linked` box of grouped toggle buttons on GTK, an exclusive `QButtonGroup`
on Qt, a tight `ToggleButton` row inside one `AppBarElementContainer` on XAML, and the same
`.day-segmented` element the picker piece uses on the web. The control enforces exclusivity; the
signal only ever holds the chosen index.

Every item takes an `id`. It is the item's identity everywhere: the native item identifier, the
dayscript target, and the key a targeted update addresses. Ids are unique within a bar.

### What rebuilds and what patches

A full install replaces the bar. That is the wrong path for a value that changes as the user
types: rebuilding would drop the search field's focus mid-word. So the values that change often
ride their own bindings and patch a single item instead:

- a `toolbar_toggle`'s signal
- a `.searchable()` surface's query signal ([docs/search.md](search.md))
- `.enabled_when(…)`

Keep those out of a derived builder's reactive reads. Put structure there: which
items exist, and their labels.

### Icons

`.icon(Symbol::…)` names what the icon means, and each backend draws its platform's own glyph:
an SF Symbol on macOS, a freedesktop icon name on GTK and Qt, a Segoe Fluent glyph on Windows.
This is the only way one icon looks native on four desktops; a bundled PNG cannot, because it is
one artist's take on all of them. Use `.image(name)` only for something app-specific.

`.image(name)` takes either a `resource/images/` file or a `resource/vectors/` glyph, the same
names the rest of the app uses. The vector is tried first, because on AppKit a vector asset stages
as an SVG and nothing else: looking only for a raster found nothing and the item silently fell back
to drawing its label, a button reading "Star" where a star belonged. Bundled glyphs are templates,
so each backend tints them to the bar's own foreground (Qt does this explicitly, since an untinted
template is a flat black shape, invisible on a dark toolbar).

On the web there is no system icon set to borrow, so day-dom draws the standard symbols itself,
as inline-SVG `data:` URLs through the same CSS mask a bundled image uses. They are plain
geometry authored in day rather than a third-party icon set, which keeps the framework free of an
icon license. Before that, `Icon::Symbol` was dropped on the web entirely and only items carrying
a bundled image had a glyph, so a bar mixed icons and words.

`Symbol` is `#[non_exhaustive]`. A backend that has no glyph for a symbol draws none and the item
falls back to its label, never to a broken-image placeholder. GTK additionally checks the running
icon theme before setting a name, because icon themes vary in completeness and a missing name
paints GTK's broken-image glyph.

### What `Cap::Toolbar` means now

```rust
capability(Cap::Toolbar)   // does this platform have WINDOW-LEVEL chrome that persists?
```

It no longer decides whether a command can be SHOWN. Every platform has somewhere to draw a
contribution — a title bar, a navigation bar, a Material app bar, a `Navigation`'s `.menus()`, a
drawn strip — so an app never needs a fallback branch for "there is no toolbar here", and the ones
that had them have been deleted. `Native` on the desktops and the phones, `Emulated` on web-dom
(a strip docked above the app root, since a browser tab has no title bar to hang chrome on) and on
HarmonyOS (the bar belongs to the navigation destination, not the window). Probe it only for a
layout decision that really turns on a persistent bar existing — Day-Tunes chooses between a
toolbar transport and a Now Playing tab that way.

A window whose content is not a navigation host anywhere — a canvas or a form filling the window —
has no page bar to put items on, so both phones give it one: iOS a navigation bar of the window's
own across the top under the status bar, Android a Material app bar in the same place.

On iOS the items ride the NAVIGATION BAR of the page that is showing, as item groups (one per
item) so what the bar cannot fit folds into its overflow, trailing items first. A leading item
SUPPLEMENTS the back button rather than replacing it (`leftItemsSupplementBackButton`, the flag
SwiftUI sets for the same reason).

On Android they go in as menu items on the nav host's app bar. The bar rests at `colorSurface`
with `AppBarLayout` LIFT ON SCROLL, which is Material 3's answer to the same question iOS answers
by blending its bar into the content: flat and continuous with the panes at rest, tonally lifted
only once content scrolls beneath it. A `colorPrimary` band is the Material 2 look and reads, on a
tiled tablet, as a stripe between the status bar and the panes. Only the OUTERMOST navigation host
carries the window's items — a window can hold several, and giving each of them the same items
painted a second app bar directly under the first.

Buttons, toggles and labels draw as themselves, a menu item drops its menu, and a segmented item
becomes a pull-down of its segments with the chosen one checked (a segmented control has no room
in a phone's bar). That pull-down is titled by the segment IN FORCE, since a segmented control
carries no label of its own: on iOS by that segment's icon where it has one, otherwise its word;
on Android by its word.

**A folded item keeps its name.** What the overflow shows for an item is its localized `label`
and its icon, never the icon alone — on iOS through the `menuRepresentation` Day gives every
item, because a bar button built from an image carries no title of its own and the recorder's
Record and Play folded away to two bare glyphs. A toggle folds to a checked row, a pull-down to
a titled submenu of the same children, and a segmented item to its segments under the name of
the segment in force. Tapping the row runs what the button would have run, the toggle's own
flip included. Two kinds never reach a phone's bar: search, which rides the navigation
list there ([docs/search.md](search.md)), and the sidebar toggle, which the split view owns.
Android stages no glyph for a `Symbol`, so an item with only a symbol has no glyph to show in
the bar and lives in the overflow, where its label reads as a menu row — a Material app bar
carries icon buttons and sends the rest to its overflow, and two text actions were enough to
squeeze the Showcase's own title to "Day Showc…". An `Icon::Image` draws as the image on both
phones, and Android re-tints it to the app bar's own color.


## Per-backend native realization

| | AppKit | GTK | Qt | XAML | UIKit | Android | ArkUI | web-dom |
|---|---|---|---|---|---|---|---|---|
| placement | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ |
| column | ✓ | — | ✓ | — | — | — | — | ✓ |

| | AppKit | GTK | Qt | XAML |
|---|---|---|---|---|
| bar | `NSToolbar`, unified style | the window's `AdwHeaderBar` | `QToolBar` | `CommandBar` |
| button | `NSToolbarItem` (bordered) | flat `GtkButton` | `QAction` | `AppBarButton` |
| toggle | `NSButton` push-on/push-off | `GtkToggleButton` | checkable `QAction` | `AppBarToggleButton` |
| menu | `NSMenuToolbarItem` | `GtkMenuButton` + `GMenu` | `QToolButton` (InstantPopup) + `QMenu` | `AppBarButton` + `MenuFlyout` |
| search | `NSSearchToolbarItem` | `GtkSearchEntry` | `QLineEdit` (clear button + find action) | `AutoSuggestBox` |
| separator | *(none — a fixed space)* | `GtkSeparator` | `QToolBar::addSeparator` | `AppBarSeparator` |
| icons | SF Symbols | freedesktop symbolic names | `QIcon::fromTheme`, then `QStyle` standard pixmaps | Segoe Fluent glyphs |

Notes that are not obvious from the table:

- **AppKit**: macOS toolbars have no separator item, so `toolbar_separator()` renders as the
  system's own fixed space, which is what macOS uses between groups. The toolbar is created once
  per window and reused across installs (a replaced `NSToolbar` flashes the title bar and drops
  focus). User customization is off: the item list is app-declared and reactive, so an autosaved
  arrangement would be in permanent conflict with the next install. Installing or removing a
  toolbar resizes the content view without a window resize, so the backend reports the new
  content size itself.
- **GTK**: could express COLUMNS — `AdwOverlaySplitView` with a per-pane `AdwHeaderBar` is the
  GNOME idiom, and Nautilus and Text Editor both do it. Day does not yet; the column is dropped
  and one header bar carries everything. That is a gap, not a toolkit limit.
- **GTK**: GNOME has no separate toolbar. The header bar is the toolbar, and GTK4 removed
  `GtkToolbar` outright, so items pack into the `AdwHeaderBar` the window already has, around the
  title. Buttons get the `flat` class, per the GNOME HIG. `pack_end` grows right-to-left, so the
  trailing group is packed in reverse to reach the screen in the order the app wrote it.
- **Qt**: the bar is a `QToolBar` parented to the window and laid out with the menu bar, not a
  `QMainWindow` dock; the geometry there is already hand-managed. It is a real `QToolBar` either
  way: it takes its icon size and its icon/text style from the user's Qt settings, which is the
  KDE convention and why the backend sets neither. It does not get dragging between dock
  areas, which needs `QMainWindow`. Columns are three plain widgets on that bar, each with a
  row layout; an action inside one is the same `QAction` shown through an auto-raise
  `QToolButton` of the bar's own style, so patches by action are unchanged. A re-lower releases
  the previous actions and their widgets (`QToolBar::clear` only removes them). Icons: Qt has no
  glyph set of its own beyond QStyle's few dialog bitmaps, so a symbol is the desktop theme's
  icon where one exists (a freedesktop theme on Linux; on macOS Qt 6.7+ maps the freedesktop
  names it knows to SF Symbols), then Day's own outline, then QStyle's. Those drawings never
  agreed on a box, so every toolbar glyph is fitted by its ink to the same fraction of the bar's
  icon box and tinted to the palette text color; the box is 24 points on macOS (an NSToolbar
  glyph's), and the user's setting on the Linux desktops.
- **XAML**: `CommandBar` right-aligns `PrimaryCommands`, left-aligns `Content` and folds
  `SecondaryCommands` into its overflow — which is exactly the three groups Day's placements
  reduce to, so `Navigation`/`Principal` land in `Content`, `Automatic`/`Primary` in
  `PrimaryCommands` and `Secondary` in the overflow. One divergence: system XAML's
  `PrimaryCommands` accepts
  only `ICommandBarElement`, so a search field, a label or a fixed space cannot go there. Those
  three always render in `Content` (on the leading side) whatever placement they asked for. A
  trailing search field therefore sits left on Windows and right on the other three. That limit is
  the toolkit's; the alternative would be drawing a search box by hand,
  which this design does not do. XAML is Windows-only, so it is built and exercised in CI rather
  than on a developer's Mac or Linux box. Secondary windows get no toolbar there, the
  same as the menu bar: this shim's chrome lives on the primary window only.

## Re-installing the same bar

A derived contribution re-runs whenever anything it reads changes, with freshly registered closures
every time, since the ids come from `register_toolbar_value` / `register_menu_action`. Handing that
to a backend rebuilds the native bar, which is invisible for a button and destructive for the
search field: recreating the widget takes the keyboard focus and the caret with it. Typing a letter
that moved the nav selection re-ran the page build and threw away the field being typed into, on
every backend that rebuilds what it is handed, which is all of them.

`set_toolbar` therefore compares the incoming model with the installed one, ignoring what
cannot matter to the widgets: the action ids, and the search field's live text and completions
(kept current through `ToolbarPatch::Text`/`Suggestions`, never through a rebuild). Same items in
the same order, with the same kinds, labels, icons and enablement, means the native bar is already
correct, so the new closures are moved onto the ids it already carries and no toolkit call is made
at all. Anything else is a real change and installs as before.

This is why a backend never has to preserve focus across an install: an install that would have
disturbed the focus does not happen.

## One model per window

Every contribution — the window's own and each showing page's — is composed into ONE model per
window before it crosses to the toolkit, so a backend draws what it has always drawn and never has
to know that a page contributed any of it. There is deliberately no per-page model: one authority,
so a live `enabled_when` patch and a re-compose cannot disagree. (They did once, and a page command
declared while nothing was selected stayed disabled on a window that never re-composed afterwards —
a tiled Android tablet, where selecting a row changes no chrome. Day-Rise's
`dayscript/toolbar-enable.yaml` is that case, kept.)

The corollary for scripting: `toolbar:` can only reach what is actually ON the bar. A step that
drives a list pane's command has to run while that list is showing.

## Events

A button and a menu item ride the **menu action rail**: they emit `Event::MenuAction(id)` from the
same registry [menus](menus.md) uses, so one closure can back both a toolbar button and its
menu-bar twin. A toggle or a search field emits `Event::ToolbarChanged { action, value }` with a
`ToolbarValue`, which day-core routes to the value callback registered for the id.

## Scripting

```yaml
- toolbar: { item: refresh }                  # run a button's command
- toolbar: { item: search, text: "swift" }    # type into a search item
- toolbar: { item: search, key: nav_stack }   # …or type a Fluent key resolved in the RUN'S locale
- toolbar: { item: star, on: true }           # set a toggle
- toolbar: { item: theme, index: 2 }          # choose a segment
```

`index:` is required for a segmented item and `on:` for a toggle, for the same reason. A toggle's action is registered in the value registry rather than
the menu-action one, so a bare `toolbar: { item }` on one used to dispatch into the wrong registry
and do nothing at all; the step passed, the app never moved, and the script went on asserting
against a state it had not reached. The step now refuses it and says which argument is missing.

The step resolves the item in the current model and dispatches its
action, so it passes even if the native control is still bound to a previous model's action, the
failure mode a real keystroke hits. A backend that rebuilds its bar must rebind the live controls,
not just diff the identifier list (day-appkit had exactly this bug: after a locale change the
search field dispatched an action id day-core had already swept, so typing did nothing).

The step goes through the same dispatch the native control fires, so it exercises the app's
wiring end to end. It does **not** prove the native widget drew; a screenshot does. The step
fails on an unknown item (retryable, since a reactive bar may not have installed yet), on a
disabled item, and on an item with no command.

## Verification

The showcase **Toolbars** page (`pages/toolbars.rs`) installs the main window's own toolbar with
every item kind, and drives the whole API from the page: add and remove an item, enable and
disable one, write both bound signals, and read back what the bar did. The walkthrough runs a
button, types into the search field, sets the toggle, adds the optional item and runs it, then
disables one and clears the search, asserting the page's live readouts after each.

Day-Rise carries the applied version, and it is the three-column case: the sidebar host's own
commands, the content-list pane's filter and add, and the editor's Done, each declared on the piece
it acts on. `dayscript/demo.yaml` drives all three, and `dayscript/toolbar-enable.yaml` is the
regression for a page command's live enablement.

Verified by running Day-Rise's demo on macos-appkit, macos-gtk, macos-qt, web-dom, ios-uikit
(iPhone and iPad) and android-mdc (phone and tablet), and the Showcase walkthrough on
macos-appkit and web-dom. A green `toolbar:` step proves the MODEL, never the pixels — an
offscreen snapshot cannot show the title bar on AppKit — so any change to a placement path is
also checked by capturing the real window.

## Follow-ups

- macOS toolbar customization, which needs the model and an autosaved arrangement to be
  reconciled rather than in conflict.
- Qt dock-area dragging, which needs `DayWindow` to become a `QMainWindow`.
- GTK columns, through `AdwOverlaySplitView` with a per-pane `AdwHeaderBar` (see the backend
  notes) — the only platform where the column is dropped for want of work rather than for want of
  an API.
- Contributions order by registration, not by tree position: a `when` arm switching on late
  appends within its placement bucket rather than inserting where it sits.
- `web-dom` measures the panes once per re-lower, so a track's width is stale until the next one.
- On an iPad the backend infers whether a tab page will bring its own navigation host from the
  tab bar's horizontal size class — the same question `gated_detail_piece` asks in the pieces
  layer. Two layers deriving one fact; the fix is for the tabs presentation to build each
  destination as a navigation host, as SwiftUI's `TabView { NavigationStack { … } }` does.
