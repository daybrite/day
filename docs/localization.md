# Localization (§12)

Day localizes with [Mozilla Fluent](https://projectfluent.org). Text is a **key** resolved against
the current locale; the current locale is a `Signal`, so every `tr()` binding re-runs on a locale
switch, followed by one incremental relayout.

```rust
use day::prelude::*;

install_locales("en", &[
    ("en", include_str!("../locales/en/app.ftl")),
    ("fr", include_str!("../locales/fr/app.ftl")),
]);

label(tr("greeting").arg("name", user_name))   // reactive, localized
set_locale("fr");                               // every visible string updates
```

## Two layers: the app catalog and the core catalog

There are two tiers of Fluent bundles:

- **App catalog**: the locales your app registers with `install_locales`. It holds your keys and
  your translations.
- **Core catalog**: a built-in set of standard UI strings the framework itself needs (dialog
  buttons, standard menu commands), shipped inside `day-l10n` in several languages (English, French,
  Spanish, German, Japanese, Simplified Chinese). Always present, even before `install_locales`.

Lookup order for any key: app[locale] → app[default] → core[locale] → core English. So your
strings always win, and the core catalog is the fallback for the `day-*` keys the framework emits and
your app didn't define. You can override any core string just by defining the same key in your own
catalog.

Because the engine (`day-l10n`) sits low in the crate graph, the central crates localize their own UI
without the app doing anything: dialog buttons and standard menu-command labels come out in the
user's language automatically.

## Core strings the framework provides

Keys are namespaced `day-*`. The catalog covers the strings Day emits itself:

| Purpose | Keys |
|---|---|
| Dialog buttons | `day-ok` `day-cancel` `day-yes` `day-no` `day-done` `day-save` `day-close` `day-delete` |
| Menu commands (`MenuRole`) | `day-cut` `day-copy` `day-paste` `day-select-all` `day-undo` `day-redo` `day-about` `day-quit` `day-preferences` `day-minimize` `day-fullscreen` |
| App-name commands | `day-about-app` (`About {$app}`), `day-quit-app` (`Quit {$app}`), `day-edit` |

Concretely:

- **`confirm(...)`/`prompt(...)`** default their buttons to `day-ok`/`day-cancel`. In French the
  buttons read *OK* / *Annuler*; `.confirm_label`/`.cancel_label` still override.
- **`menu_role(MenuRole::Cut)`** (and the rest) get their label from the core catalog (*Couper* in
  French, *Ausschneiden* in German) instead of each backend hardcoding English.
- The AppKit **standard App menu** ("About X" / "Quit X") uses `day-about-app`/`day-quit-app`, whose
  `{$app}` interpolation gives correct per-language word order (e.g. Japanese `Dayを終了`).

Adding a language for the core strings is a `catalog/<lang>.ftl` in `day-l10n`; adding a core key is
one line per language.

## How it's layered

```
day-reactive
  └── day-l10n     ← the engine: bundles, the locale Signal, format_in, the built-in core catalog
        ├── day-pieces (dialogs, menu-role labels)   ← localize their own strings
        ├── day-appkit (menu chrome)
        └── day-fluent  ← adds the reactive `tr()` text source; re-exports the engine
```

`day-fluent` re-exports the engine, so the app-facing API (`install_locales`, `tr`, `set_locale`) is
unchanged. Core crates call `day_l10n::t("day-cancel")` (resolve once, in the current locale) for the
framework's own one-shot strings.

## Right-to-left locales

An RTL locale (Arabic, Hebrew, Farsi, …) flips the whole UI (resolved once at startup, from
`DAY_LOCALE` or the locale `install_locales` settles on — runtime `set_locale` switches strings
but not direction):

- **Day's layout engine mirrors every horizontal placement** in the place pass (`day-core`):
  rows reverse, `leading` means right, padding swaps sides, the form label column right-aligns —
  no layout implementation knows about direction. Leaf CONTENT (canvas drawing, text runs) is
  not mirrored. Children whose frames are native-owned (nav pages in splitter panes /
  nav-controller views) place via `place_child_native` and are never mirrored.
- **Each toolkit enables its native RTL mode** for widget-internal behavior: AppKit registers
  `AppleTextDirection` (volatile, registration domain) before `NSApplication` init; UIKit forces
  `semanticContentAttribute` on the window + content roots; GTK calls
  `gtk_widget_set_default_direction` (which also flips the Adw split view's sidebar side); Qt
  switches label/field text direction only (its app-wide `setLayoutDirection` would re-mirror
  containers underneath Day's absolute frames); Android sets the decor view's layout direction
  (`android:supportsRtl` rides the manifest template).

The showcase ships an Arabic locale (`--locale ar`) exercising all of this; CI captures every
walkthrough screenshot in light/dark × en/fr/ar/zh-CN, and `scripts/rtl-check.yaml` is a quick
local smoke-test.

## Pseudolocale

Setting the locale to `en-XA` accents and expands every string (`Cáncél ・ロング`) to stress-test
layout for longer translations and non-Latin glyphs, without needing a real translation.
