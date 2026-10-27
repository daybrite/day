---
title: Menus, toolbars, and windows
description: "Install a native menu bar with standard roles and shortcuts, put commands in the window's toolbar, and open secondary windows, including the standard Settings window, from your app."
order: 32
section: How-to guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Menus, keyboard shortcuts, and toolbars expose an app’s commands through familiar platform
controls. For one operation shared across these surfaces, define a [reusable command](/docs/how-to/guide-commands).
Day uses a shared Rust API to define them, along with secondary windows and a Settings
window. A menu item can call the same action as a button in the interface:

```rust
menu_item(res::str::save()).key("s").action(save) // ⌘S on macOS, Ctrl+S everywhere else
```

**Check the operation's capability.** Native menu bars, context menus, toolbars, and secondary
windows have different target mappings. The DOM backend reports emulated toolbar and app-menu
support; mobile backends can expose navigation chrome instead of desktop menus. An unsupported
secondary native window falls back to a fullscreen cover. The
[coverage matrix](https://github.com/daybrite/day/blob/main/docs/coverage-matrix.md) and subsystem references
record source behavior, not native test results.

The code blocks are integration fragments. Start in a working app, import `day::prelude::*`,
and supply app callbacks such as `save`. The [command recipe](/docs/how-to/guide-commands)
contains a compile-designated operation that can feed these presentations. A menu shortcut is
installed by a menu entry; a standalone command button does not register a global hotkey.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## 1. Install the app menu

Call [`app_menu`](https://daybrite.dev/api/day_pieces/fn.app_menu.html) once at startup, with one [`sub_menu`](https://daybrite.dev/api/day_pieces/fn.sub_menu.html) per menu-bar menu:

```rust
use day::prelude::*;

app_menu(vec![
    sub_menu(
        res::str::file(),
        vec![
            menu_role(MenuRole::NewWindow), // File ▸ New Window, ⌘N (step 5)
            menu_item(res::str::open()).key("o").action(|| open_file()),
            menu_item(res::str::save()).key("s").action(|| save()),
            menu_separator(),
            menu_role(MenuRole::CloseWindow),
            menu_role(MenuRole::Quit),
        ],
    )
    .bar_role(MenuBarRole::File),
    sub_menu(
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
    )
    .bar_role(MenuBarRole::Edit),
]);
```

- Roles are the platform's items. [`menu_role(MenuRole::Copy)`](https://daybrite.dev/api/day_pieces/fn.menu_role.html) emits the native Edit ▸ Copy:
  correct localized label, default shortcut, automatic enable/disable, and focus targeting, so it
  copies from whatever control has focus with no wiring. Custom [`menu_item`](https://daybrite.dev/api/day_pieces/fn.menu_item.html)s run your closure
  instead.
- [`.key("s")`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.key) is the primary modifier (⌘ on Apple, Ctrl elsewhere), so one spec reads right on every
  desktop. For anything else, build a `Shortcut`: [`Shortcut::primary`](https://daybrite.dev/api/day_spec/struct.Shortcut.html#method.primary)`("s")`[`.shift`](https://daybrite.dev/api/day_spec/struct.Shortcut.html#method.shift)`()` is ⇧⌘S /
  Ctrl+Shift+S, [`Shortcut::plain("Delete")`](https://daybrite.dev/api/day_spec/struct.Shortcut.html#method.plain) has no modifier, and [`.alt()`](https://daybrite.dev/api/day_spec/struct.Shortcut.html#method.alt) / [`.control()`](https://daybrite.dev/api/day_spec/struct.Shortcut.html#method.control) add the
  rest. Named keys (`"Return"`, `"Delete"`, `"F5"`, arrows) work alongside single characters.
- [`.bar_role(…)`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.bar_role) claims a standard slot. Each desktop fills the standard menus (Edit, View, Help)
  with its own stock version for any slot you didn't claim. Tagging your submenu with
  `MenuBarRole::File` / `Edit` / `View` replaces the stock menu in place, in the bar's standard
  order. The tag identifies the slot, not the title; Day's catalog and yours may translate the same
  menu name differently, and a bar matched on titles would show both.

The bar appears in the system menu bar on macOS (Day prepends the standard App menu with
About and Quit, so your submenus start at File), in a bar at the top of the window on GTK and
Windows, in a `QMenuBar` on Qt (the native global bar on `macos-qt`), and in the app-bar
overflow on Android. Android allows one level of submenu; deeper ones flatten.

The menu is live: every title, [`enabled`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.enabled) and [`checked`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.checked) is re-read and the bar reinstalled when
one changes, so a runtime language picker retitles it in the new [locale](/docs/reference/glossary#locale)
with no extra code. [`app_menu`](https://daybrite.dev/api/day_pieces/fn.app_menu.html) also takes a closure ([`app_menu(move || vec![…])`](https://daybrite.dev/api/day_pieces/fn.app_menu.html)) that re-runs
on whatever else it reads.

## 2. Attach context menus

The same entries attach to any [piece](/docs/reference/glossary#piece) with [`.context_menu(…)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.context_menu), shown on secondary-click on
desktop and long-press on touch:

```rust
label(res::str::right_click_me()).context_menu(vec![
    menu_item(res::str::rename()).action(|| rename()),
    menu_item(res::str::duplicate())
        .key("d")
        .action(|| duplicate()),
    menu_separator(),
    menu_role(MenuRole::Copy),
])
```

Submenus nest inside a context menu the same way, [`menu_role`](https://daybrite.dev/api/day_pieces/fn.menu_role.html) items keep their native
behavior, and passing an empty `Vec` removes the menu.

## 3. Put commands in a toolbar

Attach [`.toolbar(…)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.toolbar) to the piece whose content the commands affect. A toolbar on the window’s
root piece stays available throughout that window. A toolbar on a page appears with that page
and disappears when it leaves the screen. The platform decides where to place those commands;
the [toolbar reference](/docs/reference/toolbars) describes each toolkit’s behavior.

```rust
item_list(scene).grow().toolbar([
    toolbar_toggle(id!(show_done), res::str::show_done(), scene.show_done).icon(Symbol::Filter),
    toolbar_button(id!(add), res::str::add())
        .icon(Symbol::Add)
        .placement(ToolbarPlacement::Primary)
        .action(move || scene.new_item()),
])
```

The items are [`toolbar_button(id, label)`](https://daybrite.dev/api/day_pieces/fn.toolbar_button.html) for a command, [`toolbar_toggle(id, label, signal)`](https://daybrite.dev/api/day_pieces/fn.toolbar_toggle.html)
for a two-state button bound two-way, [`toolbar_segmented(id, segments, signal)`](https://daybrite.dev/api/day_pieces/fn.toolbar_segmented.html) for one native
segmented control, [`toolbar_menu(id, label, entries)`](https://daybrite.dev/api/day_pieces/fn.toolbar_menu.html) for a pull-down built from the same
`MenuEntry`s the menu bar takes, [`toolbar_label(id, text)`](https://daybrite.dev/api/day_pieces/fn.toolbar_label.html) for static text, and
[`toolbar_separator(id)`](https://daybrite.dev/api/day_pieces/fn.toolbar_separator.html) for a divider. The modifiers are [`.icon(Symbol)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.icon), [`.image(name)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.image),
[`.action(f)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.action), [`.tooltip(t)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.tooltip), [`.enabled(on)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.enabled) (a `bool`, a signal or a closure), [`.placement(…)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.placement),
[`.label_style(…)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.label_style), and [`.prominent()`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.prominent).

Search has no toolbar item. Declare it on the navigation surface it filters, with
[`nav`](https://daybrite.dev/api/day_pieces/fn.nav.html)`(section)`[`.searchable`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.searchable)`(`[`Search::new`](https://daybrite.dev/api/day_pieces/struct.Search.html#method.new)`(query))`, and Day draws the field where the platform puts search.
That lets it move into the navigation list on a window too narrow for a [sidebar](/docs/reference/glossary#sidebar) without your code
changing. A sidebar supplies its own toggle button, so an app declares nothing for that either.

Alignment comes from [`.placement(…)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.placement), which names the item's role rather than a position in the
list, and each backend lays that role out its own way: `Navigation` sits at the leading edge of its
column, `Principal` is centered, `Primary` and `Secondary` go trailing, with secondaries folding
into an overflow menu first, and `Bottom` asks for a phone's bottom bar. [`.icon(Symbol::Refresh)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.icon)
names what the icon means; each backend draws its platform's glyph (an SF Symbol on macOS, a
freedesktop name on GTK and Qt, a Segoe Fluent glyph on Windows).

Per desktop, the bar is an `NSToolbar` in the unified title-bar style on macOS, where each
column's items sit over that column; on GTK the items pack into the window's `AdwHeaderBar`,
because in GNOME the header bar is the toolbar; on Qt it is a real `QToolBar` that takes its
icon size and style from the user's settings; on Windows it is a `CommandBar`, whose one limit
is that search fields and labels always render on the leading side.

[`.toolbar(…)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.toolbar) also takes a closure, [`page.toolbar(move || vec![…])`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.toolbar), which re-runs whenever the
state it reads changes and replaces that piece's items. Keep the values that change often out of
that closure: a toggle's [signal](/docs/reference/glossary#signal), a search field's signal, and a
reactive [`.enabled(…)`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.enabled) patch the one item in place, so a command greying out never disturbs a
search in progress. To show an item conditionally, put its piece under [`when`](https://daybrite.dev/api/day_pieces/fn.when.html); the item leaves with its
subtree. The [Toolbars](/docs/reference/toolbars) reference covers placement per platform and what
each backend draws.

## 4. Open a secondary window

```rust
let win = day::open_window(
    res::window_options()
        .title("AAPL")
        .size(720.0, 640.0)
        .key("detail:AAPL"),              // open-or-focus singleton; omit for always new
    || detail_page("AAPL"),
);
win.on_close(|| println!("gone"));
```

`res::window_options()` is generated with the app's locale catalog, title and version; the
builders set the rest. [`.key(..)`](https://daybrite.dev/api/day_pieces/struct.MenuEntry.html#method.key) names the logical window: opening an already-open key focuses
it instead of duplicating, and [`day::window_by_key("detail:AAPL")`](https://daybrite.dev/api/day/fn.window_by_key.html) finds it later. The default
kind, `WindowKind::Normal`, is resizable, miniaturizable, and joins the platform's tabbing
group; [`.kind(WindowKind::Preferences)`](https://daybrite.dev/api/day_spec/struct.WindowOptions.html#method.kind) drops resize and minimize and never tabs. The window is app-owned: it survives the page that
opened it. Close is asynchronous everywhere: the title-bar button, a platform gesture, and
[`WindowHandle::close()`](https://daybrite.dev/api/day_core/windows/struct.WindowHandle.html#method.close) all wait for the platform to confirm, then the content is disposed and
`on_close` runs. Closing the primary window quits the app.

Where the [toolkit](/docs/reference/glossary#toolkit) cannot open windows (iPhone, web, and the `Preferences`
kind on all mobile), the content presents as a fullscreen cover in the primary window instead, with
the same API, keys, and close path. That tier has no native title bar or close button, so probe
`Cap::MultiWindow` and give cover-tier content its own close affordance (the system back button
closes it on Android).

## 5. Add Settings, About and File ▸ New Window

Settings and About come from [`day-piece-settings`](/docs/reference/settings) ([settings](/docs/reference/settings)): one
declaration in your root builder, before [`app_menu`](https://daybrite.dev/api/day_pieces/fn.app_menu.html), registers both windows and their menu
items on every desktop and the navigation presentation on the phones:

```rust
use day_piece_settings::prelude::*;

let prefs = Preferences::new()
    .about(About::from_manifest().website("https://example.org"))
    .group(general_group(res::locales::ALL))         // Appearance + Language
    .group(Group::new("editing", res::str::settings_editing())
        .icon(res::vectors::prefs_editing)
        .row(rows::toggle(&SNAP, res::str::pref_snap())))
    .install();
day::register_new_window(|| {
    shell()                 // toolbar items come from the pieces in this window
});
```

On macOS that is "About <App>" and "Settings…" with ⌘, in the App menu; on GTK, Qt and Windows
a Preferences item with Ctrl+comma (in your first menu unless you placed
[`menu_role(MenuRole::Preferences)`](https://daybrite.dev/api/day_pieces/fn.menu_role.html) yourself) and Help ▸ About. The Settings window opens under
the singleton key `day.preferences`, so reopening focuses it; [`day_piece_settings::open()`](https://daybrite.dev/api/day_piece_settings/fn.open.html)
opens it from anywhere, such as a toolbar gear, and [`navigate("settings/editing")`](https://daybrite.dev/api/day_pieces/fn.navigate.html) opens it
on a group. [`About::from_manifest()`](https://daybrite.dev/api/day_piece_settings/struct.About.html#method.from_manifest) reads the name and version from `Day.toml`.

[`register_new_window`](https://daybrite.dev/api/day_core/windows/fn.register_new_window.html) names the builder behind [`menu_role(MenuRole::NewWindow)`](https://daybrite.dev/api/day_pieces/fn.menu_role.html) (File ▸ New
Window with ⌘N/Ctrl+N) and the macOS tab-bar "+". Each call opens an independent `Normal`
window. On macOS, Day also installs the standard Window menu (Minimize, Zoom, Bring All to
Front, plus the open-window list) unless your own menu claims `MenuRole::Minimize`.

## Pitfalls

- Register windows before the menu. A `MenuRole::NewWindow` item is disabled when no
  builder is registered, and the auto Settings and About items need their registrations. Call
  [`Preferences::install()`](https://daybrite.dev/api/day_piece_settings/struct.Preferences.html#method.install) and [`register_new_window`](https://daybrite.dev/api/day_core/windows/fn.register_new_window.html) before installing the app menu (the
  showcase's `root()` does exactly this), so those menu items are enabled.
- Toolbars follow their pieces into new windows. Items declared on a window’s root
  piece belong to that window. Items declared on a page appear wherever that page is shown. A builder whose pieces declare none opens a window with an empty bar.
- Keep bound values out of a derived toolbar closure. A [reactive](/docs/reference/glossary#reactive) rebuild replaces that
  piece's items and would drop the search field's focus mid-word. Structure and labels go in
  the closure; a toggle's signal, a search signal, and a reactive [`.enabled`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.enabled) patch single items.
- Don't put the toolkit name in your window title. Debug builds append a
  `(<version>/<toolkit>)` tag to every title so you can tell windows apart; add your own and
  it appears twice. Release builds never show the tag.

## Reference

- [menus](/docs/reference/menus) — the full role table per backend, how action dispatch works,
  and driving menus from [dayscript](/docs/reference/glossary#dayscript).
- [toolbars](/docs/reference/toolbars) — the per-backend realization table, patch semantics,
  and the `toolbar:` script step.
- [windows](/docs/reference/windows) — the backend tier table, the pending-open path, the
  debug title tag, and per-window screenshots.

## Verify scope and presentation

Create a command in the scope that owns its data, then install its button/menu/toolbar adapters.
Test invocation after disabling it, focus changes between windows, and closing the owning page.
[`Command::invoke`](https://daybrite.dev/api/day_pieces/struct.Command.html#method.invoke) rechecks both availability and scope liveness; it reports that a synchronous
handler ran, not the result of work started inside [`day::task`](https://daybrite.dev/api/day/fn.task.html). Window lifetimes can exceed the
page that opened them, so captured state must live long enough.

Evidence: [commands](https://github.com/daybrite/day/blob/main/crates/day-pieces/src/commands.rs),
[menus](https://github.com/daybrite/day/blob/main/crates/day-pieces/src/menus.rs), and
[windows](https://github.com/daybrite/day/blob/main/crates/day-core/src/windows.rs).
Native accelerator placement and secondary-window behavior still require target runtime checks.
