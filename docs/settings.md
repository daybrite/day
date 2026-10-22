---
title: "Settings and About"
description: "day-piece-settings: typed persisted prefs, a semantic declaration of groups and rows, and the Settings and About surfaces every toolkit presents its own way."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Settings and About (`day-piece-settings`)

> **Status: implemented (2026-10).** `Pref<T>` + `Store`, `Preferences`/`Group`/`Section`/rows,
> `About`, providers, `Preferences::install`, the composed presentation on every backend, and
> native rows and panels behind the per-toolkit features. The starter template and the sample
> apps use it.

Every Day app needs a settings surface, a way to reach it that matches the platform, and an
About. `day-piece-settings` provides all three from one declaration:

```rust
use day_piece_settings::prelude::*;

// root(), after the locale catalog installs and before app_menu:
let prefs = Preferences::new()
    .about(About::from_manifest().website("https://daybrite.dev"))
    .group(general_group(res::locales::ALL)) // Appearance + Language
    .group(
        Group::new("weather", res::str::settings_weather())
            .icon(res::vectors::prefs_weather)
            .row(rows::choice(
                &UNIT,
                res::str::settings_unit(),
                [
                    (Unit::Celsius, res::str::unit_c()),
                    (Unit::Fahrenheit, res::str::unit_f()),
                ],
            ))
            .row(
                rows::text(&HOST, res::str::settings_host())
                    .placeholder(res::str::api_example_org()),
            ),
    )
    .install();
```

`install()` registers the Preferences window and its Settings…/Preferences menu item on the
desktops, the About window and its About menu item, the `settings/<group>` string routes, and
collects the groups other crates contribute. On the platforms without a menu bar the app puts
the surface in its navigation:

```rust
nav(section)
    .items((
        item(Section::Welcome, res::str::nav_welcome()).page(welcome_page),
        prefs.nav_item(Section::Settings).map(|i| i.icon(res::vectors::tab_settings)),
    ))
```

`prefs.nav_item(key)` is an `Option<NavItem>` carrying the settings page: `Some` exactly when
the platform gives no app menu (the phones, `Cap::AppMenu` unsupported), `None` elsewhere, and
an `Option` is valid nav content, so the same source compiles everywhere and the section appears
only where it should. `.map(|i| i.icon(..))` decorates it like any other item. `day_piece_settings::open()` opens the window on desktop and selects the
section on a phone (a welcome page's "Settings" link).

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](https://daybrite.dev/docs/localization).

## Values: `Pref<T>`

A setting is a static, used from anywhere:

```rust
pub static SNAP: Pref<bool> = Pref::new("sketch.snap", || true);
pub static UNIT: Pref<Unit> = Pref::new("skies.unit", || Unit::Celsius);

pref_enum! {
    /// Stored as "c" / "f".
    pub enum Unit { Celsius = "c", Fahrenheit = "f" }
}

let temp = label(move || format_temp(UNIT.get()));   // tracked read
UNIT.set(Unit::Fahrenheit);                          // signal + store
UNIT.watch(|u| reload(*u));                          // later changes, in the current scope
let sig: Signal<Unit> = UNIT.signal();               // for a control or a binding
SNAP.reset();                                        // default again; the key leaves the store
```

| | |
|---|---|
| `Pref::new(key, default)` | `const fn`; `default` is a function so the static needs no `Sync` value. The signal is created on the UI thread on first use, seeded from the store, and lives for the process. `Pref` is `Copy`. |
| `.env("DAY_THEME")` | The env-wins rule: while the variable is set the stored value yields to the default for this run, so `day launch --env` runs and CI variant loops stay deterministic. Writes still persist. |
| `PrefValue` | `encode`/`decode` through one string. Implemented for `bool`, `i32`, `i64`, `u32`, `u64`, `f64`, `String`; `pref_enum!` implements it for an enum under short stable tags and gives it `ALL`. |
| `persistence_failures()` | A `Signal<Vec<String>>` of the keys whose last store write the store refused (also logged). The in-memory value stays what the user chose for this run and will not survive a relaunch, which is what an app watching the signal can say. A key leaves the list when its own next write lands. `reset` removes through the same checked path, so a refused removal is reported even when the value already sat at its default. |
| `get`, `get_untracked`, `set`, `reset`, `is_default`, `watch`, `signal`, `key` | The runtime API. A pref is a signal, so reading one inside another signal's `update` closure re-enters the reactive runtime and panics ("RefCell already borrowed"): read it before the `update`. Every write to the signal persists, whichever path wrote it (`set`, a control bound to `signal()`, the app). The store holds only non-default values: a value equal to the default removes the key, so `reset` and "set to the default" agree, and a changed default on upgrade reaches every user who never chose otherwise. |

### Where values live

Values go through a `Store`:

```rust
pub trait Store: 'static {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str) -> bool;
    fn remove(&self, key: &str) -> bool;
}
```

The default is `NativeStore`, which is [day-part-prefs](prefs.md): `NSUserDefaults` on macOS
and iOS, `SharedPreferences` on Android, a per-app file under the user's config directory on
Linux, Windows and HarmonyOS, `localStorage` on the web. `install_store(..)` before
`Preferences::install` replaces it (a `MemoryStore` for tests and deterministic script runs, or
the app's own database). There is no second storage layer and no sync: the platform store
already is the right place, and a value that is one JSON document stays one key.

## The declaration

| | |
|---|---|
| `Group::new(id, title)` | `id` is the route segment and the element id (`prefs_group:<id>`), so it must not equal a key of the app's own navigation (an `account` group beside an `account` section is reached by the section); `title` is any text source. `.icon(res::vectors::x)` or `.icon(Symbol::Settings)`, `.subtitle(..)` (the mobile root list's second line), `.section(..)`, `.row(..)` (appends to a trailing untitled section), `.custom(build)` (a bespoke piece that is the whole page), `.order(n)`, `.available(|| bool)`, `.requires(Cap)`. |
| `Section::new()` | `.title(..)`, `.footer(..)`, `.row(..)`, `.rows(..)`. One card. |
| `rows::toggle(&pref, title)` | On/off. |
| `rows::choice(&pref, title, [(value, label), …])` | One of N: a menu, or `.segmented()` for a few short labels. Two-way: a write to the pref from elsewhere (a menu command) moves the control. |
| `rows::slider(&pref, title, lo..=hi)` | `.step(n)`, `.value_text(fn)` (the value label carries `<id>-value`). |
| `rows::stepper(&pref, title, lo..=hi)` | A small integer. `.step(n)`. The composed control's buttons carry `<id>-decrement` / `<id>-increment` and the value label `<id>`. |
| `rows::text(&pref, title)` | `.placeholder(..)`, `.secure()`. Every edit persists. |
| `rows::action(title, on)` | A command: the button carries the title, and a `.caption` stands alone beside it as the explanatory line. `.destructive()`, `.confirm(message)` (an alert first) with `.confirm_label(..)` for its button, `.busy(signal)`. |
| `rows::link(title, url)` | Opens the URL. |
| `rows::navigate(title, build)` | Pushes a sub-page (a chevron row); a sub-window on desktop. |
| `rows::info(title, value)` | Read-only. `.copyable()`. |
| `rows::note(text)` | A footnote inside the card. |
| `rows::custom(build)` | Any piece as a row. |

Every row takes `.caption(..)` (a secondary line), `.id(day::id!(name))` (default `pref:<key>` for a pref
row), `.enabled(on)` and `.busy(on)` (a `bool`, a signal or a closure), and `.available(|| bool)`
/ `.requires(Cap)`.
A long caption wraps inside the title column rather than widening it; prose that belongs to
the whole card goes in `Section::footer`. A color value has no row of its own yet: wrap
`day-piece-colorpicker` in `rows::custom` over a hex `Pref<String>`. Rows never name a
toolkit; a different control on one platform is a `rows::custom` with `PerTarget` inside.

The row constructors live in the `rows` module and are used qualified, because several share a
name with a piece in the prelude (`toggle`, `slider`, `link`, `navigate`).

### The General group

`general_group(res::locales::ALL)` is the group every app starts with: Appearance (Light /
Dark / System, id `theme_picker`, present only where the toolkit honors a runtime override,
`Cap::Appearance`) and Language (System plus every bundled locale under its own name, id
`language_picker`). The prefs behind them are `APPEARANCE` (`day.appearance`) and `LANGUAGE`
(`day.locale`); a View menu or a toolbar that also sets the theme writes the same pref, so the
two can never disagree. `wire_general()` applies the stored values at boot and keeps applying
changes; the group calls it, and an app using the rows alone (`appearance_row()`,
`language_row(..)`) gets it the same way. `DAY_THEME` and `DAY_LOCALE` win at boot.

### Appearance on Android

On Android a theme pick, and a language pick, **recreates the activity**. A Material DayNight
theme resolves its light or dark variant when the activity's theme is resolved, at creation,
and a live view never re-resolves the colors it was inflated with; so once
`UiModeManager.setApplicationNightMode` has changed the app's `uiMode`, `DayActivity` recreates
itself from `onConfigurationChanged`, which is also what the platform does for any app that
leaves a system theme switch to it. The manifest keeps `uiMode` and `locale` in
`configChanges` so that the recreation is Day's, taken once the new configuration is in hand.
Two things make the recreation read as a theme change rather than a relaunch:

- **A crossfade, not a flash.** The outgoing activity captures its window (`PixelCopy`, the
  composited frame) before `recreate()`, the next activity lays the capture over its window
  from its first frame, and once the re-mounted tree has built, restored its route and laid
  out, the capture fades over the new content in 250 ms. A user who removed animations
  (`reduce_motion`, [accessibility](accessibility.md)) gets the cut instead. One capture
  exists at a time, released when the overlay comes down or the activity finishes; a tree
  that never settles drops it after a few seconds rather than keeping the old frame on
  screen.
- **The same place, not the first tab.** The recreation re-runs the app's `root()`: the tree
  is re-mounted, root-scope state (`Signal::global`, `Ambient::app()`, every `Pref`) survives,
  but every signal `root()` creates starts over. A nav surface with `.restore(key)` reads its
  own place back ([navigation](navigation.md)); for everything else day-android saves the full
  `current_route()` into the activity's instance state (`DayBridge.navState`, under the
  reserved key `day.route`) just before the teardown and `navigate`s to it one turn after the
  first build, so an app comes back on the section and the pushed page it was on (Settings ▸
  Reading, say). A launch deep link still pending wins over the saved route, as it wins over
  `.restore`, and the saved route is consumed either way. The route restore and the fade are
  ordered: the overlay comes down after the restored page is laid out.

A scripted run sees the whole switch as one step: `ui_idle` reports busy from the moment the
theme is asked for until the crossfade is done, so `select: { id: theme_picker, index: 1 }`
followed by `wait_idle` and `assert_route` proves both halves. Secondary windows do not survive
the primary's recreation (their activities go down with it), and a `std::sync::Once` in
`root()` runs once per process, not per mount.

## About

```rust
About::from_manifest() // name, version and build from Day.toml via `day build`
    .tagline(res::str::about_tagline())
    .copyright("© The Daybrite Project")
    .icon(res::images::icon)
    .website("https://…")                       // titled "Website" from Day's catalog
    .source("https://github.com/…")             // titled "Source Code"
    .link(res::str::about_forum(), "https://…") // any other link, with the app's own title
    .privacy("https://…")
    .credits(res::str::about_credits())
    .licenses(|| {
        let file = resource(res::assets::licenses_txt).expect("bundled license text");
        String::from_utf8_lossy(file.as_slice()).into_owned()
    })
    .custom(build) // replaces the composed body
```

Where windows open as covers (`Cap::MultiWindow` unsupported: the web, phones), About is a sheet
on a wide screen and a cover on a narrow one, with its title and a Done button (`about_done`),
like Settings there.

`from_manifest()` reads the identity `day build`/`day launch` export (`DAY_APP_ID`,
`DAY_APP_VERSION`, `DAY_APP_BUILD`, `DAY_APP_TITLE`), falling back to the launch window's app
name and `WindowOptions::version`. The core catalog carries the labels (`day_about_version`,
`day_about_website`, `day_about_source`, `day_about_privacy`, `day_about_licenses`,
`day_about_acknowledgements`) in seven languages.

Presentation: a Day window (`WindowKind::About`, singleton key `day.about`, fixed, centered,
out of the macOS Window menu) on macOS, Qt and Windows, a cover on the web; libadwaita's `AdwAboutDialog`
on GTK when the About has no `custom` piece (with one, the composed page in an `AdwDialog` over
the app window); on the phones and HarmonyOS, the header card at the top of the settings root
page, which opens the About page. The menu item is `MenuRole::About`:
the macOS App menu's "About <App>", Help ▸ About elsewhere, in both the default bar and an
installed `app_menu`. `day::open_about()` opens it from anywhere.

## Presentation

| Platform | Shell | Rows | Entry |
|---|---|---|---|
| macOS (appkit) | A fixed-width panel with an `NSToolbar` in preference style: one selectable icon+label item per group, the window fitting each page. One group: the page alone. | Composed: title leading, control trailing, in the grouped card (`SurfaceRole::SectionCard`). | App menu Settings… ⌘, |
| GNOME (gtk) | libadwaita's `AdwPreferencesDialog`, presented over the app window (a floating sheet in a wide window, a bottom sheet in a narrow one; Escape closes; on macOS always a window of its own, see [docs/windows.md](windows.md)): one `AdwPreferencesPage` per group with its icon and title, the dialog's own view switcher and search once there are two or more; each page a column of `AdwPreferencesGroup`s (title and description from the section's title and footer) holding `.boxed-list` rows. The dialog fits the tallest page (up to the declared height); a taller page scrolls inside its own page. A navigate row pushes its page into the dialog as an `AdwNavigationPage` with a back button. | `AdwActionRow` with the Day control mounted as its suffix; a custom row an `AdwPreferencesRow` holding the piece at the row's width; a note a caption under the group's list. About is `AdwAboutDialog`; an About with a `custom` piece is the composed page in an `AdwDialog` over the window. | Preferences Ctrl+, |
| Windows (winui) | The composed sidebar (`NavigationView`) + page window. | A settings-card row: card fill and stroke, header and description, the control at the trailing edge. | File ▸ Settings Ctrl+, |
| KDE (qt) | The preferences dialog KDE's `KConfigDialog`/`KPageDialog` are made of: a fixed-width `QListWidget` of icon+label page entries (two or more groups) beside a `QStackedWidget` of pages under a title header; each page a column of `QGroupBox`es (one per section, titled with it; an untitled section is a bare form) holding a `QFormLayout` of right-aligned labels, the footer a dim line under the rows. The window fits the tallest page (up to the declared height); a taller page scrolls inside its own page. One group: the page alone. | A form row with the Day control as the field, in a cell sized by Day; a custom row a cell spanning the form at its width; an action or navigate row a `QPushButton` (red for a destructive one, an arrow for a navigate row) with its caption under it; a link row a link label; a note a dim line. | Preferences Ctrl+, |
| iOS, Android, HarmonyOS | The navigation presentation: a root page (About header, then the groups as chevron rows; one group: its rows inline) in a `nav_stack` that pushes one page per group. On iOS the pages are a `UITableView` in the inset grouped style: configured `UITableViewCell`s with the system disclosure chevron, header and footer text, selection highlight on touch, and the Day control as the cell's accessory (a slider under its title, a custom row as the whole cell). On Android the pages are the Material 3 preference screen: full-bleed rows with category headers and footnotes, a ripple on every tappable row, the About header a large list item. On HarmonyOS they are a `List` of `ListItemGroup` cards. | Inset-grouped / Material preference / list-item rows with the Day control trailing (a slider under its title on Android). | The app's nav item; `open()` presents a cover with a Done button where no item is mounted. |
| Web (dom) | The navigation presentation, presented as a cover whose strip carries the stack title, the back button once a page is pushed, and the Done button, opened from the toolbar's app-menu button (web-dom emulates the app menu: `Cap::AppMenu` is `Emulated`, so Settings…, About and every app command live behind the ⋯ button, and `navigate("settings/<group>")` routes to the cover). | Composed. | The ⋯ app-menu button; no nav item (`in_nav()` is false). |

HarmonyOS group pages place their native card list inside a Day `scroll`. The inner list
measures its full content and disables its own scrolling, leaving one viewport for gestures
and dayscript `scroll_to` to reveal controls in custom groups. `group_body` remains a
content-sized list for a caller-owned scroll. The settings root keeps its native list viewport.
Day-Skies' `scripts/weather.yaml` exercises revealing and removing a custom city row.

A desktop page measures its content so the window fits it (`size_to_fit`), and does not
scroll: a tall `Group::custom` needs a `window_size` ceiling that holds it, and a dayscript
`scroll_to` on such a page is gated off the desktop toolkits.

`Preferences::presentation(Presentation::Navigation)` pins the mobile shape anywhere (an app
that keeps settings as an in-app route on desktop too); `Presentation::Window` the desktop one.
The default follows `Cap::MultiWindow` and `Cap::AppMenu`.

The word is the platform's: "Settings" on macOS, Windows and the phones, "Preferences" on GNOME
and KDE (`day_settings` / `day_preferences`), read from the running toolkit.

## Opening and routes

| | |
|---|---|
| `prefs.open()` / `day_piece_settings::open()` | The window on desktop; the nav item when mounted, else a cover, on mobile. |
| `prefs.open_at("weather")` | Opens on a group (`"weather/host"` names a row, reserved for native rows that can reveal one). |
| `navigate("settings/weather")` | The string route (a walkthrough with more than one group names the group: on a phone the root page lists groups, so `settings` alone shows no row control): on mobile through the app's nav item and the stack (`settings` is the conventional item key, `NAV_KEY`); on desktop a route controller `install()` registers opens the window on the group. Deep links and dayscript `navigate:` use it. |
| `prefs.page()` / `day_piece_settings::page()` | The navigation presentation as a piece: the nav item's destination, or any place the app embeds settings. The free function serves code that holds no `Prefs` (an app struct that must stay `Copy`). |
| `prefs.group_page("reading")` / `day_piece_settings::group_page_of("reading")` | One group's page alone (an in-reader sheet); `prefs.group_body(..)` is the same without the scroll, for a sheet that stacks several groups in one `scroll` of its own (on the phones the native page sizes itself to its rows). |
| `prefs.about_page()` | The composed About page. |
| `navigate("about")` / `navigate("settings/about")` | The About window on desktop; on a phone the About page is a stack destination under the settings section, so the route is `settings/about` (or a tap on `settings_about`). |
| `day::open_about()`, `day::register_about(action)`, `day::register_about_window(options, build)` | The day-core layer underneath. |

Element ids: `settings_root`, `settings_stack`, `settings_about` (the header card),
`prefs_group:<id>` (a group's root-list row and its page), `pref:<key>` (a pref row's control),
`theme_picker`, `language_picker`, `about_page`, `about_name`, `about_version`,
`about_tagline`, `about_link:<n>` (links in declaration order), `about_privacy`, `about_copyright`, and
`settings_done`, the cover's Done bar button (a toolbar item: dayscript reaches it with
`toolbar: { item: settings_done }`, not `tap`).

These are declared with `id!` on every toolkit, including native settings shells. Keyed
IDs use the macro's canonical encoding: for example, group `general` is
`prefs_group:general`. A custom row ID stays an `ElementId` through the native shell and
becomes text only when registered with the element tree.

## Providers

A crate the app depends on contributes a group without the app naming it:

```rust
day_piece_settings::provider! {
    static UPDATES: Provider = Provider {
        id: "updates",
        order: 900,
        available: || !managed_installation(),
        group: updates_group,                 // fn() -> Group
        startup: Some(schedule_checks),       // runs inside install()
    };
}
```

`provider!` registers the static in the link-time `PROVIDERS` slice (linkme) on native targets;
on wasm, where linkme has no implementation, and anywhere explicit order matters,
`register_provider(&UPDATES)` or `Preferences::new().provider(&UPDATES)` names it.
`.providers(Providers::None)` and `.without_provider("updates")` opt out. A provider's
strings live in its own private catalog ([localization](localization.md)); shared vocabulary
(`day_settings_updates`, `day_check_for_updates`) is in the core catalog.

The self-updater is the first provider: its Updates group (check automatically, frequency,
download automatically, last checked, Check Now) and its Check for Updates… command, which
`day::register_check_for_updates` places under About in the macOS App menu and in Help
elsewhere (`MenuRole::CheckForUpdates`).

## Testing

The crate's `#[day::test]` cases (`settings_theme_applies_and_persists`,
`settings_rows_persist`) run in the conformance app on every toolkit and under `cargo test` on
the mock. The day-pieces mock e2e suite covers the window registration, the menu injection and
the singleton reopen. The starter's `dayscript/demo.yaml` navigates to `settings` and waits for
`language_picker` on every target, so a broken route or id fails the template's CI. In a
script, `screenshot: { name: s, window: day.preferences }` (or `day.about`) captures the panel
instead of the primary window, and `menu: { key: day_about }` / `menu: { key: day_check_for_updates }`
dispatch the injected role items, which carry no id of their own.

### Settings on tablet widths

The standalone navigation Settings surface uses the `WindowKind::Preferences` modal fallback.
It stays fullscreen on compact windows and requests a sheet on regular-width windows by
default; applications need no special configuration. UIKit supplies its native form sheet,
and backends without native sheets (including Android and HarmonyOS) center the shared
640-by-620 maximum card over a modal scrim. Done and native Back close
the same keyed surface; closing and reopening resets the Settings path as before. Embedded
settings pages and desktop Preferences windows keep their existing presentation.
