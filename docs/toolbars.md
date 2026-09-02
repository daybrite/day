---
title: "Window toolbars"
description: "Native window toolbars: items, search, overflow, and per-platform presentation."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Window toolbars

> **Status: implemented** on the four desktop backends: AppKit, GTK, Qt, XAML. A toolbar is
> window chrome, not a piece: it does not live in the tree and day does not lay it out. Each
> backend realizes the model with its platform's own toolbar; where the platform has none, day
> installs nothing.

## Authoring

```rust
use day::prelude::*;

toolbar(vec![
    toolbar_button("refresh", tr("refresh")).icon(Symbol::Refresh).action(refresh_all),
    toolbar_separator(),
    toolbar_toggle("star", tr("star"), starred).icon(Symbol::Star),
    toolbar_flexible_space(),
]);
```

Call it from the window's content builder. day names the window being built, so the same
`build_shell` used for the primary window and for File ▸ New Window gives each window its own
bar without the app tracking any of it.

[`toolbar_reactive`] is the [`app_menu_reactive`](menus.md) counterpart: it re-lowers whenever a
reactive read inside the builder changes, which is how the labels follow a runtime language
switch and how items are added and removed.

### The items

| constructor | what it is |
|---|---|
| `toolbar_button(id, label)` | a command |
| `toolbar_toggle(id, label, signal)` | a two-state button, bound two-way |
| `toolbar_segmented(id, segments, signal)` | one native segmented control over a `Signal<usize>` |
| `toolbar_menu(id, label, entries)` | a pull-down, from the same `MenuEntry`s the menu bar takes |
| `toolbar_sidebar_toggle(id, label)` | show/hide the window's `selector(Sidebar)` pane |

**Search is declared elsewhere.** It belongs to the navigation surface it filters
(`Selector::searchable`, [docs/search.md](search.md)), and Day merges the resulting field into this bar under the
reserved id `day.search`. Declaring it on the surface lets the platform move it (into the
navigation list on a window too narrow for a sidebar) without the app re-declaring anything.

| `toolbar_label(id, text)` | static text |
| `toolbar_separator()` | a divider where the platform has one |
| `toolbar_space()` | a fixed gap |
| `toolbar_flexible_space()` | a gap that absorbs the leftover width |

Modifiers: `.icon(Symbol)`, `.image(name)`, `.action(f)`, `.tooltip(t)`, `.placeholder(t)`,
`.enabled(bool)`, `.enabled_when(f)`.

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

`toolbar_sidebar_toggle` takes no `.action` and needs no icon. It is the one item whose behavior
belongs to the toolkit rather than the app: each backend binds it to the `selector(Sidebar)` host
in that window and drives that host's own collapse (`NSToolbarToggleSidebarItem` on AppKit,
`AdwOverlaySplitView`'s `show-sidebar` on GTK, the splitter pane on Qt,
`NavigationView.PaneDisplayMode` on Windows, a class on the split element on the web). Declare it
first, before any `toolbar_flexible_space()`, which is where every desktop expects it. In a window
with no sidebar it renders disabled rather than disappearing, so the bar keeps its shape as the
route changes.

Every item takes an `id`. It is the item's identity everywhere: the native item identifier, the
dayscript target, and the key a targeted update addresses. Ids are unique within a bar.

### The flexible space is the layout

There is no leading/trailing property. Items before the first `toolbar_flexible_space()` are
leading and the rest are trailing, and each backend expresses that with its own packing: GTK
packs start/end, XAML splits `Content` from `PrimaryCommands`, AppKit and Qt insert a real
expanding spacer. One ordered list produces four native layouts.

### What rebuilds and what patches

A full install replaces the bar. That is the wrong path for a value that changes as the user
types: rebuilding would drop the search field's focus mid-word. So the values that change often
ride their own bindings and patch a single item instead:

- a `toolbar_toggle`'s signal
- a `.searchable()` surface's query signal ([docs/search.md](search.md))
- `.enabled_when(…)`

Keep those out of a `toolbar_reactive` builder's reactive reads. Put structure there: which
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

### Where there is no toolbar

Probe it:

```rust
if capability(Cap::Toolbar) != Support::Unsupported { /* toolbar(…) */ } else { /* in the content */ }
```

`Cap::Toolbar` is `Native` on the four desktop backends, **`Emulated` on web-dom** (2026-08: a
strip docked above the app root, since a browser tab has no title bar to hang chrome on; a
`toolbar_menu` pops a themed popup under its button, items activated through the same
`day_dom_toolbar_action` route a plain button takes), and
**`Native` on iOS and Android too** (2026-09). On iOS the items ride the NAVIGATION BAR of the
page that is showing — the detail column's on an expanded iPad split, the merged stack's when
it has collapsed or on a phone — as iOS 16 item groups, one per item, ahead of the page's own
`bar_action`s; what the bar cannot fit folds into its overflow menu, trailing items first, so
declare the least-used items last. No bottom bar, and the sidebar column never carries the
window's bar: one bar per window, on the content it acts on. Android does the same in the nav
host's APP BAR (2026-09): the items go in as menu items shown as actions, leading the page's own
`bar_action`s, and what the bar cannot fit folds into its overflow (⋮) in declaration order.
Day used to dock a second `MaterialToolbar` under the pages, which read as a phone's bottom bar
and, tiled on a tablet, as a strip of icons stranded below both panes with the titled bar above
them empty.
Buttons, toggles and labels draw as themselves, a menu item drops its menu, and a segmented item
becomes a pull-down of its segments with the chosen one checked (a segmented control has no room
in a phone's bar). That pull-down is titled by the segment IN FORCE, since a segmented control
carries no label of its own: on iOS by that segment's icon where it has one, otherwise its word;
on Android by its word. Two kinds never reach a phone's bar: search, which rides the navigation
list there ([docs/search.md](search.md)), and the sidebar toggle, which the split view owns.
Android stages no glyph for a `Symbol`, so an item with only a symbol has no glyph to show in
the bar and lives in the overflow, where its label reads as a menu row — a Material app bar
carries icon buttons and sends the rest to its overflow, and two text actions were enough to
squeeze the Showcase's own title to "Day Showc…". An `Icon::Image` draws as the image on both
phones, and Android re-tints it to the app bar's own color.
`Unsupported` on HarmonyOS. Probe for `!= Support::Unsupported` rather than `== Native` unless
the difference matters to the app; a caller usually wants to know whether the commands belong
in a bar at all. Where the answer is `Unsupported`, `toolbar(…)` installs nothing and the app
puts those commands in the content instead.

For commands that belong on the chrome rather than in the page (Settings, Compose, "Show Source",
"Add"), the navigation bar's trailing actions are the mobile counterpart:
`selector(…).bar_action(icon, label, action)` ([docs/navigation.md](navigation.md)) draws an upper-right bar button
on iOS/Android/HarmonyOS and is ignored on the desktop split, where the same command rides the
toolbar. Call it repeatedly for several buttons, and use `list_action` for the ones that act on
the list rather than on whatever page is open; a toolbar's commands usually map to those, since a
desktop toolbar sits over the list and detail together. One registered closure can back both a
toolbar button here and a bar action there.

## Per-backend native realization

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
- **GTK**: GNOME has no separate toolbar. The header bar is the toolbar, and GTK4 removed
  `GtkToolbar` outright, so items pack into the `AdwHeaderBar` the window already has, around the
  title. Buttons get the `flat` class, per the GNOME HIG. `pack_end` grows right-to-left, so the
  trailing group is packed in reverse to reach the screen in the order the app wrote it.
- **Qt**: the bar is a `QToolBar` parented to the window and laid out with the menu bar, not a
  `QMainWindow` dock; the geometry there is already hand-managed. It is a real `QToolBar` either
  way: it takes its icon size and its icon/text style from the user's Qt settings, which is the
  KDE convention and why the backend sets neither. It does not get dragging between dock
  areas, which needs `QMainWindow`.
- **XAML**: `CommandBar` right-aligns `PrimaryCommands` and left-aligns `Content`, which is
  the flexible-space split, with one divergence: system XAML's `PrimaryCommands` accepts
  only `ICommandBarElement`, so a search field, a label or a fixed space cannot go there. Those
  three always render in `Content` (on the leading side) wherever the app placed them. A search
  item written after the flexible space therefore sits left on Windows and right on the other
  three. That limit is the toolkit's; the alternative would be drawing a search box by hand,
  which this design does not do. XAML is Windows-only, so it is built and exercised in CI rather
  than on a developer's Mac or Linux box. Secondary windows get no toolbar there, the
  same as the menu bar: this shim's chrome lives on the primary window only.

## Re-installing the same bar

An app declares its toolbar inside the page build, so a route change re-installs it, with freshly
registered closures every time, since the ids come from `register_toolbar_value` /
`register_menu_action`. Handing that to a backend rebuilds the native bar, which is invisible for a
button and destructive for the search field: recreating the widget takes the keyboard focus and the
caret with it. Typing a letter that moved the nav selection re-ran the page build and threw away the
field being typed into, on every backend that rebuilds what it is handed, which is all of them.

`set_window_toolbar` therefore compares the incoming model with the installed one, ignoring what
cannot matter to the widgets: the action ids, and the search field's live text and completions
(kept current through `ToolbarPatch::Text`/`Suggestions`, never through a rebuild). Same items in
the same order, with the same kinds, labels, icons and enablement, means the native bar is already
correct, so the new closures are moved onto the ids it already carries and no toolkit call is made
at all. Anything else is a real change and installs as before.

This is why a backend never has to preserve focus across an install: an install that would have
disturbed the focus does not happen.

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

Day News carries the applied version: refresh and mark-all-read, next-unread and a star toggle,
then search at the trailing edge, with the same search signal moving into the timeline's own
field on a phone.

Verified by running the showcase and Day News walkthroughs on macos-appkit, macos-gtk and
macos-qt, and by capturing the real windows (an offscreen snapshot cannot show the title bar on
AppKit, and omits the header bar on GTK). XAML is CI-only.

## Follow-ups

- macOS toolbar customization, which needs the model and an autosaved arrangement to be
  reconciled rather than in conflict.
- Qt dock-area dragging, which needs `DayWindow` to become a `QMainWindow`.
- The phone bars (2026-09) take the desktop model as is; a segmented item and a search field are
  the two kinds that change shape there. Edge-to-edge Android does not yet pad the bar past the
  system navigation bar.
