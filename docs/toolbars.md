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

For an operation shared across surfaces, use a [reusable `Command`](commands.md) to define its
title, availability, optional check state, icon, shortcut, and handler once.

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
| a nav host (`Nav::toolbar`) | its sidebar column, and the root list when collapsed |
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

On UIKit, a `toolbar_label` with `Principal` placement uses a native single-line title label.
Removing that contribution restores the navigation item’s ordinary title. Only the first
principal item is shown.

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

A `nav(Sidebar)` supplies its own, so an app declares nothing for it. It reaches the backends
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
| `toolbar_separator(id)` | a divider between neighbors in the same placement bucket |

There are no spacers. Alignment is [placement](#placement), which is a fact about what the command
IS rather than about where it happens to sit in a list, and it survives a bar that has to fold.

**Search is declared elsewhere.** It belongs to the navigation surface it filters
(`Nav::searchable`, [docs/search.md](search.md)), and Day merges the resulting field into this bar under the
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
on Qt, a tight row of compact `AppBarToggleButton`s inside one `AppBarElementContainer` on XAML
(a checkable menu when it folds into the overflow), and the same `.day-segmented` element the
picker piece uses on the web. The control enforces exclusivity; the signal only ever holds the
chosen index.

Every item takes an `id`. It is the item's identity everywhere: the native item identifier, the
dayscript target, and the key a targeted update addresses. Ids are unique within a bar.

### What changes the bar, and how

The bar is never rebuilt. Day hands a backend its bar as **edits**, the way the view tree
arrives as `insert`/`remove` rather than as a new tree: each `ToolbarOp` adds or removes one
item, and an item no op names keeps its native widget. That is what keeps a search field focused
while the page around it changes.

- **Structure** (which items exist, in what order, and how each is drawn: its kind, label,
  icon, placement) changes by edits. Day compares the bar the window should now carry with the one
  it carries, item by item by id, and sends only the difference: removals, then insertions. An
  item drawn differently (a new label, say) is removed and inserted alone, and so is one that
  moved; a reshuffle moves the fewest items it can.
- **Values** (a toggle's state, a segment's selection, enablement, a search field's text and
  completions) change by `ToolbarPatch` on the live item, never by an edit. These ride their own
  bindings, so keep them out of a derived builder's reactive reads and put structure there:
  - a `toolbar_toggle`'s signal
  - a `.searchable()` surface's query signal ([docs/search.md](search.md))
  - `.enabled_when(…)`

  A value the user changes (typing, a click) is recorded in Day's model as it is reported, so the
  model never differs from what the widget shows.

Day keeps two copies of each window's bar: the one it should carry (what dayscript reads) and the
one its toolkit has. An edit is computed against the second, and recorded there only once the
toolkit took it. So an edit that could not be delivered (the window's root had no native handle
yet) is not lost: the next recompose sends what the toolkit is missing, the way a whole-bar install
used to heal on the next change.

Two things keep ordinary activity from producing edits at all:

- **One edit per turn.** Contributions change in bursts: swapping the page withdraws the old
  page's commands and adds the new page's. Day composes the bar once, when the turn settles
  (`day_reactive::at_turn_end`), so the in-between bar is never sent.
- **Stable dispatch ids.** Each item reaches the toolkit under a slot id kept for that item id in
  that window, never under its closure's id. A page rebuilt with the same commands and fresh
  closures composes the same bar, so nothing is sent; the slot is pointed at the new closure, and
  the native item keeps dispatching the id it already has.

Every item needs an id, unique within the window's bar (a separator too: `toolbar_separator(id)`).
The id is the item's identity across edits; if two showing pages declare the same one, the first
is kept and Day logs a warning.

### Icons

`.icon(Symbol::…)` names what the icon means, and each backend draws its platform's glyph:
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
Android draws a `Symbol` from day-android's own glyph set — one Material Symbols vector per
variant, shipped in the toolkit's `res/` and named `day_symbol_<variant>` — so a symbol-only
item has an icon to show in the bar, the way an SF Symbol gives it one on Apple. An item with
no icon at all still lives in the overflow, where its label reads as a menu row: a Material
app bar carries icon buttons and sends the rest to its overflow, and two text actions were
enough to squeeze the Showcase's own title to "Day Showc…". A segmented control shows in the
bar as one icon button, the segment in force's glyph, which opens the choices as a radio
submenu; the glyph follows the selection. An `Icon::Image` draws as the image on both phones,
and Android re-tints it to the app bar's own color. The bar has room for a few icons beside
the page's title, and the page's own commands take them first: Android orders the detail and
list columns' items ahead of the sidebar column's, so a page's Show Source and Star stay in the
bar and the window's New Window and appearance chooser fold into the overflow when the bar is
that narrow.


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

- **ArkUI**: the primary window's toolbar rides `Navigation` / `NavDestination.menus`.
  A custom menu builder uses native `Button`, `Menu`, and `MenuItem` components: template
  images explicitly use system foreground colors (the array API's `icon` loads black SVGs
  literally in dark mode). Page commands take the visible slots before window commands;
  text-only and excess commands go into a labeled overflow popup. Toggles show an accent
  and expose selected state; segmented items open checked choices. System color resources
  update live on appearance changes. The title bar holds up to three 40vp buttons, with
  logical end padding. This remains `Support::Emulated`: placement/column layout, nested
  toolbar pull-downs, and secondary-window toolbars are not fully implemented. Pull-down
  entries still flatten into the action list. A future bottom-placement implementation
  should use native `toolbarConfiguration`, preserving its landscape adaptation.

- **AppKit**: macOS toolbars have no separator item, so `toolbar_separator(id)` renders as the
  system's own fixed space, which is what macOS uses between groups. The toolbar is created once
  per window and edited in place (a replaced `NSToolbar` flashes the title bar and drops
  focus). User customization is off: the item list is app-declared and reactive, so an autosaved
  arrangement would be in permanent conflict with the next edit. Adding or removing the toolbar
  resizes the content view without a window resize, so the backend reports the new content size
  itself.
- **GTK**: could express COLUMNS — a per-pane `AdwHeaderBar` over each pane of the navigation
  split is the GNOME idiom, and Nautilus and Text Editor both do it. Day does not yet; the
  column is dropped and one header bar carries everything. That is a gap, not a toolkit limit.
- **GTK**: GNOME has no separate toolbar. The header bar is the toolbar, and GTK4 removed
  `GtkToolbar` outright, so items go into the `AdwHeaderBar` the window already has, around the
  title: into two boxes Day packs at its start and end, so items go in and out one at a time
  without Adwaita's own children in the way. Buttons get the `flat` class, per the GNOME HIG.
- **Qt**: the bar is a `QToolBar` parented to the window and laid out with the menu bar, not a
  `QMainWindow` dock; the geometry there is already hand-managed. It is a real `QToolBar` either
  way: it takes its icon size and its icon/text style from the user's Qt settings, which is the
  KDE convention and why the backend sets neither. It does not get dragging between dock
  areas, which needs `QMainWindow`. Columns are three plain widgets on that bar, each with a
  row layout of three groups (leading, principal, trailing) that items go in and out of; a bar
  without columns is one such track across the whole bar. A button is a `QAction` shown through
  an auto-raise `QToolButton` of the bar's own style, so patches by action reach it directly.
  Items are kept per window, since two windows show the same ids. Icons: Qt has no
  glyph set of its own beyond QStyle's few dialog bitmaps, so a symbol is the desktop theme's
  icon where one exists (a freedesktop theme on Linux; on macOS Qt 6.7+ maps the freedesktop
  names it knows to SF Symbols), then Day's outline, then QStyle's. Those drawings never
  agreed on a box, so every toolbar glyph is fitted by its ink to the same fraction of the bar's
  icon box and tinted to the palette text color; the box is 24 points on macOS (an NSToolbar
  glyph's), and the user's setting on the Linux desktops.
- **XAML**: `CommandBar` right-aligns `PrimaryCommands`, left-aligns `Content` and folds
  `SecondaryCommands` into its overflow — which is exactly the three groups Day's placements
  reduce to, so `Navigation`/`Principal` land in `Content`, `Automatic`/`Primary` in
  `PrimaryCommands` and `Secondary` in the overflow. A crowded bar folds its right-hand commands
  by role: every `Automatic` one (right to left) before any `Primary` one, through a
  `DynamicOverflowOrder` per command, since the bar's own right-to-left fold ignored placement.
  The search field sits at the right end of the bar, just before the overflow button, where
  Windows apps keep it: an `AppBarElementContainer` carries it into `PrimaryCommands`, commands
  added later land to its left, and it is the last thing a narrowing bar folds. A label still
  renders in `Content` (on the leading side) whatever placement it asked for.
  A segmented item draws as compact `AppBarToggleButton`s, flat like the bar's other toggles;
  placed `Secondary`, it becomes one overflow command titled by the choice in force, whose flyout
  lists the choices as checkable rows. `.label_style(…)` is honored (`IconOnly` collapses the
  label, `TitleOnly` drops the icon); `.prominent()` is not drawn yet. The bar is docked only
  while it draws something: the sidebar toggle alone (which `NavigationView` draws itself) leaves
  no empty strip under the title bar. It comes first in tab order, before the content it acts
  on, and Tab wraps around the window rather than stopping at its last control. Secondary
  windows carry their own bar, like the primary.

## How a backend applies an edit

`Toolkit::edit_toolbar(h, ops)` gets the removals first, then the insertions in ascending
position, so applying them in order to the bar's current items yields the new bar. Every backend
keeps a `day_spec::ToolbarMirror` of its bar and applies each op to it, and then does one of two
things with its native bar:

- **A bar of live widgets** (GTK, Qt, AppKit, XAML, web-dom) removes the one native item a
  `Remove` names and builds one for an `Insert`, placing it right after
  `ToolbarMirror::prev_where` its own group (an end of the header bar, a column's leading,
  principal or trailing group, a region of the CommandBar). Nothing already on the bar moves.
  Where the groups depend on the window (Qt's and web-dom's column tracks follow the navigation
  split's panes), the backend keeps each item's column and order, and when the split arrives,
  leaves, or a pane collapses, moves just the affected widgets into their new groups, keeping
  the keyboard focus where it was.
  AppKit's native bar is NSToolbar's identifier list, with system spacers between the groups, so
  it removes the named items and then brings the identifier list to the one the model lays out,
  inserting new items and moving only spacers, which hold no state.
- **A bar of stateless actions** (Android's app-bar menu, UIKit's bar button items, ArkUI's
  title-bar `.menus()`) repaints its action list from the mirror. Nothing on it holds state a
  repaint could lose: on those platforms search lives on the navigation surface, never the bar.

A value patch goes to the live item, and into the mirror, so a later repaint shows it too.

## One model per window

Every contribution — the window's own and each showing page's — is composed into ONE model per
window before it crosses to the toolkit, so a backend draws what it has always drawn and never has
to know that a page contributed any of it. There is deliberately no per-page model: one authority,
so a live `enabled_when` patch and a re-compose cannot disagree. (They did once, and a page command
declared while nothing was selected stayed disabled on a window that never re-composed afterwards —
a tiled Android tablet, where selecting a row changes no chrome. Day-Rise's
`dayscript/toolbar-enable.yaml` is that case, kept.)

The corollary for scripting: `toolbar:` can only reach what is actually ON the bar. A step that
drives a list pane's command has to run while that list is showing. The one exception is the
reserved `day.search`, which the step also resolves to an inline field ([search](search.md)), so a
script types the same query on a phone.

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
- GTK columns, through a per-pane `AdwHeaderBar` over the navigation split's panes (see the
  backend notes) — the only platform where the column is dropped for want of work rather than for
  want of an API.
- Contributions order by registration, not by tree position: a `when` arm switching on late
  appends within its placement bucket rather than inserting where it sits.
- `web-dom` measures the panes once per re-lower, so a track's width is stale until the next one.
- On an iPad the backend infers whether a tab page will bring its own navigation host from the
  tab bar's horizontal size class — the same question `gated_detail_piece` asks in the pieces
  layer. Two layers deriving one fact; the fix is for the tabs presentation to build each
  destination as a navigation host, as SwiftUI's `TabView { NavigationStack { … } }` does.
