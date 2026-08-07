---
title: Menus, toolbars, and windows
description: "Install a native menu bar with standard roles and shortcuts, put commands in the window's toolbar, and open secondary windows — including the standard Settings window — from Rust."
order: 32
section: Guides
---

A Day app on the desktop should behave like it was written for the desktop: a menu bar with
keyboard shortcuts, commands on the window chrome, and a Settings window under ⌘,. Day builds
all three from small Rust builders and hands them to the platform's real machinery — `NSMenu`
and `NSToolbar` on macOS, `GtkPopoverMenuBar` and the header bar on GTK, `QMenuBar` and
`QToolBar` on Qt, XAML's `MenuBar` and `CommandBar` on Windows — so one spec is correct
everywhere:

```rust
menu_item("Save").key("s").action(save)     // ⌘S on macOS, Ctrl+S everywhere else
```

**Works on:** context menus render natively everywhere (`NSMenu`, `GtkPopoverMenu`, `QMenu`,
`UIMenu`, Android `PopupMenu`, XAML `MenuFlyout`). The app menu is a menu bar on the four
desktop backends and the app-bar overflow (⋮) on Android; on iPhone it is a no-op by design —
touch platforms have no global menu bar. Toolbars exist only where the platform has them:
`Cap::Toolbar` is `Native` on the four desktop backends and `Unsupported` everywhere else.
Secondary windows work on every backend — native windows on the desktops, iPad, Android, and
HarmonyOS; on iPhone and web the same call presents the content as a fullscreen cover.

## 1. Install the app menu

Call `app_menu` once at startup, with one `sub_menu` per menu-bar menu:

```rust
use day::prelude::*;

app_menu(vec![
    sub_menu("File", vec![
        menu_role(MenuRole::NewWindow),                       // File ▸ New Window, ⌘N (step 5)
        menu_item("Open…").key("o").action(|| open_file()),
        menu_item("Save").key("s").action(|| save()),
        menu_separator(),
        menu_role(MenuRole::CloseWindow),
        menu_role(MenuRole::Quit),
    ])
    .bar_role(MenuBarRole::File),
    sub_menu("Edit", vec![
        menu_role(MenuRole::Undo), menu_role(MenuRole::Redo),
        menu_separator(),
        menu_role(MenuRole::Cut), menu_role(MenuRole::Copy),
        menu_role(MenuRole::Paste), menu_role(MenuRole::SelectAll),
    ])
    .bar_role(MenuBarRole::Edit),
]);
```

Three things to notice:

- **Roles are the platform's own items.** `menu_role(MenuRole::Copy)` emits the native Edit ▸
  Copy: correct localized label, default shortcut, automatic enable/disable, and focus
  targeting — it copies from whatever control has focus, with no wiring. Custom `menu_item`s
  run your closure instead.
- **`.key("s")` is the primary modifier** (⌘ on Apple, Ctrl elsewhere), so one spec reads
  right on every desktop. For anything else, build a `Shortcut`: `Shortcut::new("s").shift()`
  is ⇧⌘S / Ctrl+Shift+S, `Shortcut::plain("Delete")` has no modifier, and `.alt()` /
  `.control()` add the rest. Named keys (`"Return"`, `"Delete"`, `"F5"`, arrows) work
  alongside single characters.
- **`.bar_role(…)` claims a standard slot.** Each desktop fills the standard menus (Edit,
  View, Help) with its own stock version for any slot you didn't claim. Tagging your submenu
  with `MenuBarRole::File` / `Edit` / `View` replaces the stock menu in place, in the bar's
  standard order. The tag identifies the slot, not the title — Day's catalog and yours may
  translate the same menu name differently, and a bar matched on titles would show both.

Where the bar lands: the system menu bar on macOS (Day prepends the standard App menu with
About and Quit, so your submenus start at File), a bar at the top of the window on GTK and
Windows, a `QMenuBar` on Qt (the native global bar on `macos-qt`), and the app-bar overflow on
Android. Android allows one level of submenu; deeper ones flatten.

`app_menu` resolves labels once, in the install-time locale. If your app has a runtime
language picker, install with `app_menu_reactive(builder)` instead — the builder re-runs on a
locale change and reinstalls the bar in the new language.

## 2. Attach context menus

The same entries attach to any piece with `.context_menu(…)`, shown on secondary-click on
desktop and long-press on touch:

```rust
label("Right-click me").context_menu(vec![
    menu_item("Rename").action(|| rename()),
    menu_item("Duplicate").key("d").action(|| duplicate()),
    menu_separator(),
    menu_role(MenuRole::Copy),
])
```

Submenus nest inside a context menu the same way, `menu_role` items keep their native
behavior, and passing an empty `Vec` removes the menu.

## 3. Put commands in the window toolbar

A toolbar is window chrome, not a piece: it doesn't live in the tree, and Day doesn't lay it
out. Probe for it first, the way the showcase's Toolbars page does, and put the same commands
in the content where there is no bar:

```rust
if capability(Cap::Toolbar) == Support::Native {
    toolbar(vec![
        toolbar_button("refresh", "Refresh").icon(Symbol::Refresh).action(refresh_all),
        toolbar_separator(),
        toolbar_toggle("star", "Star", starred).icon(Symbol::Star),
        toolbar_flexible_space(),
        toolbar_search("search", query).placeholder("Search articles"),
    ]);
}
```

The vocabulary: `toolbar_button(id, label)` for a command, `toolbar_toggle(id, label, signal)`
for a two-state button bound two-way, `toolbar_menu(id, label, entries)` for a pull-down built
from the same `MenuEntry`s the menu bar takes, `toolbar_search(id, signal)` for the platform's
search control, `toolbar_label(id, text)` for static text, and `toolbar_separator()` /
`toolbar_space()` / `toolbar_flexible_space()` for the gaps. Modifiers: `.icon(Symbol)`,
`.image(name)`, `.action(f)`, `.tooltip(t)`, `.placeholder(t)`, `.enabled(bool)`, and
`.enabled_when(f)`.

There is no leading/trailing property — items before the first `toolbar_flexible_space()` pack
to the leading edge and the rest to the trailing edge, and each backend expresses that with
its own layout. `.icon(Symbol::Refresh)` names what the icon means; each backend draws its
platform's own glyph (an SF Symbol on macOS, a freedesktop name on GTK and Qt, a Segoe Fluent
glyph on Windows), which is the only way one icon looks native on four desktops.

Per desktop, the bar is an `NSToolbar` in the unified title-bar style on macOS; on GTK the
items pack into the window's `AdwHeaderBar`, because in GNOME the header bar is the toolbar;
on Qt it is a real `QToolBar` that takes its icon size and style from the user's settings; on
Windows it is a `CommandBar`, whose one limit is that search fields, labels, and fixed spaces
always render on the leading side.

Two rules keep a live bar stable. Use `toolbar_reactive(builder)` when the item list or its
labels derive from state — each pass replaces the bar. Keep the values that change often out
of that builder: a toggle's signal, a search field's signal, and `.enabled_when(…)` patch the
one item in place, so a command greying out never disturbs a search in progress. On mobile,
the counterpart for a single app-wide command is the navigation bar's trailing
`.bar_action(icon, label, action)`; one registered closure can back both.

## 4. Open a secondary window

```rust
let win = day::open_window(
    Some("detail:AAPL"),                  // key: open-or-focus singleton; None = always new
    WindowOptions { title: "AAPL".into(), size: Size::new(720.0, 640.0), ..Default::default() },
    WindowKind::Normal,
    || detail_page("AAPL"),
);
win.on_close(|| println!("gone"));
```

The `key` names the logical window: opening an already-open key focuses it instead of
duplicating, and `day::window_by_key("detail:AAPL")` finds it later. `WindowKind::Normal` is
resizable, miniaturizable, and joins the platform's tabbing group; `WindowKind::Preferences`
drops resize and minimize and never tabs. The window is app-owned — it survives the page that
opened it. Close is asynchronous everywhere: the title-bar button, a platform gesture, and
`WindowHandle::close()` all wait for the platform to confirm, then the content is disposed and
`on_close` runs. Closing the primary window quits the app.

All of this works on every backend. Where the toolkit cannot open windows — iPhone, web, and
the `Preferences` kind on all mobile — the content presents as a fullscreen cover in the
primary window instead: same API, same keys, same close path. That tier has no native title
bar or close button, so probe `Cap::MultiWindow` and give cover-tier content its own close
affordance (the system back button closes it on Android).

## 5. Add the Settings window and File ▸ New Window

Two registrations in your root builder, before `app_menu`, give the app its standard window
conventions:

```rust
day::register_preferences_with(
    WindowOptions { title: "Settings".into(), size: Size::new(520.0, 420.0), ..Default::default() },
    || preferences_page(),
);
day::register_new_window(|| {
    install_toolbar();      // each window gets its own bar (see Pitfalls)
    shell()
});
```

`register_preferences_with` enables the Settings item with zero menu code: on macOS,
"Settings…" with ⌘, in the App menu directly under About; on GTK, Qt, and Windows, a
Preferences item with Ctrl+comma, injected into your first menu if you didn't place a
`menu_role(MenuRole::Preferences)` yourself. The window opens under the singleton key
`day.preferences`, so reopening focuses it, and `day::open_preferences()` opens the same
surface from anywhere — a toolbar gear, say. On the cover tier it presents fullscreen.

`register_new_window` names the builder behind `menu_role(MenuRole::NewWindow)` — File ▸ New
Window with ⌘N/Ctrl+N — and the macOS tab-bar "+". Each call opens an independent `Normal`
window. On macOS, Day also installs the standard Window menu (Minimize, Zoom, Bring All to
Front, plus the open-window list) unless your own menu claims `MenuRole::Minimize`.

## Pitfalls

- **Register windows before the menu.** A `MenuRole::NewWindow` item lowers disabled when no
  builder is registered, and the auto Settings item needs the preferences registration. Call
  `register_preferences_with` and `register_new_window` before installing the app menu — the
  showcase's `root()` does exactly this — so the items lower live.
- **Toolbars install per window.** `toolbar(…)` targets the window being built: the primary
  window at startup, and each new window inside its `register_new_window` builder. A builder
  that skips the install opens a window with no bar.
- **Keep bound values out of `toolbar_reactive`.** A reactive rebuild replaces the whole bar
  and would drop the search field's focus mid-word. Structure and labels go in the builder;
  a toggle's signal, a search signal, and `.enabled_when` patch single items.
- **Don't put the toolkit name in your window title.** Debug builds append a
  `(<version>/<toolkit>)` tag to every title so you can tell windows apart; add your own and
  it appears twice. Release builds never show the tag.

## Reference

- [menus](/docs/internal/menus) — the full role table per backend, how action dispatch works,
  and driving menus from dayscript.
- [toolbars](/docs/internal/toolbars) — the per-backend realization table, patch semantics,
  and the `toolbar:` script step.
- [windows](/docs/internal/windows) — the backend tier table, the pending-open path, the
  debug title tag, and per-window screenshots.
