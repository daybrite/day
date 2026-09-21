---
title: Localization
description: "Organize Fluent catalogs by app, crate, or source module; use generated Rust accessors; add languages and check translations."
order: 21
section: Guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Keep user-visible text in Fluent catalogs and use the generated Rust functions to display it.
Day checks the message names and arguments when you build. Labels update when their arguments
or the selected locale change.

Localize text as you add a feature, including button labels, accessibility descriptions, error
messages, and help. Write complete messages so translators can change word order and plural
forms. Keep route names, element IDs, and saved-data keys stable across languages.

## Choose where a catalog belongs

A small app can keep its translations in `resource/locales/<locale>/app.ftl`. A reusable crate
can keep them beside its code. Within a crate, you can split the catalog by source module.

| Catalog | Build function | Include macro | Generated accessor |
| --- | --- | --- | --- |
| App resources | `day_build::generate_resources()` | `day::resources!()` | `res::str::greeting(name)` |
| Private crate catalog | `day_build::generate_locales()` | `day_fluent::locales!()` | `blockblast::res::str::game_title()` |
| Private file catalog, such as `board.ftl` | `day_build::generate_locales()` | `day_fluent::locales!()` | `blockblast::res::board::str::moves(count)` |

The app resource generator keeps its existing behavior: files within a locale share the app's
namespace. Private catalogs give each crate and named file a separate namespace. Two games can
both define `game_title` and return different text.

## Set up the app catalog

The scaffold from `day new app` includes the resource generator. In an existing app, call it
from `build.rs`:

```rust
fn main() {
    day_build::generate_resources().expect("resource codegen");
}
```

Include the result at the crate root in `src/lib.rs`:

```rust
day::resources!();
```

Add messages under each locale directory:

```ftl
# resource/locales/en/app.ftl
app_title = Field Notes
greeting = Hello, { $name }!
unread_count = { $count ->
    [one] You have one unread note
   *[other] You have { $count } unread notes
}
```

Use snake_case keys because they become Rust function names. Fluent permits `unread-count`,
but Day's generator requires `unread_count`.

Pass the catalog through the `WindowOptions` used by your app's entry points:

```rust
pub fn window() -> day::WindowOptions {
    day::WindowOptions {
        locales: Some((res::locales::DEFAULT, res::locales::CATALOG)),
        title_fn: Some(|| res::str::app_title().format()),
        ..Default::default()
    }
}
```

Day installs this catalog after collecting the platform's language preferences and before
computing the window title. Use the same options for `day::launch` and `day::day_start!`.
Installing the catalog before launch misses those preferences; installing it inside the root
builder is too late for the title.

## Use generated strings

Pass the generated value directly to a piece that accepts text:

```rust
use day::prelude::*;

let name = Signal::new(String::from("Ada"));
let unread = Signal::new(3_i64);

column((
    label(res::str::greeting(name)),
    label(res::str::unread_count(unread)),
))
```

Generated functions return `LocalizedText`. Arguments accept values or signals. A missing
function, the wrong number of arguments, or a string passed to a numeric plural selector is a
compile error. Every locale must use the same parameter names; parameter order in Rust is
alphabetical. IDE hover shows the reference message.

Keep numbers numeric so Fluent can choose the plural form and format the digits. Use
`NUMBER()` and `DATETIME()` when the message needs formatting options:

```ftl
balance = Balance: { NUMBER($amount, minimumFractionDigits: 2) }
last_saved = Saved { DATETIME($when, dateStyle: "long", timeStyle: "short") }
```

`DATETIME()` accepts civil date/time strings or Unix seconds rendered as UTC. The
[formatting reference](/docs/internal/localization#formatted-values-number-and-datetime)
lists the supported options. Day also provides `format_decimal`, locale-aware sorting, and
search matching for values outside messages.

Use `.format()` when an API needs a `String`. It resolves the value at the time of the call.
Put that call inside a reactive closure if the text must follow later changes.

For a choice of labels, select generated values:

```rust
let title = if finished {
    crate::res::str::play_again()
} else {
    crate::res::str::resume()
};
button(title)
```

The dynamic `tr(key)` API remains available for keys known only at runtime. It uses the global
app/core lookup and cannot check a key or its arguments at compile time.

## Give a crate a private catalog

Add these dependencies to the reusable crate. Use the same Day revision as the app:

```toml
[dependencies]
day-fluent = { git = "https://github.com/daybrite/day.git" }

[build-dependencies]
day-build = { git = "https://github.com/daybrite/day.git" }
```

In that crate's `build.rs`:

```rust
fn main() {
    day_build::generate_locales().expect("localization codegen");
}
```

In its `src/lib.rs`:

```rust
day_fluent::locales!();
```

The generator reads the crate's `resource/locales/` directory and writes `day_locales.rs` into
Cargo's output directory. The macro includes it as `pub mod res`.

```text
blockblast/
  build.rs
  src/lib.rs
  resource/locales/
    en/app.ftl
    fr/app.ftl
```

Define `game_title` in each file. Inside the crate, call `crate::res::str::game_title()`.
An app that depends on it can call `blockblast::res::str::game_title()`.

Each accessor carries its catalog, so the app does not need to register a list of dependency
catalogs. The first accessor use loads the catalog into the runtime's cache. All catalogs use
the app's locale signal. Reinstalling the app catalog leaves private catalogs intact.

The two include macros both declare `res`. Use one at a given module location. If a crate
needs both generators, include their output files under separate modules, as described in the
[reference](/docs/internal/localization#combining-resource-and-private-catalog-generation).

## Split a catalog by source module

Add a named file beside `app.ftl` in each locale:

```text
resource/locales/
  en/app.ftl
  en/board.ftl
  fr/app.ftl
  fr/board.ftl
```

The crate-wide `app.ftl` holds shared messages and Fluent terms:

```ftl
# resource/locales/en/app.ftl
-brand = Block Blast
game_title = { -brand }
close = Close
```

A named file can use those terms and add messages for one part of the interface:

```ftl
# resource/locales/en/board.ftl
heading = { -brand } board
moves = { NUMBER($count) } moves
```

The generated calls are:

```rust
crate::res::str::game_title()
crate::res::board::str::heading()
crate::res::board::str::moves(4_i64)
crate::res::board::str::close()
```

`board.ftl` imports the same locale's `app.ftl`. Its generated module includes accessors for
those shared messages. Sibling files may reuse a key; a file cannot redefine a message or term
imported from `app.ftl`. The build reports that collision.

A file can correspond to `board.rs`, but Day does not scan Rust source files to establish the
connection. It derives the module name from the `.ftl` filename. Use the flat layout
`<locale>/<file>.ftl`; nested directories are not supported by the private generator.

[Day Games](https://github.com/daybrite/Day-Games) uses a catalog per game and a separate
`gamekit` catalog for shared controls. Its `chrome.ftl` demonstrates the file-level form,
including `gamekit::res::chrome::str::done()`.

## Move existing translations into a crate

Migrate one component at a time so the remaining app strings keep working:

1. Move the component's messages into its `resource/locales/` directory for every language.
   Keep app identity, permission reasons, and platform shortcut labels at the app root.
2. Add `generate_locales()` to its build script and `day_fluent::locales!()` to its crate root.
3. Replace string-key calls with generated accessors, including calls from the parent app.
   Select accessor functions or localized values when a label depends on state.
4. Update DayScript text assertions to use qualified keys. Element IDs and routes stay the same.
5. Remove the moved messages from the global catalog, then build, lint, and run the localized
   walkthroughs.

A prefix such as `bb_` is optional inside a private catalog. If you remove it, rename the key
in every locale and update its call sites together. Moving a file within the global resource
catalog does not create a private namespace; the private generator supplies that separation.

## Add a locale

Start at the app root:

```sh
day localize list
day localize add fr ar
day localize list
```

`add` copies the default locale's Fluent files. It also adds store listing files, an Xcode
`knownRegions` entry, and a locale in `website/site.toml` when those files exist. Supported
starter messages receive translations; other app messages keep the copied text under a
`TODO: translate` comment. Store text is copied without that comment because it may be uploaded.

The command edits the app root. It does not visit private catalogs in dependency crates or
update CI's locale list. For each crate you maintain, copy the reference files into a new
locale directory, then translate them. For example, from a workspace containing `blockblast`:

```sh
mkdir -p games/blockblast/resource/locales/fr
cp games/blockblast/resource/locales/en/*.ftl games/blockblast/resource/locales/fr/
```

Keep the same files, keys, and parameter names across the languages you support. Translate
complete messages, including accessibility text and instructions. A file added later needs a
copy in each locale too. Rebuild to regenerate the accessors and embedded catalogs.

Use tags such as `fr`, `pt-BR`, or `zh-CN` consistently in project directories. Day maps tags to
store and platform spellings during generation. Keep identity text, permission reasons, and
platform shortcut labels in the app's root catalogs because platform metadata tools read
those catalogs. Store descriptions and release notes live separately in `store/<locale>/`.
See [store listings](/docs/internal/store) and [permission reasons](/docs/guide-permissions).

`day localize remove fr` removes the locale from the app surfaces that `add` manages. It refuses
to remove the default locale. Remove the corresponding dependency catalogs and CI entries
separately.

## Check missing and stale translations

Run the compiler and the localization checks before reviewing screenshots:

```sh
cargo check --workspace
day localize list
day lint --strict
```

`localize list` reports differences between the app's locale directories, store listings,
Xcode regions, and website configuration. It is an informational report. `lint --strict`
fails when findings remain.

| Check | What it detects |
| --- | --- |
| Generated Rust accessors | Removed or misspelled keys at call sites, wrong argument counts, and numeric argument type errors |
| Catalog generation | Parameter differences across translations; private catalog syntax errors, duplicate entries, and imported-key collisions |
| `day lint` | Missing messages in existing locale directories, invalid formatting calls, and locale differences between app metadata surfaces |
| Global app key lint | Unknown literal keys and apparently unused keys in the app catalog |
| Private catalog lint | Catalog validity and per-file translation coverage in workspace crates using `generate_locales()` |

Private catalog lint does not report unused public accessors. Another crate may use them.
It also does not require every dependency to support every app language. If your app promises
complete translation coverage, add a project check that compares the locale directories and
file/key sets in every maintained crate. Day Games includes an example in
[`scripts/check-locales.py`](https://github.com/daybrite/Day-Games/blob/main/scripts/check-locales.py).

A green lint result does not prove that a translation is current. Changing an English sentence
without changing its key or parameters leaves the translated message structurally valid.
Day does not track translation review dates or source-text hashes. Review source changes in
version control and update the affected translations in the same change:

```sh
git diff -- ':(glob)**/locales/en/*.ftl'
rg -n 'TODO: translate' --glob '*.ftl' .
```

The diff shows uncommitted reference-text changes. For committed work, compare against the
last revision whose translations were reviewed. A copied English sentence can pass lint;
check the text as well as the keys. When you remove a message, remove it from every locale.
When you change its meaning, review every translation that uses the key.

## Switching locale at runtime

Use the prelude's locale functions:

```rust
use day::prelude::*;

set_locale("fr");
let selected = day::locale().get();
```

Generated text bindings in the app and its dependencies update together. The app catalog
still declares the languages available at launch through `WindowOptions::locales`.
Day checks an explicit launch override, or the host's ordered language preferences, then
uses the configured default. A regional preference such as `fr-CA` can use a `fr` catalog.

A missing private message falls back to that catalog's default language. It does not search
another crate or the app catalog for a matching key. The global app catalog retains the
app-to-core fallback used by dialog buttons and menu commands. The
[lookup reference](/docs/internal/localization#lookup-and-fallback) gives both lookup orders.

Right-to-left layout direction is chosen at launch. Switching locale while the app runs
updates strings, but changing between left-to-right and right-to-left layout requires a new
launch. An in-app language preference must be saved and restored by your app.

## Test each language

Run a DayScript walkthrough in the languages you ship:

```sh
day launch -p web-dom --locales 'en fr ar' --script dayscript/walkthrough.yaml
day launch -p macos-appkit --locales en-XA --script dayscript/walkthrough.yaml
```

The `-XA` pseudolocale accents and expands the selected language's messages. It helps find
hardcoded text, clipped labels, and layouts that need more room. Use a real right-to-left
locale, such as Arabic, to check direction. Inspect screenshots from the native targets too;
font metrics and system dialogs differ between platforms.

For private catalogs, qualify text assertions with the Cargo package name and optional file
stem. Hyphens in the package name become underscores:

```yaml
- assert_text: { id: game-heading, key: blockblast::game_title }
- assert_text:
    id: move-count
    key: blockblast::board::moves
    args: { count: 4 }
```

An accessor registers its catalog on first use. If a script needs a catalog before the UI has
used it, call its generated `res::locales::register()` or
`res::board::locales::register()` during setup. Bare assertion keys keep their global meaning.
Keep element IDs unchanged across locales so the same script can drive each language.

In an existing job that calls `daybrite/actions/.github/workflows/dayapp.yml`, set both inputs:

```yaml
with:
  locales: en fr ar
  scripts: dayscript/walkthrough.yaml
```

Update this locale list when you add or remove a language. Localized screenshot titles and
captions belong in the DayScript screenshot metadata; the generated app website uses those
alongside the translated store descriptions. See [DayScript](/docs/dayscript) for assertions,
captures, and gallery metadata.
