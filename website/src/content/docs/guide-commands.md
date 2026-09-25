---
title: Reusable commands
description: "Share an operation, its title, and availability across buttons, menus, and toolbars."
order: 33
section: Guides
---

Define a `Command { id, label, action }` when several controls perform the same operation.
Call `.build()` to obtain a cloneable `CommandHandle`, then configure availability, optional
checked state, icon, and keyboard shortcut with its modifiers.
A menu item that is still visible after the operation becomes unavailable cannot bypass the
command's guard.

```rust
use day::prelude::*;

let starred = Signal::new(false);
let star = Command {
    id: "star",
    label: move || {
        if starred.get() { "Unstar" } else { "Star" }.to_string()
    },
    action: move || starred.update(|on| *on = !*on),
}
.build()
.checked(starred)
.icon(Symbol::Star)
.shortcut(Shortcut::new("d"));

let menu = star.clone();
let bar = star.clone();
let content = star.button()
    .context_menu_fn(move |_| vec![menu.menu_item()])
    .toolbar(move || vec![bar.toolbar_item()]);
```

Use localized resources in application titles, just as for a normal button. Add
`.enabled(move || ...)` for availability, and call `.invoke()` from a custom gesture or another
caller. It returns whether the handler ran; it does not report the result of asynchronous work.

## Choose the owning scope and target

Handle clones share behavior and reactive sources. `.build()` captures the owning reactive scope;
the struct literal alone does not. Call `.build()` in the scope that owns the operation; the handle
stops accepting invocations after that scope is disposed. Captured signals must still outlive
their use. Command factory functions return `CommandHandle`. Capture a window's scene
for a window command, or resolve the focused scene inside an app-menu command. Commands do not
implicitly select a window or redirect an operation to whichever document is frontmost.

A command's id defaults each presentation's id. Override it when two controls in one tree need
distinct ids, or to preserve an existing Dayscript target:

```rust
star.button().id("star-footer");
star.toolbar_item().id("star-toolbar");
```

## Keep the presentations reactive

Buttons bind automatically. Use a derived `.toolbar(move || ...)` for changing titles, and
`app_menu_reactive` or `context_menu_fn` for current menu state. A fixed menu remains a snapshot;
its handler still rechecks availability. Toolbar check and enabled state bind live even in fixed
lists. The application handler owns check changes; native toggle activation does not write a
second mirror signal.

## What this does and does not abstract

The adapters retain each toolkit's native controls and existing menu/toolbar behavior. They do
not expose a Qt `QAction` or Windows `ICommand`, introduce a new command palette, or register
OS-wide shortcuts. Shortcuts take effect through installed menu items, not a standalone button.
Continue using `menu_role` for native Copy/Paste/Undo and responder-chain targeting.

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

See the [full design, lifecycle rules, platform notes, and app audit](/docs/internal/commands),
plus [menus and toolbars](/docs/guide-desktop). Day-Showcase's Menus & dialogs page demonstrates
one counter through content buttons, toolbar items, and a context menu. Its page stars and
recorder playback also use the framework abstraction.

For other UI components, named definitions are most useful for helpers with several required
arguments. Existing one-argument functions and fluent modifiers remain the normal API. See the
[assessment of additional named component definitions](/docs/internal/commands#assessment-named-definitions-for-other-components)
for candidates, coexistence with functions, and the distinction between a definition and a
runtime piece. Those additional wrappers are proposals, not implemented APIs.

Implementation:
[`crates/day-pieces/src/commands.rs`](https://github.com/daybrite/day/blob/main/crates/day-pieces/src/commands.rs).
