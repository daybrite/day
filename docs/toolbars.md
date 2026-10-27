---
navTitle: "Toolbars"
title: "Window toolbars"
description: "Native window toolbars: items, search, overflow, and per-platform presentation."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Toolbars

Put actions for the current window or page in its toolbar. Day places them in the platform’s
own bar and keeps them present for as long as the declaring content is visible.

For an operation shared across surfaces, use a [reusable `Command`](commands.md) to define its
title, availability, optional check state, icon, shortcut, and handler once.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).
A localized label, tooltip or search prompt follows a language change in place: a fixed list
relabels its registered items (`relabel_contribution`) and a derived list re-runs on the locale,
and either way the words reach the bar as `ToolbarPatch::Label` and `ToolbarPatch::Placeholder`
on the live item, which keeps its widget, its action, its focus and its typed text.

## Authoring

**Where you declare an item is where it appears, and how long the declaring piece lives is how
long it stays.** Everything else follows from that one sentence.

```rust
use day::prelude::*;

reader_page(article).toolbar([
    toolbar_button(id!(next_unread), res::str::next_unread())
        .icon(Symbol::Down)
        .action(open_next),
    toolbar_toggle(id!(star), res::str::star(), article.is_starred).icon(Symbol::Star),
])
```

| Declared on | Visible with |
|---|---|
| the window's root piece | every page of that window |
| a nav host ([`Nav::toolbar`](https://daybrite.dev/api/day_pieces/struct.Nav.html#method.toolbar)) | its sidebar column, and the root list when collapsed |
| a content-list pane | the list column, and the middle layer when collapsed |
| a destination page | the detail column, and every page pushed onto it |
| a page pushed onto a [`nav_stack`](https://daybrite.dev/api/day_pieces/fn.nav_stack.html) (or a standalone stack's root) | that page while it is on top; a page pushed over it takes its place |
| any piece inside a page | that page's chrome |

Nothing declares which bar it belongs to, and nothing declares when to hide: a command leaves the
bar when the content it acts on leaves the screen. A collapsed content-list pane, a destination
the selection has moved off, and a list a detail has been pushed over all take their commands with
them.

[`Decorate::toolbar`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.toolbar) takes one item, a list of them, an array, or a closure that derives the list
and re-runs whenever its reactive reads change.

```rust
page.toolbar(one_item)
page.toolbar([a, b, c])
page.toolbar(vec![a, b, c])
page.toolbar(move || vec![…])          // re-derived on change
```

An item that should come and go is a piece that comes and goes — [`when`](https://daybrite.dev/api/day_pieces/fn.when.html) adds and withdraws it
through the same scope disposal that tears down any other subtree. Use a reactive
[`.enabled(…)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.enabled) instead where the command should stay visible but unavailable; that patches the
one item rather than rebuilding the bar.

## Placement

Placement names an item's ROLE on the chrome carrying it, never a surface: which bar it rides
already follows from where it was declared.

| Placement | Desktop toolbar | iOS | Android | Web |
|---|---|---|---|---|
| `Automatic` (default) | after the leading group | trailing item group | menu action, `IF_ROOM` | strip, declaration order |
| `Navigation` | leading edge of its column | `leftBarButtonItems`, after the back button | leading | leading |
| `Principal` | centered in its column | `titleView` | the bar's centered slot | centered |
| `Primary` | trailing, before secondaries | trailing, folds last | `SHOW_AS_ACTION_ALWAYS`, as its words when it has no icon (a sheet's Done) | trailing |
| `Secondary` | trailing, after primaries | folds into ⋯ first | `SHOW_AS_ACTION_NEVER` | overflow |
| `Bottom` | falls back to `Secondary` | `toolbarItems`, the bottom bar | a Material `BottomAppBar` under a navigation host's pages (a docked bar folds these into its overflow) | falls back |

There is deliberately no `Cancel`/`Confirm`: a modal's affirmative and dismissive buttons are
dialog buttons with their own roles ([docs/dialogs](dialogs.md)), and a second, weaker spelling
of the same idea would leave two right answers for one question.

On UIKit, a [`toolbar_label`](https://daybrite.dev/api/day_pieces/fn.toolbar_label.html) with `Principal` placement uses a native single-line title label.
The `Bottom` bar is laid out the way Mail and Notes lay out theirs: the items before the first
label sit at the leading edge, the label is centered as the bar's status, and the items after
it sit at the trailing edge. Each line of the label's text is a line of the status, the first in
the primary text color and the rest in the secondary, in the footnote text style. A bottom bar
with no label spreads its items across the width.
Android's is a Material `BottomAppBar` laid out the same way: the leading item is the bar's
navigation icon, the label is a centered two-line status ellipsized between the icons, and the
trailing items are its actions, always shown; the status takes new words in place too.
The status keeps UIKit's default side margins, so a long one stops short of the buttons beside
it, and a change to its words, or to a bottom button's enabled or on state, updates the views in
place rather than building the bar again (a status that changes during a sync would otherwise
remake every button on it each time).
Removing that contribution restores the navigation item’s ordinary title. Only the first
principal item is shown.

[`.label_style(…)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.label_style) chooses the title, the icon, or both where a platform can draw more than one; an
item folded into an overflow menu shows its title whatever it asks for, because a menu row with no
words is not a menu row. [`.prominent()`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.prominent) asks for the platform's emphasized style.

## Columns

A desktop toolbar spans every column of a split window at once, and a three-pane app expects each
column's commands to sit over that column — Mail, Notes and Finder all do. Day knows which column
an item came from, because the declaration site already said so, and stamps it on the item.
**An app never writes it.**

`macos-appkit` realizes it with a tracking separator at each divider: AppKit vends
`NSToolbarSidebarTrackingSeparatorItemIdentifier` for the sidebar's, and Day builds the second
with [`NSTrackingSeparatorToolbarItem`](https://developer.apple.com/documentation/appkit/nstrackingseparatortoolbaritem) bound to the split at divider 1. Windows carry
`NSWindowStyleMaskFullSizeContentView` so those items can find their dividers. The sidebar's
commands pack against its trailing edge; every other column packs leading roles first, then the
trailing ones at that column's own right edge. Web does the same with three flex tracks
whose widths follow the panes'. Qt does it inside its one [`QToolBar`](https://doc.qt.io/qt-6/qtoolbar.html): three track widgets, the
sidebar's and the list's given the width of their splitter pane every time the splitter lays
out, the detail's taking the rest — the same packing within each. A window with no navigation
splitter (a settings window, a stack-only app) packs one flat bar by placement.

Everywhere else the column is DROPPED, never the item: GTK draws one header bar with no divider
to track, WinUI's [`CommandBar`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.commandbar) spans the window, and ArkUI's `.menus()` is a flat list.
Degradation always removes the specialization and keeps the command.

## The sidebar affordance

A [`nav(Sidebar)`](https://daybrite.dev/api/day_pieces/fn.nav.html) supplies its own, so an app declares nothing for it. It reaches the toolkits
as an ordinary button under the reserved id `day_spec::SIDEBAR_TOGGLE_ID` whose action names the
host it was built for ([`Toolkit::toggle_sidebar(host)`](https://daybrite.dev/api/day_spec/trait.Toolkit.html#method.toggle_sidebar)), so a second window's button collapses
that window's sidebar and a dayscript `toolbar:` step presses it like any other item. Each
toolkit does what its platform expects: AppKit swaps in `NSToolbarToggleSidebarItemIdentifier`,
Qt keeps the button on the bar when its pane collapses (the track shrinks to it), WinUI drops it because
[`NavigationView`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.navigationview) draws its own pane button, and **UIKit drops it entirely** — [`UISplitViewController`](https://developer.apple.com/documentation/uikit/uisplitviewcontroller)
and `.tabSidebar` each supply one, and Day's copy was both dead on a phone and doubled on an iPad.
Suppress it with [`.no_sidebar_toggle()`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.no_sidebar_toggle).

### The items

| constructor | what it is |
|---|---|
| [`toolbar_button(id, label)`](https://daybrite.dev/api/day_pieces/fn.toolbar_button.html) | a command |
| [`toolbar_toggle(id, label, on)`](https://daybrite.dev/api/day_pieces/fn.toolbar_toggle.html) | a two-state button, bound two-way (`on`: a `Signal<bool>` or any `Binding<bool>`) |
| [`toolbar_segmented(id, segments, selected)`](https://daybrite.dev/api/day_pieces/fn.toolbar_segmented.html) | one native segmented control over a `usize` binding |
| [`toolbar_menu(id, label, entries)`](https://daybrite.dev/api/day_pieces/fn.toolbar_menu.html) | a pull-down, from the same `MenuEntry`s the menu bar takes |
| [`toolbar_label(id, text)`](https://daybrite.dev/api/day_pieces/fn.toolbar_label.html) | static text — a status or a caption |
| [`toolbar_separator(id)`](https://daybrite.dev/api/day_pieces/fn.toolbar_separator.html) | a divider between neighbors in the same placement bucket |

There are no spacers. Alignment is [placement](#placement), which is a fact about what the command
IS rather than about where it happens to sit in a list, and it survives a bar that has to fold.

**Search is declared elsewhere.** It belongs to the navigation surface it filters
([`Nav::searchable`](https://daybrite.dev/api/day_pieces/struct.Nav.html#method.searchable), [docs/search](search.md)), and Day merges the resulting field into this bar under the
reserved id `day::search`. Declaring it on the surface lets the platform move it (into the
navigation list on a window too narrow for a sidebar) without the app re-declaring anything.

Modifiers: [`.icon(Symbol)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.icon), [`.image(name)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.image), [`.action(f)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.action), [`.tooltip(t)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.tooltip), [`.enabled(on)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.enabled) (a
`bool`, a `Signal<bool>` or a closure; a reactive value patches the one item), [`.placement(…)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.placement),
[`.label_style(…)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.label_style), [`.prominent()`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.prominent).

**Use [`toolbar_segmented`](https://daybrite.dev/api/day_pieces/fn.toolbar_segmented.html) wherever exactly one of a set is on at a time**, such as a theme
chooser or a view mode. Three toggles instead say "three independent switches" to the eye and to
a screen reader, leave the app to keep them exclusive, and take three times the width:

```rust
toolbar_segmented(
    id!(theme),
    vec![
        segment(res::str::light()).icon(Symbol::Light),
        segment(res::str::system()).icon(Symbol::Auto),
        segment(res::str::dark()).icon(Symbol::Dark),
    ],
    mode,
) // mode: Signal<usize>, or any Binding<usize>
```

Each toolkit draws the control its platform already has: [`NSSegmentedControl`](https://developer.apple.com/documentation/appkit/nssegmentedcontrol) in `selectOne`
tracking on AppKit, a `.linked` box of grouped toggle buttons on GTK, an exclusive [`QButtonGroup`](https://doc.qt.io/qt-6/qbuttongroup.html)
on Qt, a tight row of compact `AppBarToggleButton`s inside one `AppBarElementContainer` on WinUI
(a checkable menu when it folds into the overflow), and the same `.day-segmented` element the
picker piece uses on the web. The control enforces exclusivity; the signal only ever holds the
chosen index.

Every item takes an `id`. It is the item's identity everywhere: the native item identifier, the
dayscript target, and the key a targeted update addresses. Ids are unique within a bar.

### Reactive updates and identity

Use stable, unique IDs for toolbar items, including separators. Day uses them to retain items
across updates. If visible contributions use the same ID, the first is kept and a warning is logged.

Use the item bindings for toggle state, selection, search text, and enabled state. Use a
reactive toolbar builder when the set of items or their arrangement changes. Keeping values
in their bindings lets an existing search field retain focus while surrounding items change.

### Icons

[`.icon(Symbol::…)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.icon) names what the icon means, and each toolkit draws its platform's glyph:
an SF Symbol on macOS, a freedesktop icon name on GTK and Qt, a Segoe Fluent glyph on Windows.
This is the only way one icon looks native on four desktops; a bundled PNG cannot, because it is
one artist's take on all of them. Use [`.image(name)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.image) only for something app-specific.

[`.image(name)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.image) takes either a `resource/images/` file or a `resource/vectors/` glyph, the same
names the rest of the app uses. The vector is tried first, because on AppKit a vector asset stages
as an SVG and nothing else: looking only for a raster found nothing and the item silently fell back
to drawing its label, a button reading "Star" where a star belonged. Bundled glyphs are templates,
so each toolkit tints them to the bar's own foreground (Qt does this explicitly, since an untinted
template is a flat black shape, invisible on a dark toolbar).

On the web there is no system icon set to borrow, so day-toolkit-dom draws the standard symbols itself,
as inline-SVG `data:` URLs through the same CSS mask a bundled image uses. They are plain
geometry authored in day rather than a third-party icon set, which keeps the framework free of an
icon license. Before that, `Icon::Symbol` was dropped on the web entirely and only items carrying
a bundled image had a glyph, so a bar mixed icons and words.

`Symbol` is `#[non_exhaustive]`. A toolkit that has no glyph for a symbol draws none and the item
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
that had them have been deleted. `Native` on the desktops and the phones, `Emulated` on Web
(a strip docked above the app root, since a browser tab has no title bar to hang chrome on; a
cover-tier window — the settings surface, anything [`open_window`](https://daybrite.dev/api/day_core/windows/fn.open_window.html) presents as a cover
([docs/cover](cover.md)) — gets a strip of its own over the cover, where a navigation stack
inside the cover also puts its back button and title, so the cover's Done and the stack's bar
are one bar as on a phone's modal navigation controller) and on
HarmonyOS (the bar belongs to the navigation destination, not the window). Probe it only for a
layout decision that really turns on a persistent bar existing — Day-Tunes chooses between a
toolbar transport and a Now Playing tab that way.

A window whose content is not a navigation host anywhere — a canvas or a form filling the window —
has no page bar to put items on, so both phones give it one: iOS a navigation bar of the window's
own across the top under the status bar, Android a Material app bar in the same place. The
placements land on that bar exactly as on a page's: a `Primary` item rides a fixed group a narrow
iPhone never folds, so an app's one way to a panel (Day Sketch's Inspector) stays a tap away
rather than two. On Android the bar belongs to the window root it was set for: a cover presented
over the window (the settings surface) carries its own items on its own stack's bar, and the
window's bar is untouched underneath. The same goes for one section of a wide Android window: a tablet's sidebar
shows each section's page beside it, and a section whose page has no navigation host of its own
(a list and its detail composed side by side) gets the window's bar docked across the top while it
shows. A host on a hidden section's page never takes the bar.

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
painted a second app bar directly under the first. In a tabbed window that is the outermost host
on the tab that is showing: every tab's page stays attached, hidden while another tab is selected,
so the bar moves to the selected tab's host whenever the selection changes. Left on the first
tab's host, a second tab's commands were drawn on a bar nobody could see.

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
list there ([docs/search](search.md)), and the sidebar toggle, which the split view owns.
Android draws a `Symbol` from day-toolkit-material's own glyph set — one Material Symbols vector per
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


## Toolkit implementations

Platform guidance: [Apple: toolbars](https://developer.apple.com/design/human-interface-guidelines/toolbars); [Android: app bars](https://developer.android.com/develop/ui/views/components/appbar); [Windows: command bars](https://learn.microsoft.com/en-us/windows/apps/design/controls/command-bar).

| | AppKit | GTK | Qt | WinUI | UIKit | Android | ArkUI | Web |
|---|---|---|---|---|---|---|---|---|
| placement | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ |
| column | ✓ | — | ✓ | — | — | — | — | ✓ |

| | AppKit | GTK | Qt | WinUI |
|---|---|---|---|---|
| bar | [`NSToolbar`](https://developer.apple.com/documentation/appkit/nstoolbar), unified style | the window's [`AdwHeaderBar`](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.HeaderBar.html) | [`QToolBar`](https://doc.qt.io/qt-6/qtoolbar.html) | [`CommandBar`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.commandbar) |
| button | [`NSToolbarItem`](https://developer.apple.com/documentation/appkit/nstoolbaritem) (bordered) | flat [`GtkButton`](https://docs.gtk.org/gtk4/class.Button.html) | [`QAction`](https://doc.qt.io/qt-6/qaction.html) | [`AppBarButton`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.appbarbutton) |
| toggle | [`NSButton`](https://developer.apple.com/documentation/appkit/nsbutton) push-on/push-off | [`GtkToggleButton`](https://docs.gtk.org/gtk4/class.ToggleButton.html) | checkable [`QAction`](https://doc.qt.io/qt-6/qaction.html) | `AppBarToggleButton` |
| menu | [`NSMenuToolbarItem`](https://developer.apple.com/documentation/appkit/nsmenutoolbaritem) | [`GtkMenuButton`](https://docs.gtk.org/gtk4/class.MenuButton.html) + `GMenu` | [`QToolButton`](https://doc.qt.io/qt-6/qtoolbutton.html) (InstantPopup) + [`QMenu`](https://doc.qt.io/qt-6/qmenu.html) | [`AppBarButton`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.appbarbutton) + [`MenuFlyout`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.menuflyout) |
| search | [`NSSearchToolbarItem`](https://developer.apple.com/documentation/appkit/nssearchtoolbaritem) | [`GtkSearchEntry`](https://docs.gtk.org/gtk4/class.SearchEntry.html) | [`QLineEdit`](https://doc.qt.io/qt-6/qlineedit.html) (clear button + find action) | [`AutoSuggestBox`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.autosuggestbox) |
| separator | *(none — a fixed space)* | [`GtkSeparator`](https://docs.gtk.org/gtk4/class.Separator.html) | [`QToolBar::addSeparator`](https://doc.qt.io/qt-6/qtoolbar.html) | `AppBarSeparator` |
| icons | SF Symbols | freedesktop symbolic names | [`QIcon::fromTheme`](https://doc.qt.io/qt-6/qicon.html), then [`QStyle`](https://doc.qt.io/qt-6/qstyle.html) standard pixmaps | Segoe Fluent glyphs |

Notes that are not obvious from the table:

- **ArkUI**: the primary window's toolbar rides `Navigation` / `NavDestination.menus`.
  A custom menu builder uses native `Button`, `Menu`, and `MenuItem` components: template
  images explicitly use system foreground colors (the array API's [`icon`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.icon) loads black SVGs
  literally in dark mode). Page commands take the visible slots before window commands;
  text-only and excess commands go into a labeled overflow popup. Toggles show an accent
  and expose selected state; segmented items open checked choices. System color resources
  update live on appearance changes. The title bar holds up to three 40vp buttons, with
  logical end padding. This remains `Support::Emulated`: placement/column layout, nested
  toolbar pull-downs, and secondary-window toolbars are not fully implemented. Pull-down
  entries still flatten into the action list. A future bottom-placement implementation
  should use native `toolbarConfiguration`, preserving its landscape adaptation.

- **AppKit**: macOS toolbars have no separator item, so [`toolbar_separator(id)`](https://daybrite.dev/api/day_pieces/fn.toolbar_separator.html) renders as the
  system's own fixed space, which is what macOS uses between groups. The toolbar is created once
  per window and edited in place (a replaced [`NSToolbar`](https://developer.apple.com/documentation/appkit/nstoolbar) flashes the title bar and drops
  focus). User customization is off: the item list is app-declared and reactive, so an autosaved
  arrangement would be in permanent conflict with the next edit. Adding or removing the toolbar
  resizes the content view without a window resize, so the toolkit reports the new content size
  itself.
- **GTK**: could express COLUMNS — a per-pane [`AdwHeaderBar`](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.HeaderBar.html) over each pane of the navigation
  split is the GNOME idiom, and Nautilus and Text Editor both do it. Day does not yet; the
  column is dropped and one header bar carries everything. That is a gap, not a toolkit limit.
- **GTK**: GNOME has no separate toolbar. The header bar is the toolbar, and GTK4 removed
  [`GtkToolbar`](https://docs.gtk.org/gtk3/class.Toolbar.html) outright, so items go into the [`AdwHeaderBar`](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.HeaderBar.html) the window already has, around the
  title: into two boxes Day packs at its start and end, so items go in and out one at a time
  without Adwaita's own children in the way. Buttons get the `flat` class, per the GNOME HIG.
- **Qt**: the bar is a [`QToolBar`](https://doc.qt.io/qt-6/qtoolbar.html) parented to the window and laid out with the menu bar, not a
  [`QMainWindow`](https://doc.qt.io/qt-6/qmainwindow.html) dock; the geometry there is already hand-managed. It is a real [`QToolBar`](https://doc.qt.io/qt-6/qtoolbar.html) either
  way: it takes its icon size and its icon/text style from the user's Qt settings, which is the
  KDE convention and why the toolkit sets neither. It does not get dragging between dock
  areas, which needs [`QMainWindow`](https://doc.qt.io/qt-6/qmainwindow.html). Columns are three plain widgets on that bar, each with a
  row layout of three groups (leading, principal, trailing) that items go in and out of; a bar
  without columns is one such track across the whole bar. A button is a [`QAction`](https://doc.qt.io/qt-6/qaction.html) shown through
  an auto-raise [`QToolButton`](https://doc.qt.io/qt-6/qtoolbutton.html) of the bar's own style, so patches by action reach it directly.
  Items are kept per window, since two windows show the same ids. Icons: Qt has no
  glyph set of its own beyond [QStyle](https://doc.qt.io/qt-6/qstyle.html)'s few dialog bitmaps, so a symbol is the desktop theme's
  icon where one exists (a freedesktop theme on Linux; on macOS Qt 6.7+ maps the freedesktop
  names it knows to SF Symbols), then Day's outline, then [QStyle](https://doc.qt.io/qt-6/qstyle.html)'s. Those drawings never
  agreed on a box, so every toolbar glyph is fitted by its ink to the same fraction of the bar's
  icon box and tinted to the palette text color; the box is 24 points on macOS (an [NSToolbar](https://developer.apple.com/documentation/appkit/nstoolbar)
  glyph's), and the user's setting on the Linux desktops.
- **WinUI**: [`CommandBar`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.commandbar) right-aligns `PrimaryCommands`, left-aligns `Content` and folds
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
  lists the choices as checkable rows. [`.label_style(…)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.label_style) is honored (`IconOnly` collapses the
  label, `TitleOnly` drops the icon); [`.prominent()`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.prominent) is not drawn yet. The bar is docked only
  while it draws something: the sidebar toggle alone (which [`NavigationView`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.navigationview) draws itself) leaves
  no empty strip under the title bar. It comes first in tab order, before the content it acts
  on, and Tab wraps around the window rather than stopping at its last control. Secondary
  windows carry their own bar, like the primary.

## Limits

- On WinUI, a language change may leave item labels, tooltips, and search placeholders unchanged
  until the bar's structure changes.
- Toolbar customization and Qt dock-area dragging are not exposed by the portable API.
- GTK does not place contributions in separate navigation-column header bars.
- Contributions within a placement group follow registration order. A conditional contribution
  that appears later is appended within that group.
- Web toolbar tracks are measured when the bar is recomposed; pane-width changes can leave
  a track's width stale until the next composition.

## UIKit toolbar layout

On an expanded UIKit split, `ToolbarColumn` places sidebar, list, and reader commands
on their respective navigation bars. Window commands accompany the reader. Collapsing
merges visible contributions into the stack's bar. The supplementary list omits the
repeated root title while expanded, retaining its controller title for collapsed navigation.

iOS 16+ uses native optional/pinned item groups for overflow. On iOS 15, and on shared
tab bars using plain items, Day limits the navigation bar to three controls including
an ellipsis menu. Navigation actions take priority, then Primary actions; Secondary
and excess actions remain in the native menu, including their checked/disabled state
and submenus. Principal and Bottom placements keep their separate slots.
