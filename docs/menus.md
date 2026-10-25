---
title: "Menus"
description: "The app menu model: the macOS menu bar, context menus, shortcuts, and how other platforms present the same tree."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Menus

Make app actions available in the menu bar or in a context menu beside the content they affect.
Day uses each platform’s menu controls and action conventions.

Day renders menus with each toolkit's native menu machinery: [`NSMenu`](https://developer.apple.com/documentation/appkit/nsmenu), [`GtkPopoverMenu`](https://docs.gtk.org/gtk4/class.PopoverMenu.html) /
[`GtkPopoverMenuBar`](https://docs.gtk.org/gtk4/class.PopoverMenuBar.html), [`QMenu`](https://doc.qt.io/qt-6/qmenu.html) / [`QMenuBar`](https://doc.qt.io/qt-6/qmenubar.html), [`UIMenu`](https://developer.apple.com/documentation/uikit/uimenu) (via [`UIContextMenuInteraction`](https://developer.apple.com/documentation/uikit/uicontextmenuinteraction)), Android
`PopupMenu` / the app-bar overflow, and WinUI [`MenuFlyout`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.menuflyout) / [`MenuBar`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.menubar). There are two surfaces:

- **Context menus**: per-Piece, shown on secondary-click (desktop) or long-press (touch), attached
  with the [`Decorate::context_menu`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.context_menu) modifier.
- **The app menu**: the global menu bar on desktop (a toolbar menu button on the web), installed
  once with [`app_menu`](https://daybrite.dev/api/day_pieces/fn.app_menu.html).

Both are described with the same small, toolkit-neutral tree of [`MenuEntry`] values. Day owns the
model; the toolkit owns the rendering, so a menu looks and behaves like any other native menu on
the host platform without the app making any per-platform assumptions.

For an operation shared across surfaces, use a [reusable `Command`](commands.md) to define its
title, availability, optional check state, icon, shortcut, and handler once.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

## Building a menu

```rust
use day_pieces::*;

label(res::str::right_click_me()).context_menu(vec![
    menu_item(res::str::rename()).action(|| rename()),
    menu_item(res::str::duplicate())
        .key("d")
        .action(|| duplicate()), // ⌘D / Ctrl+D
    menu_separator(),
    sub_menu(
        res::str::move_to(),
        vec![
            // nested submenu
            menu_item(res::str::inbox()).action(|| move_to(Inbox)),
            menu_item(res::str::archive()).action(|| move_to(Archive)),
        ],
    ),
    menu_separator(),
    menu_role(MenuRole::Copy), // standard Edit ▸ Copy
    menu_item(res::str::delete())
        .shortcut(Shortcut::plain("Delete"))
        .action(|| delete()),
])
```

The pieces, all in the `day_pieces` prelude:

| Builder | Produces |
|---|---|
| [`menu_item(label)`](https://daybrite.dev/api/day_pieces/fn.menu_item.html) | A clickable command. `label` is any text source (a literal, `res::str::x()`, a signal, a closure), live. Chain [`.action(f)`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.action), [`.key("s")`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.key), [`.shortcut(_)`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.shortcut), [`.enabled(on)`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.enabled), [`.id(id!(name))`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.id), [`.checked(on)`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.checked); `on` is a `bool`, a `Signal<bool>` or a closure. |
| [`sub_menu(label, vec![…])`](https://daybrite.dev/api/day_pieces/fn.sub_menu.html) | A nested submenu (arbitrarily deep on desktop; see platform notes). |
| [`menu_separator()`](https://daybrite.dev/api/day_pieces/fn.menu_separator.html) | A divider between groups. |
| [`menu_role(role)`](https://daybrite.dev/api/day_pieces/fn.menu_role.html) | A standard system command; see [Standard roles](#standard-roles). |

## Naming an item: [`.id`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.id)

[`.key("s")`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.key) sets the item's ⌘/Ctrl SHORTCUT. To NAME an item, use [`.id`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.id):

```rust
menu_item(res::str::view_grid())
    .id(id!(view_grid))
    .action(|| grid.set(!grid.get()))
```

An id is what something outside the menu addresses the item by — a dayscript
`menu: { id: "view_grid" }` step (`context_menu: { id: <element>, item_id: "view_grid" }` for an
element's context menu), or a `tap` on the row of the
[composed presentation](#the-composed-presentation-web). A label cannot do that job: it is
localized, so it changes with the run's language, and a [checked](#checked-items) item rewrites its
own label whenever the state moves. Give an id to any item a script drives; leave it off the rest.

Ids are the app's own vocabulary and need only be unique within the menu they are installed in.
They never reach the user.

## Checked items

[`.checked(on)`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.checked) draws the platform's check mark beside the item — for a setting the menu
toggles, or one of several mutually exclusive choices. `on` is a `bool`, a `Signal<bool>` or a
closure; pass the signal and the mark follows the state:

```rust
app_menu([sub_menu(
    res::str::menu_view(),
    vec![
        menu_item(res::str::view_grid())
            .id(id!(view_grid))
            .checked(grid)
            .action(move || grid.set(!grid.get())),
        menu_separator(),
        menu_item(res::str::view_day())
            .id(id!(view_day))
            .key("1")
            .checked(move || mode.get() == 0)
            .action(move || mode.set(0)),
        menu_item(res::str::view_both())
            .id(id!(view_both))
            .key("2")
            .checked(move || mode.get() == 1)
            .action(move || mode.set(1)),
    ],
)]);
```

An item that never calls [`.checked`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.checked) is a plain command and reserves no room for a mark. One that
calls it is *checkable* whichever way it sits, so a run of choices lines up on its titles instead of
stepping in and out by a tick's width as the selection moves.

**The app owns the state.** Choosing a checked item runs its [`.action`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.action) and nothing else — no toolkit
flips the mark by itself, and the ones whose native control would (GTK's stateful action, Qt's
checkable [`QAction`](https://doc.qt.io/qt-6/qaction.html), WinUI's [`ToggleMenuFlyoutItem`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.togglemenuflyoutitem)) have that flip suppressed. The mark moves when
the menu is re-installed carrying the new value, which [`app_menu`](#runtime-language-changes)
does on its own as soon as a [`checked`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.checked) source changes. So what the menu shows can never disagree
with what the app thinks, and an action that declines to change anything leaves the mark where it was.

| Toolkit | Drawn as |
|---|---|
| appkit | [`NSMenuItem.state`](https://developer.apple.com/documentation/appkit/nsmenuitem) — the system ✓ |
| uikit | [`UIAction.state`](https://developer.apple.com/documentation/uikit/uiaction) — the system ✓ |
| gtk | a stateful boolean `GAction`; GTK renders the check from the action's state |
| qt | a checkable [`QAction`](https://doc.qt.io/qt-6/qaction.html) |
| WinUI | [`ToggleMenuFlyoutItem`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.togglemenuflyoutitem) |
| android | `MenuItem.setCheckable` — Material's check |
| dom | a ✓ in a fixed check column of the composed row |

## Icons

An item can carry the platform's glyph beside its title:

```rust
menu_item(res::str::rectangle()).icon(Symbol::Rectangle).action(place_rect)
menu_item(res::str::brand()).image(res::images::brand_mark).action(insert_brand)
```

[`.icon(Symbol)`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.icon) takes the same standard vocabulary toolbars take (each toolkit draws its own
glyph: an SF Symbol, a freedesktop icon name, a Segoe Fluent code point), and [`.image(name)`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.image) a
bundled picture from `resource/images` for something only this app has.

An icon is always an addition to a menu that reads correctly without one, because not every
platform's menus carry pictures:

| | icons in menus |
|---|---|
| **AppKit**, **UIKit** | yes — [`NSMenuItem.image`](https://developer.apple.com/documentation/appkit/nsmenuitem) / [`UIAction`](https://developer.apple.com/documentation/uikit/uiaction) image, from the shared SF Symbol table |
| **GTK** | yes — the `GMenuModel` "icon" attribute, drawn by [`GtkPopoverMenu`](https://docs.gtk.org/gtk4/class.PopoverMenu.html) |
| **Qt** | yes — [`QAction::setIcon`](https://doc.qt.io/qt-6/qaction.html), resolved like a toolbar icon (theme name, then Day's outline, then the [QStyle](https://doc.qt.io/qt-6/qstyle.html) standard set) |
| **WinUI** | symbols only — a `FontIcon` from the Segoe Fluent table; a bundled image would need the toolbar's three-field icon channel |
| **Android** | ignored: Material's overflow menu is text-only, and an icon belongs on an app-bar action |
| **Web** | yes — a template glyph beside the row of the app menu's popover (and a toolbar pull-down's), from the same outline table the toolbar draws |
| **ArkUI** | no menus of this kind to put an icon in |

## Keyboard shortcuts

A [`Shortcut`] is a key plus modifiers. `primary` is the platform's command modifier (⌘ on Apple,
Ctrl elsewhere), so one spec is correct everywhere:

```rust
menu_item(res::str::save()).key("s")                       // ⌘S / Ctrl+S   (primary + key, the common case)
menu_item(res::str::save_as()).shortcut(Shortcut::primary("s").shift())   // ⇧⌘S / Ctrl+Shift+S
menu_item(res::str::delete()).shortcut(Shortcut::plain("Delete"))      // no primary modifier
```

[`Shortcut::primary(key)`](https://daybrite.dev/api/day_spec/struct.Shortcut.html#method.primary) sets `primary`; [`Shortcut::plain(key)`](https://daybrite.dev/api/day_spec/struct.Shortcut.html#method.plain) sets no modifiers; [`.shift()`](https://daybrite.dev/api/day_spec/struct.Shortcut.html#method.shift), [`.alt()`](https://daybrite.dev/api/day_spec/struct.Shortcut.html#method.alt),
[`.control()`](https://daybrite.dev/api/day_spec/struct.Shortcut.html#method.control) add the others ([`.control()`](https://daybrite.dev/api/day_spec/struct.Shortcut.html#method.control) is the physical Control key, distinct from `primary` on
macOS). Named keys (`"Return"`, `"Delete"`, `"Space"`, `"F5"`, arrows) are recognized alongside
single characters. The shortcut is drawn in the native accelerator position and is live whenever the
menu (or its window) is in the responder/focus chain. On GTK, `primary` spells the Command key on
macOS (`<Meta>`, since GTK4's `<Primary>` is a plain `<Control>` alias, unlike GTK3's) and `<Primary>`
elsewhere, so the one spec stays right on macos-gtk too.

The standard roles need no spelling at all: [`menu_role(Undo/Redo/Cut/Copy/Paste/SelectAll)`](https://daybrite.dev/api/day_pieces/fn.menu_role.html) items
take the platform-neutral defaults (primary+Z/X/C/V/A, shift for redo) unless the app sets its
own. AppKit's native items always carried these; the lowering gives GTK, Qt, and the other
action-routed toolkits the same accelerators.

Shortcuts render on the platforms that have an app menu: the three desktops' menu bars and
Web's [emulated app menu](#the-composed-presentation-web), which is
[`Cap::AppMenu`](duty-matrix.md) native or emulated. On the web the shim matches the model's
accelerators on the document's capture phase, so a key the menu owns reaches it before the
focused widget, the way a menu bar's key equivalents do; a bare key (Delete) stays with a
focused text field, and a combination the browser reserves for itself (⌘N, ⌘T, ⌘W) never
reaches the page at all. The UIKit and Android menu builders take the item and drop its
accelerator, so an app that wants a key to work there routes it through [`.on_key`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_key) on the
focused piece instead; probe `Cap::AppMenu` to decide which, or the key fires twice where both
apply. Day-Sketch's `canvas_key` is the worked example.

## Standard roles

[`menu_role(MenuRole::…)`](https://daybrite.dev/api/day_pieces/fn.menu_role.html) emits the platform's built-in command rather than a custom action, so the
familiar items keep their native label, default shortcut, automatic enable/disable, and their
focus targeting: Edit ▸ Copy copies from whatever control has focus, with no wiring:

```rust
app_menu(vec![sub_menu(
    res::str::edit(),
    vec![
        menu_role(MenuRole::Undo),
        menu_role(MenuRole::Redo),
        menu_separator(),
        menu_role(MenuRole::Cut),
        menu_role(MenuRole::Copy),
        menu_role(MenuRole::Paste),
        menu_role(MenuRole::SelectAll),
    ],
)])
```

| Role | AppKit | GTK | Qt | UIKit | Android | WinUI | Web |
|---|---|---|---|---|---|---|---|
| Cut/Copy/Paste | `cut:`/`copy:`/`paste:` nav hosts — a focused text view answers first, then the app's edit bridge⁴ | `clipboard.*` actions | dispatched to the focused [`QLineEdit`](https://doc.qt.io/qt-6/qlineedit.html)/[`QTextEdit`](https://doc.qt.io/qt-6/qtextedit.html) | responder chain, then the edit bridge⁴ | edit bridge⁴ (text keeps its own selection toolbar¹) | accelerator² | popover row → edit bridge⁴; the keys stay the browser's own clipboard events |
| SelectAll | `selectAll:` nav host — a focused text view answers first, then the edit bridge⁴ | `selection.select-all` | focused editor | responder chain, then the edit bridge⁴ | edit bridge⁴ | accelerator² | popover row → edit bridge⁴; ⌘A is the shim's document route |
| Undo/Redo | `undo:`/`redo:` (responder chain — the acting [`NSUndoManager`](https://developer.apple.com/documentation/foundation/undomanager), a focused text field's before the document's) | stock actions (`text.undo`) | focused editor | installed undo bridge³ | installed undo bridge³ | — | installed undo bridge³; ⌘Z/⇧⌘Z are the shim's document route |
| Quit / Close / Minimize / Fullscreen | standard App-menu items | window actions | window / `qApp` | — | — | Quit closes the window | left out of the popover (no meaning in a browser tab; New Window likewise) |

The platform quit shortcut works with no menu declared: GTK registers `app.quit` on
`<Primary>q`, and Qt gives the window a [`QKeySequence::Quit`](https://doc.qt.io/qt-6/qkeysequence.html) action (Ctrl+Q; ⌘Q on macOS,
where Qt's own application menu carries Quit as well). A declared `MenuRole::Quit` item takes
the key over. Neither desktop adds a Quit item on its own: the GNOME guidelines keep Quit out of
menus (the window closes, Ctrl+Q quits), and an app that wants one on Qt declares the role.
| About / Preferences / Check for Updates | moved into the App menu; About shows the stock panel until [`day::register_about`](https://daybrite.dev/api/day/fn.register_about.html) names a Day action ([settings](settings.md)) | Help ▸ About / Check for Updates… dispatch the registered actions | `menuRole` → app menu (mac); Help elsewhere | — | — | Help menu | popover rows dispatching the registered actions; an unregistered About is left out (no stock panel to fall back on) |

You can override a role's label ([`menu_role(r)`](https://daybrite.dev/api/day_pieces/fn.menu_role.html) starts empty and the toolkit fills the standard label;
supply your own via [`MenuEntry::role`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.role) on a [`menu_item`](https://daybrite.dev/api/day_pieces/fn.menu_item.html) if you want a custom title). Roles with no native
equivalent on a platform render as an inert labeled item; no behavior is imposed.

³ On a toolkit with no native undo responder, `MenuRole::Undo`/`Redo` items lower onto a
standing dispatcher ([`day_core::undo_action_id`](https://daybrite.dev/api/day_core/fn.undo_action_id.html)) that invokes the undo history installed via
[`day::install_undo`](https://daybrite.dev/api/day/fn.install_undo.html) ([docs/model](model.md)), the same stack the platform's route reaches on
macOS/iOS, so one [`menu_role`](https://daybrite.dev/api/day_pieces/fn.menu_role.html) pair behaves consistently everywhere the menu renders.

⁴ The edit bridge (`Cap::EditBridge`). An app that can cut/copy/paste its own objects installs
handlers once:

```rust
day::install_edit_commands(
    || !selection().get().is_empty(),   // can_copy — a tracked read
    copy_selection,                     // -> Option<String>: the serialized payload
    cut_selection,                      // -> Option<String>, and removes the objects
    |text| paste_payload(text),         // whatever text the clipboard held
    select_all,                         // Edit ▸ Select All (⌘A/Ctrl+A)
);
```

`copy`/`cut` place their payload on the system clipboard, and Paste asks for it back — but the
answer is not guaranteed. Android grants clipboard READS only to the app holding input focus
([clipboard](clipboard.md)), and its own copy overlay can take that focus for a moment, so a
Paste seconds after a Copy can be handed nothing. Day keeps what the app last placed there and
pastes that when the platform comes back empty, which is what makes an app's own Cut/Copy ▸ Paste
work on every target; a clip from another app is still what the platform answers with, and it
always wins.

Two companions round out the platform's input idioms. [`day::modifiers()`](https://daybrite.dev/api/day/fn.modifiers.html) answers the keyboard
modifiers held right now (`shift`, `primary`, which is ⌘ on Apple platforms and Ctrl elsewhere,
and `alt`), for interactions whose meaning they change: shift-click adding to a selection instead
of replacing it. It is a live query, not an event field, so it is right wherever it is asked. A
toolkit that cannot answer reports all-false, which is correct for touch platforms and something
to check before relying on it elsewhere ([the `modifiers` row of the duty
matrix](duty-matrix.md)). A dayscript `tap:` or `drag:` step's declared `modifiers:` take
precedence while it dispatches.

[`.on_key(f)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_key) handles the non-text keys, under the web `KeyboardEvent.key` names and with the
held modifiers on the event ([`ev.shift()`](https://daybrite.dev/api/day_spec/struct.Shortcut.html#method.shift) scales a nudge from 1px to 10). The four arrows
arrive everywhere, and so do the ten digits, `"0"` through `"9"` from the main row or the
numeric keypad ([`ev.digit()`](https://daybrite.dev/api/day_spec/struct.KeyEvent.html#method.digit) reads the value). A digit arrives only while neither primary nor
alt is held (nor a Mac's Control), since ⌘1 and Ctrl+1 are accelerators. Most toolkits name
the digit a keypress types, so a layout that needs Shift for digits reports `"1"` with
[`ev.shift()`](https://daybrite.dev/api/day_spec/struct.Shortcut.html#method.shift) set; WinUI and ArkUI read the physical key instead. `Delete` and `Backspace` arrive
only where [`Cap::AppMenu`](duty-matrix.md) is unsupported, where there is no menu bar and so
no accelerator that could own them.

That split keeps one key to one owner. **A focused piece
that claims a key stops any accelerator from ever seeing it**: the platform offers the key to
the focus chain, and a piece with an [`.on_key`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_key) handler claims every key the route carries, not
just the ones its handler acts on. So a canvas that received `Delete` on a menu-bar platform
would swallow the very `Delete` its own Edit menu was about to act on. Route a key through the
menu or through [`.on_key`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_key), never both, and probe `Cap::AppMenu` to decide which. Day-Sketch's
`canvas_key` is the worked example, and the delete keys are routed to match it.

The two delete keys keep the names the platform gives the physical key, so a handler meaning
"remove this" takes both: a Mac's ⌫ reports `Backspace`, a full-size keyboard's Del reports
`Delete`. **Keys follow focus.** The handler hangs off a piece and fires only while that piece is the focused
one, so it is scoped the way every other input is:

```rust
canvas(draw)
    .on_key(nudge_selection_by)     // only while the canvas has the keyboard
    .focused(canvas_focused)        // …which it takes at mount and on every press
```

That scoping is why there is no window-level key handler to pair it with. A global route cannot
tell a nudge the app wants from the keys a focused widget needs: it has to run ahead of the
platform's dispatch, which means guessing whether the first
responder would have wanted the key, and every guess is wrong for something. Day-Sketch's
arrow-nudge used to be installed that way, and it took the arrow keys away from every list and
sidebar in every app that had one. Hanging the handler on the canvas removes the question:
AppKit's responder chain and the DOM's focus already answer it.

The cost is that a piece must be able to hold focus for its keys to arrive
([docs/focus](focus.md)). A [`canvas`](https://daybrite.dev/api/day_pieces/fn.canvas.html) can on every toolkit (each toolkit
makes its drawing surface a tab stop and reports focus both ways), which makes it the piece a
drawing app hangs its keys on. A piece that cannot take focus on a given toolkit never
hears a key there, and a canvas nobody gave a handler to keeps none of them: an unclaimed
arrow keeps walking, so an enclosing scroll view still scrolls and the platform's focus
navigation still moves between controls.

The payload format is the app's own; Day-Sketch uses a standalone SVG document, so shapes
paste into anything that reads SVG and SVG from other editors pastes back. day places the
payload on the system clipboard ([day-part-clipboard](/docs/reference/clipboard)) and reads it back for paste; on Web
the browser's `copy`/`cut`/`paste` events themselves are the route, with the event's
`clipboardData` as transport. Precedence is the platform's own: on macOS/iOS the responder
chain lets a focused text field keep its clipboard behavior, and the app's handlers see only
what falls through, with the same items, shortcuts, and menu validation
(`can_copy` greys Cut/Copy; Paste additionally requires clipboard text). On toolkits whose
role items come back as plain menu actions (Android's app bar, web context menus), the
[`menu_role(Cut/Copy/Paste)`](https://daybrite.dev/api/day_pieces/fn.menu_role.html) items dispatch to the same handlers via standing ids.

¹ Android editable views raise the system selection toolbar for Cut/Copy/Paste; a role in a Day menu is
shown for parity and dispatches nothing.
² WinUI carries the standard accelerator; the focused [`TextBox`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.textbox) handles the keystroke itself.

## The app menu

```rust
app_menu(vec![
    sub_menu(res::str::file(), vec![
        menu_item(res::str::new()).key("n").action(|| …),
        menu_item(res::str::open()).key("o").action(|| …),
        menu_separator(),
        menu_item(res::str::save()).key("s").action(|| …),
        menu_role(MenuRole::CloseWindow),
    ]),
    sub_menu(res::str::edit(), vec![ /* roles, as above */ ]),
])
```

Top-level entries are the menu-bar menus. [`app_menu`](https://daybrite.dev/api/day_pieces/fn.app_menu.html) takes a `Vec` or an array of entries, or
a closure that derives them and re-runs whenever its reactive reads change. Call it once, at
startup; a second call replaces the previous app menu.

### Claiming a standard slot

Each desktop fills the standard menu-bar slots it knows (Edit, View, Help) with its own stock menu
for any slot the app did not claim, so an app never restates the platform's furniture. **Tag your own
version with [`.bar_role(...)`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.bar_role) to take a slot:**

```rust
sub_menu(res::str::view(), vec![ /* … */ ]).bar_role(MenuBarRole::View)
```

A tagged menu replaces the stock one *in place*, so it also lands where the platform expects that
menu to sit: `File`, `Edit`, `View` in the bar's leading order rather than adrift after them.

The tag identifies the slot, not the title, because of localization: day's catalog and your
app's may translate the same menu differently (day's `day_view` is *Présentation*; the showcase's is
*Affichage*), so a bar matched on titles would show both under `--locale fr`. An untagged submenu
whose title *does* equal the slot's standard name still takes the slot (that stops the most common
accidental duplicate), but it is a safety net, not the contract. Tag the menu.

Where each toolkit puts the bar:

- **AppKit**: the system menu bar. Day prepends the standard **App menu** automatically,
  so your [`sub_menu`](https://daybrite.dev/api/day_pieces/fn.sub_menu.html)s start at *File*, not at a second app-name menu. Both the default bar and
  custom/reactive bars include About, Settings… (when provided), a registered system Services
  submenu, Hide App (⌘H), Hide Others (⌥⌘H), Show All, and Quit (⌘Q). Hide commands target
  [`NSApplication`](https://developer.apple.com/documentation/appkit/nsapplication); Close (⌘W) targets the key window. About/Preferences/Quit roles are hoisted
  recursively into the application menu, preserving callbacks and shortcuts; the first role
  declaration wins and duplicate roles, empty submenus, and dangling separators are removed.
  Settings uses the native macOS label. Application-menu chrome uses generated private catalog
  accessors in `day-toolkit-appkit/resource/locales`; app-authored menu labels remain app resources.
- **GTK**: a [`GtkPopoverMenuBar`](https://docs.gtk.org/gtk4/class.PopoverMenuBar.html) at the top of the window; accelerators registered on the
  [`GtkApplication`](https://docs.gtk.org/gtk4/class.Application.html). On macOS the model goes to [`gtk_application_set_menubar`](https://docs.gtk.org/gtk4/method.Application.set_menubar.html) instead; GTK's quartz
  toolkit renders it in the system menu bar, and the stock GTK app menu's *Settings…* item enables
  through an `app.preferences` action wired to the Preferences dispatch id.
- **Qt**: a [`QMenuBar`](https://doc.qt.io/qt-6/qmenubar.html) (the native global bar on macOS-qt).
- **Android**: the app-bar overflow (⋮), built by `DayActivity.onCreateOptionsMenu`.
- **WinUI**: a [`MenuBar`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.menubar) docked at the top of the window.
- **iOS/iPhone**: a no-op, because touch platforms have no persistent global menu bar; the native
  affordances are the per-Piece context menu and the system edit menu. (iPad/Catalyst [`UIMenuBuilder`](https://developer.apple.com/documentation/uikit/uimenubuilder)
  wiring is a future addition.)
- **Web**: a trailing menu button (the `⋯` overflow glyph, `#day-app-menu`) on the
  [toolbar strip](toolbars.md) the shim docks above the page, there whenever the installed
  model has anything the web can answer; the strip appears for it even when the app declared
  no toolbar items. Pressing it opens the model as a popover under the button
  ([below](#the-composed-presentation-web)); the popover is a real `role=menu`: its rows
  are `menuitem`s the arrow keys (and Home/End) walk, Enter or Space activates, Escape closes,
  and closing puts the focus back on the button, so every command, About included, is
  reachable without a shortcut. `Cap::AppMenu` answers `Emulated`. An app that
  installed no [`app_menu`](https://daybrite.dev/api/day_pieces/fn.app_menu.html) still gets the button when it registered Settings…, About or Check for
  Updates…, the items a desktop's default bar carries.

## How it works

The builder lowers to a flat, toolkit-neutral [`day_spec::MenuItem`] tree. Each item's closure is
registered with day-core, which hands back a process-unique **action id**; only the id travels into the
native menu. When the user chooses an item the toolkit emits `Event::MenuAction(id)`; the event pump
routes it to [`dispatch_menu_action`](https://daybrite.dev/api/day_core/menu/fn.dispatch_menu_action.html), which runs the closure inside a reactive batch (so signal writes
made from a menu coalesce into one update, just like a button tap). Standard roles carry no id; they
resolve to the toolkit's command instead. This keeps the crossing minimal (an integer), avoids
holding native handles across the FFI boundary, and lets any toolkit add menu support by implementing
just two `Toolkit` methods: [`set_app_menu`](https://daybrite.dev/api/day_spec/trait.Toolkit.html#method.set_app_menu) and `set_context_menu`.

## Re-installing the same menu

[`set_app_menu`](https://daybrite.dev/api/day_core/menu/fn.set_app_menu.html) compares the incoming model with the installed one, ignoring the action ids, because an app
declares its menu inside the page build, so every route change re-installs the same commands behind
freshly registered closures. A menu that differs only in those ids rebinds them onto the ids the
platform already holds and makes no toolkit call, which keeps a menu the user has open from closing
under them and stops the menu bar being rebuilt on every navigation. Anything else (a label, a
shortcut, an enablement, a new command) installs as before.

This is the rule the toolbar follows for the same reason ([docs/toolbars](toolbars.md), "Re-installing the same
bar"), where the rebuild also took the keyboard focus out of the search field.

## Nav-row context menus

A nav host's rows can each carry their own context menu ([`item`](https://daybrite.dev/api/day_pieces/fn.item.html)`(…)`[`.context_menu`](https://daybrite.dev/api/day_pieces/struct.NavItem.html#method.context_menu)`(vec![…])`
inside the [`.items`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.items) mapper, [docs/navigation](navigation.md)) for the sidebar idioms every desktop app
grows: per-feed "Mark all read", per-project "Reveal in Finder", the Showcase's per-page
"Show Source". The entries are the same builders as everywhere else and lower through the
same action registry, so a chosen entry dispatches identically to a piece context menu; the
menus re-lower (re-localizing their labels) whenever the rows re-derive, like the
row titles.

Per toolkit: AppKit serves them through the outline's `menuForEvent:` (NSTableView-family
views consume right-clicks themselves, so a menu attached to a cell's subviews would never
be consulted); UIKit through the table delegate's row-context hook (the standard long-press
row menu); GTK a per-row `PopoverMenu` with secondary-click + long-press gestures; Qt one
[`QMenu`](https://doc.qt.io/qt-6/qmenu.html) per row popped from the list's custom-context request; Android a best-effort
`setNavRowMenus` follow-up after the nav mounts (the same off-critical-path rule as the row
tints, [docs/vectors](vectors.md)). Web and ArkUI drop them for now, same as the piece decorator's
matrix.

## Platform notes

Platform guidance: [Apple: menus](https://developer.apple.com/design/human-interface-guidelines/menus); [Windows: menus and context menus](https://learn.microsoft.com/en-us/windows/apps/design/controls/menus-and-context-menus).

- **Nested submenus** are unlimited on the desktop toolkits and iOS. Android menus support a single
  level of submenu (a platform limit); deeper submenus flatten into the nearest one.
- **Separators** render as dividers everywhere; on Android they become menu-group boundaries (dividers
  on API 28+).
- A [`context_menu(vec![])`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.context_menu) (empty) or a later reconfigure detaches/replaces the menu on the Piece.
- **Checked items** reach every toolkit that renders a menu at all. Android draws the check inside a
  single level of submenu like any other item; the composed presentation draws its own ✓ column.
  One observed gap: under **macos-gtk** the item dispatches and its state is correct, but the mark
  does not appear in the global menu bar — GTK derives the check from the action's state, and the
  quartz bridge that mirrors the model into [`NSMenu`](https://developer.apple.com/documentation/appkit/nsmenu) does not carry it across. Linux GTK's
  in-window `PopoverMenuBar` resolves the same action group directly and is unaffected.

## Dock menu

[`dock_menu(entries)`](https://daybrite.dev/api/day_pieces/fn.dock_menu.html) installs the items macOS lists when the user right-clicks the app's Dock
icon, above the system's Show, Hide and Quit (`Cap::DockMenu`). The entries are the same
`MenuEntry` tree as [`app_menu`](https://daybrite.dev/api/day_pieces/fn.app_menu.html), with ordinary closures, because the Dock menu only exists while
the app runs. It is live like [`app_menu`](https://daybrite.dev/api/day_pieces/fn.app_menu.html): a derived closure rebuilds it whenever a signal it
reads changes, for a "Resume <document>" item or a check mark:

```rust
use day::prelude::*;

fn install_dock_menu(timer_running: Signal<bool>) {
    dock_menu(move || {
        vec![
            menu_item(if timer_running.get() {
                res::str::pause_timer()
            } else {
                res::str::start_timer()
            })
            .action(move || timer_running.set(!timer_running.get())),
            menu_item(res::str::new_note()).action(new_note),
        ]
    });
}
```

The app's launcher shortcuts ([docs/deep-links](deep-links.md#launcher-shortcuts)) come first
in the menu, then these entries. Other platforms have no Dock menu; their equivalents are the
shortcuts themselves: Windows jump lists, Linux `.desktop` actions, iOS quick actions, Android
launcher shortcuts.

## Runtime language changes

The app menu is live. Every title, [`enabled`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.enabled) and [`checked`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.checked) source is re-read, and the menu
re-lowered and re-installed, whenever one of them changes: [`menu_role`](https://daybrite.dev/api/day_pieces/fn.menu_role.html) labels, `res::str`
titles and [`day::tr`](https://daybrite.dev/api/day/fn.tr.html) all read the locale signal, so a preferences language picker
([docs/windows](windows.md)) retitles the whole bar, and a derived closure
([`app_menu(move || vec![..])`](https://daybrite.dev/api/day_pieces/fn.app_menu.html)) re-runs on whatever else it reads. Replacement drops the
previous install's action closures; context menus share the dispatch map and are untouched,
and the durable Preferences/New Window dispatch ids always survive.

## The auto Preferences item + the Window menu

`day::register_preferences*` gives every platform its standard Settings…/Preferences item
(⌘, / Ctrl+comma; on the web the item has no shortcut, because the browser answers that key with
its own settings before the page sees it) without any menu code in the app, and macOS also auto-installs the standard
Window menu. The mechanics (injection into an installed menu, role rewiring, and the
[`register_new_window`](https://daybrite.dev/api/day_core/windows/fn.register_new_window.html) builder) are described in [docs/windows](windows.md).

## Driving menus from dayscript

`menu: { id: "view_grid" }` invokes the item the app NAMED with [`.id`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.id). Prefer
it to everything below: it is the only address that survives a locale change or a moving check mark,
and it never falls back to matching a label, so a step naming an id that is not in the menu fails
instead of quietly hitting some other item that reads the same.

`menu: { item: "Save" }` invokes a unique app-menu action by exact label;
`menu: { key: menu_save }` resolves a Fluent key in the run's locale first (locale-portable:
app keys and the `day_*` role keys both work, so the auto Preferences item is
`key: day_preferences`, with or without an installed app menu). `path: [File]`
disambiguates by ancestor submenu: each entry matches a submenu's literal label or its
Fluent key resolved in the run's locale, so `path: [menu_file]` works wherever
`key: menu_file` does and one script stays valid in every language. The step dispatches the
registered day action directly (toolkit-uniform, no native menu automation), so role-only
items that run a native selector (Cut, Quit, …) are not invokable this way.

## Dynamic context menus

[`.context_menu(items)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.context_menu) is declarative: one menu, set at build time. Some surfaces cannot
know their menu until the click lands (a canvas whose commands describe the selection under
the pointer, a tree whose rows each mean something different), so two summon-time forms
exist ([docs/tree](tree.md) is the driving case):

- [`.context_menu(|point: Point| … -> Vec<MenuEntry>)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.context_menu) on any piece (the same [`context_menu`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.context_menu)
  that takes a fixed list; a closure taking the point must type its argument): the closure runs
  when the user summons the menu (right-click on desktop, long-press on touch), receives the
  location in the piece's own coordinates, and whatever it returns is shown. A closure with no
  argument ([`.context_menu(|| …)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.context_menu)) builds on summon without the point. An empty result shows
  nothing. The closure may adjust app state first (select what is under the pointer, then
  build); it runs on the UI thread, outside any day-core borrow, like every other synchronous
  Toolkit callback.
- [`tree`](https://daybrite.dev/api/day_pieces/fn.tree.html)`(…)`[`.row_context_menu`](https://daybrite.dev/api/day_pieces/trait.TreeBuilder.html#tymethod.row_context_menu)`(|key| … -> Vec<MenuEntry>)`: the per-row form, handed the row's
  key. By convention, a summon on a row outside the current selection selects that row
  first, so the menu describes what it acts on.

Layout-only decorators such as [`.padding`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.padding) and [`.frame`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame) forward dynamic menus to the first
native root beneath them, just like static menus.

Action closures are lowered per summon into their own scope, disposed when the next summon
(or the piece's teardown) replaces them, so per-click menus never accumulate registrations.

Toolkits: the duty is [`Toolkit::set_context_menu_fn`](https://daybrite.dev/api/day_spec/trait.Toolkit.html#method.set_context_menu_fn) (default no-op). AppKit serves it from
`menuForEvent:` on the canvas view and the tree's outline; GTK from a button-3 click (and a
long-press) building a one-summon `PopoverMenu`; UIKit from [`UIContextMenuInteraction`](https://developer.apple.com/documentation/uikit/uicontextmenuinteraction),
whose configuration callback is already summon-time, and the tree's rows from the
collection view's own `contextMenuConfigurationForItemAtIndexPath`. On AppKit the covered
surfaces are canvases, tree rows, and recycling list rows. Lists resolve the provider from
the clicked visible cell (including its layout wrappers), preserve selection, and invoke it
only at summon time. Empty menus suppress presentation; blank list space does not invoke a
row provider. Container and plain-label clicks resolve the nearest dynamic owner through
the live view hierarchy, so recycled descendants never hold copied providers. Selectable
labels keep their native text menu. Other controls keep the static `.menu` path. The native regression is
`crates/day-toolkit-appkit/tests/native_list_menus.rs`. GTK's
form is generic over any widget. Qt connects `customContextMenuRequested` to a callback
that asks the provider and pops a fresh [`QMenu`](https://doc.qt.io/qt-6/qmenu.html) per summon (`day_qt_context_menu_fn`),
generic over any widget like GTK's.

### The composed presentation (Web)

A toolkit with no native menu to hand the model to reports the summon instead: Web's
shim listens for the browser's `contextmenu` (default prevented; **primary-button taps and
drags ignore the right button**, or the summon's own pointer-up would come out as a tap and
re-target the selection under the menu it just built) and emits
`Event::ContextMenu { local, window }`, carrying the point in the node's coordinates for the
app's provider and the same point in window coordinates for placement. The `.context_menu*`
decorators mount a lazily-armed, unrouted cover beside the decorated node (inside an
overlay-host wrapper, so single-child layouts still reach it in the place pass); on the
event they run the same provider and present the lowered model as day pieces at the summon
point, pulled inside the window near edges: item rows (`day-menu-item-N` ids, so a browser
test can click them), separators, inlined submenus, role items resolved to their standing
dispatchers, and disabled items dimmed. A tap outside dismisses. Toolkits that serve
menus natively never emit the event, so the fallback costs them nothing, not even a node,
until a first summon that never comes.

The app menu takes the other route: the toolkit keeps the model. [`set_app_menu`](https://daybrite.dev/api/day_spec/trait.Toolkit.html#method.set_app_menu) reduces it to
what the web can answer (window and process roles out, a role that lowered without a day
action out, empty submenus and stranded separators pruned, an injected role item given its
catalog label) and hands it to the shim as the trailing button's pull-down, so the popover is
the one a [toolbar pull-down](toolbars.md) already gets: item rows (`data-menu-id` carries the
app's [`.id`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.id), for a browser test), separators, inlined submenus, a check column, dimmed
disabled rows, the shortcut hint at each row's trailing edge, dismissed by a press outside or
Escape, pulled back inside the window at the right edge. A chosen row emits
`Event::MenuAction(id)` like every other toolkit's item, so [`dispatch_menu_action`](https://daybrite.dev/api/day_core/menu/fn.dispatch_menu_action.html) runs the
app's closure, and the role items reach the standing dispatchers they lowered to. The model's
accelerators are live ([above](#keyboard-shortcuts)). A dayscript `menu:` step never needs the
popover: it dispatches by action id straight from the installed model.

An app whose context menu commands need the clipboard (Cut/Copy/Paste items) calls
[`day::invoke_edit(EditOp::…)`](https://daybrite.dev/api/day/fn.invoke_edit.html), the same handler (transport included) that the platform's own
Edit route reaches, instead of duplicating the clipboard plumbing.
