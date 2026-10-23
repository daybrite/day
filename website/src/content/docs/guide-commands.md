---
navTitle: "Commands"
title: Reusable commands
description: "Share an operation, its title, and availability across buttons, menus, and toolbars."
order: 33
section: How-to guides
---

Define a [`Command::new(id, label, action)`](https://daybrite.dev/api/day_pieces/struct.Command.html#method.new) when several controls perform the same operation.
It is cloneable, and its modifiers configure availability, optional checked state, icon, and
keyboard shortcut.
A menu item that is still visible after the operation becomes unavailable cannot bypass the
command's guard.

```rust
use day::prelude::*;

let starred = Signal::new(false);
let star = Command::new(
    id!(star),
    move || {
        if starred.get() {
            res::str::unstar().format()
        } else {
            res::str::star().format()
        }
    },
    move || starred.update(|on| *on = !*on),
)
.checked(starred)
.icon(Symbol::Star)
.shortcut(Shortcut::primary("d"));

let menu = star.clone();
let bar = star.clone();
let content = star
    .button()
    .context_menu(move || vec![menu.menu_item()])
    .toolbar(move || vec![bar.toolbar_item()]);
```

Use localized resources in application titles, just as for a normal button. Add
[`.enabled(move || ...)`](https://daybrite.dev/api/day_pieces/struct.Command.html#method.enabled) for availability, and call [`.invoke()`](https://daybrite.dev/api/day_pieces/struct.Command.html#method.invoke) from a custom gesture or another
caller. It returns whether the handler ran; it does not report the result of asynchronous work.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/examples/catalog) and [catalog setup](/docs/how-to/localization).

## Build a guarded command

Start in a live UI scope. This module defines one action and binds its enabled state to the same
signal as its label. The numeric limit is sample policy. The other snippets on this page are
integration fragments.

Complete module for `src/commands.rs` in the [technical recipe harness](/docs/reference/examples/catalog#technical-recipes).
The harness compiles this exact block; it is not a standalone app. Add the catalog entries below
to `resource/locales/en/app.ftl` and declare the module in `src/lib.rs`.

<!-- technical-example: commands -->
```rust
use crate::res;
use day::prelude::*;

pub fn increment_command(count: Signal<i64>) -> Command {
    Command::new(
        id!(recipe_increment),
        res::str::recipe_increment(),
        move || {
            count.update(|value| *value += 1);
        },
    )
    .enabled(move || count.get() < 10)
}
```

<!-- technical-catalog: commands -->
```ftl
recipe_increment = Add one
```

## Choose the owning scope and target

Clones share behavior and reactive sources. [`Command::new`](https://daybrite.dev/api/day_pieces/struct.Command.html#method.new) captures the owning reactive scope,
so build the command in the scope that owns the operation; it stops accepting invocations after
that scope is disposed. Captured signals must still outlive their use. Command factory functions
return `Command`. Capture a window's scene
for a window command, or resolve the focused scene inside an app-menu command. Commands do not
implicitly select a window or redirect an operation to whichever document is frontmost.

A command's id defaults each presentation's id. Override it when two controls in one tree need
distinct ids, or to preserve an existing Dayscript target:

```rust
star.button().id(id!(star_footer));
star.toolbar_item().id(id!(star_toolbar));
```

## Keep the presentations reactive

Buttons bind automatically. Use a derived [`.toolbar(move || ...)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.toolbar) for changing titles. The app
menu is live (a command's menu item re-lowers it when the title or availability changes), and a
[`.context_menu(move || ...)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.context_menu) closure builds the menu on summon. A fixed [`context_menu(vec![..])`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.context_menu)
remains a snapshot; its handler still rechecks availability. Toolbar check and enabled state bind live even in fixed
lists. The application handler owns check changes; native toggle activation does not write a
second mirror signal.

## What this does and does not abstract

The adapters retain each toolkit's native controls and existing menu/toolbar behavior. They do
not expose a Qt `QAction` or Windows `ICommand`, introduce a new command palette, or register
OS-wide shortcuts. Shortcuts take effect through installed menu items, not a standalone button.
Continue using [`menu_role`](https://daybrite.dev/api/day_pieces/fn.menu_role.html) for native Copy/Paste/Undo and responder-chain targeting.

- AppKit, GTK, Qt, and XAML retain their native menu and toolbar presentation; primary modifiers
  are Command on Apple and Ctrl elsewhere. GTK uses header-bar controls; XAML placement remains
  constrained by `CommandBar`.
- UIKit contributes commands to navigation bars and context menus. An iPhone app must provide a
  visible route to operations instead of relying on the application menu.
- Android keeps Material app-bar/overflow behavior, including restricted nesting and optional
  omission of menu icons. Menu shortcut metadata is not a cross-platform hardware-keyboard API.
- Web uses DOM buttons and composed toolbar/context-menu surfaces. Browser-reserved shortcuts
  stay reserved, and there is no native application menu.
- ArkUI supports the button path. A reusable command does not add unsupported application-menu
  or toolbar chrome; keep content controls available.

See the [full design, lifecycle rules, platform notes, and app audit](/docs/reference/commands),
plus [menus and toolbars](/docs/how-to/guide-desktop). Day-Showcase's Menus & dialogs page demonstrates
one counter through content buttons, toolbar items, and a context menu. Its page stars and
recorder playback also use the framework abstraction.

For other UI components, named definitions are most useful for helpers with several required
arguments. Existing one-argument functions and fluent modifiers remain the normal API. See the
[assessment of additional named component definitions](/docs/reference/commands#assessment-named-definitions-for-other-components)
for candidates, coexistence with functions, and the distinction between a definition and a
runtime piece. Those additional wrappers are proposals, not implemented APIs.

Implementation:
[`crates/day-pieces/src/commands.rs`](https://github.com/daybrite/day/blob/main/crates/day-pieces/src/commands.rs).
