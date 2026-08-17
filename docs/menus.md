---
title: "Menus"
description: "The app menu model: the macOS menu bar, context menus, shortcuts, and how other platforms present the same tree."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Menus (§ menus)

Day renders menus with each toolkit's native menu machinery: `NSMenu`, `GtkPopoverMenu` /
`GtkPopoverMenuBar`, `QMenu` / `QMenuBar`, `UIMenu` (via `UIContextMenuInteraction`), Android
`PopupMenu` / the app-bar overflow, and XAML `MenuFlyout` / `MenuBar`. There are two surfaces:

- **Context menus**: per-Piece, shown on secondary-click (desktop) or long-press (touch), attached
  with the [`Decorate::context_menu`] modifier.
- **The app menu**: the global menu bar on desktop, installed once with [`app_menu`].

Both are described with the same small, toolkit-neutral tree of [`MenuEntry`] values. Day owns the
model; the backend owns the rendering, so a menu looks and behaves like any other native menu on
the host platform without the app making any per-platform assumptions.

## Building a menu

```rust
use day_pieces::*;

label("Right-click me")
    .context_menu(vec![
        menu_item("Rename").action(|| rename()),
        menu_item("Duplicate").key("d").action(|| duplicate()),   // ⌘D / Ctrl+D
        menu_separator(),
        sub_menu("Move to", vec![                                  // nested submenu
            menu_item("Inbox").action(|| move_to(Inbox)),
            menu_item("Archive").action(|| move_to(Archive)),
        ]),
        menu_separator(),
        menu_role(MenuRole::Copy),                                 // standard Edit ▸ Copy
        menu_item("Delete").shortcut(Shortcut::plain("Delete")).action(|| delete()),
    ])
```

The pieces, all in the `day_pieces` prelude:

| Builder | Produces |
|---|---|
| `menu_item(label)` | A clickable command. Chain `.action(f)`, `.key("s")`, `.shortcut(_)`, `.enabled(bool)`. |
| `sub_menu(label, vec![…])` | A nested submenu (arbitrarily deep on desktop; see platform notes). |
| `menu_separator()` | A divider between groups. |
| `menu_role(role)` | A standard system command; see [Standard roles](#standard-roles). |

## Keyboard shortcuts

A [`Shortcut`] is a key plus modifiers. `primary` is the platform's command modifier (⌘ on Apple,
Ctrl elsewhere), so one spec is correct everywhere:

```rust
menu_item("Save").key("s")                       // ⌘S / Ctrl+S   (primary + key, the common case)
menu_item("Save As…").shortcut(Shortcut::new("s").shift())   // ⇧⌘S / Ctrl+Shift+S
menu_item("Delete").shortcut(Shortcut::plain("Delete"))      // no primary modifier
```

`Shortcut::new(key)` sets `primary`; `Shortcut::plain(key)` sets no modifiers; `.shift()`, `.alt()`,
`.control()` add the others (`.control()` is the physical Control key, distinct from `primary` on
macOS). Named keys (`"Return"`, `"Delete"`, `"Space"`, `"F5"`, arrows) are recognized alongside
single characters. The shortcut is drawn in the native accelerator position and is live whenever the
menu (or its window) is in the responder/focus chain.

Shortcuts render on every platform that has a hardware-keyboard convention: all three desktops, plus
iPad/Catalyst. On iPhone and Android touch, items appear without accelerators (there is no keyboard),
which is the correct platform behavior.

## Standard roles

`menu_role(MenuRole::…)` emits the platform's built-in command rather than a custom action, so the
familiar items keep their native label, default shortcut, automatic enable/disable, and their
focus targeting: Edit ▸ Copy copies from whatever control has focus, with no wiring:

```rust
app_menu(vec![
    sub_menu("Edit", vec![
        menu_role(MenuRole::Undo), menu_role(MenuRole::Redo),
        menu_separator(),
        menu_role(MenuRole::Cut), menu_role(MenuRole::Copy),
        menu_role(MenuRole::Paste), menu_role(MenuRole::SelectAll),
    ]),
])
```

| Role | AppKit | GTK | Qt | UIKit | Android | XAML |
|---|---|---|---|---|---|---|
| Cut/Copy/Paste/SelectAll | `cut:`/`copy:`… selectors (first responder) | `clipboard.*` actions | dispatched to the focused `QLineEdit`/`QTextEdit` | responder chain (`cut:`…) | system text toolbar¹ | accelerator² |
| Undo/Redo | `undo:`/`redo:` | stock actions | focused editor | — | — | — |
| Quit / Close / Minimize / Fullscreen | standard App-menu items | window actions | window / `qApp` | — | — | Quit closes the window |
| About / Preferences | moved into the App menu | — | `menuRole` → app menu (mac) | — | — | — |

You can override a role's label (`menu_role(r)` starts empty and the backend fills the standard label;
supply your own via `MenuEntry::role` on a `menu_item` if you want a custom title). Roles with no native
equivalent on a platform render as an inert labeled item; no behavior is imposed.

¹ Android editable views raise the system selection toolbar for Cut/Copy/Paste; a role in a Day menu is
shown for parity and dispatches nothing.
² XAML carries the standard accelerator; the focused `TextBox` handles the keystroke itself.

## The app menu

```rust
app_menu(vec![
    sub_menu("File", vec![
        menu_item("New").key("n").action(|| …),
        menu_item("Open…").key("o").action(|| …),
        menu_separator(),
        menu_item("Save").key("s").action(|| …),
        menu_role(MenuRole::CloseWindow),
    ]),
    sub_menu("Edit", vec![ /* roles, as above */ ]),
])
```

Top-level entries are the menu-bar menus. Call `app_menu` at startup or any time the menu changes; it
replaces the previous app menu.

### Claiming a standard slot

Each desktop fills the standard menu-bar slots it knows (Edit, View, Help) with its own stock menu
for any slot the app did not claim, so an app never restates the platform's furniture. **Tag your own
version with `.bar_role(...)` to take a slot:**

```rust
sub_menu("View", vec![ /* … */ ]).bar_role(MenuBarRole::View)
```

A tagged menu replaces the stock one *in place*, so it also lands where the platform expects that
menu to sit: `File`, `Edit`, `View` in the bar's leading order rather than adrift after them.

The tag, not the title, is what identifies the slot, and localization is why: day's catalog and your
app's may translate the same menu differently (day's `day-view` is *Présentation*; the showcase's is
*Affichage*), so a bar matched on titles would show both under `--locale fr`. An untagged submenu
whose title *does* equal the slot's standard name still takes the slot (that stops the most common
accidental duplicate), but it is a safety net, not the contract. Tag the menu.

Where each backend puts the bar:

- **AppKit**: the system menu bar. Day prepends the standard **App menu** (About/Quit) automatically,
  so your `sub_menu`s start at *File*.
- **GTK**: a `GtkPopoverMenuBar` at the top of the window; accelerators registered on the
  `GtkApplication`. On macOS the model goes to `gtk_application_set_menubar` instead; GTK's quartz
  backend renders it in the system menu bar, and the stock GTK app menu's *Settings…* item enables
  through an `app.preferences` action wired to the Preferences dispatch id.
- **Qt**: a `QMenuBar` (the native global bar on macOS-qt).
- **Android**: the app-bar overflow (⋮), built by `DayActivity.onCreateOptionsMenu`.
- **XAML**: a `MenuBar` docked at the top of the window.
- **iOS/iPhone**: a no-op by design. Touch platforms have no persistent global menu bar; the native
  affordances are the per-Piece context menu and the system edit menu. (iPad/Catalyst `UIMenuBuilder`
  wiring is a future addition.)

## How it works

The builder lowers to a flat, toolkit-neutral [`day_spec::MenuItem`] tree. Each item's closure is
registered with day-core, which hands back a process-unique **action id**; only the id travels into the
native menu. When the user chooses an item the backend emits `Event::MenuAction(id)`; the event pump
routes it to `dispatch_menu_action`, which runs the closure inside a reactive batch (so signal writes
made from a menu coalesce into one update, just like a button tap). Standard roles carry no id; they
resolve to the toolkit's own command instead. This keeps the crossing minimal (an integer), avoids
holding native handles across the FFI boundary, and lets any backend add menu support by implementing
just two `Toolkit` methods: `set_app_menu` and `set_context_menu`.

## Re-installing the same menu

`set_app_menu` compares the incoming model with the installed one, ignoring the action ids — an app
declares its menu inside the page build, so every route change re-installs the same commands behind
freshly registered closures. A menu that differs only in those ids rebinds them onto the ids the
platform already holds and makes no toolkit call, which keeps a menu the user has open from closing
under them and stops the menu bar being rebuilt on every navigation. Anything else — a label, a
shortcut, an enablement, a new command — installs as before.

This is the rule the toolbar follows for the same reason ([docs/toolbars.md](toolbars.md), "Re-installing the same
bar"), where the rebuild also took the keyboard focus out of the search field.

## Nav-row context menus

A selector's rows can each carry their OWN context menu — `item(…).context_menu(vec![…])`
inside the `.items` mapper ([docs/navigation.md](navigation.md)) — for the sidebar idioms every desktop app
grows: per-feed "Mark all read", per-project "Reveal in Finder", the Showcase's per-page
"Show Source". The entries are the same builders as everywhere else and lower through the
same action registry, so a chosen entry dispatches identically to a piece context menu; the
menus re-lower (re-localizing their labels) whenever the rows re-derive, exactly like the
row titles.

Per backend: AppKit serves them through the outline's `menuForEvent:` (NSTableView-family
views consume right-clicks themselves, so a menu attached to a cell's subviews would never
be consulted); UIKit through the table delegate's row-context hook (the standard long-press
row menu); GTK a per-row `PopoverMenu` with secondary-click + long-press gestures; Qt one
`QMenu` per row popped from the list's custom-context request; Android a best-effort
`setNavRowMenus` follow-up after the nav mounts (the same off-critical-path rule as the row
tints — [docs/vectors.md](vectors.md)). Web and ArkUI drop them for now, same as the piece decorator's
matrix.

## Platform notes

- **Nested submenus** are unlimited on the desktop backends and iOS. Android menus support a single
  level of submenu (a platform limit); deeper submenus flatten into the nearest one.
- **Separators** render as dividers everywhere; on Android they become menu-group boundaries (dividers
  on API 28+).
- A `context_menu(vec![])` (empty) or a later reconfigure detaches/replaces the menu on the Piece.

## Future surfaces: dock, taskbar, and launcher menus

The same [`MenuEntry`] tree is the right shape for the app-wide surfaces day does not drive
yet: a macOS Dock menu is the existing builder plus one delegate hook
(`applicationDockMenu(_:)`), while Windows jump lists and `.desktop` Actions persist while
the app is closed, so their dispatch would have to lower to a relaunch argument — the real
design gap, gated on those platforms' deep-link intake.

Launcher shortcuts already shipped by another road: Day.toml `[[shortcuts]]` drives iOS,
Android, and HarmonyOS as route-keyed saved deep links, and
[docs/deep-links.md](deep-links.md) owns that story end to end.

## Runtime language changes: `app_menu_reactive`

`app_menu(vec)` resolves labels once, in the install-time locale. An app whose language can
change at runtime (a preferences language picker, [docs/windows.md](windows.md)) installs with
`app_menu_reactive(builder)` instead: the builder re-runs whenever a locale-tracked read
inside it changes (`menu_role` labels, `res::str` titles, and `day::tr` all read the locale
signal), re-lowering and re-installing the whole bar in the new language. Replacement drops
the previous install's action closures; context menus share the dispatch map and are
untouched, and the durable Preferences/New Window dispatch ids always survive.

## The auto Preferences item + the Window menu

`day::register_preferences*` gives every platform its standard Settings…/Preferences item
(⌘, / Ctrl+comma) with zero menu code, and macOS also auto-installs the standard Window
menu. The mechanics — injection into an installed menu, role rewiring, and the
`register_new_window` builder — are [docs/windows.md](windows.md)'s story.

## Driving menus from dayscript

`menu: { item: "Save" }` invokes a unique app-menu action by exact label;
`menu: { key: menu_save }` resolves a Fluent key in the run's locale first (locale-portable:
app keys and the `day-*` role keys both work, so the auto Preferences item is
`key: day-preferences`, with or without an installed app menu). `path: [File]`
disambiguates by ancestor submenu: each entry matches a submenu's literal label OR its
Fluent key resolved in the run's locale, so `path: [menu_file]` works wherever
`key: menu_file` does and one script stays valid in every language. The step dispatches the
registered day action directly (toolkit-uniform, no native menu automation), so role-only
items that run a native selector (Cut, Quit, …) are not invokable this way.
