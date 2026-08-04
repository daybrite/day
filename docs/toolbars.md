# Window toolbars

> **Status: implemented** on the four desktop backends: AppKit, GTK, Qt, XAML. A toolbar is
> window chrome, not a piece: it does not live in the tree and day does not lay it out. Each
> backend realizes the model with its platform's own toolbar; where the platform has none, day
> installs nothing and draws no imitation.

## Authoring

```rust
use day::prelude::*;

toolbar(vec![
    toolbar_button("refresh", tr("refresh")).icon(Symbol::Refresh).action(refresh_all),
    toolbar_separator(),
    toolbar_toggle("star", tr("star"), starred).icon(Symbol::Star),
    toolbar_flexible_space(),
    toolbar_search("search", query).placeholder(tr("search-articles")),
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
| `toolbar_menu(id, label, entries)` | a pull-down, from the same `MenuEntry`s the menu bar takes |
| `toolbar_search(id, signal)` | the platform's search control, bound two-way |
| `toolbar_label(id, text)` | static text |
| `toolbar_separator()` | a divider where the platform has one |
| `toolbar_space()` | a fixed gap |
| `toolbar_flexible_space()` | a gap that absorbs the leftover width |

Modifiers: `.icon(Symbol)`, `.image(name)`, `.action(f)`, `.tooltip(t)`, `.placeholder(t)`,
`.enabled(bool)`, `.enabled_when(f)`.

Every item takes an `id`. It is the item's identity everywhere: the native item identifier, the
dayscript target, and the key a targeted update addresses. Ids are unique within a bar.

### The flexible space is the layout

There is no leading/trailing property. Items before the first `toolbar_flexible_space()` are
leading and the rest are trailing, and each backend expresses that with its own packing: GTK
packs start/end, XAML splits `Content` from `PrimaryCommands`, AppKit and Qt insert a real
expanding spacer. One ordered list, four native layouts.

### What rebuilds and what patches

A full install replaces the bar. That is the wrong path for a value that changes as the user
types: rebuilding would drop the search field's focus mid-word. So the values that change often
ride their own bindings and patch a single item instead:

- a `toolbar_toggle`'s signal
- a `toolbar_search`'s signal
- `.enabled_when(…)`

Keep those OUT of a `toolbar_reactive` builder's reactive reads. Put structure there: which
items exist, and their labels.

### Icons

`.icon(Symbol::…)` names what the icon MEANS, and each backend draws its platform's own glyph:
an SF Symbol on macOS, a freedesktop icon name on GTK and Qt, a Segoe Fluent glyph on Windows.
This is the only way one icon looks native on four desktops; a bundled PNG cannot, because it is
one artist's take on all of them. Use `.image(name)` only for something app-specific.

`Symbol` is `#[non_exhaustive]`. A backend that has no glyph for a symbol draws none and the item
falls back to its label, never to a broken-image placeholder. GTK additionally checks the running
icon theme before setting a name, because icon themes vary in completeness and a missing name
paints GTK's broken-image glyph.

### Where there is no toolbar

Probe it:

```rust
if capability(Cap::Toolbar) == Support::Native { /* toolbar(…) */ } else { /* in the content */ }
```

`Cap::Toolbar` is `Native` on the four desktop backends and `Unsupported` everywhere else. A
phone has no toolbar, so `toolbar(…)` installs nothing there and the app puts those commands in
the content instead; see Day Sheets, whose search is a toolbar item on the desktops and a
timeline field on a phone.

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

- **AppKit** — macOS toolbars have no separator item, so `toolbar_separator()` renders as the
  system's own fixed space, which is what macOS uses between groups. The toolbar is created once
  per window and reused across installs (a replaced `NSToolbar` flashes the title bar and drops
  focus). User customization is off: the item list is app-declared and reactive, so an autosaved
  arrangement would be in permanent conflict with the next install. Installing or removing a
  toolbar resizes the content view without a window resize, so the backend reports the new
  content size itself.
- **GTK** — GNOME has no separate toolbar. The header bar IS the toolbar, and GTK4 removed
  `GtkToolbar` outright, so items pack into the `AdwHeaderBar` the window already has, around the
  title. Buttons get the `flat` class, per the GNOME HIG. `pack_end` grows right-to-left, so the
  trailing group is packed in reverse to reach the screen in the order the app wrote it.
- **Qt** — the bar is a `QToolBar` parented to the window and laid out with the menu bar, not a
  `QMainWindow` dock; the geometry there is already hand-managed. It is a real `QToolBar` either
  way: it takes its icon size and its icon/text style from the user's Qt settings, which is the
  KDE convention and why the backend sets neither. What it does not get is dragging between dock
  areas, which needs `QMainWindow`.
- **XAML** — `CommandBar` right-aligns `PrimaryCommands` and left-aligns `Content`, which is
  exactly the flexible-space split. With one divergence: system XAML's `PrimaryCommands` accepts
  only `ICommandBarElement`, so a search field, a label or a fixed space cannot go there. Those
  three always render in `Content` (on the LEADING side) wherever the app placed them. A search
  item written after the flexible space therefore sits left on Windows and right on the other
  three. This is the toolkit's limit, not a shim shortcut; the alternative would be drawing a
  fake search box, which this design does not do. Windows-only, so XAML is built and exercised in
  CI rather than on a developer's Mac or Linux box. Secondary windows get no toolbar there, the
  same as the menu bar: this shim's chrome lives on the primary window only.

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

Note what this step does NOT prove. It resolves the item in the CURRENT model and dispatches its
action, so it passes even if the native control is still bound to a previous model's action — the
failure mode a real keystroke hits. A backend that rebuilds its bar must rebind the live controls,
not just diff the identifier list (day-appkit had exactly this bug: after a locale change the
search field dispatched an action id day-core had already swept, so typing did nothing).
```

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

Day Sheets carries the applied version: refresh and mark-all-read, next-unread and a star toggle,
then search at the trailing edge, with the same search signal moving into the timeline's own
field on a phone.

Verified by running the showcase and Day Sheets walkthroughs on macos-appkit, macos-gtk and
macos-qt, and by capturing the real windows (an offscreen snapshot cannot show the title bar on
AppKit, and omits the header bar on GTK). XAML is CI-only.

## Follow-ups

- macOS toolbar customization, which needs the model and an autosaved arrangement to be
  reconciled rather than in conflict.
- Qt dock-area dragging, which needs `DayWindow` to become a `QMainWindow`.
- A mobile counterpart: iOS has `UINavigationBar` items and Android an app-bar action row. They
  are a different shape, not a toolbar, so they deserve their own model rather than a
  reinterpretation of this one.
