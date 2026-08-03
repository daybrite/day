# Secondary windows (§8.1)

> **Status: implemented** on every backend. Desktop — AppKit, GTK, Qt native (runtime-
> verified) and XAML native (compile-verified on the windows-xaml CI leg — its runtime pass
> and per-window screenshot capture are follow-ups noted below). Mobile — `Normal` windows
> are NATIVE where the platform has a real secondary-window surface: iPad UIScenes (the
> uikit backend runs the scene lifecycle now), Android document-style `DayWindowActivity`
> instances, and OHOS multiton `DayWindowAbility` instances; iPhone and the `Preferences`
> kind on all mobile present as a fullscreen cover in the primary window (the platform
> settings idiom) — same API, ids, and close path. web answers `Unsupported` (a second
> browser window cannot share the wasm instance) and always takes the cover. Verified by
> mock e2e (`crates/day-pieces/tests/mock_e2e.rs`, the window suite), the showcase
> walkthrough's preferences leg (369/369 on appkit, gtk, qt, ios-sim, android-emulator,
> and the OHOS emulator), the desktop `dayscript/windows.yaml` / `windows-theme.yaml`
> scripts, and second-window emulator captures on Android and OHOS.

One tree, many roots: a secondary window's content container is adopted as an additional
boundary root of the SAME thread-local tree (the `create_cell_anchor` trick the primary root
and list cells already use), laid out at that window's own size. Bindings, `find_by_id`,
navigation state, and dayscript therefore work across windows with no window parameter —
element ids stay globally unique, and a control in one window can drive a label in another
through ordinary signals.

## Opening windows

```rust
let win = day::open_window(
    Some("detail:AAPL"),                  // key: open-or-focus singleton; None = always new
    WindowOptions { title: "AAPL".into(), size: Size::new(720.0, 640.0), ..Default::default() },
    WindowKind::Normal,                   // or WindowKind::Preferences
    || detail_page("AAPL"),               // built under the new window's root
);
win.set_title("AAPL — live");
win.on_close(|| println!("gone"));
win.close();                              // async: confirmed by the platform, THEN torn down
```

- `key` names the LOGICAL window: opening an already-open key focuses it instead of
  duplicating — the preferences pattern. `window_by_key` finds it later.
- `WindowKind` shapes the chrome: `Normal` is resizable/miniaturizable and joins the
  platform's tabbing group; `Preferences` drops resize/minimize and never tabs (macOS
  convention; other platforms map as fits).
- The window's lifetime is app-owned: it survives the page that opened it. Its content
  builds in a fresh scope disposed at close.
- Close is ASYNC everywhere: the title-bar close, a platform gesture, and
  `WindowHandle::close()` all route through the platform's confirm
  (`Event::WindowClosed` on the window's root), and day-core tears the subtree down on a
  deferred hop — never inside the native close callback. `on_close` runs after disposal.
- Closing the PRIMARY window quits the app, taking secondary windows with it (unified
  across the desktop backends; the mac-conventional zero-window state is a recorded
  non-goal for v1).
- Probe `Cap::MultiWindow` to adapt chrome: on `Unsupported` backends the surface is a
  fullscreen cover with no native title bar or close button — content that needs a close
  affordance should carry its own (system back closes it on Android).
- Dialogs (`docs/dialogs.md`) attach to the KEY window at present time, falling back to the
  primary.

## The preferences paradigm

```rust
// root(), once, before app_menu:
day::register_preferences_with(
    WindowOptions { title: tr("day-preferences").format(), size: Size::new(520.0, 640.0), ..Default::default() },
    || my_prefs_page(),
);
// anywhere (menu items get this wired automatically; toolbar gears call it directly):
day::open_preferences();
```

Registering a preferences piece enables, with zero menu code:

- **macOS**: "Settings…" + ⌘, in the App menu, directly under About — in both the default
  menu (apps that never call `app_menu`) and an installed one (the item is hoisted out of
  the model into its standard position).
- **GTK/Qt/XAML**: a Preferences item with the platform accelerator (Ctrl+comma; Qt's
  menu-role relocates it into the app menu on macOS, and macOS-gtk additionally enables the
  stock GTK app menu's *Settings…* through an `app.preferences` action). day-core injects it
  into the first (File) menu when the app didn't place a `menu_role(MenuRole::Preferences)`
  itself.
- The window opens under the singleton key `day.preferences` (`WindowKind::Preferences`);
  reopening focuses. On cover-tier backends `open_preferences` presents the same piece
  fullscreen — mobile apps typically keep their in-nav settings route as the visible entry
  point and gate on `Cap::MultiWindow` (Day-Matrix's `settings::show()` is the pattern).

`pieces/day-piece-settings` supplies the shared theme/language rows most preferences
surfaces need: `appearance_picker(key)` (Light/Dark/System, id `theme-picker`, gated on
`Cap::Appearance`), `language_picker(key, res::locales::ALL)` (System + autonyms, id
`language-picker`), `settings_sections(..)`, and `apply_startup(theme_key, locale_key)` —
which applies persisted overrides at boot with the **env-wins rule**: `DAY_THEME` /
`DAY_LOCALE` launches keep their forced values regardless of persistence (CI variant loops
stay deterministic), while live picker changes always apply.

## New Window + the macOS Window menu

`day::register_new_window(|| shell())` names the builder behind `menu_role(MenuRole::NewWindow)`
(File ▸ New Window, ⌘N/Ctrl+N — lowers disabled when unregistered) and the macOS tab-bar "+"
(`newWindowForTab:`). Each call opens an independent `Normal` window; give each its own nav
signals, and mark secondary shells' routed navs `.local()` so `navigate()` stays unambiguous
(the showcase's `window_root(primary)` is the pattern).

On macOS, day-appkit also auto-installs the standard **Window menu** (Minimize ⌘M, Zoom,
Bring All to Front) registered as `NSApp.windowsMenu`, so AppKit appends the open-window
list and — while automatic tabbing is live — the tab commands (Show Next/Previous Tab,
Merge All Windows). `Normal` Day windows share the `day.normal` tabbing identifier and
group as native tabs per the system "prefer tabs" setting. When no new-window builder is
registered, automatic tabbing is turned off entirely (no tab bar, no dead menu items). An
app that places `MenuRole::Minimize` in its own model owns window management and skips the
auto menu.

## The debug title tag

A **debug** build appends `(<version>/<toolkit>[/<script>])` to every window title it sets —
the primary window's, each secondary window's, and every `WindowHandle::set_title`:

```
Day Showcase (1.1.0/gtk/walkthrough.yaml)
Day Sheets (0.1.0/appkit)
```

With several apps, several toolkits and a scripted run open at once, the title bar is the only
place that says which window is which. Release builds get none of it: `day_core::debug_title_tag`
returns `None` outside `debug_assertions`, so the decoration can never ship.

The version and script name arrive as `DAY_APP_VERSION` and `DAY_SCRIPT`, which `day launch` sets
from the project manifest and the `--script` arguments (docs/environment.md). Run the binary
another way and the tag carries only what it knows — `(gtk)`. Apps do nothing: **do not** put the
toolkit in your own title, or it will be there twice.

Two rules the join follows. An **empty** title stays empty, so a window the app deliberately left
untitled does not grow a bar of build metadata. And an already-tagged title is left alone, since
the same window can be retitled many times.

The tag is on the window title only. The macOS App menu, the About panel and the process name
read the app's *name*, so `launch_with` pins `WindowOptions::app_name` to the undecorated title
before tagging — an app that sets only `title` still shows "Day Sheets" in its App menu.

## dayscript

```yaml
- menu: { key: day-preferences }                       # invoke a menu action (menus.md)
- wait_for: { id: prefs-title }                        # ids are tree-global — no window scoping
- screenshot: { name: prefs, window: day.preferences } # capture a window by its open key
- close_window: { window: day.preferences }            # async confirm → teardown, like the title bar
```

`screenshot.window` resolves the key through the registry; on the cover tier it captures the
primary window, whose fullscreen cover IS the content — same pixels, no special case. (XAML
currently also answers the primary for per-window captures — noted follow-up.)

## The seam (backends)

`Toolkit::open_window(id, options, kind) -> WindowOpenReply<Handle>` creates and shows the
native window, wires ITS events to `id` (`WindowResized` in content points, `WindowClosed`
after the platform committed the close, `WindowFocused` on key changes), and answers the
CONTENT container handle — the same contract as `ready`'s root. Backends whose window
creation is asynchronous (a scene, an activity, an ability) answer `Pending` and complete
later through `day_core::finish_window_open(id, raw, size)` — the type-erased `RawHandle`
adoption seam list cells use; day-core parks the record (build deferred) and a close before
completion cancels it (`finish_window_open` answers `false`; the backend drops its window).
`close_window`/`focus_window`/`set_window_title`/`snapshot_window_of` round out the duties;
day-core releases the content handle after teardown, which is each backend's signal to
destroy the native window (Qt/XAML defer destruction to exactly this point so child-widget
releases stay sound).

| Backend | Tier | Mechanism |
|---|---|---|
| AppKit | Native | per-window `NSWindow` + delegate (retained; windows are not released-when-closed); tabbing groups; Window menu |
| GTK | Native | additional `AdwApplicationWindow`s on the shared `GtkApplication`; app-level active-state debounced across windows |
| Qt | Native | shim `DayWindow` carrying its node id; explicit quit policy (`quitOnLastWindowClosed(false)`) |
| XAML | Native (CI-verified) | second Win32 host + its own `DesktopWindowXamlSource` island; accelerators in secondary islands are a noted v1 limit |
| iOS | Native on iPad (`UIScene` request/connect; the whole backend runs the scene lifecycle) | iPhone answers Unsupported → cover; iPad runtime check pends a `day launch` device flag |
| Android | Native | document-style `DayWindowActivity` per window (own recents entry; split-screen/freeform); `Preferences` → cover |
| HarmonyOS | Native (when the ArkTS host registers the launchers) | multiton `DayWindowAbility` per window; `Preferences` → cover. Pre-existing backend quirk: presented covers pass asserts and receive taps but device captures show the page beneath (affects the cover piece identically — follow-up) |
| web | Unsupported → cover fallback | a second browser window cannot share the wasm instance |
| mock | Native | recorded windows + synthesized confirms — the seam-freezing e2e suite |
