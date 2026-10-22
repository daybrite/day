---
title: "Secondary windows"
description: "Real secondary windows on desktop, covers on mobile: open_window, the preferences singleton, and the Window menu."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Secondary windows (§8.1)

> **Status:** the secondary-window API is implemented. The source chooses native windows where
> supported and a fullscreen cover otherwise.
> iPad, Android, and OpenHarmony normal-window paths differ from preferences covers; query the
> capability at runtime and test the target you ship.

## Signature and defaults

API excerpt, not a complete app:

```rust
pub fn open_window<P: Piece>(
    options: WindowOptions,
    build: impl FnOnce() -> P + 'static,
) -> WindowHandle;
```

Call on the UI thread after the tree is installed. The return type is a handle, not a
`Result`; this is not a promise that an OS surface appears synchronously. A logical key reuses
an existing window and does not call the new builder. Without a key, the call opens new content.
The window is app-owned, so it can outlive the page that opened it; captured page-local signal
handles do not acquire a longer lifetime. Create shared state in a suitable parent scope.

| `WindowOptions::default()` field | Default |
| --- | --- |
| `size` | 480 × 640 logical points |
| `min_size`, `max_size` | `None` |
| `key`, `remember_frame` | `None` |
| `kind` | `WindowKind::Normal` |
| `start_hidden`, `size_to_fit` | false |
| `shadow`, `resizable` | true |
| `chrome`, `background`, `placement` | Standard, Opaque, Automatic |
| `tabbing` | Automatic |
| `locales`, `title_fn`, `app_name`, `version` | `None` |

The raw default title is `Day`; app examples must use generated `res::window_options()` so
catalog, title, version, and manifest window dimensions are installed. `.title(value)` clears
an earlier `title_fn`. `start_hidden` is currently AppKit-only. A raw option's presence does
not establish native support everywhere. Detailed contracts below are integration fragments.

Evidence: [window options](../crates/day-spec/src/lib.rs) and
[window ownership and handles](../crates/day-core/src/windows.rs).

One tree has many roots: a secondary window's content container is adopted as an additional
boundary root of the same thread-local tree (the `create_cell_anchor` trick the primary root
and list cells already use), laid out at that window's own size. Bindings, `find_by_id`,
navigation state, and dayscript therefore work across windows with no window parameter;
unique element ids remain reachable globally, and a control in one window can drive a label
in another through ordinary signals. When a reusable form mounts the same ids in multiple
windows, lookup prefers the focused window (including a cover-backed window). Recycled
list and tree rows follow their owning view's window. A dismissing cover no longer takes
precedence over the primary window. Keep ids unique within each window.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](https://daybrite.dev/docs/how-to/localization).

## Titles: what the platform manages windows by

Every platform's automatic window management identifies a window by its **title**, and supplies
no fallback when there isn't one. On macOS an untitled window is skipped when AppKit builds the
Window menu and shows a blank tab in a tab group; on iPad it is an unlabeled card in the app
switcher; on Android an unlabeled recents entry; on GTK/Qt/Windows an unlabeled entry in the
window list or taskbar. So a window with no title is missing from every place the system lists
windows.

These rules keep that from happening:

- A window opened by File ▸ New Window **inherits the app's launch `WindowOptions`** (the title,
  minimum size, and display name the app handed `launch`). An app needs no code for this; a new
  window is another window of the same app and describes itself that way.
- `day::window_title(|| …)` binds the title of the window the calling piece is building into, so
  a window names itself after what it shows. It is reactive like any binding, and window-scoped:
  the target is resolved once, at build, exactly as a toolbar contribution resolves its own.

```rust
fn window_shell() -> impl Piece {
    Scene::scoped(|scene| {
        day::window_title(move || match scene.selected.get() {
            Some(id) => scene.name_of(id),
            None => app_title(),
        });
        my_ui(scene)
    })
}
```

Two windows sharing one title are two windows the user cannot tell apart in the Window menu, the
tab bar, Mission Control, or the app switcher, so title them by content wherever there is content
to name. `WindowHandle::set_title` remains the imperative form for a window you hold a handle to.

**The platform places windows.** macOS staggers each new window from the last
(`cascadeTopLeftFromPoint:`) rather than centering it, since two centered windows would hide each
other, and remembers the primary window's frame between launches under an autosave name. That restore is
turned off while `DAY_SCRIPT` or `DAY_WINDOW` is set, so captured screenshots keep the size the
script asked for instead of the size the developer last dragged. Every other desktop leaves
placement to its window manager, which is that platform's convention.

`WindowKind::Preferences` is centered and kept out of the macOS Window menu, the convention stock
apps follow.

## Opening windows

```rust
let win = day::open_window(
    res::window_options()                 // the app's catalog, title and version, generated
        .title("AAPL")
        .size(720.0, 640.0)
        .key("detail:AAPL"),              // open-or-focus singleton; omit for always new
                                          // .kind(WindowKind::Preferences) for a panel
    || detail_page("AAPL"),               // built under the new window's root
);
win.set_title(res::str::live_quote_title("AAPL").format());
win.on_close(|| println!("gone"));
win.close();                              // async: confirmed by the platform, THEN torn down
```

`WindowOptions` is built from `res::window_options()` (generated by `day::resources!`: the
locale catalog, the localized `app_title`, the size and minimum size Day.toml `[window]`
declares, and the version from Cargo.toml; it is what `day::launch` and `day_start!` take for
the main window) or
`WindowOptions::default()`, with builders for everything else: `.size(w, h)`, `.min_size(w, h)`,
`.max_size(w, h)`, `.title(..)`, `.app_name(..)`, `.start_hidden()`, `.size_to_fit()`,
`.fixed_size()`, `.no_shadow()`, `.tabbing(..)`, `.chrome(..)`, `.background(..)`,
`.placement(..)`, `.remember_frame(key)`, `.key(..)`, `.kind(..)`.

- `.key(..)` names the logical window: opening an already-open key focuses it instead of
  duplicating, which is how `day.preferences` stays a singleton. `window_by_key` finds it later.
  A key reopened while its window is still *closing* is the same window arriving again: on the
  cover tier that reverses the hide transition and keeps the content that is already there,
  rather than handing back a surface the pending confirmation is about to dispose.
- `.kind(WindowKind)` shapes the chrome: `Normal` (the default) is resizable/miniaturizable
  and joins the platform's tabbing group; `Preferences` drops resize/minimize and never tabs
  (macOS convention; other platforms map as fits).
- The window's lifetime is app-owned: it survives the page that opened it. Its content
  builds in a fresh scope disposed at close.
- Close is async everywhere: the title-bar close, a platform gesture, and
  `WindowHandle::close()` all route through the platform's confirm
  (`Event::WindowClosed` on the window's root), and day-core tears the subtree down on a
  deferred hop, never inside the native close callback. `on_close` runs after disposal.
  A programmatic close immediately excludes the window from focused-window state and command
  lookup, even while a native dialog or cover is still animating its dismissal.
- Closing the last primary window quits the app, taking secondary windows with it; a settings
  panel does not keep an app alive, however long it has been up. **macOS is the exception**:
  `applicationShouldTerminateAfterLastWindowClosed` defaults to false there, an app
  with no windows keeps its menu bar live, and ⌘N reopens one. So on macOS the app stays up and
  its secondary windows stay with it; every other desktop treats the last primary as the app.
  A window's role comes from its `WindowKind` (`Preferences` ⇒ secondary).
- Probe `Cap::MultiWindow` to adapt chrome: on `Unsupported` backends the surface is a
  fullscreen cover with no native title bar or close button; content that needs a close
  affordance should carry its own (system back closes it on Android).
- Dialogs ([`docs/dialogs.md`](dialogs.md)) attach to the key window at present time, falling back to the
  primary.

## The preferences paradigm

```rust
// root(), once, before app_menu:
day::register_preferences_with(
    res::window_options().title(res::str::day_preferences().format()).size(520.0, 640.0),
    || my_prefs_page(),
);
// anywhere (menu items get this wired automatically; toolbar gears call it directly):
day::open_preferences();
```

Registering a preferences piece enables the following, without any menu code in the app:

- **macOS**: "Settings…" + ⌘, in the App menu, directly under About, in both the default
  menu (apps that never call `app_menu`) and an installed one (the item is hoisted out of
  the model into its standard position).
- **GTK/Qt/XAML**: a Preferences item with the platform accelerator (Ctrl+comma; Qt's
  menu-role relocates it into the app menu on macOS, and macOS-gtk additionally enables the
  stock GTK app menu's *Settings…* through an `app.preferences` action). day-core injects it
  into the first (File) menu when the app didn't place a `menu_role(MenuRole::Preferences)`
  itself.
- The window opens under the singleton key `day.preferences` (`WindowKind::Preferences`);
  reopening focuses. On cover-tier backends `open_preferences` presents the same piece
  fullscreen. The default title is the platform's word (`day_settings`).

Apps rarely call these directly: `day-piece-settings` ([docs/settings.md](settings.md))
declares the settings and About once and registers both windows, the menu items and the
mobile navigation presentation itself.

## About

```rust
day::register_about_window(
    res::window_options().title(res::str::about_app().format()),
    || about_page(),
);
day::register_about(|| show_native_about_dialog()); // or: any action
day::open_about();
```

`register_about` names what the About command does; `register_about_window` is the common
case, a Day window under the singleton key `day.about` as `WindowKind::About` (the same fixed,
centered, untabbed panel chrome as Preferences, a dialog over the app window on GTK, a cover
on mobile). With an About registered,
`MenuRole::About` items lower to it: the macOS App menu's "About <App>" (replacing the stock
Info.plist panel), Help ▸ About on the other desktops, in both the default bar and an installed
`app_menu`. `day::register_check_for_updates(action)` adds the updater's Check for Updates…
command the same way (`MenuRole::CheckForUpdates`, under About on macOS, in Help elsewhere).

## New Window + the macOS Window menu

`day::register_new_window(|| shell())` names the builder behind `menu_role(MenuRole::NewWindow)`
(File ▸ New Window, ⌘N/Ctrl+N; lowers disabled when unregistered) and the macOS tab-bar "+"
(`newWindowForTab:`). Each call opens an independent `Normal` window, and mark secondary shells'
routed navs `.local()` so `navigate()` stays unambiguous (the showcase's `window_root(primary)`
is the pattern). The starter uses one main window.

Because the builder runs again per window, whatever state that shell reaches for decides whether
the windows are independent; a `thread_local!` gives all of them one selection.
[docs/state.md](state.md) covers this: `T::scoped(…)` for per-window state, `T::ambient()`
to read it back anywhere below, and `T::focused()` for the app-wide menu bar, whose items belong
to no window and have to resolve the front one when they run. The same shell should call
`day::window_title` (above), so the windows it builds are distinguishable everywhere the system
lists them.

> [!NOTE]
> A live retitle reaches AppKit, GTK, Qt, XAML, UIKit and Android, primary window included.
> `day-arkui` takes the trait's default, so an OpenHarmony window keeps the title it was opened
> with; the inherited launch title makes that correct rather than blank, but it does not follow
> content yet.

On macOS, day-appkit also auto-installs the standard **Window menu** (Minimize ⌘M, Zoom,
Bring All to Front) registered as `NSApp.windowsMenu`, so AppKit appends the open-window
list and, while automatic tabbing is live, the tab commands (Show Next/Previous Tab,
Merge All Windows). `Normal` Day windows share the `day.normal` tabbing identifier and
group as native tabs per the system "prefer tabs" setting. When no new-window builder is
registered, automatic tabbing is turned off entirely, so there is no tab bar and the menu carries no tab
commands. An
app that places `MenuRole::Minimize` in its own model owns window management and skips the
auto menu.

## Window properties

Every window, the first one included, has a `WindowHandle`: `day::initial_window()`,
`day::current_window()` (the window being built, else the focused one, else the first),
`open_window`'s return value, or `day::window_by_key`.

### Display state

`state()` is the window's display state as a two-way signal:

```rust
use day::prelude::*;

fn fullscreen_button() -> impl Piece {
    let state = day::current_window()
        .map(|w| w.state())
        .unwrap_or_else(|| Signal::new(day::WindowState::Normal));
    button(move || {
        if state.get() == day::WindowState::Fullscreen {
            "Exit Full Screen"
        } else {
            "Enter Full Screen"
        }
    })
    .action(move || {
        state.set(if state.get() == day::WindowState::Fullscreen {
            day::WindowState::Normal
        } else {
            day::WindowState::Fullscreen
        })
    })
}
```

Reading it follows what the user does: minimize, zoom, enter or leave fullscreen, from the title
bar, the keyboard or a gesture. Writing it asks the platform for that state. The platform has the
last word: the signal settles on the state the window actually reached, so a request the platform
declines (a browser refusing fullscreen outside a click) reads back as the state the window kept.
`set_state(s)` is the same write spelled as an action. Wayland compositors do not tell an app
when the user minimizes it, so under Wayland GTK and Qt report `Minimized` only for a minimize the
app asked for, until the window is next activated.

| State | Desktops | iOS | Android | HarmonyOS | Web |
|---|---|---|---|---|---|
| `Minimized`, `Maximized` (`Cap::WindowStates`) | Native | Unsupported | Unsupported | Unsupported | Unsupported |
| `Fullscreen` (`Cap::WindowFullscreen`) | Native | Emulated: status bar hidden, home indicator auto-hidden | Native: system bars hidden, shown again by a swipe | Native | Native during a click or key press |

### Content protection

`set_content_protected(true)` keeps a window out of screenshots, screen recordings and screen
sharing, for passwords, payment details and private messages (`Cap::ContentProtection`). macOS
(`NSWindowSharingNone`), Windows (`WDA_EXCLUDEFROMCAPTURE`), Android (`FLAG_SECURE`) and
HarmonyOS (privacy mode) protect natively. HarmonyOS needs the app to opt in with
`screen-privacy = true` in Day.toml's `[permissions]` ([docs/permissions.md](permissions.md));
without it `Cap::ContentProtection` answers `Unsupported` there. iOS has no switch for it: Day
covers the window while the screen is being recorded or mirrored, and a screenshot still
captures it. Linux and the web cannot protect a window.

### Size, position and stacking

| Method | What it does | Cap |
|---|---|---|
| `set_frame(origin, size)` | Move the frame's top-left corner to `origin` (desktop points, top-left origin) and/or resize the content | `WindowPosition`, `WindowGeometry` |
| `frame()` | The window's outer frame on the desktop, where the platform says | `WindowGeometry` |
| `set_limits(min, max)` | The sizes the user can resize between | `WindowGeometry` |
| `set_resizable(bool)` | Whether the user can resize at all | `WindowGeometry` |
| `set_level(WindowLevel::Floating)` | Stay above other apps' windows | `WindowLevel` |
| `set_on_all_workspaces(bool)` | Show on every Space or virtual desktop | `WindowLevel` |
| `set_skip_taskbar(bool)` | Leave the window out of the taskbar and window switcher (the Window menu on macOS) | `WindowLevel` |
| `set_minimizable`, `set_maximizable`, `set_closable` | Offer or withhold the title-bar controls | `WindowStates` |
| `set_visible(bool)` | Hide the window without closing it, or show it again | `WindowStates` |
| `set_appearance(Option<bool>)` | This window's own light or dark appearance, over the app-wide `set_appearance` | `WindowAppearance` |
| `request_attention(Attention)` | A Dock bounce, a flashing taskbar button | `RequestAttention` |

`WindowOptions` sets the same things at open: `min_size`, `max_size`, `resizable`, `placement`,
and the frame options in [docs/window-chrome.md](window-chrome.md).

| Backend | What it supports |
|---|---|
| AppKit | Everything above. Taskbar exclusion is the Window menu's, since macOS has no taskbar. |
| Windows XAML | Everything except showing a window on every virtual desktop. Compile-checked; the Windows runtime pass is pending. |
| Qt | Everything except per-window appearance and every-desktop windows. Under Wayland the compositor places windows, so `set_frame`'s origin and `WindowPosition` are unsupported there. |
| GTK | Size, minimum size, resizable, closable, visibility and the minimize/maximize controls. GTK 4 removed window positioning, stacking, the taskbar hint, the urgency hint and maximum sizes, and `frame()` answers `None`. On Linux the Qt backend covers those. |
| iOS | Per-window appearance (`overrideUserInterfaceStyle`). |
| Android, HarmonyOS, web | None: the platform sizes and places the app's window. The web follows the app-wide appearance per page. |

`set_frame`'s size is the size Day lays the content out at, the size `WindowResized` reports;
the backend adds the title bar, the frame and any menu bar around it. `frame()` reports the
outer frame, chrome included.

GTK reports the content wrapper's allocated size, excluding the header and client-side
decoration borders. This keeps Day layout and content snapshots in the same coordinate
system on X11 (including headless Xvfb), Wayland and other GTK window systems.

### Monitors

`day::monitors()` lists the displays: an id, a name, the frame, the work area (the frame minus
the menu bar, Dock, taskbar or panels), the scale, and which one is primary, all in desktop
points (`Cap::Monitors`). A phone or the web reports its one screen. GTK 4 exposes no work area or
primary display, so there the work area is the whole display and the first one is primary.

### Remembered frames

```rust
res::window_options().remember_frame("library")
```

A window with `.remember_frame(key)` reopens at the size, position and maximized state it had when it
last closed. The frame is saved when the window closes and when the app quits, in
`day-windows.txt` under the app's configuration directory (`$XDG_CONFIG_HOME/<app id>/` when that
is set, else `~/.config`, `~/Library/Application Support` or `%APPDATA%`). A saved position that
no attached display shows any more is dropped, so a window never reopens off-screen; the size is
kept. A minimized or fullscreen window keeps the last ordinary frame it had.

## Window tabs on macOS

Ordinary windows group as native macOS tabs per the user's "Prefer tabs" setting. `WindowOptions::
tabbing` puts a window in a tab group of its own:

| `WindowTabbing` | Effect |
|---|---|
| `Automatic` | The default: the window tabs with the app's other ordinary windows; preferences windows never do. |
| `Group(id)` | Tab only with windows of the same group, following the user's setting. |
| `Preferred(id)` | Always open as a tab of a window in the group. |
| `Disallowed` | Never become a tab. |

`register_new_window_for(group, build)` is the builder behind the tab bar's "+" in that group's
windows. A group without one falls back to the app's `register_new_window` builder. AppKit and
Qt on macOS (through `toolkits/day-macos-tabs`) answer `Cap::WindowTabbing`; every other
platform has no system window tabs and ignores the option.

## Keeping the app running

A status item keeps an app running after its last window closes, and `day::set_keep_running`
chooses that for any app: see [docs/status-item.md](status-item.md#keeping-the-app-running).

## The debug title tag

A **debug** build appends `(<version>/<toolkit>[/<script>])` to every window title it sets
(the primary window's, each secondary window's, and every `WindowHandle::set_title`):

```text
Day Showcase (1.1.0/gtk/walkthrough.yaml)
Day News (0.1.0/appkit)
```

With several apps, several toolkits and a scripted run open at once, the title bar is the only
place that says which window is which. Release builds get none of it: `day_core::debug_title_tag`
returns `None` outside `debug_assertions`, so the decoration can never ship.

The version and script name arrive as `DAY_APP_VERSION` and `DAY_SCRIPT`, which `day launch` sets
from the project manifest and the `--script` arguments ([docs/environment.md](environment.md)). Run the binary
another way and the tag carries only what it knows: `(gtk)`. Apps do nothing: **do not** put the
toolkit in your own title, or it will be there twice.

The join follows two rules. An **empty** title stays empty, so a window the app left
untitled does not grow a bar of build metadata. And an already-tagged title is left alone, since
the same window can be retitled many times.

The tag is on the window title only. The macOS App menu, the About panel and the process name
read the app's *name*, so `launch_with` pins `WindowOptions::app_name` to the undecorated title
before tagging; an app that sets only `title` still shows "Day News" in its App menu.

## dayscript

```yaml
- menu: { key: day_preferences }                       # invoke a menu action (menus.md)
- wait_for: { id: prefs_title }                        # global lookup; focused window wins duplicates
- screenshot: { name: prefs, window: day.preferences } # capture a window by its open key
- close_window: { window: day.preferences }            # async confirm → teardown, like the title bar
- close_window: {}                                     # the initial window: the app ends (close policy above; GTK today)
- expect_exit: { within: 20 }                          # …so the run waits for that instead of stopping the app
```

`screenshot.window` resolves the key through the registry; on the cover tier it captures the
primary window, whose fullscreen cover is the content, so the same pixels come back with no
special case. (XAML
currently also answers the primary for per-window captures, a noted follow-up.)

## The Toolkit duties (backends)

`Toolkit::open_window(id, options, kind) -> WindowOpenReply<Handle>` creates and shows the
native window, wires its events to `id` (`WindowResized` in content points, `WindowClosed`
after the platform committed the close, `WindowFocused` on key changes), and answers the
content container handle, the same contract as `ready`'s root.

**`WindowResized` must carry `id`, not the primary's node.** It is what re-buckets that window's
size class ([docs/size-classes.md](size-classes.md)), so a backend that reports every window's
geometry against the primary re-lays-out the wrong window, which is what day-uikit's holder view
did, invisibly, until iPadOS made two windows at two sizes an ordinary thing. day-core relayouts
*and* re-buckets on receipt, so a secondary window dragged from narrow to wide re-presents its
navigation exactly as the primary does. Backends whose window
creation is asynchronous (a scene, an activity, an ability) answer `Pending` and complete
later through `day_core::finish_window_open(id, raw, size)`, the type-erased `RawHandle`
adoption path list cells use; day-core parks the record (build deferred) and a close before
completion cancels it (`finish_window_open` answers `false`; the backend drops its window).
**A window the platform opens by itself takes the same path in reverse.** iPadOS 26 offers
Window ▸ New Window for any app that supports multiple scenes, an app icon dragged into Split
View connects a scene, and a relaunch reconnects the sessions from the previous run — none of
them carry the node id a `Toolkit::open_window` request does. day-uikit builds the window and
then calls `day_core::open_new_window()`, so the content is whatever the app registered with
[`register_new_window`](#new-window--the-macos-window-menu), the same builder behind File ▸ New Window on
desktop; an app that registered none has nothing to put in the window, and the backend hands the
scene back. Until 2026-09 those scenes were destroyed on sight, which is what made the system's
New Window flash a window and lose it.

`apply_window(host, change)` carries every window property above, one `WindowChange` at a time,
for any window including the first (its root container is its host). A state change the platform
completes is reported back as `Event::WindowStateChanged`; a state the backend cannot reach is
reported as the state the window is in, so the signal settles. `window_frame(host)` and
`monitors()` answer queries, and `set_keep_running` tells a desktop backend to report its first
window's close like any other instead of ending the app itself.

`close_window`/`focus_window`/`set_window_title`/`snapshot_window_of` round out the duties;
day-core releases the content handle after teardown, which is each backend's signal to
destroy the native window (Qt/XAML defer destruction to exactly this point so child-widget
releases stay sound).

| Backend | Tier | Mechanism |
|---|---|---|
| AppKit | Native | per-window `NSWindow` + delegate (retained; windows are not released-when-closed); tabbing groups; Window menu |
| GTK | Native | additional `AdwApplicationWindow`s on the shared `GtkApplication`; app-level active-state debounced across windows. The panel kinds are dialogs, not windows: `Preferences` presents as an `AdwPreferencesDialog` and `About` as an `AdwDialog` with a header bar, over the active app window (a floating sheet in a wide window, a bottom sheet in a narrow one, Escape closes). The record keeps the window contract — `close_window` is `adw_dialog_close` and the dialog's `closed` signal is the `WindowClosed` confirm, `focus_window` presents it again, `set_window_title` is its title, `fit_window` is its content width and height, `snapshot_window_of` renders its sheet, and `apply_window` is ignored. A panel opened before any window exists is a window as before. On macOS the panel is always a window of its own (the same `AdwPreferencesDialog`, presented without a parent): that is where a Mac keeps Preferences, and a piece's Cocoa view in the GTK window's content view (day-piece-webview's `WKWebView`) draws above everything GTK draws there, so an in-window sheet could be covered whenever such a view is up. Alert dialogs stay in their window unless such a view would cover them, in which case they too present as windows. |
| Qt | Native | shim `DayWindow` carrying its node id; explicit quit policy (`quitOnLastWindowClosed(false)`) |
| XAML | Native (CI-verified) | second Win32 host + its own `DesktopWindowXamlSource` island; accelerators in secondary islands are a noted v1 limit |
| iOS | Native on iPad (`UIScene` request/connect; the whole backend runs the scene lifecycle, and a scene the system opens on its own is filled from `register_new_window`) | iPhone answers Unsupported → cover; iPad runtime check pends a `day launch` device flag |
| Android | Native | document-style `DayWindowActivity` per window (own recents entry; split-screen/freeform); `Preferences` → cover |
| HarmonyOS | Native (when the ArkTS host registers the launchers) | multiton `DayWindowAbility` per window; `Preferences` → cover. Pre-existing backend quirk: presented covers pass asserts and receive taps but device captures show the page beneath (affects the cover piece identically — follow-up) |
| web | Unsupported → cover fallback | a second browser window cannot share the wasm instance |
| mock | Native | recorded windows + synthesized confirms; the e2e suite that pins the window duties |

## Document-window tabs

`document_tabs` can move resident documents into OS window groups on AppKit and macOS Qt.
`Toolkit::group_windows` explicitly groups and orders the supplied window handles, independently
of the user's automatic-tabbing preference. `window_tab_order(host)` returns known root `NodeId`s
in native group order, or an empty vector where unsupported. GTK uses AdwTabView on every platform;
it does not rewrite GDK's NSWindow style mask or hide its title bar to create OS tabs.

Each presentation window is registered with day-core for layout, focus, size, and close events.
`TreeOps::reparent` moves the existing content subtree without disposing its scope or handles;
it rejects dead nodes, cycles, and window roots. `WindowHandle::set_content_scope` directs focused
ambient and command lookup to the retained document. A mutable `DocumentWindow` context moves
its toolbar contributions with it, including nested-navigation visibility gates.

AppKit initializes each secondary window's content coordinates below its title/toolbar before
mounting the document. Grouping windows updates that offset for the native tab strip and reports
the resulting usable size to Day, even when AppKit sends no resize notification. Overlay windows
retain their full content coordinates and report the chrome inset as safe area instead. The
`native_document_layout` test checks initial and grouped window geometry.

Native activation selects the document without echoing a focus request to the toolkit. Order is
read on focus, close request, and completed close. Each detached group's order is merged into its
existing positions in `TabSet`; observation does not merge detached groups back together. There
is no polling timer or frame-clock subscription. A native reorder that emits no focus/close event
is reflected at the next such event.

`WindowHandle::on_close_request` defers a native close to the document owner. The owner can keep
the key while asking about unsaved edits; accepted removal calls `TabSet::close`. Programmatic
`WindowHandle::close()` bypasses the request hook. Switching to composed presentation first moves
content home, then closes the empty presentation windows, so closure cannot dispose the documents.
The initial host becomes visible again when the collection is empty or composed mode is selected.
A dedicated browser/document shell can opt into `.hide_when_empty(true)`: its host stays hidden
with no documents in either presentation, and becomes visible again when a document opens in
composed mode. The host scope remains alive; the app supplies New Window/New Tab commands rather
than inserting a replacement document on close. This option currently controls OS-window-tab
hosts (AppKit and macOS Qt); other toolkits retain their ordinary in-window behavior.

Focused ambient lookup is reactive: window activation, content-scope assignment and window
removal invalidate menu builders that read `focused_environment`/`Ambient::focused`, so enabled
states and captured actions follow the front document even when its selected tab is unchanged.
Explicit `WindowHandle::focus()` also updates command routing immediately: an inactive AppKit
application may accept activation without emitting a key-window notification. Native focus
notifications still reconcile the registry when the user changes windows.

Qt's macOS bridge is in `toolkits/day-macos-tabs`. Its delegate proxy forwards toolkit-owned
methods, retains the original delegate, and restores it at unregister. When necessary it supplies
an NSWindowController for the native add-tab action. day-core stays platform-independent.

This API does not change Preferences presentation: desktop Preferences use a separate singleton
window, while mobile Preferences remain an in-app cover, including on iPad. See the
[navigation contract](navigation.md#document-tabs) for keys, close handlers, and regression tests.
