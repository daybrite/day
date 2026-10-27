---
title: Localization
description: "Organize Fluent catalogs by app, crate, or source module; use generated Rust accessors; add languages and check translations."
order: 21
section: How-to guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day uses Mozilla's [Fluent](https://projectfluent.org/) for translations. Fluent is a
localization system that lets translators express messages using the grammar of their own
language. A translation can change word order, choose plural forms, or account for grammatical
gender without requiring other languages to use the same structure.

Fluent messages live in plain-text `.ftl` files, grouped into catalogs for each language.
Each message has an identifier and can contain variables, references to other messages, and
conditional variants. The app supplies data such as a name or a count; the catalog contains
the wording and language-specific rules. For example, a language with several plural forms
can select the appropriate wording from the same count your app already supplies.

The [Fluent syntax guide](https://projectfluent.org/fluent/guide/) introduces the message
format with examples. Its [selectors chapter](https://projectfluent.org/fluent/guide/selectors.html)
explains how messages choose variants, including plural forms.

Day turns these catalogs into generated Rust functions that your app uses across platforms.
Keep user-visible text in the catalogs and call those functions to display it. Day checks the
message names and arguments when you build. Labels update when their arguments or the selected
locale change.

Localize text as you add a feature, including button labels, accessibility descriptions, error
messages, and help. Write complete messages so translators can change word order and plural
forms. Keep route names, element IDs, and saved-data keys stable across languages.

## Choose where a catalog belongs

A small app can keep its translations in `resource/locales/<locale>/app.ftl`. A reusable crate
can keep them beside its code. Within a crate, you can split the catalog by source module.

| Catalog | Build function | Include macro | Generated accessor |
| --- | --- | --- | --- |
| App resources | [`day_build::prebuild_project()`](https://docs.rs/day-build/latest/day_build/fn.prebuild_project.html) | `day::resources!()` | `res::str::greeting(name)` |
| Private crate catalog | [`day_build::generate_locales()`](https://docs.rs/day-build/latest/day_build/fn.generate_locales.html) | `day_localization_ui::locales!()` | `blockblast::res::str::game_title()` |
| Private file catalog, such as `board.ftl` | [`day_build::generate_locales()`](https://docs.rs/day-build/latest/day_build/fn.generate_locales.html) | `day_localization_ui::locales!()` | `blockblast::res::board::str::moves(count)` |

The app resource generator keeps its existing behavior: files within a locale share the app's
namespace. Private catalogs give each crate and named file a separate namespace. Two games can
both define `game_title` and return different text.

## Set up the app catalog

The scaffold from `day new app` includes the resource generator. In an existing app, call it
from `build.rs`:

```rust
fn main() {
    day_build::prebuild_project().expect("day-build: prebuild");
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

Pass the catalog through the `WindowOptions` used by your app's entry points. The generated
`res::window_options()` already carries it, with the title from the [`app_title`](https://docs.rs/day-build/latest/day_build/fn.app_title.html) message and the
version from Cargo.toml:

```rust
pub fn window() -> day::WindowOptions {
    res::window_options().size(960.0, 640.0)
}
```

A message with two or more variables takes one generated named-field struct
(`res::str::items_moved(res::str::ItemsMoved { count, folder })`), so same-typed arguments
cannot be swapped.

Day installs this catalog after collecting the platform's language preferences and before
computing the window title. Use the same options for [`day::launch`](https://docs.rs/day-app/latest/day/fn.launch.html) and `day::day_start!`.
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

`Ada` stands in for a user's name; the greeting itself belongs in Fluent.
Generated functions return `LocalizedText`. Arguments accept values or signals. A missing
function, the wrong number of arguments, or a string passed to a numeric plural selector is a
compile error. Every locale must use the same parameter names. A message with one parameter
takes that argument directly; a message with several takes a generated struct with named
fields. IDE hover shows the reference message:

```ftl
items_moved = Moved { $count } items to { $folder }
```

```rust
label(res::str::items_moved(res::str::ItemsMoved {
    count,
    folder,
}))
```

The [example catalog](/docs/reference/localization#reference-example-messages) defines the messages used throughout
the UI snippets. Copy the messages your example needs into your app's catalog.

Keep numbers numeric so Fluent can choose the plural form and format the digits. Use
`NUMBER()` and `DATETIME()` when the message needs formatting options:

```ftl
balance = Balance: { NUMBER($amount, minimumFractionDigits: 2) }
last_saved = Saved { DATETIME($when, dateStyle: "long", timeStyle: "short") }
```

`DATETIME()` accepts civil date/time strings or Unix seconds rendered as UTC. The
[formatting reference](/docs/reference/localization#formatted-values-number-and-datetime)
lists the supported options. Day also provides [`format_decimal`](https://docs.rs/day-localization-ui/latest/day_localization_ui/fn.format_decimal.html), locale-aware sorting, and
search matching for values outside messages.

Use [`.format()`](https://docs.rs/day-localization-ui/latest/day_localization_ui/struct.LocalizedText.html#method.format) when an API needs a `String`. It resolves the value at the time of the call.
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

Add the corresponding messages to each locale:

```ftl
play_again = Play again
resume = Resume
```

The dynamic [`tr(key)`](https://docs.rs/day-localization-ui/latest/day_localization_ui/fn.tr.html) API remains available for keys known only at runtime. It uses the global
app/core lookup and cannot check a key or its arguments at compile time.

## Give a crate a private catalog

Add these dependencies to the reusable crate:

```sh
cargo add day-localization-ui
cargo add --build day-build
```

In that crate's `build.rs`:

```rust
fn main() {
    day_build::generate_locales().expect("localization codegen");
}
```

In its `src/lib.rs`:

```rust
day_localization_ui::locales!();
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
[reference](/docs/reference/localization#combining-resource-and-private-catalog-generation).

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
crate::res::str::game_title();
crate::res::board::str::heading();
crate::res::board::str::moves(4_i64);
crate::res::board::str::close();
```

`board.ftl` imports the same locale's `app.ftl`. Its generated module includes accessors for
those shared messages. Sibling files may reuse a key; a file cannot redefine a message or term
imported from `app.ftl`. The build reports that collision.

A file can correspond to `board.rs`, but Day does not scan Rust source files to establish the
connection. It derives the module name from the `.ftl` filename. Use the flat layout
`<locale>/<file>.ftl`; nested directories are not supported by the private generator.

A workspace can keep one catalog per game crate and a separate catalog for the controls they
share. A `chrome.ftl` in that shared crate produces the file-level form, and the games reach its
strings as `gamekit::res::chrome::str::done()`.

## Move existing translations into a crate

Migrate one component at a time so the remaining app strings keep working:

1. Move the component's messages into its `resource/locales/` directory for every language.
   Keep app identity, permission reasons, and platform shortcut labels at the app root.
2. Add [`generate_locales()`](https://docs.rs/day-build/latest/day_build/fn.generate_locales.html) to its build script and `day_localization_ui::locales!()` to its crate root.
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

`add` copies the default locale's Fluent files. It also adds a store listing table, an Xcode
persistent `project.xcproj` localization, and a locale in `website/site.toml` when those files exist. Generated Xcode projects derive their locales from the Fluent directories on preparation. Supported
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
those catalogs. Store descriptions and release notes live separately in `store/storefront.toml`, one
`[storefront.metadata.<tag>]` table per locale.
See [store listings](/docs/how-to/store) and [permission reasons](/docs/how-to/guide-permissions).

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

`localize list` reports differences between the app's locale directories, the store listing's locales,
Xcode regions, and website configuration. It is an informational report. `lint --strict`
fails when findings remain.

| Check | What it detects |
| --- | --- |
| Generated Rust accessors | Removed or misspelled keys at call sites, wrong argument counts, and numeric argument type errors |
| Catalog generation | Parameter differences across translations; private catalog syntax errors, duplicate entries, and imported-key collisions |
| `day lint` | Missing messages in existing locale directories, invalid formatting calls, and locale differences between app metadata surfaces |
| Global app key lint | Unknown literal keys and apparently unused keys in the app catalog |
| Private catalog lint | Catalog validity and per-file translation coverage in workspace crates using [`generate_locales()`](https://docs.rs/day-build/latest/day_build/fn.generate_locales.html) |

Private catalog lint does not report unused public accessors. Another crate may use them.
It also does not require every dependency to support every app language. If your app promises
complete translation coverage, add a project check that compares the locale directories and
file/key sets in every maintained crate: a script that walks each crate's `resource/locales/` and
diffs the tags and keys it finds covers it.

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
still declares the languages available at launch through [`WindowOptions::locales`](https://docs.rs/day-spec/latest/day_spec/struct.WindowOptions.html#method.locales).
Day checks an explicit launch override, or the host's ordered language preferences, then
uses the configured default. A regional preference such as `fr-CA` can use a `fr` catalog.
When the user changes the system language (or, on Android 13 and later, the app's own
language) while the app is running, Android rebuilds the app in the new language, just as iOS
does by restarting it.

A missing private message falls back to that catalog's default language. It does not search
another crate or the app catalog for a matching key. The global app catalog retains the
app-to-core fallback used by dialog buttons and menu commands. The
[lookup reference](/docs/reference/localization#lookup-and-fallback) gives both lookup orders.

Right-to-left layout direction is chosen at launch. Switching locale while the app runs
updates strings, but changing between left-to-right and right-to-left layout requires a new
launch. An in-app language preference must be saved and restored by your app.

## Test each language

Run a DayScript walkthrough in the languages you ship:

```sh
day launch -t web-dom --locales 'en fr ar' --script dayscript/walkthrough.yaml
day launch -t macos-appkit --locales en-XA --script dayscript/walkthrough.yaml
```

The `-XA` pseudolocale accents and expands the selected language's messages. It helps find
hardcoded text, clipped labels, and layouts that need more room. Use a real right-to-left
locale, such as Arabic, to check direction. Inspect screenshots from the native targets too;
font metrics and system dialogs differ between platforms.

For private catalogs, qualify text assertions with the Cargo package name and optional file
stem. Hyphens in the package name become underscores:

```yaml
- assert_text: { id: game_heading, key: blockblast::game_title }
- assert_text:
    id: move_count
    key: blockblast::board::moves
    args: { count: 4 }
```

An accessor registers its catalog on first use. If a script needs a catalog before the UI has
used it, call its generated `res::locales::register()` or
`res::board::locales::register()` during setup. Bare assertion keys keep their global meaning.
Keep element IDs unchanged across locales so the same script can drive each language.

In a [Day app workflow](/docs/how-to/github-actions), set both inputs:

```yaml
with:
  locales: all   # or name them: en fr ar
  scripts: dayscript/walkthrough.yaml
```

`all` is every locale the app ships, read from its `resource/locales/` catalogs with the default
locale first, so `day localize add` reaches CI as well. Name the locales instead and that list is
yours to update. Localized screenshot titles and
captions belong in the DayScript screenshot metadata; the generated app website uses those
alongside the translated store descriptions. See [DayScript](/docs/how-to/dayscript) for assertions,
captures, and gallery metadata.

## Check message lifetime and build failures

Use generated message values directly where a piece accepts `IntoText`; call [`.format()`](https://docs.rs/day-localization-ui/latest/day_localization_ui/struct.LocalizedText.html#method.format) only
where an API needs a `String`. Formatting creates a snapshot. To follow locale changes, keep
message inputs in state and format inside a tracked UI closure. A callback with a `Send` bound
should send data through a `Setter`; do not format UI messages on its worker thread.

The Rust blocks on this page are setup excerpts and integration fragments. For a complete,
compiled catalog and app, use [the first-app lesson](/docs/tutorials/first-app); the
[technical recipes](https://github.com/daybrite/day/blob/main/docs/snippet-catalog.md#technical-recipes) also compile their exact
catalog blocks. Undefined generated names are compiler errors, while invalid catalog/schema
input can fail [`prebuild_project`](https://docs.rs/day-build/latest/day_build/fn.prebuild_project.html), whose return is `Result<(), String>`. Keep the error handling
in `build.rs` so generation failure stops the build.

Source: [generation entry points](https://github.com/daybrite/day/blob/main/crates/day-build/src/lib.rs),
[Fluent runtime](https://github.com/daybrite/day/tree/main/crates/day-localization-ui), and
[locale runtime](https://github.com/daybrite/day/tree/main/crates/day-localization).
A compile check is not translation-quality or all-locale rendering validation.

## Bind complete messages to state

The caller supplies a user's name and a count. Both labels follow signal and locale changes.
The second message uses named arguments so the wording and order remain in Fluent.

Complete module for `src/localization_recipe.rs` in the
[technical recipe harness](https://github.com/daybrite/day/blob/main/docs/snippet-catalog.md#technical-recipes).
Add the matching Fluent messages below. This module is compiled as part of the harness;
it is not a standalone app.

<!-- technical-example: localization_recipe -->
```rust
use crate::res;
use day::prelude::*;

pub fn greeting(name: Signal<String>, count: Signal<i64>) -> impl Piece {
    column((
        label(res::str::recipe_greeting(name)).id(id!(recipe_greeting)),
        label(res::str::recipe_unread(res::str::RecipeUnread {
            count,
            name,
        }))
        .id(id!(recipe_unread)),
    ))
    .spacing(Space::M)
}
```

<!-- technical-catalog: localization_recipe -->
```ftl
recipe_greeting = Hello, { $name }!
recipe_unread = { $count ->
    [one] { $name }, you have one unread note.
   *[other] { $name }, you have { $count } unread notes.
}
```
