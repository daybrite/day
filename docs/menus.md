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
macOS). Named keys (`"Return"`, `"Delete"`, `"Space"`, `"F5"`, arrows) are recognised alongside
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
equivalent on a platform render as an inert labelled item; no behavior is imposed.

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
replaces the previous app menu. Where each backend puts it:

- **AppKit**: the system menu bar. Day prepends the standard **App menu** (About/Quit) automatically,
  so your `sub_menu`s start at *File*.
- **GTK**: a `GtkPopoverMenuBar` at the top of the window; accelerators registered on the
  `GtkApplication`. On macOS the model goes to `gtk_application_set_menubar` instead — GTK's quartz
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

## Platform notes

- **Nested submenus** are unlimited on the desktop backends and iOS. Android menus support a single
  level of submenu (a platform limit); deeper submenus flatten into the nearest one.
- **Separators** render as dividers everywhere; on Android they become menu-group boundaries (dividers
  on API 28+).
- A `context_menu(vec![])` (empty) or a later reconfigure detaches/replaces the menu on the Piece.

## Runtime language changes — `app_menu_reactive`

`app_menu(vec)` resolves labels once, in the install-time locale. An app whose language can
change at runtime (a preferences language picker, docs/windows.md) installs with
`app_menu_reactive(builder)` instead: the builder re-runs whenever a locale-tracked read
inside it changes (`menu_role` labels, `res::str` titles, `day::tr` — all read the locale
signal), re-lowering and re-installing the whole bar in the new language. Replacement drops
the previous install's action closures; context menus share the dispatch map and are
untouched, and the durable Preferences/New Window dispatch ids always survive.

## The auto Preferences item + the Window menu (docs/windows.md)

`day::register_preferences*` enables a Settings…/Preferences item with the platform
shortcut (⌘, / Ctrl+comma) with zero menu code: day-core injects it into an installed
menu's first submenu when absent (an app-placed `menu_role(MenuRole::Preferences)` is
rewired instead — an explicit `.action` always wins), and the backend default menus carry
it too. macOS hoists the item into the App menu under About, its standard home, and also
auto-installs the standard Window menu (Minimize ⌘M / Zoom / Bring All to Front,
registered as `windowsMenu` so AppKit appends the open-window list and tab commands) unless
the app's model owns `MenuRole::Minimize`. `MenuRole::NewWindow` lowers to the
`register_new_window` builder (⌘N/Ctrl+N; disabled when unregistered).

## Driving menus from dayscript

`menu: { item: "Save" }` invokes a unique app-menu action by exact label;
`menu: { key: menu_save }` resolves a Fluent key in the run's locale first (locale-portable
— app keys and the `day-*` role keys both work, so the auto Preferences item is
`key: day-preferences`, with or without an installed app menu). `path: [File]`
disambiguates by ancestor submenu labels. The step dispatches the registered day action
directly — toolkit-uniform, no native menu automation — so role-only items that run a
native selector (Cut, Quit, …) are not invokable this way.
